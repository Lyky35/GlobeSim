# Texture & asset credits

| Asset | Source | License |
|---|---|---|
| `assets/textures/earth_topography_3840.jpg` | NOAA ETOPO Global Relief map, via [Wikimedia Commons](https://commons.wikimedia.org/wiki/File:NOAA_ETOPO_Global_Relief_map.jpg); rolled 180 deg so the left edge is lon 0 deg E (SphereGrid convention) | Public domain (NOAA) |
| `assets/textures/moon_2k.jpg` | [Solar System Scope](https://www.solarsystemscope.com/textures/) | CC BY 4.0 |
| `assets/textures/stars_milky_way_2k.jpg` | [Solar System Scope](https://www.solarsystemscope.com/textures/) | CC BY 4.0 |
| `assets/textures/gravity_anomaly.jpg` | Baked from the EGM96 gravity model (EGM96 + CORRCOEF coefficient sets, distributed by [GeographicLib](https://geographiclib.sourceforge.io/html/gravity.html)); validated against the NGA official geoid grid WW15MGH | Public domain (NGA/NIMA EGM96 — U.S. government work) |
| `assets/textures/etopo_dem.bin` | Baked from NOAA [ETOPO 2022](https://www.ncei.noaa.gov/products/etopo-global-relief-model) 60-arc-second bed topography (`ETOPO_2022_v1_60s_N90W180_bed.nc`), decimated to 0.5° by `tools/bake_dem.rs` | Public domain (NOAA — U.S. government work) |
| `vendor/three.module.js`, `vendor/addons/controls/OrbitControls.js` | [three.js r169](https://threejs.org) | MIT |

Attribution for CC BY 4.0 assets: *Textures by [Solar System Scope](https://www.solarsystemscope.com/textures/), licensed under [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).*
