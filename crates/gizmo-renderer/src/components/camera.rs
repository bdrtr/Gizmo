use gizmo_math::Vec3;

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Camera {
    pub fov: f32,
    pub near: f32,
    pub far: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub primary: bool,
}

impl Camera {
    pub fn new(fov: f32, near: f32, far: f32, yaw: f32, pitch: f32, primary: bool) -> Self {
        Self { fov, near, far, yaw, pitch, primary }
    }

    pub fn get_projection(&self, aspect: f32) -> gizmo_math::Mat4 {
        gizmo_math::Mat4::perspective_rh(self.fov, aspect, self.near, self.far)
    }

    pub fn get_view(&self, position: Vec3) -> gizmo_math::Mat4 {
        let front = self.get_front();
        gizmo_math::Mat4::look_at_rh(position, position + front, Vec3::new(0.0, 1.0, 0.0))
    }

    pub fn get_front(&self) -> Vec3 {
        let fx = self.yaw.cos() * self.pitch.cos();
        let fy = self.pitch.sin();
        let fz = self.yaw.sin() * self.pitch.cos();
        Vec3::new(fx, fy, fz).normalize()
    }

    pub fn get_right(&self) -> Vec3 {
        self.get_front().cross(Vec3::new(0.0, 1.0, 0.0)).normalize()
    }
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Camera2D {
    pub zoom: f32,
    pub primary: bool,
}

impl Camera2D {
    pub fn new(zoom: f32) -> Self {
        Self { zoom, primary: true }
    }

    pub fn get_projection(&self, width: f32, height: f32) -> gizmo_math::Mat4 {
        let hw = (width / 2.0) / self.zoom;
        let hh = (height / 2.0) / self.zoom;
        gizmo_math::Mat4::orthographic_rh(-hw, hw, -hh, hh, -1000.0, 1000.0)
    }
}
