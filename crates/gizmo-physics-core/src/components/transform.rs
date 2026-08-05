//! Transform components: an entity's authored local placement, and the world matrix
//! composed from it.
//!
//! [`Transform`] holds position/rotation/scale plus a cached `S·R·T` matrix and is the
//! transform the rigid-body integrator advances each step. [`GlobalTransform`] holds the
//! world matrix a hierarchy-propagation pass composes from those locals. [`TransformData`]
//! is only the serialized form of `Transform` — the cached matrix is derived data and is
//! rebuilt on load rather than persisted.

use gizmo_math::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};
#[cfg(feature = "reflect")]
use bevy_reflect::Reflect;

/// Serialized form of [`Transform`]: the three authored fields, without the derived matrix.
///
/// [`Transform`] is declared `#[serde(from = "TransformData")]`, so every deserialization
/// lands here first and then goes through `From<TransformData>`, which rebuilds
/// [`Transform::local_matrix`] from scale/rotation/translation. That is why a scene file
/// never has to store — or be trusted about — the matrix. Serializing a `Transform` skips
/// the matrix as well, so both types have the same field set on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "reflect", derive(Reflect))]
pub struct TransformData {
    /// Copied verbatim into [`Transform::position`] (metres). The conversion neither
    /// validates nor rescales it.
    pub position: Vec3,
    /// Copied verbatim into [`Transform::rotation`]. It is *not* renormalised on the way
    /// in, so a non-unit quaternion in the source data reaches the rebuilt matrix and skews
    /// it.
    pub rotation: Quat,
    /// Copied verbatim into [`Transform::scale`]. A zero component is accepted and leaves
    /// the rebuilt matrix singular.
    pub scale: Vec3,
}

/// An entity's local placement, together with a cached local-to-parent matrix.
///
/// # Cache invariant
///
/// [`local_matrix`](Self::local_matrix) is derived from `position`/`rotation`/`scale` and is
/// refreshed only by the methods on this type ([`set_position`](Self::set_position),
/// [`with_scale`](Self::with_scale), [`translate`](Self::translate), …). Those three fields
/// are public, so writing one directly leaves the matrix stale until
/// [`update_local_matrix`](Self::update_local_matrix) runs — and
/// [`world_matrix`](Self::world_matrix) reads the cache, not the fields.
///
/// # Determinism
///
/// `position` and `rotation` are part of the simulation state the rigid-body world mixes
/// into its state hash bit-for-bit (via `f32::to_bits`) to detect replay/rollback desync;
/// `scale` and the cached matrix are not. As everywhere in this engine, that guarantee is
/// same-platform only — cross-platform bit-exactness is out of scope.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "reflect", derive(Reflect))]
#[serde(from = "TransformData")]
pub struct Transform {
    /// Translation in metres, expressed in the parent's space when the entity is parented
    /// and in world space otherwise.
    pub position: Vec3,
    /// Orientation, expected to be a unit quaternion. The `rotate_*` helpers multiply onto
    /// it without renormalising, so a long chain of incremental rotations drifts off unit
    /// length; renormalise yourself if that matters.
    pub rotation: Quat,
    /// Per-axis multipliers (dimensionless), applied before rotation and translation.
    /// Non-uniform and negative values are stored as given.
    ///
    /// The physics crates do not feed `scale` into collision or solver math — a body's
    /// extents come from its [`Collider`](crate::components::Collider) shape — so scaling a
    /// transform does not resize the collider that moves with it.
    pub scale: Vec3,
    /// Cached local-to-parent matrix: `Mat4::from_scale_rotation_translation(scale,
    /// rotation, position)` as of the last refresh.
    ///
    /// Excluded from serialization and from `reflect` because it is derived — a load
    /// rebuilds it from the other three fields (see [`TransformData`]) instead of restoring
    /// stored bytes.
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
    /// Places the transform at `position` (metres) with identity rotation and unit scale.
    ///
    /// The matrix cache is built before returning, so the value handed back is never in the
    /// stale state described on the type.
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

    /// Moves the transform to `position` (metres) and refreshes the matrix cache. Builder
    /// counterpart of [`set_position`](Self::set_position).
    pub fn with_position(mut self, position: Vec3) -> Self {
        self.position = position;
        self.update_local_matrix();
        self
    }

    /// Sets the per-axis scale and refreshes the matrix cache.
    ///
    /// Every builder call rebuilds the matrix from all three fields rather than
    /// post-multiplying onto it, so chaining these in a different order gives the same
    /// result.
    pub fn with_scale(mut self, scale: Vec3) -> Self {
        self.scale = scale;
        self.update_local_matrix();
        self
    }

    /// Replaces the orientation — it does *not* compose with the current one, unlike
    /// [`rotate_y`](Self::rotate_y) — and refreshes the matrix cache.
    pub fn with_rotation(mut self, rotation: Quat) -> Self {
        self.rotation = rotation;
        self.update_local_matrix();
        self
    }

    /// Teleports the transform to `pos` (metres, same frame as
    /// [`position`](Self::position)) and refreshes the matrix cache. Prefer this over
    /// assigning the field, which leaves the cache stale.
    pub fn set_position(&mut self, pos: Vec3) {
        self.position = pos;
        self.update_local_matrix();
    }

    /// Overwrites the orientation with `rot` and refreshes the matrix cache. `rot` is stored
    /// exactly as given — no normalisation happens here.
    pub fn set_rotation(&mut self, rot: Quat) {
        self.rotation = rot;
        self.update_local_matrix();
    }

    /// Overwrites the per-axis scale and refreshes the matrix cache. A zero component is
    /// accepted and makes the cached matrix singular.
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

    /// Rebuilds the cached matrix from the current fields, composing scale, then rotation,
    /// then translation.
    ///
    /// Idempotent, and the only way to publish direct field writes into
    /// [`local_matrix`](Self::local_matrix) — call it once after mutating the fields in bulk.
    pub fn update_local_matrix(&mut self) {
        self.local_matrix =
            Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.position);
    }

    /// Composes this transform under an optional parent and returns the resulting matrix.
    ///
    /// Exactly **one** level is composed: the parent contributes its own local matrix, so a
    /// grandparent's transform is not folded in. With `None` the local matrix is returned
    /// unchanged. Both inputs are the *cached* matrices, so a stale cache on either
    /// transform propagates straight into the result.
    ///
    /// Deeper hierarchies are the job of the engine's transform-propagation pass, which
    /// writes [`GlobalTransform`] instead of using this method.
    pub fn world_matrix(&self, parent: Option<&Transform>) -> Mat4 {
        match parent {
            Some(p) => p.world_matrix(None) * self.local_matrix,
            None => self.local_matrix,
        }
    }
}

gizmo_core::impl_component!(Transform);

/// The composed world matrix of an entity, produced by the engine's transform-propagation
/// pass.
///
/// A root entity's world matrix is simply its own [`Transform::local_matrix`]; a child's is
/// `parent_world * child_local`. Because it is a published result rather than a live view, it
/// lags any edit to the local [`Transform`] until that pass runs again.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GlobalTransform {
    /// Local-to-world matrix as of the last propagation, in metres.
    ///
    /// Skipped by serde: a round-tripped `GlobalTransform` comes back as `Mat4::IDENTITY`
    /// (glam's `Default` for `Mat4`) rather than the saved pose, and stays there until
    /// propagation recomputes it from the entity's [`Transform`].
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
    /// Returns the stored matrix by value.
    ///
    /// Despite the name it computes nothing: if the local [`Transform`] has changed since
    /// the last propagation, this hands back the stale pose.
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
