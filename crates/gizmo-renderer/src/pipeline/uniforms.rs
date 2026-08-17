//! Creating the global uniform buffer and the shadow texture/view resources.

use crate::csm::SHADOW_MAP_RES;
use crate::frame_uniforms::SceneFrame;
use crate::gpu_types::SceneUniforms;
use wgpu::util::DeviceExt;

pub(super) fn build_global_uniforms(device: &wgpu::Device) -> wgpu::Buffer {
    // Contents are overwritten every frame before first use; what matters here is that the buffer
    // is the right size and holds a coherent block rather than whatever the driver hands back.
    let initial_uniforms = SceneUniforms::new(&SceneFrame::default());
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
