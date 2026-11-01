//! CAS — contrast-adaptive sharpening, the last pass of the post chain.
//!
//! TAA blends each frame into a history and FXAA blurs along edges; both cost detail, and a
//! picture with both on reads soft. CAS gives it back: each pixel is pushed away from its four
//! neighbours by a weight that adapts to the local contrast (the shader, `shaders/cas.wgsl`, has
//! the formula), so flat areas are left exactly as they are and edges that are already near black
//! or white are not driven into ringing.
//!
//! Off by default — it changes every frame's look, and that is the author's call:
//!
//! ```ignore
//! if let Some(cas) = renderer.cas.as_mut() {
//!     cas.enabled = true;
//!     cas.set_sharpness(&renderer.queue, 0.6);
//! }
//! ```
//!
//! Where it sits: composite → FXAA (if on) → **CAS** → output, so it sharpens the final
//! anti-aliased image and is the pass that writes a split-screen view's rectangle
//! ([`Renderer::run_post_processing_in`](crate::Renderer::run_post_processing_in)). The HUD is
//! drawn after it and is never sharpened.

use wgpu::util::DeviceExt;

/// The CAS pass: its pipeline, its input texture (what the pass before it writes), and its one
/// parameter.
#[non_exhaustive]
pub struct CasState {
    /// The full-screen sharpening pipeline, built for the output format.
    pub pipeline: wgpu::RenderPipeline,
    /// Layout of [`bind_group`](Self::bind_group): input texture, sampler, parameters.
    pub bind_group_layout: wgpu::BindGroupLayout,
    /// Binds [`input_texture_view`](Self::input_texture_view). Rebuilt on resize.
    pub bind_group: wgpu::BindGroup,
    /// [`CasParams`], written by [`set_sharpness`](Self::set_sharpness).
    pub params_buffer: wgpu::Buffer,
    /// What the previous pass (composite, or FXAA) renders into while CAS is enabled.
    pub input_texture: wgpu::Texture,
    /// View of [`input_texture`](Self::input_texture).
    pub input_texture_view: wgpu::TextureView,
    /// Bilinear clamp-to-edge; taps land on texel centres, so it returns texels exactly.
    pub sampler: wgpu::Sampler,
    /// Whether the pass runs. `false` (the default) leaves the chain exactly as without CAS.
    pub enabled: bool,
    sharpness: f32,
}

/// The uniform block of `cas.wgsl`.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CasParams {
    /// 0 = the mildest sharpening, 1 = the strongest.
    pub sharpness: f32,
    /// Pads the block to 16 bytes.
    pub _padding: [f32; 3],
}

/// Sharpness a new [`CasState`] starts with — the middle of the range.
pub const DEFAULT_SHARPNESS: f32 = 0.5;

impl CasState {
    /// Builds the pass for `surface_format` at `width` × `height`, disabled.
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        let source = std::fs::read_to_string("demo/assets/shaders/cas.wgsl")
            .unwrap_or_else(|_| include_str!("shaders/cas.wgsl").to_string());
        #[cfg(target_arch = "wasm32")]
        let source = include_str!("shaders/cas.wgsl").to_string();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("CAS Shader"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("CAS Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let (input_texture, input_texture_view) =
            create_input_texture(device, surface_format, width, height);
        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("CAS Params Buffer"),
            contents: bytemuck::cast_slice(&[CasParams {
                sharpness: DEFAULT_SHARPNESS,
                _padding: [0.0; 3],
            }]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("CAS Bind Group Layout"),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
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
        let bind_group =
            create_bind_group(device, &bind_group_layout, &input_texture_view, &sampler, &params_buffer);

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("CAS Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("CAS Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Self {
            pipeline,
            bind_group_layout,
            bind_group,
            params_buffer,
            input_texture,
            input_texture_view,
            sampler,
            enabled: false,
            sharpness: DEFAULT_SHARPNESS,
        }
    }

    /// The current sharpness, 0..=1.
    pub fn sharpness(&self) -> f32 {
        self.sharpness
    }

    /// Sets the sharpness, clamped to 0..=1 (0 = mildest, 1 = strongest), and uploads it.
    pub fn set_sharpness(&mut self, queue: &wgpu::Queue, sharpness: f32) {
        self.sharpness = if sharpness.is_finite() { sharpness.clamp(0.0, 1.0) } else { DEFAULT_SHARPNESS };
        queue.write_buffer(
            &self.params_buffer,
            0,
            bytemuck::cast_slice(&[CasParams { sharpness: self.sharpness, _padding: [0.0; 3] }]),
        );
    }

    /// Rebuilds the input texture (and its bind group) at the new size.
    pub fn resize(&mut self, device: &wgpu::Device, surface_format: wgpu::TextureFormat, width: u32, height: u32) {
        let (texture, view) = create_input_texture(device, surface_format, width, height);
        self.input_texture = texture;
        self.input_texture_view = view;
        self.bind_group = create_bind_group(
            device,
            &self.bind_group_layout,
            &self.input_texture_view,
            &self.sampler,
            &self.params_buffer,
        );
    }
}

/// Records the CAS pass: [`CasState::input_texture_view`] → `output_view`. With a `region`
/// (`[x, y, w, h]` in pixels, split screen) only that rectangle is written and the rest of the
/// output is kept.
#[tracing::instrument(skip_all, level = "trace")]
pub fn run_cas_pass(
    cas: &CasState,
    encoder: &mut wgpu::CommandEncoder,
    output_view: &wgpu::TextureView,
    region: Option<[f32; 4]>,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("CAS Pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: output_view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: if region.is_some() {
                    wgpu::LoadOp::Load
                } else {
                    wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                },
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        ..Default::default()
    });
    if let Some([x, y, w, h]) = region {
        pass.set_viewport(x, y, w, h, 0.0, 1.0);
    }
    pass.set_pipeline(&cas.pipeline);
    pass.set_bind_group(0, &cas.bind_group, &[]);
    pass.draw(0..3, 0..1);
}

fn create_input_texture(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("CAS Input Texture"),
        size: wgpu::Extent3d { width: width.max(1), height: height.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        // COPY_DST so a test can put a known picture in it.
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

fn create_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    input: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    params: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("CAS Bind Group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(input) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
            wgpu::BindGroupEntry { binding: 2, resource: params.as_entire_binding() },
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u32 = 64; // 64 × 4 bytes = 256: one aligned row
    const H: u32 = 4;

    /// A soft vertical edge: 50 on the left, one 128 column at x = 32, 200 to the right.
    fn soft_edge() -> Vec<u8> {
        let mut px = Vec::with_capacity((W * H * 4) as usize);
        for _ in 0..H {
            for x in 0..W {
                let v = match x {
                    0..32 => 50,
                    32 => 128,
                    _ => 200,
                };
                px.extend_from_slice(&[v, v, v, 255]);
            }
        }
        px
    }

    /// Runs CAS over `input` at `sharpness` and returns the output's red channel, row 1.
    async fn sharpen(device: &wgpu::Device, queue: &wgpu::Queue, input: &[u8], sharpness: f32) -> Vec<u8> {
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut cas = CasState::new(device, format, W, H);
        cas.set_sharpness(queue, sharpness);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &cas.input_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            input,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(W * 4), rows_per_image: Some(H) },
            wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        );
        let out = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cas_test_out"),
            size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = out.create_view(&Default::default());
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (W * H * 4) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        run_cas_pass(&cas, &mut encoder, &view, None);
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &out,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(W * 4), rows_per_image: Some(H) },
            },
            wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        );
        queue.submit(Some(encoder.finish()));
        let slice = staging.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        let data = slice.get_mapped_range().expect("mapped").to_vec();
        (0..W as usize).map(|x| data[(W as usize + x) * 4]).collect()
    }

    /// CAS sharpens an edge, leaves flat areas alone, and does more at a higher sharpness.
    ///
    /// The edge is measured as the step across the soft column: input 200 − 50 = 150 between
    /// x = 31 and x = 33. Each mutation measured red: a shader that returns its input gives
    /// exactly 150; dropping the `1 + 4w` normalisation gives 124 (and moves every flat pixel); a
    /// sharpness that never reaches the shader gives the same step, 171, at 0 and at 1. The flat
    /// check is the one an over-eager kernel trips — a flat area has nothing to sharpen.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn cas_sharpens_edges_and_leaves_flat_areas_alone() {
        let _gpu = crate::test_gpu::gpu_lock();
        pollster::block_on(async {
            let Some((device, queue)) = crate::test_gpu::headless_device().await else {
                eprintln!("skipping: no usable GPU adapter");
                return;
            };
            let input = soft_edge();
            let mild = sharpen(&device, &queue, &input, 0.0).await;
            let strong = sharpen(&device, &queue, &input, 1.0).await;
            let step = |row: &[u8]| row[33] as i32 - row[31] as i32;
            assert!(step(&mild) > 150, "sharpness 0 left the edge step at {} (input 150)", step(&mild));
            for (x, want) in [(4usize, 50u8), (20, 50), (45, 200), (60, 200)] {
                assert!(
                    strong[x].abs_diff(want) <= 1,
                    "a flat pixel at x = {x} moved from {want} to {}",
                    strong[x]
                );
            }
            assert!(
                step(&strong) > step(&mild),
                "sharpness 1 gives a step of {} and 0 gives {} — the parameter does not reach the shader",
                step(&strong),
                step(&mild)
            );
        });
    }
}
