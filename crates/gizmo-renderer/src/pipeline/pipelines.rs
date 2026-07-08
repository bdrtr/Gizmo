//! Render pipeline'larının (core + shadow) oluşturulması.

use super::layouts::LayoutRefs;
use super::shaders::load_shader;
#[cfg(not(target_arch = "wasm32"))]
use super::shaders::load_shader_composed;
#[cfg(target_arch = "wasm32")]
use super::shaders::load_shader_composed_web;
use crate::gpu_types::Vertex;

pub(super) struct CorePipelines {
    pub(super) render: wgpu::RenderPipeline,
    pub(super) render_double_sided: wgpu::RenderPipeline,
    pub(super) wireframe: wgpu::RenderPipeline,
    pub(super) unlit: wgpu::RenderPipeline,
    pub(super) sky: wgpu::RenderPipeline,
    pub(super) water: wgpu::RenderPipeline,
    pub(super) transparent: wgpu::RenderPipeline,
    pub(super) grid: wgpu::RenderPipeline,
}

pub(super) fn build_core_pipelines(device: &wgpu::Device, layouts: &LayoutRefs) -> CorePipelines {
    // WASM: max 4 bind groups (Chrome WebGPU), shadow kaldırıldı
    // Native: 5 bind groups (shadow dahil)
    #[cfg(not(target_arch = "wasm32"))]
    let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Render Pipeline Layout"),
        bind_group_layouts: &[
            Some(layouts.global),   // 0
            Some(layouts.texture),  // 1
            Some(layouts.shadow),   // 2
            Some(layouts.skeleton), // 3
            Some(layouts.instance), // 4
        ],
        immediate_size: 0,
    });
    #[cfg(target_arch = "wasm32")]
    let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Render Pipeline Layout"),
        bind_group_layouts: &[
            Some(layouts.global),   // 0
            Some(layouts.texture),  // 1
            Some(layouts.skeleton), // 2
            Some(layouts.instance), // 3
        ],
        immediate_size: 0,
    });

    #[cfg(not(target_arch = "wasm32"))]
    let shader = load_shader_composed(
        device,
        "demo/assets/shaders/shader.wgsl",
        include_str!("../shaders/shader.wgsl"),
        "Shader",
    );
    #[cfg(target_arch = "wasm32")]
    let shader = load_shader_composed_web(device, include_str!("../shaders/shader.wgsl"), "Shader");

    #[cfg(not(target_arch = "wasm32"))]
    let unlit_shader = load_shader_composed(
        device,
        "demo/assets/shaders/unlit.wgsl",
        include_str!("../shaders/unlit.wgsl"),
        "Unlit Shader",
    );
    #[cfg(target_arch = "wasm32")]
    let unlit_shader = load_shader_composed_web(device, include_str!("../shaders/unlit.wgsl"), "Unlit Shader");

    #[cfg(not(target_arch = "wasm32"))]
    let water_shader = load_shader_composed(
        device,
        "demo/assets/shaders/water.wgsl",
        include_str!("../shaders/water.wgsl"),
        "Water Shader",
    );
    #[cfg(target_arch = "wasm32")]
    let water_shader = load_shader_composed_web(device, include_str!("../shaders/water.wgsl"), "Water Shader");

    #[cfg(not(target_arch = "wasm32"))]
    let sky_shader = load_shader_composed(
        device,
        "demo/assets/shaders/sky.wgsl",
        include_str!("../shaders/sky.wgsl"),
        "Sky Shader",
    );
    #[cfg(target_arch = "wasm32")]
    let sky_shader = load_shader_composed_web(device, include_str!("../shaders/sky.wgsl"), "Sky Shader");

    #[cfg(not(target_arch = "wasm32"))]
    let grid_shader = load_shader_composed(
        device,
        "demo/assets/shaders/grid.wgsl",
        include_str!("../shaders/grid.wgsl"),
        "Grid Shader",
    );
    #[cfg(target_arch = "wasm32")]
    let grid_shader = load_shader_composed_web(device, include_str!("../shaders/grid.wgsl"), "Grid Shader");

    let create_main = |sm: &wgpu::ShaderModule,
                       label: &str,
                       depth_write: bool,
                       cull: Option<wgpu::Face>,
                       blend: Option<wgpu::BlendState>,
                       polygon_mode: wgpu::PolygonMode| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: sm,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Vertex::desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: sm,
                entry_point: Some("fs_main"),
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
                polygon_mode,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(depth_write),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview_mask: None,
            cache: None,
        })
    };

    CorePipelines {
        render: create_main(
            &shader,
            "Render Pipeline",
            true,
            Some(wgpu::Face::Back),
            Some(wgpu::BlendState::ALPHA_BLENDING),
            wgpu::PolygonMode::Fill,
        ),
        render_double_sided: create_main(
            &shader,
            "Render TwoSided Pipeline",
            true,
            None,
            Some(wgpu::BlendState::ALPHA_BLENDING),
            wgpu::PolygonMode::Fill,
        ),
        wireframe: create_main(
            &shader,
            "Wireframe Pipeline",
            true,
            None,
            Some(wgpu::BlendState::ALPHA_BLENDING),
            // WebGPU/WebGL2 PolygonMode::Line desteklemiyor
            if cfg!(target_arch = "wasm32") {
                wgpu::PolygonMode::Fill
            } else {
                wgpu::PolygonMode::Line
            },
        ),
        transparent: create_main(
            &shader,
            "Transparent Pipeline",
            false,
            None,
            Some(wgpu::BlendState::ALPHA_BLENDING),
            wgpu::PolygonMode::Fill,
        ),
        unlit: create_main(
            &unlit_shader,
            "Unlit Pipeline",
            true,
            None,
            Some(wgpu::BlendState::ALPHA_BLENDING),
            wgpu::PolygonMode::Fill,
        ),
        sky: create_main(
            &sky_shader,
            "Sky Pipeline",
            false,
            None,
            Some(wgpu::BlendState::ALPHA_BLENDING),
            wgpu::PolygonMode::Fill,
        ),
        water: create_main(
            &water_shader,
            "Water Pipeline",
            true,
            Some(wgpu::Face::Back),
            Some(wgpu::BlendState::ALPHA_BLENDING),
            wgpu::PolygonMode::Fill,
        ),
        grid: create_main(
            &grid_shader,
            "Grid Pipeline",
            false,
            None,
            Some(wgpu::BlendState::ALPHA_BLENDING),
            wgpu::PolygonMode::Fill,
        ),
    }
}

pub(super) fn build_shadow_pipeline(device: &wgpu::Device, layouts: &LayoutRefs) -> wgpu::RenderPipeline {
    let shadow_shader = load_shader(
        device,
        "demo/assets/shaders/shadow.wgsl",
        include_str!("../shaders/shadow.wgsl"),
        "Shadow Shader",
    );
    let shadow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Shadow Pipeline Layout"),
        bind_group_layouts: &[Some(layouts.shadow_pass), Some(layouts.skeleton), Some(layouts.instance)],
        immediate_size: 0,
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Shadow Pipeline"),
        layout: Some(&shadow_layout),
        vertex: wgpu::VertexState {
            module: &shadow_shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Vertex::desc()],
        },
        fragment: None,
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            polygon_mode: wgpu::PolygonMode::Fill,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            // Shadow-map depth bias. Kept low: an aggressive slope_scale shoves the
            // caster deep into the shadow map and detaches its shadow from the base
            // (peter-panning — a visible grey gap between a cube and its shadow). The
            // shader-side normal offset + compare bias handle self-shadow acne, so
            // this only needs a light touch. (Was 2 / 2.0 → visible gap.)
            bias: wgpu::DepthBiasState {
                constant: 1,
                slope_scale: 1.0,
                clamp: 0.0,
            },
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
            cache: None,
    })
}
