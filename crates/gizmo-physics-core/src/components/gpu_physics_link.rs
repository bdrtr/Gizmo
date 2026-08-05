//! Link from an ECS entity to its slot in the renderer's GPU physics buffers.

/// Marks an entity whose rigid-body state lives in the renderer's GPU physics buffers, and
/// records which slot it occupies there.
///
/// Nothing in this crate attaches it. It appears only while an engine GPU-physics submit path
/// is actually running, and that path is opt-in — on a plain CPU-physics configuration the
/// component is never attached at all. Code that reads the link must therefore tolerate its
/// absence permanently, not merely for the frame an entity is spawned on. Plane colliders are
/// excluded even when that path does run: they are routed to a separate static-collider table
/// and never receive this component, while every other shape is linked and uploaded as a box.
///
/// It derives `Serialize`/`Deserialize` for convenience, but a persisted [`id`](Self::id)
/// describes one particular upload and means nothing in a later run.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct GpuPhysicsLink {
    /// Index of this body's slot in the GPU box buffer — the same index its state is read
    /// back from into [`Transform`](crate::components::Transform).
    ///
    /// The submit path derives the next id from how many bodies are already linked, so ids
    /// stay dense as long as bodies are only added. Nothing reserves a slot for a body that
    /// goes away, which is what makes this a buffer index for the current upload rather than
    /// a lasting identity. It is neither the ECS entity id nor a
    /// [`BodyHandle`](crate::BodyHandle).
    pub id: u32,
}
#[cfg(feature = "ecs")]
gizmo_core::impl_component!(GpuPhysicsLink);
