// Physical constants used by the simulation (real-world values), and the
// scene scale derived from them. 1 scene unit = 1 Earth radius.

/// Sidereal day: Earth's true rotation period relative to the stars,
/// ~23h 56m 04s. (The 24h solar day adds the rotation gained orbiting the Sun.)
pub const SIDEREAL_DAY_S: f64 = 86164.0905;

/// Sidereal month: the Moon's orbital period relative to the stars.
pub const SIDEREAL_MONTH_S: f64 = 27.321661 * 86400.0; // 2 360 592 s

/// Earth's axial tilt (obliquity), degrees.
pub const AXIAL_TILT_DEG: f32 = 23.44;

/// Typical amplitude of the equilibrium (astronomical) tide raised by the
/// Moon: roughly 0.36 m. Only used to report the visual exaggeration.
pub const EQUILIBRIUM_TIDE_M: f32 = 0.36;

/// Vertical exaggeration applied to real ETOPO land relief. At x15 the
/// highest ~0.5-deg box-mean heights (~5.7 km) lift ~85 km in scene units:
/// readably mountainous from orbit, with the grand ranges poking through
/// the (also exaggerated) cloud deck, much as orographic clouds break.
pub const RELIEF_EXAGG: f32 = 15.0;

/// The Sun's tide-raising power relative to the Moon's (0.46: same order,
/// because the Sun's huge distance mostly cancels its huge mass). Adding
/// this second bulge is what produces the spring/neap cycle: bulges lined
/// up at new/full moon -> ~1.46x; crossed at the quarters -> ~0.54x.
pub const SOLAR_TIDE_RATIO: f32 = 0.46;

/// Earth radius in km, used to convert the tide-height slider (km) into scene
/// length units (1 unit = 1 Earth radius).
pub const EARTH_RADIUS_KM: f32 = 6371.0;

/// Visual (not to scale — the real Moon is at ~60 Earth radii, far outside any
/// sensible field of view) Moon distance in Earth radii, and Moon radius.
pub const MOON_SCENE_DISTANCE: f32 = 6.5;
pub const MOON_SCENE_RADIUS: f32 = 0.2727;

/// Cloud deck and atmospheric glow shell radii, in Earth radii.
pub const CLOUD_SHELL_RADIUS: f32 = 1.012;
pub const ATMO_SHELL_RADIUS: f32 = 1.055;

/// The cloud circulation is advanced in chunks of at most this many sim
/// seconds per frame (huge backtrace steps would smear the field).
pub const MAX_CLOUD_STEP_S: f32 = 4.0 * 3600.0;

/// Zonal speed the GPU "drift" mode uses for the frozen cloud texture —
/// the trade-wind speed — as a fraction of 2π of longitude per sim second.
pub const CLOUD_DRIFT_RATE: f32 = 8.0 / (6.371e6 * std::f32::consts::TAU);
