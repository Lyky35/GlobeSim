use crate::sphere::SphereGrid;
use macroquad::prelude::*;

// ---------------------------------------------------------------------------
// Real land relief from a baked ETOPO-style height grid (see
// tools/bake_dem.rs). Land vertices are pushed radially outward once at
// startup - the relief is static, so unlike the tides there is no per-frame
// pass: the displaced mesh simply rotates with the Earth model matrix.
// Ocean vertices stay put (the water shell covers them), keeping coastlines
// as the silhouette where displaced land meets the bare sphere.
// ---------------------------------------------------------------------------

/// Baked height grid: rows north -> south, columns lon 0 -> 360 deg E
/// (both endpoints included), int16 metres, little-endian, "GDEM" magic.
pub struct Dem {
    w: u32,
    h: u32,
    data: Vec<i16>,
}

impl Dem {
    pub fn load(path: &str) -> Dem {
        let bytes = std::fs::read(path)
            .unwrap_or_else(|e| panic!("cannot read DEM ({e}) — run from the repo root"));
        if bytes.len() < 16 || &bytes[0..4] != b"GDEM" {
            panic!("{path} is not a GDEM file (run tools/bake_dem.rs to rebuild it)");
        }
        let g = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
        let (w, h, ver) = (g(4) as usize, g(8) as usize, g(12));
        if ver != 1 || bytes.len() != 16 + w * h * 2 {
            panic!("{path}: unexpected GDEM layout (v{ver}, {w}x{h})");
        }
        let data = (0..w * h)
            .map(|k| i16::from_le_bytes(bytes[16 + k * 2..18 + k * 2].try_into().unwrap()))
            .collect();
        Dem {
            w: w as u32,
            h: h as u32,
            data,
        }
    }

    /// Bilinear height at (lat, lon) in degrees, metres. Longitude wraps,
    /// latitude clamps at the poles.
    pub fn height_m(&self, lat: f32, lon: f32) -> f32 {
        let u = lon.rem_euclid(360.0) / 360.0 * (self.w - 1) as f32;
        let v = (90.0 - lat.clamp(-90.0, 90.0)) / 180.0 * (self.h - 1) as f32;
        let (u0, v0) = (u.floor() as usize, v.floor() as usize);
        let (tx, ty) = (u - u0 as f32, v - v0 as f32);
        let v1 = (v0 + 1).min(self.h as usize - 1);
        let w = self.w as usize;
        let at = |r: usize, c: usize| self.data[r * w + c % w] as f32;
        (at(v0, u0) * (1.0 - tx) + at(v0, u0 + 1) * tx) * (1.0 - ty)
            + (at(v1, u0) * (1.0 - tx) + at(v1, u0 + 1) * tx) * ty
    }

    pub fn max_m(&self) -> f32 {
        self.data.iter().copied().max().unwrap_or(0) as f32
    }
}

/// Radial displacement of a height, in scene units (1 = one Earth radius).
/// Ocean heights (<= 0) displace nothing.
pub fn relief_offset_m(h_m: f32, radius_km: f32, exaggeration: f32) -> f32 {
    if h_m > 0.0 {
        h_m / (radius_km * 1000.0) * exaggeration
    } else {
        0.0
    }
}

/// Push land vertices out along their radial direction and recompute
/// normals from the displaced grid (same finite-difference scheme the CPU
/// tide pass uses). One-shot at startup.
pub fn displace_land(mesh: &mut Mesh, grid: &SphereGrid, dem: &Dem, exaggeration: f32) {
    let r_km = 6371.0; // EARTH_RADIUS_KM, but keep the module dependency-free
    let disp = |i: u32, j: u32| -> Vec3 {
        let d = grid.dir(i, j);
        let lat = 90.0 - i as f32 * 180.0 / grid.rings as f32;
        let lon = j as f32 * 360.0 / grid.slices as f32;
        let off = relief_offset_m(dem.height_m(lat, lon), r_km, exaggeration);
        d * (1.0 + off)
    };

    // Pass 1: positions.
    for i in 0..=grid.rings {
        for j in 0..grid.slices {
            mesh.vertices[grid.index(i, j)].position = disp(i, j);
        }
    }

    // Pass 2: normals from grid neighbours (wrapping longitude, clamped
    // latitude at the poles).
    for i in 0..=grid.rings {
        for j in 0..grid.slices {
            let jm = (j + grid.slices - 1) % grid.slices;
            let du = disp(i, j + 1) - disp(i, jm);
            let (i0, i1) = if i == 0 {
                (0, 1)
            } else if i == grid.rings {
                (grid.rings - 1, grid.rings)
            } else {
                (i - 1, i + 1)
            };
            let dv = disp(i1, j) - disp(i0, j);
            // At a pole every vertex of the row sits on the same point, so du
            // collapses to zero and the cross product degenerates. The poles
            // are local caps anyway: take the radial direction. Otherwise
            // orient the finite-difference normal against the sphere
            // direction (same winding fix the CPU tide pass uses).
            let d = grid.dir(i, j);
            let n = if i == 0 || i == grid.rings {
                d
            } else {
                let mut n = du.cross(dv).normalize();
                if n.dot(d) < 0.0 {
                    n = -n;
                }
                n
            };
            mesh.vertices[grid.index(i, j)].normal = vec4(n.x, n.y, n.z, 0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synth_dem() -> Dem {
        // 1-deg grid, ocean everywhere except a +4000 m blob centred at
        // (lat 0, lon 0) spanning a few degrees.
        let (w, h) = (361u32, 181u32);
        let data = vec![0i16; (w * h) as usize];
        let mut dem = Dem { w, h, data };
        for r in 0..h as usize {
            for c in 0..w as usize {
                let lat = 90.0 - r as f32;
                let lon = c as f32;
                let lon = if lon > 180.0 { lon - 360.0 } else { lon };
                if lat.abs() < 3.0 && lon.abs() < 3.0 {
                    dem.data[r * w as usize + c] = 4000;
                }
            }
        }
        dem
    }

    #[test]
    fn bilinear_samples_land_and_wraps_longitude() {
        let dem = synth_dem();
        assert!((dem.height_m(0.0, 0.0) - 4000.0).abs() < 1.0);
        assert!((dem.height_m(10.0, 0.0)).abs() < 1.0); // open ocean
                                                        // lon -0.5 and lon 359.5 are the same point; both read the blob edge.
        let a = dem.height_m(0.0, -0.5);
        let b = dem.height_m(0.0, 359.5);
        assert!((a - b).abs() < 1e-3);
        assert!(a > 3500.0);
    }

    #[test]
    fn exaggeration_maps_metres_to_scene_radii() {
        // 8 km at x15 over a 6371 km Earth: 0.0188 Re (~120 km).
        let off = relief_offset_m(8000.0, 6371.0, 15.0);
        assert!((off - 0.018835).abs() < 1e-4);
        assert_eq!(relief_offset_m(-5000.0, 6371.0, 15.0), 0.0); // seafloor stays
        assert_eq!(relief_offset_m(0.0, 6371.0, 15.0), 0.0);
    }

    #[test]
    fn land_blobs_lift_ocean_stays_and_normals_recover() {
        let grid = SphereGrid {
            rings: 18,
            slices: 36,
        };
        let mut mesh = grid.mesh(1.0, None);
        let dem = synth_dem();
        displace_land(&mut mesh, &grid, &dem, 15.0);

        // Vertex at (lat 0, lon 0): grid.dir(9, 0) -> +Z; inside the blob.
        let land = &mesh.vertices[grid.index(9, 0)];
        let r = land.position.length();
        assert!(r > 1.005 && r < 1.015, "blob radius {r}");
        // Same latitude, lon 180 (grid index 18): plain ocean sphere.
        let sea = &mesh.vertices[grid.index(9, 18)];
        assert!((sea.position.length() - 1.0).abs() < 1e-6);

        // Every normal unit-length and (mostly) outward.
        for v in &mesh.vertices {
            let n = vec3(v.normal.x, v.normal.y, v.normal.z);
            assert!((n.length() - 1.0).abs() < 1e-4);
            assert!(n.dot(v.position.normalize()) > 0.5);
        }
    }

    #[test]
    fn baked_asset_parses_with_real_earth_extremes() {
        // Only when run from the repo root (same condition as the app).
        let Ok(dem) = std::fs::read("assets/textures/etopo_dem.bin")
            .map(|_| Dem::load("assets/textures/etopo_dem.bin"))
        else {
            return;
        };
        // Stored heights are 0.5-deg box means, so extremes are damped:
        assert!(dem.max_m() > 5000.0, "Himalaya missing: {}", dem.max_m());
        assert!(dem.height_m(11.5, 142.0) < -6500.0, "Mariana missing");
        assert!(dem.height_m(27.9, 86.9) > 4500.0, "Himalaya missing");
        assert!(dem.height_m(-60.0, -170.0) < 0.0, "Pacific must be ocean");
    }
}
