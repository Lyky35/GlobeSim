// GLSL shader sources (ES 1.00 flavour, as macroquad's pipeline expects).
//
// Every macroquad custom material automatically receives:
//   * attributes: position (vec3), texcoord (vec2), color0 (vec4), normal (vec4)
//   * built-in uniforms: Projection (camera projection*view), Model, _Time
//   * reserved sampler name `Texture`, bound from Mesh::texture at draw time.
// Per-object transforms are passed through our own `u_model` uniform.

pub const LAMBERT_VS: &str = r#"#version 100
attribute vec3 position;
attribute vec2 texcoord;
attribute vec4 color0;
attribute vec4 normal;

uniform mat4 Projection;
uniform mat4 u_model;

varying vec2 v_uv;
varying vec3 v_normal;

void main() {
    v_uv = texcoord;
    v_normal = normalize((u_model * vec4(normal.xyz, 0.0)).xyz);
    gl_Position = Projection * u_model * vec4(position, 1.0);
}
"#;

pub const LAMBERT_FS: &str = r#"#version 100
precision mediump float;

varying vec2 v_uv;
varying vec3 v_normal;

uniform sampler2D Texture;
uniform vec3 u_light_dir;

void main() {
    vec4 tex = texture2D(Texture, v_uv);
    float lambert = max(dot(normalize(v_normal), u_light_dir), 0.0);
    vec3 c = tex.rgb * (0.22 + 0.95 * lambert);
    gl_FragColor = vec4(c, 1.0);
}
"#;

// Water: same lighting plus the equilibrium-tide vertex displacement
// (active when u_tide_amp > 0 — in CPU mode amplitude is 0 and the mesh
// arrives pre-displaced with recomputed normals).
pub const WATER_VS: &str = r#"#version 100
attribute vec3 position;
attribute vec2 texcoord;
attribute vec4 color0;
attribute vec4 normal;

uniform mat4 Projection;
uniform mat4 u_model;
uniform vec3 u_moon_dir;   // direction Earth -> Moon, object space
uniform float u_tide_amp;  // bulge amplitude A, scene units
uniform vec3 u_cam_pos;    // camera position, world space
uniform vec4 u_tsu_dir;    // xyz: impact point (world); w: crest ring radius (rad)
uniform vec2 u_tsu;        // tsunami crest amplitude a0, ring angular width (rad)
uniform float u_gpu;       // 1.0: shader displaces; 0.0: CPU already did
uniform float u_base_r;    // undisplaced water shell radius

varying vec3 v_normal;
varying vec3 v_view;
varying float v_anom;      // sea-surface anomaly, scene units
varying float v_wave;      // tsunami crest component only

void main() {
    vec3 dir = normalize(position);
    float cosPsi = dot(dir, u_moon_dir);
    float tide = u_tide_amp * (1.5 * cosPsi * cosPsi - 0.5);

    // Expanding tsunami crest: Gaussian ring at angular radius u_tsu_dir.w.
    float psi = acos(clamp(dot(dir, u_tsu_dir.xyz), -1.0, 1.0));
    float x = (psi - u_tsu_dir.w) / max(u_tsu.y, 1e-4);
    float ring = u_tsu.x * exp(-(x * x));

    float disp = u_gpu * (tide + ring);
    v_wave = ring;
    // CPU mode: the mesh positions already carry the anomaly, read it back.
    v_anom = disp + (1.0 - u_gpu) * (length(position) - u_base_r);

    vec3 p = position + dir * disp;
    vec3 world = (u_model * vec4(p, 1.0)).xyz;
    v_normal = normalize((u_model * vec4(normal.xyz, 0.0)).xyz);
    v_view = u_cam_pos - world;
    gl_Position = Projection * vec4(world, 1.0);
}
"#;

pub const WATER_FS: &str = r#"#version 100
precision mediump float;

varying vec3 v_normal;
varying vec3 v_view;
varying float v_anom;
varying float v_wave;

uniform vec3 u_light_dir;
uniform float u_anom_scale; // anomaly that maps to full red/blue

void main() {
    vec3 n = normalize(v_normal);
    vec3 v = normalize(v_view);
    float lambert = max(dot(n, u_light_dir), 0.0);
    // Fresnel-ish rim: more opaque and brighter at grazing angles.
    float fres = pow(1.0 - max(dot(n, v), 0.0), 2.0);
    vec3 deep = vec3(0.04, 0.20, 0.45);
    vec3 lit  = vec3(0.22, 0.45, 0.75);
    vec3 c = mix(deep, lit, lambert) + fres * vec3(0.10, 0.20, 0.35);

    // Diverging sea-surface anomaly colormap: red = water gain (m up),
    // blue = depression (m down), neutral at the undisturbed level.
    // Scale is asymmetric: the equilibrium trough sits at -A/2, so normalise
    // dips against half the gain scale to keep them readable.
    float s = max(u_anom_scale, 1e-4);
    float t = clamp(v_anom / (v_anom >= 0.0 ? s : 0.5 * s), -1.0, 1.0);
    c = mix(c, vec3(0.82, 0.13, 0.10), 0.70 * max(t, 0.0));
    c = mix(c, vec3(0.05, 0.25, 0.95), 0.80 * max(-t, 0.0));
    // Breaking-white foam along tsunami crests.
    float foam = smoothstep(0.004, 0.011, v_wave);
    c = mix(c, vec3(0.90, 0.93, 0.95) * (0.35 + 0.75 * lambert), foam);

    float a = min(0.40 + 0.35 * fres + 0.35 * abs(t), 0.88);
    gl_FragColor = vec4(c, a);
}
"#;

// Gravity anomaly overlay: EGM96 sea-level free-air map wrapped onto a shell
// just above the ground. Alpha follows the colormap's colour saturation, so
// near-zero anomalies stay transparent and only the strong features paint.
pub const GRAVITY_VS: &str = r#"#version 100
attribute vec3 position;
attribute vec2 texcoord;
attribute vec4 color0;
attribute vec4 normal;

uniform mat4 Projection;
uniform mat4 u_model;

varying vec2 v_uv;
varying vec3 v_normal;

void main() {
    v_uv = texcoord;
    v_normal = normalize((u_model * vec4(normal.xyz, 0.0)).xyz);
    gl_Position = Projection * u_model * vec4(position, 1.0);
}
"#;

pub const GRAVITY_FS: &str = r#"#version 100
precision mediump float;

varying vec2 v_uv;
varying vec3 v_normal;

uniform sampler2D Texture;
uniform vec3 u_light_dir;
uniform float u_alpha;

void main() {
    vec4 tex = texture2D(Texture, v_uv);
    float lambert = max(dot(normalize(v_normal), u_light_dir), 0.0);
    // The baker's diverging colormap grows more colourful with anomaly
    // strength; channel spread measures that strength without decoding it.
    float mx = max(max(tex.r, tex.g), tex.b);
    float mn = min(min(tex.r, tex.g), tex.b);
    float a = u_alpha * smoothstep(0.04, 0.35, mx - mn);
    vec3 c = tex.rgb * (0.25 + 0.85 * lambert);
    gl_FragColor = vec4(c, a);
}
"#;

// Generic unlit textured overlay (temperature map, current/wind streaks):
// alpha = texel alpha * u_alpha; shell radius/blend live in the material.
pub const TEX_VS: &str = r#"#version 100
attribute vec3 position;
attribute vec2 texcoord;
attribute vec4 color0;
attribute vec4 normal;

uniform mat4 Projection;
uniform mat4 u_model;

varying vec2 v_uv;

void main() {
    v_uv = texcoord;
    gl_Position = Projection * u_model * vec4(position, 1.0);
}
"#;

pub const TEX_FS: &str = r#"#version 100
precision mediump float;

varying vec2 v_uv;

uniform sampler2D Texture;
uniform float u_alpha;

void main() {
    vec4 t = texture2D(Texture, v_uv);
    gl_FragColor = vec4(t.rgb, t.a * u_alpha);
}
"#;

// Unlit tinted shell for impact flash / plume / dust haze (u_color rgba).
pub const GLOW_VS: &str = r#"#version 100
attribute vec3 position;
attribute vec2 texcoord;
attribute vec4 color0;
attribute vec4 normal;

uniform mat4 Projection;
uniform mat4 u_model;

void main() {
    gl_Position = Projection * u_model * vec4(position, 1.0);
}
"#;

pub const GLOW_FS: &str = r#"#version 100
precision mediump float;

uniform vec4 u_color;

void main() {
    gl_FragColor = u_color;
}
"#;

// Cloud shell: textured lambert layer just above the surface. `u_drift`
// offsets longitude (GPU mode fakes the circulation by scrolling a frozen
// texture; CPU mode leaves it 0 and updates the texture itself).
pub const CLOUD_VS: &str = r#"#version 100
attribute vec3 position;
attribute vec2 texcoord;
attribute vec4 color0;
attribute vec4 normal;

uniform mat4 Projection;
uniform mat4 u_model;
uniform float u_drift;

varying vec2 v_uv;
varying vec3 v_normal;

void main() {
    v_uv = texcoord + vec2(u_drift, 0.0);
    v_normal = normalize((u_model * vec4(normal.xyz, 0.0)).xyz);
    gl_Position = Projection * u_model * vec4(position, 1.0);
}
"#;

pub const CLOUD_FS: &str = r#"#version 100
precision mediump float;

varying vec2 v_uv;
varying vec3 v_normal;

uniform sampler2D Texture;
uniform vec3 u_light_dir;

void main() {
    vec4 t = texture2D(Texture, vec2(fract(v_uv.x), v_uv.y));
    float lambert = max(dot(normalize(v_normal), u_light_dir), 0.0);
    vec3 c = vec3(0.97, 0.98, 1.0) * (0.28 + 0.95 * lambert);
    // Thin cloud = faint veil, dense cloud = near-solid deck — real overcast
    // is patchy, so boost contrast between the two. Nightside clouds are dark
    // *and* translucent so the surface below still reads through the deck.
    float dens = t.a;
    float opaque = smoothstep(0.35, 0.75, dens);
    float a = (0.30 * dens + 0.85 * opaque) * (0.35 + 0.65 * smoothstep(0.0, 0.35, lambert));
    gl_FragColor = vec4(c, min(a, 1.0) * 0.95);
}
"#;

// Atmosphere shell: additive Rayleigh-ish rim glow. Brightest at the limb
// and on the dayside, with extra forward scatter toward the Sun.
pub const ATMO_VS: &str = r#"#version 100
attribute vec3 position;
attribute vec2 texcoord;
attribute vec4 color0;
attribute vec4 normal;

uniform mat4 Projection;
uniform vec3 u_cam_pos;

varying vec3 v_normal;
varying vec3 v_view;

void main() {
    v_normal = normalize(normal.xyz);
    v_view = position - u_cam_pos;
    gl_Position = Projection * vec4(position, 1.0);
}
"#;

pub const ATMO_FS: &str = r#"#version 100
precision mediump float;

varying vec3 v_normal;
varying vec3 v_view;

uniform vec3 u_light_dir;

void main() {
    vec3 n = normalize(v_normal);
    vec3 v = normalize(v_view);
    // Grazing view rays traverse the most air -> glow concentrates at the limb.
    float rim = 1.0 - abs(dot(n, v));
    float lit = max(dot(n, u_light_dir), 0.0);
    float forward = pow(max(dot(v, u_light_dir), 0.0), 2.0);
    float a = pow(rim, 3.2) * (0.28 + 1.1 * lit + 0.5 * forward);
    a = min(a, 0.9);
    vec3 col = mix(vec3(0.23, 0.44, 0.95), vec3(0.72, 0.86, 1.0), pow(lit, 0.65));
    gl_FragColor = vec4(col * a, a);
}
"#;

// Unlit textured sky sphere (drawn from the inside, no depth write).
pub const SKY_VS: &str = r#"#version 100
attribute vec3 position;
attribute vec2 texcoord;
attribute vec4 color0;
attribute vec4 normal;

uniform mat4 Projection;

varying vec2 v_uv;

void main() {
    v_uv = texcoord;
    gl_Position = Projection * vec4(position, 1.0);
}
"#;

pub const SKY_FS: &str = r#"#version 100
precision mediump float;

varying vec2 v_uv;

uniform sampler2D Texture;

void main() {
    gl_FragColor = vec4(texture2D(Texture, v_uv).rgb, 1.0);
}
"#;
