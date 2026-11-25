use crate::cities;
use crate::climate::{Climate, CGH, CGW};
use crate::dem::Dem;
use crate::ephemeris;
use macroquad::prelude::*;

// ---------------------------------------------------------------------------
// Flat equirectangular side panel under the tide graph. Same layout every
// grid in the app uses: rows north -> south, columns lon 0 -> 360 deg E.
// [p] cycles three views: a live equilibrium-tide heat map, a mirror of the
// climate sim's surface air temperature, and the static ETOPO relief.
// ---------------------------------------------------------------------------

pub const PW: usize = CGW; // 256
pub const PH: usize = CGH; // 128
pub const HIDDEN: usize = 3; // mode index meaning "panel off"

pub const TITLES: [&str; 3] = [
    "tide heat map (equilibrium, live)",
    "climate mirror: surface air temp",
    "relief: ETOPO 2022 bed topography",
];

/// Diverging white -> red/blue for signed fields (warm = positive).
fn diverge(x: f32) -> (u8, u8, u8) {
    let x = x.clamp(-1.0, 1.0);
    if x >= 0.0 {
        (
            (235.0 + 20.0 * x) as u8,
            (235.0 - 185.0 * x) as u8,
            (235.0 - 195.0 * x) as u8,
        )
    } else {
        let x = -x;
        (
            (235.0 - 180.0 * x) as u8,
            (235.0 - 125.0 * x) as u8,
            (235.0 - 5.0 * x) as u8,
        )
    }
}

/// Ocean depth in blues, land from green lowlands to white peaks.
fn lerp8(a: u8, b: u8, x: f32) -> u8 {
    (a as f32 + (b as i32 - a as i32) as f32 * x) as u8
}

fn relief_ramp(h: f32) -> (u8, u8, u8) {
    if h < 0.0 {
        let x = (-h / 7000.0).clamp(0.0, 1.0);
        (lerp8(110, 20, x), lerp8(160, 30, x), lerp8(220, 80, x))
    } else if h < 1500.0 {
        let x = h / 1500.0;
        (lerp8(90, 120, x), lerp8(140, 160, x), lerp8(80, 60, x))
    } else if h < 4500.0 {
        let x = (h - 1500.0) / 3000.0;
        (lerp8(120, 210, x), lerp8(160, 180, x), lerp8(60, 40, x))
    } else {
        let x = ((h - 4500.0) / 3500.0).clamp(0.0, 1.0);
        let c = lerp8(210, 255, x);
        (c, c, c)
    }
}

pub struct FlatPanel {
    tex: Texture2D,
    px: Vec<u8>,
    relief_px: Vec<u8>, // baked once; the tide/climate views recompute live
}

/// Paint the live equilibrium-tide view into an RGBA buffer (pw x ph rows,
/// north -> south, lon 0 -> 360 E). Kept context-free so tests can run it.
pub fn fill_tide(px: &mut [u8], t: f64) {
    let model = cities::earth_model_at(t);
    let moon = ephemeris::moon_dir(t);
    let sun = ephemeris::sun_dir(t);
    for i in 0..PH {
        let lat = 90.0 - (i as f32 + 0.5) * 180.0 / PH as f32;
        for j in 0..PW {
            let lon = (j as f32 + 0.5) * 360.0 / PW as f32;
            let d = model.transform_vector3(cities::ground_dir(lat, lon));
            let h = cities::tide_m(d, moon, sun);
            // Full scale ~0.55 m: the combined bulge crest to trough.
            let (r, g, b) = diverge(h / 0.55);
            let p = (i * PW + j) * 4;
            px[p..p + 4].copy_from_slice(&[r, g, b, 255]);
        }
    }
}

/// Paint the static DEM relief view (baked once at startup).
pub fn fill_relief(px: &mut [u8], dem: &Dem) {
    for i in 0..PH {
        let lat = 90.0 - (i as f32 + 0.5) * 180.0 / PH as f32;
        for j in 0..PW {
            let lon = (j as f32 + 0.5) * 360.0 / PW as f32;
            let (r, g, b) = relief_ramp(dem.height_m(lat, lon));
            let p = (i * PW + j) * 4;
            px[p..p + 4].copy_from_slice(&[r, g, b, 255]);
        }
    }
}

impl FlatPanel {
    pub fn new() -> Self {
        let px = vec![0u8; PW * PH * 4];
        let tex = Texture2D::from_rgba8(PW as u16, PH as u16, &px);
        Self {
            tex,
            relief_px: Vec::new(),
            px,
        }
    }

    /// Bake the static relief view once at startup.
    pub fn bake_relief(&mut self, dem: &Dem) {
        self.relief_px = vec![0u8; PW * PH * 4];
        fill_relief(&mut self.relief_px, dem);
    }

    /// Redraw one view into the texture (called once per frame while the
    /// panel is visible; 33k cheap samples, no allocation).
    pub fn refresh(&mut self, mode: usize, t: f64, climate: &Climate) {
        match mode {
            0 => fill_tide(&mut self.px, t),
            1 => {
                let bytes = climate.temp_rgba();
                self.px[..].copy_from_slice(&bytes);
                // The overlay's semi-transparency is for the globe, not a
                // dashboard panel: the panel must not ghost the scene.
                for p in (3..self.px.len()).step_by(4) {
                    self.px[p] = 255;
                }
            }
            2 => self.px[..].copy_from_slice(&self.relief_px),
            _ => return,
        }
        self.tex.update(&Image {
            bytes: self.px.clone(),
            width: PW as u16,
            height: PH as u16,
        });
    }

    pub fn draw(&self, x: f32, y: f32) {
        // from_rgba8 textures sit bottom-up under macroquad's 2D camera;
        // flip_y puts row 0 (north) at the top, matching locate().
        draw_texture_ex(
            &self.tex,
            x,
            y,
            WHITE,
            DrawTextureParams {
                flip_y: true,
                ..Default::default()
            },
        );
        draw_rectangle_lines(
            x,
            y,
            PW as f32,
            PH as f32,
            1.0,
            Color::new(0.45, 0.55, 0.8, 0.8),
        );
    }

    /// Map (lat, lon) to panel pixels - used for the selected-city crosshair.
    pub fn locate(lat: f32, lon: f32) -> (f32, f32) {
        (
            (lon.rem_euclid(360.0) / 360.0) * PW as f32,
            ((90.0 - lat) / 180.0) * PH as f32,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px_rgb(px: &[u8], i: usize, j: usize) -> (u8, u8, u8) {
        let p = (i * PW + j) * 4;
        (px[p], px[p + 1], px[p + 2])
    }

    fn cell(lat: f32, lon: f32) -> (usize, usize) {
        (
            (((90.0 - lat) / 180.0) * PH as f32) as usize,
            (((lon + 360.0) % 360.0) / 360.0 * PW as f32) as usize,
        )
    }

    #[test]
    fn relief_bake_reads_like_a_map() {
        let dem = Dem::load("assets/textures/etopo_dem.bin");
        let mut px = vec![0u8; PW * PH * 4];
        fill_relief(&mut px, &dem);
        // Mt Fuji (35.4 N, 138.7 E): land -> greens/tans dominate blue.
        let (i, j) = cell(35.4, 138.7);
        let (r, _, b) = px_rgb(&px, i, j);
        assert!(r > b, "Fuji should be a land colour, got ({r},?,{b})");
        // Western Pacific abyss (11.5 N, 142 E): deep blue.
        let (i, j) = cell(11.5, 142.0);
        let (r, _, b) = px_rgb(&px, i, j);
        assert!(b > r + 60, "abyss should be blue, got ({r},?,{b})");
    }

    #[test]
    fn tide_map_agrees_with_the_city_readouts() {
        let mut px = vec![0u8; PW * PH * 4];
        // Any t: the panel cell under a city must carry the same sign as
        // the city tide graph (warm = high water, cold = low water).
        for t in [0.0f64, 4300.0, 12000.0] {
            fill_tide(&mut px, t);
            for c in cities::CITIES.iter() {
                let h = cities::tide_at(c, t);
                if h.abs() < 0.18 {
                    continue; // near the crossing: colour is nearly neutral
                }
                let (i, j) = cell(c.lat, c.lon);
                let (r, _, b) = px_rgb(&px, i, j);
                if h > 0.0 {
                    assert!(r as i32 > b as i32 + 20, "{} high but cold", c.name);
                } else {
                    assert!(b as i32 > r as i32 + 20, "{} low but warm", c.name);
                }
            }
        }
    }

    #[test]
    fn locate_wraps_longitude_and_clips_poles() {
        let (x0, _) = FlatPanel::locate(0.0, -90.0);
        let (x1, _) = FlatPanel::locate(0.0, 270.0);
        assert!((x0 - x1).abs() < 0.01);
        let (_, y) = FlatPanel::locate(90.0, 0.0);
        assert!(y < 1.0);
    }
}
