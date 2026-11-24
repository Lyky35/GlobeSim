// ---------------------------------------------------------------------------
// A toy atmospheric circulation, simulated on the CPU every frame.
//
// Cloud water density c(lat, lon) in [0, 1] on a 256 x 128 lat/lon grid:
//
//   advection  surface wind field u(lat), v(lat) — trades, westerlies and a
//              Hadley/Ferrel-style meridional cell — moves the cloud field
//              around with a semi-Lagrangian (backtrace + bilinear sample)
//              step, which is stable at any timestep.
//   convection clouds grow where the Sun is high (insolation^1.5, moisture
//              biased to the tropics) and decay everywhere; the subsolar point
//              sweeps westward once per sidereal day, which is what organises
//              the growth into helical cloud bands.
//
// The GPU comparison mode (main.rs) just drifts a frozen texture with the
// trade-wind speed via one uniform — same trick as the GPU tide path.
// ---------------------------------------------------------------------------

/// Longitude / latitude resolution of the cloud grid.
pub const GRID_W: usize = 256;
pub const GRID_H: usize = 128;

const EARTH_R_M: f32 = 6.371e6;
const DAY_S: f32 = 86400.0;

/// Cloud cover grows with a ~1.2-day e-folding under the tropics...
const GROW_PER_S: f32 = 1.0 / (1.2 * DAY_S);
/// ...and decays with a ~6-day e-folding (persisting cirrus on the nightside).
const DECAY_PER_S: f32 = 1.0 / (6.0 * DAY_S);

pub struct CloudField {
    field: Vec<f32>,
    next: Vec<f32>,
    pixels: Vec<u8>,
    /// Mean cloud fraction (HUD stat).
    pub coverage: f32,
    /// ms spent in the CPU circulation pass last frame.
    pub last_ms: f64,
}

/// Deterministic 32-bit hash for the initial noise seed.
fn hash2(i: usize, j: usize) -> f32 {
    let mut x = (i * 374761393 + j * 668265263) as u32;
    x ^= x >> 13;
    x = x.wrapping_mul(1274126177);
    x ^= x >> 16;
    x as f32 / u32::MAX as f32
}

impl CloudField {
    pub fn new() -> Self {
        let mut field = vec![0.0; GRID_W * GRID_H];
        for i in 0..GRID_H {
            let lat = 90.0 - (i as f32 + 0.5) * 180.0 / GRID_H as f32;
            let moist = 0.35 + 0.65 * (-(lat / 22.0).powi(2)).exp();
            for j in 0..GRID_W {
                field[i * GRID_W + j] = (0.55 * moist + 0.25 * hash2(i, j)).clamp(0.02, 0.95);
            }
        }
        let coverage = field.iter().sum::<f32>() / field.len() as f32;
        Self {
            field,
            next: vec![0.0; GRID_W * GRID_H],
            pixels: vec![0; GRID_W * GRID_H * 4],
            coverage,
            last_ms: 0.0,
        }
    }

    // Test-only accessors into the raw grid.
    #[allow(dead_code)]
    pub fn at(&self, i: usize, j: usize) -> f32 {
        self.field[i * GRID_W + j]
    }

    #[allow(dead_code)]
    pub fn set_at(&mut self, i: usize, j: usize, v: f32) {
        self.field[i * GRID_W + j] = v;
    }

    pub fn cell_lat(&self, i: usize) -> f32 {
        90.0 - (i as f32 + 0.5) * 180.0 / GRID_H as f32
    }

    pub fn cell_lon(&self, j: usize) -> f32 {
        (j as f32 + 0.5) * 360.0 / GRID_W as f32
    }

    /// Zonal (u, east+) and meridional (v, north+) surface winds in m/s:
    /// trade easterlies near ±15°, mid-latitude westerlies near ±47°, plus a
    /// Hadley/Ferrel meridional cell v = -H sin(2·lat).
    pub fn wind(&self, lat_deg: f32) -> (f32, f32) {
        let g = lat_deg.to_radians();
        let band = |center_deg: f32, width_deg: f32, amp: f32| {
            let d = (g - center_deg.to_radians()) / width_deg.to_radians();
            amp * (-d * d).exp()
        };
        let u = band(15.0, 14.0, -8.0)
            + band(-15.0, 14.0, -8.0)
            + band(47.0, 18.0, 20.0)
            + band(-47.0, 18.0, 20.0);
        let v = -3.0 * (2.0 * g).sin();
        (u, v)
    }

    /// Sample the field at arbitrary (lat_deg, lon_deg): bilinear in the
    /// interior, wrapping in longitude, clamping in latitude.
    fn sample(&self, lat_deg: f32, lon_deg: f32) -> f32 {
        let lat_deg = lat_deg.clamp(-89.0, 89.0);
        let x = (lon_deg.rem_euclid(360.0) / 360.0 * GRID_W as f32) - 0.5;
        let y = ((90.0 - lat_deg) / 180.0 * GRID_H as f32 - 0.5).clamp(0.0, GRID_H as f32 - 1.0);
        let j0 = x.floor() as isize;
        let i0 = y.floor() as usize;
        let i1 = (i0 + 1).min(GRID_H - 1);
        let j0w = j0.rem_euclid(GRID_W as isize) as usize;
        let j1w = (j0 + 1).rem_euclid(GRID_W as isize) as usize;
        let fx = x - x.floor();
        let fy = (y - y.floor()).clamp(0.0, 1.0);
        let a = self.field[i0 * GRID_W + j0w];
        let b = self.field[i0 * GRID_W + j1w];
        let c = self.field[i1 * GRID_W + j0w];
        let d = self.field[i1 * GRID_W + j1w];
        (a * (1.0 - fx) + b * fx) * (1.0 - fy) + (c * (1.0 - fx) + d * fx) * fy
    }

    /// Semi-Lagrangian advection by the wind field alone (dt in sim seconds).
    pub fn step_advect(&mut self, dt: f32) {
        let mass0: f32 = self.field.iter().sum();
        for i in 0..GRID_H {
            let lat = self.cell_lat(i);
            let (u, v) = self.wind(lat);
            let dlat = (v / EARTH_R_M * dt).to_degrees();
            let dlon = (u / EARTH_R_M * dt).to_degrees();
            for j in 0..GRID_W {
                let lon = self.cell_lon(j);
                // Backtrace along the flow over the last dt.
                self.next[i * GRID_W + j] = self.sample(lat - dlat, lon - dlon);
            }
        }
        std::mem::swap(&mut self.field, &mut self.next);

        // Global mass fixer: the 2-D wind field is not divergence-free (the
        // Hadley cell converges at the ITCZ), and backtrace-only advection
        // implements the transport derivative -v·∇c while dropping the
        // compression term -c∇·v, which shows up as spurious mass drift.
        // Rescaling restores the conserved total (standard GCM trick).
        let mass1: f32 = self.field.iter().sum();
        if mass1 > 1e-6 {
            let scale = mass0 / mass1;
            if (scale - 1.0).abs() > 1e-6 {
                for c in self.field.iter_mut() {
                    *c = (*c * scale).min(1.0);
                }
            }
        }
    }

    /// Growth/decay from insolation. `sun` is the Sun direction expressed in
    /// the *ground* frame (unit vector to the subsolar point).
    pub fn step_convection(&mut self, dt: f32, sun: macroquad::prelude::Vec3) {
        let lat_s = sun.y.asin().to_degrees();
        let lon_s = sun.x.atan2(sun.z).to_degrees();
        for i in 0..GRID_H {
            let lat = self.cell_lat(i);
            let (sl, cl) = (lat.to_radians().sin(), lat.to_radians().cos());
            let moist = 0.35 + 0.65 * (-(lat / 22.0).powi(2)).exp();
            for j in 0..GRID_W {
                let lon = self.cell_lon(j);
                let cos_zen = sl * lat_s.to_radians().sin()
                    + cl * lat_s.to_radians().cos() * (lon - lon_s).to_radians().cos();
                let ins = cos_zen.max(0.0).powf(1.5);
                let c = self.field[i * GRID_W + j];
                let dc = GROW_PER_S * (1.0 - c) * ins * moist - DECAY_PER_S * c;
                self.field[i * GRID_W + j] = (c + dc * dt).clamp(0.0, 1.0);
            }
        }
    }

    /// Full CPU circulation step; split into advection + convection.
    pub fn step(&mut self, dt: f32, sun_ground: macroquad::prelude::Vec3) {
        let start = std::time::Instant::now();
        self.step_advect(dt);
        self.step_convection(dt, sun_ground);
        let sum: f32 = self.field.iter().sum();
        self.coverage = sum / self.field.len() as f32;
        self.last_ms = start.elapsed().as_secs_f64() * 1000.0;
    }

    /// White pixels with cloud fraction as alpha, row 0 = 90 N (matches the
    /// sphere UV layout).
    pub fn rgba8(&mut self) -> &mut Vec<u8> {
        for (k, c) in self.field.iter().enumerate() {
            // Slightly thinner cloud edges read better than a hard ramp.
            let a = (c.powf(0.85) * 235.0).min(235.0) as u8;
            let p = &mut self.pixels[k * 4..k * 4 + 4];
            p.copy_from_slice(&[255, 255, 255, a]);
        }
        &mut self.pixels
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wind_belts_have_real_signs() {
        let f = CloudField::new();
        assert!(f.wind(15.0).0 < -5.0, "trades blow west in the tropics");
        assert!(f.wind(-15.0).0 < -5.0, "trades blow west in the tropics");
        assert!(f.wind(47.0).0 > 10.0, "westerlies in the mid-latitudes");
        assert!(f.wind(-47.0).0 > 10.0, "westerlies in the mid-latitudes");
        assert!(f.wind(15.0).1 < 0.0, "Hadley cell returns equatorward");
        assert!(f.wind(-15.0).1 > 0.0, "Hadley cell returns equatorward");
    }

    #[test]
    fn blob_advects_west_and_mass_is_conserved() {
        let mut f = CloudField::new();
        f.field.iter_mut().for_each(|c| *c = 0.0);
        // Gaussian-ish blob at 15 N, 10 E.
        for i in 0..GRID_H {
            for j in 0..GRID_W {
                let lat = f.cell_lat(i);
                let lon = f.cell_lon(j);
                let d2 = ((lat - 15.0) / 6.0).powi(2) + ((lon - 10.0) / 6.0).powi(2);
                f.set_at(i, j, (-d2).exp());
            }
        }
        let mass0: f32 = f.field.iter().sum();
        // 3 days of advection: trades (~8 m/s west) move it ~18 deg of longitude.
        for _ in 0..6 {
            f.step_advect(43200.0);
        }
        let mass1: f32 = f.field.iter().sum();
        assert!(
            (mass1 / mass0 - 1.0).abs() < 0.03,
            "advection should conserve mass: {mass0} -> {mass1}"
        );

        let mut wsum = 0.0;
        let mut wlon = 0.0;
        for i in 0..GRID_H {
            for j in 0..GRID_W {
                let c = f.at(i, j);
                if c > 0.01 {
                    // Unwrap longitude relative to the original blob at 10 E.
                    let mut lon = f.cell_lon(j);
                    if lon > 180.0 {
                        lon -= 360.0;
                    }
                    wsum += c;
                    wlon += c * lon;
                }
            }
        }
        let shift = wlon / wsum - 10.0;
        assert!(
            shift < -8.0 && shift > -40.0,
            "blob should drift ~18 deg west, moved {shift:.1}"
        );
    }

    #[test]
    fn convection_grows_on_dayside_and_stays_bounded() {
        let mut f = CloudField::new();
        f.field.iter_mut().for_each(|c| *c = 0.05);
        // Sun straight over (0 N, 0 E); 10 days of growth/decay.
        for _ in 0..10 {
            f.step_convection(86400.0, macroquad::prelude::vec3(0.0, 0.0, 1.0));
        }
        let day = f.at(GRID_H / 2, 0); // lat ~0, lon ~0.25: subsolar
        let night = f.at(GRID_H / 2, GRID_W / 2); // lon ~180: midnight
        assert!(day > 0.3, "subsolar cloud should grow, got {day}");
        assert!(night < day, "nightside should be cloudier-poor, {night} vs {day}");
        assert!(f.field.iter().all(|c| *c >= 0.0 && *c <= 1.0));
    }
}
