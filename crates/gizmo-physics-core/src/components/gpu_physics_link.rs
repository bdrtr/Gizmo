#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct GpuPhysicsLink {
    pub id: u32,
}
gizmo_core::impl_component!(GpuPhysicsLink);
