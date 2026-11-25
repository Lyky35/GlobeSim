# tools

## `bake_gravity.rs` — regenerates `assets/textures/gravity_anomaly.jpg`

Standalone Rust (std only, `rustc -O`), no crates. It synthesises the
sea-level (free-air) gravity anomaly from the degree-360 EGM96 spherical
harmonic model and writes a 1440×720 equirectangular PPM whose orientation
matches `SphereGrid`'s UVs (top row = north pole, left column = lon 0° E).
A diverging white→red/blue colormap at ±120 mGal full scale produces the
in-app texture; the shader turns colour saturation into overlay alpha.

Coefficient inputs (public domain, GeographicLib gravity distribution):

```sh
# EGM96 + CORRCOEF complete coefficient set (egm96.egm.cof):
curl -L -o egm96.zip https://sourceforge.net/projects/geographiclib/files/gravity-distrib/egm96.zip/download
# WGS84 normal field, for the zonal terms subtracted from C(n,0) (wgs84.egm.cof):
curl -L -o wgs84.zip https://sourceforge.net/projects/geographiclib/files/gravity-distrib/wgs84.zip/download
unzip egm96.zip && unzip wgs84.zip   # -> gravity/egm96.egm.cof, gravity/wgs84.egm.cof
```

```sh
rustc -O tools/bake_gravity.rs -o bake_gravity
./bake_gravity gravity/egm96.egm.cof gravity/wgs84.egm.cof gravity_anomaly.ppm
magick gravity_anomaly.ppm -quality 90 assets/textures/gravity_anomaly.jpg
```

It also writes `<out>.geoid.bin`: the raw f64 geoid-height grid (Helmert
normal gravity, `N = T/γ₀`), a self-check of the same harmonic machinery —
sanity values: New Guinea ≈ +71 m, Indian Ocean minimum ≈ −106 m. Against
the official NGA EGM96 geoid grid (WW15MGH.GRD, 15′ ASCII GMT grid) this
grid correlates at **r = 0.9999** with no roll or flip.

The math follows GeographicLib's `GravityModel::SphericalAnomaly` /
`SphericalEngine`: fully-normalised, Condon-Shortley-free associated
Legendre recursion (Holmes & Featherstone, *AIG* 2002, Eqs. 11/13) — the
convention the `.cof` files are baked in — with `C00 = 1` and the WGS84
zonal terms removed from the m = 0 column, evaluated on the ellipsoid and
scaled by `Δg = (GM/R)·Σₙ (n−1)·(a/R)ⁿ⁺¹·[…]`.
