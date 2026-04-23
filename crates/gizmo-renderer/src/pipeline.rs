use crate::csm::SHADOW_MAP_RES;
use crate::gpu_types::{LightData, SceneUniforms, ShadowVsUniform, Vertex};
use std::sync::Arc;
use wgpu::util::DeviceExt;

/// Sahne render durumu — pipeline'lar, shadow, skeleton ve global bind group'lar
pub struct SceneState {
    pub render_pipeline: wgpu::RenderPipeline,
    pub render_double_sided_pipeline: wgpu::RenderPipeline,
    pub unlit_pipeline: wgpu::RenderPipeline,
    pub sky_pipeline: wgpu::RenderPipeline,
    pub water_pipeline: wgpu::RenderPipeline,
    pub shadow_pipeline: wgpu::RenderPipeline,
    pub transparent_pipeline: wgpu::RenderPipeline,
    pub grid_pipeline: wgpu::RenderPipeline,
    pub global_uniform_buffer: wgpu::Buffer,
    pub global_bind_group_layout: wgpu::BindGroupLayout,
    pub global_bind_group: wgpu::BindGroup,
    pub shadow_bind_group_layout: wgpu::BindGroupLayout,
    pub shadow_bind_group: wgpu::BindGroup,
    /// Depth `texture_2d_array` (all CSM layers) for comparison sampling in lit shaders.
    pub shadow_texture_view: wgpu::TextureView,
    /// One 2D depth view per cascade for shadow map rendering passes.
    pub shadow_cascade_layer_views: [wgpu::TextureView; 4],
    pub shadow_depth_texture: wgpu::Texture,
    pub shadow_pass_bind_group_layout: wgpu::BindGroupLayout,
    /// One uniform buffer + bind group per CSM cascade (avoids per-pass overwrite races on the queue).
    pub shadow_cascade_uniform_buffers: [wgpu::Buffer; 4],
    pub shadow_pass_bind_groups: [wgpu::BindGroup; 4],
    pub texture_bind_group_layout: wgpu::BindGroupLayout,
    pub skeleton_bind_group_layout: wgpu::BindGroupLayout,
    pub dummy_skeleton_bind_group: Arc<wgpu::BindGroup>,
    pub instance_bind_group_layout: wgpu::BindGroupLayout,
    pub instance_buffer: wgpu::Buffer,
    pub instance_bind_group: wgpu::BindGroup,
    /// Current capacity (number of InstanceRaw items) of `instance_buffer`.
    pub instance_capacity: usize,
}

impl SceneState {
    pub fn ensure_instance_capacity(&mut self, device: &wgpu::Device, needed: usize) -> bool {
        if needed <= self.instance_capacity {
            return false;
        }

        let new_capacity = (needed * 2).max(8_192);
        let new_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Instance Buffer (grown)"),
            size: (new_capacity * std::mem::size_of::<crate::InstanceRaw>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let new_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &self.instance_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: new_buffer.as_entire_binding(),
            }],
            label: Some("instance_bind_group (grown)"),
        });

        self.instance_buffer = new_buffer;
        self.instance_bind_group = new_bind_group;
        self.instance_capacity = new_capacity;
        true
    }
}

pub fn load_shader(
    device: &wgpu::Device,
    file_path: &str,
    fallback_src: &str,
    label: &str,
) -> wgpu::ShaderModule {
    let source = std::fs::read_to_string(file_path).unwrap_or_else(|_| fallback_src.to_string());
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
}

// ------------------------------------------------------------------
// BİLDİRİM / OLUŞTURUCU METOTLAR (BUILDERS)
// ------------------------------------------------------------------

fn build_global_uniforms(device: &wgpu::Device) -> wgpu::Buffer {
    let id4 = [
        [1.0f32, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let initial_uniforms = SceneUniforms {
        view_proj: [[0.0; 4]; 4],
        camera_pos: [0.0; 4],
        sun_direction: [0.0, -1.0, 0.0, 0.0],
        sun_color: [1.0, 1.0, 1.0, 0.0],
        lights: [LightData {
            position: [0.0; 4],
            color: [0.0; 4],
            direction: [0.0, -1.0, 0.0, 0.0],
            params: [0.0; 4],
        }; 10],
        light_view_proj: [id4; 4],
        cascade_splits: [1.0, 10.0, 50.0, 500.0],
        camera_forward: [0.0, 0.0, -1.0, 0.0],
        cascade_params: [0.1, 1.0 / SHADOW_MAP_RES as f32, 0.0, 0.0],
        num_lights: 0,
        _pre_align_pad: [0; 3],
        _align_pad: [0; 3],
        _post_align_pad: 0,
        _pad_scene: [0; 3],
        _end_pad: 0,
    };
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Global Uniform Buffer"),
        contents: bytemuck::cast_slice(&[initial_uniforms]),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    })
}

fn build_shadow_resources(
    device: &wgpu::Device,
) -> (
    wgpu::Texture,
    wgpu::TextureView,
    [wgpu::TextureView; 4],
    wgpu::Sampler,
) {
    let shadow_depth_texture = device.create_texture(&wgpu::TextureDescriptor {
        size: wgpu::Extent3d {
            width: SHADOW_MAP_RES,
            height: SHADOW_MAP_RES,
            depth_or_array_layers: 4,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        label: Some("shadow_csm_texture"),
        view_formats: &[],
    });

    let shadow_texture_view = shadow_depth_texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("shadow_csm_array_view"),
        format: None,
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        aspect: wgpu::TextureAspect::All,
        base_mip_level: 0,
        mip_level_count: None,
        base_array_layer: 0,
        array_layer_count: None,
    });

    let shadow_cascade_layer_views = std::array::from_fn(|i| {
        shadow_depth_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some(&format!("shadow_cascade_layer_{i}")),
            format: None,
            dimension: Some(wgpu::TextureViewDimension::D2),
            aspect: wgpu::TextureAspect::DepthOnly,
            base_mip_level: 0,
            mip_level_count: None,
            base_array_layer: i as u32,
            array_layer_count: Some(1),
        })
    });

    let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Nearest,
        compare: Some(wgpu::CompareFunction::LessEqual),
        ..Default::default()
    });

    (
        shadow_depth_texture,
        shadow_texture_view,
        shadow_cascade_layer_views,
        shadow_sampler,
    )
}

struct Layouts {
    global: wgpu::BindGroupLayout,
    shadow: wgpu::BindGroupLayout,
    shadow_pass: wgpu::BindGroupLayout,
    texture: wgpu::BindGroupLayout,
    skeleton: wgpu::BindGroupLayout,
    instance: wgpu::BindGroupLayout,
}

fn build_layouts(device: &wgpu::Device) -> Layouts {
    let global = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("global_bind_group_layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX
                | wgpu::ShaderStages::FRAGMENT
                | wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });

    let shadow = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("shadow_bind_group_layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    sample_type: wgpu::TextureSampleType::Depth,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                count: None,
            },
        ],
    });

    let shadow_pass = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("shadow_pass_bind_group_layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });

    let texture = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("texture_bind_group_layout"),
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

    let skeleton = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("skeleton_bind_group_layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });

    let instance = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("instance_bind_group_layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });

    Layouts {
        global,
        shadow,
        shadow_pass,
        texture,
        skeleton,
        instance,
    }
}

struct CorePipelines {
    render: wgpu::RenderPipeline,
    render_double_sided: wgpu::RenderPipeline,
    unlit: wgpu::RenderPipeline,
    sky: wgpu::RenderPipeline,
    water: wgpu::RenderPipeline,
    transparent: wgpu::RenderPipeline,
    grid: wgpu::RenderPipeline,
}

struct LayoutRefs<'a> {
    global: &'a wgpu::BindGroupLayout,
    shadow: &'a wgpu::BindGroupLayout,
    shadow_pass: &'a wgpu::BindGroupLayout,
    texture: &'a wgpu::BindGroupLayout,
    skeleton: &'a wgpu::BindGroupLayout,
    instance: &'a wgpu::BindGroupLayout,
}

fn build_core_pipelines(device: &wgpu::Device, layouts: &LayoutRefs) -> CorePipelines {
    let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Render Pipeline Layout"),
        bind_group_layouts: &[
            layouts.global,
            layouts.texture,
            layouts.shadow,
            layouts.skeleton,
            layouts.instance,
        ],
        push_constant_ranges: &[],
    });

    let shader = load_shader(
        device,
        "demo/assets/shaders/shader.wgsl",
        include_str!("shaders/shader.wgsl"),
        "Shader",
    );
    let unlit_shader = load_shader(
        device,
        "demo/assets/shaders/unlit.wgsl",
        include_str!("shaders/unlit.wgsl"),
        "Unlit Shader",
    );
    let water_shader = load_shader(
        device,
        "demo/assets/shaders/water.wgsl",
        include_str!("shaders/water.wgsl"),
        "Water Shader",
    );
    let sky_shader = load_shader(
        device,
        "demo/assets/shaders/sky.wgsl",
        include_str!("shaders/sky.wgsl"),
        "Sky Shader",
    );
    let grid_shader = load_shader(
        device,
        "demo/assets/shaders/grid.wgsl",
        include_str!("shaders/grid.wgsl"),
        "Grid Shader",
    );

    let create_main = |sm: &wgpu::ShaderModule,
                       label: &str,
                       depth_write: bool,
                       cull: Option<wgpu::Face>,
                       blend: Option<wgpu::BlendState>| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: sm,
                entry_point: "vs_main",
                compilation_options: Default::default(),
                buffers: &[Vertex::desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: sm,
                entry_point: "fs_main",
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba16Float,
                    blend,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: cull,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: depth_write,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
        })
    };

    CorePipelines {
        render: create_main(
            &shader,
            "Render Pipeline",
            true,
            Some(wgpu::Face::Back),
            Some(wgpu::BlendState::ALPHA_BLENDING),
        ),
        render_double_sided: create_main(
            &shader,
            "Render TwoSided Pipeline",
            true,
            None,
            Some(wgpu::BlendState::ALPHA_BLENDING),
        ),
        transparent: create_main(
            &shader,
            "Transparent Pipeline",
            false,
            None,
            Some(wgpu::BlendState::ALPHA_BLENDING),
        ),
        unlit: create_main(
            &unlit_shader,
            "Unlit Pipeline",
            true,
            Some(wgpu::Face::Back),
            Some(wgpu::BlendState::ALPHA_BLENDING),
        ),
        sky: create_main(
            &sky_shader,
            "Sky Pipeline",
            false,
            Some(wgpu::Face::Back),
            Some(wgpu::BlendState::ALPHA_BLENDING),
        ),
        water: create_main(
            &water_shader,
            "Water Pipeline",
            true,
            Some(wgpu::Face::Back),
            Some(wgpu::BlendState::ALPHA_BLENDING),
        ),
        grid: create_main(
            &grid_shader,
            "Grid Pipeline",
            false,
            None,
            Some(wgpu::BlendState::ALPHA_BLENDING),
        ),
    }
}

fn build_shadow_pipeline(device: &wgpu::Device, layouts: &LayoutRefs) -> wgpu::RenderPipeline {
    let shadow_shader = load_shader(
        device,
        "demo/assets/shaders/shadow.wgsl",
        include_str!("shaders/shadow.wgsl"),
        "Shadow Shader",
    );
    let shadow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Shadow Pipeline Layout"),
        bind_group_layouts: &[layouts.shadow_pass, layouts.skeleton, layouts.instance],
        push_constant_ranges: &[],
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Shadow Pipeline"),
        layout: Some(&shadow_layout),
        vertex: wgpu::VertexState {
            module: &shadow_shader,
            entry_point: "vs_main",
            compilation_options: Default::default(),
            buffers: &[Vertex::desc()],
        },
        fragment: None,
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: Some(wgpu::Face::Front),
            polygon_mode: wgpu::PolygonMode::Fill,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::LessEqual,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState {
                constant: 2,
                slope_scale: 2.0,
                clamp: 0.0,
            },
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
    })
}

// ------------------------------------------------------------------
// ANA YÖNETİCİ METOTLAR
// ------------------------------------------------------------------

pub fn build_scene_pipelines(device: &wgpu::Device) -> SceneState {
    let global_uniform_buffer = build_global_uniforms(device);
    let (shadow_depth_texture, shadow_texture_view, shadow_cascade_layer_views, shadow_sampler) =
        build_shadow_resources(device);
    let layouts = build_layouts(device);

    let global_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &layouts.global,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: global_uniform_buffer.as_entire_binding(),
        }],
        label: Some("global_bind_group"),
    });

    let shadow_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &layouts.shadow,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&shadow_texture_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&shadow_sampler),
            },
        ],
        label: Some("shadow_bind_group"),
    });

    let id4 = [
        [1.0f32, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let shadow_cascade_uniform_buffers = std::array::from_fn(|i| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("Shadow cascade VS uniform {i}")),
            contents: bytemuck::bytes_of(&ShadowVsUniform {
                light_view_proj: id4,
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    });

    let shadow_pass_bind_groups = std::array::from_fn(|i| {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &layouts.shadow_pass,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: shadow_cascade_uniform_buffers[i].as_entire_binding(),
            }],
            label: Some(&format!("shadow_pass_bind_group_{i}")),
        })
    });

    let dummy_skeleton_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Dummy Skeleton Buffer"),
        contents: bytemuck::cast_slice(&[[[0.0f32; 4]; 4]; 64]),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let dummy_skeleton_bind_group =
        Arc::new(device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &layouts.skeleton,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: dummy_skeleton_buffer.as_entire_binding(),
            }],
            label: Some("dummy_skeleton_bind_group"),
        }));

    let initial_capacity: usize = 8_192;
    let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Instance Buffer"),
        size: (initial_capacity * std::mem::size_of::<crate::InstanceRaw>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let instance_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &layouts.instance,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: instance_buffer.as_entire_binding(),
        }],
        label: Some("instance_bind_group"),
    });

    let layout_refs = LayoutRefs {
        global: &layouts.global,
        shadow: &layouts.shadow,
        shadow_pass: &layouts.shadow_pass,
        texture: &layouts.texture,
        skeleton: &layouts.skeleton,
        instance: &layouts.instance,
    };
    let core_pipelines = build_core_pipelines(device, &layout_refs);
    let shadow_pipeline = build_shadow_pipeline(device, &layout_refs);

    SceneState {
        render_pipeline: core_pipelines.render,
        render_double_sided_pipeline: core_pipelines.render_double_sided,
        unlit_pipeline: core_pipelines.unlit,
        sky_pipeline: core_pipelines.sky,
        water_pipeline: core_pipelines.water,
        transparent_pipeline: core_pipelines.transparent,
        grid_pipeline: core_pipelines.grid,
        shadow_pipeline,
        global_uniform_buffer,
        global_bind_group_layout: layouts.global,
        global_bind_group,
        shadow_bind_group_layout: layouts.shadow,
        shadow_bind_group,
        shadow_texture_view,
        shadow_cascade_layer_views,
        shadow_depth_texture,
        shadow_pass_bind_group_layout: layouts.shadow_pass,
        shadow_cascade_uniform_buffers,
        shadow_pass_bind_groups,
        texture_bind_group_layout: layouts.texture,
        skeleton_bind_group_layout: layouts.skeleton,
        dummy_skeleton_bind_group,
        instance_bind_group_layout: layouts.instance,
        instance_buffer,
        instance_bind_group,
        instance_capacity: initial_capacity,
    }
}

pub fn rebuild_pipelines(renderer: &mut crate::Renderer) {
    let device = &renderer.device;
    let post_shader = load_shader(
        device,
        "demo/assets/shaders/post_process.wgsl",
        include_str!("shaders/post_process.wgsl"),
        "Post-Processing Shader",
    );

    // Geçici LayoutRefs tutucusu, render pipeline'ı için mevcut layoutları referans alır
    let layouts = LayoutRefs {
        global: &renderer.scene.global_bind_group_layout,
        shadow: &renderer.scene.shadow_bind_group_layout,
        shadow_pass: &renderer.scene.shadow_pass_bind_group_layout,
        texture: &renderer.scene.texture_bind_group_layout,
        skeleton: &renderer.scene.skeleton_bind_group_layout,
        instance: &renderer.scene.instance_bind_group_layout,
    };

    let core_pipelines = build_core_pipelines(device, &layouts);
    let shadow_pipeline = build_shadow_pipeline(device, &layouts);

    renderer.scene.render_pipeline = core_pipelines.render;
    renderer.scene.render_double_sided_pipeline = core_pipelines.render_double_sided;
    renderer.scene.unlit_pipeline = core_pipelines.unlit;
    renderer.scene.sky_pipeline = core_pipelines.sky;
    renderer.scene.water_pipeline = core_pipelines.water;
    renderer.scene.transparent_pipeline = core_pipelines.transparent;
    renderer.scene.grid_pipeline = core_pipelines.grid;
    renderer.scene.shadow_pipeline = shadow_pipeline;

    crate::post_process::rebuild_post_pipelines(renderer, &post_shader);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dynamic_instance_buffer_resize() {
        pollster::block_on(async {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::all(),
                ..Default::default()
            });

            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::LowPower,
                    ..Default::default()
                })
                .await;

            let adapter = match adapter {
                Some(a) => a,
                None => {
                    println!(
                        "No suitable GPU adapter found for headless test. Skipping wgpu test."
                    );
                    return;
                }
            };

            let (device, _) = adapter
                .request_device(
                    &wgpu::DeviceDescriptor {
                        required_features: wgpu::Features::empty(),
                        required_limits: wgpu::Limits {
                            max_bind_groups: 6,
                            ..wgpu::Limits::default()
                        },
                        label: None,
                    },
                    None,
                )
                .await
                .unwrap();

            // Sahnemizi kur, default capacity 8_192 olmali!
            let mut scene_state = build_scene_pipelines(&device);
            assert_eq!(scene_state.instance_capacity, 8_192);

            // Daha kucuk bir obje listesi istenirse buyumez.
            let grew = scene_state.ensure_instance_capacity(&device, 100);
            assert!(!grew, "Buffer should not grow if capacity is enough");
            assert_eq!(scene_state.instance_capacity, 8_192);

            // Mevcudun disine ciktiginda (Ornegin 10_000) 2 katina grow eder.
            let grew2 = scene_state.ensure_instance_capacity(&device, 10_000);
            assert!(grew2, "Buffer should grow since needed > capacity");
            assert_eq!(scene_state.instance_capacity, 20_000);

            // Gercek byte miktarinin da artmis oldugundan emin olalim.
            let expected_bytes = (20_000 * std::mem::size_of::<crate::InstanceRaw>()) as u64;
            assert_eq!(scene_state.instance_buffer.size(), expected_bytes);

            // Yeniden mevcut sinirlar icinde kaldiginda grow etmez
            let grew3 = scene_state.ensure_instance_capacity(&device, 12_000);
            assert!(!grew3, "Buffer should not grow if capacity is enough");
            assert_eq!(scene_state.instance_capacity, 20_000);
        });
    }
}
