//! Bind group layout'ları (global, shadow, texture, skeleton, instance).

pub(super) struct Layouts {
    pub(super) global: wgpu::BindGroupLayout,
    pub(super) shadow: wgpu::BindGroupLayout,
    pub(super) shadow_pass: wgpu::BindGroupLayout,
    pub(super) texture: wgpu::BindGroupLayout,
    pub(super) skeleton: wgpu::BindGroupLayout,
    pub(super) instance: wgpu::BindGroupLayout,
}

pub(super) fn build_layouts(device: &wgpu::Device) -> Layouts {
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

pub(super) struct LayoutRefs<'a> {
    pub(super) global: &'a wgpu::BindGroupLayout,
    // WASM pipeline layout'u shadow grubunu kullanmaz (4-grup şeması).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(super) shadow: &'a wgpu::BindGroupLayout,
    pub(super) shadow_pass: &'a wgpu::BindGroupLayout,
    pub(super) texture: &'a wgpu::BindGroupLayout,
    pub(super) skeleton: &'a wgpu::BindGroupLayout,
    pub(super) instance: &'a wgpu::BindGroupLayout,
}
