//! [`Occluder`] — marks a big opaque thing as hiding what is behind it.

use gizmo_math::Aabb;

/// What is completely behind this entity, from the camera, is not drawn: no instance, no draw call,
/// no fragment. Put it on the walls, buildings and hills that hide a lot; each one is rasterised into
/// a small depth buffer every view ([`crate::occlusion`]), so a scene wants tens of them, not
/// thousands. A scene with none pays nothing.
///
/// **The box has to be inside the object.** By default it is the mesh's bounds, which is exactly a
/// box mesh and is *too big* for anything else: a sphere's bounds would hide what is seen past its
/// curve. Give any other shape [`inner_box`](Self::inner_box), a box wholly inside it.
///
/// It counts only while the entity draws solid: a hidden, transparent, pooled or `ShadowCasting::Only`
/// occluder hides nothing. An object it hides still casts its shadow.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Occluder {
    pub(crate) inner: Option<Aabb>,
}

impl Occluder {
    /// The mesh's own bounds: for a box mesh, a wall or a crate.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A box in the mesh's local space, wholly inside its surface.
    #[must_use]
    pub fn inner_box(local: Aabb) -> Self {
        Self { inner: Some(local) }
    }

    /// The box to rasterise, given the mesh's bounds.
    #[must_use]
    pub fn box_for(&self, mesh_bounds: Aabb) -> Aabb {
        self.inner.unwrap_or(mesh_bounds)
    }
}

gizmo_core::impl_component!(Occluder);
