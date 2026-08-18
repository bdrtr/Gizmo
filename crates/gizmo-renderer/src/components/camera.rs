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
    Orthographic {
        /// The vertical extent of the view volume, in world units.
        height: f32,
    },
}

/// A 3-D camera: where it looks, how far it sees, and how the result is projected.
///
/// The camera's **position** is not here — it comes from the entity's `Transform`, which is why
/// [`Camera::get_view`] takes one. Its orientation, though, is: yaw and pitch rather than a
/// quaternion, because the controllers that drive a camera all think in those two angles and
/// round-tripping them through a quaternion loses the roll-free invariant.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Camera {
    /// Vertical field of view, in radians. Perspective only.
    pub fov: f32,
    /// The near clip plane, in metres. Everything closer is clipped away.
    pub near: f32,
    /// The far clip plane. The near/far ratio is what depth precision costs, so a near plane of
    /// 0.001 with a far of 10 000 will z-fight.
    pub far: f32,
    /// Rotation about the world Y axis, in radians.
    pub yaw: f32,
    /// Rotation above/below the horizon, in radians, clamped just short of ±90° — at exactly
    /// vertical the view basis is degenerate.
    pub pitch: f32,
    /// Exposure multiplier applied to the whole frame, before tone mapping.
    pub exposure: f32,
    /// Whether this is the camera the frame is rendered from. With several primaries the first
    /// one found wins, which is a scene authoring error rather than a supported configuration.
    pub primary: bool,
    /// Perspective (default) or orthographic projection. `#[serde(default)]` keeps
    /// scenes saved before this field was added loading as perspective.
    #[serde(default)]
    pub projection: ProjectionMode,
}

impl Camera {
    /// A perspective camera, with every argument corrected into a usable range: a non-positive
    /// FOV or near plane becomes 0.001, a far plane is pushed to at least `near + 0.1`, yaw is
    /// wrapped into one turn and pitch clamped just short of vertical.
    ///
    /// Corrected rather than rejected because these come from scene files and editor fields as
    /// often as from code, and a camera that renders nothing is a harder thing to diagnose than
    /// one that renders slightly wrong.
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
    /// Switches between perspective and orthographic.
    ///
    /// `distance` is how far the subject is: going orthographic, it is what the ortho height is
    /// derived from, so the subject keeps the on-screen size it had under perspective. Without
    /// that the view appears to jump.
    pub fn toggle_projection(&mut self, distance: f32) {
        self.projection = match self.projection {
            ProjectionMode::Perspective => ProjectionMode::Orthographic {
                height: 2.0 * distance.abs().max(0.001) * (self.fov * 0.5).tan(),
            },
            ProjectionMode::Orthographic { .. } => ProjectionMode::Perspective,
        };
    }

    /// Tidies the angles to stop them accumulating without bound: yaw modulo TAU, pitch
    /// clamped.
    /// Wraps yaw into one turn and clamps pitch away from vertical — the same correction
    /// [`Camera::new`] applies, for callers that have written the fields directly.
    pub fn sanitize_angles(&mut self) {
        self.yaw %= std::f32::consts::TAU;
        self.pitch = self.pitch.clamp(
            -std::f32::consts::PI / 2.0 + 0.001,
            std::f32::consts::PI / 2.0 - 0.001,
        );
    }

    /// The projection matrix for a viewport of the given aspect ratio (width / height),
    /// perspective or orthographic according to [`Self::projection`].
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

    /// The view matrix for a camera at `position` looking along its yaw/pitch. The position is
    /// taken as an argument because it lives on the entity's `Transform`, not here.
    pub fn get_view(&self, position: Vec3) -> gizmo_math::Mat4 {
        let front = self.get_front();
        let right = self.get_right();
        let up = right.cross(front);
        gizmo_math::Mat4::look_at_rh(position, position + front, up)
    }

    /// The world-space forward (aim) vector from yaw/pitch. [`get_front`] AND the first-person
    /// camera controller (`FpsLook`, gizmo-engine::systems) share it, so the "aim direction"
    /// maths is not re-written BY HAND in every demo and game.
    pub fn forward_from(yaw: f32, pitch: f32) -> Vec3 {
        let pitch = pitch.clamp(
            -std::f32::consts::PI / 2.0 + 0.001,
            std::f32::consts::PI / 2.0 - 0.001,
        );
        Vec3::new(
            yaw.cos() * pitch.cos(),
            pitch.sin(),
            yaw.sin() * pitch.cos(),
        )
        .normalize()
    }

    /// The INVERSE of [`forward_from`](Self::forward_from): the yaw/pitch needed to look along a
    /// direction.
    ///
    /// This formula was written out by hand in four separate places in the repository (studio
    /// setup, the fighting camera's look-at, focusing, and the viewport's axis gizmo) — each
    /// under the same two-line "Invert `get_front()`" comment. One of them was written *wrong*:
    /// `x.atan2(-z)` / `(-y).asin()`, which does not invert `get_front` and points the camera
    /// somewhere else entirely. The inverse now lives next to the function it inverts, and the
    /// two have to change together.
    ///
    /// Two degenerate cases, both named instead of silently answered wrongly:
    /// - **A zero or non-finite direction** → `None`. Today that path writes a NaN yaw into the
    ///   camera through `atan2(NaN, NaN)` and kills the view matrix; "nowhere" has no angle.
    /// - **Looking straight up or down** → yaw is undefined (`atan2(0, 0)` returns `0` without
    ///   complaint and swings the scene to world +X), so it is not invented: `fallback_yaw` is
    ///   carried through instead.
    ///
    /// Because `forward_from` keeps pitch 0.001 rad short of vertical, an exactly vertical
    /// direction does not come back bit-identical through a round trip. That is the whole
    /// difference.
    pub fn yaw_pitch_from_forward(dir: Vec3, fallback_yaw: f32) -> Option<(f32, f32)> {
        let d = dir.normalize_or_zero();
        if d == Vec3::ZERO {
            return None;
        }
        let pitch = d.y.clamp(-1.0, 1.0).asin();
        let yaw = if d.x.abs() + d.z.abs() < 1e-4 {
            fallback_yaw
        } else {
            d.z.atan2(d.x)
        };
        Some((yaw, pitch))
    }

    /// The world-space right vector from yaw (horizontal). The closed form of
    /// forward × (0,1,0).
    pub fn right_from(yaw: f32) -> Vec3 {
        Vec3::new(-yaw.sin(), 0.0, yaw.cos())
    }

    /// The world-space direction this camera is aimed.
    pub fn get_front(&self) -> Vec3 {
        Self::forward_from(self.yaw, self.pitch)
    }

    /// The world-space right vector, horizontal regardless of pitch.
    pub fn get_right(&self) -> Vec3 {
        Self::right_from(self.yaw)
    }

    /// Build a world-space picking ray from a screen/cursor pixel through this
    /// camera — the engine's screen→world unproject (à la Bevy's
    /// `Camera::viewport_to_world`). Combine with `PhysicsWorld::raycast` (or a
    /// plane intersection) to pick / drag the object under the cursor.
    ///
    /// * `screen` — cursor position in pixels, origin **top-left** (matches
    ///   [`gizmo_core`]'s `Input::mouse_position`).
    /// * `viewport` — framebuffer size in the same pixels (e.g. `WindowInfo`).
    /// * `world_pos` — the camera's world position (its `Transform.position`,
    ///   since the view matrix takes the position separately).
    ///
    /// The heavy lifting (NDC → world via the inverse view-projection, with
    /// singular-matrix / degenerate-direction guards) is [`gizmo_math::Ray::from_ndc`].
    pub fn screen_to_ray(
        &self,
        screen: (f32, f32),
        viewport: (f32, f32),
        world_pos: Vec3,
    ) -> gizmo_math::Ray {
        let (w, h) = (viewport.0.max(1.0), viewport.1.max(1.0));
        // Pixel → NDC: x∈[-1,1] rightward, y∈[-1,1] UPward (flip the top-left screen y).
        let ndc = gizmo_math::Vec2::new((screen.0 / w) * 2.0 - 1.0, 1.0 - (screen.1 / h) * 2.0);
        let view_proj_inv = (self.get_projection(w / h) * self.get_view(world_pos)).inverse();
        gizmo_math::Ray::from_ndc(ndc, view_proj_inv)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_to_ray_center_points_along_camera_front() {
        // Camera at (0,0,10) looking down -Z (yaw = -90°, pitch = 0 → front = (0,0,-1)).
        let cam = Camera::new(
            std::f32::consts::FRAC_PI_2,
            0.1,
            100.0,
            -std::f32::consts::FRAC_PI_2,
            0.0,
            true,
        );
        let pos = Vec3::new(0.0, 0.0, 10.0);
        // Centre pixel of a 200x100 viewport → NDC (0,0) → ray along the camera front.
        let ray = cam.screen_to_ray((100.0, 50.0), (200.0, 100.0), pos);
        assert!((ray.direction.z - (-1.0)).abs() < 1e-4, "centre ray looks -Z, got {:?}", ray.direction);
        assert!(ray.direction.x.abs() < 1e-4 && ray.direction.y.abs() < 1e-4);
        assert!((ray.direction.length() - 1.0).abs() < 1e-5, "direction normalized");
    }

    #[test]
    fn screen_to_ray_offset_pixels_tilt_the_ray() {
        let cam = Camera::new(
            std::f32::consts::FRAC_PI_2,
            0.1,
            100.0,
            -std::f32::consts::FRAC_PI_2,
            0.0,
            true,
        );
        let pos = Vec3::new(0.0, 0.0, 10.0);
        // Right of centre → ray tilts +X; below centre (larger screen-y) → ray tilts -Y.
        let right = cam.screen_to_ray((150.0, 50.0), (200.0, 100.0), pos);
        assert!(right.direction.x > 0.05, "right pixel tilts +X, got {:?}", right.direction);
        let down = cam.screen_to_ray((100.0, 90.0), (200.0, 100.0), pos);
        assert!(down.direction.y < -0.05, "lower pixel tilts -Y, got {:?}", down.direction);
    }

    #[test]
    fn new_sanitizes_out_of_range_inputs() {
        // Degenerate fov/near, far ≤ near, huge yaw, over-vertical pitch.
        let cam = Camera::new(
            -1.0,
            -1.0,
            -5.0,
            10.0 * std::f32::consts::TAU + 0.5,
            5.0,
            true,
        );
        assert!(cam.fov >= 0.001);
        assert!(cam.near >= 0.001);
        assert!(cam.far >= cam.near + 0.1, "far must sit past near, got {}", cam.far);
        assert!(cam.yaw.abs() <= std::f32::consts::TAU, "yaw wrapped, got {}", cam.yaw);
        assert!(
            cam.pitch < std::f32::consts::FRAC_PI_2 && cam.pitch > -std::f32::consts::FRAC_PI_2,
            "pitch clamped below vertical, got {}",
            cam.pitch
        );
    }

    #[test]
    fn forward_and_right_are_orthonormal() {
        let (yaw, pitch) = (0.7f32, 0.3f32);
        let f = Camera::forward_from(yaw, pitch);
        let r = Camera::right_from(yaw);
        assert!((f.length() - 1.0).abs() < 1e-5, "forward not unit: {f:?}");
        assert!((r.length() - 1.0).abs() < 1e-5, "right not unit: {r:?}");
        assert!(r.y.abs() < 1e-6, "right must stay horizontal: {r:?}");
        assert!(f.dot(r).abs() < 1e-5, "right ⟂ forward expected, dot={}", f.dot(r));
    }

    #[test]
    fn forward_with_zero_pitch_is_horizontal() {
        let f = Camera::forward_from(0.0, 0.0);
        assert!(f.y.abs() < 1e-6, "zero pitch → horizontal aim, got {f:?}");
        // Pitch beyond vertical is clamped, so y never reaches ±1.
        let up = Camera::forward_from(0.0, std::f32::consts::PI); // way over vertical
        assert!(up.y.abs() < 1.0);
        assert!((up.length() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn toggle_projection_is_an_involution_matching_the_fov_framing() {
        let mut cam = Camera::new(std::f32::consts::FRAC_PI_2, 0.1, 100.0, 0.0, 0.0, true);
        assert!(matches!(cam.projection, ProjectionMode::Perspective));

        cam.toggle_projection(10.0);
        match cam.projection {
            ProjectionMode::Orthographic { height } => {
                let expected = 2.0 * 10.0 * (cam.fov * 0.5).tan();
                assert!((height - expected).abs() < 1e-3, "height {height} vs {expected}");
            }
            _ => panic!("expected orthographic after first toggle"),
        }
        // Toggling again returns to perspective (involution).
        cam.toggle_projection(10.0);
        assert!(matches!(cam.projection, ProjectionMode::Perspective));
    }

    #[test]
    fn sanitize_angles_wraps_yaw_and_clamps_pitch() {
        let mut cam = Camera::new(std::f32::consts::FRAC_PI_2, 0.1, 100.0, 0.0, 0.0, true);
        cam.yaw = 100.0;
        cam.pitch = 5.0;
        cam.sanitize_angles();
        assert!(cam.yaw.abs() <= std::f32::consts::TAU);
        assert!(cam.pitch < std::f32::consts::FRAC_PI_2 && cam.pitch > -std::f32::consts::FRAC_PI_2);
    }

    /// The 3-D camera's projection is finite in both modes — and orthographic is the mode a 2-D
    /// view uses, which is why the separate `Camera2D` type could go: it re-implemented this with
    /// a zoom instead of a height, and no draw path ever read it.
    #[test]
    fn projections_are_finite_in_both_modes() {
        let mut cam = Camera::new(std::f32::consts::FRAC_PI_2, 0.1, 100.0, 0.0, 0.0, true);
        assert!(cam.get_projection(1.777).to_cols_array().iter().all(|v| v.is_finite()));

        cam.toggle_projection(10.0); // the distance the ortho height is derived from
        let ortho = cam.get_projection(1.777);
        assert!(ortho.to_cols_array().iter().all(|v| v.is_finite()));
        assert!(
            matches!(cam.projection, ProjectionMode::Orthographic { .. }),
            "the toggle must actually reach orthographic"
        );
    }
}
