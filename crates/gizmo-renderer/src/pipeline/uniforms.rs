//! Global uniform buffer ve shadow doku/görünüm kaynaklarının oluşturulması.

use crate::csm::SHADOW_MAP_RES;
use crate::gpu_types::{LightData, SceneUniforms};
use wgpu::util::DeviceExt;

pub(super) fn build_global_uniforms(device: &wgpu::Device) -> wgpu::Buffer {
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
        exposure: 1.0,
        _pre_align_pad: [0; 2],
        _align_pad: [0; 3],
        environment_blend_t: 0.0,
        environment_preset: 0,
        point_shadows_enabled: 0,
        environment_preset_2: 0,
        shading_mode: 0,
    };
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Global Uniform Buffer"),
        contents: bytemuck::cast_slice(&[initial_uniforms]),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    })
}

pub(super) fn build_shadow_resources(
    device: &wgpu::Device,
) -> (
    wgpu::Texture,
    wgpu::TextureView,
    [wgpu::TextureView; 4],
    wgpu::Sampler,
    wgpu::Texture,
    wgpu::TextureView,
    [wgpu::TextureView; 6],
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
        usage: None,
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
            usage: None,
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
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        compare: Some(wgpu::CompareFunction::LessEqual),
        ..Default::default()
    });

    let point_shadow_depth_texture = device.create_texture(&wgpu::TextureDescriptor {
        size: wgpu::Extent3d {
            width: 1024,
            height: 1024,
            depth_or_array_layers: 6,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        label: Some("point_shadow_texture"),
        view_formats: &[],
    });

    let point_shadow_cube_view = point_shadow_depth_texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("point_shadow_cube_view"),
        format: None,
        dimension: Some(wgpu::TextureViewDimension::Cube),
        usage: None,
        aspect: wgpu::TextureAspect::DepthOnly,
        base_mip_level: 0,
        mip_level_count: None,
        base_array_layer: 0,
        array_layer_count: None,
    });

    let point_shadow_face_views = std::array::from_fn(|i| {
        point_shadow_depth_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some(&format!("point_shadow_face_{i}")),
            format: None,
            dimension: Some(wgpu::TextureViewDimension::D2),
            usage: None,
            aspect: wgpu::TextureAspect::DepthOnly,
            base_mip_level: 0,
            mip_level_count: None,
            base_array_layer: i as u32,
            array_layer_count: Some(1),
        })
    });

    (
        shadow_depth_texture,
        shadow_texture_view,
        shadow_cascade_layer_views,
        shadow_sampler,
        point_shadow_depth_texture,
        point_shadow_cube_view,
        point_shadow_face_views,
    )
}
