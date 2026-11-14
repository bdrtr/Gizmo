//! [`LightmapReceiver`] — marks a static mesh as given a lightmap when the scene is baked.

/// This entity's mesh is given a lightmap of `width × height` texels, laid out by its second UV
/// set (`Vertex::lightmap_uv`), when the scene is baked (`gizmo::lightmap::bake_lightmaps`). The
/// bake writes the picture and sets the material's
/// [`MaterialMaps::lightmap`](crate::components::MaterialMaps::lightmap) to it, so the component is
/// what a scene keeps and the map is what it draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LightmapReceiver {
    /// Texels across.
    pub width: u32,
    /// Texels down.
    pub height: u32,
}

impl LightmapReceiver {
    /// A `width × height` lightmap.
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

impl Default for LightmapReceiver {
    /// 128 × 128.
    fn default() -> Self {
        Self::new(128, 128)
    }
}

gizmo_core::impl_component!(LightmapReceiver);
