//! Texture creation methods on [`Renderer`].
//!
//! Cached convenience textures (checkerboard/white), disk texture loading, and the
//! hand-supplied-RGBA path. Split out of `renderer.rs` for navigability.
//!
//! The mipmap blitter used to live here as `Renderer::generate_mipmaps`, where the asset
//! pipeline could not reach it — which is why two of the engine's three texture upload paths
//! shipped without mips. It is [`crate::texture_quality::MipmapBlitter`] now, together with the
//! sampler policy that decides when a mip chain is actually sampled.

use std::sync::Arc;

use super::Renderer;

impl Renderer {
    /// Creates a checkerboard texture — ideal for test materials.
    /// Cached: the same texture is not created twice.
    pub fn create_checkerboard_texture(&self) -> Arc<wgpu::BindGroup> {
        self.asset_manager
            .write()
            .unwrap()
            .create_checkerboard_texture(
                &self.device,
                &self.queue,
                &self.scene.texture_bind_group_layout,
            )
    }

    /// A plain white texture, for the default material.
    /// Cached: the same texture is not created twice.
    pub fn create_white_texture(&self) -> Arc<wgpu::BindGroup> {
        self.asset_manager.write().unwrap().create_white_texture(
            &self.device,
            &self.queue,
            &self.scene.texture_bind_group_layout,
        )
    }

    /// The white 1×1 material bind group — what an untextured material draws with.
    ///
    /// Cached in the asset manager under a fixed key, so the second caller gets the first
    /// caller's upload. `material_sync` uses it for a description with no texture and for one
    /// whose texture will not load: a warning and a white surface, rather than an entity that
    /// silently fails to appear.
    pub fn white_material_bind_group(&self) -> std::sync::Arc<wgpu::BindGroup> {
        self.asset_manager.write().unwrap().create_white_texture(
            &self.device,
            &self.queue,
            &self.scene.texture_bind_group_layout,
        )
    }

    /// Loads a texture from disk (including the BC7 pipeline).
    /// Cached: the same path is not loaded twice.
    pub fn load_texture(
        &self,
        path: &str,
    ) -> Result<Arc<wgpu::BindGroup>, crate::asset::AssetError> {
        self.asset_manager.write().unwrap().load_material_texture(
            &self.device,
            &self.queue,
            &self.scene.texture_bind_group_layout,
            path,
        )
    }

    /// Uploads RGBA8 pixels and returns a material bind group over them, built against
    /// [`SceneState::texture_bind_group_layout`](crate::pipeline::SceneState::texture_bind_group_layout).
    pub fn create_texture(&self, rgba_bytes: &[u8], width: u32, height: u32) -> wgpu::BindGroup {
        let mip_level_count = crate::texture_quality::mip_level_count(width, height);
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Game Texture"),
            size,
            mip_level_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: crate::texture_quality::MIPPED_TEXTURE_USAGE,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba_bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: Some(height),
            },
            size,
        );

        crate::texture_quality::MipmapBlitter::new(
            &self.device,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        )
        .generate(&self.device, &self.queue, &texture, mip_level_count);

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        // Bu yol mip'i zaten üretiyordu ama anizotropi istemiyordu (`..Default::default()`
        // → `anisotropy_clamp: 1`), yani eğik yüzeylerde trilinear'da kalıyordu.
        let sampler = crate::texture_quality::material_sampler(
            &self.device,
            "texture_sampler",
            wgpu::AddressMode::Repeat,
            wgpu::AddressMode::Repeat,
            wgpu::FilterMode::Linear,
            wgpu::FilterMode::Linear,
            mip_level_count > 1,
        );
        // Fill the auxiliary textured-PBR slots (normal/MR/emissive/AO/params) with
        // the shared neutral defaults so this bind group matches the 7-entry layout.
        self.asset_manager
            .write()
            .unwrap()
            .ensure_material_defaults(&self.device, &self.queue);
        let am = self.asset_manager.read().unwrap();
        let d = am
            .material_defaults()
            .expect("material defaults ensured above");
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &self.scene.texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&d.flat_normal_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&d.white_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&d.white_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&d.white_view),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: d.params_buffer.as_entire_binding(),
                },
            ],
            label: Some("texture_bind_group"),
        })
    }

}
