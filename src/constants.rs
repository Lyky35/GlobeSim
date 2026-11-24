// Physical constants used by the simulation (real-world values), and the
// scene scale derived from them. 1 scene unit = 1 Earth radius.

/// Sidereal day: Earth's true rotation period relative to the stars,
/// ~23h 56m 04s. (The 24h solar day adds the rotation gained orbiting the Sun.)
pub const SIDEREAL_DAY_S: f64 = 86164.0905;

/// Sidereal month: the Moon's orbital period relative to the stars.
pub const SIDEREAL_MONTH_S: f64 = 27.321661 * 86400.0; // 2 360 592 s

/// Earth's axial tilt (obliquity), degrees.
pub const AXIAL_TILT_DEG: f32 = 23.44;

/// The Moon's orbital inclination to the ecliptic, degrees.
pub const MOON_ORBIT_INCL_DEG: f32 = 5.15;

/// Typical amplitude of the equilibrium (astronomical) tide raised by the
/// Moon: roughly 0.36 m. Only used to report the visual exaggeration.
pub const EQUILIBRIUM_TIDE_M: f32 = 0.36;

/// Earth radius in km, used to convert the tide-height slider (km) into scene
/// length units (1 unit = 1 Earth radius).
pub const EARTH_RADIUS_KM: f32 = 6371.0;

/// Visual (not to scale — the real Moon is at ~60 Earth radii, far outside any
/// sensible field of view) Moon distance in Earth radii, and Moon radius.
pub const MOON_SCENE_DISTANCE: f32 = 6.5;
pub const MOON_SCENE_RADIUS: f32 = 0.2727;
