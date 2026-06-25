use gizmo_math::Vec3;

/// How a [`Camera`] projects the scene onto the screen.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub enum ProjectionMode {
    /// Perspective projection (the default), using the camera's `fov`.
    #[default]
    Perspective,
    /// Orthographic projection. `height` is the vertical extent of the view
    /// volume in world units; the width is derived from the aspect ratio.
    Orthographic { height: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Camera {
    pub fov: f32,
    pub near: f32,
    pub far: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub exposure: f32, // Fiziksel kamera pozlaması (EV tabanlı veya doğrudan çarpan)
    pub primary: bool,
    /// Perspective (default) or orthographic projection. `#[serde(default)]` keeps
    /// scenes saved before this field was added loading as perspective.
    #[serde(default)]
    pub projection: ProjectionMode,
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
            exposure: 1.0, // Varsayılan pozlama 1.0
            primary,
            projection: ProjectionMode::Perspective,
        }
    }

    /// Toggles between perspective and orthographic projection. When switching to
    /// orthographic, the vertical extent is chosen so the framing roughly matches
    /// the current perspective `fov` at the given `distance` from the camera.
    pub fn toggle_projection(&mut self, distance: f32) {
        self.projection = match self.projection {
            ProjectionMode::Perspective => ProjectionMode::Orthographic {
                height: 2.0 * distance.abs().max(0.001) * (self.fov * 0.5).tan(),
            },
            ProjectionMode::Orthographic { .. } => ProjectionMode::Perspective,
        };
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
        match self.projection {
            ProjectionMode::Perspective => {
                gizmo_math::Mat4::perspective_rh(self.fov, aspect, self.near, self.far)
            }
            ProjectionMode::Orthographic { height } => {
                let half_h = (height * 0.5).max(0.001);
                let half_w = half_h * aspect.max(0.001);
                gizmo_math::Mat4::orthographic_rh(
                    -half_w, half_w, -half_h, half_h, self.near, self.far,
                )
            }
        }
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

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
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
