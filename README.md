# GlobeSim

A real-time 3D simulation of planet Earth: a topographic globe rotating at the
true sidereal rate, with the Moon orbiting at its true sidereal period and a
translucent ocean whose water is raised into the classic **tidal bulge** by the
Moon's gravity — wrapped in a live-simulated cloud deck and a scattering
atmosphere.

The main implementation is **Rust** ([macroquad](https://github.com/not-fl3/macroquad),
OpenGL, no heavy engine) — a native desktop app where the tide surface can be
computed on **either the GPU (vertex shader) or the CPU (per-vertex Rust)**,
switchable live with one key. The original three.js browser prototype (in
[`web/`](web/)) is a frozen v0 — kept for reference, far behind the native app.

![GlobeSim native render](docs/screenshot.png)

## Download

Prebuilt binaries are attached to every
[release](https://github.com/Lyky35/GlobeSim/releases/latest) — the game loads
its textures from the `assets/` folder beside the executable, so run it from
inside the extracted folder:

**Linux x86-64**

```sh
tar xzf globesim-v0.1.0-linux-x86_64.tar.gz
cd globesim-v0.1.0-linux-x86_64 && ./globesim
```

**Windows x86-64** — unzip and run `globesim.exe` (double-click works too,
the exe finds its `assets/` next to itself):

```
globesim-v0.1.0-windows-x86_64.zip  →  globesim.exe
```

Any GL 3.3-or-newer GPU will do (macroquad loads OpenGL at runtime); the Linux
build runs on Wayland and X11. macOS comes from source below — macroquad is
cross-platform, I just ship binaries for what I can run.

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
cargo run --release -- --clouds=off   # start with the cloud deck off
cargo run --release -- --impact=apophis    # Apophis-class strike once framing settles
cargo run --release -- --impact=chicxulub  # 10-km killer for scale
cargo run --release -- --no-anomaly        # start with the gravity anomaly map off
cargo run --release -- --temp              # start with the temperature overlay on
cargo test                            # unit tests: tide math, CPU mesh, clouds, impact physics
```

### Controls

| Key | Action |
|---|---|
| drag / wheel | orbit camera / zoom |
| `space` | pause / resume |
| `c` / `g` | tide computed on CPU / on GPU |
| `[` `]` | tide height −/+ 25 km (0–1000 km) |
| `-` `=` | sim speed ÷2 / ×2 (×0.01 … ×1 000 000) |
| `k` | clouds: live CPU sim → GPU drift → off |
| `o` | atmosphere (scattering glow shell) on/off |
| `m` | toggle Moon |
| `f` | toggle dashed gravity vector (Earth ← Moon) |
| `v` | toggle the gravity anomaly map (EGM96 free-air overlay) |
| `t` | toggle the surface temperature map (air temp, blue cold → red warm) |
| `w` | toggle wind streaks (animated tracers in the surface wind belts) |
| `n` | toggle ocean-current streaks (animated tracers in the gyres) |
| `a` | toggle rotation axis + equator |
| `i` | Apophis-class impact (340 m, 1 176 Mt) under the camera |
| `u` | Chicxulub-class impact (10 km, ~75 Gt) under the camera |
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
- **Atmosphere** —
  - *Cloud deck*: a 256×128 lat/lon cloud-fraction field (**CPU**, ~1 ms/frame)
    advected semi-Lagrangian by a three-band surface wind (trade easterlies at
    ±15°, mid-latitude westerlies at ±47°, and a Hadley/Ferrel meridional cell).
    Clouds grow where insolation is high (moisture biased to the tropics) and
    decay elsewhere; the subsolar point sweeping west once per sidereal day
    winds the growth into helical bands. The HUD reports the measured pass time
    and the mean cloud cover.
  - *GPU mode* (`k`): a frozen cloud texture scrolled by one uniform — the same
    "cheap vs. simulated" switch as the tide.
  - *Scattering glow*: an additive rayleigh-ish shell, brightest at the limb and
    on the dayside, with extra forward-scatter toward the Sun.
- **Sea-surface anomaly colouring** — the water shell is tinted by its height
  anomaly in metres: **red = water gain, blue = depression**, neutral at the
  undisturbed level (full red/blue at ±the current tide amplitude, dips scaled
  ×2 since the equilibrium trough is −A/2). Works identically on both compute
  paths: the GPU shader colours `tide + tsunami` straight from the formula, the
  CPU pass reads the anomaly back off the displaced mesh. The HUD prints the
  full-scale value in metres.
- **Gravity anomaly map** (`v`, on by default) — Earth's *own* gravity field
  painted over the globe: a 1440×720 texture baked offline by a standalone Rust
  program that synthesises the sea-level (free-air) gravity anomaly from the
  full **degree-360 EGM96** spherical-harmonic model (EGM96 + CORRCOEF, public
  domain, distributed by GeographicLib) using the Holmes & Featherstone
  fully-normalised Legendre recursion — the same algorithm as GeographicLib's
  `SphericalEngine` — evaluated on the WGS84 ellipsoid. Diverging colormap at
  ±120 mGal: **red = stronger gravity** (mid-ocean ridges, island arcs, the
  Himalayan mass excess), **blue = weaker** (ocean trenches, the Hudson Bay
  glacial-isostatic low); overlay alpha tracks colour saturation, so near-zero
  regions stay transparent and the terrain shows through. The baker validates
  itself: the geoid heights from the same coefficients reproduce the official
  NGA EGM96 geoid grid (WW15MGH) with correlation 0.9999.
- **Ocean currents, winds & temperature** (`n` `w` `t`) — a CPU circulation
  layer on the cloud grid: the wind belts drive a depth-integrated ocean gyre
  (Sverdrup interior + narrow **western boundary jets** — the Gulf Stream and
  Kuroshio fall out of the β term rather than hand-drawn arrows), while surface
  air and sea-surface temperature relax toward radiative equilibrium and are
  advected by winds and currents. Cloud shade cools the sea surface, impact
  dust chills the whole planet. The streak canvases and temperature overlay
  show what these flows *change*: warm western-boundary pools, a narrowed
  equator-to-pole contrast, and the HUD prints the numbers — jet speed vs
  gyre interior, oceanic heat transport in PW, mean and polar air temperature.
- **Asteroid impacts** (`i` / `u`, or `--impact=`) — an Apophis-class strike
  (340 m, 6.2×10¹⁰ kg, 12.6 km/s → **1 176 Mt**, ~78 000 Hiroshimas) or a
  Chicxulub-class 10 km rock for scale. The console prints a physics briefing
  and the globe shows the consequences: fireball flash and rising ejecta plume,
  an expanding **tsunami crest ring** (deep-ocean wave speed √(g·4 km) ≈
  198 m/s, Gaussian crest with cylindrical spreading — painted in the red/blue
  anomaly colormap with foam on the crest), an orange seismic ring racing over
  the ground at ~4.5 km/s, a pale airblast ring in the atmosphere, and global
  dust loading: Apophis ≈ 1% (regional disaster, climate untouched) versus
  Chicxulub ≈ 90% — an **impact winter**: the dust dims insolation in the cloud
  model and the deck collapses, replaced by a brown soot veil.
- **Extras** — Milky Way sky sphere, MSAA, HUD with fps / sim time / measured
  tide-pass time / real-time high-tide interval.

## The physics, briefly

The Moon pulls harder on the near side of the ocean than on the solid Earth,
and harder on the solid Earth than on the far-side ocean. After subtracting the
mean (orbital) acceleration, what's left is a *differential* (tide-generating)
force that stretches the water into two bulges along the Earth–Moon line. The
real equilibrium amplitude is only ~0.36 m — about 0.000006% of Earth's radius,
invisible at planetary scale — so the sim exaggerates it and tells you by how
much. The Sun raises a second, weaker bulge (0.46 of the Moon's tide-raising
power — the Sun's enormous mass is mostly cancelled by its enormous distance),
so the two bulges beat against each other: lined up at new and full moon you
get spring tides, crossed at the quarters, neaps. Both lights are placed by
truncated real ephemerides (Meeus' solar longitude, Brown's leading lunar
terms), which brings the seasons and the Moon's inclined, speed-varying orbit
along for free. The Moon's gravity here is kinematic (the equilibrium-tide
formula), not
an N-body integration; real ocean tides additionally involve basin resonance,
Coriolis and friction, which produce *co-tidal* patterns rather than one smooth
bulge. Roadmap items below include a full shallow-water solver.

The cloud deck runs a miniature general circulation: a surface wind field
(trade easterlies at ±15°, westerlies at ±47°, a Hadley/Ferrel meridional cell)
advects cloud water semi-Lagrangian, while insolation-driven convection grows
it on the dayside and slow decay burns it off elsewhere. Because the subsolar
point sweeps west once per sidereal day in the ground frame, growth organises
itself into the helical cloud bands you see from orbit. The backtrace transport
implements `-v·∇c` but not the `-c∇·v` compression term (the wind field is not
divergence-free), so a global mass fixer restores the conserved total each
step — the same trick real GCMs use.

The gravity anomaly texture solves the *forward* problem of geodesy: given the
full Earth gravity field as spherical harmonics (EGM96: 65 341 coefficients to
degree/order 360, i.e. features down to ~110 km), the sea-level gravity
anomaly is `Δg(φ,λ) = (1/R)·Σₙ (n−1)·(GM/a)·(a/r)ⁿ⁺¹·Σₘ P̄ₙₘ(sin φ′)·[Cₙₘ cos mλ + Sₙₘ sin mλ]`
evaluated on the ellipsoid — the radial derivative of the disturbing potential,
computed with the fully-normalised stable recursion of Holmes & Featherstone
(2002) and WGS84 normal-gravity subtraction. What it shows is geology: mid-ocean
ridges read positive (hot, dense mantle upwelling), deep-sea trenches negative
(cold slab + deep water), and Hudson Bay still negative from the ice age that
left it ~200 m of mantle depression — isostasy runs on geological time.

The ocean currents solve the steady wind-driven barotropic balance
`ν∇²ψ + β·∂ψ/∂x = curl(wind stress)` — Gauss–Seidel on a land/sea mask
classified straight from the ETOPO pixels — with β = 2Ωcosφ/R, the
north–south gradient of the Coriolis parameter. The interior obeys Sverdrup
balance (meridional flow = stress curl ÷ β: a few cm/s, *equatorward* under
the westerlies), and since β keeps its sign in both hemispheres each gyre can
only close its circulation through a boundary layer ~ν/β wide on the *west*
side of every basin — Stommel's western intensification. That is what makes
western boundary currents: jets an order of magnitude faster than the
interior, at the real basins' west edges (Gulf Stream, Kuroshio, Agulhas,
Brazil). Advected by those jets, the SST field grows the warm pools downstream
of each western boundary, and the heat the currents carry poleward — the HUD
prints it — ~0.8 PW for this one-layer model once the SST has spun up,
against the real ~1.3 PW — is what lifts the poles; the unit tests assert the
jet structure, the downstream warming and the poleward transport, plus the
Chicxulub case where dust blocks sunlight while the surface still radiates to
space and the surface temperature collapses by more than 10 °C — and an
`#[ignore]`d regression test pins the Gulf Stream, Kuroshio, California, Peru
and Brazil currents to their real latitudes and longitudes on the real mask.
Caveats: one
ocean layer (no depth profile, no eddy resolution at
1.4°), winds are idealised belts, and the land mask treats white ice sheets as
land and pale Arctic sea ice as water.

An impact is booked from order-of-magnitude scaling laws, shared by the CPU and
GPU render paths: energy `½mv²` (Apophis: 4.9×10¹⁸ J = 1 176 Mt); transient
crater diameter from Collins et al. (2005) gravity-regime scaling
(`1.161·(ρi/ρt)^⅓·L^0.78·v^0.44·g^−0.22`, ×1.3 for the final crater → ~6 km
for Apophis, ~100 km for Chicxulub); seismic `M_w = (log₁₀E_rad − 4.8)/1.5`
with ~10⁻⁴ of the energy coupled into quake waves (M6.6 / M9.8). The ocean
punched out of the crater refalls as a Gaussian crest ring running at the
deep-water long-wave speed √(g·h) ≈ 198 m/s, spreading as 1/(1+r/4000 km) and
fading over days; an orange seismic ring (4.5 km/s) and an acoustic airblast
ring (340 m/s) race ahead of it. Ejecta dust scales with energy and decays on
a ~3-year residence time, feeding straight back into the cloud model's
insolation term — which is why the 10-km case starves convection and turns
into an impact winter. All heights remain exaggerated for visibility, in the
same convention as the tide, and the anomaly colormap reports the exaggerated
metres the scene actually draws.

## Original browser prototype (`web/`)

The very first version, in three.js (no build step, vendored modules): kept
purely as the project's starting point, **frozen and far behind the native
app** — no impacts, no gravity map, no climate layer, and it is not maintained.
The roadmap item that matters here is a WASM build of the Rust app to retire
it properly.

## Texture credits

See [CREDITS.md](CREDITS.md). TL;DR: NOAA ETOPO relief (public domain),
Solar System Scope Moon & Milky Way (CC BY 4.0), EGM96 gravity coefficients
(public domain, via GeographicLib).

## Roadmap

- [ ] Real DEM displacement of the land sphere (ETOPO grid -> bump/displacement map)
- [ ] GPU *compute-shader* tide variant (vs. today's vertex-shader path) and a CPU side-thread variant
- [ ] WASM build of the Rust app (macroquad targets the web) to replace the three.js prototype
- [ ] Shallow-water (Laplace tidal) equations on the sphere — co-tidal points, amphidromes
- [x] Solar tide component (spring/neap cycles)
- [x] Moon ephemeris (real orbital elements) and an actual Sun position for day/night
- [ ] City markers + local tide-height graph

## License

MIT (code). Textures have their own licenses — see [CREDITS.md](CREDITS.md).
