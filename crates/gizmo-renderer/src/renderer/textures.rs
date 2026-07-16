//! Texture creation + mipmap generation methods on [`Renderer`].
//!
//! Cached convenience textures (checkerboard/white), disk texture loading, and
//! the runtime mipmap-blit chain. Split out of `renderer.rs` for navigability —
//! no logic change.

use std::sync::Arc;

use super::Renderer;

impl Renderer {
    /// Dama dokusu (checkerboard) oluşturur — test materyalleri için idealdir.
    /// Cache'lenir: aynı doku tekrar oluşturulmaz.
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

    /// Düz beyaz doku — varsayılan materyal için.
    /// Cache'lenir: aynı doku tekrar oluşturulmaz.
    pub fn create_white_texture(&self) -> Arc<wgpu::BindGroup> {
        self.asset_manager.write().unwrap().create_white_texture(
            &self.device,
            &self.queue,
            &self.scene.texture_bind_group_layout,
        )
    }

    /// Diskten doku yükler (BC7 pipeline dahil).
    /// Cache'lenir: aynı dosya yolu tekrar yüklenmez.
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

    pub fn create_texture(&self, rgba_bytes: &[u8], width: u32, height: u32) -> wgpu::BindGroup {
        let mip_level_count = width.max(height).ilog2() + 1;
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
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
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

        Self::generate_mipmaps(
            &self.device,
            &self.queue,
            &texture,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            mip_level_count,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
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

    #[tracing::instrument(skip_all, level = "debug")]
    pub(super) fn generate_mipmaps(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        format: wgpu::TextureFormat,
        mip_level_count: u32,
    ) {
        if mip_level_count <= 1 {
            return;
        }

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Mipmap Blit Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/mipmap.wgsl").into()),
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Mipmap Blit Pipeline"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let bind_group_layout = pipeline.get_bind_group_layout(0);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Mipmap Encoder"),
        });

        let views: Vec<wgpu::TextureView> = (0..mip_level_count)
            .map(|mip| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some(&format!("Mip {}", mip)),
                    format: None,
                    dimension: None,
                    usage: None,
                    aspect: wgpu::TextureAspect::All,
                    base_mip_level: mip,
                    mip_level_count: Some(1),
                    base_array_layer: 0,
                    array_layer_count: None,
                })
            })
            .collect();

        for target_mip in 1..mip_level_count as usize {
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                layout: &bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&views[target_mip - 1]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
                label: None,
            });

            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &views[target_mip],
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        queue.submit(Some(encoder.finish()));
        tracing::debug!(
            mip_levels = mip_level_count,
            ?format,
            "[Renderer] generated mipmap chain"
        );
    }
}
