use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use wgpu::util::DeviceExt;

use super::pipeline::{create_physics_pipelines, PhysicsPipelines};
use super::types::*;

/// The renderer's own GPU rigid-body solver: bodies that are simulated, culled and drawn without
/// the CPU seeing them.
///
/// It is **not** the engine's physics — that is `gizmo-physics-rigid`, on the CPU and
/// deterministic. This one trades determinism and generality for scale, and the two must not run
/// over the same entities: enable it through
/// [`Renderer::enable_gpu_physics`](crate::Renderer::enable_gpu_physics), and only for a game that
/// is not using `PhysicsPlugin`.
pub struct GpuPhysicsSystem {
    /// The body buffer's capacity.
    pub max_boxes: u32,
    /// The broadphase grid's side length, in cells.
    pub grid_size: u32,
    /// The bodies — [`GpuBox`]es.
    pub boxes_buffer: wgpu::Buffer,
    /// The per-step [`PhysicsSimParams`](crate::gpu_physics::types::PhysicsSimParams).
    pub params_buffer: wgpu::Buffer,
    /// One head index per grid cell, into the linked-node list below. Grid + linked list, rather
    /// than a per-cell array, because a cell's occupancy is unbounded.
    pub grid_heads_buffer: wgpu::Buffer,
    /// The linked-list nodes those heads point into.
    pub linked_nodes_buffer: wgpu::Buffer,
    /// Per-body contact manifolds — see
    /// [`GpuBoxContacts`](crate::gpu_physics::types::GpuBoxContacts), whose stride is the one
    /// number in this file that has already been got wrong once.
    pub box_contacts_buffer: wgpu::Buffer,
    /// The static colliders.
    pub colliders_buffer: wgpu::Buffer,
    /// Which bodies are awake, written by the integrator and read by the next step's solver.
    pub awake_flags_buffer: wgpu::Buffer,
    /// The joints.
    pub joints_buffer: wgpu::Buffer,
    /// How many of them are live.
    pub joint_count: u32,
    /// The joint buffer's capacity.
    pub max_joints: u32,

    /// Every pipeline the solver runs.
    pub pipelines: PhysicsPipelines,

    /// The unit cube every body is drawn as.
    pub box_vertex_buffer: wgpu::Buffer,
    /// Its indices.
    pub box_index_buffer: wgpu::Buffer,
    /// How many of them.
    pub index_count: u32,

    /// A mappable copy of the body buffer, for the rare case where the CPU does need to see the
    /// simulation — see [`request_readback`](Self::request_readback).
    pub readback_buffer: wgpu::Buffer,
    // 0 = Idle, 1 = Copied to buffer (awaiting map), 2 = Mapping, 3 = Mapped (ready to read)
    /// Where that readback is: 0 = idle, 1 = copied and awaiting a map, 2 = mapping, 3 = mapped
    /// and readable.
    ///
    /// A state machine rather than a blocking read because mapping a buffer takes at least a frame:
    /// waiting on it would stall the pipeline that the whole system exists to keep full.
    pub readback_state: Arc<AtomicU8>,

    /// The indirect draw arguments the culling pass writes — the instance count never reaches the
    /// CPU.
    pub indirect_buffer: wgpu::Buffer,
    /// The visible subset the culling pass compacts into, and the draw reads.
    pub culled_boxes_buffer: wgpu::Buffer,

    // ═══ Debug Renderer ═══
    /// Whether the debug overlay is generated this frame.
    pub debug_enabled: bool,
    /// The lines the debug compute pass emits.
    pub debug_line_buffer: wgpu::Buffer,
    /// How many it emitted, doubling as the indirect draw count — again, the CPU never learns the
    /// number.
    pub debug_line_count_buffer: wgpu::Buffer,
    /// What the overlay should draw — see
    /// [`DebugParams`](crate::gpu_physics::types::DebugParams).
    pub debug_params_buffer: wgpu::Buffer,
    /// The overlay's compute bind group.
    pub debug_compute_bind_group: wgpu::BindGroup,
    /// The pass that turns bodies and joints into line vertices.
    pub debug_compute_pipeline: wgpu::ComputePipeline,
    /// The pass that draws them.
    pub debug_render_pipeline: wgpu::RenderPipeline,
    /// The line buffer's capacity.
    pub debug_max_lines: u32,
}

impl GpuPhysicsSystem {
    /// Allocates every simulation buffer for `max_boxes` bodies and builds every pipeline.
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

        // Joint buffer — max 16384 joints
        let max_joints = 16384u32;
        let joints_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GPU Physics Joints Buffer"),
            size: (max_joints as wgpu::BufferAddress)
                * std::mem::size_of::<GpuJoint>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // 352 bytes per box under std430 (see `GpuBoxContacts` for the offset
        // walk). Driven by `size_of` so the CPU allocation and the shader's
        // std430 element stride can never drift apart again.
        let box_contacts_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GPU Physics Box Contacts Cache"),
            size: (max_boxes as wgpu::BufferAddress)
                * std::mem::size_of::<GpuBoxContacts>() as wgpu::BufferAddress,
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
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::VERTEX
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            debug_line_count_buffer: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Debug Line Count"),
                contents: bytemuck::cast_slice(&[0u32, 1u32, 0u32, 0u32]), // IndirectDrawArgs: vertex_count, instance_count, first_vertex, first_instance
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::INDIRECT
                    | wgpu::BufferUsages::COPY_DST,
            }),
            debug_params_buffer: {
                let dp = DebugParams {
                    num_boxes: max_boxes,
                    num_joints: 0,
                    show_wireframes: 0,
                    _pad: 0,
                };
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
                    source: wgpu::ShaderSource::Wgsl(
                        include_str!("../shaders/physics_debug.wgsl").into(),
                    ),
                });
                let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Debug Compute Layout"),
                    bind_group_layouts: &[Some(&device.create_bind_group_layout(
                        &wgpu::BindGroupLayoutDescriptor {
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
                                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                                        has_dynamic_offset: false,
                                        min_binding_size: None,
                                    },
                                    count: None,
                                },
                                wgpu::BindGroupLayoutEntry {
                                    binding: 3,
                                    visibility: wgpu::ShaderStages::COMPUTE,
                                    ty: wgpu::BindingType::Buffer {
                                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                                        has_dynamic_offset: false,
                                        min_binding_size: None,
                                    },
                                    count: None,
                                },
                                wgpu::BindGroupLayoutEntry {
                                    binding: 4,
                                    visibility: wgpu::ShaderStages::COMPUTE,
                                    ty: wgpu::BindingType::Buffer {
                                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                                        has_dynamic_offset: false,
                                        min_binding_size: None,
                                    },
                                    count: None,
                                },
                            ],
                            label: Some("debug_compute_layout_inner"),
                        },
                    ))],
                    immediate_size: 0,
                });
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("Physics Debug Compute"),
                    layout: Some(&layout),
                    module: &shader,
                    entry_point: Some("generate_debug_lines"),
                    compilation_options: Default::default(),
                    cache: None,
                })
            },
            debug_render_pipeline: {
                let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("Physics Debug Shader"),
                    source: wgpu::ShaderSource::Wgsl(
                        include_str!("../shaders/physics_debug.wgsl").into(),
                    ),
                });
                let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Debug Render Layout"),
                    bind_group_layouts: &[Some(global_bind_group_layout)],
                    immediate_size: 0,
                });
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("Physics Debug Lines"),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs_debug"),
                        compilation_options: Default::default(),
                        buffers: &[Some(DebugVertex::desc())],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("fs_debug"),
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
                        depth_write_enabled: Some(false),
                        depth_compare: Some(wgpu::CompareFunction::LessEqual),
                        stencil: wgpu::StencilState::default(),
                        bias: wgpu::DepthBiasState::default(),
                    }),
                    multisample: wgpu::MultisampleState::default(),
                    multiview_mask: None,
            cache: None,
                })
            },
            debug_max_lines: 32768,
        }
    }

    /// Overwrites one body. Seeding the simulation, or teleporting a body from the CPU.
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

    /// Overwrites one static collider.
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

    /// Adds a joint and returns its index.
    pub fn add_joint(&mut self, queue: &wgpu::Queue, joint: GpuJoint) -> Option<u32> {
        if self.joint_count >= self.max_joints {
            return None;
        }
        let idx = self.joint_count;
        let offset =
            (idx as wgpu::BufferAddress) * std::mem::size_of::<GpuJoint>() as wgpu::BufferAddress;
        queue.write_buffer(&self.joints_buffer, offset, bytemuck::cast_slice(&[joint]));
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
            queue.write_buffer(&self.joints_buffer, offset, bytemuck::cast_slice(&[empty]));
        }
    }

    /// Updates the simulation parameters (dt, num_joints and the rest).
    pub fn update_params(&self, queue: &wgpu::Queue, dt: f32, gravity: [f32; 3]) {
        let params = PhysicsSimParams {
            dt,
            _pad0: [0; 3],
            _pad1: [0.0; 3],
            _pad1b: 0,
            gravity,
            damping: 0.99,
            num_boxes: self.max_boxes,
            num_colliders: 100, // max static colliders
            num_joints: self.joint_count,
            _pad2: 0,
        };
        queue.write_buffer(&self.params_buffer, 0, bytemuck::cast_slice(&[params]));
    }

    /// Enables the debug visualisation, building the bind group from the real buffer
    /// references.
    pub fn enable_debug(&mut self, device: &wgpu::Device, _show_flags: u32) {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
            label: Some("debug_compute_layout"),
        });

        self.debug_compute_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.debug_params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.boxes_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.joints_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.debug_line_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.debug_line_count_buffer.as_entire_binding(),
                },
            ],
            label: Some("debug_compute_bind_group"),
        });

        self.debug_enabled = true;
    }

    /// Turns the debug visualisation on or off.
    pub fn toggle_debug(&mut self) {
        self.debug_enabled = !self.debug_enabled;
    }

    /// Updates the debug flags (show_wireframes).
    pub fn set_debug_flags(&self, queue: &wgpu::Queue, show_wireframes: u32) {
        let dp = DebugParams {
            num_boxes: self.max_boxes,
            num_joints: self.joint_count,
            show_wireframes,
            _pad: 0,
        };
        queue.write_buffer(&self.debug_params_buffer, 0, bytemuck::cast_slice(&[dp]));
    }

    #[tracing::instrument(skip_all, level = "trace")]
    /// Records one whole simulation step: clear, build, narrowphase, solve, joints, integrate —
    /// each its own dispatch, in that order.
    pub fn compute_pass(&self, encoder: &mut wgpu::CommandEncoder) {
        tracing::trace!(
            boxes = self.max_boxes,
            joints = self.joint_count,
            si_iterations = 6,
            "[GpuPhysics] recording sequential-impulse solver passes"
        );
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

    #[tracing::instrument(skip_all, level = "trace")]
    /// Records the frustum-culling dispatch, which compacts the visible bodies and writes the
    /// indirect draw arguments.
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

    #[tracing::instrument(skip_all, level = "trace")]
    /// Records the instanced box draw, indirect off the culling pass's count.
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

    /// Records the dispatch that generates the debug overlay's lines.
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
        cpass.dispatch_workgroups(
            self.max_boxes
                .div_ceil(256)
                .max(self.max_joints.div_ceil(256)),
            1,
            1,
        );
    }

    /// Records the debug overlay's draw.
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

    /// Asks for a copy of the body buffer, if no readback is already in flight.
    ///
    /// Idempotent: a second call while one is outstanding does nothing, so it is safe to call every
    /// frame. Pair with [`poll_readback_data`](Self::poll_readback_data), which is what eventually
    /// returns the data.
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

    /// Advances the readback state machine, returning the bodies once they are mapped.
    ///
    /// `None` means "not ready", not "nothing there" — a readback takes at least a frame to arrive,
    /// so this returns `None` several times before it returns the data once.
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

        let _ = device.poll(wgpu::PollType::Poll);

        if self.readback_state.load(Ordering::SeqCst) == 3 {
            let slice = self.readback_buffer.slice(..);
            let view = slice.get_mapped_range()
                // wgpu 30 made this fallible; the range is the whole buffer we just mapped, so a
                // failure here is a programming error rather than a runtime condition.
                .expect("a just-mapped buffer's full range is always valid");

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
                color: [1.0, 1.0, 1.0, 1.0],
                normal: *normal,
                tex_coords: [0.0, 0.0],
                joint_indices: [0; 4],
                joint_weights: [0.0; 4],
                ..Default::default()
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base + 2, base + 3, base]);
    }

    (vertices, indices)
}
