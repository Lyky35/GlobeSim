// ---------------------------------------------------------------------------
// Ocean surface currents + surface temperature, CPU, in the ground frame.
//
//   currents  wind-driven subtropical gyres from the steady depth-integrated
//             momentum balance (Sverdrup interior + Munk friction):
//                 nu * laplacian(psi) - beta * dpsi/dx = -curl(wind stress)
//             solved by Gauss-Seidel on the ocean mask (psi = 0 at coasts),
//             with u = -dpsi/dy, v = dpsi/dx. The beta term (Coriolis varies
//             with latitude) is what makes the return flow concentrate into
//             narrow WESTERN boundary jets — Gulf Stream, Kuroshio emerge
//             from the math, not from hand-drawing them.
//   temp      surface air + sea-surface temperature, relaxed to a radiative
//             equilibrium that depends on latitude, pushed by the daily
//             insolation cycle, shaded by the cloud deck and impact dust,
//             and ADVECTED — air by the wind belts, SST by the currents.
//             The advection terms are the heat transport: currents and winds
//             carry ~PW of heat poleward and measurably narrow the
//             equator-to-pole contrast (the HUD prints the numbers).
//
// Grid matches CloudField (256 x 128, i = 0 north pole, j = 0 at lon 0 deg E),
// so cloud cover plugs in directly and everything shares SphereGrid's UV.
// ---------------------------------------------------------------------------

use crate::atmos::wind_belts;
use macroquad::prelude::Vec3;

pub const CGW: usize = 256;
pub const CGH: usize = 128;
/// Streak texture resolution (finer than the physics grid, for smooth traces).
pub const STREAK_W: usize = 512;
pub const STREAK_H: usize = 256;

const R: f32 = 6.371e6; // m
const OMEGA: f32 = 7.292_115_9e-5; // rad/s
const DAY: f32 = 86_400.0;

/// Lumped wind-stress efficiency (drag x density ratio / mixed-layer depth),
/// calibrated so the gyre interior sits near the real ~2 cm/s and the
/// western boundary jets near ~15 cm/s.
const STRESS_K: f32 = 4.0e-8;
/// Streamfunction closure for the gyre solve (m^2/s): not physical viscosity
/// but the small diffusive term that closes the circulation through a narrow
/// western boundary layer of width nu/beta ~ 5 cells (the true interior
/// balance is Sverdrup: v = curl/beta, which needs beta*L/nu >> 1).
const MUNK_NU: f32 = 1.0e-5;

pub fn cell_lat(i: usize) -> f32 {
    90.0 - (i as f32 + 0.5) * 180.0 / CGH as f32
}
pub fn cell_lon(j: usize) -> f32 {
    (j as f32 + 0.5) * 360.0 / CGW as f32
}

/// Classify one relief-map pixel as ocean: the ETOPO palette paints water
/// blue (b >> r); land is green/tan and the ice sheets are white (b ~ r).
pub fn classify_ocean(r: u8, g: u8, b: u8) -> bool {
    let _ = g;
    b as i32 > r as i32 + 40
}

/// Ocean mask downsampled from the full-res earth relief texture (majority
/// vote per block), lon 0..360 across, north->south rows.
pub struct LandMask {
    ocean: Vec<bool>,
}

impl LandMask {
    pub fn from_rgba(w: u32, h: u32, rgba: &[u8]) -> Self {
        let mut ocean = vec![false; CGW * CGH];
        let bx = (w / CGW as u32) as usize;
        let by = (h / CGH as u32) as usize;
        for ci in 0..CGH {
            for cj in 0..CGW {
                let (mut sea, mut n) = (0u32, 0u32);
                for y in 0..by {
                    let row = ((ci * by + y) * w as usize + cj * bx) * 4;
                    for x in 0..bx {
                        let p = row + x * 4;
                        if classify_ocean(rgba[p], rgba[p + 1], rgba[p + 2]) {
                            sea += 1;
                        }
                        n += 1;
                    }
                }
                ocean[ci * CGW + cj] = sea * 2 > n;
            }
        }
        Self { ocean }
    }

    #[cfg(test)]
    fn rect(ocean: bool, i0: usize, i1: usize, j0: usize, j1: usize) -> Self {
        let mut ocean_v = vec![!ocean; CGW * CGH];
        for i in i0..i1 {
            for j in j0..j1 {
                ocean_v[i * CGW + j] = ocean;
            }
        }
        Self { ocean: ocean_v }
    }

    pub fn is_ocean(&self, i: usize, j: usize) -> bool {
        self.ocean[i * CGW + j]
    }
}

/// Bilinear sample of a physics-grid field at (lat, lon) degrees; longitude
/// wraps, latitude clamps.
fn sample_grid(f: &[f32], lat: f32, lon: f32) -> f32 {
    let lat = lat.clamp(-89.5, 89.5);
    let x = (lon.rem_euclid(360.0) / 360.0 * CGW as f32) - 0.5;
    let y = ((90.0 - lat) / 180.0 * CGH as f32 - 0.5).clamp(0.0, CGH as f32 - 1.0);
    let j0 = x.floor() as isize;
    let i0 = y.floor() as usize;
    let i1 = (i0 + 1).min(CGH - 1);
    let jw = |j: isize| j.rem_euclid(CGW as isize) as usize;
    let fx = x - x.floor();
    let fy = y - y.floor();
    let a = f[i0 * CGW + jw(j0)];
    let b = f[i0 * CGW + jw(j0 + 1)];
    let c = f[i1 * CGW + jw(j0)];
    let d = f[i1 * CGW + jw(j0 + 1)];
    (a * (1.0 - fx) + b * fx) * (1.0 - fy) + (c * (1.0 - fx) + d * fx) * fy
}

// --- temperature: radiative equilibrium backgrounds (degC, annual-ish) -----
fn t_eq_sst(lat_deg: f32) -> f32 {
    let s = lat_deg.to_radians().sin();
    29.0 - 58.0 * s * s
}
fn t_eq_air(lat_deg: f32) -> f32 {
    let s = lat_deg.to_radians().sin();
    27.0 - 62.0 * s * s
}

pub struct Climate {
    mask: LandMask,
    psi: Vec<f32>,
    uo: Vec<f32>, // surface current, east+ m/s
    vo: Vec<f32>, //                north+ m/s
    t_air: Vec<f32>,
    t_sst: Vec<f32>,
    tmp: Vec<f32>,

    // streak particle fields
    wind_parts: Vec<[f32; 3]>, // lat, lon, age days
    cur_parts: Vec<[f32; 3]>,
    pub wind_rgba: Vec<u8>,
    pub cur_rgba: Vec<u8>,

    // diagnostics for the HUD
    pub jet_ms: f32,      // strongest western-boundary current
    pub interior_ms: f32, // median gyre speed
    pub heat_pw: f32,     // peak oceanic meridional heat transport (PW)
    pub air_mean: f32,
    pub pole_air: f32, // mean air temp |lat| > 75
    pub last_ms: f64,
}

fn rng(s: &mut u32) -> f32 {
    *s ^= *s << 13;
    *s ^= *s >> 17;
    *s ^= *s << 5;
    (*s >> 8) as f32 / 16_777_216.0
}

impl Climate {
    pub fn new(mask: LandMask) -> Self {
        let mut c = Self {
            mask,
            psi: vec![0.0; CGW * CGH],
            uo: vec![0.0; CGW * CGH],
            vo: vec![0.0; CGW * CGH],
            t_air: vec![0.0; CGW * CGH],
            t_sst: vec![0.0; CGW * CGH],
            tmp: vec![0.0; CGW * CGH],
            wind_parts: vec![[0.0; 3]; 1600],
            cur_parts: vec![[0.0; 3]; 1600],
            wind_rgba: vec![0; STREAK_W * STREAK_H * 4],
            cur_rgba: vec![0; STREAK_W * STREAK_H * 4],
            jet_ms: 0.0,
            interior_ms: 0.0,
            heat_pw: 0.0,
            air_mean: 0.0,
            pole_air: 0.0,
            last_ms: 0.0,
        };
        for i in 0..CGH {
            let lat = cell_lat(i);
            for j in 0..CGW {
                c.t_air[i * CGW + j] = t_eq_air(lat);
                c.t_sst[i * CGW + j] = t_eq_sst(lat);
            }
        }
        let mut seed = 0x9E37_79B9u32;
        for p in c.wind_parts.iter_mut().chain(c.cur_parts.iter_mut()) {
            p[0] = (rng(&mut seed) * 2.0 - 1.0) * 85.0;
            p[1] = rng(&mut seed) * 360.0;
            p[2] = rng(&mut seed) * 4.0;
        }
        c.solve_gyre();
        c.currents_from_psi();
        c.stats();
        c
    }

    // --- wind-driven gyres ----------------------------------------------------

    /// Wind stress curl: the belts depend on latitude only, so
    /// curl(tau) = -d(Cd*u)/dy = -(STRESS_K/R) * du/dlat_rad.
    fn stress_curl(&self) -> Vec<f32> {
        let dlat = (180.0 / CGH as f32).to_radians();
        let mut c = vec![0.0; CGW * CGH];
        for i in 0..CGH {
            let lat = cell_lat(i);
            let du = wind_belts(lat + 90.0 / CGH as f32).0 - wind_belts(lat - 90.0 / CGH as f32).0;
            let curl = -STRESS_K * du / (R * dlat);
            for j in 0..CGW {
                c[i * CGW + j] = curl;
            }
        }
        c
    }

    /// Gauss-Seidel relaxation of nu*lap(psi) - beta*dpsi/dx = -curl on the
    /// ocean interior; psi pinned to 0 on land, ice margins and |lat| > 84.
    pub fn solve_gyre(&mut self) {
        let curl = self.stress_curl();
        let dphi = (180.0 / CGH as f32).to_radians();
        let dlam = (360.0 / CGW as f32).to_radians();
        // Warm start with the analytic Sverdrup ramp (psi = 0 at the row's
        // east edge, ramping westward): the iteration then only has to bend
        // the ramp onto the coastlines and grow the boundary layers, rather
        // than build the whole circulation cell by cell.
        for i in 1..CGH - 1 {
            let lat = cell_lat(i);
            let cos = lat.to_radians().cos().max(0.08);
            let beta = (2.0 * OMEGA * lat.to_radians().cos() / R).max(1e-12);
            let per_col = curl[i * CGW] / beta * R * cos * dlam;
            for j in 0..CGW {
                self.psi[i * CGW + j] = if self.mask.is_ocean(i, j) {
                    per_col * (j as f32 - (CGW - 1) as f32)
                } else {
                    0.0
                };
            }
        }
        for _ in 0..6000 {
            for i in 1..CGH - 1 {
                let lat = cell_lat(i);
                let cos = lat.to_radians().cos().max(0.08);
                let beta = 2.0 * OMEGA * lat.to_radians().cos() / R;
                let inv_n = 1.0 / (R * dphi);
                let inv_e = 1.0 / (R * cos * dlam);
                let a_ns = MUNK_NU * inv_n * inv_n;
                let a_ew = MUNK_NU * inv_e * inv_e;
                let g = beta * 0.5 * inv_e;
                for j in 0..CGW {
                    let k = i * CGW + j;
                    if !self.mask.is_ocean(i, j) || lat.abs() > 84.0 {
                        self.psi[k] = 0.0;
                        continue;
                    }
                    let jp = (j + 1) % CGW;
                    let jm = (j + CGW - 1) % CGW;
                    let (p_n, p_s) = (self.psi[k - CGW], self.psi[k + CGW]);
                    let (p_e, p_w) = (self.psi[i * CGW + jp], self.psi[i * CGW + jm]);
                    // nu*lap(psi) + beta*dpsi/dx = -curl. The beta term's
                    // sign is what puts the narrow return jet on the WESTERN
                    // side of each basin (Stommel's result); flip it and the
                    // whole gyre mirrors east.
                    let num = a_ns * (p_n + p_s) + (a_ew + g) * p_e + (a_ew - g) * p_w - curl[k];
                    let den = 2.0 * (a_ns + a_ew);
                    self.psi[k] = num / den;
                }
            }
        }
    }

    /// u = -dpsi/dy (east+), v = dpsi/dx (north+), y = north, x = east.
    pub fn currents_from_psi(&mut self) {
        let dphi = (180.0 / CGH as f32).to_radians();
        let dlam = (360.0 / CGW as f32).to_radians();
        for i in 1..CGH - 1 {
            let lat = cell_lat(i);
            let cos = lat.to_radians().cos().max(0.08);
            for j in 0..CGW {
                let k = i * CGW + j;
                if !self.mask.is_ocean(i, j) {
                    self.uo[k] = 0.0;
                    self.vo[k] = 0.0;
                    continue;
                }
                let jp = (j + 1) % CGW;
                let jm = (j + CGW - 1) % CGW;
                let dpdy = (self.psi[k - CGW] - self.psi[k + CGW]) / (2.0 * R * dphi);
                let dpdx =
                    (self.psi[i * CGW + jp] - self.psi[i * CGW + jm]) / (2.0 * R * cos * dlam);
                self.uo[k] = (-dpdy).clamp(-1.2, 1.2);
                self.vo[k] = dpdx.clamp(-1.2, 1.2);
            }
        }
    }

    fn stats(&mut self) {
        let mut speeds: Vec<f32> = self
            .uo
            .iter()
            .zip(self.vo.iter())
            .map(|(u, v)| (u * u + v * v).sqrt())
            .filter(|s| *s > 1e-4)
            .collect();
        speeds.sort_by(|a, b| a.total_cmp(b));
        if speeds.is_empty() {
            return;
        }
        self.interior_ms = speeds[speeds.len() / 2];
        self.jet_ms = speeds[speeds.len() - 1];

        // Meridional heat transport by the currents (1 PW = 1e15 W): v is a
        // depth-integrated transport velocity, so the flux area is a vertical
        // strip — zonal width times the ~500 m upper-ocean layer, NOT the
        // sphere surface area. rho*cp = 4.18e6 J/(m^3 K).
        let dlam = (360.0 / CGW as f32).to_radians();
        let h_layer = 500.0f32;
        let mut pw = 0.0f32;
        for i in 20..CGH - 20 {
            let lat = cell_lat(i);
            let cos = lat.to_radians().cos();
            let zonal: f32 = (0..CGW).map(|j| self.t_sst[i * CGW + j]).sum::<f32>() / CGW as f32;
            let h: f32 = (0..CGW)
                .map(|j| {
                    let k = i * CGW + j;
                    self.vo[k] * (self.t_sst[k] - zonal) * R * cos * dlam
                })
                .sum();
            pw = pw.max((h * 4.18e6 * h_layer * 1e-15).abs()); // W -> PW
        }
        self.heat_pw = pw;

        let mean = |f: &[f32]| f.iter().sum::<f32>() / f.len() as f32;
        self.air_mean = mean(&self.t_air);
        let pole: Vec<f32> = (0..6)
            .flat_map(|i| (0..CGW).map(move |j| (i, j)))
            .chain((CGH - 6..CGH).flat_map(|i| (0..CGW).map(move |j| (i, j))))
            .map(|(i, j)| self.t_air[i * CGW + j])
            .collect();
        self.pole_air = mean(&pole);
    }

    // --- temperature ------------------------------------------------------------

    /// One sim step: radiative terms + advection (air by the wind belts, SST
    /// by the currents). `sun` is the subsolar direction in the ground frame;
    /// `cloud` is the CloudField coverage grid (same layout), `dust` the
    /// impact dust loading 0..1.
    pub fn step(&mut self, dt: f32, sun: Vec3, cloud: &[f32], dust: f32) {
        let start = std::time::Instant::now();
        let lat_s = sun.y.asin().to_degrees();
        let lon_s = sun.x.atan2(sun.z).to_degrees();
        let mut rem = dt;
        while rem > 0.0 {
            let h = rem.min(6.0 * 3600.0);
            rem -= h;
            let hd = h / DAY; // step in days
            self.t_relax(hd, lat_s, lon_s, cloud, dust);
            self.advect_air(h);
            self.advect_sst(h);
        }
        self.stats();
        self.last_ms = start.elapsed().as_secs_f64() * 1000.0;
    }

    fn t_relax(&mut self, hd: f32, lat_s: f32, lon_s: f32, cloud: &[f32], dust: f32) {
        let dim = 1.0 - 0.85 * dust.min(1.0);
        for i in 0..CGH {
            let lat = cell_lat(i);
            let (sl, cl) = (lat.to_radians().sin(), lat.to_radians().cos());
            let tsst = t_eq_sst(lat);
            for j in 0..CGW {
                let lon = cell_lon(j);
                let cos_zen = (sl * lat_s.to_radians().sin()
                    + cl * lat_s.to_radians().cos() * (lon - lon_s).to_radians().cos())
                .max(0.0);
                let shade = (1.0 - 0.35 * cloud[i * CGW + j]) * dim;
                let diurnal = cos_zen - 0.5f32.max(cl * lat_s.to_radians().cos()); // ~daily mean
                let k = i * CGW + j;
                let ocean = self.mask.is_ocean(i, j);
                // SST: heavy mixed layer, slow to move; air couples back.
                // Dust blocks sunlight but the surface still radiates to
                // space, hence the explicit cooling term (impact winter).
                let t_s = &mut self.t_sst[k];
                *t_s += hd
                    * (0.9 * diurnal * shade * 2.2
                        + 0.35 * (self.t_air[k] - *t_s)
                        + (tsst - *t_s) * 0.12
                        - 3.5 * dust.min(1.0));
                // Air: light, dragged toward the surface it sits on; land has
                // no reservoir, so it swings with the sun.
                let t_a = &mut self.t_air[k];
                let surf_target = if ocean {
                    *t_s + 1.5
                } else {
                    t_eq_air(lat) + 13.0 * diurnal * shade
                };
                let rate = if ocean { 0.45 } else { 1.6 };
                *t_a += hd * (rate * (surf_target - *t_a) - 8.0 * dust.min(1.0));
            }
        }
    }

    fn advect_air(&mut self, dt: f32) {
        for i in 0..CGH {
            let lat = cell_lat(i);
            let (u, v) = wind_belts(lat);
            let dlat = (v / R * dt).to_degrees();
            let dlon = (u / R * dt).to_degrees();
            for j in 0..CGW {
                self.tmp[i * CGW + j] = sample_grid(&self.t_air, lat - dlat, cell_lon(j) - dlon);
            }
        }
        self.t_air.copy_from_slice(&self.tmp);
        diffuse(&mut self.t_air, &mut self.tmp);
    }

    fn advect_sst(&mut self, dt: f32) {
        for i in 0..CGH {
            for j in 0..CGW {
                let k = i * CGW + j;
                if !self.mask.is_ocean(i, j) {
                    self.tmp[k] = self.t_sst[k];
                    continue;
                }
                let (lat, lon) = (cell_lat(i), cell_lon(j));
                let (u, v) = (self.uo[k], self.vo[k]);
                let mut t = sample_grid(
                    &self.t_sst,
                    lat - (v / R * dt).to_degrees(),
                    lon - (u / R * dt).to_degrees(),
                );
                // keep land pixels out of the ocean field
                if t < -60.0 {
                    t = self.t_sst[k];
                }
                self.tmp[k] = t;
            }
        }
        self.t_sst.copy_from_slice(&self.tmp);
        diffuse(&mut self.t_sst, &mut self.tmp);
    }

    // --- streaks -----------------------------------------------------------------

    /// Fade the streak canvases, advance the particles and splat their heads.
    pub fn step_streaks(&mut self, dt: f32) {
        let h = dt.min(4.0 * DAY);
        let fade = (h / (1.5 * DAY)).exp();
        for tex in [&mut self.wind_rgba, &mut self.cur_rgba] {
            for px in tex.chunks_mut(4) {
                px[3] = (px[3] as f32 * fade) as u8;
            }
        }

        for p in self.wind_parts.iter_mut() {
            let (u, v) = wind_belts(p[0]);
            p[0] += (v / R * h).to_degrees();
            p[1] += (u / R * h).to_degrees();
            p[2] += h / DAY;
            if p[2] > 3.0 + (p[1] % 3.0) || p[0].abs() > 86.0 {
                p[0] = (p[1] / 360.0 - 0.5) * 170.0;
                p[1] = (p[1] * 7.13) % 360.0;
                p[2] = 0.0;
            }
            let spd = (u * u + v * v).sqrt().min(25.0) / 25.0;
            splat(&mut self.wind_rgba, p[0], p[1], 90.0 + 140.0 * spd);
        }
        for p in self.cur_parts.iter_mut() {
            let (u, v) = (
                sample_grid(&self.uo, p[0], p[1]),
                sample_grid(&self.vo, p[0], p[1]),
            );
            let spd = (u * u + v * v).sqrt();
            p[0] += (v / R * h).to_degrees();
            p[1] += (u / R * h).to_degrees();
            p[2] += h / DAY;
            let k = (((90.0 - p[0]) / 180.0 * CGH as f32) as usize).min(CGH - 1) * CGW
                + ((p[1].rem_euclid(360.0) / 360.0 * CGW as f32) as usize).min(CGW - 1);
            if p[2] > 2.0 + (p[1] % 5.0)
                || spd < 5e-3
                || !self.mask.is_ocean(k / CGW, k % CGW)
                || p[0].abs() > 84.0
            {
                p[0] = (p[1] / 360.0 - 0.5) * 168.0;
                p[1] = (p[1] * 3.71) % 360.0;
                p[2] = 0.0;
            }
            splat(
                &mut self.cur_rgba,
                p[0],
                p[1],
                70.0 + 160.0 * (spd / 0.6).min(1.0),
            );
        }
    }

    /// Temperature as an RGBA overlay (row 0 = north, matches SphereGrid UV).
    pub fn temp_rgba(&self) -> Vec<u8> {
        let mut out = vec![0u8; CGW * CGH * 4];
        for (k, &t) in self.t_air.iter().enumerate() {
            let (r, g, b) = heat_ramp(t);
            let p = &mut out[k * 4..k * 4 + 4];
            p.copy_from_slice(&[r, g, b, 205]);
        }
        out
    }
}

/// Tiny neighbour blend: keeps the semi-Lagrangian fields from getting
/// sawtooth on the 1.4-degree grid.
fn diffuse(f: &mut [f32], tmp: &mut [f32]) {
    tmp.copy_from_slice(f);
    for i in 1..CGH - 1 {
        for j in 0..CGW {
            let jp = (j + 1) % CGW;
            let jm = (j + CGW - 1) % CGW;
            let k = i * CGW + j;
            let nb = (f[k - CGW] + f[k + CGW] + f[i * CGW + jp] + f[i * CGW + jm]) * 0.25;
            tmp[k] = f[k] * 0.96 + nb * 0.04;
        }
    }
    f.copy_from_slice(tmp);
}

/// Cold blue -> teal -> warm sand -> hot red, -40..40 degC.
fn heat_ramp(t: f32) -> (u8, u8, u8) {
    let x = ((t + 40.0) / 80.0).clamp(0.0, 1.0);
    let stops: [(f32, (f32, f32, f32)); 5] = [
        (0.0, (28.0, 42.0, 110.0)),
        (0.35, (60.0, 150.0, 200.0)),
        (0.55, (120.0, 200.0, 150.0)),
        (0.75, (235.0, 200.0, 110.0)),
        (1.0, (215.0, 70.0, 45.0)),
    ];
    for w in stops.windows(2) {
        let (a, ca) = w[0];
        let (b, cb) = w[1];
        if x <= b || (b - a).abs() < 1e-6 {
            let f = ((x - a) / (b - a)).clamp(0.0, 1.0);
            return (
                (ca.0 + (cb.0 - ca.0) * f) as u8,
                (ca.1 + (cb.1 - ca.1) * f) as u8,
                (ca.2 + (cb.2 - ca.2) * f) as u8,
            );
        }
    }
    (255, 255, 255)
}

/// Splat a streak head onto a canvas: white-ish pixels, alpha deposit.
fn splat(tex: &mut [u8], lat: f32, lon: f32, a: f32) {
    let j = ((lon.rem_euclid(360.0) / 360.0 * STREAK_W as f32) as usize).min(STREAK_W - 1);
    let i = (((90.0 - lat) / 180.0 * STREAK_H as f32) as usize).min(STREAK_H - 1);
    let k = (i * STREAK_W + j) * 4;
    let px = &mut tex[k..k + 4];
    px[0] = 235;
    px[1] = 245;
    px[2] = 255;
    px[3] = (px[3] as f32 + a).min(255.0) as u8;
}

#[cfg(test)]
mod tests {
    use super::*;
    use macroquad::prelude::vec3;

    /// Idealized planet: one rectangular ocean basin 20N..60S, land elsewhere.
    fn basin() -> LandMask {
        // lat 60N..60S -> rows (90-60)/1.406..(90+60)/1.406 ~ 21..107
        LandMask::rect(true, 21, 107, 40, 216)
    }

    #[test]
    fn classifier_splits_ocean_land_ice() {
        assert!(classify_ocean(9, 129, 205), "ETOPO ocean is blue");
        assert!(!classify_ocean(53, 117, 21), "land is green/tan");
        assert!(!classify_ocean(225, 225, 225), "ice sheets count as land");
    }

    #[test]
    fn gyres_circulate_like_the_real_ones() {
        let c = Climate::new(basin()); // new() solves the gyre + currents
        let lat_of = |i: usize| cell_lat(i);
        // NH interior subtropical gyre flows EQUATORWARD (Sverdrup): at 25N,
        // mid-basin v < 0. Southern hemisphere mirrors it.
        let i25n = ((90.0 - 25.0) / 180.0 * CGH as f32) as usize;
        let i25s = ((90.0 + 25.0) / 180.0 * CGH as f32) as usize;
        let jm = 128;
        assert!(c.vo[i25n * CGW + jm] < 0.0, "NH gyre interior flows south");
        assert!(c.vo[i25s * CGW + jm] > 0.0, "SH gyre interior flows north");
        // The return flow concentrates in a narrow WESTERN boundary jet
        // (poleward at ~35N near the basin's west edge).
        let i35n = ((90.0 - 35.0) / 180.0 * CGH as f32) as usize;
        let wb = (40..46)
            .map(|j| c.vo[i35n * CGW + j])
            .fold(0.0f32, f32::max);
        let interior = (120..140)
            .map(|j| c.vo[i35n * CGW + j].abs())
            .fold(0.0f32, f32::max);
        assert!(wb > 0.15, "western boundary jet poleward, got {wb:.3} m/s");
        assert!(
            wb > 3.0 * interior,
            "jet must beat the interior: {wb:.3} vs {interior:.3}"
        );
        assert!(lat_of(i25n) > 0.0);
    }

    #[test]
    fn currents_transport_heat_poleward() {
        let mut c = Climate::new(basin());
        // seed a latitudinal SST contrast and integrate a while
        for _ in 0..60 {
            c.step(86_400.0, vec3(0.0, 0.0, 1.0), &[0.0; CGW * CGH], 0.0);
        }
        let sst_at = |lat: f32, lon: f32| {
            let i = ((90.0 - lat) / 180.0 * CGH as f32) as usize;
            let j = ((lon % 360.0) / 360.0 * CGW as f32) as usize;
            c.t_sst[i * CGW + j]
        };
        // 40N in the western half of the basin (downstream of the western
        // boundary current) must be warmer than the eastern half, same latitude.
        let west = sst_at(42.0, 70.0); // basin lon ~56..300; west side ~70E
        let east = sst_at(42.0, 270.0);
        assert!(
            west > east + 1.0,
            "western boundary warms downwind: {west:.1} vs {east:.1}"
        );
        assert!(
            c.heat_pw > 0.05,
            "heat transport measured: {:.2} PW",
            c.heat_pw
        );
    }

    #[test]
    fn dust_collapses_surface_temperature() {
        let mut clean = Climate::new(basin());
        let mut dirty = Climate::new(basin());
        for _ in 0..90 {
            clean.step(86_400.0, vec3(0.0, 0.0, 1.0), &[0.0; CGW * CGH], 0.0);
            dirty.step(86_400.0, vec3(0.0, 0.0, 1.0), &[0.0; CGW * CGH], 0.9);
        }
        assert!(
            dirty.air_mean < clean.air_mean - 10.0,
            "impact winter: {} vs {} degC mean",
            dirty.air_mean,
            clean.air_mean
        );
    }

    /// Georegression against the real planet (needs the repo assets, so it
    /// is #[ignore]d in CI): the solver must place real-named jets at real
    /// longitudes on the real land mask.
    #[test]
    #[ignore = "requires assets/textures; run with --ignored from the repo root"]
    fn real_world_jets_sit_at_real_places() {
        let bytes = std::fs::read("assets/textures/earth_topography_3840.jpg")
            .expect("run from the repo root");
        let img = image::load_from_memory(&bytes).unwrap().to_rgba8();
        let (w, h) = img.dimensions();
        let mask = LandMask::from_rgba(w, h, img.as_raw());
        let c = Climate::new(mask);
        let v_at = |lat: f32, lon: f32| {
            let i = ((90.0 - lat) / 180.0 * CGH as f32) as usize;
            let j = ((lon.rem_euclid(360.0)) / 360.0 * CGW as f32) as usize;
            c.vo[i * CGW + j]
        };
        // Max over a lat/lon box: on the 1.4-degree grid a narrow jet core
        // can sit a couple of cells off the textbook coordinate.
        let v_max = |lat0: f32, lat1: f32, lon0: f32, lon1: f32| {
            let mut m = -1.0f32;
            let mut lat = lat0;
            while lat <= lat1 {
                let mut lon = lon0;
                while lon <= lon1 {
                    m = m.max(v_at(lat, lon));
                    lon += 2.0;
                }
                lat += 2.0;
            }
            m
        };
        // Gulf Stream (35N, 75W) and Kuroshio (32N, 140E): poleward jets.
        assert!(
            v_max(33.0, 37.0, 278.0, 292.0) > 0.05,
            "Gulf Stream: {:.3}",
            v_max(33.0, 37.0, 278.0, 292.0)
        );
        assert!(
            v_max(28.0, 34.0, 132.0, 148.0) > 0.05,
            "Kuroshio: {:.3}",
            v_max(28.0, 34.0, 132.0, 148.0)
        );
        // California (~32N offshore): NH eastern boundary, equatorward =
        // southward. Peru (~15S, 80W): SH subtropics flow equatorward =
        // NORTHWARD, so v > 0 there (same Sverdrup interior the gyre tests
        // pin down). Brazil (30S, 50W) SH western boundary: poleward =
        // southward. Boxes stay clear of the coast mask.
        let v_min = |lat0: f32, lat1: f32, lon0: f32, lon1: f32| {
            let mut m = 1.0f32;
            let mut lat = lat0;
            while lat <= lat1 {
                let mut lon = lon0;
                while lon <= lon1 {
                    m = m.min(v_at(lat, lon));
                    lon += 2.0;
                }
                lat += 2.0;
            }
            m
        };
        assert!(
            v_min(28.0, 36.0, 228.0, 242.0) < -0.01,
            "California: {:.3}",
            v_min(28.0, 36.0, 228.0, 242.0)
        );
        assert!(
            v_max(-20.0, -10.0, 274.0, 286.0) > 0.008,
            "Peru (equatorward/N): {:.3}",
            v_max(-20.0, -10.0, 274.0, 286.0)
        );
        assert!(
            v_min(-34.0, -26.0, 306.0, 320.0) < -0.05,
            "Brazil: {:.3}",
            v_min(-34.0, -26.0, 306.0, 320.0)
        );
    }
}
