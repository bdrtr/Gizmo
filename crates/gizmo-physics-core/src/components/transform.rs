use gizmo_math::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};
#[cfg(feature = "reflect")]
use bevy_reflect::Reflect;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "reflect", derive(Reflect))]
pub struct TransformData {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "reflect", derive(Reflect))]
#[serde(from = "TransformData")]
pub struct Transform {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
    #[serde(skip)]
    #[cfg_attr(feature = "reflect", reflect(ignore))]
    pub local_matrix: Mat4,
}

impl From<TransformData> for Transform {
    fn from(data: TransformData) -> Self {
        let mut t = Self {
            position: data.position,
            rotation: data.rotation,
            scale: data.scale,
            local_matrix: Mat4::IDENTITY,
        };
        t.update_local_matrix();
        t
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::new(Vec3::ZERO)
    }
}

impl Transform {
    pub fn new(position: Vec3) -> Self {
        let mut t = Self {
            position,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
            local_matrix: Mat4::IDENTITY,
        };
        t.update_local_matrix();
        t
    }

    pub fn with_position(mut self, position: Vec3) -> Self {
        self.position = position;
        self.update_local_matrix();
        self
    }

    pub fn with_scale(mut self, scale: Vec3) -> Self {
        self.scale = scale;
        self.update_local_matrix();
        self
    }

    pub fn with_rotation(mut self, rotation: Quat) -> Self {
        self.rotation = rotation;
        self.update_local_matrix();
        self
    }

    pub fn set_position(&mut self, pos: Vec3) {
        self.position = pos;
        self.update_local_matrix();
    }

    pub fn set_rotation(&mut self, rot: Quat) {
        self.rotation = rot;
        self.update_local_matrix();
    }

    pub fn set_scale(&mut self, scale: Vec3) {
        self.scale = scale;
        self.update_local_matrix();
    }

    /// X ekseni etrafında döndürür (radyan).
    #[inline]
    pub fn rotate_x(&mut self, angle: f32) {
        self.rotation *= Quat::from_rotation_x(angle);
        self.update_local_matrix();
    }

    /// Y ekseni etrafında döndürür (radyan).
    #[inline]
    pub fn rotate_y(&mut self, angle: f32) {
        self.rotation *= Quat::from_rotation_y(angle);
        self.update_local_matrix();
    }

    /// Z ekseni etrafında döndürür (radyan).
    #[inline]
    pub fn rotate_z(&mut self, angle: f32) {
        self.rotation *= Quat::from_rotation_z(angle);
        self.update_local_matrix();
    }

    /// Mevcut pozisyona bir delta ekler.
    #[inline]
    pub fn translate(&mut self, delta: Vec3) {
        self.position += delta;
        self.update_local_matrix();
    }

    pub fn update_local_matrix(&mut self) {
        self.local_matrix =
            Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.position);
    }

    pub fn world_matrix(&self, parent: Option<&Transform>) -> Mat4 {
        match parent {
            Some(p) => p.world_matrix(None) * self.local_matrix,
            None => self.local_matrix,
        }
    }
}

gizmo_core::impl_component!(Transform);

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GlobalTransform {
    #[serde(skip)]
    pub matrix: Mat4,
}

impl Default for GlobalTransform {
    fn default() -> Self {
        Self {
            matrix: Mat4::IDENTITY,
        }
    }
}

impl GlobalTransform {
    pub fn compute_matrix(&self) -> Mat4 {
        self.matrix
    }
}

gizmo_core::impl_component!(GlobalTransform);

// ─────────────────────────────────────────────────────────────────────────────
// Tests — local matrix caching, serde rebuild of the skipped matrix, hierarchy
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-5;

    #[test]
    fn local_matrix_matches_scale_rotation_translation() {
        let t = Transform::new(Vec3::new(1.0, 2.0, 3.0))
            .with_rotation(Quat::from_rotation_y(0.5))
            .with_scale(Vec3::splat(2.0));
        let expected = Mat4::from_scale_rotation_translation(
            Vec3::splat(2.0),
            Quat::from_rotation_y(0.5),
            Vec3::new(1.0, 2.0, 3.0),
        );
        let p = Vec3::new(1.0, -0.5, 0.25);
        assert!(
            (t.local_matrix.transform_point3(p) - expected.transform_point3(p)).length() < EPS
        );
    }

    #[test]
    fn serde_from_data_rebuilds_skipped_matrix() {
        // `local_matrix` is #[serde(skip)]; deserialization goes through
        // `From<TransformData>`, which must rebuild it from S/R/T (not leave IDENTITY).
        let data = TransformData {
            position: Vec3::new(4.0, 5.0, 6.0),
            rotation: Quat::from_rotation_z(0.3),
            scale: Vec3::splat(1.5),
        };
        let t = Transform::from(data);
        let expected =
            Mat4::from_scale_rotation_translation(data.scale, data.rotation, data.position);
        let p = Vec3::new(1.0, 1.0, 1.0);
        assert!(
            (t.local_matrix.transform_point3(p) - expected.transform_point3(p)).length() < EPS,
            "matrix must be rebuilt on deserialize, not left as identity"
        );
        assert_ne!(t.local_matrix, Mat4::IDENTITY);
    }

    #[test]
    fn setters_keep_matrix_in_sync() {
        let mut t = Transform::new(Vec3::ZERO);
        t.set_scale(Vec3::splat(3.0));
        // Scaling by 3 maps (1,1,1) → (3,3,3).
        assert!((t.local_matrix.transform_point3(Vec3::ONE) - Vec3::splat(3.0)).length() < EPS);
        t.set_position(Vec3::new(10.0, 0.0, 0.0));
        assert!(
            (t.local_matrix.transform_point3(Vec3::ZERO) - Vec3::new(10.0, 0.0, 0.0)).length()
                < EPS
        );
    }

    #[test]
    fn world_matrix_composes_parent() {
        let parent = Transform::new(Vec3::new(10.0, 0.0, 0.0));
        let child = Transform::new(Vec3::new(1.0, 0.0, 0.0));
        // Child origin = parent(10) ∘ child(1) = 11 along X.
        let world = child.world_matrix(Some(&parent));
        assert!(
            (world.transform_point3(Vec3::ZERO) - Vec3::new(11.0, 0.0, 0.0)).length() < EPS
        );
        // No parent → local only.
        let solo = child.world_matrix(None);
        assert!((solo.transform_point3(Vec3::ZERO) - Vec3::new(1.0, 0.0, 0.0)).length() < EPS);
    }

    #[test]
    fn rotate_y_accumulates_onto_rotation() {
        let mut t = Transform::new(Vec3::ZERO);
        t.rotate_y(std::f32::consts::FRAC_PI_2);
        // +90° about Y sends +X to -Z.
        let img = t.local_matrix.transform_vector3(Vec3::X);
        assert!((img - Vec3::new(0.0, 0.0, -1.0)).length() < 1e-4, "{img:?}");
    }

    #[test]
    fn translate_adds_delta() {
        let mut t = Transform::new(Vec3::new(1.0, 1.0, 1.0));
        t.translate(Vec3::new(2.0, 0.0, -1.0));
        assert!((t.position - Vec3::new(3.0, 1.0, 0.0)).length() < EPS);
        // Matrix reflects the moved origin.
        assert!(
            (t.local_matrix.transform_point3(Vec3::ZERO) - Vec3::new(3.0, 1.0, 0.0)).length()
                < EPS
        );
    }
}
