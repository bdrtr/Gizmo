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

/// The sun's cascades, `SHADOW_MAP_RES`² a layer. **On the forward-only path a texel a layer**: that
/// path — a browser's, a phone's — draws no shadow maps and its lit pipelines have no shadow group, so
/// the array is only what the shadow group is built over. It was the full 3072² × 4 there too until
/// 2026-11-12, 144 MiB of an Android game's memory that nothing wrote or read.
pub(super) fn build_shadow_resources(
    device: &wgpu::Device,
) -> (wgpu::Texture, wgpu::TextureView, [wgpu::TextureView; 4], wgpu::Sampler) {
    let res = if cfg!(gizmo_forward_only) { 1 } else { SHADOW_MAP_RES };
    let shadow_depth_texture = device.create_texture(&wgpu::TextureDescriptor {
        size: wgpu::Extent3d {
            width: res,
            height: res,
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

    (shadow_depth_texture, shadow_texture_view, shadow_cascade_layer_views, shadow_sampler)
}
