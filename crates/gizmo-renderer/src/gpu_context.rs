//! **What the game side may do with the GPU while a frame is drawn: make new things.**
//!
//! The [`Renderer`](crate::Renderer) is not a resource of the game world. It belongs to whoever
//! draws — the app's render hook, and later a render thread drawing frame N while the game updates
//! frame N+1 — and a game system that reached it could write into a buffer the draw is reading.
//! What the game side does need is to *create*: a mesh's vertex buffer when a file finishes
//! loading, a streamed texture and its bind group. [`GpuContext`] is that and only that. It hands
//! out the [`Device`](wgpu::Device) (every call on it makes something new) and the texture layout
//! materials bind against, and uploads through the queue only into what it has just made — there
//! is no queue to write an existing buffer with.

use crate::asset::AssetManager;
use std::sync::{Arc, RwLock};

/// The game side's view of the GPU: creation, and uploads into what it creates. A resource of the
/// game world, inserted by the app; cheap to clone (handles).
///
/// There is no queue to reach — writing an existing buffer is the draw's:
///
/// ```compile_fail
/// fn write(gpu: &gizmo_renderer::GpuContext, buffer: &wgpu::Buffer) {
///     gpu.queue.write_buffer(buffer, 0, &[0u8; 4]);
/// }
/// ```
#[derive(Clone)]
pub struct GpuContext {
    device: wgpu::Device,
    queue: wgpu::Queue,
    texture_bind_group_layout: wgpu::BindGroupLayout,
    asset_manager: Arc<RwLock<AssetManager>>,
}

impl std::fmt::Debug for GpuContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuContext").finish_non_exhaustive()
    }
}

impl GpuContext {
    /// The device, to create buffers, textures and bind groups.
    #[must_use]
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// The layout a material's texture bind group is made against.
    #[must_use]
    pub fn texture_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.texture_bind_group_layout
    }

    /// The renderer's texture and mesh cache — the same one [`Renderer::asset_manager`](crate::Renderer::asset_manager)
    /// holds, behind its lock.
    #[must_use]
    pub fn asset_manager(&self) -> &RwLock<AssetManager> {
        &self.asset_manager
    }

    /// Makes a material texture from decoded RGBA and its bind group, cached under `cache_key` in
    /// the renderer's asset cache — what texture streaming installs when a decode finishes.
    ///
    /// # Errors
    ///
    /// What [`AssetManager::install_decoded_material_texture`] returns.
    pub fn install_material_texture(
        &self,
        cache_key: &str,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Result<Arc<wgpu::BindGroup>, crate::asset::AssetError> {
        let mut assets = self.asset_manager.write().unwrap_or_else(|poisoned| poisoned.into_inner());
        assets.install_decoded_material_texture(
            &self.device,
            &self.queue,
            &self.texture_bind_group_layout,
            cache_key,
            rgba,
            width,
            height,
        )
    }

    /// One pass of the lazy asset loader ([`run_asset_loading_system`](crate::asset_loading::run_asset_loading_system))
    /// over `world`, with `asset_manager` — it creates the meshes and textures entities ask for.
    pub fn run_asset_loading(&self, world: &mut gizmo_core::World, asset_manager: &mut AssetManager) {
        crate::asset_loading::run_asset_loading_system(
            world,
            &self.device,
            &self.queue,
            &self.texture_bind_group_layout,
            asset_manager,
        );
    }
}

impl crate::Renderer {
    /// The game side's view of this renderer's GPU — see [`GpuContext`].
    #[must_use]
    pub fn gpu_context(&self) -> GpuContext {
        GpuContext {
            device: self.device.clone(),
            queue: self.queue.clone(),
            texture_bind_group_layout: self.scene.texture_bind_group_layout.clone(),
            asset_manager: Arc::clone(&self.asset_manager),
        }
    }
}
