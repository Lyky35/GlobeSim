use macroquad::prelude::*;

// ---------------------------------------------------------------------------
// Where the Sun and the Moon actually are at simulation time t (seconds
// since an epoch that matches a real date).
//
// Truncated classical theories: Meeus' low-precision solar longitude (a few
// tens of arcseconds) and Brown's leading lunar terms (~0.3 deg) — vastly
// more precision than placing a day/night terminator, tide-raising bulges
// and a Moon phase needs, and it buys the seasons, the Moon's 5.1 deg
// inclined wandering, its speed variation, and — via the solar tide term
// elsewhere — real spring/neap beats, all for a handful of sin() calls.
//
// World frame: ecliptic plane = XZ, ecliptic north = +Y. Ecliptic longitude
// lambda is measured from the +Z axis (vernal equinox) toward -X, i.e.
// dir(lambda) = (-sin lambda, 0, cos lambda): at lambda = 90 deg the Sun
// sits toward -X, exactly where the 23.44 deg axial tilt leans the north
// pole, so northern summer arrives when the Sun reaches 90 deg.
// ---------------------------------------------------------------------------

/// Simulation t = 0 corresponds to this many Julian days past J2000.0
/// (2000-01-01 12:00 UTC). Chosen as 2026-08-29 12:00 UTC: ~1 day after
/// that month's full Moon, so the opening view is a brightly lit gibbous
/// Earth with a big spring tide rolling and a bright Moon on screen.
pub const EPOCH_DAYS: f64 = 9737.0;

/// Julian days past J2000 at sim time t seconds.
pub fn day(t: f64) -> f64 {
    EPOCH_DAYS + t / 86_400.0
}

/// Apparent solar ecliptic longitude, degrees (mean longitude + equation of
/// centre to second order).
pub fn sun_lon(d: f64) -> f64 {
    let l = 280.4603 + 0.985_647_366 * d;
    let g = (356.0470 + 0.985_600_259 * d).to_radians();
    (l + 1.9195 * g.sin() + 0.0200 * (2.0 * g).sin()).rem_euclid(360.0)
}

/// Moon ecliptic (longitude deg, latitude deg, distance km): leading
/// anomaly/node terms of Brown's theory.
pub fn moon_lon_lat(d: f64) -> (f64, f64, f64) {
    let l = 218.3162 + 13.176_396 * d; // mean longitude
    let m = (134.9634 + 13.064_993 * d).to_radians(); // mean anomaly
    let f = (93.2721 + 13.229_350 * d).to_radians(); // mean elongation from node
    let lon = (l + 6.289 * m.sin()).rem_euclid(360.0);
    let lat = 5.128 * f.sin();
    let dist = 385_000.6 - 20_905.3 * m.cos();
    (lon, lat, dist)
}

fn dir_of(lon_deg: f64, lat_deg: f64) -> Vec3 {
    let (sl, cl) = lon_deg.to_radians().sin_cos();
    let (sb, cb) = lat_deg.to_radians().sin_cos();
    vec3((-sl * cb) as f32, sb as f32, (cl * cb) as f32)
}

/// Earth -> Sun, world frame.
pub fn sun_dir(t: f64) -> Vec3 {
    dir_of(sun_lon(day(t)), 0.0)
}

/// Earth -> Moon, world frame (the 5.1 deg inclination is in the latitude
/// term; the node precesses on an 18.6 yr cycle inside `f`).
pub fn moon_dir(t: f64) -> Vec3 {
    let (lon, lat, _) = moon_lon_lat(day(t));
    dir_of(lon, lat)
}

/// Earth -> Moon distance, km.
pub fn moon_dist_km(t: f64) -> f64 {
    moon_lon_lat(day(t)).2
}

/// Sun-Moon elongation in degrees, 0 = new Moon, 180 = full Moon.
pub fn elongation(t: f64) -> f64 {
    let d = day(t);
    (moon_lon_lat(d).0 - sun_lon(d)).rem_euclid(360.0)
}

/// Illuminated fraction of the Moon's disc, 0..1.
pub fn illumination(t: f64) -> f32 {
    (1.0 - elongation(t).to_radians().cos() as f32) * 0.5
}

pub fn phase_name(t: f64) -> &'static str {
    match elongation(t) {
        e if e < 11.25 || e >= 348.75 => "new moon",
        e if e < 78.75 => "waxing crescent",
        e if e < 101.25 => "first quarter",
        e if e < 168.75 => "waxing gibbous",
        e if e < 191.25 => "full moon",
        e if e < 258.75 => "waning gibbous",
        e if e < 281.25 => "last quarter",
        _ => "waning crescent",
    }
}

/// How close the geometry is to spring tide: |cos(elongation)|, 1.0 at
/// new/full, 0 at the quarters.
pub fn spring_factor(t: f64) -> f32 {
    elongation(t).to_radians().cos().abs() as f32
}

pub fn spring_or_neap(t: f64) -> &'static str {
    let s = spring_factor(t);
    if s > 0.85 {
        "spring tide"
    } else if s < 0.35 {
        "neap tide"
    } else {
        "mid-cycle tide"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sun_advances_about_one_degree_per_day_and_comes_round() {
        let d0 = EPOCH_DAYS;
        // Mean motion 0.9856 deg/day; equation of centre wobbles it by
        // +-0.033 deg/day, so one day's step is only loosely pinned.
        let step = (sun_lon(d0 + 1.0) - sun_lon(d0) + 540.0).rem_euclid(360.0) - 180.0;
        assert!((step - 0.9856).abs() < 0.04, "daily solar step {step}");
        // A half year on the Sun is on the opposite side within the equation
        // of centre's +-1.9 deg swing doubled (Earth near perigee covers the
        // southern half-year faster than the northern one: Kepler in action).
        let half = (sun_lon(d0 + 182.62) - sun_lon(d0) + 540.0).rem_euclid(360.0) - 180.0;
        assert!(180.0 - half.abs() < 4.0, "half-year shift {half}");
    }

    #[test]
    fn moon_synodic_month_is_29_5_days() {
        let d0 = EPOCH_DAYS;
        let el0 = elongation_at(d0);
        let el1 = elongation_at(d0 + 29.53);
        let drift = (el1 - el0 + 540.0).rem_euclid(360.0) - 180.0;
        assert!(
            drift.abs() < 2.0,
            "elongation drifts {drift} deg per lunation"
        );
    }

    #[test]
    fn moon_distance_swings_between_perigee_and_apogee() {
        let mut lo = f64::MAX;
        let mut hi: f64 = 0.0;
        for k in 0..440 {
            let (_, _, r) = moon_lon_lat(EPOCH_DAYS + k as f64 * 0.1);
            lo = lo.min(r);
            hi = hi.max(r);
        }
        assert!(lo > 3.54e5 && lo < 3.66e5, "perigee {lo}");
        assert!(hi > 4.0e5 && hi < 4.08e5, "apogee {hi}");
    }

    #[test]
    fn solstice_geometry_lights_the_tilted_pole() {
        // sun_lon = 90 deg -> direction toward -X, where the axial tilt
        // (rotation +23.44 about Z) leans the north pole.
        let d = dir_of_test(90.0, 0.0);
        assert!(d.dot(vec3(-1.0, 0.0, 0.0)) > 0.999, "solstice dir {d}");
        let pole = (Mat4::from_rotation_z(23.44f32.to_radians()) * Mat4::from_rotation_y(0.0))
            .transform_vector3(vec3(0.0, 1.0, 0.0));
        assert!(
            pole.dot(d) > pole.dot(dir_of_test(270.0, 0.0)),
            "north pole must prefer the June sun"
        );
    }

    #[test]
    fn solar_tide_beats_spring_and_neap_into_the_equilibrium_field() {
        // Combined equilibrium field, unit lunar amplitude, ratio 0.46.
        let m = vec3(1.0, 0.0, 0.0);
        let peak = |elong: f32| -> f32 {
            let s = vec3(elong.to_radians().cos(), elong.to_radians().sin(), 0.0);
            let mut mx = -1e9f32;
            for i in 0..=180 {
                let a = i as f32 * std::f32::consts::PI / 180.0;
                for j in 0..360 {
                    let b = j as f32 * std::f32::consts::TAU / 360.0;
                    let d = vec3(a.sin() * b.cos(), a.sin() * b.sin(), a.cos());
                    let h = (1.5 * d.dot(m).powi(2) - 0.5) + 0.46 * (1.5 * d.dot(s).powi(2) - 0.5);
                    mx = mx.max(h);
                }
            }
            mx
        };
        let spring = peak(0.0); // bulges aligned: peak height 1.0 + 0.46
        let neap = peak(90.0); // bulges crossed: peak height 1.0 - 0.46/2
        assert!(
            (spring / neap - 1.9).abs() < 0.15,
            "spring/neap {spring} vs {neap}"
        );
    }

    // local helper for lon-only directions (lat is always 0 for the sun)
    fn dir_of_test(lon: f64, lat: f64) -> Vec3 {
        dir_of(lon, lat)
    }

    fn elongation_at(d: f64) -> f64 {
        (moon_lon_lat(d).0 - sun_lon(d)).rem_euclid(360.0)
    }
}
