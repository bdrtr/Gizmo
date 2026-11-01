//! Auto exposure — the loop closed on the GPU.
//!
//! The pieces existed separately: the actuator (`Camera::exposure`, applied once in the composite
//! pass) and, since 2026-08-24, the sensor ([`LuminanceReduce`], the frame's mean luminance in a
//! storage buffer). What was missing was the loop between them, and the one thing it must not do
//! is read the number back: the `auto_exposure` demo measured a 4-byte readback taking the frame
//! from **0.19 ms to 0.79 ms**, all of it the stall, while recording the reduction costs 2 µs.
//!
//! So every step stays on the GPU. Each frame, after the scene is in the HDR target and before
//! tone mapping:
//!
//! 1. [`LuminanceReduce`] writes the mean luminance;
//! 2. one compute invocation (`shaders/auto_exposure.wgsl`) moves the exposure toward
//!    `key / mean`, in stops, at a frame-rate-independent rate, and stores it;
//! 3. the composite pass multiplies by it, after `Camera::exposure` — which therefore becomes
//!    exposure *compensation* while this is on.
//!
//! Off by default; a scene keeps its authored exposure until it asks:
//!
//! ```ignore
//! if let Some(ae) = renderer.auto_exposure.as_mut() {
//!     ae.enabled = true;
//!     ae.settings.adapt_to_dark = 0.5; // slower into the dark, as eyes are
//! }
//! ```
//!
//! While disabled, the composite pass binds a buffer holding exactly `1.0`, so a frame is
//! bit-identical to one rendered without this module. Native only: the browser's reduced post
//! shader has no exposure stage to feed, and WebGL2 has no storage buffers in fragment shaders.

use crate::luminance::LuminanceReduce;
use wgpu::util::DeviceExt;

/// How the exposure is chosen and how fast it follows.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutoExposureSettings {
    /// The mean luminance the frame is exposed to — 0.18, "middle grey", by default. Raise it
    /// for a brighter picture overall.
    pub key: f32,
    /// How fast, per second, the exposure follows when the scene gets **brighter** (exposure
    /// going down). Higher is faster; 3.0 closes ~95 % of the gap in one second.
    pub adapt_to_bright: f32,
    /// How fast it follows when the scene gets **darker**. Slower than `adapt_to_bright` by
    /// default, as eyes are.
    pub adapt_to_dark: f32,
    /// The lowest exposure it will choose. Keeps a blinding scene from going grey.
    pub min_exposure: f32,
    /// The highest. Keeps a black scene from being pulled up into noise.
    pub max_exposure: f32,
}

impl Default for AutoExposureSettings {
    fn default() -> Self {
        Self {
            key: 0.18,
            adapt_to_bright: 3.0,
            adapt_to_dark: 1.0,
            min_exposure: 1.0 / 64.0,
            max_exposure: 64.0,
        }
    }
}

/// The uniform block of `auto_exposure.wgsl`.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct AdaptParams {
    key: f32,
    adapt_to_bright: f32,
    adapt_to_dark: f32,
    min_exposure: f32,
    max_exposure: f32,
    dt: f32,
    reset: f32,
    _pad: f32,
}

/// The exposure the composite pass multiplies by, and how it is fed: [`LuminanceReduce`] plus the
/// adaptation step. Lives on [`Renderer::auto_exposure`](crate::Renderer::auto_exposure).
#[non_exhaustive]
pub struct AutoExposureState {
    /// Whether the loop runs. `false` (the default) binds a neutral `1.0` instead.
    pub enabled: bool,
    /// Target and speeds, uploaded by [`update`](Self::update) every frame.
    pub settings: AutoExposureSettings,
    reduce: LuminanceReduce,
    params_buffer: wgpu::Buffer,
    exposure_buffer: wgpu::Buffer,
    readback_buffer: wgpu::Buffer,
    adapt_pipeline: wgpu::ComputePipeline,
    adapt_bind_group: wgpu::BindGroup,
    composite_bind_group: wgpu::BindGroup,
    /// Set until the first frame after enabling: snap to the target instead of fading in.
    reset: bool,
    was_enabled: bool,
}

impl AutoExposureState {
    /// Builds the loop against `hdr_view` (`width` × `height`). `composite_layout` is
    /// [`ExposureBinding::layout`](crate::post_process::ExposureBinding::layout) — the composite
    /// pass's group 3.
    pub fn new(
        device: &wgpu::Device,
        hdr_view: &wgpu::TextureView,
        width: u32,
        height: u32,
        composite_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let reduce = LuminanceReduce::new(device, hdr_view, width, height);
        let settings = AutoExposureSettings::default();
        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("auto_exposure_params"),
            contents: bytemuck::bytes_of(&params(&settings, 1.0 / 60.0, true)),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let exposure_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("auto_exposure_value"),
            contents: bytemuck::cast_slice(&[1.0f32, 0.0, 0.0, 0.0]),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
        let readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("auto_exposure_readback"),
            size: 4,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("auto_exposure"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/auto_exposure.wgsl").into()),
        });
        let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let adapt_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("auto_exposure_adapt_layout"),
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
                storage(1, true),
                storage(2, false),
            ],
        });
        let adapt_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("auto_exposure_adapt"),
            layout: &adapt_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: params_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: reduce.result_buffer().as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: exposure_buffer.as_entire_binding() },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("auto_exposure_adapt"),
            bind_group_layouts: &[Some(&adapt_layout)],
            immediate_size: 0,
        });
        let adapt_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("auto_exposure_adapt"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("adapt"),
            compilation_options: Default::default(),
            cache: None,
        });
        let composite_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("auto_exposure_composite"),
            layout: composite_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: exposure_buffer.as_entire_binding() }],
        });

        Self {
            enabled: false,
            settings,
            reduce,
            params_buffer,
            exposure_buffer,
            readback_buffer,
            adapt_pipeline,
            adapt_bind_group,
            composite_bind_group,
            reset: true,
            was_enabled: false,
        }
    }

    /// Uploads this frame's settings and `dt` (seconds since the last frame). Call once per frame
    /// before recording; the game path does. A host that never calls it **snaps** to the target
    /// every frame instead of adapting — the uploaded state is the initial "reset" one. That is
    /// what `gizmo-studio` does, on purpose: it post-processes the same frame twice (Scene and
    /// Game panels), and a gradual step taken twice a frame would chase two pictures.
    pub fn update(&mut self, queue: &wgpu::Queue, dt: f32) {
        if self.enabled && !self.was_enabled {
            self.reset = true;
        }
        self.was_enabled = self.enabled;
        if !self.enabled {
            return;
        }
        queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(&params(&self.settings, dt, self.reset)));
        self.reset = false;
    }

    /// Records the measurement and the adaptation step. Call after the scene is in the HDR target
    /// and before the composite pass reads [`composite_bind_group`](Self::composite_bind_group).
    pub fn record(&self, encoder: &mut wgpu::CommandEncoder) {
        self.reduce.record(encoder);
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("auto_exposure_adapt"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.adapt_pipeline);
        pass.set_bind_group(0, &self.adapt_bind_group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }

    /// The composite pass's group 3 while this is enabled.
    pub fn composite_bind_group(&self) -> &wgpu::BindGroup {
        &self.composite_bind_group
    }

    /// Rebinds the sensor to a resized HDR target. The exposure itself carries over.
    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        hdr_view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) {
        self.reduce.resize(device, hdr_view, width, height, queue);
    }

    /// The exposure as it stands on the GPU — **a stall**, for tests and debugging, not for a
    /// frame loop (that is the cost this module exists to avoid).
    pub fn read_exposure(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Option<f32> {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("auto_exposure_readback"),
        });
        encoder.copy_buffer_to_buffer(&self.exposure_buffer, 0, &self.readback_buffer, 0, 4);
        queue.submit(std::iter::once(encoder.finish()));
        let slice = self.readback_buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |v| {
            let _ = tx.send(v);
        });
        let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        rx.recv().ok()?.ok()?;
        let data = slice.get_mapped_range().ok()?;
        let value = f32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        drop(data);
        self.readback_buffer.unmap();
        Some(value)
    }
}

fn params(s: &AutoExposureSettings, dt: f32, reset: bool) -> AdaptParams {
    let min = s.min_exposure.max(1e-6);
    AdaptParams {
        key: s.key.max(0.0),
        adapt_to_bright: s.adapt_to_bright.max(0.0),
        adapt_to_dark: s.adapt_to_dark.max(0.0),
        min_exposure: min,
        max_exposure: s.max_exposure.max(min),
        dt: if dt.is_finite() { dt.max(0.0) } else { 0.0 },
        reset: if reset { 1.0 } else { 0.0 },
        _pad: 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u32 = 32;
    const H: u32 = 32;

    /// An `Rgba16Float` frame of one grey at `luminance` — Rec. 709 weights sum to 1, so a grey
    /// `v` has luminance `v`.
    fn grey_texture(device: &wgpu::Device, queue: &wgpu::Queue, texture: Option<&wgpu::Texture>, luminance: f32) -> wgpu::Texture {
        let texture = texture.cloned().unwrap_or_else(|| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("auto_exposure_test_hdr"),
                size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        });
        let h = half_bits(luminance);
        let texel = [h, h, h, half_bits(1.0)];
        let data: Vec<u8> = (0..W * H).flat_map(|_| texel.iter().flat_map(|v| v.to_le_bytes())).collect();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(W * 8), rows_per_image: Some(H) },
            wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        );
        texture
    }

    /// f32 → IEEE half, for the positive normal values these tests use.
    fn half_bits(v: f32) -> u16 {
        let bits = v.to_bits();
        let exp = ((bits >> 23) & 0xff) as i32 - 127 + 15;
        let mantissa = (bits >> 13) & 0x3ff;
        ((exp.clamp(0, 31) as u32) << 10 | mantissa) as u16
    }

    fn step(device: &wgpu::Device, queue: &wgpu::Queue, ae: &mut AutoExposureState, dt: f32) -> f32 {
        ae.update(queue, dt);
        let mut encoder = device.create_command_encoder(&Default::default());
        ae.record(&mut encoder);
        queue.submit(Some(encoder.finish()));
        ae.read_exposure(device, queue).expect("readback")
    }

    /// The loop's arithmetic, end to end on the GPU: the first enabled frame snaps to
    /// `key / mean`; afterwards each step closes `1 − e^(−dt·speed)` of the gap **in stops**, with
    /// the dark speed when the scene got darker and the bright speed when it got brighter; and the
    /// result stays inside `min_exposure..=max_exposure`.
    ///
    /// Every number here is the formula worked by hand, so each part of the shader is pinned: a
    /// linear (not log2) blend misses the second value, swapped speeds miss both directions, a
    /// missing clamp misses the last.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn exposure_snaps_then_adapts_in_stops_at_each_direction_s_speed() {
        let _gpu = crate::test_gpu::gpu_lock();
        pollster::block_on(async {
            let Some((device, queue)) = crate::test_gpu::headless_device().await else {
                eprintln!("skipping: no usable GPU adapter");
                return;
            };
            let bgl = crate::post_process::exposure_layout(&device);
            let hdr = grey_texture(&device, &queue, None, 0.72);
            let view = hdr.create_view(&Default::default());
            let mut ae = AutoExposureState::new(&device, &view, W, H, &bgl);
            ae.enabled = true;

            // Snap: 0.18 / 0.72 = 0.25.
            let e0 = step(&device, &queue, &mut ae, 0.1);
            assert!((e0 - 0.25).abs() < 1e-3, "first frame should snap to 0.25, got {e0}");

            // Darker scene, target 0.18 / 0.09 = 2.0 — three stops up, at adapt_to_dark = 1:
            // t = 1 − e^(−0.1) = 0.09516 → 2^(−2 + 3·0.09516) = 0.3049.
            grey_texture(&device, &queue, Some(&hdr), 0.09);
            let e1 = step(&device, &queue, &mut ae, 0.1);
            assert!((e1 - 0.3049).abs() < 2e-3, "one dark step should reach 0.3049, got {e1}");

            // Brighter again, target 0.25 — down from 0.3049 at adapt_to_bright = 3:
            // t = 1 − e^(−0.3) = 0.2592 → 2^(log2 0.3049 + (−2 − log2 0.3049)·0.2592).
            grey_texture(&device, &queue, Some(&hdr), 0.72);
            let e2 = step(&device, &queue, &mut ae, 0.1);
            let want = 2f32.powf(e1.log2() + (-2.0 - e1.log2()) * (1.0 - (-0.3f32).exp()));
            assert!((e2 - want).abs() < 2e-3, "one bright step should reach {want}, got {e2}");

            // Clamped: a near-black frame wants 0.18 / 0.001 = 180, the ceiling is 4.
            ae.settings.max_exposure = 4.0;
            grey_texture(&device, &queue, Some(&hdr), 0.001);
            let mut e = 0.0;
            for _ in 0..200 {
                e = step(&device, &queue, &mut ae, 0.1);
            }
            assert!((e - 4.0).abs() < 1e-2, "exposure should settle at the 4.0 ceiling, got {e}");
        });
    }
}
