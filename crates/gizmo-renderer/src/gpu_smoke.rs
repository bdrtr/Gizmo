//! VOLUMETRİK DUMAN (T6) — 3B yoğunluk grid sim'i + raymarch render.
//! `smoke_advect.wgsl` (compute): grid'i semi-Lagrangian advekte eder (prosedürel buoyancy +
//! curl hız alanı), kaynaktan enjekte eder, dissipation uygular (src→dst ping-pong).
//! `smoke_raymarch.wgsl` (render): grid yoğunluğunu ışın boyunca yürütür (Beer-Lambert + güneş
//! saçılımı + sahne-derinliği occlusion), HDR'ye premultiplied-over kompozit eder. Billboard
//! DEĞİL — gerçek katılımcı ortam. Demo `Renderer.smoke = Some(..)` verip ayarlar; her frame
//! `render()` compute+raymarch yapar (parity ile ping-pong).

use std::sync::atomic::{AtomicU32, Ordering};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct SmokeParams {
    bounds_min: [f32; 4], // xyz = min, w = zaman
    bounds_max: [f32; 4], // xyz = max, w = absorption
    p0: [f32; 4],         // x=density_scale, y=(boş), z=steps, w=dt
    color: [f32; 4],      // rgb = renk, w = ambient
    grid: [f32; 4],       // x=N, z=source_radius, w=inject
    source: [f32; 4],     // xyz = kaynak, w = dissipation
    sim: [f32; 4],        // x=buoyancy, y=curl_strength, z=curl_scale
}

pub struct SmokeVolume {
    grid_n: u32,
    // Bind grup'lar buffer'ları canlı tutar; alan sadece sahiplik/ömür içindir.
    #[allow(dead_code)]
    density: [wgpu::Buffer; 2],
    params_buffer: wgpu::Buffer,

    compute_pipeline: wgpu::ComputePipeline,
    compute_bg: [wgpu::BindGroup; 2], // [0]: buf0→buf1, [1]: buf1→buf0

    raymarch_pipeline: wgpu::RenderPipeline,
    depth_layout: wgpu::BindGroupLayout,
    params_bind_group: wgpu::BindGroup,
    density_bg: [wgpu::BindGroup; 2], // raymarch: [i] buf i okur

    parity: AtomicU32, // güncel yoğunluk hangi buffer'da

    // Ayarlanabilir (demo yazar):
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    pub absorption: f32,
    pub density_scale: f32,
    pub steps: u32,
    pub color: [f32; 3],
    pub ambient: f32,
    pub source: [f32; 3],
    pub source_radius: f32,
    pub inject: f32,
    pub dissipation: f32,
    pub buoyancy: f32,
    pub curl_strength: f32,
    pub curl_scale: f32,
}

impl SmokeVolume {
    pub fn new(
        device: &wgpu::Device,
        scene_global_layout: &wgpu::BindGroupLayout,
        output_format: wgpu::TextureFormat,
    ) -> Self {
        let grid_n: u32 = 64;
        let cells = (grid_n * grid_n * grid_n) as usize;
        let zero = vec![0.0f32; cells];
        let mk_density = |label: &str| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::cast_slice(&zero),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            })
        };
        let density = [mk_density("smoke_density_0"), mk_density("smoke_density_1")];

        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("smoke_params"),
            contents: bytemuck::cast_slice(&[SmokeParams {
                bounds_min: [0.0; 4],
                bounds_max: [0.0; 4],
                p0: [0.0; 4],
                color: [0.0; 4],
                grid: [0.0; 4],
                source: [0.0; 4],
                sim: [0.0; 4],
            }]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // ── Compute (advect) ──
        let compute_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("smoke_compute_layout"),
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
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
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
        let mk_compute_bg = |src: &wgpu::Buffer, dst: &wgpu::Buffer, label: &str| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &compute_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: params_buffer.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: src.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: dst.as_entire_binding() },
                ],
            })
        };
        let compute_bg = [
            mk_compute_bg(&density[0], &density[1], "smoke_advect_0to1"),
            mk_compute_bg(&density[1], &density[0], "smoke_advect_1to0"),
        ];
        let advect_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Smoke Advect Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/smoke_advect.wgsl").into()),
        });
        let compute_pl_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Smoke Advect Layout"),
            bind_group_layouts: &[Some(&compute_layout)],
            immediate_size: 0,
        });
        let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Smoke Advect Pipeline"),
            layout: Some(&compute_pl_layout),
            module: &advect_shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        // ── Raymarch (render) ──
        let depth_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("smoke_depth_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let params_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("smoke_params_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let density_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("smoke_density_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let params_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("smoke_params_bg"),
            layout: &params_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: params_buffer.as_entire_binding() }],
        });
        let mk_density_bg = |buf: &wgpu::Buffer, label: &str| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &density_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() }],
            })
        };
        let density_bg = [
            mk_density_bg(&density[0], "smoke_rm_density_0"),
            mk_density_bg(&density[1], "smoke_rm_density_1"),
        ];

        let rm_shader = crate::pipeline::load_shader_composed(
            device,
            "crates/gizmo-renderer/src/shaders/smoke_raymarch.wgsl",
            include_str!("shaders/smoke_raymarch.wgsl"),
            "Smoke Raymarch Shader",
        );
        let rm_pl_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Smoke Raymarch Layout"),
            bind_group_layouts: &[
                Some(scene_global_layout),
                Some(&depth_layout),
                Some(&params_layout),
                Some(&density_layout),
            ],
            immediate_size: 0,
        });
        let raymarch_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Smoke Raymarch Pipeline"),
            layout: Some(&rm_pl_layout),
            vertex: wgpu::VertexState {
                module: &rm_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &rm_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: output_format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Self {
            grid_n,
            density,
            params_buffer,
            compute_pipeline,
            compute_bg,
            raymarch_pipeline,
            depth_layout,
            params_bind_group,
            density_bg,
            parity: AtomicU32::new(0),
            bounds_min: [-1.6, 0.05, -1.6],
            bounds_max: [1.6, 5.5, 1.6],
            absorption: 2.0,
            density_scale: 1.0,
            steps: 56,
            color: [0.95, 0.96, 1.0],
            ambient: 0.4,
            source: [0.0, 0.4, 0.0],
            source_radius: 0.6,
            inject: 3.0,
            dissipation: 0.985,
            buoyancy: 1.2,
            curl_strength: 1.6,
            curl_scale: 0.7,
        }
    }

    fn write_params(&self, queue: &wgpu::Queue, time: f32, dt: f32) {
        let p = SmokeParams {
            bounds_min: [self.bounds_min[0], self.bounds_min[1], self.bounds_min[2], time],
            bounds_max: [self.bounds_max[0], self.bounds_max[1], self.bounds_max[2], self.absorption],
            p0: [self.density_scale, 0.0, self.steps as f32, dt],
            color: [self.color[0], self.color[1], self.color[2], self.ambient],
            grid: [self.grid_n as f32, 0.0, self.source_radius, self.inject],
            source: [self.source[0], self.source[1], self.source[2], self.dissipation],
            sim: [self.buoyancy, self.curl_strength, self.curl_scale, 0.0],
        };
        queue.write_buffer(&self.params_buffer, 0, bytemuck::cast_slice(&[p]));
    }

    /// Bir sim adımı (advect compute) + volumetrik raymarch (HDR'ye). Ping-pong ile buffer değişir.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene_bg: &wgpu::BindGroup,
        hdr_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        time: f32,
        dt: f32,
    ) {
        // Advection çok büyük dt'de instabil olmasın; sabit küçük adım.
        let sim_dt = dt.clamp(1.0 / 240.0, 1.0 / 30.0);
        self.write_params(queue, time, sim_dt);

        // 1) Advect compute (src=cur → dst=other).
        let cur = self.parity.load(Ordering::Relaxed) as usize;
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Smoke Advect"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&self.compute_pipeline);
            cpass.set_bind_group(0, &self.compute_bg[cur], &[]);
            let wg = self.grid_n.div_ceil(4);
            cpass.dispatch_workgroups(wg, wg, wg);
        }
        let new_cur = 1 - cur;
        self.parity.store(new_cur as u32, Ordering::Relaxed);

        // 2) Raymarch (yeni yoğunluk buffer'ını oku).
        let depth_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("smoke_depth_bg"),
            layout: &self.depth_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(depth_view),
            }],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Smoke Raymarch Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: hdr_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.raymarch_pipeline);
        pass.set_bind_group(0, scene_bg, &[]);
        pass.set_bind_group(1, &depth_bg, &[]);
        pass.set_bind_group(2, &self.params_bind_group, &[]);
        pass.set_bind_group(3, &self.density_bg[new_cur], &[]);
        pass.draw(0..3, 0..1);
    }
}
