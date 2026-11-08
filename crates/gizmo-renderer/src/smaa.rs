//! SMAA 1x: morphological anti-aliasing that finds edges, measures the shape of each, and blends along it.
//!
//! It takes FXAA's place in the post chain when it is on (`composite → SMAA → [CAS] → output`), and it
//! is a different trade: FXAA blurs whatever has contrast, so it softens texture and text as well as
//! stair steps; SMAA looks the steps' lengths up in a precomputed area table and blends only as much as a
//! straight edge would have covered, so detail away from edges keeps its sharpness. Three passes:
//! edges, blending weights, neighbourhood blending (`shaders/smaa.wgsl`, Bevy's port of the reference).
//!
//! Edge detection reads the frame as stored, display-encoded, which is the space SMAA's thresholds are
//! for; the blend reads it as linear light and writes it back through the target's own encoding.

/// How hard SMAA looks: the reference's presets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum SmaaQuality {
    /// Threshold 0.15, 4 search steps, no diagonals or corners.
    Low,
    /// Threshold 0.1, 8 search steps, no diagonals or corners.
    Medium,
    /// Threshold 0.1, 16 search steps, diagonals and corners. The default.
    #[default]
    High,
    /// Threshold 0.05, 32 search steps, diagonals and corners.
    Ultra,
}

impl SmaaQuality {
    fn def(self) -> &'static str {
        match self {
            SmaaQuality::Low => "SMAA_PRESET_LOW",
            SmaaQuality::Medium => "SMAA_PRESET_MEDIUM",
            SmaaQuality::High => "SMAA_PRESET_HIGH",
            SmaaQuality::Ultra => "SMAA_PRESET_ULTRA",
        }
    }
}

const EDGES_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg8Unorm;
const WEIGHTS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// SMAA's state: its input, the two intermediate targets, the reference's tables and the three pipelines.
/// See the [module docs](self).
pub struct SmaaState {
    quality: SmaaQuality,
    format: wgpu::TextureFormat,
    /// The composite renders here when SMAA is on.
    pub input_texture: wgpu::Texture,
    /// Its view, as the composite's target.
    pub input_texture_view: wgpu::TextureView,
    edges: wgpu::TextureView,
    weights: wgpu::TextureView,
    area: wgpu::TextureView,
    search: wgpu::TextureView,
    linear: wgpu::Sampler,
    info: wgpu::Buffer,
    colour_layout: wgpu::BindGroupLayout,
    edge_layout: wgpu::BindGroupLayout,
    weight_layout: wgpu::BindGroupLayout,
    blend_layout: wgpu::BindGroupLayout,
    pipelines: [wgpu::RenderPipeline; 3],
    groups: Groups,
}

/// The bind groups that hold the size-dependent views, built again on a resize.
struct Groups {
    /// The input as stored (display-encoded), for edge detection.
    encoded: wgpu::BindGroup,
    /// The input as linear light, for the blend.
    linear: wgpu::BindGroup,
    edge: wgpu::BindGroup,
    weight: wgpu::BindGroup,
    blend: wgpu::BindGroup,
}

impl SmaaState {
    /// SMAA for a frame of `format`, `width` × `height`, at the default quality.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat, width: u32, height: u32) -> Self {
        let table = |label, width, height, format, bytes: &[u8], texel: u32| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                texture.as_image_copy(),
                bytes,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * texel), rows_per_image: Some(height) },
                wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            );
            texture.create_view(&Default::default())
        };
        let area = table("SMAA area", 160, 560, wgpu::TextureFormat::Rg8Unorm, include_bytes!("smaa/area_tex.bin"), 2);
        let search = table("SMAA search", 64, 16, wgpu::TextureFormat::R8Unorm, include_bytes!("smaa/search_tex.bin"), 1);
        let linear = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("SMAA"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let info = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("SMAA info"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                multisampled: false,
                view_dimension: wgpu::TextureViewDimension::D2,
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
            },
            count: None,
        };
        let sampler = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let layout = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(label), entries })
        };
        let colour_layout = layout(
            "SMAA colour",
            &[
                texture(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        );
        let edge_layout = layout("SMAA edges", &[sampler(0)]);
        let weight_layout = layout("SMAA weights", &[texture(0), sampler(1), texture(2), texture(3)]);
        let blend_layout = layout("SMAA blend", &[texture(0), sampler(1)]);
        let quality = SmaaQuality::default();
        let pipelines = build_pipelines(device, quality, format, &colour_layout, [&edge_layout, &weight_layout, &blend_layout]);
        let (input_texture, input_texture_view, edges, weights) = targets(device, format, width, height);
        let layouts = [&colour_layout, &edge_layout, &weight_layout, &blend_layout];
        let views = Views { input: &input_texture, edges: &edges, weights: &weights, area: &area, search: &search };
        let groups = groups(device, queue, format, layouts, views, &linear, &info, width, height);
        Self {
            quality,
            format,
            input_texture,
            input_texture_view,
            edges,
            weights,
            area,
            search,
            linear,
            info,
            colour_layout,
            edge_layout,
            weight_layout,
            blend_layout,
            pipelines,
            groups,
        }
    }

    /// The preset in use.
    #[must_use]
    pub fn quality(&self) -> SmaaQuality {
        self.quality
    }

    /// Builds the pipelines again for `quality`.
    pub fn set_quality(&mut self, device: &wgpu::Device, quality: SmaaQuality) {
        if quality == self.quality {
            return;
        }
        self.quality = quality;
        self.pipelines = build_pipelines(
            device,
            quality,
            self.format,
            &self.colour_layout,
            [&self.edge_layout, &self.weight_layout, &self.blend_layout],
        );
    }

    /// New targets for a frame of `width` × `height` in `format`.
    pub fn resize(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat, width: u32, height: u32) {
        if format != self.format {
            self.format = format;
            self.pipelines = build_pipelines(
                device,
                self.quality,
                format,
                &self.colour_layout,
                [&self.edge_layout, &self.weight_layout, &self.blend_layout],
            );
        }
        let (input_texture, input_texture_view, edges, weights) = targets(device, format, width, height);
        self.input_texture = input_texture;
        self.input_texture_view = input_texture_view;
        self.edges = edges;
        self.weights = weights;
        self.rebind(device, queue, width, height);
    }

    fn rebind(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32) {
        let layouts = [&self.colour_layout, &self.edge_layout, &self.weight_layout, &self.blend_layout];
        let views = Views {
            input: &self.input_texture,
            edges: &self.edges,
            weights: &self.weights,
            area: &self.area,
            search: &self.search,
        };
        self.groups = groups(device, queue, self.format, layouts, views, &self.linear, &self.info, width, height);
    }
}

/// What the bind groups point at.
struct Views<'a> {
    input: &'a wgpu::Texture,
    edges: &'a wgpu::TextureView,
    weights: &'a wgpu::TextureView,
    area: &'a wgpu::TextureView,
    search: &'a wgpu::TextureView,
}

/// The bind groups for a frame of `width` × `height`, and its size in the info uniform.
fn groups(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    format: wgpu::TextureFormat,
    [colour_layout, edge_layout, weight_layout, blend_layout]: [&wgpu::BindGroupLayout; 4],
    views: Views<'_>,
    linear: &wgpu::Sampler,
    info: &wgpu::Buffer,
    width: u32,
    height: u32,
) -> Groups {
    let (w, h) = (width.max(1) as f32, height.max(1) as f32);
    queue.write_buffer(info, 0, bytemuck::cast_slice(&[1.0 / w, 1.0 / h, w, h]));
    let view = |format| views.input.create_view(&wgpu::TextureViewDescriptor { format: Some(format), ..Default::default() });
    let colour = |view: &wgpu::TextureView| {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("SMAA colour"),
            layout: colour_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) },
                wgpu::BindGroupEntry { binding: 1, resource: info.as_entire_binding() },
            ],
        })
    };
    let group = |layout, entries: &[wgpu::BindGroupEntry]| {
        device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("SMAA"), layout, entries })
    };
    let sampler = || wgpu::BindingResource::Sampler(linear);
    Groups {
        encoded: colour(&view(format.remove_srgb_suffix())),
        linear: colour(&view(format)),
        edge: group(edge_layout, &[wgpu::BindGroupEntry { binding: 0, resource: sampler() }]),
        weight: group(
            weight_layout,
            &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(views.edges) },
                wgpu::BindGroupEntry { binding: 1, resource: sampler() },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(views.search) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(views.area) },
            ],
        ),
        blend: group(
            blend_layout,
            &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(views.weights) },
                wgpu::BindGroupEntry { binding: 1, resource: sampler() },
            ],
        ),
    }
}

/// The input (with a display-encoded view besides its own), and the edges and weights targets.
fn targets(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::TextureView, wgpu::TextureView) {
    let size = wgpu::Extent3d { width: width.max(1), height: height.max(1), depth_or_array_layers: 1 };
    let encoded = format.remove_srgb_suffix();
    let input = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("SMAA input"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        // COPY_DST: a test writes its picture straight in.
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: if encoded == format { &[] } else { std::slice::from_ref(&encoded) },
    });
    let input_view = input.create_view(&Default::default());
    let target = |label, format| {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default())
    };
    (input, input_view, target("SMAA edges", EDGES_FORMAT), target("SMAA weights", WEIGHTS_FORMAT))
}

fn build_pipelines(
    device: &wgpu::Device,
    quality: SmaaQuality,
    format: wgpu::TextureFormat,
    colour: &wgpu::BindGroupLayout,
    [edges, weights, blend]: [&wgpu::BindGroupLayout; 3],
) -> [wgpu::RenderPipeline; 3] {
    let source = include_str!("shaders/smaa.wgsl");
    let pass = |stage: &str, vertex: &str, fragment: &str, layout: &wgpu::BindGroupLayout, target: wgpu::TextureFormat| {
        let defs = std::collections::HashMap::from([
            (stage.to_string(), naga_oil::compose::ShaderDefValue::Bool(true)),
            (quality.def().to_string(), naga_oil::compose::ShaderDefValue::Bool(true)),
        ]);
        let wgsl = crate::pipeline::shaders::compose_wgsl(source, "smaa.wgsl", defs);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(stage),
            source: wgpu::ShaderSource::Wgsl(wgsl.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(stage),
            bind_group_layouts: &[Some(colour), Some(layout)],
            immediate_size: 0,
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(stage),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some(vertex),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some(fragment),
                targets: &[Some(wgpu::ColorTargetState { format: target, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    };
    [
        pass("SMAA_EDGE_DETECTION", "edge_detection_vertex_main", "luma_edge_detection_fragment_main", edges, EDGES_FORMAT),
        pass(
            "SMAA_BLENDING_WEIGHT_CALCULATION",
            "blending_weight_calculation_vertex_main",
            "blending_weight_calculation_fragment_main",
            weights,
            WEIGHTS_FORMAT,
        ),
        pass("SMAA_NEIGHBORHOOD_BLENDING", "neighborhood_blending_vertex_main", "neighborhood_blending_fragment_main", blend, format),
    ]
}

/// One full-screen pass into `view`, cleared first unless it keeps what is there.
fn pass<'e>(encoder: &'e mut wgpu::CommandEncoder, label: &str, view: &wgpu::TextureView, clear: bool) -> wgpu::RenderPass<'e> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: if clear { wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT) } else { wgpu::LoadOp::Load },
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        ..Default::default()
    })
}

/// Runs SMAA from its input into `output_view`, inside `region` of it when one is given (a split screen's
/// view).
pub fn run_smaa_pass(smaa: &SmaaState, encoder: &mut wgpu::CommandEncoder, output_view: &wgpu::TextureView, region: Option<[f32; 4]>) {
    {
        let mut p = pass(encoder, "SMAA edges", &smaa.edges, true);
        p.set_pipeline(&smaa.pipelines[0]);
        p.set_bind_group(0, &smaa.groups.encoded, &[]);
        p.set_bind_group(1, &smaa.groups.edge, &[]);
        p.draw(0..3, 0..1);
    }
    {
        let mut p = pass(encoder, "SMAA weights", &smaa.weights, true);
        p.set_pipeline(&smaa.pipelines[1]);
        p.set_bind_group(0, &smaa.groups.encoded, &[]);
        p.set_bind_group(1, &smaa.groups.weight, &[]);
        p.draw(0..3, 0..1);
    }
    let mut p = pass(encoder, "SMAA blend", output_view, region.is_none());
    if let Some([x, y, w, h]) = region {
        p.set_viewport(x, y, w, h, 0.0, 1.0);
    }
    p.set_pipeline(&smaa.pipelines[2]);
    p.set_bind_group(0, &smaa.groups.linear, &[]);
    p.set_bind_group(1, &smaa.groups.blend, &[]);
    p.draw(0..3, 0..1);
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u32 = 64;

    /// Runs SMAA at `quality` over a `W` × `W` grey picture `shade` draws, and reads the result back.
    fn run(quality: SmaaQuality, shade: impl Fn(u32, u32) -> u8) -> Option<(Vec<u8>, Vec<u8>)> {
        let _gpu = crate::test_gpu::gpu_lock();
        pollster::block_on(async {
            let (device, queue) = crate::test_gpu::headless_device().await?;
            let format = wgpu::TextureFormat::Rgba8Unorm;
            let mut smaa = SmaaState::new(&device, &queue, format, W, W);
            smaa.set_quality(&device, quality);
            let input: Vec<u8> = (0..W * W).flat_map(|i| { let v = shade(i % W, i / W); [v, v, v, 255] }).collect();
            queue.write_texture(
                smaa.input_texture.as_image_copy(),
                &input,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(W * 4), rows_per_image: Some(W) },
                wgpu::Extent3d { width: W, height: W, depth_or_array_layers: 1 },
            );
            let out = device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d { width: W, height: W, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let staging = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: u64::from(W * W * 4),
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            run_smaa_pass(&smaa, &mut encoder, &out.create_view(&Default::default()), None);
            encoder.copy_texture_to_buffer(
                out.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(W * 4), rows_per_image: Some(W) },
                },
                wgpu::Extent3d { width: W, height: W, depth_or_array_layers: 1 },
            );
            queue.submit(Some(encoder.finish()));
            let slice = staging.slice(..);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
            let output = slice.get_mapped_range().expect("mapped").to_vec();
            Some((input, output))
        })
    }

    /// **SMAA smooths a staircase and leaves everything else alone.** A shallow edge, white below a line
    /// climbing one pixel every five, has long steps, which is what SMAA measures: after it, the pixels
    /// along the edge take values between black and white, where there were none, and the rows far from the
    /// edge are untouched to the byte. A vertical edge on the pixel grid has no steps, and SMAA, unlike a
    /// blur, leaves it exactly as it was. At every preset.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn smaa_smooths_a_staircase_and_leaves_straight_edges_and_flat_ground_alone() {
        let shallow = |x: u32, y: u32| if y * 5 > x + 100 { 255 } else { 0 };
        let vertical = |x: u32, _: u32| if x >= W / 2 { 255 } else { 0 };
        for quality in [SmaaQuality::Low, SmaaQuality::Medium, SmaaQuality::High, SmaaQuality::Ultra] {
            let Some((input, output)) = run(quality, shallow) else {
                eprintln!("skipping: no usable GPU adapter");
                return;
            };
            let red = |b: &[u8], x: u32, y: u32| b[((y * W + x) * 4) as usize];
            let between = |b: &[u8]| (0..W * W).filter(|&i| (8..248).contains(&b[(i * 4) as usize])).count();
            assert_eq!(between(&input), 0, "the picture starts hard-edged");
            let smoothed = between(&output);
            assert!(smoothed >= 20, "{quality:?}: only {smoothed} pixels along the edge took an in-between value");
            for x in 0..W {
                for y in (0..12).chain(W - 6..W) {
                    assert_eq!(red(&output, x, y), red(&input, x, y), "{quality:?}: ({x}, {y}) is far from the edge and moved");
                }
            }
            let (input, output) = run(quality, vertical).expect("the adapter was there a moment ago");
            assert_eq!(output, input, "{quality:?}: a straight edge on the grid has nothing to smooth");
        }
    }
}
