use macroquad::prelude::*;

/// A lat/lon sphere grid laid out in row-major order:
/// row `i` in `0..=rings` runs from the north pole (lat +90) to the south pole,
/// column `j` in `0..slices` runs longitude 0..360.
/// Vertex index of (i, j) is `i * slices + j` — this layout lets the CPU tide
/// pass derive surface normals from grid neighbours.
#[derive(Clone, Copy)]
pub struct SphereGrid {
    pub rings: u32,
    pub slices: u32,
}

impl SphereGrid {
    pub fn vertex_count(&self) -> usize {
        (self.rings as usize + 1) * self.slices as usize
    }

    pub fn index(&self, i: u32, j: u32) -> usize {
        (i * self.slices + j % self.slices) as usize
    }

    /// Unit direction and equirectangular UV of grid point (i, j).
    pub fn dir(&self, i: u32, j: u32) -> Vec3 {
        let lat = (90.0 - i as f32 * 180.0 / self.rings as f32).to_radians();
        let lon = (j as f32 * 360.0 / self.slices as f32).to_radians();
        vec3(lat.cos() * lon.sin(), lat.sin(), lat.cos() * lon.cos())
    }

    fn uv(&self, i: u32, j: u32) -> Vec2 {
        vec2(j as f32 / self.slices as f32, i as f32 / self.rings as f32)
    }

    /// Triangle indices, counter-clockwise as seen from outside (front faces).
    pub fn indices(&self) -> Vec<u16> {
        let mut idx = Vec::with_capacity(self.rings as usize * self.slices as usize * 6);
        for i in 0..self.rings {
            for j in 0..self.slices {
                let a = self.index(i, j) as u16;
                let b = self.index(i + 1, j) as u16;
                let c = self.index(i, j + 1) as u16;
                let d = self.index(i + 1, j + 1) as u16;
                idx.extend_from_slice(&[a, b, c, b, d, c]);
            }
        }
        idx
    }

    /// A macroquad mesh: sphere of `radius`, UV-mapped equirectangularly
    /// (v = 0 at north pole, matching the source maps top-down).
    pub fn mesh(&self, radius: f32, texture: Option<Texture2D>) -> Mesh {
        let count = self.vertex_count();
        let mut vertices = Vec::with_capacity(count);
        for i in 0..=self.rings {
            for j in 0..self.slices {
                let d = self.dir(i, j);
                vertices.push(Vertex {
                    position: d * radius,
                    uv: self.uv(i, j),
                    color: Color::new(1.0, 1.0, 1.0, 1.0).into(),
                    normal: vec4(d.x, d.y, d.z, 0.0),
                });
            }
        }
        Mesh {
            vertices,
            indices: self.indices(),
            texture,
        }
    }
}
