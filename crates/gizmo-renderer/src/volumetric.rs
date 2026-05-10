
use crate::deferred::DeferredState;
use crate::pipeline::{load_shader, SceneState};

pub struct VolumetricState {
    pub volumetric_texture: wgpu::Texture,
    pub volumetric_view: wgpu::TextureView,

    pub volumetric_pipeline: wgpu::RenderPipeline,
    volumetric_bgl: wgpu::BindGroupLayout,
    pub volumetric_bind_group: wgpu::BindGroup,

    pub apply_pipeline: wgpu::RenderPipeline,
    apply_bgl: wgpu::BindGroupLayout,
    pub apply_bind_group: wgpu::BindGroup,

    linear_sampler: wgpu::Sampler,

    pub width: u32,
    pub height: u32,
}

impl VolumetricState {
    pub fn new(
        device: &wgpu::Device,
        scene: &SceneState,
        deferred: &DeferredState,
        width: u32,
        height: u32,
    ) -> Self {
        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let half_w = (width / 2).max(1);
        let half_h = (height / 2).max(1);
        let (volumetric_texture, volumetric_view) = Self::mk_tex(device, half_w, half_h);

        let volumetric_bgl = Self::mk_volumetric_bgl(device);
        let apply_bgl = Self::mk_apply_bgl(device);

        let volumetric_bind_group = Self::mk_volumetric_bg(
            device,
            &volumetric_bgl,
            &deferred.world_position_view,
            &linear_sampler,
        );

        let apply_bind_group =
            Self::mk_apply_bg(device, &apply_bgl, &volumetric_view, &linear_sampler);

        let volumetric_pipeline = Self::mk_volumetric_pipeline(device, scene, &volumetric_bgl);
        let apply_pipeline = Self::mk_apply_pipeline(device, &apply_bgl);

        Self {
            volumetric_texture,
            volumetric_view,
            volumetric_pipeline,
            volumetric_bgl,
            volumetric_bind_group,
            apply_pipeline,
            apply_bgl,
            apply_bind_group,
            linear_sampler,
            width,
            height,
        }
    }

    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        deferred: &DeferredState,
        width: u32,
        height: u32,
    ) {
        let half_w = (width / 2).max(1);
        let half_h = (height / 2).max(1);
        let (t, v) = Self::mk_tex(device, half_w, half_h);
        self.volumetric_texture = t;
        self.volumetric_view = v;

        self.volumetric_bind_group = Self::mk_volumetric_bg(
            device,
            &self.volumetric_bgl,
            &deferred.world_position_view,
            &self.linear_sampler,
        );

        self.apply_bind_group = Self::mk_apply_bg(
            device,
            &self.apply_bgl,
            &self.volumetric_view,
            &self.linear_sampler,
        );

        self.width = width;
        self.height = height;
    }

    fn mk_tex(device: &wgpu::Device, w: u32, h: u32) -> (wgpu::Texture, wgpu::TextureView) {
        let t = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("volumetric_texture"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let v = t.create_view(&wgpu::TextureViewDescriptor::default());
        (t, v)
    }

    fn mk_volumetric_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("volumetric_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    // t_world_position
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    // sampler
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        })
    }

    fn mk_apply_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("volumetric_apply_bgl"),
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
        })
    }

    fn mk_volumetric_bg(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        pos_view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("volumetric_bg"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(pos_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        })
    }

    fn mk_apply_bg(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        vol_view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("volumetric_apply_bg"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(vol_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        })
    }

    fn mk_volumetric_pipeline(
        device: &wgpu::Device,
        scene: &SceneState,
        bgl: &wgpu::BindGroupLayout,
    ) -> wgpu::RenderPipeline {
        let shader = load_shader(
            device,
            "demo/assets/shaders/volumetric.wgsl",
            include_str!("shaders/volumetric.wgsl"),
            "Volumetric Shader",
        );
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("volumetric_layout"),
            bind_group_layouts: &[
                &scene.global_bind_group_layout, // Group 0
                &scene.shadow_bind_group_layout, // Group 1 (Shadow Maps)
                bgl,                             // Group 2
            ],
            push_constant_ranges: &[],
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("volumetric_pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba16Float,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        })
    }

    fn mk_apply_pipeline(
        device: &wgpu::Device,
        bgl: &wgpu::BindGroupLayout,
    ) -> wgpu::RenderPipeline {
        let shader = load_shader(
            device,
            "demo/assets/shaders/volumetric_apply.wgsl",
            include_str!("shaders/volumetric_apply.wgsl"),
            "Volumetric Apply Shader",
        );
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("volumetric_apply_layout"),
            bind_group_layouts: &[bgl],
            push_constant_ranges: &[],
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("volumetric_apply_pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba16Float,
                    // Additive blending!
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent::REPLACE,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        })
    }
}
