//! The wgpu descriptors the renderer writes over and over, written once.
//!
//! A compute pipeline the renderer builds is always one entry point of one module over one layout,
//! with default compilation options and no pipeline cache, and a buffer binding is nearly always the
//! whole buffer: spelled out, each took eight lines that differed in one or two words, and the GPU
//! fluid's and rigid bodies' builders were hundreds of lines of them.

/// A compute pipeline running `entry` of `module` over `layout`.
pub(crate) fn compute_pipeline(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    entry: &str,
) -> wgpu::ComputePipeline {
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        module,
        entry_point: Some(entry),
        compilation_options: Default::default(),
        cache: None,
    })
}

/// A whole buffer of type `ty` at `binding`, seen by `visibility`: no dynamic offset, no minimum
/// size.
pub(crate) fn buffer_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
    ty: wgpu::BufferBindingType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    }
}

/// A single-sampled 2D float texture at `binding`, seen by `visibility`.
pub(crate) fn texture_2d_entry(binding: u32, visibility: wgpu::ShaderStages, filterable: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// A single-sampled texture of `dimension` and `sample_type` at `binding`, seen by the fragment stage:
/// what nearly every texture a render pipeline binds is.
pub(crate) fn fragment_texture(
    binding: u32,
    dimension: wgpu::TextureViewDimension,
    sample_type: wgpu::TextureSampleType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture { sample_type, view_dimension: dimension, multisampled: false },
        count: None,
    }
}

/// A sampler of `kind` at `binding`, seen by the fragment stage.
pub(crate) fn fragment_sampler(binding: u32, kind: wgpu::SamplerBindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(kind), count: None }
}

/// A bind group's entries for `buffers`, each bound whole at its index: the first at binding 0.
pub(crate) fn whole_buffer_entries<'a>(buffers: &[&'a wgpu::Buffer]) -> Vec<wgpu::BindGroupEntry<'a>> {
    buffers
        .iter()
        .zip(0u32..)
        .map(|(buffer, binding)| wgpu::BindGroupEntry { binding, resource: buffer.as_entire_binding() })
        .collect()
}
