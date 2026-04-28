use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use wgpu::util::DeviceExt;

use super::pipeline::{create_physics_pipelines, PhysicsPipelines};
use super::types::*;

pub struct GpuPhysicsSystem {
    pub max_boxes: u32,
    pub grid_size: u32,
    pub boxes_buffer: wgpu::Buffer,
    pub params_buffer: wgpu::Buffer,
    pub grid_heads_buffer: wgpu::Buffer,
    pub linked_nodes_buffer: wgpu::Buffer,
    pub box_contacts_buffer: wgpu::Buffer,
    pub colliders_buffer: wgpu::Buffer,
    pub awake_flags_buffer: wgpu::Buffer,
    pub joints_buffer: wgpu::Buffer,
    pub joint_count: u32,
    pub max_joints: u32,

    pub pipelines: PhysicsPipelines,

    pub box_vertex_buffer: wgpu::Buffer,
    pub box_index_buffer: wgpu::Buffer,
    pub index_count: u32,

    pub readback_buffer: wgpu::Buffer,
    // 0 = Idle, 1 = Copied to buffer (awaiting map), 2 = Mapping, 3 = Mapped (ready to read)
    pub readback_state: Arc<AtomicU8>,

    pub indirect_buffer: wgpu::Buffer,
    pub culled_boxes_buffer: wgpu::Buffer,

    // ═══ Debug Renderer ═══
    pub debug_enabled: bool,
    pub debug_line_buffer: wgpu::Buffer,
    pub debug_line_count_buffer: wgpu::Buffer,
    pub debug_params_buffer: wgpu::Buffer,
    pub debug_compute_bind_group: wgpu::BindGroup,
    pub debug_compute_pipeline: wgpu::ComputePipeline,
    pub debug_render_pipeline: wgpu::RenderPipeline,
    pub debug_max_lines: u32,
}

impl GpuPhysicsSystem {
    pub fn new(
        device: &wgpu::Device,
        max_boxes: u32,
        global_bind_group_layout: &wgpu::BindGroupLayout,
        output_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) -> Self {
        let mut initial_boxes = Vec::with_capacity(max_boxes as usize);
        let grid_dim = (max_boxes as f32).powf(1.0 / 3.0).ceil() as u32;
        let spacing = 2.1f32;
        let offset = (grid_dim as f32 * spacing) / 2.0;

        for i in 0..max_boxes {
            let ix = i % grid_dim;
            let iy = (i / grid_dim) % grid_dim;
            let iz = i / (grid_dim * grid_dim);

            let x = (ix as f32 * spacing) - offset;
            let y = 30.0 + (iy as f32 * spacing); // Y=30'dan yukarı doğru diz
            let z = (iz as f32 * spacing) - offset;

            // Görselliği arttırmak için Y koordinatına göre renk gradyanı:
            let color_r = ix as f32 / grid_dim as f32;
            let color_g = iy as f32 / grid_dim as f32;
            let color_b = iz as f32 / grid_dim as f32;

            initial_boxes.push(GpuBox {
                position: [x, y, z],
                mass: 1.0,
                velocity: [0.0, 0.0, 0.0],
                state: 0,
                rotation: [0.0, 0.0, 0.0, 1.0],
                angular_velocity: [0.0, 0.0, 0.0],
                sleep_counter: 0,
                color: [color_r, color_g, color_b, 1.0],
                half_extents: [1.0, 1.0, 1.0],
                _pad: 0,
            });
        }

        let boxes_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("GPU Physics Buffer"),
            contents: bytemuck::cast_slice(&initial_boxes),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
        });

        let mut initial_colliders = Vec::new();
        // 1. Zemin (Sonsuz Plane) -> Y = 0
        initial_colliders.push(GpuCollider {
            shape_type: 1,
            _pad1: [0; 3],
            data1: [0.0, 1.0, 0.0, 0.0], // Normal vec
            data2: [0.0, 0.0, 0.0, 0.0], // distance = 0
        });

        // 2. Ortadaki Devasa Zemin Platformu (AABB)
        initial_colliders.push(GpuCollider {
            shape_type: 0,
            _pad1: [0; 3],
            data1: [-40.0, 0.0, -40.0, 0.0], // aabb_min
            data2: [40.0, 20.0, 40.0, 0.0],  // aabb_max
        });

        // 3. Eğik bir rampa veya duvar
        initial_colliders.push(GpuCollider {
            shape_type: 0,
            _pad1: [0; 3],
            data1: [45.0, 0.0, -40.0, 0.0], // aabb_min
            data2: [55.0, 40.0, 40.0, 0.0], // aabb_max (Sağ Duvar)
        });

        let max_static_colliders = 100;
        let num_initial = initial_colliders.len();
        if num_initial < max_static_colliders {
            let empty_col = GpuCollider {
                shape_type: 0,
                _pad1: [0; 3],
                data1: [0.0; 4],
                data2: [0.0; 4],
            };
            initial_colliders.resize(max_static_colliders, empty_col);
        }

        let colliders_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("GPU Static Colliders Buffer"),
            contents: bytemuck::cast_slice(&initial_colliders),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        let initial_awake_flags: Vec<u32> = vec![0; max_boxes as usize];
        let awake_flags_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("GPU Physics Awake Flags Buffer"),
            contents: bytemuck::cast_slice(&initial_awake_flags),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        let params = PhysicsSimParams {
            dt: 0.016,
            _pad0: [0; 3],
            _pad1: [0.0; 3],
            _pad1b: 0,
            gravity: [0.0, -9.81, 0.0],
            damping: 0.99,
            num_boxes: max_boxes,
            num_colliders: initial_colliders.len() as u32,
            num_joints: 0,
            _pad2: 0,
        };

        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("GPU Physics Params Buffer"),
            contents: bytemuck::cast_slice(&[params]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let grid_size = 262144u32;
        let initial_heads = vec![-1i32; grid_size as usize];
        let grid_heads_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("GPU Physics Grid Heads Buffer"),
            contents: bytemuck::cast_slice(&initial_heads),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        let initial_nodes = vec![-1i32; max_boxes as usize];
        let linked_nodes_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("GPU Physics Linked Nodes Buffer"),
            contents: bytemuck::cast_slice(&initial_nodes),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        let (vertices, indices) = create_cube();

        let box_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Box Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let box_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Box Index Buffer"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let indirect_data: [u32; 5] = [
            indices.len() as u32, // vertex_count
            0,                    // instance_count
            0,                    // first_index
            0,                    // base_vertex
            0,                    // first_instance
        ];

        let indirect_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Culling Indirect Buffer"),
            contents: bytemuck::cast_slice(&indirect_data),
            usage: wgpu::BufferUsages::INDIRECT
                | wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST,
        });

        let culled_boxes_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Culled Boxes Buffer"),
            size: (max_boxes as wgpu::BufferAddress)
                * std::mem::size_of::<GpuBox>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::VERTEX,
            mapped_at_creation: false,
        });

        // Joint buffer — max 4096 joints
        let max_joints = 4096u32;
        let joints_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GPU Physics Joints Buffer"),
            size: (max_joints as wgpu::BufferAddress)
                * std::mem::size_of::<GpuJoint>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // 336 bytes per box (4 count, 12 pad, 32 neighbors, 128 normals, 128 accum_impulse, 32 is_active)
        let box_contacts_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GPU Physics Box Contacts Cache"),
            size: (max_boxes as wgpu::BufferAddress) * 336,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let pipelines = create_physics_pipelines(
            device,
            global_bind_group_layout,
            output_format,
            depth_format,
            &params_buffer,
            &boxes_buffer,
            &grid_heads_buffer,
            &linked_nodes_buffer,
            &colliders_buffer,
            &awake_flags_buffer,
            &joints_buffer,
            &box_contacts_buffer,
            &culled_boxes_buffer,
            &indirect_buffer,
        );

        Self {
            max_boxes,
            grid_size,
            boxes_buffer,
            params_buffer,
            grid_heads_buffer,
            linked_nodes_buffer,
            box_contacts_buffer,
            colliders_buffer,
            awake_flags_buffer,
            joints_buffer,
            joint_count: 0,
            max_joints,
            pipelines,
            box_vertex_buffer,
            box_index_buffer,
            index_count: indices.len() as u32,

            readback_buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("GPU Physics Readback Buffer"),
                size: (max_boxes as wgpu::BufferAddress)
                    * std::mem::size_of::<GpuBox>() as wgpu::BufferAddress,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            readback_state: Arc::new(AtomicU8::new(0)),

            indirect_buffer,
            culled_boxes_buffer,

            // Debug Renderer — bind group ve pipeline enable_debug() ile oluşturulur
            debug_enabled: false,
            debug_line_buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Debug Line Buffer"),
                size: 32768 * 2 * std::mem::size_of::<DebugVertex>() as wgpu::BufferAddress,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            debug_line_count_buffer: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Debug Line Count"),
                contents: bytemuck::cast_slice(&[0u32, 1u32, 0u32, 0u32]), // IndirectDrawArgs: vertex_count, instance_count, first_vertex, first_instance
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
            }),
            debug_params_buffer: {
                let dp = DebugParams { num_boxes: max_boxes, num_joints: 0, show_wireframes: 0, _pad: 0 };
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Debug Params"),
                    contents: bytemuck::cast_slice(&[dp]),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                })
            },
            // Dummy — enable_debug() ile yeniden oluşturulur
            debug_compute_bind_group: device.create_bind_group(&wgpu::BindGroupDescriptor {
                layout: &device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    entries: &[],
                    label: Some("empty_layout"),
                }),
                entries: &[],
                label: Some("debug_placeholder"),
            }),
            debug_compute_pipeline: {
                let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("Physics Debug Compute Shader"),
                    source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/physics_debug.wgsl").into()),
                });
                let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Debug Compute Layout"),
                    bind_group_layouts: &[
                        &device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                            entries: &[
                                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                                wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                                wgpu::BindGroupLayoutEntry { binding: 4, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                            ],
                            label: Some("debug_compute_layout_inner"),
                        }),
                    ],
                    push_constant_ranges: &[],
                });
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("Physics Debug Compute"),
                    layout: Some(&layout),
                    module: &shader,
                    entry_point: "generate_debug_lines",
                    compilation_options: Default::default(),
                })
            },
            debug_render_pipeline: {
                let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("Physics Debug Shader"),
                    source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/physics_debug.wgsl").into()),
                });
                let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Debug Render Layout"),
                    bind_group_layouts: &[global_bind_group_layout],
                    push_constant_ranges: &[],
                });
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("Physics Debug Lines"),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: "vs_debug",
                        compilation_options: Default::default(),
                        buffers: &[DebugVertex::desc()],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: "fs_debug",
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: output_format,
                            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::LineList,
                        ..Default::default()
                    },
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: depth_format,
                        depth_write_enabled: false,
                        depth_compare: wgpu::CompareFunction::LessEqual,
                        stencil: wgpu::StencilState::default(),
                        bias: wgpu::DepthBiasState::default(),
                    }),
                    multisample: wgpu::MultisampleState::default(),
                    multiview: None,
                })
            },
            debug_max_lines: 32768,
        }
    }

    pub fn update_box(&self, queue: &wgpu::Queue, index: u32, box_struct: &GpuBox) {
        if index < self.max_boxes {
            let offset = (index as wgpu::BufferAddress)
                * std::mem::size_of::<GpuBox>() as wgpu::BufferAddress;
            queue.write_buffer(
                &self.boxes_buffer,
                offset,
                bytemuck::cast_slice(&[*box_struct]),
            );
        }
    }

    pub fn update_collider(&self, queue: &wgpu::Queue, index: u32, collider: &GpuCollider) {
        if index < 100 {
            let offset = (index as wgpu::BufferAddress)
                * std::mem::size_of::<GpuCollider>() as wgpu::BufferAddress;
            queue.write_buffer(
                &self.colliders_buffer,
                offset,
                bytemuck::cast_slice(&[*collider]),
            );
        }
    }

    /// Joint ekle — indeksini döndürür.
    pub fn add_joint(&mut self, queue: &wgpu::Queue, joint: GpuJoint) -> Option<u32> {
        if self.joint_count >= self.max_joints {
            return None;
        }
        let idx = self.joint_count;
        let offset = (idx as wgpu::BufferAddress)
            * std::mem::size_of::<GpuJoint>() as wgpu::BufferAddress;
        queue.write_buffer(
            &self.joints_buffer,
            offset,
            bytemuck::cast_slice(&[joint]),
        );
        self.joint_count += 1;
        Some(idx)
    }

    /// Joint'i deaktive et.
    pub fn remove_joint(&self, queue: &wgpu::Queue, index: u32) {
        if index < self.joint_count {
            let mut empty = GpuJoint::ball(0, 0, [0.0; 3], [0.0; 3]);
            empty.flags = 0; // inactive
            let offset = (index as wgpu::BufferAddress)
                * std::mem::size_of::<GpuJoint>() as wgpu::BufferAddress;
            queue.write_buffer(
                &self.joints_buffer,
                offset,
                bytemuck::cast_slice(&[empty]),
            );
        }
    }

    /// Simülasyon parametrelerini güncelle (dt, num_joints, vb.)
    pub fn update_params(&self, queue: &wgpu::Queue, dt: f32) {
        let params = PhysicsSimParams {
            dt,
            _pad0: [0; 3],
            _pad1: [0.0; 3],
            _pad1b: 0,
            gravity: [0.0, -9.81, 0.0],
            damping: 0.99,
            num_boxes: self.max_boxes,
            num_colliders: 100, // max static colliders
            num_joints: self.joint_count,
            _pad2: 0,
        };
        queue.write_buffer(
            &self.params_buffer,
            0,
            bytemuck::cast_slice(&[params]),
        );
    }

    /// Debug görselleştirmeyi etkinleştir. Bind group'u gerçek buffer referanslarıyla oluşturur.
    pub fn enable_debug(&mut self, device: &wgpu::Device, _show_flags: u32) {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 4, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
            ],
            label: Some("debug_compute_layout"),
        });

        self.debug_compute_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.debug_params_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: self.boxes_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: self.joints_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: self.debug_line_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: self.debug_line_count_buffer.as_entire_binding() },
            ],
            label: Some("debug_compute_bind_group"),
        });

        self.debug_enabled = true;
    }

    /// Debug'u aç/kapat.
    pub fn toggle_debug(&mut self) {
        self.debug_enabled = !self.debug_enabled;
    }

    pub fn compute_pass(&self, encoder: &mut wgpu::CommandEncoder) {
        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Physics Compute Pass"),
            timestamp_writes: None,
        });
        cpass.set_bind_group(0, &self.pipelines.compute_bind_group, &[]);

        // ═══ Sequential Impulse Solver ═══
        // Faz 1: Grid'i bir kez inşa et
        cpass.set_pipeline(&self.pipelines.pipeline_clear);
        cpass.dispatch_workgroups(self.grid_size.div_ceil(256), 1, 1);

        cpass.set_pipeline(&self.pipelines.pipeline_build);
        cpass.dispatch_workgroups(self.max_boxes.div_ceil(256), 1, 1);

        // Faz 2: Narrowphase (Çarpışma Tespiti ve Contact Caching)
        cpass.set_pipeline(&self.pipelines.pipeline_narrowphase);
        cpass.dispatch_workgroups(self.max_boxes.div_ceil(256), 1, 1);

        // Faz 3: Çarpışma çözümünü N kez tekrarla (SI iterasyon)
        // Artık grid üzerinden değil, doğrudan contact cache üzerinden hesaplama yapıyor!
        let si_iterations = 6;
        for _ in 0..si_iterations {
            cpass.set_pipeline(&self.pipelines.pipeline_solve);
            cpass.dispatch_workgroups(self.max_boxes.div_ceil(256), 1, 1);
        }

        // Faz 4: Hız ve pozisyon entegrasyonu (tek seferde)
        cpass.set_pipeline(&self.pipelines.pipeline_integrate);
        cpass.dispatch_workgroups(self.max_boxes.div_ceil(256), 1, 1);

        // Faz 4: Joint constraint çözümü (entegrasyondan sonra)
        if self.joint_count > 0 {
            let joint_iterations = 4;
            for _ in 0..joint_iterations {
                cpass.set_pipeline(&self.pipelines.pipeline_solve_joints);
                cpass.dispatch_workgroups(self.max_boxes.div_ceil(256), 1, 1);
            }
        }
    }

    pub fn cull_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        global_bind_group: &wgpu::BindGroup,
    ) {
        encoder.clear_buffer(&self.indirect_buffer, 4, Some(4));

        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Physics Culling Pass"),
            timestamp_writes: None,
        });
        cpass.set_pipeline(&self.pipelines.pipeline_culling);
        cpass.set_bind_group(0, global_bind_group, &[]);
        cpass.set_bind_group(1, &self.pipelines.culling_bind_group, &[]);
        cpass.dispatch_workgroups(self.max_boxes.div_ceil(256), 1, 1);
    }

    pub fn render_pass<'a>(
        &'a self,
        rpass: &mut wgpu::RenderPass<'a>,
        global_bind_group: &'a wgpu::BindGroup,
    ) {
        rpass.set_pipeline(&self.pipelines.render_pipeline);
        rpass.set_bind_group(0, global_bind_group, &[]);
        rpass.set_vertex_buffer(0, self.box_vertex_buffer.slice(..));
        rpass.set_vertex_buffer(1, self.culled_boxes_buffer.slice(..));
        rpass.set_index_buffer(self.box_index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        rpass.draw_indexed_indirect(&self.indirect_buffer, 0);
    }

    pub fn debug_compute_pass(&self, encoder: &mut wgpu::CommandEncoder) {
        if !self.debug_enabled {
            return;
        }

        // Clear line count to 0 (4 bytes)
        encoder.clear_buffer(&self.debug_line_count_buffer, 0, Some(4));

        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Physics Debug Compute Pass"),
            timestamp_writes: None,
        });

        cpass.set_pipeline(&self.debug_compute_pipeline);
        cpass.set_bind_group(0, &self.debug_compute_bind_group, &[]);
        // Dispatch enough workgroups for all boxes
        cpass.dispatch_workgroups(self.max_boxes.div_ceil(256).max(self.max_joints.div_ceil(256)), 1, 1);
    }

    pub fn debug_render_pass<'a>(
        &'a self,
        rpass: &mut wgpu::RenderPass<'a>,
        global_bind_group: &'a wgpu::BindGroup,
    ) {
        if !self.debug_enabled {
            return;
        }

        rpass.set_pipeline(&self.debug_render_pipeline);
        rpass.set_bind_group(0, global_bind_group, &[]);
        rpass.set_vertex_buffer(0, self.debug_line_buffer.slice(..));
        rpass.draw_indirect(&self.debug_line_count_buffer, 0);
    }

    pub fn request_readback(&self, encoder: &mut wgpu::CommandEncoder) {
        if self
            .readback_state
            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let size = (self.max_boxes as wgpu::BufferAddress)
                * std::mem::size_of::<GpuBox>() as wgpu::BufferAddress;
            encoder.copy_buffer_to_buffer(&self.boxes_buffer, 0, &self.readback_buffer, 0, size);
        }
    }

    pub fn poll_readback_data(&self, device: &wgpu::Device) -> Option<Vec<GpuBox>> {
        if self
            .readback_state
            .compare_exchange(1, 2, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let slice = self.readback_buffer.slice(..);
            let state_clone = self.readback_state.clone();
            slice.map_async(wgpu::MapMode::Read, move |result| {
                if result.is_ok() {
                    state_clone.store(3, Ordering::SeqCst);
                } else {
                    state_clone.store(0, Ordering::SeqCst);
                }
            });
        }

        device.poll(wgpu::Maintain::Poll);

        if self.readback_state.load(Ordering::SeqCst) == 3 {
            let slice = self.readback_buffer.slice(..);
            let view = slice.get_mapped_range();

            let data: &[GpuBox] = bytemuck::cast_slice(&view);
            let vec_data = data.to_vec();

            drop(view);
            self.readback_buffer.unmap();

            self.readback_state.store(0, Ordering::SeqCst);

            return Some(vec_data);
        }
        None
    }
}

fn create_cube() -> (Vec<crate::gpu_types::Vertex>, Vec<u32>) {
    let s = 1.0f32;
    let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
        (
            [0.0, 0.0, 1.0],
            [[-s, -s, s], [s, -s, s], [s, s, s], [-s, s, s]],
        ),
        (
            [0.0, 0.0, -1.0],
            [[s, -s, -s], [-s, -s, -s], [-s, s, -s], [s, s, -s]],
        ),
        (
            [1.0, 0.0, 0.0],
            [[s, -s, s], [s, -s, -s], [s, s, -s], [s, s, s]],
        ),
        (
            [-1.0, 0.0, 0.0],
            [[-s, -s, -s], [-s, -s, s], [-s, s, s], [-s, s, -s]],
        ),
        (
            [0.0, 1.0, 0.0],
            [[-s, s, s], [s, s, s], [s, s, -s], [-s, s, -s]],
        ),
        (
            [0.0, -1.0, 0.0],
            [[-s, -s, -s], [s, -s, -s], [s, -s, s], [-s, -s, s]],
        ),
    ];

    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);

    for (normal, corners) in &faces {
        let base = vertices.len() as u32;
        for &p in corners {
            vertices.push(crate::gpu_types::Vertex {
                position: p,
                color: [1.0, 1.0, 1.0],
                normal: *normal,
                tex_coords: [0.0, 0.0],
                joint_indices: [0; 4],
                joint_weights: [0.0; 4],
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base + 2, base + 3, base]);
    }

    (vertices, indices)
}
