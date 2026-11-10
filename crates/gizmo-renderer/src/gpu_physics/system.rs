use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use wgpu::util::DeviceExt;

use super::pipeline::{create_physics_pipelines, PhysicsPipelines};
use super::types::*;

/// How many static colliders the collider buffer holds.
pub const MAX_STATIC_COLLIDERS: u32 = 100;

/// Who holds a slot of the box or the static-collider table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    /// Nobody: free to give out.
    Free,
    /// Put in by the game directly; freed only when it asks.
    Live,
    /// Put in for this entity; freed as well once the entity no longer claims it.
    Owned(gizmo_core::Entity),
    /// Its body was taken out and has not yet been through a step (`BOX_LEAVING`): held by nobody,
    /// not given out yet.
    Leaving,
}

/// `body` as a live one: a body put in or written is never free or leaving — that is
/// `remove_box`'s.
fn live(mut body: GpuBox) -> GpuBox {
    if body.state >= BOX_FREE {
        body.state = 0;
    }
    body
}

/// A full table, logged once until a slot of it is freed.
fn report_full(table: &str, capacity: u32) {
    tracing::error!(table, capacity, "[GpuPhysics] the table is full; what does not fit is left out until a slot is freed");
}

/// The solver's parameters, from the step, gravity and the live counts.
fn sim_params(dt: f32, gravity: [f32; 3], num_boxes: u32, num_colliders: u32, num_joints: u32) -> PhysicsSimParams {
    PhysicsSimParams {
        dt,
        _pad0: [0; 3],
        _pad1: [0.0; 3],
        _pad1b: 0,
        gravity,
        damping: 0.99,
        num_boxes,
        num_colliders,
        num_joints,
        _pad2: 0,
    }
}

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
    /// The per-step [`PhysicsSimParams`].
    pub params_buffer: wgpu::Buffer,
    /// One head index per grid cell, into the linked-node list below. Grid + linked list, rather
    /// than a per-cell array, because a cell's occupancy is unbounded.
    pub grid_heads_buffer: wgpu::Buffer,
    /// The linked-list nodes those heads point into.
    pub linked_nodes_buffer: wgpu::Buffer,
    /// Per-body contact manifolds — see
    /// [`GpuBoxContacts`], whose stride is the one
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

    /// The simulation's own fixed-step clock: how many steps the game's time owes the bodies —
    /// see [`advance`](Self::advance).
    clock: gizmo_core::time::PhysicsTime,
    /// Gravity, m/s², as [`update_params`](Self::update_params) last set it.
    gravity: [f32; 3],
    /// One past the highest static-collider slot in use: the solver checks this many, and skips
    /// the free ones among them ([`GpuCollider::NONE`]). The buffer holds [`MAX_STATIC_COLLIDERS`].
    collider_count: u32,
    /// Who holds each static-collider slot.
    collider_slots: Vec<Slot>,
    /// Freed collider slots below `collider_count`, reused first.
    free_colliders: Vec<u32>,
    /// One past the highest box slot in use: every pass runs over this many and skips the free
    /// ones among them ([`BOX_FREE`]).
    box_high_water: u32,
    /// Who holds each box slot.
    box_slots: Vec<Slot>,
    /// Freed box slots below `box_high_water`, reused first.
    free_boxes: Vec<u32>,
    /// Box slots whose body was taken out and has not been through a step yet: not given out until
    /// one has run, because that step is what wakes the bodies round it and forgets its contacts
    /// (`BOX_LEAVING`).
    leaving: Vec<u32>,
    /// Box slots whose leaving body is in the steps last recorded: free from the next `advance` on.
    left: Vec<u32>,
    /// Whether a full box table has been reported since a box slot was last freed, so a game that
    /// keeps asking is told once.
    boxes_full_reported: bool,
    /// The same for the static-collider table.
    colliders_full_reported: bool,
    /// How many readback copies have been recorded ([`request_readback`](Self::request_readback)).
    copies_recorded: std::sync::atomic::AtomicU64,
    /// Which copy the readback in flight is of.
    copy_in_flight: std::sync::atomic::AtomicU64,
    /// Which copy the data [`poll_readback_data`](Self::poll_readback_data) last returned is of.
    copy_returned: std::sync::atomic::AtomicU64,
    /// For each box slot, the first copy that holds the body in it now — see
    /// [`readback_holds`](Self::readback_holds).
    first_copy: Vec<u64>,
    /// What the solver was last given — see [`params`](Self::params).
    params: PhysicsSimParams,

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
    /// [`DebugParams`].
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
        // **Empty: every slot free.** The system is capacity, not a scene. It used to start with all
        // `max_boxes` slots live — a grid of 50 000 one-kilo cubes falling from 30 m — and three
        // static colliders (the ground plane, an 80 × 20 × 80 m platform round the origin and a wall
        // beside it): the `1_milyon_kup` demo's scene, simulated and drawn in every game that turned
        // GPU bodies on, whatever it put in itself (`kumas` hung its cloth inside the falling field).
        // A game puts in what it wants (`add_box`, `add_boxes`, `add_collider`, or entities with a
        // rigid body through the engine's submit path).
        let initial_boxes = vec![GpuBox::FREE; max_boxes as usize];

        let boxes_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("GPU Physics Buffer"),
            contents: bytemuck::cast_slice(&initial_boxes),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
        });

        let initial_colliders = vec![GpuCollider::NONE; MAX_STATIC_COLLIDERS as usize];

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

        // The simulation steps on its own fixed-step clock, at 60 Hz until `update_params` says
        // otherwise; `advance` turns the game's time into whole steps of it.
        let clock = gizmo_core::time::PhysicsTime::new(60);
        let gravity = [0.0, -9.81, 0.0];
        let params = sim_params(clock.fixed_dt(), gravity, 0, 0, 0);

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
            clock,
            gravity,
            collider_count: 0,
            collider_slots: vec![Slot::Free; MAX_STATIC_COLLIDERS as usize],
            free_colliders: Vec::new(),
            box_high_water: 0,
            box_slots: vec![Slot::Free; max_boxes as usize],
            free_boxes: Vec::new(),
            leaving: Vec::new(),
            left: Vec::new(),
            boxes_full_reported: false,
            colliders_full_reported: false,
            copies_recorded: std::sync::atomic::AtomicU64::new(0),
            copy_in_flight: std::sync::atomic::AtomicU64::new(0),
            copy_returned: std::sync::atomic::AtomicU64::new(0),
            first_copy: vec![0; max_boxes as usize],
            params,
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

    /// Overwrites the body in slot `index` — teleporting it, or pushing it, from the CPU. Only a slot
    /// that holds a body: a body is put in with [`add_box`](Self::add_box).
    pub fn update_box(&self, queue: &wgpu::Queue, index: u32, box_struct: &GpuBox) {
        // A free slot stays free: a body written into it would be simulated with nothing in the
        // table knowing it is there (that is `add_box`'s job). A body is not freed by writing
        // `BOX_FREE` into it either — that is `remove_box`'s.
        if self.box_slots.get(index as usize).is_some_and(|held| matches!(held, Slot::Live | Slot::Owned(_))) {
            self.write_box(queue, index, &live(*box_struct));
        }
    }

    fn write_box(&self, queue: &wgpu::Queue, slot: u32, body: &GpuBox) {
        let offset = (slot as wgpu::BufferAddress) * std::mem::size_of::<GpuBox>() as wgpu::BufferAddress;
        queue.write_buffer(&self.boxes_buffer, offset, bytemuck::cast_slice(&[*body]));
    }

    /// Puts `body` in a free slot and returns the slot, or `None` when all
    /// [`max_boxes`](Self::max_boxes) are taken (logged once, until a slot is freed). With an
    /// `owner` the slot is that entity's: [`retain_owned_boxes`](Self::retain_owned_boxes) frees it
    /// once the entity no longer claims it. A slot freed before is given out first.
    pub fn add_box(
        &mut self,
        queue: &wgpu::Queue,
        body: &GpuBox,
        owner: Option<gizmo_core::Entity>,
    ) -> Option<u32> {
        let slot = match self.free_boxes.pop() {
            Some(slot) => slot,
            None if self.box_high_water < self.max_boxes => {
                self.box_high_water += 1;
                self.write_params(queue);
                self.box_high_water - 1
            }
            None => {
                if !std::mem::replace(&mut self.boxes_full_reported, true) {
                    report_full("box", self.max_boxes);
                }
                return None;
            }
        };
        self.box_slots[slot as usize] = owner.map_or(Slot::Live, Slot::Owned);
        self.first_copy[slot as usize] = self.copies_recorded.load(Ordering::SeqCst);
        self.write_box(queue, slot, &live(*body));
        Some(slot)
    }

    /// As many of `bodies` as fit in consecutive slots past the highest in use, in one upload — a
    /// field of thousands put in at once — and the slots they took (empty when none fit). The run is
    /// contiguous, so a slot freed below is not used; a table emptied from the top gives its room
    /// back (see [`remove_box`](Self::remove_box)).
    pub fn add_boxes(&mut self, queue: &wgpu::Queue, bodies: &[GpuBox]) -> std::ops::Range<u32> {
        let start = self.box_high_water;
        let room = (self.max_boxes - start) as usize;
        if bodies.len() > room && !std::mem::replace(&mut self.boxes_full_reported, true) {
            report_full("box", self.max_boxes);
        }
        let bodies: Vec<GpuBox> = bodies.iter().take(room).map(|b| live(*b)).collect();
        let end = start + bodies.len() as u32;
        if bodies.is_empty() {
            return start..end;
        }
        let offset = (start as wgpu::BufferAddress) * std::mem::size_of::<GpuBox>() as wgpu::BufferAddress;
        queue.write_buffer(&self.boxes_buffer, offset, bytemuck::cast_slice(&bodies));
        self.box_slots[start as usize..end as usize].fill(Slot::Live);
        let next_copy = self.copies_recorded.load(Ordering::SeqCst);
        self.first_copy[start as usize..end as usize].fill(next_copy);
        self.box_high_water = end;
        self.write_params(queue);
        start..end
    }

    /// Frees `slot`: the body in it is no longer simulated, found by its neighbours or drawn, and a
    /// joint to it holds nothing. The next [`add_box`](Self::add_box) reuses it. A free slot is left
    /// as it is.
    ///
    /// There was no way out: a body stayed in the simulation until the renderer was dropped, and
    /// the engine numbered its bodies by counting them, so a body despawned and another spawned
    /// shared a slot with one still alive.
    pub fn remove_box(&mut self, queue: &wgpu::Queue, slot: u32) {
        let Some(held) = self.box_slots.get_mut(slot as usize) else { return };
        if !matches!(held, Slot::Live | Slot::Owned(_)) {
            return;
        }
        *held = Slot::Leaving;
        // Only the state, so the next step knows where the body was: it wakes what rested on it
        // there, forgets its contacts and frees the slot (`BOX_LEAVING`). Not given out until then.
        let offset = (slot as wgpu::BufferAddress) * std::mem::size_of::<GpuBox>() as wgpu::BufferAddress
            + std::mem::offset_of!(GpuBox, state) as wgpu::BufferAddress;
        queue.write_buffer(&self.boxes_buffer, offset, bytemuck::bytes_of(&BOX_LEAVING));
        self.leaving.push(slot);
    }

    /// The slots whose leaving body has been through a step are free to give out; and a table whose
    /// top slots are free gives that room back, so its passes run over fewer slots and a run of
    /// [`add_boxes`](Self::add_boxes) fits again. A slot still leaving is never given back: its body
    /// has yet to be taken out by a step, which runs over the slots below the high-water mark only.
    fn release_left_slots(&mut self, queue: &wgpu::Queue) {
        if self.left.is_empty() {
            return;
        }
        for &slot in &self.left {
            self.box_slots[slot as usize] = Slot::Free;
        }
        self.free_boxes.append(&mut self.left);
        self.boxes_full_reported = false;
        let high_water = self.box_high_water;
        while self.box_high_water > 0 && self.box_slots[self.box_high_water as usize - 1] == Slot::Free {
            self.box_high_water -= 1;
        }
        if self.box_high_water != high_water {
            let top = self.box_high_water;
            self.free_boxes.retain(|&slot| slot < top);
            self.write_params(queue);
        }
    }

    /// Frees every box slot held for an entity that `keep` — asked with the entity and its slot —
    /// says no longer claims it. The engine's submit path calls it every frame with "is still
    /// linked to that slot".
    pub fn retain_owned_boxes(&mut self, queue: &wgpu::Queue, mut keep: impl FnMut(gizmo_core::Entity, u32) -> bool) {
        let gone: Vec<u32> = self.box_slots[..self.box_high_water as usize]
            .iter()
            .enumerate()
            .filter_map(|(slot, held)| match *held {
                Slot::Owned(owner) if !keep(owner, slot as u32) => Some(slot as u32),
                _ => None,
            })
            .collect();
        for slot in gone {
            self.remove_box(queue, slot);
        }
    }

    /// How many box slots hold a body.
    pub fn live_boxes(&self) -> u32 {
        self.box_slots[..self.box_high_water as usize]
            .iter()
            .filter(|held| matches!(held, Slot::Live | Slot::Owned(_)))
            .count() as u32
    }

    /// Whether box slot `slot` holds the body put in for `owner` — the engine's check that an
    /// entity's `GpuPhysicsLink` is backed.
    pub fn holds_box(&self, slot: u32, owner: gizmo_core::Entity) -> bool {
        self.box_slots.get(slot as usize) == Some(&Slot::Owned(owner))
    }

    /// Whether static-collider slot `slot` holds the collider put in for `owner`.
    pub fn holds_collider(&self, slot: u32, owner: gizmo_core::Entity) -> bool {
        self.collider_slots.get(slot as usize) == Some(&Slot::Owned(owner))
    }

    /// Whether the data [`poll_readback_data`](Self::poll_readback_data) last returned was copied
    /// after the body now in `slot` was put in. A readback is copied a frame or more before it
    /// arrives, so a body put in meanwhile — into a fresh slot, or one a body left — is not in it:
    /// its slot there still holds nothing, or the body that left. Reading that back put a new body at
    /// the origin, or where the old one was, for a frame.
    pub fn readback_holds(&self, slot: u32) -> bool {
        self.first_copy
            .get(slot as usize)
            .is_some_and(|&first| first <= self.copy_returned.load(Ordering::SeqCst))
    }

    /// Overwrites one static collider.
    /// Overwrites the static collider in slot `index`. A free slot stays free — that is
    /// [`add_collider`](Self::add_collider)'s.
    pub fn update_collider(&self, queue: &wgpu::Queue, index: u32, collider: &GpuCollider) {
        if self.collider_slots.get(index as usize).is_some_and(|held| matches!(held, Slot::Live | Slot::Owned(_))) {
            self.write_collider(queue, index, collider);
        }
    }

    fn write_collider(&self, queue: &wgpu::Queue, slot: u32, collider: &GpuCollider) {
        let offset = (slot as wgpu::BufferAddress) * std::mem::size_of::<GpuCollider>() as wgpu::BufferAddress;
        queue.write_buffer(&self.colliders_buffer, offset, bytemuck::cast_slice(&[*collider]));
    }

    /// Adds a static collider in a free slot and returns the slot, or `None` when all
    /// [`MAX_STATIC_COLLIDERS`] are taken (logged once, until a slot is freed). With an `owner`,
    /// [`retain_owned_colliders`](Self::retain_owned_colliders) frees it once that entity no longer
    /// claims it.
    pub fn add_collider(
        &mut self,
        queue: &wgpu::Queue,
        collider: &GpuCollider,
        owner: Option<gizmo_core::Entity>,
    ) -> Option<u32> {
        let slot = match self.free_colliders.pop() {
            Some(slot) => slot,
            None if self.collider_count < MAX_STATIC_COLLIDERS => {
                self.collider_count += 1;
                self.write_params(queue);
                self.collider_count - 1
            }
            None => {
                if !std::mem::replace(&mut self.colliders_full_reported, true) {
                    report_full("static collider", MAX_STATIC_COLLIDERS);
                }
                return None;
            }
        };
        self.collider_slots[slot as usize] = owner.map_or(Slot::Live, Slot::Owned);
        self.write_collider(queue, slot, collider);
        Some(slot)
    }

    /// Frees static-collider slot `slot`: the solver skips it ([`GpuCollider::NONE`]) and the next
    /// [`add_collider`](Self::add_collider) reuses it.
    pub fn remove_collider(&mut self, queue: &wgpu::Queue, slot: u32) {
        let Some(held) = self.collider_slots.get_mut(slot as usize) else { return };
        if *held == Slot::Free {
            return;
        }
        *held = Slot::Free;
        self.write_collider(queue, slot, &GpuCollider::NONE);
        self.free_colliders.push(slot);
        self.colliders_full_reported = false;
    }

    /// Frees every static-collider slot held for an entity that `keep` — asked with the entity and
    /// its slot — says no longer claims it.
    pub fn retain_owned_colliders(&mut self, queue: &wgpu::Queue, mut keep: impl FnMut(gizmo_core::Entity, u32) -> bool) {
        let gone: Vec<u32> = self.collider_slots[..self.collider_count as usize]
            .iter()
            .enumerate()
            .filter_map(|(slot, held)| match *held {
                Slot::Owned(owner) if !keep(owner, slot as u32) => Some(slot as u32),
                _ => None,
            })
            .collect();
        for slot in gone {
            self.remove_collider(queue, slot);
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
        // The solver reads how many joints there are from the parameters: a joint added without
        // them was one it never looked at, until the game happened to call `update_params`.
        self.write_params(queue);
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

    /// Sets the simulation's step — the fixed length of time each step advances, in seconds — and
    /// its gravity. How many steps run is not this: [`advance`](Self::advance) turns the game's time
    /// into steps of this length, so a longer step means fewer of them, not faster bodies.
    pub fn update_params(&mut self, queue: &wgpu::Queue, dt: f32, gravity: [f32; 3]) {
        self.clock.set_fixed_dt(dt);
        self.gravity = gravity;
        self.write_params(queue);
    }

    /// Writes the parameters the solver reads from what this system holds — the step, gravity and
    /// how many colliders and joints are live — so no one of them can be written without the rest.
    fn write_params(&mut self, queue: &wgpu::Queue) {
        self.params = sim_params(
            self.clock.fixed_dt(),
            self.gravity,
            self.box_high_water,
            self.collider_count,
            self.joint_count,
        );
        queue.write_buffer(&self.params_buffer, 0, bytemuck::cast_slice(&[self.params]));
    }

    /// The parameters the solver was last given: its step, gravity, and how many bodies, static
    /// colliders and joints it reads.
    pub fn params(&self) -> PhysicsSimParams {
        self.params
    }

    /// **Steps the bodies by `elapsed` seconds of the game's time**, in whole steps of the fixed
    /// step ([`update_params`](Self::update_params), 1/60 s by default): none while the game is
    /// paused, two in a frame of 1/30 s, and the remainder carried to the next frame. At most eight
    /// in one frame — time past that is dropped and logged rather than owed, as the CPU bodies'
    /// clock does ([`PhysicsTime`](gizmo_core::time::PhysicsTime)). Returns how many it recorded.
    ///
    /// It stepped once for every frame drawn, whatever the frame's length: the bodies fell twice as
    /// fast at 120 Hz as at 60, went on falling under ⏸, and took no notice of a time scale.
    pub fn advance(&mut self, encoder: &mut wgpu::CommandEncoder, queue: &wgpu::Queue, elapsed: f32) -> u32 {
        // The bodies taken out before the steps last recorded have been through them: their slots
        // are free now, and given back from the top. Before recording, so the parameters this
        // writes (`queue.write_buffer`, applied at the next submission) are the ones these steps run on.
        self.release_left_slots(queue);
        self.clock.accumulate(elapsed);
        let mut steps = 0;
        while self.clock.should_step() {
            self.clock.consume_step();
            // With no body in the table a step has nothing to move; the time is still spent, so
            // the first body put in does not inherit a backlog.
            if self.box_high_water > 0 {
                self.compute_pass(encoder);
            }
            steps += 1;
        }
        if steps > 0 && self.box_high_water > 0 {
            // These steps take the leaving bodies out; their slots are free from the next advance.
            self.left.append(&mut self.leaving);
        }
        steps
    }

    /// The simulation's clock: its step, how many steps it has run and the time it still owes.
    pub fn clock(&self) -> &gizmo_core::time::PhysicsTime {
        &self.clock
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
    /// Records one whole simulation step: forces, clear, build, narrowphase, solve, integrate,
    /// joints — each its own dispatch, in that order.
    pub fn compute_pass(&self, encoder: &mut wgpu::CommandEncoder) {
        tracing::trace!(
            boxes = self.box_high_water,
            joints = self.joint_count,
            si_iterations = 6,
            "[GpuPhysics] recording sequential-impulse solver passes"
        );
        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Physics Compute Pass"),
            timestamp_writes: None,
        });
        cpass.set_bind_group(0, &self.pipelines.compute_bind_group, &[]);

        // Faz 0: the step's forces, before the contacts are solved (see `apply_forces`).
        cpass.set_pipeline(&self.pipelines.pipeline_apply_forces);
        cpass.dispatch_workgroups(self.box_high_water.div_ceil(256), 1, 1);

        // ═══ Sequential Impulse Solver ═══
        // Faz 1: Grid'i bir kez inşa et
        cpass.set_pipeline(&self.pipelines.pipeline_clear);
        cpass.dispatch_workgroups(self.grid_size.div_ceil(256), 1, 1);

        cpass.set_pipeline(&self.pipelines.pipeline_build);
        cpass.dispatch_workgroups(self.box_high_water.div_ceil(256), 1, 1);

        // Faz 2: Narrowphase (Çarpışma Tespiti ve Contact Caching)
        cpass.set_pipeline(&self.pipelines.pipeline_narrowphase);
        cpass.dispatch_workgroups(self.box_high_water.div_ceil(256), 1, 1);

        // Faz 3: Çarpışma çözümünü N kez tekrarla (SI iterasyon)
        // Artık grid üzerinden değil, doğrudan contact cache üzerinden hesaplama yapıyor!
        let si_iterations = 6;
        for _ in 0..si_iterations {
            cpass.set_pipeline(&self.pipelines.pipeline_solve);
            cpass.dispatch_workgroups(self.box_high_water.div_ceil(256), 1, 1);
        }

        // Faz 4: Hız ve pozisyon entegrasyonu (tek seferde)
        cpass.set_pipeline(&self.pipelines.pipeline_integrate);
        cpass.dispatch_workgroups(self.box_high_water.div_ceil(256), 1, 1);

        // Faz 4: Joint constraint çözümü (entegrasyondan sonra)
        if self.joint_count > 0 {
            let joint_iterations = 4;
            for _ in 0..joint_iterations {
                cpass.set_pipeline(&self.pipelines.pipeline_solve_joints);
                cpass.dispatch_workgroups(self.box_high_water.div_ceil(256), 1, 1);
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
        cpass.dispatch_workgroups(self.box_high_water.div_ceil(256), 1, 1);
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
        // Enough workgroups for every slot in use and every joint; free slots draw no lines.
        cpass.dispatch_workgroups(
            self.box_high_water
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
            // Which copy this is, for `readback_holds`.
            let copy = self.copies_recorded.fetch_add(1, Ordering::SeqCst);
            self.copy_in_flight.store(copy, Ordering::SeqCst);
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

            self.copy_returned.store(self.copy_in_flight.load(Ordering::SeqCst), Ordering::SeqCst);
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
