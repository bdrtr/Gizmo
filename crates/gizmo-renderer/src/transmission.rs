//! What a pane of glass looks through: the frame as it stood before any glass was drawn.
//!
//! A surface that lets light through ([`Material::transmission`](crate::components::Material::transmission))
//! has to read the picture behind it, and a render target cannot be read while it is being drawn
//! to. So the frame is drawn in two passes when it has glass, and in one when it has not: the opaque
//! world, then [`BehindSurface::capture`](crate::transmission::BehindSurface::capture) copies the result and builds its mip chain, then the glass
//! in a pass of its own, then what blends over everything. A rough pane samples further down the
//! chain, which is its blur.
//!
//! The texture is allocated the first frame that draws glass and dropped on a resize, so a game
//! without glass pays neither the memory (about 21 MB at 1080p) nor the copy.
//!
//! It costs what any screen-space method costs: only what is on screen can show through, one pane
//! does not show through another (the nearest wins), and anything drawn after the copy, blended
//! surfaces and particles, is not seen through glass.

/// The copy of the frame under the glass, and what builds it.
pub struct BehindSurface {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    pipeline: wgpu::RenderPipeline,
    /// Per level from the second down: the target, and the bind group that reads the level above.
    levels: Vec<(wgpu::TextureView, wgpu::BindGroup)>,
}

/// The target and the copy share this format.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

impl BehindSurface {
    /// Builds the texture, every mip view and the downsample pipeline for a frame of this size.
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let size = wgpu::Extent3d { width: width.max(1), height: height.max(1), depth_or_array_layers: 1 };
        let mip_level_count = size.max_mips(wgpu::TextureDimension::D2);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("behind glass"),
            size,
            mip_level_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("behind glass downsample"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("behind glass downsample"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let shader = crate::pipeline::shaders::load_shader(
            device,
            "demo/assets/shaders/behind_downsample.wgsl",
            include_str!("shaders/behind_downsample.wgsl"),
            "behind glass downsample",
        );
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("behind glass downsample"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("behind glass downsample"),
            layout: Some(&pipeline_layout),
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
                    format: FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let level_view = |level: u32| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                base_mip_level: level,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };
        let levels = (1..mip_level_count)
            .map(|level| {
                let source = level_view(level - 1);
                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("behind glass downsample"),
                    layout: &layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&source) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
                    ],
                });
                (level_view(level), bind_group)
            })
            .collect();
        Self { texture, view, pipeline, levels }
    }

    /// The view group 0 offers the glass shader: every level.
    #[must_use]
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Copies the frame as it stands and rebuilds the mip chain.
    pub fn capture(&self, encoder: &mut wgpu::CommandEncoder, frame: &wgpu::Texture) {
        encoder.copy_texture_to_texture(
            frame.as_image_copy(),
            self.texture.as_image_copy(),
            wgpu::Extent3d { width: frame.width(), height: frame.height(), depth_or_array_layers: 1 },
        );
        for (target, source) in &self.levels {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("behind glass downsample"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
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
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, source, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}
