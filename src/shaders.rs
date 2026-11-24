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

varying vec3 v_normal;
varying vec3 v_view;

void main() {
    vec3 dir = normalize(position);
    float cosPsi = dot(dir, u_moon_dir);
    float tide = u_tide_amp * (1.5 * cosPsi * cosPsi - 0.5);
    vec3 p = position + dir * tide;

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

uniform vec3 u_light_dir;

void main() {
    vec3 n = normalize(v_normal);
    vec3 v = normalize(v_view);
    float lambert = max(dot(n, u_light_dir), 0.0);
    // Fresnel-ish rim: more opaque and brighter at grazing angles.
    float fres = pow(1.0 - max(dot(n, v), 0.0), 2.0);
    vec3 deep = vec3(0.04, 0.20, 0.45);
    vec3 lit  = vec3(0.22, 0.45, 0.75);
    vec3 c = mix(deep, lit, lambert) + fres * vec3(0.10, 0.20, 0.35);
    gl_FragColor = vec4(c, 0.40 + 0.35 * fres);
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
