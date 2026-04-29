use std::sync::Arc;

#[derive(Clone)]
pub struct Decal {
    pub bind_group: Arc<wgpu::BindGroup>, // Holds the uniform buffer + texture + sampler
    pub color: gizmo_math::Vec4,
}

impl Decal {
    pub fn new(bind_group: Arc<wgpu::BindGroup>, color: gizmo_math::Vec4) -> Self {
        Self { bind_group, color }
    }
}
