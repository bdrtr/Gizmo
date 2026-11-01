//! FXAA (Fast Approximate Anti-Aliasing) — Post-Processing Pass
//!
//! An implementation of Timothy Lottes' FXAA 3.11 Quality.
//! Runs as the final pass, after composite and tone mapping.
//! Edge detection is luma-based, which gives cheap edge smoothing on the GPU.

use wgpu::util::DeviceExt;

/// FXAA's GPU resources.
pub struct FxaaState {
    /// The FXAA pass.
    pub pipeline: wgpu::RenderPipeline,
    /// Its bind-group layout, kept so the bind group can be rebuilt on resize.
    pub bind_group_layout: wgpu::BindGroupLayout,
    /// The input texture and params. Rebuilt on resize.
    pub bind_group: wgpu::BindGroup,
    /// The [`FxaaParams`] uniform buffer.
    pub params_buffer: wgpu::Buffer,
    /// What post-processing writes into when FXAA is enabled — FXAA reads this and writes the
    /// swapchain, so it sees the final tone-mapped image rather than the HDR one.
    pub input_texture: wgpu::Texture,
    /// Its view.
    pub input_texture_view: wgpu::TextureView,
    /// The linear sampler FXAA reads its input with — the filtering is what lets it resolve an
    /// edge from neighbouring texels.
    pub sampler: wgpu::Sampler,
    /// Whether the pass runs. Off, post-processing writes straight to the output.
    pub enabled: bool,
    /// The preset; change it through [`set_quality`](Self::set_quality), which uploads it.
    pub quality: FxaaQuality,
}

/// The parameters handed to the FXAA shader.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FxaaParams {
    /// `1 / (width, height)` — the texel size the edge search steps by.
    pub inv_screen_size: [f32; 2],
    /// 1 = on, 0 = off. A uniform as well as a Rust flag, so the pass can be neutralised without
    /// rebuilding anything.
    pub fxaa_enabled: f32,
    /// The [`FxaaQuality`] preset, as its index (0 = Low … 3 = Ultra). Was a padding float.
    pub quality: f32,
}

/// How hard FXAA looks for edges, after the classic FXAA presets: a lower contrast threshold treats
/// fainter edges as edges, and more search steps follow an edge farther to find where it ends.
/// [`High`](Self::High) is the default — exactly what the pass did before presets existed.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FxaaQuality {
    /// Edge threshold 0.25, minimum 0.0833, 3 search steps — only strong edges, cheapest.
    Low,
    /// 0.166, 0.0833, 4 steps.
    Medium,
    /// 0.125, 0.0625, 6 steps — the default.
    #[default]
    High,
    /// 0.063, 0.0312, 12 steps — faint edges too, the longest search.
    Ultra,
}

impl FxaaQuality {
    fn index(self) -> f32 {
        match self {
            Self::Low => 0.0,
            Self::Medium => 1.0,
            Self::High => 2.0,
            Self::Ultra => 3.0,
        }
    }
}

impl FxaaState {
    /// Builds the FXAA pass and its input texture for a target of the given size.
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Self {
        // Shader yükleme
        let shader = {
            #[cfg(not(target_arch = "wasm32"))]
            let source = std::fs::read_to_string("demo/assets/shaders/fxaa.wgsl")
                .unwrap_or_else(|_| include_str!("shaders/fxaa.wgsl").to_string());

            #[cfg(target_arch = "wasm32")]
            let source = include_str!("shaders/fxaa.wgsl").to_string();

            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("FXAA Shader"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            })
        };

        // Sampler — bilinear filtering
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("FXAA Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        // Input texture (composite pass buraya yazar, FXAA buradan okur)
        let (input_texture, input_texture_view) =
            create_fxaa_texture(device, surface_format, width, height);

        // Parametreler
        let params = FxaaParams {
            inv_screen_size: [1.0 / width as f32, 1.0 / height as f32],
            fxaa_enabled: 1.0,
            quality: FxaaQuality::default().index(),
        };
        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("FXAA Params Buffer"),
            contents: bytemuck::cast_slice(&[params]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Bind group layout
        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("FXAA Bind Group Layout"),
                entries: &[
                    // @binding(0) — input texture
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
                    // @binding(1) — sampler
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    // @binding(2) — params uniform
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

        // Bind group
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("FXAA Bind Group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&input_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: params_buffer.as_entire_binding(),
                },
            ],
        });

        // Pipeline
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("FXAA Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("FXAA Pipeline"),
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
            enabled: true,
            quality: FxaaQuality::default(),
        }
    }

    /// Recreates the textures when the window size changes.
    pub fn resize(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, surface_format: wgpu::TextureFormat, width: u32, height: u32) {
        let (tex, view) = create_fxaa_texture(device, surface_format, width, height);
        self.input_texture = tex;
        self.input_texture_view = view;

        // Params güncelle
        let params = FxaaParams {
            inv_screen_size: [1.0 / width as f32, 1.0 / height as f32],
            fxaa_enabled: if self.enabled { 1.0 } else { 0.0 },
            quality: self.quality.index(),
        };
        queue.write_buffer(&self.params_buffer, 0, bytemuck::cast_slice(&[params]));

        // Bind group yeniden oluştur (yeni texture view)
        self.bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("FXAA Bind Group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.input_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.params_buffer.as_entire_binding(),
                },
            ],
        });
    }

    /// Sets the edge-search preset and uploads it.
    pub fn set_quality(&mut self, queue: &wgpu::Queue, quality: FxaaQuality) {
        self.quality = quality;
        queue.write_buffer(&self.params_buffer, 12, bytemuck::cast_slice(&[quality.index()]));
    }

    /// Turns FXAA on or off.
    pub fn set_enabled(&mut self, queue: &wgpu::Queue, enabled: bool) {
        self.enabled = enabled;
        let params = FxaaParams {
            inv_screen_size: [0.0, 0.0], // Resize'da güncellenir
            fxaa_enabled: if enabled { 1.0 } else { 0.0 },
            quality: self.quality.index(),
        };
        // Sadece enabled alanını güncelle
        queue.write_buffer(
            &self.params_buffer,
            8, // offset: inv_screen_size(8 byte) sonrası
            bytemuck::cast_slice(&[params.fxaa_enabled]),
        );
    }
}

/// Runs the FXAA render pass.
/// `input_view` is the composite's output, `output_view` the screen/swapchain. With a `region`
/// (split screen, `[x, y, w, h]` in pixels) only that rectangle of the output is written and the
/// rest is loaded, not cleared.
#[tracing::instrument(skip_all, level = "trace")]
pub fn run_fxaa_pass(
    fxaa: &FxaaState,
    encoder: &mut wgpu::CommandEncoder,
    output_view: &wgpu::TextureView,
    region: Option<[f32; 4]>,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("FXAA Pass"),
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
    pass.set_pipeline(&fxaa.pipeline);
    pass.set_bind_group(0, &fxaa.bind_group, &[]);
    pass.draw(0..3, 0..1);
}

/// Creates FXAA's input texture (the composite pass writes into it).
fn create_fxaa_texture(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("FXAA Input Texture"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
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
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u32 = 64; // 64 × 4 bytes: one aligned row
    const H: u32 = 64;

    /// Eight 8-row stripes, each split by a shallow staircase edge whose contrast grows stripe by
    /// stripe (0.02 … 0.32 in luma) — so how many stripes FXAA treats as edges is exactly what its
    /// contrast threshold decides.
    fn stripes() -> Vec<u8> {
        let contrast = [0.02f32, 0.04, 0.06, 0.08, 0.12, 0.16, 0.24, 0.32];
        let mut px = Vec::with_capacity((W * H * 4) as usize);
        for y in 0..H {
            let (stripe, local) = ((y / 8) as usize, y % 8);
            for x in 0..W {
                let bright = x > 2 * local + 20;
                let v = 0.4 + if bright { contrast[stripe] } else { 0.0 };
                let b = (v * 255.0).round() as u8;
                px.extend_from_slice(&[b, b, b, 255]);
            }
        }
        px
    }

    /// Runs FXAA (optionally at `quality`) over `input` and returns the output's bytes.
    async fn run(device: &wgpu::Device, queue: &wgpu::Queue, input: &[u8], quality: Option<FxaaQuality>) -> Vec<u8> {
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut fxaa = FxaaState::new(device, format, W, H);
        if let Some(q) = quality {
            fxaa.set_quality(queue, q);
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &fxaa.input_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            input,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(W * 4), rows_per_image: Some(H) },
            wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        );
        let out = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("fxaa_test_out"),
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
        run_fxaa_pass(&fxaa, &mut encoder, &view, None);
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &out, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
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
        slice.get_mapped_range().expect("mapped").to_vec()
    }

    /// **The presets reach the shader, in order, and `High` is what FXAA always did.** Over stripes
    /// of rising edge contrast, the number of pixels FXAA changes must grow from Low to High to
    /// Ultra (a lower threshold takes fainter edges), and `High` must equal a pass that was never
    /// given a preset — the default frame is untouched. Measured: 396 / 564 / 672 changed bytes;
    /// 564 for all three with the shader ignoring the preset. This pins the thresholds, not the
    /// search length: these edges are short, so a search fixed at 6 steps reads the same.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn fxaa_presets_take_fainter_edges_in_order_and_high_is_the_default() {
        let _gpu = crate::test_gpu::gpu_lock();
        pollster::block_on(async {
            let Some((device, queue)) = crate::test_gpu::headless_device().await else {
                eprintln!("skipping: no usable GPU adapter");
                return;
            };
            let input = stripes();
            let changed = |out: &[u8]| out.iter().zip(&input).filter(|(a, b)| a.abs_diff(**b) > 1).count();
            let default = run(&device, &queue, &input, None).await;
            let low = changed(&run(&device, &queue, &input, Some(FxaaQuality::Low)).await);
            let high_out = run(&device, &queue, &input, Some(FxaaQuality::High)).await;
            let ultra = changed(&run(&device, &queue, &input, Some(FxaaQuality::Ultra)).await);
            let high = changed(&high_out);
            assert_eq!(high_out, default, "High is not what FXAA did without a preset");
            assert!(low < high && high < ultra, "changed bytes should rise with the preset: Low {low}, High {high}, Ultra {ultra}");
        });
    }
}
