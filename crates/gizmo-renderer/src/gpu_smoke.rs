//! VOLUMETRİK DUMAN (T6) — 3B yoğunluk grid sim'i + raymarch render.
//! `smoke_advect.wgsl` (compute): grid'i semi-Lagrangian advekte eder (prosedürel buoyancy +
//! curl hız alanı), kaynaktan enjekte eder, dissipation uygular (src→dst ping-pong).
//! `smoke_raymarch.wgsl` (render): grid yoğunluğunu ışın boyunca yürütür (Beer-Lambert + güneş
//! saçılımı + sahne-derinliği occlusion), HDR'ye premultiplied-over kompozit eder. Billboard
//! DEĞİL — gerçek katılımcı ortam. Demo `Renderer.smoke = Some(..)` verip ayarlar; her frame
//! `render()` compute+raymarch yapar (parity ile ping-pong).

use std::sync::atomic::{AtomicU32, Ordering};
use wgpu::util::DeviceExt;

const IDENTITY4: [[f32; 4]; 4] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

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
    // Inverse of the camera view-projection, computed on the CPU. The raymarch reconstructs
    // world-space rays with THIS instead of inverting scene.view_proj in the shader — the WGSL
    // inverse_mat4 (common.wgsl) returns a wrong inverse for the perspective matrix, which
    // placed the whole volume in the wrong screen region (a thin sliver). Identity until set.
    inv_view_proj: [[f32; 4]; 4],
}

pub struct SmokeVolume {
    grid_n: u32,
    // Bind grup'lar buffer'ları canlı tutar; alan sadece sahiplik/ömür içindir.
    density: [wgpu::Buffer; 2],
    // Obstacle solidity field; kept alive by the compute bind groups (populated via
    // `set_obstacle_boxes`). The field itself is only used for ownership/lifetime + upload.
    obstacle: wgpu::Buffer,
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
    /// Bounded radial fill: smoke is pushed OUTWARD from the source so it spreads to fill a
    /// volume (rather than only rising), the push fading to zero past `fill_radius`. This is a
    /// force term, NOT a pressure solve — it does not re-route smoke around corners/doorways
    /// (that is roadmap T7). 0 = off (pure plume).
    pub fill_strength: f32,
    /// Radius the radial fill expands to before the outward push fades to zero (world units).
    pub fill_radius: f32,

    // Obstacle voxelization inputs, retained so the field can be re-baked when `bounds_*`
    // change (the solidity buffer holds grid indices baked against a specific bounds box).
    obstacle_boxes: Vec<([f32; 3], [f32; 3])>,
    obstacle_bounds: [[f32; 3]; 2], // (min, max) the obstacle field was last voxelized against
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
                // COPY_SRC so a step's density can be read back (headless tests / debug).
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
            })
        };
        let density = [mk_density("smoke_density_0"), mk_density("smoke_density_1")];

        // Obstacle voxel field (per-cell solidity: 0 = open air, 1 = inside solid geometry).
        // The advect shader forces solid cells to zero density, blocks velocity from pushing
        // smoke into solids, and refuses to advect density THROUGH a solid — so the smoke
        // conforms to and flows around walls/pillars (the CS2-style filling behaviour).
        // Populated on the CPU from AABBs via `set_obstacle_boxes`; all-zero = no obstacles.
        let obstacle = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("smoke_obstacle"),
            contents: bytemuck::cast_slice(&zero),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

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
                inv_view_proj: IDENTITY4,
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
                // binding 3: obstacle solidity field (read-only).
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
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
                    wgpu::BindGroupEntry { binding: 3, resource: obstacle.as_entire_binding() },
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
            obstacle,
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
            fill_strength: 0.0,
            fill_radius: 1.5,
            obstacle_boxes: Vec::new(),
            obstacle_bounds: [[-1.6, 0.05, -1.6], [1.6, 5.5, 1.6]],
        }
    }

    /// Grid resolution N (the volume is N×N×N cells).
    pub fn grid_n(&self) -> u32 {
        self.grid_n
    }

    /// The density buffer holding the most recent simulation step (for readback / debugging).
    pub fn density_buffer(&self) -> &wgpu::Buffer {
        &self.density[self.parity.load(Ordering::Relaxed) as usize]
    }

    /// Voxelize a set of world-space AABBs (`(min, max)`) into the obstacle solidity field.
    /// Smoke then conforms to and flows around these — pass the room's floor/walls/pillars (or
    /// collider AABBs). An empty slice clears all obstacles. The boxes are retained so the field
    /// can be re-baked if `bounds_*` change (see [`refresh_obstacles`]). Cheap CPU voxelization
    /// at N³ (64³ ≈ 262k cells) — call when the STATIC geometry changes, not every frame.
    ///
    /// **Conservative rasterization:** a cell is solid if the box OVERLAPS the cell's AABB (not
    /// merely if the box contains the cell centre) — so a wall thinner than one cell still
    /// produces a watertight ≥1-cell-thick solid layer instead of vanishing between cell centres.
    ///
    /// [`refresh_obstacles`]: SmokeVolume::refresh_obstacles
    pub fn set_obstacle_boxes(&mut self, queue: &wgpu::Queue, boxes: &[([f32; 3], [f32; 3])]) {
        self.obstacle_boxes = boxes.to_vec();
        self.bake_obstacles(queue);
    }

    /// Re-voxelize the retained obstacle boxes against the CURRENT `bounds_*`. Call this if you
    /// mutate `bounds_min`/`bounds_max` after `set_obstacle_boxes` — the solidity buffer stores
    /// grid indices baked against a specific bounds box, so a bounds change otherwise leaves the
    /// obstacles mapped to the wrong world positions.
    pub fn refresh_obstacles(&mut self, queue: &wgpu::Queue) {
        self.bake_obstacles(queue);
    }

    fn bake_obstacles(&mut self, queue: &wgpu::Queue) {
        let n = self.grid_n as usize;
        let bmin = self.bounds_min;
        let bmax = self.bounds_max;
        let cs = [
            (bmax[0] - bmin[0]) / n as f32,
            (bmax[1] - bmin[1]) / n as f32,
            (bmax[2] - bmin[2]) / n as f32,
        ];
        let mut solid = vec![0.0f32; n * n * n];
        for k in 0..n {
            for j in 0..n {
                for i in 0..n {
                    // The cell's world-space AABB [cmin, cmax].
                    let cmin = [
                        bmin[0] + i as f32 * cs[0],
                        bmin[1] + j as f32 * cs[1],
                        bmin[2] + k as f32 * cs[2],
                    ];
                    let cmax = [cmin[0] + cs[0], cmin[1] + cs[1], cmin[2] + cs[2]];
                    for (mn, mx) in &self.obstacle_boxes {
                        // AABB overlap on all three axes (conservative — thin walls survive).
                        if mn[0] <= cmax[0]
                            && mx[0] >= cmin[0]
                            && mn[1] <= cmax[1]
                            && mx[1] >= cmin[1]
                            && mn[2] <= cmax[2]
                            && mx[2] >= cmin[2]
                        {
                            solid[(k * n + j) * n + i] = 1.0;
                            break;
                        }
                    }
                }
            }
        }
        queue.write_buffer(&self.obstacle, 0, bytemuck::cast_slice(&solid));
        self.obstacle_bounds = [bmin, bmax];
    }

    /// Record the advection compute pass onto `encoder` and flip the ping-pong parity.
    /// Returns the buffer index now holding the freshest density.
    fn record_advect(&self, encoder: &mut wgpu::CommandEncoder) -> usize {
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
        new_cur
    }

    /// Run ONE simulation step (advect compute only, no rendering) on its own submission.
    /// Useful headless — e.g. to warm up the volume or to verify behaviour in tests. Returns
    /// the buffer index now holding the freshest density.
    #[tracing::instrument(skip_all, level = "trace")]
    pub fn step(&self, device: &wgpu::Device, queue: &wgpu::Queue, time: f32, dt: f32) -> usize {
        let sim_dt = dt.clamp(1.0 / 240.0, 1.0 / 30.0);
        tracing::trace!(grid_n = self.grid_n, dt = sim_dt, "[Smoke] advect step");
        // step() only advects (no raymarch) → the inverse view-proj is unused; identity.
        self.write_params(queue, time, sim_dt, IDENTITY4);
        let mut enc =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Smoke Step") });
        let new_cur = self.record_advect(&mut enc);
        queue.submit(Some(enc.finish()));
        new_cur
    }

    fn write_params(&self, queue: &wgpu::Queue, time: f32, dt: f32, inv_view_proj: [[f32; 4]; 4]) {
        // The obstacle field is voxelized against a specific bounds box; if bounds are mutated
        // after set_obstacle_boxes without refresh_obstacles, the solidity indices map to the
        // wrong world positions. Catch that footgun in debug builds.
        debug_assert!(
            self.obstacle_boxes.is_empty()
                || (self.obstacle_bounds[0] == self.bounds_min
                    && self.obstacle_bounds[1] == self.bounds_max),
            "smoke bounds changed after set_obstacle_boxes — call refresh_obstacles(queue)"
        );
        let p = SmokeParams {
            bounds_min: [self.bounds_min[0], self.bounds_min[1], self.bounds_min[2], time],
            bounds_max: [self.bounds_max[0], self.bounds_max[1], self.bounds_max[2], self.absorption],
            // p0.y carries fill_radius (advect only; raymarch ignores it).
            p0: [self.density_scale, self.fill_radius, self.steps as f32, dt],
            color: [self.color[0], self.color[1], self.color[2], self.ambient],
            grid: [self.grid_n as f32, 0.0, self.source_radius, self.inject],
            source: [self.source[0], self.source[1], self.source[2], self.dissipation],
            // sim.w carries fill_strength (radial outward push, 0 = pure plume).
            sim: [self.buoyancy, self.curl_strength, self.curl_scale, self.fill_strength],
            inv_view_proj,
        };
        queue.write_buffer(&self.params_buffer, 0, bytemuck::cast_slice(&[p]));
    }

    /// Bir sim adımı (advect compute) + volumetrik raymarch (HDR'ye). Ping-pong ile buffer değişir.
    #[allow(clippy::too_many_arguments)]
    #[tracing::instrument(skip_all, level = "trace")]
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
        // Inverse of the camera view-projection (column-major), computed on the CPU — the
        // raymarch reconstructs rays with this (the WGSL inverse_mat4 returns a wrong inverse).
        inv_view_proj: [[f32; 4]; 4],
    ) {
        // Advection çok büyük dt'de instabil olmasın; sabit küçük adım.
        let sim_dt = dt.clamp(1.0 / 240.0, 1.0 / 30.0);
        tracing::trace!(grid_n = self.grid_n, dt = sim_dt, "[Smoke] advect + raymarch");
        self.write_params(queue, time, sim_dt, inv_view_proj);

        // 1) Advect compute (src=cur → dst=other). Obstacle-aware (see smoke_advect.wgsl).
        let new_cur = self.record_advect(encoder);

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

#[cfg(test)]
mod tests {
    use super::SmokeVolume;

    async fn setup_headless_gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            flags: wgpu::InstanceFlags::default(),
            memory_budget_thresholds: Default::default(),
            backend_options: Default::default(),
            display: None,
        });
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .ok()?;
        adapter.request_device(&wgpu::DeviceDescriptor::default()).await.ok()
    }

    async fn read_f32(device: &wgpu::Device, queue: &wgpu::Queue, buffer: &wgpu::Buffer) -> Vec<f32> {
        let size = buffer.size();
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("smoke_readback"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        enc.copy_buffer_to_buffer(buffer, 0, &staging, 0, size);
        queue.submit(Some(enc.finish()));
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |v| tx.send(v).unwrap());
        let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        rx.recv().unwrap().unwrap();
        let data = slice.get_mapped_range();
        let out: Vec<f32> = bytemuck::cast_slice(&data).to_vec();
        drop(data);
        staging.unmap();
        out
    }

    // Minimal group-0 layout (SceneUniforms uniform @binding 0) so SmokeVolume::new can build
    // its raymarch pipeline headlessly — this also validates BOTH smoke shaders compile.
    fn scene_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("test_scene_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        })
    }

    // Render the smoke to an offscreen HDR with a real perspective camera (no depth occlusion)
    // and assert it covers a substantial fraction of the frame. Guards the ray reconstruction:
    // when the raymarch inverted scene.view_proj with the (buggy) WGSL inverse_mat4 the whole
    // volume collapsed to a <1% thin sliver in the wrong screen region; with the CPU-computed
    // inverse passed to render() it fills ~15%. A regression to the bad inverse fails here.
    #[test]
    fn raymarch_renders_the_volume_not_a_sliver() {
        use bytemuck::Zeroable;
        use gizmo_math::{Mat4, Vec3};
        use wgpu::util::DeviceExt;
        pollster::block_on(async {
            let Some((device, queue)) = setup_headless_gpu().await else { return };
            let layout = scene_layout(&device);
            fn f16_to_f32(hbits: u16) -> f32 {
                let sign = (hbits >> 15) & 1;
                let exp = (hbits >> 10) & 0x1f;
                let mant = hbits & 0x3ff;
                let v = if exp == 0 {
                    mant as f32 * 2f32.powi(-24)
                } else if exp == 0x1f {
                    1e4
                } else {
                    (1.0 + mant as f32 / 1024.0) * 2f32.powi(exp as i32 - 15)
                };
                if sign == 1 { -v } else { v }
            }
            let (w, h) = (320u32, 240u32);
            let mut s = SmokeVolume::new(&device, &layout, wgpu::TextureFormat::Rgba16Float);
            s.bounds_min = [-1.8, 0.02, -1.8];
            s.bounds_max = [1.8, 4.0, 1.8];
            s.source = [0.0, 0.8, 0.0];
            s.source_radius = 0.6;
            s.inject = 9.0;
            s.dissipation = 0.985;
            s.buoyancy = 1.7;
            s.curl_strength = 2.0;
            s.fill_strength = 2.5;
            s.fill_radius = 2.0;
            s.density_scale = 1.6;
            s.absorption = 2.8;
            s.set_obstacle_boxes(&queue, &[([0.45, 0.0, -0.25], [0.95, 3.2, 0.25])]);
            for f in 0..200 {
                s.step(&device, &queue, f as f32 / 60.0, 1.0 / 60.0);
            }

            // Scene uniforms — camera matching the demo (pos (6,3,7) → target (0,2.2,0)).
            let cam = Vec3::new(6.0, 3.0, 7.0);
            let view = Mat4::look_at_rh(cam, Vec3::new(0.0, 2.2, 0.0), Vec3::Y);
            let proj = Mat4::perspective_rh(45f32.to_radians(), w as f32 / h as f32, 0.1, 500.0);
            let vp_mat = proj * view;
            let vp = vp_mat.to_cols_array_2d();
            // The whole point of the fix: give the smoke the CORRECT inverse (CPU-computed).
            let inv_vp = vp_mat.inverse().to_cols_array_2d();
            let mut uni = crate::gpu_types::SceneUniforms::zeroed();
            uni.view_proj = vp;
            uni.camera_pos = [cam.x, cam.y, cam.z, 1.0];
            uni.sun_direction = [-0.3, -0.8, -0.3, 0.0];
            uni.sun_color = [1.0, 0.96, 0.9, 3.2];
            uni.exposure = 1.0;
            let uni_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("test_scene_uni"),
                contents: bytemuck::cast_slice(&[uni]),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let scene_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("test_scene_bg"),
                layout: &layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: uni_buf.as_entire_binding() }],
            });

            let hdr = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("test_hdr"), size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let hdr_view = hdr.create_view(&Default::default());
            let depth = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("test_depth"), size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let depth_view = depth.create_view(&Default::default());

            // Clear HDR to black + depth to 1.0 (far → NO occlusion).
            let mut enc = device.create_command_encoder(&Default::default());
            enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &hdr_view, depth_slice: None, resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None, occlusion_query_set: None, multiview_mask: None,
            });
            queue.submit(Some(enc.finish()));

            // Raymarch the smoke onto the HDR.
            let mut enc2 = device.create_command_encoder(&Default::default());
            s.render(&mut enc2, &device, &queue, &scene_bg, &hdr_view, &depth_view, 3.0, 1.0 / 60.0, inv_vp);
            queue.submit(Some(enc2.finish()));

            // Read back HDR (Rgba16Float, 8 bytes/px; 320*8=2560, 256-aligned).
            let bpr = w * 8;
            let staging = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("hdr_readback"), size: (bpr * h) as u64,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
            });
            let mut enc3 = device.create_command_encoder(&Default::default());
            enc3.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo { texture: &hdr, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                wgpu::TexelCopyBufferInfo { buffer: &staging, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: Some(h) } },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
            queue.submit(Some(enc3.finish()));
            let slice = staging.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |v| { let _ = tx.send(v); });
            let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
            let _ = rx.recv();
            let data = slice.get_mapped_range();
            let px: &[u16] = bytemuck::cast_slice(&data);
            let mut lit = 0u32;
            let mut rows_with_smoke = std::collections::BTreeSet::new();
            for y in 0..h {
                for x in 0..w {
                    let i = ((y * w + x) * 4) as usize;
                    let b = f16_to_f32(px[i]) + f16_to_f32(px[i + 1]) + f16_to_f32(px[i + 2]);
                    if b > 0.05 { lit += 1; rows_with_smoke.insert(y); }
                }
            }
            let total_px = w * h;
            let coverage = lit as f32 / total_px as f32;
            let span = rows_with_smoke.len();
            drop(data);
            staging.unmap();
            assert!(
                coverage > 0.05,
                "raymarch coverage {:.1}% is far too low — ray reconstruction is broken (the \
                 volume collapsed to a sliver); expected the box to fill ~15% of the frame",
                coverage * 100.0
            );
            // And it must span many rows (a real volume), not a one-row streak.
            assert!(span > 40, "smoke spans only {span} rows — not a volume");
        });
    }

    // A vertical wall down the middle of the volume, with the smoke source on the LEFT, must
    // BLOCK smoke from reaching the right half: obstacle cells stay empty, and no smoke tunnels
    // or advects across the wall. Without the obstacle-conforming advection (solid=0 +
    // no-penetration + no-tunnel backtrace) the radial fill + curl would spread smoke to the
    // right, so this is discriminating.
    #[test]
    fn smoke_conforms_to_and_does_not_cross_a_wall() {
        pollster::block_on(async {
            let Some((device, queue)) = setup_headless_gpu().await else {
                eprintln!("no GPU adapter — skipping smoke obstacle test");
                return;
            };
            let layout = scene_layout(&device);
            let mut smoke = SmokeVolume::new(&device, &layout, wgpu::TextureFormat::Rgba16Float);
            let n = smoke.grid_n() as usize;

            // Volume x,z ∈ [-1,1], y ∈ [0,2]. Source on the LEFT; strong radial fill so smoke
            // is actively pushed toward the wall (a strong test of the blocking).
            smoke.bounds_min = [-1.0, 0.0, -1.0];
            smoke.bounds_max = [1.0, 2.0, 1.0];
            smoke.source = [-0.55, 0.5, 0.0];
            smoke.source_radius = 0.35;
            smoke.inject = 6.0;
            smoke.buoyancy = 0.3;
            smoke.fill_strength = 2.5;
            smoke.fill_radius = 1.6;
            smoke.dissipation = 0.99;

            // Wall: x ∈ [-0.1, 0.1] (≈6 cells thick at N=64), full height/depth.
            smoke.set_obstacle_boxes(&queue, &[([-0.1, 0.0, -1.0], [0.1, 2.0, 1.0])]);

            // Warm up the volume.
            for f in 0..60 {
                smoke.step(&device, &queue, f as f32 * (1.0 / 60.0), 1.0 / 60.0);
            }
            let d = read_f32(&device, &queue, smoke.density_buffer()).await;
            assert_eq!(d.len(), n * n * n);

            // World x → grid i: i = (x + 1) / 2 * n.
            let xi = |x: f32| (((x + 1.0) / 2.0) * n as f32) as usize;
            let mut left = 0.0f64; // source side, x < -0.2
            let mut wall = 0.0f64; // inside the wall, x ∈ [-0.1, 0.1]
            let mut right = 0.0f64; // far side, x > 0.2
            // `right` starts at the wall's FAR FACE (w1), not a gap past it — the cells
            // immediately beyond are where any leak/tunnel would first show.
            let (li, w0, w1) = (xi(-0.2), xi(-0.1), xi(0.1));
            for k in 0..n {
                for j in 0..n {
                    for i in 0..n {
                        let v = d[(k * n + j) * n + i] as f64;
                        if i < li {
                            left += v;
                        } else if i >= w0 && i < w1 {
                            wall += v;
                        } else if i >= w1 {
                            right += v;
                        }
                    }
                }
            }

            assert!(left > 5.0, "smoke should accumulate on the source side (left={left})");
            assert!(wall < 1e-3, "obstacle cells must hold no smoke (wall={wall})");
            assert!(
                right < left * 0.02,
                "smoke must not cross the wall: right={right} vs left={left}"
            );
        });
    }

    // POSITIVE CONTROL (clean A/B): the EXACT same setup as the wall test but with NO obstacle.
    // The region the wall WOULD occupy (x ∈ [-0.1, 0.1]) now FILLS with smoke. Paired with the
    // wall test (same region ≈ 0 WITH the obstacle), this proves the blocking comes from the
    // obstacle mechanism itself, not the geometry/params: the only difference between the two
    // tests is `set_obstacle_boxes`, and it flips that region from full to empty.
    #[test]
    fn without_an_obstacle_the_wall_region_fills() {
        pollster::block_on(async {
            let Some((device, queue)) = setup_headless_gpu().await else {
                eprintln!("no GPU adapter — skipping smoke control test");
                return;
            };
            let layout = scene_layout(&device);
            let mut smoke = SmokeVolume::new(&device, &layout, wgpu::TextureFormat::Rgba16Float);
            let n = smoke.grid_n() as usize;
            smoke.bounds_min = [-1.0, 0.0, -1.0];
            smoke.bounds_max = [1.0, 2.0, 1.0];
            smoke.source = [-0.55, 0.5, 0.0];
            smoke.source_radius = 0.35;
            smoke.inject = 6.0;
            smoke.buoyancy = 0.3;
            smoke.fill_strength = 2.5;
            smoke.fill_radius = 1.6;
            smoke.dissipation = 0.99;
            // No obstacles — the ONLY difference from `smoke_conforms_to_and_does_not_cross_a_wall`.
            for f in 0..60 {
                smoke.step(&device, &queue, f as f32 * (1.0 / 60.0), 1.0 / 60.0);
            }
            let d = read_f32(&device, &queue, smoke.density_buffer()).await;
            let xi = |x: f32| (((x + 1.0) / 2.0) * n as f32) as usize;
            let (w0, w1) = (xi(-0.1), xi(0.1));
            let mut wall_region = 0.0f64;
            for k in 0..n {
                for j in 0..n {
                    for i in w0..w1 {
                        wall_region += d[(k * n + j) * n + i] as f64;
                    }
                }
            }
            assert!(
                wall_region > 5.0,
                "without an obstacle the wall region must fill (got {wall_region}); \
                 the wall test asserts this same region is ~0 WITH the obstacle"
            );
        });
    }

    // A horizontal ceiling obstacle above the source: rising smoke POOLS underneath and does
    // not penetrate above — exercises the +Y no-penetration + no-tunnel path (a different axis
    // and mechanism than the vertical-wall test).
    #[test]
    fn smoke_pools_under_a_ceiling() {
        pollster::block_on(async {
            let Some((device, queue)) = setup_headless_gpu().await else {
                eprintln!("no GPU adapter — skipping smoke ceiling test");
                return;
            };
            let layout = scene_layout(&device);
            let mut smoke = SmokeVolume::new(&device, &layout, wgpu::TextureFormat::Rgba16Float);
            let n = smoke.grid_n() as usize;
            smoke.bounds_min = [-1.0, 0.0, -1.0];
            smoke.bounds_max = [1.0, 2.0, 1.0];
            smoke.source = [0.0, 0.35, 0.0];
            smoke.source_radius = 0.4;
            smoke.inject = 6.0;
            smoke.buoyancy = 1.6; // strong rise → actively pushes into the ceiling
            smoke.fill_strength = 0.0;
            smoke.dissipation = 0.99;
            // Horizontal ceiling slab at y ∈ [1.2, 1.4], spanning x/z.
            smoke.set_obstacle_boxes(&queue, &[([-1.0, 1.2, -1.0], [1.0, 1.4, 1.0])]);
            for f in 0..80 {
                smoke.step(&device, &queue, f as f32 * (1.0 / 60.0), 1.0 / 60.0);
            }
            let d = read_f32(&device, &queue, smoke.density_buffer()).await;
            let yj = |y: f32| ((y / 2.0) * n as f32) as usize;
            let (below, c0, c1, above) = (yj(1.1), yj(1.2), yj(1.4), yj(1.5));
            let mut below_s = 0.0f64;
            let mut ceil_s = 0.0f64;
            let mut above_s = 0.0f64;
            for k in 0..n {
                for j in 0..n {
                    for i in 0..n {
                        let v = d[(k * n + j) * n + i] as f64;
                        if j < below {
                            below_s += v;
                        } else if j >= c0 && j < c1 {
                            ceil_s += v;
                        } else if j >= above {
                            above_s += v;
                        }
                    }
                }
            }
            assert!(below_s > 5.0, "smoke should pool under the ceiling (below={below_s})");
            assert!(ceil_s < 1e-3, "ceiling cells must hold no smoke (ceil={ceil_s})");
            assert!(
                above_s < below_s * 0.05,
                "smoke must not penetrate above the ceiling (above={above_s}, below={below_s})"
            );
        });
    }

    // THIN-WALL LOAD-BEARING TEST — the one that makes mechanisms (2)/(3) genuinely proven.
    // A wall THINNER than one cell (0.016 < cs.x 0.03125) with strong lateral fill, so the
    // semi-Lagrangian backtrace SPANS it. This exercises exactly the two fixes the audit
    // demanded: (A) conservative box↔cell voxelization — the old center-in-box test found NO
    // cell centre inside this wall and voxelized it to nothing (smoke crossed freely); and
    // (B) segment-marched no-tunnel — an endpoint-only backtrace overshoots the sub-cell wall
    // into the dense source side and pulls density across (tunnels). With both fixes the wall
    // is a watertight ≥1-cell layer and nothing crosses. (Reverting either fix fails this test.)
    #[test]
    fn thin_wall_blocks_voxelization_and_tunneling_leaks() {
        pollster::block_on(async {
            let Some((device, queue)) = setup_headless_gpu().await else {
                eprintln!("no GPU adapter — skipping thin-wall test");
                return;
            };
            let layout = scene_layout(&device);
            let mut smoke = SmokeVolume::new(&device, &layout, wgpu::TextureFormat::Rgba16Float);
            let n = smoke.grid_n() as usize; // 64 → cs.x = 2/64 = 0.03125
            smoke.bounds_min = [-1.0, 0.0, -1.0];
            smoke.bounds_max = [1.0, 2.0, 1.0];
            smoke.source = [-0.5, 1.0, 0.0];
            smoke.source_radius = 0.35;
            smoke.inject = 6.0;
            smoke.buoyancy = 0.2;
            smoke.curl_strength = 1.6;
            // Very strong lateral push so |vel*dt| ≈ 8 cells: a far-side cell's backtrace spans
            // the wall AND the no-penetration buffer cell in front of it, reaching the dense
            // region beyond. An endpoint-only guard then samples that dense cell (tunnels); the
            // segment march hits the wall first and stops. This is what isolates mechanism (3).
            smoke.fill_strength = 10.0;
            smoke.fill_radius = 2.5;
            smoke.dissipation = 0.99;
            // Sub-cell wall centered at x=0 (cells 31 & 32 under conservative rasterization).
            smoke.set_obstacle_boxes(&queue, &[([-0.008, 0.0, -1.0], [0.008, 2.0, 1.0])]);
            // dt at the sim clamp cap (1/30) so |vel*dt| clearly exceeds the wall thickness.
            for f in 0..60 {
                smoke.step(&device, &queue, f as f32 * (1.0 / 30.0), 1.0 / 30.0);
            }
            let d = read_f32(&device, &queue, smoke.density_buffer()).await;
            let xi = |x: f32| (((x + 1.0) / 2.0) * n as f32) as usize;
            // Cell boundaries land exactly on x = ±0.03125 → wall = i ∈ [31,33). Compare the 3
            // columns IMMEDIATELY left of the wall (near face, dense) with the 3 IMMEDIATELY
            // right (far face). With endpoint-only tunneling the backtrace (|vel*dt| ≈ 3 cells)
            // jumps the wall and pulls the dense near-face across, so far ≈ near; with the
            // segment march it stops at the wall and far ≈ 0. Comparing thin adjacent bands
            // (not far vs the whole near region) keeps a single tunnelled column detectable.
            let (near0, near1) = (xi(-0.25), xi(-0.03125)); // dense source-side band i ∈ [24,31)
            let (w0, w1) = (xi(-0.03125), xi(0.03125)); // wall i ∈ [31,33)
            let (far0, far1) = (xi(0.03125), xi(0.20)); // just past the wall i ∈ [33,38)
            let mut near_face = 0.0f64;
            let mut wall_cells = 0.0f64;
            let mut far_face = 0.0f64;
            for k in 0..n {
                for j in 0..n {
                    for i in 0..n {
                        let v = d[(k * n + j) * n + i] as f64;
                        if i >= near0 && i < near1 {
                            near_face += v;
                        } else if i >= w0 && i < w1 {
                            wall_cells += v;
                        } else if i >= far0 && i < far1 {
                            far_face += v;
                        }
                    }
                }
            }
            assert!(
                near_face > 5.0,
                "smoke should pile against the wall's near face (near_face={near_face})"
            );
            assert!(
                wall_cells < 1e-3,
                "the sub-cell wall's cells must be solid/empty (wall_cells={wall_cells})"
            );
            assert!(
                far_face < near_face * 0.1,
                "no smoke may tunnel/leak past a sub-cell wall (far_face={far_face}, near_face={near_face})"
            );
        });
    }

    // CONFORMING / FLOW-THROUGH: a wall with a DOORWAY gap. Smoke must flow THROUGH the gap to the
    // far side (proving it flows around/through geometry, not just stops dead everywhere), while
    // the far side directly behind the SOLID part stays blocked. Closes the "flow-around asserted
    // nowhere" gap — a regression that froze smoke at every face would fail the flow-through half.
    #[test]
    fn smoke_flows_through_a_doorway_but_not_the_solid_wall() {
        pollster::block_on(async {
            let Some((device, queue)) = setup_headless_gpu().await else {
                eprintln!("no GPU adapter — skipping doorway test");
                return;
            };
            let layout = scene_layout(&device);
            let mut smoke = SmokeVolume::new(&device, &layout, wgpu::TextureFormat::Rgba16Float);
            let n = smoke.grid_n() as usize;
            smoke.bounds_min = [-1.0, 0.0, -1.0];
            smoke.bounds_max = [1.0, 2.0, 1.0];
            smoke.source = [-0.55, 1.0, 0.0];
            smoke.source_radius = 0.35;
            smoke.inject = 6.0;
            smoke.buoyancy = 0.2;
            smoke.curl_strength = 1.6;
            smoke.fill_strength = 4.0;
            smoke.fill_radius = 2.2;
            smoke.dissipation = 0.99;
            // A wall at x ∈ [-0.05, 0.05] with a doorway gap in z ∈ [-0.25, 0.25]: two solid
            // panels z ∈ [-1,-0.25] and z ∈ [0.25,1].
            smoke.set_obstacle_boxes(
                &queue,
                &[
                    ([-0.05, 0.0, -1.0], [0.05, 2.0, -0.25]),
                    ([-0.05, 0.0, 0.25], [0.05, 2.0, 1.0]),
                ],
            );
            for f in 0..90 {
                smoke.step(&device, &queue, f as f32 * (1.0 / 30.0), 1.0 / 30.0);
            }
            let d = read_f32(&device, &queue, smoke.density_buffer()).await;
            let xi = |x: f32| (((x + 1.0) / 2.0) * n as f32) as usize;
            let zk = |z: f32| (((z + 1.0) / 2.0) * n as f32) as usize;
            let far = xi(0.15); // past the wall's far face (x > 0.1)
            // At the wall's IMMEDIATE far face, smoke exits only through the doorway lane — the
            // z-lanes behind the SOLID panels can only fill later by wrapping around from the far
            // side, so right at the face they are far emptier. (Measuring the whole far side would
            // include that legitimate wrap-around, which is NOT tunneling.)
            let face_hi = xi(0.28);
            let (gap0, gap1) = (zk(-0.2), zk(0.2)); // doorway lane in z
            let mut face_gap = 0.0f64; // at the far face, behind the doorway → should FILL
            let mut face_solid = 0.0f64; // at the far face, behind a solid panel → blocked here
            let mut far_gap = 0.0f64; // whole far side behind the doorway
            for k in 0..n {
                for j in 0..n {
                    for i in far..n {
                        let v = d[(k * n + j) * n + i] as f64;
                        let behind_gap = k >= gap0 && k < gap1;
                        let behind_solid = k < zk(-0.4) || k >= zk(0.4);
                        if behind_gap {
                            far_gap += v;
                        }
                        if i < face_hi {
                            if behind_gap {
                                face_gap += v;
                            } else if behind_solid {
                                face_solid += v;
                            }
                        }
                    }
                }
            }
            // Flow-through: smoke reaches the far side via the gap (a "frozen at every face"
            // regression, or a closed wall, gives ~0 here — cf. the solid-wall tests).
            assert!(
                far_gap > 1.0,
                "smoke must flow THROUGH the doorway to the far side (far_gap={far_gap})"
            );
            // And at the immediate far face it exits through the gap, not through the solid panels.
            assert!(
                face_gap > 5.0 * face_solid.max(1e-6),
                "at the wall face, smoke must emerge through the doorway, not the solid \
                 (face_gap={face_gap}, face_solid={face_solid})"
            );
        });
    }

    // FRAME-RATE INDEPENDENCE: the smoke's amount must not depend on the step size. Simulating
    // the same 3 seconds at 60 fps and at 240 fps must give a similar total density. Before
    // dissipation was made time-based it was applied per FRAME, so the 240 fps run dissipated
    // ~4x more and collapsed to a thin sliver near the source — exactly the "no smoke, just a
    // line" the demo showed at its (high) release frame rate while a 60 fps headless run filled
    // the volume. Reverting the shader to `* P.source.w` fails this test.
    #[test]
    fn smoke_density_is_frame_rate_independent() {
        async fn total_after_3s(
            device: &wgpu::Device,
            queue: &wgpu::Queue,
            layout: &wgpu::BindGroupLayout,
            dt: f32,
            steps: usize,
        ) -> f64 {
            let mut s = SmokeVolume::new(device, layout, wgpu::TextureFormat::Rgba16Float);
            s.bounds_min = [-1.5, 0.0, -1.5];
            s.bounds_max = [1.5, 3.0, 1.5];
            s.source = [0.0, 0.5, 0.0];
            s.source_radius = 0.4;
            s.inject = 6.0;
            s.dissipation = 0.985;
            s.buoyancy = 1.4;
            s.fill_strength = 1.5;
            s.fill_radius = 1.5;
            for f in 0..steps {
                s.step(device, queue, f as f32 * dt, dt);
            }
            read_f32(device, queue, s.density_buffer())
                .await
                .iter()
                .map(|&v| v as f64)
                .sum()
        }

        pollster::block_on(async {
            let Some((device, queue)) = setup_headless_gpu().await else {
                eprintln!("no GPU adapter — skipping frame-rate independence test");
                return;
            };
            let layout = scene_layout(&device);
            let a = total_after_3s(&device, &queue, &layout, 1.0 / 60.0, 180).await; // 3s @ 60fps
            let b = total_after_3s(&device, &queue, &layout, 1.0 / 240.0, 720).await; // 3s @ 240fps
            assert!(a > 100.0 && b > 100.0, "both runs should hold smoke (60fps={a}, 240fps={b})");
            let ratio = a.min(b) / a.max(b);
            assert!(
                ratio > 0.7,
                "smoke amount must be frame-rate independent (60fps={a}, 240fps={b}, ratio={ratio:.2})"
            );
        });
    }
}
