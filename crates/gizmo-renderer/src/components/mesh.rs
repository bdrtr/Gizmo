use gizmo_math::Vec3;
use std::sync::Arc;

#[derive(Clone)]
pub struct Mesh {
    pub vbuf: Arc<wgpu::Buffer>,
    pub vertex_count: u32,
    pub center_offset: Vec3,
    pub source: String,
    pub bounds: gizmo_math::Aabb,
}

impl Mesh {
    pub fn new(
        vbuf: Arc<wgpu::Buffer>,
        vertex_count: u32,
        center_offset: Vec3,
        source: String,
        bounds: gizmo_math::Aabb,
    ) -> Self {
        Self {
            vbuf,
            vertex_count,
            center_offset,
            source,
            bounds,
        }
    }
}

#[derive(Clone)]
pub struct MeshRenderer;

impl MeshRenderer {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MeshRenderer {
    fn default() -> Self {
        Self::new()
    }
}
