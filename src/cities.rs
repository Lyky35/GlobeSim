use crate::constants::{AXIAL_TILT_DEG, EQUILIBRIUM_TIDE_M, SIDEREAL_DAY_S, SOLAR_TIDE_RATIO};
use crate::ephemeris;
use macroquad::prelude::*;

// ---------------------------------------------------------------------------
// City markers + local tide readouts. The astronomical (equilibrium) tide at
// a coastal city is a closed-form function of time - no simulation history
// needed - so the little HUD graph re-samples the whole tide curve from the
// ephemerides on every draw.
// ---------------------------------------------------------------------------

pub struct City {
    pub name: &'static str,
    pub lat: f32,
    pub lon: f32,
}

/// Coastal cities with famous or well-served tide gauges.
pub const CITIES: &[City] = &[
    City {
        name: "Halifax",
        lat: 44.65,
        lon: -63.58,
    }, // Fundy: world's biggest tidal range
    City {
        name: "London",
        lat: 51.51,
        lon: -0.13,
    },
    City {
        name: "Rotterdam",
        lat: 51.92,
        lon: 4.48,
    },
    City {
        name: "New York",
        lat: 40.71,
        lon: -74.01,
    },
    City {
        name: "Santos",
        lat: -23.96,
        lon: -46.33,
    },
    City {
        name: "Cape Town",
        lat: -33.92,
        lon: 18.42,
    },
    City {
        name: "Mumbai",
        lat: 19.08,
        lon: 72.88,
    },
    City {
        name: "Singapore",
        lat: 1.35,
        lon: 103.82,
    },
    City {
        name: "Tokyo",
        lat: 35.68,
        lon: 139.65,
    },
    City {
        name: "Sydney",
        lat: -33.87,
        lon: 151.21,
    },
];

/// Unit direction in the ground frame (same convention as SphereGrid::dir).
pub fn ground_dir(lat: f32, lon: f32) -> Vec3 {
    let (la, lo) = (lat.to_radians(), lon.to_radians());
    vec3(la.cos() * lo.sin(), la.sin(), la.cos() * lo.cos())
}

/// Earth orientation (tilt x spin) at sim time - one place for the formula
/// main.rs and the tide sampler share. The spin sign is negative because
/// `dir(lon)` and the ephemerides' `dir(λ) = (-sinλ, 0, cosλ)` measure
/// longitude clockwise about ecliptic north (+Y): a *positive* rotation_y
/// would carry the ground against the sky and give a 23.9 h solar day and a
/// 23.1 h lunar day. Negative keeps everything prograde, and the familiar
/// 24 h solar / 24.84 h lunar synodic days fall out of the ephemeris.
pub fn earth_model_at(t: f64) -> Mat4 {
    let spin = (std::f64::consts::TAU * t / SIDEREAL_DAY_S as f64) as f32;
    Mat4::from_rotation_z(AXIAL_TILT_DEG.to_radians()) * Mat4::from_rotation_y(-spin)
}

/// Equilibrium tide height (metres, real-world scale) at a world-frame
/// point, from both bulges. Same formula as the ocean shells.
pub fn tide_m(dir_w: Vec3, moon: Vec3, sun: Vec3) -> f32 {
    let c_m = dir_w.dot(moon);
    let c_s = dir_w.dot(sun);
    EQUILIBRIUM_TIDE_M * (1.5 * c_m * c_m - 0.5)
        + SOLAR_TIDE_RATIO * EQUILIBRIUM_TIDE_M * (1.5 * c_s * c_s - 0.5)
}

/// Tide height at a city at sim time t.
pub fn tide_at(city: &City, t: f64) -> f32 {
    let d = earth_model_at(t).transform_vector3(ground_dir(city.lat, city.lon));
    tide_m(d, ephemeris::moon_dir(t), ephemeris::sun_dir(t))
}

/// n samples of the tide at a city over [t0, t0+span].
pub fn tide_series(city: &City, t0: f64, span: f64, n: usize) -> Vec<f32> {
    (0..n)
        .map(|k| tide_at(city, t0 + span * k as f64 / (n - 1) as f64))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LUNAR_DAY: f64 = 86400.0 * 24.84 / 24.0;

    /// Sim seconds until a fixed ground point is next under the Moon's
    /// right ascension - i.e. the *actual* lunar day from the ephemeris.
    /// Walks the real relative azimuth (tilted rotation + wandering Moon)
    /// until it has completed one full turn.
    fn lunar_day_secs(t0: f64) -> f64 {
        let tau = std::f64::consts::TAU;
        let pi = std::f64::consts::PI;
        let rel = |t: f64| {
            let p = earth_model_at(t).transform_vector3(ground_dir(0.0, 0.0));
            let m = ephemeris::moon_dir(t);
            f64::from(p.x.atan2(p.z)) - f64::from(m.x.atan2(m.z))
        };
        let dt = 900.0;
        let mut acc = 0.0f64; // unwrapped relative drift
        let mut g_prev = rel(t0);
        for k in 1..200 {
            let t = t0 + k as f64 * dt;
            let g = rel(t);
            let step = (g - g_prev + 3.0 * pi) % tau - pi; // unwrap to (-pi, pi]
            acc += step;
            if acc.abs() >= tau {
                let prev = acc - step;
                let frac = (tau - prev.abs()) / (acc.abs() - prev.abs());
                return (k as f64 - 1.0 + frac) * dt;
            }
            g_prev = g;
        }
        panic!("relative azimuth never completed a turn");
    }

    #[test]
    fn the_real_lunar_day_is_about_24h50m() {
        for d0 in [0.0, 7.0, 14.0, 21.0] {
            let p = lunar_day_secs(d0 * 86400.0);
            let hrs = p / 3600.0;
            assert!(
                (24.3..=25.4).contains(&hrs),
                "day from {d0}: lunar day {hrs:.2} h"
            );
        }
    }

    #[test]
    fn tide_repeats_on_the_lunar_day() {
        // With the *solved* period, the lunar term recurs to within the
        // Moon's slow declination drift; the solar term barely moves.
        let t0 = 3.0 * 86400.0;
        let c = &CITIES[0];
        let h0 = tide_at(c, t0);
        let h1 = tide_at(c, t0 + lunar_day_secs(t0));
        assert!((h0 - h1).abs() < 0.05, "{h0} vs {h1} a lunar day apart");
    }

    #[test]
    fn two_highs_per_lunar_day() {
        // Count local maxima over 2 lunar days: expect 4 (semidiurnal), and
        // no wild extra wiggles.
        let s = tide_series(&CITIES[3], 0.0, 2.0 * LUNAR_DAY, 800);
        let peaks = (1..s.len() - 1)
            .filter(|k| s[*k] > s[*k - 1] && s[*k] >= s[*k + 1])
            .count();
        assert!((3..=5).contains(&peaks), "semidiurnal peaks: {peaks}");
    }

    #[test]
    fn spring_days_beat_neap_days() {
        // Equatorial station (Singapore), where the declinational diurnal
        // inequality can't mask the spring/neap beat the way it does at
        // London's 51 deg N. Epoch: ~1 day past full (spring); day 6: last
        // quarter (neap).
        let range = |t: f64| {
            let s = tide_series(&CITIES[7], t, LUNAR_DAY, 300);
            let max = s.iter().cloned().fold(f32::MIN, f32::max);
            let min = s.iter().cloned().fold(f32::MAX, f32::min);
            max - min
        };
        let (spring, neap) = (range(86400.0), range(5.0 * 86400.0));
        assert!(
            spring > 2.0 * neap,
            "spring range {spring} should tower over neap {neap}"
        );
    }

    #[test]
    fn highest_water_under_the_moon() {
        let t = 0.0;
        let moon = ephemeris::moon_dir(t);
        let sun = ephemeris::sun_dir(t);
        // A direction 90 deg from the Moon, built without depending on the
        // (nearly antiparallel near full moon) sun direction.
        let junk = if moon.dot(vec3(0.0, 1.0, 0.0)).abs() < 0.9 {
            vec3(0.0, 1.0, 0.0)
        } else {
            vec3(1.0, 0.0, 0.0)
        };
        let side = moon.cross(junk).normalize();
        let under = tide_m(moon, moon, sun);
        assert!(
            under > tide_m(side, moon, sun) + 0.2,
            "bulge must peak under the Moon"
        );
    }
}
