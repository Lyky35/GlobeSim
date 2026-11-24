use macroquad::prelude::*;

/// Minimal mouse/keyboard orbit camera (drag to rotate, wheel to zoom).
pub struct OrbitCam {
    pub yaw: f32,
    pub pitch: f32,
    pub dist: f32,
    /// Default framing, restored by reset().
    pub yaw0: f32,
    pub pitch0: f32,
    pub dist0: f32,
    prev_mouse: Vec2,
    dragging: bool,
}

impl OrbitCam {
    pub fn new() -> Self {
        Self {
            yaw: 0.0,
            pitch: 0.35,
            dist: 3.4,
            yaw0: 0.0,
            pitch0: 0.35,
            dist0: 3.4,
            prev_mouse: Vec2::ZERO,
            dragging: false,
        }
    }

    pub fn reset(&mut self) {
        self.yaw = self.yaw0;
        self.pitch = self.pitch0;
        self.dist = self.dist0;
    }

    pub fn position(&self) -> Vec3 {
        vec3(
            self.dist * self.pitch.cos() * self.yaw.sin(),
            self.dist * self.pitch.sin(),
            self.dist * self.pitch.cos() * self.yaw.cos(),
        )
    }

    pub fn handle_input(&mut self) {
        let mouse = mouse_position().into();
        if is_mouse_button_pressed(MouseButton::Left) {
            self.dragging = true;
            self.prev_mouse = mouse;
        }
        if is_mouse_button_released(MouseButton::Left) {
            self.dragging = false;
        }
        if self.dragging {
            let d = mouse - self.prev_mouse;
            self.yaw -= d.x * 0.005;
            self.pitch = (self.pitch + d.y * 0.005).clamp(-1.45, 1.45);
        }
        self.prev_mouse = mouse;

        let (_x, wheel) = mouse_wheel();
        if wheel != 0.0 {
            self.dist = (self.dist * (1.0 - wheel * 0.08)).clamp(1.3, 60.0);
        }
    }

    pub fn camera3d(&self) -> Camera3D {
        Camera3D {
            position: self.position(),
            target: Vec3::ZERO,
            up: Vec3::Y,
            fovy: 45f32.to_radians(),
            ..Default::default()
        }
    }
}
