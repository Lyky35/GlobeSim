// ---------------------------------------------------------------------------
// Physical constants used by the simulation (real-world values).
// ---------------------------------------------------------------------------

export const EARTH_RADIUS_KM = 6371;

// Sidereal day: Earth's true rotation period relative to the stars,
// ~23h 56m 04s. (The 24h solar day is the day minus the extra rotation
// gained by orbiting the Sun.)
export const SIDEREAL_DAY_S = 86164.0905;

// Sidereal month: the Moon's orbital period relative to the stars.
export const SIDEREAL_MONTH_S = 27.321661 * 86400; // 2 360 592 s

// Earth's axial tilt (obliquity).
export const AXIAL_TILT_DEG = 23.44;

// The Moon's orbital inclination to the ecliptic.
export const MOON_ORBIT_INCL_DEG = 5.15;

// Mean Earth-Moon distance (km). Only used for display: the scene draws the
// Moon much closer than real scale (60 Earth radii away would be off-screen).
export const MOON_DISTANCE_KM = 384400;

// Typical amplitude of the equilibrium (astronomical) tide raised by the
// Moon: roughly 0.36 m. Used only to report how much the visual tide is
// exaggerated.
export const EQUILIBRIUM_TIDE_M = 0.36;

// --- Scene scale ------------------------------------------------------------
// 1 scene unit = 1 Earth radius.
export const EARTH_SCENE_RADIUS = 1;

// Visual (not to scale — the real Moon sits at ~60 Earth radii, far outside
// any sensible field of view) Moon distance in Earth radii and Moon radius.
export const MOON_SCENE_DISTANCE = 6.5;
export const MOON_SCENE_RADIUS = 0.2727 * EARTH_SCENE_RADIUS;

// Scene length units per km, so tide amplitudes can be set in km.
export const KM_PER_UNIT = EARTH_RADIUS_KM;
