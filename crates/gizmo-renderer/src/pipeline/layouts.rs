//! The bind group layouts (global, shadow, texture, skeleton, instance).

pub(crate) struct Layouts {
    pub(crate) global: wgpu::BindGroupLayout,
    pub(super) shadow: wgpu::BindGroupLayout,
    pub(super) shadow_pass: wgpu::BindGroupLayout,
    pub(super) texture: wgpu::BindGroupLayout,
    pub(super) skeleton: wgpu::BindGroupLayout,
    pub(super) instance: wgpu::BindGroupLayout,
}

pub(crate) fn build_layouts(device: &wgpu::Device) -> Layouts {
    // The clustered-light buffers live on the GLOBAL group rather than a group of their own, and
    // that is a hard constraint rather than a preference: the web forward pipeline already binds
    // four groups (global, texture, skeleton, instance) and `max_bind_groups` is 4 on every WebGPU
    // baseline this engine targets, so a fifth group would take clustering away from the browser.
    // Group 0 is bound by every pipeline, and a shader that does not light simply does not declare
    // bindings 1 and 2 — wgpu allows a layout to offer more than a shader uses, not less.
    let global = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("global_bind_group_layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
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
            },
            // Cluster table: (offset, count) per cluster. FRAGMENT only — lighting is a
            // fragment-stage job, and a storage buffer visible to the vertex stage spends a slot
            // from a per-stage budget that is 8 on the WebGPU baseline.
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            // Light indices, referenced by the table.
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            // The frame under the glass (`crate::transmission`) and how to read it. On the global
            // group for the reason the clusters are: the web has no group to spare.
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            // The environment, mip by roughness (`crate::env_cube`), which the forward-only path's lit
            // shader reads in place of working it out per pixel. On the global group for the same reason.
            wgpu::BindGroupLayoutEntry {
                binding: 5,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::Cube,
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
            // Reflection probes on the forward-only path (`reflection_probes.wgsl`): the prefiltered
            // cube array, its sampler and the uniform that places them. The desktop reads its probes
            // through the irradiance group and binds placeholders here.
            wgpu::BindGroupLayoutEntry {
                binding: 6,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::CubeArray,
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 7,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 8,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
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
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::Cube,
                    sample_type: wgpu::TextureSampleType::Depth,
                },
                count: None,
            },
            // Light cookies (`crate::cookies`): beside the shadow maps because a spot's cookie is
            // sampled where its shadow is, and a fixed-size array never needs the group rebuilt.
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });

    let shadow_pass = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("shadow_pass_bind_group_layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            // The fragment stage too: a cut-out's (`shadow.wgsl`'s `CUT_OUT`) reads the frame's
            // camera and clock for a game's surface.
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });

    // Per-material texture set (bind group 1). The base-colour texture + sampler
    // at bindings 0/1 are the original layout; bindings 2..=5 add the textured-PBR
    // maps (normal, metallic-roughness, emissive, ambient-occlusion) and binding 6
    // carries the scalar `MaterialParams` (emissive factor, normal scale, AO
    // strength). Forward/decal shaders that only reference bindings 0/1 remain
    // valid — wgpu permits a layout with more entries than a shader uses — but
    // every bind group built against this layout MUST populate all seven entries
    // (see `AssetManager::assemble_material_bind_group`).
    let filterable_tex = |binding: u32| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            multisampled: false,
            view_dimension: wgpu::TextureViewDimension::D2,
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
        },
        count: None,
    };
    let texture = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("texture_bind_group_layout"),
        entries: &[
            filterable_tex(0), // base colour
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            filterable_tex(2), // normal map
            filterable_tex(3), // metallic-roughness (ARM/MR)
            filterable_tex(4), // emissive
            filterable_tex(5), // ambient occlusion
            wgpu::BindGroupLayoutEntry {
                binding: 6,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
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

/// The wireframe pipeline's group 5: one colour.
pub(super) fn wire_color_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("wire_color_bind_group_layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    })
}

pub(super) struct LayoutRefs<'a> {
    pub(super) global: &'a wgpu::BindGroupLayout,
    // WASM pipeline layout'u shadow grubunu kullanmaz (4-grup şeması).
    #[cfg_attr(gizmo_forward_only, allow(dead_code))]
    pub(super) shadow: &'a wgpu::BindGroupLayout,
    pub(super) shadow_pass: &'a wgpu::BindGroupLayout,
    pub(super) texture: &'a wgpu::BindGroupLayout,
    pub(super) skeleton: &'a wgpu::BindGroupLayout,
    pub(super) instance: &'a wgpu::BindGroupLayout,
    /// The irradiance volume's group (with the reflection probes), group 5 of the native forward
    /// pass.
    #[cfg_attr(gizmo_forward_only, allow(dead_code))]
    pub(super) indirect: &'a wgpu::BindGroupLayout,
    /// The wireframe pipeline's group 5: this draw's line colour.
    #[cfg_attr(gizmo_forward_only, allow(dead_code))]
    pub(super) wire_color: &'a wgpu::BindGroupLayout,
}
