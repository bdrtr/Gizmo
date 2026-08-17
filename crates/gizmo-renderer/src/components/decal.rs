use std::sync::Arc;

/// A texture projected onto whatever geometry lies inside the entity's volume — a bullet hole, a
/// puddle, a painted marking.
///
/// The entity's `Transform` is the projector box: the decal is applied where that box intersects
/// the depth buffer, so it wraps over whatever it lands on rather than being a piece of geometry
/// that has to match the surface.
#[derive(Clone)]
pub struct Decal {
    /// The decal's bind group: its uniform buffer, texture and sampler.
    pub bind_group: Arc<wgpu::BindGroup>,
    /// A tint multiplied into the projected texture; `a` scales its opacity.
    pub color: gizmo_math::Vec4,
}

impl Decal {
    /// A decal drawn with the given bind group and tint.
    pub fn new(bind_group: Arc<wgpu::BindGroup>, color: gizmo_math::Vec4) -> Self {
        Self { bind_group, color }
    }
}
