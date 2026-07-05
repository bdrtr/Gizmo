use super::*;

pub(super) fn create_ssfr_sized(
    device: &wgpu::Device,
    pipelines: &FluidPipelines,
    particles_buffer: &wgpu::Buffer,
    output_format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> SsfrSized {
    let extent = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
    let make_tex = |label: &str, format: wgpu::TextureFormat, usage: wgpu::TextureUsages| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };

    let depth_texture = make_tex(
        "SSFR Depth Texture",
        wgpu::TextureFormat::Depth32Float,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
    );
    let depth_texture_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());

    let raw_depth_texture = make_tex(
        "SSFR Raw Depth Color Texture",
        wgpu::TextureFormat::Rgba16Float,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
    );
    let raw_depth_texture_view =
        raw_depth_texture.create_view(&wgpu::TextureViewDescriptor::default());

    let blur_texture = make_tex(
        "SSFR Blur Texture",
        wgpu::TextureFormat::Rgba16Float,
        wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
    );
    let blur_texture_view = blur_texture.create_view(&wgpu::TextureViewDescriptor::default());

    let thickness_texture = make_tex(
        "SSFR Thickness Texture",
        wgpu::TextureFormat::R16Float,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
    );
    let thickness_texture_view =
        thickness_texture.create_view(&wgpu::TextureViewDescriptor::default());

    let blur_temp_texture = make_tex(
        "SSFR Blur Temp Texture",
        wgpu::TextureFormat::Rgba16Float,
        wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
    );
    let blur_temp_texture_view =
        blur_temp_texture.create_view(&wgpu::TextureViewDescriptor::default());

    let opaque_bg_texture = make_tex(
        "Opaque Background Texture",
        output_format,
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    );
    let opaque_bg_texture_view =
        opaque_bg_texture.create_view(&wgpu::TextureViewDescriptor::default());

    // Blur direction/radius params (size-independent, but recreated with the
    // bind groups they feed for simplicity).
    let blur_params_buffer_x = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Blur Params X"),
        contents: bytemuck::cast_slice(&[1u32, 0u32, 16u32, 1.0f32.to_bits()]),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let blur_params_buffer_y = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Blur Params Y"),
        contents: bytemuck::cast_slice(&[0u32, 1u32, 16u32, 1.0f32.to_bits()]),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });

    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    });

    let ssfr_particle_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("SSFR Particle BG"),
        layout: &pipelines.particle_render_bg_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 1,
            resource: particles_buffer.as_entire_binding(),
        }],
    });

    let ssfr_blur_x_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("SSFR Blur X BG"),
        layout: &pipelines.blur_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&raw_depth_texture_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&blur_temp_texture_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: blur_params_buffer_x.as_entire_binding(),
            },
        ],
    });

    let ssfr_blur_y_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("SSFR Blur Y BG"),
        layout: &pipelines.blur_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&blur_temp_texture_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&blur_texture_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: blur_params_buffer_y.as_entire_binding(),
            },
        ],
    });

    let ssfr_composite_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("SSFR Composite BG"),
        layout: &pipelines.composite_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&blur_texture_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&thickness_texture_view),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(&opaque_bg_texture_view),
            },
        ],
    });

    SsfrSized {
        depth_texture_view,
        raw_depth_texture,
        raw_depth_texture_view,
        blur_texture_view,
        thickness_texture_view,
        opaque_bg_texture,
        opaque_bg_texture_view,
        ssfr_particle_bg,
        ssfr_blur_x_bg,
        ssfr_blur_y_bg,
        ssfr_composite_bg,
    }
}
