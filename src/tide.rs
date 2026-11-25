use crate::constants::SOLAR_TIDE_RATIO;
use crate::impact::{wave_h, WaveState};
use crate::sphere::SphereGrid;
use macroquad::prelude::*;

// ---------------------------------------------------------------------------
// The ocean: a translucent water sphere displaced by the Moon's
// tide-generating (differential gravity) potential.
//
// Equilibrium tide height as a function of psi, the angular distance from
// the sub-lunar point:
//
//     h(psi) = A * (3 cos^2(psi) - 1) / 2
//
// The classic two-bulge shape: +A toward the Moon, +A at the antipode,
// -A/2 at 90 degrees. The bulge is pinned to the Moon direction while the
// ground rotates underneath, so any fixed point sees two high tides per
// lunar day.
//
// Two computation paths, switchable at runtime:
//   * GPU: the vertex shader (see shaders.rs) displaces every vertex; the
//     CPU only updates two uniforms per frame.
//   * CPU: this module recomputes every vertex position *and* normal in
//     Rust each frame.
// ---------------------------------------------------------------------------

/// Equilibrium tide height at angular distance psi from the sub-lunar point.
pub fn tide_height(cos_psi: f32, amplitude: f32) -> f32 {
    amplitude * (1.5 * cos_psi * cos_psi - 0.5)
}

/// Two-body equilibrium tide: lunar bulge plus the Sun's 0.46-strength
/// bulge. Where the two add (new/full moon) spring tides peak; where they
/// cross (quarters) neap tides flatten out.
pub fn tide_height2(cos_moon: f32, cos_sun: f32, amplitude: f32) -> f32 {
    tide_height(cos_moon, amplitude) + SOLAR_TIDE_RATIO * tide_height(cos_sun, amplitude)
}

pub enum TideMode {
    Cpu,
    Gpu,
}

pub struct TideOcean {
    pub grid: SphereGrid,
    pub mesh: Mesh,
    /// Unit direction of every vertex (undeformed sphere).
    dirs: Vec<Vec3>,
    base_radius: f32,
    /// ms spent in the CPU pass last frame (HUD stat).
    pub last_cpu_ms: f64,
}

impl TideOcean {
    pub fn new(radius: f32, rings: u32, slices: u32, texture: Option<Texture2D>) -> Self {
        let grid = SphereGrid { rings, slices };
        let mesh = grid.mesh(radius, texture);
        let dirs = (0..=rings)
            .flat_map(|i| (0..slices).map(move |j| grid.dir(i, j)))
            .collect();
        Self {
            grid,
            mesh,
            dirs,
            base_radius: radius,
            last_cpu_ms: 0.0,
        }
    }

    pub fn vertex_count(&self) -> usize {
        self.grid.vertex_count()
    }

    /// Displaced radius of grid point (i, j) — also used for finite-difference
    /// normals, so neighbours must wrap longitude and clamp latitude.
    fn displaced(
        &self,
        i: u32,
        j: u32,
        moon: Vec3,
        sun: Vec3,
        amp: f32,
        wave: Option<&WaveState>,
    ) -> Vec3 {
        let d = self.dirs[self.grid.index(i, j % self.grid.slices)];
        let h = self.base_radius
            + tide_height2(d.dot(moon), d.dot(sun), amp)
            + wave.map_or(0.0, |w| wave_h(d, w));
        d * h
    }

    /// CPU pass: reposition and re-normal every vertex. Normals come from
    /// central differences on the (wrapping) grid, falling back to one-sided
    /// differences at the poles.
    pub fn update_cpu(&mut self, moon: Vec3, sun: Vec3, amp: f32, wave: Option<&WaveState>) {
        let start = std::time::Instant::now();
        let grid = self.grid;
        let base = self.base_radius;

        // Pass 1: positions.
        for i in 0..=grid.rings {
            for j in 0..grid.slices {
                let idx = grid.index(i, j);
                let d = self.dirs[idx];
                let h = base
                    + tide_height2(d.dot(moon), d.dot(sun), amp)
                    + wave.map_or(0.0, |w| wave_h(d, w));
                self.mesh.vertices[idx].position = d * h;
            }
        }

        // Pass 2: normals from the deformed surface (field-split borrows:
        // `dirs` is read while `mesh` is written).
        for i in 0..=grid.rings {
            for j in 0..grid.slices {
                let idx = grid.index(i, j);
                let jm = (j + grid.slices - 1) % grid.slices;
                let du = self.displaced(i, j + 1, moon, sun, amp, wave)
                    - self.displaced(i, jm, moon, sun, amp, wave);
                let (i0, i1) = if i == 0 {
                    (0, 1)
                } else if i == grid.rings {
                    (grid.rings - 1, grid.rings)
                } else {
                    (i - 1, i + 1)
                };
                let dv = self.displaced(i1, j, moon, sun, amp, wave)
                    - self.displaced(i0, j, moon, sun, amp, wave);
                // Pole rows collapse du to zero (all vertices coincide), so
                // the cross product degenerates there: use the radial cap.
                let mut n = if i == 0 || i == grid.rings {
                    self.dirs[idx]
                } else {
                    du.cross(dv).normalize()
                };
                if n.dot(self.dirs[idx]) < 0.0 {
                    n = -n;
                }
                self.mesh.vertices[idx].normal = vec4(n.x, n.y, n.z, 0.0);
            }
        }

        self.last_cpu_ms = start.elapsed().as_secs_f64() * 1000.0;
    }

    /// Reset the mesh to the undeformed sphere (switching GPU->CPU boundaries,
    /// GPU mode must not inherit CPU-displaced positions).
    pub fn reset_shape(&mut self) {
        for (idx, v) in self.mesh.vertices.iter_mut().enumerate() {
            let d = self.dirs[idx];
            v.position = d * self.base_radius;
            v.normal = vec4(d.x, d.y, d.z, 0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bulge_shape() {
        let a = 0.1;
        // +A toward the Moon and at the antipode, -A/2 at 90 deg.
        assert!((tide_height(1.0, a) - a).abs() < 1e-6);
        assert!((tide_height(-1.0, a) - a).abs() < 1e-6);
        assert!((tide_height(0.0, a) + a / 2.0).abs() < 1e-6);
        // Volume preserving over the sphere: mean of (3cos^2-1)/2 is zero.
        // Midpoint sampling of cos(psi) over [-1, 1].
        let n = 1000;
        let mean: f32 = (0..n)
            .map(|k| tide_height(-1.0 + (k as f32 + 0.5) * 2.0 / n as f32, a))
            .sum::<f32>()
            / n as f32;
        assert!(mean.abs() < 1e-3, "mean tide height {mean} should be ~0");
    }

    #[test]
    fn cpu_mesh_update_moves_vertices_to_bulges() {
        // No GPU context needed: TideOcean::new only builds data.
        // Sun at 90 deg to the Moon (neap-ish): the sub-lunar radius is
        // 1 + A + 0.46*A*(1.5*0 - 0.5) = 1 + 0.1 - 0.023.
        let mut ocean = TideOcean::new(1.0, 32, 64, None);
        ocean.update_cpu(vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), 0.1, None);

        // Grid: lat 0 row is i == rings/2; lon 0 (j=0) is dir (0,0,1)... choose
        // directions by searching for the best dot products instead.
        let radius_at = |target: Vec3| -> f32 {
            ocean
                .mesh
                .vertices
                .iter()
                .map(|v| {
                    let d = v.position.normalize();
                    (d.dot(target), v.position.length())
                })
                .filter(|(d, _)| *d > 0.999)
                .map(|(_, r)| r)
                .max_by(|a, b| a.partial_cmp(b).unwrap())
                .map(|r| r)
                .unwrap_or(0.0)
        };
        let sub = radius_at(vec3(1.0, 0.0, 0.0));
        let anti = radius_at(vec3(-1.0, 0.0, 0.0));
        assert!(
            (sub - 1.077).abs() < 1e-3,
            "sub-lunar radius {sub} should be ~1.077"
        );
        assert!(
            (anti - 1.077).abs() < 1e-3,
            "antipode radius {anti} should be ~1.077"
        );

        // Normals near the 90-degree trough point slightly downhill: they must
        // still be unit length and point outward.
        for v in &ocean.mesh.vertices {
            let n = v.normal.truncate();
            assert!(
                (n.length() - 1.0).abs() < 1e-3,
                "normal must stay unit length"
            );
            assert!(n.dot(v.position) >= -1e-3, "normal must point outward");
        }
    }

    #[test]
    fn solar_term_gives_spring_and_neap_strengths() {
        let a = 0.1;
        // New/full moon (directions aligned): 1.46 * lunar amplitude.
        assert!((tide_height2(1.0, 1.0, a) - 1.46 * a).abs() < 1e-6);
        // Quarter moon (90 deg apart): sub-lunar point is down 0.46*A/2.
        assert!((tide_height2(1.0, 0.0, a) - 0.77 * a).abs() < 1e-6);
    }

    #[test]
    fn cpu_pass_raises_tsunami_ring() {
        let mut ocean = TideOcean::new(1.0005, 32, 64, None);
        let wave = WaveState {
            dir: vec3(1.0, 0.0, 0.0),
            front: 0.3,
            a0: 0.02,
            width: 0.04,
        };
        ocean.update_cpu(vec3(0.0, 0.0, 1.0), vec3(0.0, 0.0, 1.0), 0.0, Some(&wave));

        let radius_near = |target: Vec3| -> f32 {
            ocean
                .mesh
                .vertices
                .iter()
                .filter(|v| target.dot(v.position.normalize()) > 0.99)
                .map(|v| v.position.length())
                .fold(0.0f32, f32::max)
        };
        let crest = radius_near(vec3(0.3f32.cos(), 0.0, 0.3f32.sin()));
        let calm = radius_near(vec3(1.0f32.cos(), 0.0, 1.0f32.sin())); // 57 deg out
        assert!(crest > 1.015, "ring crest radius {crest}");
        assert!(calm < 1.002, "outside the ring {calm}");
    }
}
