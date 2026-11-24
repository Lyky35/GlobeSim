# GlobeSim

A real-time 3D simulation of planet Earth: a topographic globe rotating at the
true sidereal rate, with the Moon orbiting at its true sidereal period and a
translucent ocean whose water is raised into the classic **tidal bulge** by the
Moon's gravity.

The main implementation is **Rust** ([macroquad](https://github.com/not-fl3/macroquad),
OpenGL, no heavy engine) — a native desktop app where the tide surface can be
computed on **either the GPU (vertex shader) or the CPU (per-vertex Rust)**,
switchable live with one key. A browser prototype (three.js) lives in
[`web/`](web/) and is [played online here](https://lyky35.github.io/GlobeSim/).

![GlobeSim native render](docs/screenshot.png)

## Build & run (Rust)

```sh
cargo run --release
```

Run from the repo root (it loads textures from `assets/textures/`). First build
takes a minute; the binary lands in `target/release/globesim`.

Useful flags (mainly for benchmarking/screenshots):

```sh
cargo run --release -- --cpu          # start in CPU tide mode
cargo run --release -- --tide=600     # start with a 600 km bulge
cargo test                            # unit tests for the tide math & CPU mesh pass
```

### Controls

| Key | Action |
|---|---|
| drag / wheel | orbit camera / zoom |
| `space` | pause / resume |
| `c` / `g` | tide computed on CPU / on GPU |
| `[` `]` | tide height −/+ 25 km (0–1000 km) |
| `-` `=` | sim speed ÷2 / ×2 (×0.01 … ×1 000 000) |
| `m` | toggle Moon |
| `f` | toggle dashed gravity vector (Earth ← Moon) |
| `a` | toggle rotation axis + equator |
| `r` | reset camera |
| `h` | toggle this help |

## Features

- **Topographic Earth** — NOAA ETOPO1 global shaded-relief map (topography +
  ocean bathymetry, 3840×1920, public domain), mapped onto a 64×160 sphere grid.
- **True rotation** — one turn per sidereal day (23 h 56 m 04 s), axial tilt
  23.44°.
- **Moon** — orbits once per sidereal month (27.32 d), prograde, inclination
  5.15°, optional dashed gravity vector toward Earth. (Drawn at 6.5 Earth
  radii instead of the real 60 so it stays on screen.)
- **Tidal bulge** — equilibrium tide
  `h(ψ) = A·(3cos²ψ − 1)/2` pinned to the Moon's direction: water piles up on
  the Moon-facing side *and* the antipode, while the ground rotates underneath,
  so any fixed point sees **two high tides per lunar day** (~24 h 50 m cycle).
- **CPU *or* GPU tide** —
  - *GPU*: the displacement runs in the vertex shader; the CPU only updates two
    uniforms per frame.
  - *CPU*: all 10 400 ocean vertices are re-positioned *and* re-normalled in
    Rust every frame (~0.3 ms/frame measured; the HUD shows the number).
- **Extras** — Milky Way sky sphere, MSAA, HUD with fps / sim time / measured
  tide-pass time / real-time high-tide interval.

## The physics, briefly

The Moon pulls harder on the near side of the ocean than on the solid Earth,
and harder on the solid Earth than on the far-side ocean. After subtracting the
mean (orbital) acceleration, what's left is a *differential* (tide-generating)
force that stretches the water into two bulges along the Earth–Moon line. The
real equilibrium amplitude is only ~0.36 m — about 0.000006% of Earth's radius,
invisible at planetary scale — so the sim exaggerates it and tells you by how
much. The Moon's gravity here is kinematic (the equilibrium-tide formula), not
an N-body integration; real ocean tides additionally involve basin resonance,
Coriolis and friction, which produce *co-tidal* patterns rather than one smooth
bulge. Roadmap items below include a full shallow-water solver.

## Web prototype (`web/`)

The original three.js version (no build step, vendored modules, same physics,
live CPU/GPU switch with sliders) runs off any static server:

```sh
python3 -m http.server 8080   # then open http://localhost:8080/web/
```

It's live on GitHub Pages: <https://lyky35.github.io/GlobeSim/> (root page
redirects to `web/`).

## Texture credits

See [CREDITS.md](CREDITS.md). TL;DR: NOAA ETOPO relief (public domain),
Solar System Scope Moon & Milky Way (CC BY 4.0).

## Roadmap

- [ ] Real DEM displacement of the land sphere (ETOPO grid -> bump/displacement map)
- [ ] GPU *compute-shader* tide variant (vs. today's vertex-shader path) and a CPU side-thread variant
- [ ] WASM build of the Rust app (macroquad targets the web) to replace the three.js prototype
- [ ] Shallow-water (Laplace tidal) equations on the sphere — co-tidal points, amphidromes
- [ ] Solar tide component (spring/neap cycles)
- [ ] Moon ephemeris (real orbital elements) and an actual Sun position for day/night
- [ ] City markers + local tide-height graph

## License

MIT (code). Textures have their own licenses — see [CREDITS.md](CREDITS.md).
