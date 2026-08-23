//! Average scene luminance, reduced on the GPU.
//!
//! # What this is for
//!
//! Auto-exposure needs one number out of the frame: how bright it is. The actuator has always
//! existed (`Camera::exposure`); the sensor did not, and `CAPABILITY_GAPS.md` recorded it as "the
//! actuator is live and the sensor does not exist".
//!
//! That entry turned out to be half wrong. The frame *is* readable — `PostProcessState::hdr_texture`
//! is `TEXTURE_BINDING | COPY_SRC` and public — so a game can compute the number itself, and the
//! `auto_exposure` demo does. What it cannot do is compute it cheaply: pulling the frame to the CPU
//! costs **10.1–10.9 ms a sample**, effectively all of it `map_async` + `poll(Wait)` stalling the
//! GPU for four bytes.
//!
//! So what was missing was never access. It was a reduction, and none of the nine compute shaders
//! in the tree was one.
//!
//! # Shape
//!
//! Two dispatches over one storage buffer:
//!
//! 1. **Tile pass** — one workgroup per tile, each summing its texels and tree-reducing to a single
//!    partial, written to `sums[1 + workgroup]`.
//! 2. **Final pass** — one workgroup sums the partials into `sums[0]` and divides by the texel
//!    count.
//!
//! Two passes rather than atomics because f32 atomics are not core WebGPU, and rather than one
//! because a single workgroup walking the whole frame serialises it. `sums[0]` is the answer, at a
//! known offset, so a game that *does* want it on the CPU copies four bytes instead of a frame.
//!
//! # What it does not do
//!
//! It does not drive exposure. Reading the number and deciding what to do with it — target
//! brightness, adaptation rate, metering mask — is the game's, and the engine has no opinion about
//! any of them. This produces the measurement the actuator was missing, nothing else.

use wgpu::util::DeviceExt;

/// How many partial sums the tile pass produces.
///
/// 256 workgroups covers a 4K frame at ~32 k texels each, and keeps the final pass to one workgroup
/// of 64 walking 256 values. Larger would push more work into the serial pass for no gain.
const PARTIALS: u32 = 256;

/// Workgroup size, matching `@workgroup_size(64)` in the shader. Both must agree or the tree
/// reduction reads uninitialised scratch.
const WORKGROUP: u32 = 64;

/// The parameters both passes read. `repr(C)` and 16 bytes, so no padding question arises.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ReduceParams {
    width: u32,
    height: u32,
    partial_count: u32,
    stage: u32,
}

/// The GPU-side reduction: pipeline, buffers, and the bind group tying them to one HDR target.
pub struct LuminanceReduce {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    /// One params buffer **per stage**, written once at build and never again.
    ///
    /// Not one buffer rewritten between dispatches: `queue.write_buffer` orders against submission
    /// rather than against recording, so both dispatches would read whichever `stage` was written
    /// last and the tile pass would run the final-sum branch. That is the same trap
    /// [`SceneView`](crate::pipeline::SceneView) exists for, one level down, and the same fix —
    /// give the two writes nowhere to collide.
    params_buffers: [wgpu::Buffer; 2],
    /// `sums[0]` is the mean; `sums[1..=PARTIALS]` are the tile partials.
    sums_buffer: wgpu::Buffer,
    /// A 4-byte staging buffer, for callers that want the number on the CPU.
    readback_buffer: wgpu::Buffer,
    /// One bind group per stage, each holding that stage's params.
    bind_groups: [wgpu::BindGroup; 2],
    width: u32,
    height: u32,
}

impl LuminanceReduce {
    /// Builds the reduction against an HDR texture view of the given size.
    ///
    /// Rebuild — or call [`resize`](Self::resize) — when the target changes size, since the bind
    /// group holds the view and the params hold the dimensions.
    #[must_use]
    pub fn new(
        device: &wgpu::Device,
        hdr_view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("luminance_reduce"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("shaders/luminance_reduce.wgsl").into(),
            ),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("luminance_reduce_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        // `textureLoad`, not sampling — no filtering needed and none requested.
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("luminance_reduce_layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("luminance_reduce"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let params_buffers: [wgpu::Buffer; 2] = std::array::from_fn(|stage| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(&format!("luminance_reduce_params_{stage}")),
                contents: bytemuck::bytes_of(&ReduceParams {
                    width,
                    height,
                    partial_count: PARTIALS,
                    stage: stage as u32,
                }),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            })
        });
        let sums_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("luminance_reduce_sums"),
            size: u64::from(PARTIALS + 1) * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("luminance_reduce_readback"),
            size: 4,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_groups: [wgpu::BindGroup; 2] = std::array::from_fn(|stage| {
            Self::mk_bind_group(device, &layout, &params_buffers[stage], hdr_view, &sums_buffer)
        });

        Self {
            pipeline,
            layout,
            params_buffers,
            sums_buffer,
            readback_buffer,
            bind_groups,
            width,
            height,
        }
    }

    fn mk_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        params: &wgpu::Buffer,
        hdr_view: &wgpu::TextureView,
        sums: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("luminance_reduce_bg"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(hdr_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: sums.as_entire_binding(),
                },
            ],
        })
    }

    /// Rebinds to a resized HDR target.
    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        hdr_view: &wgpu::TextureView,
        width: u32,
        height: u32,
        queue: &wgpu::Queue,
    ) {
        self.width = width;
        self.height = height;
        // Both stages carry the size, and both are written here — outside any recording, so the
        // submission-ordering question does not arise.
        for (stage, buf) in self.params_buffers.iter().enumerate() {
            queue.write_buffer(
                buf,
                0,
                bytemuck::bytes_of(&ReduceParams {
                    width,
                    height,
                    partial_count: PARTIALS,
                    stage: stage as u32,
                }),
            );
        }
        self.bind_groups = std::array::from_fn(|stage| {
            Self::mk_bind_group(
                device,
                &self.layout,
                &self.params_buffers[stage],
                hdr_view,
                &self.sums_buffer,
            )
        });
    }

    /// Records both passes into `encoder`.
    ///
    /// Call it after the frame has been drawn into the HDR target and before post-processing reads
    /// it. The result lands in `sums[0]` on the GPU; nothing is read back unless
    /// [`read_back`](Self::read_back) is called.
    /// Nothing is written to a buffer here — both stages' params were fixed at build — so the two
    /// dispatches genuinely differ, and compute passes run in the order they are recorded.
    pub fn record(&self, encoder: &mut wgpu::CommandEncoder) {
        for (stage, bind_group) in self.bind_groups.iter().enumerate() {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(if stage == 0 { "luma_tiles" } else { "luma_final" }),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.dispatch_workgroups(if stage == 0 { PARTIALS } else { 1 }, 1, 1);
        }
    }

    /// Copies `sums[0]` into the staging buffer and maps it — **four bytes, not a frame**.
    ///
    /// Still a stall, because reading GPU memory from the CPU is one. The difference from copying
    /// the whole HDR target is the size of the copy, and the demo measures both.
    ///
    /// Returns `None` if the map fails.
    pub fn read_back(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Option<f32> {
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("luminance_readback"),
        });
        enc.copy_buffer_to_buffer(&self.sums_buffer, 0, &self.readback_buffer, 0, 4);
        queue.submit(std::iter::once(enc.finish()));

        let slice = self.readback_buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |v| {
            let _ = tx.send(v);
        });
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        rx.recv().ok()?.ok()?;
        let data = slice.get_mapped_range().ok()?;
        let value = f32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        drop(data);
        self.readback_buffer.unmap();
        Some(value)
    }

    /// The buffer holding the result at offset 0, for a pass that wants it without a readback.
    #[must_use]
    pub fn result_buffer(&self) -> &wgpu::Buffer {
        &self.sums_buffer
    }

    /// Workgroup size the shader was compiled with, exposed so a caller sizing its own dispatch
    /// cannot disagree with it.
    #[must_use]
    pub const fn workgroup_size() -> u32 {
        WORKGROUP
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds an `Rgba16Float` texture from f32 RGB triples and reduces it.
    ///
    /// Returns `None` when there is no usable adapter, so the tests skip rather than fail on a
    /// machine without one — the same shape every other GPU test here uses.
    fn reduce(pixels: &[[f32; 3]], w: u32, h: u32) -> Option<f32> {
        let _gpu = crate::test_gpu::gpu_lock();
        let (device, queue) = pollster::block_on(crate::test_gpu::headless_device())?;

        // f32 → f16 by hand: the whole point is to feed the shader exactly what a real HDR target
        // holds, and `Rgba16Float` is what the post-process chain uses.
        let mut bytes = Vec::with_capacity(pixels.len() * 8);
        for p in pixels {
            for c in p {
                bytes.extend_from_slice(&f32_to_f16_bits(*c).to_le_bytes());
            }
            bytes.extend_from_slice(&f32_to_f16_bits(1.0).to_le_bytes());
        }

        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("luma_test"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 8),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());

        let r = LuminanceReduce::new(&device, &view, w, h);
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("luma_test"),
        });
        r.record(&mut enc);
        queue.submit(std::iter::once(enc.finish()));
        r.read_back(&device, &queue)
    }

    /// Minimal f32 → f16, enough for the test inputs (finite, in range).
    fn f32_to_f16_bits(v: f32) -> u16 {
        let bits = v.to_bits();
        let sign = ((bits >> 16) & 0x8000) as u16;
        let exp = ((bits >> 23) & 0xff) as i32 - 127 + 15;
        let frac = (bits >> 13) & 0x03ff;
        if exp <= 0 {
            return sign;
        }
        if exp >= 0x1f {
            return sign | 0x7c00;
        }
        sign | ((exp as u16) << 10) | frac as u16
    }

    /// A flat frame's mean is that value. The simplest thing that can be wrong, and the one a
    /// tree reduction with an off-by-one stride gets wrong first.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn a_flat_frame_reduces_to_its_own_luminance() {
        // 0.5 grey: luma = 0.5 * (0.2126 + 0.7152 + 0.0722) = 0.5.
        let px = vec![[0.5f32, 0.5, 0.5]; 64 * 64];
        let Some(got) = reduce(&px, 64, 64) else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        assert!(
            (got - 0.5).abs() < 0.01,
            "a flat 0.5 frame reduced to {got}, not 0.5"
        );
    }

    /// Half bright, half dark: the mean has to be the mean, not one half or the other.
    ///
    /// This is what catches a reduction that drops tiles — with 256 workgroups over 4096 texels,
    /// losing a stride still leaves a plausible-looking number.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn a_split_frame_reduces_to_the_average_of_its_halves() {
        let mut px = vec![[1.0f32, 1.0, 1.0]; 64 * 64];
        for p in px.iter_mut().take(64 * 32) {
            *p = [0.0, 0.0, 0.0];
        }
        let Some(got) = reduce(&px, 64, 64) else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        assert!(
            (got - 0.5).abs() < 0.02,
            "half-black half-white reduced to {got}, not 0.5"
        );
    }

    /// The Rec. 709 weights are applied, not a plain RGB average.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn the_channels_are_weighted_rather_than_averaged() {
        // Pure green: luma = 0.7152, a plain average would give 1/3.
        let px = vec![[0.0f32, 1.0, 0.0]; 64 * 64];
        let Some(got) = reduce(&px, 64, 64) else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        assert!(
            (got - 0.7152).abs() < 0.01,
            "pure green reduced to {got}; 0.7152 is weighted, 0.333 would be an average"
        );
    }

    /// One blown highlight must not take the mean with it.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn an_infinite_texel_does_not_poison_the_mean() {
        let mut px = vec![[0.25f32, 0.25, 0.25]; 64 * 64];
        px[100] = [f32::INFINITY; 3];
        let Some(got) = reduce(&px, 64, 64) else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        assert!(
            got.is_finite() && (got - 0.25).abs() < 0.01,
            "one infinite texel gave {got}; it should be skipped, leaving ~0.25"
        );
    }
}
