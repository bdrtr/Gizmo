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
    pub fn new(
        mut fov: f32,
        mut near: f32,
        mut far: f32,
        mut yaw: f32,
        mut pitch: f32,
        primary: bool,
    ) -> Self {
        fov = fov.max(0.001);
        near = near.max(0.001);
        far = far.max(near + 0.1);
        yaw %= std::f32::consts::TAU;
        pitch = pitch.clamp(
            -std::f32::consts::PI / 2.0 + 0.001,
            std::f32::consts::PI / 2.0 - 0.001,
        );

        Self {
            fov,
            near,
            far,
            yaw,
            pitch,
            primary,
        }
    }

    /// Fazla birikmeyi önlemek icin acilari temizler (yaw mod TAU, pitch clamp)
    pub fn sanitize_angles(&mut self) {
        self.yaw %= std::f32::consts::TAU;
        self.pitch = self.pitch.clamp(
            -std::f32::consts::PI / 2.0 + 0.001,
            std::f32::consts::PI / 2.0 - 0.001,
        );
    }

    pub fn get_projection(&self, aspect: f32) -> gizmo_math::Mat4 {
        gizmo_math::Mat4::perspective_rh(self.fov, aspect, self.near, self.far)
    }

    pub fn get_view(&self, position: Vec3) -> gizmo_math::Mat4 {
        let front = self.get_front();
        let right = self.get_right();
        let up = right.cross(front);
        gizmo_math::Mat4::look_at_rh(position, position + front, up)
    }

    pub fn get_front(&self) -> Vec3 {
        let pitch = self.pitch.clamp(
            -std::f32::consts::PI / 2.0 + 0.001,
            std::f32::consts::PI / 2.0 - 0.001,
        );
        let fx = self.yaw.cos() * pitch.cos();
        let fy = pitch.sin();
        let fz = self.yaw.sin() * pitch.cos();
        Vec3::new(fx, fy, fz).normalize()
    }

    pub fn get_right(&self) -> Vec3 {
        // Front x (0,1,0) reduces mathematically to (-sin(yaw), 0, cos(yaw))
        Vec3::new(-self.yaw.sin(), 0.0, self.yaw.cos())
    }
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Camera2D {
    pub zoom: f32,
    pub primary: bool,
}

impl Camera2D {
    pub fn new(zoom: f32, primary: bool) -> Self {
        Self { zoom, primary }
    }

    pub fn get_projection(&self, width: f32, height: f32) -> gizmo_math::Mat4 {
        let safe_zoom = self.zoom.max(0.001);
        let hw = (width / 2.0) / safe_zoom;
        let hh = (height / 2.0) / safe_zoom;
        gizmo_math::Mat4::orthographic_rh(-hw, hw, -hh, hh, -1000.0, 1000.0)
    }
}
