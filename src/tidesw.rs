use crate::cities;
use crate::climate::{CGH, CGW};
use crate::dem::Dem;
use macroquad::prelude::{Mat4, Vec3};

// ---------------------------------------------------------------------------
// Linear shallow-water tides on the sphere: the *dynamic* ocean response to
// the same tidal potential the equilibrium map draws. Forward-backward-ish
// RK2 on an A-grid over the climate lattice, driven by the analytic tidal
// potential, over real ETOPO bathymetry with closed land boundaries and
// modest bottom friction. Run long enough it develops genuine co-tidal
// behaviour - amphidromes, wave setup against continents - instead of the
// frozen equilibrium bulge.
//
// Grid latitudes beyond +/-77.5 deg are capped out (treated as land): the
// zonal cell shrinks like cos(lat), and holding the gravity-wave CFL at a
// 120 s timestep down to the poles would cost the whole polar band. Ice
// tides are not the point.
// ---------------------------------------------------------------------------

pub const DW_DT: f32 = 120.0; // s; CFL ~180 s at the cap latitude
pub const DW_FRICTION: f32 = 2.5e-5; // 1/s; transient spin-down ~11 h
const OMEGA: f32 = 7.292_115_9e-5; // rad/s
const G: f32 = 9.81;
const R: f32 = 6.371e6;

pub struct ShallowTide {
    ocean: Vec<bool>,
    h: Vec<f32>,       // resting depth, m (0 on land)
    pub eta: Vec<f32>, // surface elevation, m
    u: Vec<f32>,       // zonal velocity, m/s
    v: Vec<f32>,       // meridional velocity, m/s
    // scratch for RK2 stages
    u0: Vec<f32>,
    v0: Vec<f32>,
    du: Vec<f32>,
    dv: Vec<f32>,
    d_eta: Vec<f32>,
    sub_acc: f32, // leftover sim seconds for the 120 s clock
    pub steps: u64,
}

fn cell_ocean(dem: &Dem, lat: f32, lon: f32) -> bool {
    lat.abs() < 77.5 && dem.height_m(lat, lon) < 0.0
}

impl ShallowTide {
    pub fn new(dem: &Dem) -> Self {
        let n = CGW * CGH;
        let mut ocean = vec![false; n];
        let mut h = vec![0.0f32; n];
        for i in 0..CGH {
            let lat = 90.0 - (i as f32 + 0.5) * 180.0 / CGH as f32;
            for j in 0..CGW {
                let lon = (j as f32 + 0.5) * 360.0 / CGW as f32;
                let k = i * CGW + j;
                if cell_ocean(dem, lat, lon) {
                    ocean[k] = true;
                    h[k] = (-dem.height_m(lat, lon)).clamp(30.0, 5200.0);
                }
            }
        }
        Self {
            ocean,
            h,
            eta: vec![0.0; n],
            u: vec![0.0; n],
            v: vec![0.0; n],
            u0: vec![0.0; n],
            v0: vec![0.0; n],
            du: vec![0.0; n],
            dv: vec![0.0; n],
            d_eta: vec![0.0; n],
            sub_acc: 0.0,
            steps: 0,
        }
    }

    /// Catch the 120 s solver clock up with sim time. At astronomic speeds
    /// the catch-up is capped at 60 steps/frame, so the tide degrades to
    /// "slower than sim time" instead of melting.
    pub fn advance(&mut self, dt_sim: f32, model: Mat4, moon: Vec3, sun: Vec3) {
        self.sub_acc += dt_sim.max(0.0);
        let n = ((self.sub_acc / DW_DT) as usize).min(60);
        self.sub_acc -= n as f32 * DW_DT;
        for _ in 0..n {
            self.step(DW_DT, model, moon, sun);
        }
    }

    /// One RK2 (Heun) step. Forcing: the analytic equilibrium-tide
    /// elevation field zeta(lat,lon); the tidal acceleration is -g grad zeta.
    pub fn step(&mut self, dt: f32, model: Mat4, moon: Vec3, sun: Vec3) {
        // stage 1: slopes of the current state
        Self::slopes(
            &self.eta,
            &self.u,
            &self.v,
            &self.ocean,
            &self.h,
            &mut self.du,
            &mut self.dv,
            &mut self.d_eta,
            model,
            moon,
            sun,
        );
        self.u0.clone_from_slice(&self.u);
        self.v0.clone_from_slice(&self.v);
        let mut eta = self.eta.clone();
        for k in 0..eta.len() {
            if self.ocean[k] {
                self.u[k] += dt * self.du[k];
                self.v[k] += dt * self.dv[k];
                eta[k] += dt * self.d_eta[k];
            }
        }
        // stage 2: slopes of the Euler predictor, average for the corrector
        Self::slopes(
            &eta,
            &self.u,
            &self.v,
            &self.ocean,
            &self.h,
            &mut self.du,
            &mut self.dv,
            &mut self.d_eta,
            model,
            moon,
            sun,
        );
        for k in 0..eta.len() {
            if self.ocean[k] {
                self.u[k] = 0.5 * (self.u0[k] + self.u[k] + dt * self.du[k]);
                self.v[k] = 0.5 * (self.v0[k] + self.v[k] + dt * self.dv[k]);
                eta[k] = 0.5 * (self.eta[k] + eta[k] + dt * self.d_eta[k]);
            } else {
                self.u[k] = 0.0;
                self.v[k] = 0.0;
                eta[k] = 0.0;
            }
        }
        self.eta = eta;

        // grid-scale sponge (kills sawtooth the RK2 can't damp)
        let n = self.eta.clone();
        for i in 1..CGH - 1 {
            for j in 0..CGW {
                let k = i * CGW + j;
                if self.ocean[k] {
                    let nb = (n[i * CGW + (j + 1) % CGW]
                        + n[i * CGW + (j + CGW - 1) % CGW]
                        + n[(i + 1) * CGW + j]
                        + n[(i - 1) * CGW + j])
                        * 0.25;
                    self.eta[k] += 0.015 * (nb - n[k]);
                }
            }
        }

        // global mass fixer: tides redistribute water, they don't create it
        let mut sum = 0.0f64;
        let mut nsea = 0usize;
        for (k, &e) in self.eta.iter().enumerate() {
            if self.ocean[k] {
                sum += e as f64;
                nsea += 1;
            }
        }
        let mean = (sum / nsea as f64) as f32;
        for (k, e) in self.eta.iter_mut().enumerate() {
            if self.ocean[k] {
                *e -= mean;
            }
        }

        self.steps += 1;
    }

    /// Write time derivatives of (eta, u, v) into d_eta, du, dv. Row index
    /// grows southward: im is the north neighbour, ip the south one.
    fn slopes(
        eta: &[f32],
        u: &[f32],
        v: &[f32],
        ocean: &[bool],
        h: &[f32],
        du: &mut [f32],
        dv: &mut [f32],
        d_eta: &mut [f32],
        model: Mat4,
        moon: Vec3,
        sun: Vec3,
    ) {
        let dlon = std::f32::consts::TAU / CGW as f32;
        let dlat = std::f32::consts::PI / CGH as f32;
        let dlat_deg = 180.0 / CGH as f32;
        for i in 0..CGH {
            let lat = 90.0 - (i as f32 + 0.5) * 180.0 / CGH as f32;
            let rlat = lat.to_radians();
            let f = 2.0 * OMEGA * rlat.sin();
            let cl = rlat.cos().max(0.05);
            for j in 0..CGW {
                let k = i * CGW + j;
                if !ocean[k] {
                    du[k] = 0.0;
                    dv[k] = 0.0;
                    d_eta[k] = 0.0;
                    continue;
                }
                let lon = (j as f32 + 0.5) * 360.0 / CGW as f32;
                let dl = 360.0 / CGW as f32;
                let (jm, jp) = ((j + CGW - 1) % CGW, (j + 1) % CGW);
                let (im, ip) = (i.saturating_sub(1), (i + 1).min(CGH - 1));
                // total surface (own + tidal potential / g) at each face
                let e_w =
                    face_eta(eta, ocean, i, jm, k) + tidal_at(lat, lon - dl, model, moon, sun);
                let e_e =
                    face_eta(eta, ocean, i, jp, k) + tidal_at(lat, lon + dl, model, moon, sun);
                let e_n = face_eta(eta, ocean, im, j, k)
                    + tidal_at(lat + dlat_deg, lon, model, moon, sun);
                let e_s = face_eta(eta, ocean, ip, j, k)
                    + tidal_at(lat - dlat_deg, lon, model, moon, sun);
                let gx = (e_e - e_w) / (2.0 * R * cl * dlon);
                let gy = (e_n - e_s) / (2.0 * R * dlat);
                du[k] = f * v[k] - G * gx - DW_FRICTION * u[k];
                dv[k] = -f * u[k] - G * gy - DW_FRICTION * v[k];

                // continuity: div(H u) in spherical coords; land faces carry
                // no flux because H and u are both zero there
                let hu = |kk: usize| h[kk] * u[kk];
                let hv = |kk: usize| h[kk] * v[kk];
                let fx_e = 0.5 * (hu(k) + hu(i * CGW + jp));
                let fx_w = 0.5 * (hu(k) + hu(i * CGW + jm));
                let fy_n = 0.5 * (hv(k) + hv(im * CGW + j));
                let fy_s = 0.5 * (hv(k) + hv(ip * CGW + j));
                let dfx = (fx_e - fx_w) / (2.0 * R * cl * dlon);
                let dfy = (fy_n - fy_s) / (2.0 * R * dlat);
                d_eta[k] = -(dfx + dfy);
            }
        }
    }

    /// Largest |eta| over the ocean (panel caption + test sanity).
    pub fn max_abs(&self) -> f32 {
        let mut m = 0.0f32;
        for (k, &e) in self.eta.iter().enumerate() {
            if self.ocean[k] {
                m = m.max(e.abs());
            }
        }
        m
    }

    /// Mean kinetic + potential energy density (tests).
    #[cfg(test)]
    pub fn energy(&self) -> f64 {
        let mut e = 0.0f64;
        for k in 0..self.eta.len() {
            if self.ocean[k] {
                e += (self.h[k] as f64) * (self.u[k] * self.u[k] + self.v[k] * self.v[k]) as f64
                    + G as f64 * (self.eta[k] * self.eta[k]) as f64;
            }
        }
        e
    }

    pub fn eta_at(&self, lat: f32, lon: f32) -> Option<f32> {
        if lat.abs() > 88.0 {
            return None;
        }
        let i = (((90.0 - lat) / 180.0) * CGH as f32) as usize;
        let j = (lon.rem_euclid(360.0) / 360.0 * CGW as f32) as usize;
        let k = i * CGW + j;
        self.ocean[k].then(|| self.eta[k])
    }
}

/// Neighbour eta for a pressure-gradient face: land reflects (zero normal
/// gradient), so borrow the ocean cell's own value.
fn face_eta(eta: &[f32], ocean: &[bool], i: usize, j: usize, own: usize) -> f32 {
    let k = i * CGW + j;
    if ocean[k] {
        eta[k]
    } else {
        eta[own]
    }
}

/// Tidal potential expressed as an equivalent water elevation (m).
fn tidal_at(lat: f32, lon: f32, model: Mat4, moon: Vec3, sun: Vec3) -> f32 {
    let d = model.transform_vector3(cities::ground_dir(lat, lon));
    cities::tide_m(d, moon, sun)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ephemeris;

    fn forced_spinup(s: &mut ShallowTide, days: u64, start_day: u64) {
        for n in 0..(days * 86400 / DW_DT as u64) {
            let t = (start_day * 86400) as f64 + n as f64 * DW_DT as f64;
            let m = cities::earth_model_at(t);
            s.step(DW_DT, m, ephemeris::moon_dir(t), ephemeris::sun_dir(t));
        }
    }

    #[test]
    fn real_earth_forced_for_four_days_is_stable_and_lively() {
        let dem = Dem::load("assets/textures/etopo_dem.bin");
        let mut s = ShallowTide::new(&dem);
        forced_spinup(&mut s, 4, 0);
        let bad = s
            .eta
            .iter()
            .enumerate()
            .filter(|(k, e)| s.ocean[*k] && !e.is_finite())
            .count();
        assert_eq!(bad, 0, "solver went non-finite");
        let max_abs = s.max_abs();
        assert!(max_abs > 0.05, "no tide developed: {max_abs} m");
        assert!(max_abs < 4.0, "solver overheating: {max_abs} m");
    }

    #[test]
    fn the_forced_ocean_oscillates_at_semidiurnal_rate() {
        let dem = Dem::load("assets/textures/etopo_dem.bin");
        let mut s = ShallowTide::new(&dem);
        forced_spinup(&mut s, 4, 0);
        // Sample the strongest open-ocean cell (avoid amphidromes: those
        // nodes are the point of the solver, not a failure).
        let mut best = (0usize, 0.0f32);
        for (k, &e) in s.eta.iter().enumerate() {
            if s.ocean[k] && e.abs() > best.1 {
                best = (k, e.abs());
            }
        }
        assert!(best.1 > 0.05, "no cell oscillates");
        let mut prev = s.eta[best.0];
        let mut crossings = 0;
        for n in 0..(24 * 3600 / DW_DT as u64) {
            let t = (4 * 86400) as f64 + n as f64 * DW_DT as f64;
            let m = cities::earth_model_at(t);
            s.step(DW_DT, m, ephemeris::moon_dir(t), ephemeris::sun_dir(t));
            let e = s.eta[best.0];
            if prev * e < 0.0 {
                crossings += 1;
            }
            prev = e;
        }
        assert!(
            (2..=4).contains(&crossings),
            "semidiurnal zero crossings in 24 h: {crossings}"
        );
    }

    #[test]
    fn friction_sloshes_energy_down_not_up() {
        let dem = Dem::load("assets/textures/etopo_dem.bin");
        let mut s = ShallowTide::new(&dem);
        // initial condition: a frozen equilibrium-style bulge, zero forcing
        let m = cities::earth_model_at(0.0);
        for i in 0..CGH {
            let lat = 90.0 - (i as f32 + 0.5) * 180.0 / CGH as f32;
            for j in 0..CGW {
                let k = i * CGW + j;
                if s.ocean[k] {
                    let lon = (j as f32 + 0.5) * 360.0 / CGW as f32;
                    s.eta[k] = tidal_at(lat, lon, m, Vec3::Z, Vec3::X);
                }
            }
        }
        let e0 = s.energy();
        for _ in 0..1000 {
            s.step(DW_DT, m, Vec3::ZERO, Vec3::ZERO);
        }
        let e1 = s.energy();
        assert!(e1.is_finite() && e1 < e0, "energy {e0} -> {e1}");
        assert!(e1 > 0.0, "friction should damp, not annihilate");
    }

    #[test]
    fn land_and_polar_caps_are_closed_boundaries() {
        let dem = Dem::load("assets/textures/etopo_dem.bin");
        let mut t = ShallowTide::new(&dem);
        assert!(t.ocean.iter().filter(|o| **o).count() > CGW * CGH / 3);
        let i0 = 2; // lat ~87.8 N: inside the polar cap
        for j in 0..CGW {
            assert!(!t.ocean[i0 * CGW + j]);
        }
        let m = cities::earth_model_at(0.0);
        for _ in 0..300 {
            t.step(DW_DT, m, ephemeris::moon_dir(0.0), ephemeris::sun_dir(0.0));
        }
        assert_eq!(t.u[i0 * CGW + 10], 0.0, "flow through the ice cap");
    }
}
