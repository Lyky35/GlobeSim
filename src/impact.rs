// ---------------------------------------------------------------------------
// Asteroid impact events — an Apophis-class strike (340 m rock, the famous
// ~1,200 megaton case) and a Chicxulub-class 10 km impactor for scale.
//
// Back-of-the-envelope physics, single source of truth for both the CPU and
// GPU rendering paths:
//
//   energy       E = ½ m v²
//   crater       transient diameter D_tc = 1.161·(ρi/ρt)^⅓·L^0.78·v^0.44·g^-0.22
//                (Collins et al. 2005 gravity-regime scaling); final ≈ 1.3·D_tc
//   seismic      M_w = (log10(E_rad) − 4.8) / 1.5  (Kanamori), radiated
//                seismic energy ~1e-4 of impact energy
//   tsunami      deep-ocean speed c = √(g·h), h ≈ 4 km → ~198 m/s; the water
//                punched out of the crater refalls as a Gaussian crest ring
//                with amplitude ~√(2ηE / ρgA_crater) near the source and
//                cylindrical-ish spreading ~1/(1+r/4000 km) afterwards
//   atmosphere   shock ring runs out at ~the speed of sound; ejecta plume and
//                global dust loading scale with energy — Apophis is a
//                *regional* disaster (dust is optically irrelevant globally),
//                Chicxulub-class is an impact winter.
// ---------------------------------------------------------------------------

use macroquad::prelude::Vec3;

pub struct Impactor {
    pub name: &'static str,
    pub diameter_m: f64,
    pub mass_kg: f64,
    pub vel_m_s: f64,
}

/// 2004-era Apophis parameters: ~340 m rock, ~6.2e10 kg, 12.6 km/s.
pub const APHOPIS: Impactor = Impactor {
    name: "Apophis",
    diameter_m: 340.0,
    mass_kg: 6.2e10,
    vel_m_s: 12.6e3,
};

/// The K-Pg killball for scale: 10 km, ~1.6e15 kg, 20 km/s.
pub const CHICXULUB: Impactor = Impactor {
    name: "Chicxulub",
    diameter_m: 10_000.0,
    mass_kg: 1.57e15,
    vel_m_s: 20.0e3,
};

pub const EARTH_R_M: f64 = 6.371e6;
/// Deep-ocean long-wave speed √(g·h) with h = 4 km.
pub const TSUNAMI_C_M_S: f64 = 198.0;
/// S/Rayleigh surface-wave speed for the seismic ring.
pub const SEISMIC_V_M_S: f64 = 4.5e3;
/// Impact airblast ring races ahead at ~Mach 1 in the acoustic limit.
pub const SHOCK_V_M_S: f64 = 340.0;
/// Dust e-folding: ~3 years (fine soot/硫酸 aerosol residence time).
pub const DUST_TAU_S: f64 = 9.5e7;

const MT_J: f64 = 4.184e15;

pub struct ImpactStats {
    pub energy_j: f64,
    pub energy_mt: f64,
    pub crater_km: f64,
    pub mw: f64,
    pub tsunami_m: f64,
    pub plume_km: f64,
    pub fireball_km: f64,
    pub dust0: f32,
}

pub fn stats(r: &Impactor) -> ImpactStats {
    let e = 0.5 * r.mass_kg * r.vel_m_s * r.vel_m_s;
    let mt = e / MT_J;
    // Collins et al. (2005) gravity-regime transient crater, rock into rock.
    let d_tc = 1.161
        * (3000.0f64 / 2500.0).cbrt()
        * r.diameter_m.powf(0.78)
        * r.vel_m_s.powf(0.44)
        * 9.81f64.powf(-0.22);
    let crater_km = 1.3 * d_tc / 1000.0;
    let e_seis = 1e-4 * e;
    let mw = (e_seis.log10() - 4.8) / 1.5;
    // Water column punched out of the transient crater falls back as a wave.
    let a_cr = std::f64::consts::PI * (d_tc / 2.0) * (d_tc / 2.0);
    let tsunami_m = ((2.0 * 0.1 * e / (1025.0 * 9.81 * a_cr)).sqrt()).min(4000.0);
    let plume_km = (3.7 * mt.powf(0.25)).min(120.0);
    // Thermal radius for ~25 kJ/m^2 third-degree-burn fluence (1e-4 radiative).
    let fireball_km = (1e-4 * e / (4.0 * std::f64::consts::PI * 2.5e4)).sqrt() / 1000.0;
    let dust0 = (mt / 1e5).clamp(0.01, 0.9) as f32;
    ImpactStats {
        energy_j: e,
        energy_mt: mt,
        crater_km,
        mw,
        tsunami_m,
        plume_km,
        fireball_km,
        dust0,
    }
}

/// Tsunami state for one frame: `dir` is the impact point in the *world*
/// frame (recomputed as the ground rotates), `front` the crest's angular
/// radius in radians, `a0` the current crest amplitude in scene units.
pub struct WaveState {
    pub dir: Vec3,
    pub front: f32,
    pub a0: f32,
    pub width: f32,
}

/// Gaussian crest ring at angular distance psi from the impact point.
pub fn wave_h(d: Vec3, w: &WaveState) -> f32 {
    let psi = d.dot(w.dir).clamp(-1.0, 1.0).acos();
    let x = (psi - w.front) / w.width;
    w.a0 * (-x * x).exp()
}

pub struct Impact {
    pub rock: Impactor,
    pub stats: ImpactStats,
    /// Impact point in the *ground* frame (fixed to the rotating Earth).
    pub ground_dir: Vec3,
    /// Simulation seconds since impact.
    pub age_s: f64,
}

impl Impact {
    pub fn trigger(rock: Impactor, ground_dir: Vec3) -> Impact {
        let stats = stats(&rock);
        Impact {
            rock,
            stats,
            ground_dir,
            age_s: 0.0,
        }
    }

    pub fn step(&mut self, dt_sim: f64) {
        if dt_sim > 0.0 {
            self.age_s += dt_sim;
        }
    }

    /// Scene-unit wave amplitude today: source amplitude scaled to scene
    /// (the same "exaggerated but proportional" convention as the tide),
    /// reduced by cylindrical spreading and a multi-day viscous fade.
    fn a0_scene(&self, front_m: f64) -> f32 {
        let src = 0.013 * (self.stats.tsunami_m / 2500.0).min(1.0);
        let spread = 1.0 / (1.0 + front_m / 4.0e6);
        let fade = (-self.age_s / 1.5e6).exp();
        (src * spread * fade) as f32
    }

    pub fn wave(&self, ground_dir_world: Vec3) -> WaveState {
        let front_m = TSUNAMI_C_M_S * self.age_s;
        WaveState {
            dir: ground_dir_world,
            front: (front_m / EARTH_R_M) as f32,
            a0: self.a0_scene(front_m),
            width: 0.04, // ~250 km crest width
        }
    }

    /// Global dust loading 0..1, decaying with a ~3-year e-folding.
    pub fn dust(&self) -> f32 {
        self.stats.dust0 * (-self.age_s / DUST_TAU_S).exp() as f32
    }

    /// Angular radii of the seismic (ground) and atmospheric shock rings.
    pub fn seismic_psi(&self) -> f32 {
        (SEISMIC_V_M_S * self.age_s / EARTH_R_M) as f32
    }
    pub fn shock_psi(&self) -> f32 {
        (SHOCK_V_M_S * self.age_s / EARTH_R_M) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use macroquad::prelude::vec3;

    #[test]
    fn apophis_energy_crater_and_magnitude() {
        let s = stats(&APHOPIS);
        // 6.2e10 kg at 12.6 km/s ~ 4.9e18 J ~ 1,200 Mt.
        assert!(
            s.energy_j > 4.0e18 && s.energy_j < 6.0e18,
            "E = {}",
            s.energy_j
        );
        assert!(
            s.energy_mt > 900.0 && s.energy_mt < 1400.0,
            "Mt = {}",
            s.energy_mt
        );
        // Literature: ~4-6 km final crater for Apophis on rock.
        assert!(
            s.crater_km > 3.0 && s.crater_km < 9.0,
            "crater = {}",
            s.crater_km
        );
        // Radiated seismic (1e-4 coupling) ~ M6.6-ish.
        assert!(s.mw > 6.0 && s.mw < 7.2, "Mw = {}", s.mw);
        // Ocean punch-out: km-scale near the crater, and Chicxulub is bigger.
        assert!(s.tsunami_m > 500.0 && s.tsunami_m <= 4000.0);
        let c = stats(&CHICXULUB);
        assert!(c.crater_km > s.crater_km * 10.0);
        assert!(
            c.dust0 > 0.5 && s.dust0 < 0.05,
            "Apophis must NOT be an impact winter"
        );
    }

    #[test]
    fn tsunami_ring_moves_at_wave_speed_and_fades() {
        let mut imp = Impact::trigger(APHOPIS, vec3(1.0, 0.0, 0.0));
        // Wave travels at ~198 m/s: after 2 h the crest sits at ~0.22 rad.
        let t = 7200.0;
        imp.step(t);
        let w = imp.wave(vec3(1.0, 0.0, 0.0));
        assert!((w.front as f64 - 198.0 * t / EARTH_R_M).abs() < 1e-6);
        assert!(w.a0 > 0.0);

        // Causality: nothing 60 deg away while the ring is still at ~13 deg.
        let far = wave_h(vec3(0.5f32.sqrt(), 0.5f32.sqrt(), 0.0).normalize(), &w);
        assert!(far < 1e-6, "wave should not have reached 60 deg, got {far}");

        // The crest peaks on the ring, not at the (calm) impact point.
        let on_ring = vec3(w.front.cos(), 0.0, w.front.sin()).normalize();
        let peak = wave_h(on_ring, &w);
        assert!(peak > 0.95 * w.a0, "ring sample {peak} vs a0 {}", w.a0);
        let centre = wave_h(vec3(1.0, 0.0, 0.0), &w);
        assert!(centre < 1e-6, "source point relaxes once the wave left");

        // Cylindrical spreading + fade shrink the crest over time.
        let early = imp.wave(vec3(1.0, 0.0, 0.0)); // age 2 h
        imp.step(58800.0); // total age 16.7 h
        let late = imp.wave(vec3(1.0, 0.0, 0.0));
        assert!(
            early.a0 > late.a0 * 1.5,
            "spreading should shrink the crest"
        );
    }

    #[test]
    fn dust_decays_with_three_year_efold() {
        let mut imp = Impact::trigger(CHICXULUB, vec3(1.0, 0.0, 0.0));
        let d0 = imp.dust();
        assert!(d0 > 0.5);
        imp.step(DUST_TAU_S);
        let d1 = imp.dust();
        assert!((d1 / d0 - 1.0 / std::f32::consts::E).abs() < 0.01);
    }
}
