//! Local exposure — the frame's contrast compressed where it is, not over the whole frame.
//!
//! One exposure for the whole frame ([`Camera::exposure`](crate::components::Camera::exposure), and
//! [`auto_exposure`](crate::auto_exposure) moving it) cannot hold a night street and a car's bumper lit
//! by headlights five metres off at once: the bumper takes tens of times the light the road does, and
//! an exposure that keeps the street dark burns the bumper white. What Unreal Engine 5 calls *local
//! exposure* does what the eye does: each pixel is exposed for its own neighbourhood. This is its
//! bilateral-grid form.
//!
//! **The grid.** The screen in cells of [`CELL`]² pixels, by [`BINS`] bins of log2 luminance over
//! [`LOG2_RANGE`]: a cell's pixels are sorted by brightness, and each bin holds how many fell in it and
//! their mean (`shaders/local_exposure.wgsl`, `build`). Blurred along x, y and the bins (`blur_axis`), it
//! gives every pixel the average of its neighbourhood **among the pixels about as bright as it** — the
//! lit bumper is averaged with the bumper, not with the dark boot above it, so compressing one does not
//! darken the other: no halo.
//!
//! **The compression**, in the composite (`post_process.wgsl`), in log2 of the exposed luminance: the
//! neighbourhood's average `b` is pulled towards [`LocalExposureSettings::pivot`] — scaled by
//! `highlight_contrast` above it and `shadow_contrast` below — and the pixel keeps its difference from
//! `b` (its detail, scaled by `detail`):
//!
//! ```text
//! out = pivot + (b − pivot) · contrast + (l − b) · detail
//! ```
//!
//! At 1, 1 and 1 that is `l`; the composite skips the stage then, and while it is off the pass binds a
//! neutral group ([`LocalExposureBinding::neutral_bind_group`]), so a frame is bit-identical to one
//! without this module. Off by default, as auto exposure is. Native only: the browser's reduced
//! composite has no group to read it from.

use wgpu::util::DeviceExt;

/// A grid cell's side, in pixels: 60 × 34 cells at 1080p.
pub const CELL: u32 = 32;
/// How many luminance bins a cell is sorted into.
pub const BINS: u32 = 24;
/// The log2 luminance the bins span: 20 stops, from a night's black to well past a lamp.
pub const LOG2_RANGE: (f32, f32) = (-14.0, 6.0);
/// The grid's texels: `(w · m, w)`, premultiplied (see the shader).
const GRID_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// How the contrast is compressed. The defaults change nothing.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalExposureSettings {
    /// The contrast kept among neighbourhoods **brighter** than [`pivot`](Self::pivot): 1 keeps it,
    /// 0.5 halves it in stops — a bumper four stops over the pivot comes down to two.
    pub highlight_contrast: f32,
    /// The same for neighbourhoods darker than the pivot. Under 1 lifts the dark ones towards it.
    pub shadow_contrast: f32,
    /// The exposed luminance the contrast turns about: what is this bright stays as it is.
    pub pivot: f32,
    /// How much of each pixel's difference from its neighbourhood is kept: 1 keeps the texture of a
    /// compressed surface as it was.
    pub detail: f32,
    /// The neighbourhood's size: the blur's sigma across the screen, in cells.
    pub blur_cells: f32,
    /// How far apart in brightness pixels may be and still share a neighbourhood: the blur's sigma
    /// along the bins, in bins (a bin is 20/24 of a stop).
    pub blur_bins: f32,
}

impl Default for LocalExposureSettings {
    fn default() -> Self {
        Self { highlight_contrast: 1.0, shadow_contrast: 1.0, pivot: 0.18, detail: 1.0, blur_cells: 1.5, blur_bins: 1.0 }
    }
}

/// The uniform both the grid's passes and the composite read (`LocalParams` in both shaders).
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct LocalParams {
    grid: [u32; 4],
    range: [f32; 4],
    shape: [f32; 4],
    flags: [f32; 4],
}

impl LocalParams {
    fn new(settings: &LocalExposureSettings, on: bool) -> Self {
        Self {
            grid: [0, 0, BINS, CELL],
            range: [LOG2_RANGE.0, LOG2_RANGE.1, settings.blur_cells.max(0.0), settings.blur_bins.max(0.0)],
            shape: [
                settings.highlight_contrast.max(0.0),
                settings.shadow_contrast.max(0.0),
                settings.pivot.max(1e-6).log2(),
                settings.detail.max(0.0),
            ],
            flags: [f32::from(u8::from(on)), 0.0, 0.0, 0.0],
        }
    }
}

/// The composite pass's **group 5** — the grid, its sampler and the parameters. Lives on
/// `PostProcessState::local`.
#[non_exhaustive]
pub struct LocalExposureBinding {
    /// The group-5 layout, shared by the neutral group and every [`LocalExposureState`]'s.
    pub layout: wgpu::BindGroupLayout,
    /// Group 5 around a one-texel grid and parameters that say "off": what the composite binds while
    /// local exposure is not running.
    pub neutral_bind_group: wgpu::BindGroup,
}

/// The layout of the composite's group 5 (see [`LocalExposureBinding`]).
pub fn composite_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("local_exposure_composite_layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D3,
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
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            },
        ],
    })
}

fn grid_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("local_exposure_grid"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    })
}

fn grid_texture(device: &wgpu::Device, label: &str, size: [u32; 3]) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: size[2] },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: GRID_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
        view_formats: &[],
    })
}

fn composite_group(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, grid: &wgpu::TextureView, sampler: &wgpu::Sampler, params: &wgpu::Buffer) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("local_exposure_composite"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(grid) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
            wgpu::BindGroupEntry { binding: 2, resource: params.as_entire_binding() },
        ],
    })
}

/// The neutral group 5: a one-texel grid and parameters that say "off".
pub fn build_binding(device: &wgpu::Device) -> LocalExposureBinding {
    let layout = composite_layout(device);
    let grid = grid_texture(device, "local_exposure_neutral_grid", [1, 1, 1]).create_view(&Default::default());
    let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("local_exposure_neutral_params"),
        contents: bytemuck::bytes_of(&LocalParams::new(&LocalExposureSettings::default(), false)),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let neutral_bind_group = composite_group(device, &layout, &grid, &grid_sampler(device), &params);
    LocalExposureBinding { layout, neutral_bind_group }
}

/// One grid, for one frame size: its two textures (built into the first, blurred back and forth), the
/// blur passes' groups, and the composite's group around the result.
struct Grid {
    size: [u32; 2],
    dims: [u32; 3],
    a: wgpu::Texture,
    blur_groups: [wgpu::BindGroup; 3],
    composite: wgpu::BindGroup,
}

/// The grid and its passes. Lives on [`Renderer::local_exposure`](crate::Renderer::local_exposure).
pub struct LocalExposureState {
    /// Whether it runs. Off by default.
    pub enabled: bool,
    /// How the contrast is compressed; read every frame it runs.
    pub settings: LocalExposureSettings,
    params_buffer: wgpu::Buffer,
    params_group: wgpu::BindGroup,
    build_pipeline: wgpu::ComputePipeline,
    blur_pipeline: wgpu::ComputePipeline,
    build_layout: wgpu::BindGroupLayout,
    blur_layout: wgpu::BindGroupLayout,
    axis_buffers: [wgpu::Buffer; 3],
    sampler: wgpu::Sampler,
    composite_layout: wgpu::BindGroupLayout,
    grid: Option<Grid>,
}

impl LocalExposureState {
    /// Builds the passes. `composite_layout` is [`LocalExposureBinding::layout`] — the composite pass's
    /// group 5.
    pub fn new(device: &wgpu::Device, composite_layout: &wgpu::BindGroupLayout) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("local_exposure"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/local_exposure.wgsl").into()),
        });
        let params_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("local_exposure_params_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: GRID_FORMAT,
                view_dimension: wgpu::TextureViewDimension::D3,
            },
            count: None,
        };
        let build_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("local_exposure_build_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    },
                    count: None,
                },
                storage(1),
            ],
        });
        let blur_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("local_exposure_blur_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D3,
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    },
                    count: None,
                },
                storage(3),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let pipeline = |layout: &wgpu::BindGroupLayout, entry: &str| {
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(entry),
                bind_group_layouts: &[Some(&params_layout), Some(layout)],
                immediate_size: 0,
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let build_pipeline = pipeline(&build_layout, "build");
        let blur_pipeline = pipeline(&blur_layout, "blur_axis");
        let settings = LocalExposureSettings::default();
        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("local_exposure_params"),
            contents: bytemuck::bytes_of(&LocalParams::new(&settings, true)),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let params_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("local_exposure_params"),
            layout: &params_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: params_buffer.as_entire_binding() }],
        });
        let axis_buffers = [[1i32, 0, 0, 0], [0, 1, 0, 0], [0, 0, 1, 0]].map(|axis| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("local_exposure_axis"),
                contents: bytemuck::cast_slice(&axis),
                usage: wgpu::BufferUsages::UNIFORM,
            })
        });
        Self {
            enabled: false,
            settings,
            params_buffer,
            params_group,
            build_pipeline,
            blur_pipeline,
            build_layout,
            blur_layout,
            axis_buffers,
            sampler: grid_sampler(device),
            composite_layout: composite_layout.clone(),
            grid: None,
        }
    }

    /// Makes the grid for a frame of `width` × `height` if it is not that size yet, and uploads the
    /// settings. Call before [`record`](Self::record), each frame it runs.
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32) {
        let size = [width.max(1), height.max(1)];
        if self.grid.as_ref().is_none_or(|g| g.size != size) {
            let dims = [size[0].div_ceil(CELL), size[1].div_ceil(CELL), BINS];
            let a = grid_texture(device, "local_exposure_grid_a", dims);
            let b = grid_texture(device, "local_exposure_grid_b", dims);
            let (va, vb) = (a.create_view(&Default::default()), b.create_view(&Default::default()));
            // x: a → b, y: b → a, bins: a → b. The result is in b.
            let blur = |from: &wgpu::TextureView, to: &wgpu::TextureView, axis: &wgpu::Buffer| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("local_exposure_blur"),
                    layout: &self.blur_layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(from) },
                        wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(to) },
                        wgpu::BindGroupEntry { binding: 4, resource: axis.as_entire_binding() },
                    ],
                })
            };
            let blur_groups = [blur(&va, &vb, &self.axis_buffers[0]), blur(&vb, &va, &self.axis_buffers[1]), blur(&va, &vb, &self.axis_buffers[2])];
            let composite = composite_group(device, &self.composite_layout, &vb, &self.sampler, &self.params_buffer);
            self.grid = Some(Grid { size, dims, a, blur_groups, composite });
        }
        queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(&LocalParams::new(&self.settings, true)));
    }

    /// Records the grid's build from `hdr_view` and its three blurs. After [`prepare`](Self::prepare),
    /// with the scene in the HDR target, before the composite reads
    /// [`composite_bind_group`](Self::composite_bind_group).
    pub fn record(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, hdr_view: &wgpu::TextureView) {
        let Some(grid) = self.grid.as_ref() else { return };
        let build_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("local_exposure_build"),
            layout: &self.build_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(hdr_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&grid.a.create_view(&Default::default())) },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("local_exposure"), timestamp_writes: None });
        pass.set_bind_group(0, &self.params_group, &[]);
        pass.set_pipeline(&self.build_pipeline);
        pass.set_bind_group(1, &build_group, &[]);
        pass.dispatch_workgroups(grid.dims[0], grid.dims[1], 1);
        pass.set_pipeline(&self.blur_pipeline);
        for group in &grid.blur_groups {
            pass.set_bind_group(1, group, &[]);
            pass.dispatch_workgroups(grid.dims[0].div_ceil(4), grid.dims[1].div_ceil(4), grid.dims[2].div_ceil(4));
        }
    }

    /// The composite pass's group 5 around the grid, once [`prepare`](Self::prepare) has made one.
    pub fn composite_bind_group(&self) -> Option<&wgpu::BindGroup> {
        self.grid.as_ref().map(|g| &g.composite)
    }
}
