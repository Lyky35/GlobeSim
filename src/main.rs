mod atmos;
mod camera;
mod climate;
mod constants;
mod ephemeris;
mod impact;
mod shaders;
mod sphere;
mod tide;

use atmos::{CloudField, GRID_H, GRID_W};
use camera::OrbitCam;
use climate::{Climate, LandMask, CGH, CGW, STREAK_H, STREAK_W};
use constants::*;
use impact::{Impact, Impactor, APHOPIS, CHICXULUB, TSUNAMI_C_M_S};
use macroquad::miniquad::{
    BlendFactor, BlendState, BlendValue, CullFace, Equation, PipelineParams, ShaderSource,
    UniformDesc, UniformType,
};
use macroquad::prelude::*;
use sphere::SphereGrid;
use tide::{TideMode, TideOcean};

use shaders::{
    ATMO_FS, ATMO_VS, CLOUD_FS, CLOUD_VS, GLOW_FS, GLOW_VS, GRAVITY_FS, GRAVITY_VS, LAMBERT_FS,
    LAMBERT_VS, SKY_FS, SKY_VS, TEX_FS, TEX_VS, WATER_FS, WATER_VS,
};

pub enum CloudMode {
    /// Live CPU circulation simulation (advection + convection).
    Sim,
    /// Frozen texture scrolled at trade-wind speed (GPU-cheap comparison).
    Drift,
    Off,
}

const TAU: f64 = std::f64::consts::TAU;

fn window_conf() -> macroquad::conf::Conf {
    macroquad::conf::Conf {
        miniquad_conf: macroquad::miniquad::conf::Conf {
            window_title: "GlobeSim — Earth rotation & lunar tidal bulge".to_string(),
            window_width: 1280,
            window_height: 800,
            window_resizable: true,
            sample_count: 4, // MSAA
            ..Default::default()
        },
        // macroquad batches geometry into single draw calls; its defaults
        // (10k verts / 5k indices) are far below one u16 batch, so raise them.
        // The anomaly shell needs ~86k indices (index *values* still fit u16).
        draw_call_vertex_capacity: 65535,
        draw_call_index_capacity: 131070,
        ..Default::default()
    }
}

fn fmt_x(n: f64) -> String {
    if n >= 1e6 {
        format!("x{:.2}M", n / 1e6)
    } else if n >= 1e3 {
        format!("x{:.2}k", n / 1e3)
    } else {
        format!("x{:.0}", n)
    }
}

fn fmt_grouped(n: u64) -> String {
    let s = n.to_string();
    s.as_bytes()
        .iter()
        .rev()
        .enumerate()
        .fold(String::new(), |acc, (i, b)| {
            if i > 0 && i % 3 == 0 {
                format!("{}{}{}", acc, ",", *b as char)
            } else {
                format!("{}{}", acc, *b as char)
            }
        })
        .chars()
        .rev()
        .collect()
}

fn fmt_hm(sec: f64) -> String {
    if sec < 60.0 {
        format!("{:.1} s", sec)
    } else if sec < 86400.0 {
        format!("{:.1} h", sec / 3600.0)
    } else {
        format!("{:.1} d", sec / 86400.0)
    }
}

/// Orthonormal pair spanning the plane perpendicular to `axis` (used to walk
/// a small circle of angular radius psi around the impact point).
fn ring_basis(axis: Vec3) -> (Vec3, Vec3) {
    let tmp = if axis.abs().y < 0.9 {
        vec3(0.0, 1.0, 0.0)
    } else {
        vec3(1.0, 0.0, 0.0)
    };
    let e1 = axis.cross(tmp).normalize();
    let e2 = axis.cross(e1);
    (e1, e2)
}

/// Great-circle ring of angular radius `psi` around `axis`, drawn at `radius`
/// after applying `rot` (ring points are built in the ground frame so they
/// ride the Earth's rotation for free).
fn draw_ring(rot: Mat4, axis: Vec3, e1: Vec3, e2: Vec3, psi: f32, radius: f32, color: Color) {
    let segs = 72;
    let (cp, sp) = (psi.cos(), psi.sin());
    let mut prev = rot.transform_vector3((axis * cp + e1 * sp) * radius);
    for k in 1..=segs {
        let a = std::f32::consts::TAU * k as f32 / segs as f32;
        let d = axis * cp + (e1 * a.cos() + e2 * a.sin()) * sp;
        let p = rot.transform_vector3(d * radius);
        draw_line_3d(prev, p, color);
        prev = p;
    }
}

/// One-off physics briefing printed to the console when an impact fires.
fn report_impact(im: &Impact) {
    let s = &im.stats;
    let hiros = fmt_grouped((s.energy_mt / 0.015) as u64);
    println!();
    println!(
        "=== {}-class impact: {:.0} m at {:.1} km/s ===",
        im.rock.name,
        im.rock.diameter_m,
        im.rock.vel_m_s / 1000.0
    );
    println!(
        "energy    {:.2} J = {:.0} Mt (about {} Hiroshimas)",
        s.energy_j, s.energy_mt, hiros
    );
    println!("crater    ~{:.0} km final diameter", s.crater_km);
    println!(
        "seismic   Mw {:.1} (~1e-4 of the energy couples into quake waves)",
        s.mw
    );
    println!(
        "tsunami   ~{:.0} m crest near the crater, ring runs at {:.0} m/s, ocean-wide crossing ~{:.0} h",
        s.tsunami_m,
        TSUNAMI_C_M_S,
        (std::f64::consts::PI * impact::EARTH_R_M) / TSUNAMI_C_M_S / 3600.0
    );
    println!(
        "fireball  third-degree burns out to ~{:.0} km, ejecta plume ~{:.0} km",
        s.fireball_km, s.plume_km
    );
    if s.dust0 > 0.3 {
        println!(
            "climate   ~{:.0}% global dust loading -> impact winter (3-yr e-folding)",
            s.dust0 * 100.0
        );
    } else {
        println!(
            "climate   dust loading {:.1}% — regional disaster, no global cooling",
            s.dust0 * 100.0
        );
    }
    println!("(water: red = sea-surface gain, blue = depression)");
}

/// Decode an image file (JPEG/PNG) with the `image` crate and upload it as a
/// macroquad texture — macroquad's built-in loader has no JPEG support.
fn load_rgba_texture(path: &str) -> Texture2D {
    let bytes = std::fs::read(path)
        .unwrap_or_else(|e| panic!("cannot read {path} ({e}) — run from the repo root"));
    let img =
        image::load_from_memory(&bytes).unwrap_or_else(|e| panic!("cannot decode {path} ({e})"));
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Texture2D::from_rgba8(w as u16, h as u16, rgba.as_raw())
}

struct Sim {
    running: bool,
    speed: f64, // sim seconds per real second
    tide_km: f32,
    mode: TideMode,
    sim_time: f64,
    show_moon: bool,
    show_gravity: bool,
    show_axis: bool,
    show_help: bool,
    cloud_mode: CloudMode,
    show_atmo: bool,
    show_anomaly: bool,
    show_temp: bool,
    show_winds: bool,
    show_currents: bool,
    /// Longitude offset for GPU cloud-drift mode (fraction of full circle).
    cloud_drift: f32,
}

#[macroquad::main(window_conf)]
async fn main() {
    // --- assets -------------------------------------------------------------
    // macroquad's built-in loader has no JPEG decoder, so decode with `image`
    // and upload manually. Run from the repo root.
    // Earth relief is decoded once and kept around: the climate model
    // classifies ocean/land straight from the same pixels.
    let earth_bytes = std::fs::read("assets/textures/earth_topography_3840.jpg")
        .unwrap_or_else(|e| panic!("cannot read earth texture ({e}) — run from the repo root"));
    let earth_rgba = image::load_from_memory(&earth_bytes)
        .unwrap_or_else(|e| panic!("cannot decode earth texture ({e})"))
        .to_rgba8();
    let (ew, eh) = earth_rgba.dimensions();
    let earth_tex = Texture2D::from_rgba8(ew as u16, eh as u16, earth_rgba.as_raw());
    let moon_tex = load_rgba_texture("assets/textures/moon_2k.jpg");
    let stars_tex = load_rgba_texture("assets/textures/stars_milky_way_2k.jpg");
    let gravity_tex = load_rgba_texture("assets/textures/gravity_anomaly.jpg");

    // --- materials ----------------------------------------------------------
    let lambert_params = |extra: Vec<UniformDesc>| MaterialParams {
        pipeline_params: PipelineParams {
            cull_face: CullFace::Back,
            ..Default::default()
        },
        uniforms: extra,
        textures: Vec::new(),
    };

    let earth_mat = load_material(
        ShaderSource::Glsl {
            vertex: LAMBERT_VS,
            fragment: LAMBERT_FS,
        },
        lambert_params(vec![
            UniformDesc::new("u_model", UniformType::Mat4),
            UniformDesc::new("u_light_dir", UniformType::Float3),
        ]),
    )
    .unwrap();
    // Separate pipeline instances keep per-object uniforms from leaking
    // between draw calls that macroquad might otherwise batch together.
    let moon_mat = load_material(
        ShaderSource::Glsl {
            vertex: LAMBERT_VS,
            fragment: LAMBERT_FS,
        },
        lambert_params(vec![
            UniformDesc::new("u_model", UniformType::Mat4),
            UniformDesc::new("u_light_dir", UniformType::Float3),
        ]),
    )
    .unwrap();

    let water_mat = load_material(
        ShaderSource::Glsl {
            vertex: WATER_VS,
            fragment: WATER_FS,
        },
        MaterialParams {
            pipeline_params: PipelineParams {
                cull_face: CullFace::Back,
                color_blend: Some(BlendState::new(
                    Equation::Add,
                    BlendFactor::Value(BlendValue::SourceAlpha),
                    BlendFactor::OneMinusValue(BlendValue::SourceAlpha),
                )),
                alpha_blend: Some(BlendState::new(
                    Equation::Add,
                    BlendFactor::One,
                    BlendFactor::OneMinusValue(BlendValue::SourceAlpha),
                )),
                ..Default::default()
            },
            uniforms: vec![
                UniformDesc::new("u_model", UniformType::Mat4),
                UniformDesc::new("u_moon_dir", UniformType::Float3),
                UniformDesc::new("u_sun_dir", UniformType::Float3),
                UniformDesc::new("u_tide_ratio", UniformType::Float1),
                UniformDesc::new("u_tide_amp", UniformType::Float1),
                UniformDesc::new("u_cam_pos", UniformType::Float3),
                UniformDesc::new("u_light_dir", UniformType::Float3),
                UniformDesc::new("u_tsu_dir", UniformType::Float4),
                UniformDesc::new("u_tsu", UniformType::Float2),
                UniformDesc::new("u_gpu", UniformType::Float1),
                UniformDesc::new("u_base_r", UniformType::Float1),
                UniformDesc::new("u_anom_scale", UniformType::Float1),
            ],
            textures: Vec::new(),
        },
    )
    .unwrap();

    // Gravity anomaly overlay: alpha-blended shell just above the ground
    // (radius set on the mesh), co-rotating with the earth.
    let gravity_mat = load_material(
        ShaderSource::Glsl {
            vertex: GRAVITY_VS,
            fragment: GRAVITY_FS,
        },
        MaterialParams {
            pipeline_params: PipelineParams {
                cull_face: CullFace::Back,
                color_blend: Some(BlendState::new(
                    Equation::Add,
                    BlendFactor::Value(BlendValue::SourceAlpha),
                    BlendFactor::OneMinusValue(BlendValue::SourceAlpha),
                )),
                ..Default::default()
            },
            uniforms: vec![
                UniformDesc::new("u_model", UniformType::Mat4),
                UniformDesc::new("u_light_dir", UniformType::Float3),
                UniformDesc::new("u_alpha", UniformType::Float1),
            ],
            textures: Vec::new(),
        },
    )
    .unwrap();

    let sky_mat = load_material(
        ShaderSource::Glsl {
            vertex: SKY_VS,
            fragment: SKY_FS,
        },
        MaterialParams {
            pipeline_params: PipelineParams {
                cull_face: CullFace::Front, // view the sphere from inside
                depth_write: false,
                ..Default::default()
            },
            uniforms: Vec::new(),
            textures: Vec::new(),
        },
    )
    .unwrap();

    let cloud_mat = load_material(
        ShaderSource::Glsl {
            vertex: CLOUD_VS,
            fragment: CLOUD_FS,
        },
        MaterialParams {
            pipeline_params: PipelineParams {
                cull_face: CullFace::Back,
                color_blend: Some(BlendState::new(
                    Equation::Add,
                    BlendFactor::Value(BlendValue::SourceAlpha),
                    BlendFactor::OneMinusValue(BlendValue::SourceAlpha),
                )),
                ..Default::default()
            },
            uniforms: vec![
                UniformDesc::new("u_model", UniformType::Mat4),
                UniformDesc::new("u_drift", UniformType::Float1),
                UniformDesc::new("u_light_dir", UniformType::Float3),
            ],
            textures: Vec::new(),
        },
    )
    .unwrap();

    // Climate overlays share one generic unlit textured shader: the air
    // temperature map blends normally over the ground, the streak canvases
    // (wind + current tracers) are additive glows that never occlude.
    let tex_params = |additive: bool| MaterialParams {
        pipeline_params: PipelineParams {
            cull_face: CullFace::Back,
            depth_write: !additive,
            color_blend: Some(BlendState::new(
                Equation::Add,
                BlendFactor::Value(BlendValue::SourceAlpha),
                if additive {
                    BlendFactor::One
                } else {
                    BlendFactor::OneMinusValue(BlendValue::SourceAlpha)
                },
            )),
            ..Default::default()
        },
        uniforms: vec![
            UniformDesc::new("u_model", UniformType::Mat4),
            UniformDesc::new("u_alpha", UniformType::Float1),
        ],
        textures: Vec::new(),
    };
    let temp_mat = load_material(
        ShaderSource::Glsl {
            vertex: TEX_VS,
            fragment: TEX_FS,
        },
        tex_params(false),
    )
    .unwrap();
    let wind_mat = load_material(
        ShaderSource::Glsl {
            vertex: TEX_VS,
            fragment: TEX_FS,
        },
        tex_params(true),
    )
    .unwrap();
    let cur_mat = load_material(
        ShaderSource::Glsl {
            vertex: TEX_VS,
            fragment: TEX_FS,
        },
        tex_params(true),
    )
    .unwrap();

    // Additive glow shell: blends onto whatever is behind it, never occludes.
    let atmo_mat = load_material(
        ShaderSource::Glsl {
            vertex: ATMO_VS,
            fragment: ATMO_FS,
        },
        MaterialParams {
            pipeline_params: PipelineParams {
                cull_face: CullFace::Back,
                depth_write: false,
                color_blend: Some(BlendState::new(
                    Equation::Add,
                    BlendFactor::One,
                    BlendFactor::One,
                )),
                ..Default::default()
            },
            uniforms: vec![
                UniformDesc::new("u_cam_pos", UniformType::Float3),
                UniformDesc::new("u_light_dir", UniformType::Float3),
            ],
            textures: Vec::new(),
        },
    )
    .unwrap();

    // Flash / plume / dust-haze shells: unlit tinted quads, one material each
    // so their uniforms can't leak between draw calls.
    let glow_params = |additive: bool| MaterialParams {
        pipeline_params: PipelineParams {
            cull_face: CullFace::Back,
            depth_write: false,
            color_blend: Some(BlendState::new(
                Equation::Add,
                if additive {
                    BlendFactor::One
                } else {
                    BlendFactor::Value(BlendValue::SourceAlpha)
                },
                if additive {
                    BlendFactor::One
                } else {
                    BlendFactor::OneMinusValue(BlendValue::SourceAlpha)
                },
            )),
            ..Default::default()
        },
        uniforms: vec![
            UniformDesc::new("u_model", UniformType::Mat4),
            UniformDesc::new("u_color", UniformType::Float4),
        ],
        textures: Vec::new(),
    };
    let flash_mat = load_material(
        ShaderSource::Glsl {
            vertex: GLOW_VS,
            fragment: GLOW_FS,
        },
        glow_params(true),
    )
    .unwrap();
    let plume_mat = load_material(
        ShaderSource::Glsl {
            vertex: GLOW_VS,
            fragment: GLOW_FS,
        },
        glow_params(false),
    )
    .unwrap();
    let dust_mat = load_material(
        ShaderSource::Glsl {
            vertex: GLOW_VS,
            fragment: GLOW_FS,
        },
        glow_params(false),
    )
    .unwrap();

    // --- geometry -------------------------------------------------------------
    // 64 x 160 grid -> 61 440 indices: under one u16 draw call, still 10 400
    // vertices for the CPU tide pass (matches the web prototype's vertex count).
    let earth_grid = SphereGrid {
        rings: 64,
        slices: 160,
    };
    let earth_mesh = earth_grid.mesh(1.0, Some(earth_tex));
    let moon_mesh = SphereGrid {
        rings: 48,
        slices: 64,
    }
    .mesh(MOON_SCENE_RADIUS, Some(moon_tex));
    let sky_mesh = SphereGrid {
        rings: 32,
        slices: 64,
    }
    .mesh(100.0, Some(stars_tex));

    let mut ocean = TideOcean::new(1.0005, 64, 160, None);

    // Cloud field: 256 x 128 lat/lon grid uploaded as an RGBA texture.
    let mut clouds = CloudField::new();
    let cloud_tex = Texture2D::from_rgba8(GRID_W as u16, GRID_H as u16, clouds.rgba8());
    let cloud_mesh = SphereGrid {
        rings: 48,
        slices: 96,
    }
    .mesh(CLOUD_SHELL_RADIUS, Some(cloud_tex.clone()));
    let atmo_mesh = SphereGrid {
        rings: 32,
        slices: 64,
    }
    .mesh(ATMO_SHELL_RADIUS, None);
    // Anomaly shell: sits between ground (1.0) and water (1.0005); finer grid
    // than the earth so the 1440x720 anomaly texture stays crisp. 81 x 181
    // verts, 86 400 indices — fits the raised draw-call capacities above.
    let gravity_mesh = SphereGrid {
        rings: 80,
        slices: 180,
    }
    .mesh(1.0003, Some(gravity_tex));
    let glow_mesh = SphereGrid {
        rings: 16,
        slices: 32,
    }
    .mesh(1.0, None);

    // Climate layer: ocean mask straight from the relief pixels, currents
    // solved at startup, then temperature + tracer streaks evolve per frame.
    let mask = LandMask::from_rgba(ew, eh, earth_rgba.as_raw());
    let mut climate = Climate::new(mask);
    let temp_tex = Texture2D::from_rgba8(CGW as u16, CGH as u16, &climate.temp_rgba());
    let wind_tex = Texture2D::from_rgba8(STREAK_W as u16, STREAK_H as u16, &climate.wind_rgba);
    let cur_tex = Texture2D::from_rgba8(STREAK_W as u16, STREAK_H as u16, &climate.cur_rgba);
    // Shell stack: currents ride just above the water (1.0005), the air
    // temperature map over everything, wind streaks under the cloud deck.
    let cur_mesh = SphereGrid {
        rings: 64,
        slices: 160,
    }
    .mesh(1.0006, Some(cur_tex.clone()));
    let temp_mesh = SphereGrid {
        rings: 64,
        slices: 160,
    }
    .mesh(1.006, Some(temp_tex.clone()));
    let wind_mesh = SphereGrid {
        rings: 64,
        slices: 160,
    }
    .mesh(1.010, Some(wind_tex.clone()));

    // --- simulation state ------------------------------------------------------
    // Small CLI conveniences for benchmarking/screenshots: --cpu, --tide=<km>,
    // --clouds=off|drift, --impact=apophis|chicxulub, --no-anomaly, --temp.
    let args: Vec<String> = std::env::args().collect();
    let tide_km = args
        .iter()
        .find_map(|a| a.strip_prefix("--tide="))
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(250.0)
        .clamp(0.0, 1000.0);
    let mode = if args.iter().any(|a| a == "--cpu") {
        TideMode::Cpu
    } else {
        TideMode::Gpu
    };
    let cloud_mode = match args
        .iter()
        .find_map(|a| a.strip_prefix("--clouds="))
        .map(|s| s.to_string())
        .as_deref()
    {
        Some("off") => CloudMode::Off,
        Some("drift") => CloudMode::Drift,
        _ => CloudMode::Sim,
    };
    let pending_rock: Option<Impactor> = args
        .iter()
        .find_map(|a| a.strip_prefix("--impact="))
        .map(|s| if s == "chicxulub" { CHICXULUB } else { APHOPIS });
    let anomaly_on = !args.iter().any(|a| a == "--no-anomaly");
    let temp_on = args.iter().any(|a| a == "--temp");

    let mut sim = Sim {
        running: true,
        speed: 3600.0, // one day every 24 s
        tide_km,
        mode,
        sim_time: 0.0,
        show_moon: true,
        show_gravity: true,
        show_axis: false,
        show_help: true,
        cloud_mode,
        show_atmo: true,
        show_anomaly: anomaly_on,
        show_temp: temp_on, // over the streaks: off at launch, [t] reveals it
        show_winds: true,
        show_currents: true,
        cloud_drift: 0.0,
    };
    let mut cam = OrbitCam::new();

    // Frame the view so the Moon sits beside Earth's limb at launch: put the
    // camera roughly opposite the Moon, offset by an angle that fits the
    // window's field of view (narrow windows need a smaller offset). Tiling
    // window managers often resize the window a few frames after launch, so
    // keep reframing until the user grabs the camera or the window settles.
    let mut framing_active = true;
    let t_start = get_time();

    // Impact state: a CLI-launched impact waits for the framing to settle;
    // impact_flash_t runs on the real clock (flash & plume are seconds-long).
    let mut pending = pending_rock;
    let mut impact: Option<Impact> = None;
    let mut impact_flash_t = -1.0f32;
    let mut spawn_rock: Option<Impactor> = None;

    // Lunar day and the resulting high-tide interval at the current speed.
    let lunar_day_s = SIDEREAL_DAY_S * SIDEREAL_MONTH_S / (SIDEREAL_MONTH_S - SIDEREAL_DAY_S);

    loop {
        next_frame().await;
        let dt = get_frame_time() as f64;

        if framing_active {
            if is_mouse_button_pressed(MouseButton::Left) || mouse_wheel().1 != 0.0 {
                framing_active = false; // user took over
            } else if get_time() - t_start > 2.0 {
                framing_active = false; // window settled, stop reframing
            } else {
                let m0 = ephemeris::moon_dir(sim.sim_time);
                let moon_az = m0.x.atan2(m0.z);
                let off = if screen_width() > screen_height() {
                    0.6
                } else {
                    0.35
                };
                cam.yaw0 = moon_az - std::f32::consts::PI - off;
                cam.yaw = cam.yaw0;
            }
        }

        // ------------------------------------------------------------- input
        if is_key_pressed(KeyCode::Space) {
            sim.running = !sim.running;
        }
        if is_key_pressed(KeyCode::C) {
            sim.mode = TideMode::Cpu;
        }
        if is_key_pressed(KeyCode::G) {
            sim.mode = TideMode::Gpu;
            ocean.reset_shape(); // GPU displaces in the shader, not the mesh
        }
        if is_key_pressed(KeyCode::LeftBracket) {
            sim.tide_km = (sim.tide_km - 25.0).max(0.0);
        }
        if is_key_pressed(KeyCode::RightBracket) {
            sim.tide_km = (sim.tide_km + 25.0).min(1000.0);
        }
        if is_key_pressed(KeyCode::Minus) {
            sim.speed = (sim.speed / 2.0).max(0.01);
        }
        if is_key_pressed(KeyCode::Equal) {
            sim.speed = (sim.speed * 2.0).min(1e6);
        }
        if is_key_pressed(KeyCode::M) {
            sim.show_moon = !sim.show_moon;
        }
        if is_key_pressed(KeyCode::F) {
            sim.show_gravity = !sim.show_gravity;
        }
        if is_key_pressed(KeyCode::A) {
            sim.show_axis = !sim.show_axis;
        }
        if is_key_pressed(KeyCode::R) {
            cam.reset();
        }
        if is_key_pressed(KeyCode::H) {
            sim.show_help = !sim.show_help;
        }
        if is_key_pressed(KeyCode::K) {
            sim.cloud_mode = match sim.cloud_mode {
                CloudMode::Sim => CloudMode::Drift,
                CloudMode::Drift => CloudMode::Off,
                CloudMode::Off => CloudMode::Sim,
            };
        }
        if is_key_pressed(KeyCode::O) {
            sim.show_atmo = !sim.show_atmo;
        }
        if is_key_pressed(KeyCode::I) {
            spawn_rock = Some(APHOPIS);
        }
        if is_key_pressed(KeyCode::U) {
            spawn_rock = Some(CHICXULUB);
        }
        if is_key_pressed(KeyCode::V) {
            sim.show_anomaly = !sim.show_anomaly;
        }
        if is_key_pressed(KeyCode::T) {
            sim.show_temp = !sim.show_temp;
        }
        if is_key_pressed(KeyCode::W) {
            sim.show_winds = !sim.show_winds;
        }
        if is_key_pressed(KeyCode::N) {
            sim.show_currents = !sim.show_currents;
        }
        cam.handle_input();

        if sim.running {
            sim.sim_time += dt * sim.speed;
        }

        // ------------------------------------------------------------ physics
        // Earth: one turn per sidereal day, tilted 23.44 deg around world Z.
        let spin = (TAU * sim.sim_time / SIDEREAL_DAY_S) as f32;
        let tilt = Mat4::from_rotation_z(AXIAL_TILT_DEG.to_radians());
        let earth_model = tilt * Mat4::from_rotation_y(spin);

        // Sun and Moon from truncated real ephemerides: seasons, the Moon's
        // inclined wandering, and the spring/neap beat of two tidal bulges.
        let sun_world = ephemeris::sun_dir(sim.sim_time);
        let moon_dir = ephemeris::moon_dir(sim.sim_time);
        let moon_pos = moon_dir * MOON_SCENE_DISTANCE;

        let tide_amp = sim.tide_km / EARTH_RADIUS_KM; // scene units (1 = 1 Re)
        let dt_sim = if sim.running { dt * sim.speed } else { 0.0 };

        // Impacts: keys i/u queue one; a CLI --impact fires once framing ends.
        if pending.is_some() && get_time() - t_start > 2.4 {
            spawn_rock = pending.take();
        }
        if let Some(rock) = spawn_rock.take() {
            let gd = earth_model
                .inverse()
                .transform_vector3(cam.position().normalize());
            let im = Impact::trigger(rock, gd);
            report_impact(&im);
            impact = Some(im);
            impact_flash_t = 0.0;
        }
        if impact_flash_t >= 0.0 {
            impact_flash_t += dt as f32;
        }
        if let Some(im) = impact.as_mut() {
            im.step(dt_sim);
        }
        // Impact dust dims convection (impact winter) — Chicxulub-class wipes
        // the deck, Apophis-class is optically negligible.
        clouds.dust = impact.as_ref().map_or(0.0, |im| im.dust());

        // Clouds: the CPU circulation lives in the ground frame, so the
        // (fixed, world-frame) Sun is expressed in that frame; the subsolar
        // point then sweeps west once per sidereal day.
        match sim.cloud_mode {
            CloudMode::Sim => {
                let sun_ground = earth_model.inverse().transform_vector3(sun_world);
                let mut rem = dt_sim as f32;
                while rem > 0.0 {
                    let h = rem.min(MAX_CLOUD_STEP_S);
                    clouds.step(h, sun_ground);
                    rem -= h;
                }
                cloud_tex.update(&Image {
                    bytes: clouds.rgba8().clone(),
                    width: GRID_W as u16,
                    height: GRID_H as u16,
                });
            }
            CloudMode::Drift => sim.cloud_drift += dt_sim as f32 * CLOUD_DRIFT_RATE,
            CloudMode::Off => {}
        }

        // Climate: the gyres are steady (the wind belts are), so psi was
        // solved at startup; temperature relaxes toward radiative
        // equilibrium, gets shaded by the deck and impact dust, and is
        // advected — air by the winds, SST by the currents. The streak
        // canvases animate both flows.
        if dt_sim > 0.0 {
            let sun_ground = earth_model.inverse().transform_vector3(sun_world);
            let mut rem = dt_sim as f32;
            while rem > 0.0 {
                let h = rem.min(MAX_CLOUD_STEP_S);
                climate.step(h, sun_ground, clouds.coverage_grid(), clouds.dust);
                rem -= h;
            }
            climate.step_streaks(dt_sim as f32);
            temp_tex.update(&Image {
                bytes: climate.temp_rgba(),
                width: CGW as u16,
                height: CGH as u16,
            });
            wind_tex.update(&Image {
                bytes: climate.wind_rgba.clone(),
                width: STREAK_W as u16,
                height: STREAK_H as u16,
            });
            cur_tex.update(&Image {
                bytes: climate.cur_rgba.clone(),
                width: STREAK_W as u16,
                height: STREAK_H as u16,
            });
        }

        // ------------------------------------------------------------ render
        clear_background(Color::new(0.0, 0.0, 0.02, 1.0));
        let cam3d = cam.camera3d();
        set_camera(&cam3d);

        // sky
        gl_use_material(&sky_mat);
        draw_mesh(&sky_mesh);

        // earth
        earth_mat.set_uniform("u_model", earth_model);
        earth_mat.set_uniform("u_light_dir", sun_world);
        gl_use_material(&earth_mat);
        draw_mesh(&earth_mesh);

        // gravity anomaly map — EGM96 free-air overlay, co-rotating with the
        // ground, transparent where the anomaly is near zero
        if sim.show_anomaly {
            gravity_mat.set_uniform("u_model", earth_model);
            gravity_mat.set_uniform("u_light_dir", sun_world);
            gravity_mat.set_uniform("u_alpha", 0.9f32);
            gl_use_material(&gravity_mat);
            draw_mesh(&gravity_mesh);
        }

        // ocean — CPU pass displaces the mesh in Rust, GPU pass in the shader;
        // both fold in the tsunami crest ring, both feed the red/blue anomaly
        // colormap the same way (GPU by formula, CPU by reading the mesh back).
        let wave = impact
            .as_ref()
            .map(|im| im.wave(earth_model.transform_vector3(im.ground_dir)));
        match sim.mode {
            TideMode::Cpu => ocean.update_cpu(moon_dir, sun_world, tide_amp, wave.as_ref()),
            TideMode::Gpu => {
                water_mat.set_uniform("u_moon_dir", moon_dir);
                water_mat.set_uniform("u_sun_dir", sun_world);
                water_mat.set_uniform("u_tide_ratio", SOLAR_TIDE_RATIO);
                water_mat.set_uniform("u_tide_amp", tide_amp);
            }
        }
        let wave_a0 = wave.as_ref().map_or(0.0, |w| w.a0);
        match &wave {
            Some(w) => {
                water_mat.set_uniform("u_tsu_dir", vec4(w.dir.x, w.dir.y, w.dir.z, w.front));
                water_mat.set_uniform("u_tsu", vec2(w.a0, w.width));
            }
            None => {
                water_mat.set_uniform("u_tsu_dir", vec4(0.0f32, 0.0, 0.0, 0.0));
                water_mat.set_uniform("u_tsu", vec2(0.0f32, 0.04));
            }
        }
        water_mat.set_uniform(
            "u_gpu",
            if matches!(sim.mode, TideMode::Gpu) {
                1.0f32
            } else {
                0.0f32
            },
        );
        water_mat.set_uniform("u_base_r", 1.0005f32);
        water_mat.set_uniform("u_anom_scale", (tide_amp + wave_a0).max(1e-4));
        water_mat.set_uniform("u_model", Mat4::IDENTITY);
        water_mat.set_uniform("u_cam_pos", cam.position());
        water_mat.set_uniform("u_light_dir", sun_world);
        gl_use_material(&water_mat);
        draw_mesh(&ocean.mesh);

        // climate overlays, inside-out like the other shells: current
        // streaks over the water, temperature map, wind streaks
        if sim.show_currents {
            cur_mat.set_uniform("u_model", earth_model);
            cur_mat.set_uniform("u_alpha", 1.0f32);
            gl_use_material(&cur_mat);
            draw_mesh(&cur_mesh);
        }
        if sim.show_temp {
            temp_mat.set_uniform("u_model", earth_model);
            temp_mat.set_uniform("u_alpha", 0.8f32);
            gl_use_material(&temp_mat);
            draw_mesh(&temp_mesh);
        }
        if sim.show_winds {
            wind_mat.set_uniform("u_model", earth_model);
            wind_mat.set_uniform("u_alpha", 1.0f32);
            gl_use_material(&wind_mat);
            draw_mesh(&wind_mesh);
        }

        // cloud deck — co-rotates with the ground (winds are surface-relative)
        if !matches!(sim.cloud_mode, CloudMode::Off) {
            cloud_mat.set_uniform("u_model", earth_model);
            cloud_mat.set_uniform(
                "u_drift",
                if matches!(sim.cloud_mode, CloudMode::Drift) {
                    sim.cloud_drift
                } else {
                    0.0
                },
            );
            cloud_mat.set_uniform("u_light_dir", sun_world);
            gl_use_material(&cloud_mat);
            draw_mesh(&cloud_mesh);
        }

        // impact dust haze: a brown veil replacing the deck in an impact winter
        let dust = impact.as_ref().map_or(0.0, |im| im.dust());
        if dust > 0.015 {
            dust_mat.set_uniform("u_model", Mat4::IDENTITY);
            dust_mat.set_uniform("u_color", vec4(0.48, 0.37, 0.27, (dust * 0.6).min(0.55)));
            gl_use_material(&dust_mat);
            draw_mesh(&atmo_mesh);
        }

        // moon
        if sim.show_moon {
            moon_mat.set_uniform("u_model", Mat4::from_translation(moon_pos));
            moon_mat.set_uniform("u_light_dir", sun_world);
            gl_use_material(&moon_mat);
            draw_mesh(&moon_mesh);
        }

        // atmospheric glow: additive rim shell drawn over the solid bodies
        if sim.show_atmo {
            atmo_mat.set_uniform("u_cam_pos", cam.position());
            atmo_mat.set_uniform("u_light_dir", sun_world);
            gl_use_material(&atmo_mat);
            draw_mesh(&atmo_mesh);
        }

        // fireball flash + rising ejecta plume (real-seconds envelope)
        if let Some(im) = &impact {
            let fl = impact_flash_t;
            let dw = earth_model.transform_vector3(im.ground_dir);
            if fl >= 0.0 && fl < 0.9 {
                let m = Mat4::from_translation(dw * 1.02)
                    * Mat4::from_cols(
                        vec4(0.05, 0.0, 0.0, 0.0),
                        vec4(0.0, 0.05, 0.0, 0.0),
                        vec4(0.0, 0.0, 0.05, 0.0),
                        vec4(0.0, 0.0, 0.0, 1.0),
                    );
                flash_mat.set_uniform("u_model", m);
                flash_mat.set_uniform("u_color", vec4(1.0, 0.98, 0.9, 1.0 - fl / 0.9));
                gl_use_material(&flash_mat);
                draw_mesh(&glow_mesh);
            }
            if fl >= 0.0 && fl < 4.0 {
                let h = 0.05 + 0.10 * (fl / 2.5).min(1.0);
                let y = vec3(0.0, 1.0, 0.0);
                let axis = y.cross(dw);
                let rot = if axis.length_squared() > 1e-8 {
                    Mat4::from_axis_angle(axis.normalize(), y.dot(dw).clamp(-1.0, 1.0).acos())
                } else if dw.y > 0.0 {
                    Mat4::IDENTITY
                } else {
                    Mat4::from_axis_angle(vec3(1.0, 0.0, 0.0), std::f32::consts::PI)
                };
                let m = rot
                    * Mat4::from_translation(vec3(0.0, 1.0 + h * 0.5, 0.0))
                    * Mat4::from_cols(
                        vec4(0.018, 0.0, 0.0, 0.0),
                        vec4(0.0, h * 0.5, 0.0, 0.0),
                        vec4(0.0, 0.0, 0.018, 0.0),
                        vec4(0.0, 0.0, 0.0, 1.0),
                    );
                plume_mat.set_uniform("u_model", m);
                plume_mat.set_uniform("u_color", vec4(0.55, 0.45, 0.38, 0.65 * (1.0 - fl / 4.0)));
                gl_use_material(&plume_mat);
                draw_mesh(&glow_mesh);
            }
        }

        // helper lines (default material, colour-only)
        gl_use_default_material();
        if sim.show_moon && sim.show_gravity {
            let dashes = 24;
            for k in 0..dashes {
                if k % 2 == 1 {
                    continue;
                }
                let t0 = k as f32 / dashes as f32;
                let t1 = (k + 1) as f32 / dashes as f32;
                draw_line_3d(
                    moon_pos * (1.0 - t0),
                    moon_pos * (1.0 - t1),
                    Color::new(1.0, 0.8, 0.27, 0.9),
                );
            }
        }
        if sim.show_axis {
            let north = tilt.transform_point3(vec3(0.0, 1.5, 0.0));
            let south = tilt.transform_point3(vec3(0.0, -1.5, 0.0));
            draw_line_3d(south, north, Color::new(1.0, 0.33, 0.33, 0.85));
            let segs = 96;
            let tau32 = std::f32::consts::TAU;
            for k in 0..segs {
                let a0 = tau32 * k as f32 / segs as f32;
                let a1 = tau32 * (k + 1) as f32 / segs as f32;
                let p0 = tilt.transform_point3(vec3(a0.cos() * 1.001, 0.0, a0.sin() * 1.001));
                let p1 = tilt.transform_point3(vec3(a1.cos() * 1.001, 0.0, a1.sin() * 1.001));
                draw_line_3d(p0, p1, Color::new(0.4, 0.67, 1.0, 0.5));
            }
        }

        // impact wavefronts: seismic ring racing over the ground (orange),
        // airblast ring in the atmosphere shell (pale cyan)
        if let Some(im) = &impact {
            let (e1, e2) = ring_basis(im.ground_dir);
            let psi_s = im.seismic_psi();
            if psi_s > 0.02 && psi_s < std::f32::consts::PI + 0.05 {
                let a = (1.0 - psi_s / (std::f32::consts::PI + 0.05)).max(0.15);
                draw_ring(
                    earth_model,
                    im.ground_dir,
                    e1,
                    e2,
                    psi_s,
                    1.004,
                    Color::new(1.0, 0.55, 0.15, 0.85 * a),
                );
            }
            let psi_k = im.shock_psi();
            if psi_k > 0.02 && psi_k < std::f32::consts::PI + 0.05 {
                let a = (1.0 - psi_k / (std::f32::consts::PI + 0.05)).max(0.1);
                draw_ring(
                    earth_model,
                    im.ground_dir,
                    e1,
                    e2,
                    psi_k,
                    ATMO_SHELL_RADIUS + 0.012,
                    Color::new(0.75, 0.9, 1.0, 0.5 * a),
                );
            }
        }

        // ---------------------------------------------------------------- HUD
        set_default_camera();
        let mode_txt = match sim.mode {
            TideMode::Cpu => "CPU (per-vertex)",
            TideMode::Gpu => "GPU (vertex shader)",
        };
        let exaggeration = if sim.tide_km > 0.0 {
            format!(
                "{} km  (~{}x the real {} m bulge)",
                sim.tide_km,
                fmt_grouped(((sim.tide_km * 1000.0) / EQUILIBRIUM_TIDE_M) as u64),
                EQUILIBRIUM_TIDE_M
            )
        } else {
            "off".to_string()
        };
        let cloud_txt = match sim.cloud_mode {
            CloudMode::Sim => format!(
                "simulated on CPU ({:.2} ms, {} cells)",
                clouds.last_ms,
                GRID_W * GRID_H
            ),
            CloudMode::Drift => "GPU drift (frozen texture scrolls)".to_string(),
            CloudMode::Off => "off".to_string(),
        };
        let mut lines = vec![
            "GlobeSim  [H] help".to_string(),
            format!(
                "{}  t = {:.2} sim days  speed {}",
                if sim.running { "run " } else { "PAUSE" },
                sim.sim_time / 86400.0,
                fmt_x(sim.speed)
            ),
            format!("tide: {}  mode: {}", exaggeration, mode_txt),
            format!(
                "sky: moon {} {:.0}% lit {:.0}k km  ·  {}",
                ephemeris::phase_name(sim.sim_time),
                100.0 * ephemeris::illumination(sim.sim_time),
                ephemeris::moon_dist_km(sim.sim_time) / 1000.0,
                ephemeris::spring_or_neap(sim.sim_time)
            ),
            format!(
                "high tide every ~{} real time",
                fmt_hm(lunar_day_s / 2.0 / sim.speed)
            ),
            format!(
                "clouds: {}{}  atmo: {}",
                cloud_txt,
                if matches!(sim.cloud_mode, CloudMode::Off) {
                    String::new()
                } else {
                    format!("  cover {:.0}%", clouds.coverage * 100.0)
                },
                if sim.show_atmo { "on" } else { "off" }
            ),
            format!(
                "climate: air mean {:+.0} C, poles {:+.0} C | currents jet {:.2} m/s (gyre {:.3})  heat {:.1} PW",
                climate.air_mean,
                climate.pole_air,
                climate.jet_ms,
                climate.interior_ms,
                climate.heat_pw
            ),
            format!(
                "sea anomaly colour: red = +m gain, blue = -m dip, full scale +{} / -{} m",
                fmt_grouped(((tide_amp + wave_a0) * EARTH_RADIUS_KM * 1000.0) as u64),
                fmt_grouped((tide_amp * 0.5 * EARTH_RADIUS_KM * 1000.0) as u64)
            ),
            format!(
                "{:.0} fps  {}",
                get_fps(),
                match sim.mode {
                    TideMode::Cpu => format!(
                        "tide pass: {:.2} ms/frame on CPU ({} verts)",
                        ocean.last_cpu_ms,
                        ocean.vertex_count()
                    ),
                    TideMode::Gpu => "tide pass: vertex shader (GPU), ~0 ms on CPU".to_string(),
                }
            ),
        ];
        if sim.show_anomaly {
            lines.push(
                "gravity anomaly: EGM96 free-air, red + / blue - mGal, clear near 0, full scale +/-120"
                    .to_string(),
            );
        }
        if let Some(im) = &impact {
            let s = &im.stats;
            lines.push(format!(
                "IMPACT {} {} m @ {:.1} km/s = {} Mt (~{} Hiroshimas)",
                im.rock.name.to_uppercase(),
                im.rock.diameter_m as u32,
                im.rock.vel_m_s / 1000.0,
                fmt_grouped(s.energy_mt as u64),
                fmt_grouped((s.energy_mt / 0.015) as u64)
            ));
            lines.push(format!(
                "  age {}  crater ~{:.0} km  Mw {:.1}  tsunami crest ~{:.0} m, ring {:.0} deg @ {:.0} m/s  dust {:.1}%",
                fmt_hm(im.age_s),
                s.crater_km,
                s.mw,
                s.tsunami_m,
                wave.as_ref().map_or(0.0, |w| w.front.to_degrees()),
                TSUNAMI_C_M_S,
                im.dust() * 100.0
            ));
        }
        for (i, line) in lines.iter().enumerate() {
            draw_text(line, 16.0, 20.0 + 22.0 * i as f32, 15.0, WHITE);
        }
        if sim.show_help {
            draw_multiline_text(
                "[space] pause  [c]/[g] cpu/gpu tide  [ / ] tide +/-25 km  [-]/[=] speed /2/x2\n\
                 [k] clouds sim/drift/off  [o] atmosphere  [m] moon  [f] tidal pull  [a] axis\n\
                 [i] Apophis impact  [u] 10-km impact  [v] gravity map  [r] reset camera  [H] hide this\n\
                 [t] temperature map  [w] wind streaks  [n] current streaks\n\
                 drag = orbit  wheel = zoom",
                16.0,
                screen_height() - 94.0,
                14.0,
                None,
                WHITE,
            );
        }
    }
}
