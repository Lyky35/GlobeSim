mod camera;
mod constants;
mod shaders;
mod sphere;
mod tide;

use camera::OrbitCam;
use constants::*;
use macroquad::miniquad::{
    BlendFactor, BlendState, BlendValue, CullFace, Equation, PipelineParams, ShaderSource,
    UniformDesc, UniformType,
};
use macroquad::prelude::*;
use sphere::SphereGrid;
use tide::{TideMode, TideOcean};

use shaders::{LAMBERT_FS, LAMBERT_VS, SKY_FS, SKY_VS, WATER_FS, WATER_VS};

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
        draw_call_vertex_capacity: 65535,
        draw_call_index_capacity: 65535,
        ..Default::default()
    }
}

fn sun_dir() -> Vec3 {
    vec3(0.87, 0.32, 0.38).normalize()
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
}

#[macroquad::main(window_conf)]
async fn main() {
    // --- assets -------------------------------------------------------------
    // macroquad's built-in loader has no JPEG decoder, so decode with `image`
    // and upload manually. Run from the repo root.
    let earth_tex = load_rgba_texture("assets/textures/earth_topography_3840.jpg");
    let moon_tex = load_rgba_texture("assets/textures/moon_2k.jpg");
    let stars_tex = load_rgba_texture("assets/textures/stars_milky_way_2k.jpg");

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
                UniformDesc::new("u_tide_amp", UniformType::Float1),
                UniformDesc::new("u_cam_pos", UniformType::Float3),
                UniformDesc::new("u_light_dir", UniformType::Float3),
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

    // --- simulation state ------------------------------------------------------
    // Small CLI conveniences for benchmarking/screenshots: --cpu, --tide=<km>.
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
    };
    let mut cam = OrbitCam::new();

    // Moon position helper (also used for the initial camera framing).
    let moon_start = 40f32.to_radians();
    let moon_pos_at = |t: f64| -> Vec3 {
        let a = moon_start + (TAU * t / SIDEREAL_MONTH_S) as f32;
        (Mat4::from_rotation_x(MOON_ORBIT_INCL_DEG.to_radians()) * Mat4::from_rotation_y(a))
            .transform_point3(vec3(MOON_SCENE_DISTANCE, 0.0, 0.0))
    };

    // Frame the view so the Moon sits beside Earth's limb at launch: put the
    // camera roughly opposite the Moon, offset by an angle that fits the
    // window's field of view (narrow windows need a smaller offset). Tiling
    // window managers often resize the window a few frames after launch, so
    // keep reframing until the user grabs the camera or the window settles.
    let mut framing_active = true;
    let t_start = get_time();

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
                let m0 = moon_pos_at(sim.sim_time);
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
        cam.handle_input();

        if sim.running {
            sim.sim_time += dt * sim.speed;
        }

        // ------------------------------------------------------------ physics
        // Earth: one turn per sidereal day, tilted 23.44 deg around world Z.
        let spin = (TAU * sim.sim_time / SIDEREAL_DAY_S) as f32;
        let tilt = Mat4::from_rotation_z(AXIAL_TILT_DEG.to_radians());
        let earth_model = tilt * Mat4::from_rotation_y(spin);

        // Moon: one orbit per sidereal month in the ecliptic (XZ) plane,
        // inclined 5.15 deg and prograde (same sense as Earth's spin).
        let moon_pos = moon_pos_at(sim.sim_time);
        let moon_dir = moon_pos.normalize();

        let tide_amp = sim.tide_km / EARTH_RADIUS_KM; // scene units (1 = 1 Re)

        // ------------------------------------------------------------ render
        clear_background(Color::new(0.0, 0.0, 0.02, 1.0));
        let cam3d = cam.camera3d();
        set_camera(&cam3d);

        // sky
        gl_use_material(&sky_mat);
        draw_mesh(&sky_mesh);

        // earth
        earth_mat.set_uniform("u_model", earth_model);
        earth_mat.set_uniform("u_light_dir", sun_dir());
        gl_use_material(&earth_mat);
        draw_mesh(&earth_mesh);

        // ocean — CPU pass displaces the mesh in Rust, GPU pass in the shader
        match sim.mode {
            TideMode::Cpu => ocean.update_cpu(moon_dir, tide_amp),
            TideMode::Gpu => {
                water_mat.set_uniform("u_moon_dir", moon_dir);
                water_mat.set_uniform("u_tide_amp", tide_amp);
            }
        }
        water_mat.set_uniform("u_model", Mat4::IDENTITY);
        water_mat.set_uniform("u_cam_pos", cam.position());
        water_mat.set_uniform("u_light_dir", sun_dir());
        gl_use_material(&water_mat);
        draw_mesh(&ocean.mesh);

        // moon
        if sim.show_moon {
            moon_mat.set_uniform("u_model", Mat4::from_translation(moon_pos));
            moon_mat.set_uniform("u_light_dir", sun_dir());
            gl_use_material(&moon_mat);
            draw_mesh(&moon_mesh);
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
        let lines = [
            format!("GlobeSim  [H] help"),
            format!(
                "{}  t = {:.2} sim days  speed {}",
                if sim.running { "run " } else { "PAUSE" },
                sim.sim_time / 86400.0,
                fmt_x(sim.speed)
            ),
            format!("tide: {}  mode: {}", exaggeration, mode_txt),
            format!(
                "high tide every ~{} real time",
                fmt_hm(lunar_day_s / 2.0 / sim.speed)
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
        for (i, line) in lines.iter().enumerate() {
            draw_text(line, 16.0, 20.0 + 22.0 * i as f32, 15.0, WHITE);
        }
        if sim.show_help {
            draw_multiline_text(
                "[space] pause  [c]/[g] cpu/gpu tide  [ / ] tide ±25 km  [-]/[=] speed ÷2/×2\n\
                 [m] moon  [f] gravity line  [a] axis  [r] reset camera  [H] hide this\n\
                 drag = orbit  wheel = zoom",
                16.0,
                screen_height() - 74.0,
                14.0,
                None,
                WHITE,
            );
        }
    }
}
