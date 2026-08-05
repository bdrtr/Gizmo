use std::sync::Arc;
use gizmo_physics_core::BodyHandle;

/// WGPU compute state for stepping tetrahedral FEM soft bodies on the GPU.
///
/// This whole module is compiled only under this crate's `gpu_physics` feature, which is
/// off by default. It is an *alternative* to the CPU integrator
/// [`crate::soft_body::SoftBodyMesh::step`], not a replacement wired into it: the engine's
/// own driver [`crate::system::soft_body_step_system`] runs the CPU path, so this type is
/// only reached by constructing it and calling [`GpuCompute::step_soft_bodies`] yourself.
/// Treat it as experimental — no test in this workspace exercises it (it needs a real
/// adapter).
///
/// The pipelines and the bind group layout are built once by [`GpuCompute::new`]; every
/// buffer starts as `None` and is allocated lazily on the first step, then grown as the
/// flattened node/element counts grow.
///
/// The CPU [`crate::soft_body::SoftBodyMesh`] instances stay the source of truth: each step
/// re-uploads them in full, runs two compute passes, and reads the integrated node state
/// back — no state is kept on the GPU between steps except the (overwritten) buffers.
pub struct GpuCompute {
    /// Logical device that owns the pipelines and every buffer below. Held in an `Arc`
    /// because [`GpuCompute::step_soft_bodies`] clones it into a short-lived helper thread
    /// that polls the device while the readback mapping is in flight.
    pub device: Arc<wgpu::Device>,

    /// Queue used for all buffer writes and for the single command submission per step.
    /// Must belong to [`GpuCompute::device`].
    pub queue: Arc<wgpu::Queue>,

    /// Pass 1 — the `main` entry point of `soft_body.wgsl`, dispatched with one invocation
    /// per tetrahedron (workgroup size 64). It builds the deformation gradient
    /// `F = Ds · Dm⁻¹`, evaluates the Neo-Hookean first Piola–Kirchhoff stress, and scatters
    /// the four nodal forces into the per-axis accumulators. An element whose determinant
    /// `J = det(F)` falls below 0.05 (near-inverted or crushed) contributes no force at all.
    pub compute_pipeline: wgpu::ComputePipeline,

    /// Pass 2 — the `integrate` entry point of `soft_body.wgsl`, dispatched with one
    /// invocation per node (workgroup size 64). It turns the accumulated force into an
    /// acceleration, adds gravity, applies damping, takes one semi-implicit Euler step,
    /// resolves a ground plane hard-coded at y = 0.1 m, and finally zeroes that node's force
    /// accumulators for the next step. The ground contact is not a bare clamp: a node whose
    /// step would end below the plane keeps its new x/z, has its y pinned to 0.1, and gets
    /// `velocity.y *= -0.1` (restitution) plus `velocity.x/z *= 0.8` (friction).
    pub integrate_pipeline: wgpu::ComputePipeline,

    /// Layout shared by both pipelines: binding 0 = nodes (read/write storage),
    /// 1 = elements (read-only storage), 2 = parameters (uniform), 3/4/5 = the X/Y/Z force
    /// accumulators (read/write storage). All entries are `COMPUTE`-visible, none use a
    /// dynamic offset, and the bind group is rebuilt from this layout on every step.
    pub bind_group_layout: wgpu::BindGroupLayout,

    /// Flattened [`GpuSoftBodyNode`] array covering *all* soft bodies of the current step,
    /// concatenated body by body in slice order. `None` until the first step. Rewritten
    /// before each dispatch and read back after it. Sized for [`GpuCompute::node_capacity`]
    /// nodes; the tail past the live node count holds stale data, which the shaders skip via
    /// [`GpuParameters::num_nodes`].
    pub nodes_buffer: Option<wgpu::Buffer>,

    /// Flattened [`GpuTetrahedron`] array for the same set of bodies, with every node index
    /// already rebased onto [`GpuCompute::nodes_buffer`]. Read-only on the GPU. `None` until
    /// the first step; sized for [`GpuCompute::element_capacity`] elements, the tail again
    /// bounded out by [`GpuParameters::num_elements`].
    pub elements_buffer: Option<wgpu::Buffer>,

    /// Uniform buffer holding the single [`GpuParameters`] record for the step. Its size is
    /// fixed, so it is allocated once on the first step and only rewritten afterwards.
    pub params_buffer: Option<wgpu::Buffer>,

    /// X-axis force accumulator: one `i32` per node slot, in fixed point with a scale of
    /// 10 000 — i.e. the stored integer is newtons × 10⁴, so per-node axis forces beyond
    /// roughly ±2.1 × 10⁵ N overflow. Three separate scalar buffers are used because the
    /// shader accumulates with integer atomic adds. Zeroed by the host before each dispatch
    /// and again per node by the integrate pass.
    pub forces_x_buffer: Option<wgpu::Buffer>,

    /// Y-axis force accumulator; same fixed-point encoding and lifetime as
    /// [`GpuCompute::forces_x_buffer`].
    pub forces_y_buffer: Option<wgpu::Buffer>,

    /// Z-axis force accumulator; same fixed-point encoding and lifetime as
    /// [`GpuCompute::forces_x_buffer`].
    pub forces_z_buffer: Option<wgpu::Buffer>,

    /// Host-visible mirror of [`GpuCompute::nodes_buffer`] (`MAP_READ | COPY_DST`) and the
    /// destination of the post-dispatch copy that brings integrated node state back to the
    /// CPU. Reallocated together with the nodes buffer, so the two always have equal size.
    pub staging_nodes_buffer: Option<wgpu::Buffer>,

    /// Number of [`GpuSoftBodyNode`] slots currently allocated — a count of nodes, not
    /// bytes. Starts at 0. When a step needs more it becomes `max(needed, 2 × current,
    /// 1024)` and the nodes, three force and staging buffers are all recreated at that size.
    /// Never shrinks, so peak node count across all bodies determines the resident VRAM.
    pub node_capacity: usize,

    /// Number of [`GpuTetrahedron`] slots currently allocated — a count of elements, not
    /// bytes. Grows by the same `max(needed, 2 × current, 1024)` rule as
    /// [`GpuCompute::node_capacity`], independently of it, and likewise never shrinks.
    pub element_capacity: usize,
}

impl GpuCompute {
    /// Initializes the WGPU compute path for FEM soft bodies.
    ///
    /// # Errors
    ///
    /// Returns [`SoftBodyError::NoCompatibleAdapter`] if no GPU adapter could be
    /// acquired, and [`SoftBodyError::DeviceRequestFailed`] if requesting the
    /// logical device failed.
    pub async fn new() -> Result<Self, crate::SoftBodyError> {
        let instance = wgpu::Instance::default();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .map_err(|_| crate::SoftBodyError::NoCompatibleAdapter)?;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("Gizmo Physics Compute Device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    experimental_features: wgpu::ExperimentalFeatures::default(),
                    memory_hints: wgpu::MemoryHints::default(),
                    trace: wgpu::Trace::Off,
                },
            )
            .await
            .map_err(|e| crate::SoftBodyError::DeviceRequestFailed(e.to_string()))?;

        let shader = device.create_shader_module(wgpu::include_wgsl!("soft_body.wgsl"));

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Soft Body Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
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
                        ty: wgpu::BufferBindingType::Uniform,
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
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
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
            label: Some("Soft Body Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Soft Body Forces Compute Pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let integrate_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Soft Body Integrate Compute Pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("integrate"),
            compilation_options: Default::default(),
            cache: None,
        });

        Ok(Self {
            device: Arc::new(device),
            queue: Arc::new(queue),
            compute_pipeline,
            integrate_pipeline,
            bind_group_layout,
            nodes_buffer: None,
            elements_buffer: None,
            params_buffer: None,
            forces_x_buffer: None,
            forces_y_buffer: None,
            forces_z_buffer: None,
            staging_nodes_buffer: None,
            node_capacity: 0,
            element_capacity: 0,
        })
    }

    /// Advances every soft body by one timestep on the GPU and writes the integrated node
    /// state back into the CPU meshes.
    ///
    /// `dt` is in seconds and `gravity` is a world-space acceleration in m/s². `dt` reaches
    /// the shader unclamped, unlike [`crate::system::soft_body_step_system`], which caps the
    /// CPU path at 1/30 s for stability. Soft-body node positions are treated as already being
    /// in world space, so the [`BodyHandle`] and `Transform` carried alongside each mesh are
    /// ignored; the transforms of `rigid_colliders`, by contrast, are used.
    ///
    /// One call is one full pipeline flush: all bodies are flattened into a single
    /// node/element array, uploaded, dispatched (stress pass, then integrate pass), copied
    /// into the staging buffer and read back, all before the call returns.
    ///
    /// Material parameters are **not** per body: `mu`, `lambda` and `damping` are read from
    /// `soft_bodies[0]` and applied to every body. Debug builds log a `tracing` error when
    /// the bodies' `mu` values disagree; release builds apply the first body's material
    /// silently.
    ///
    /// After readback, each node that is not [`crate::soft_body::SoftBodyNode::is_fixed`] is
    /// swept through [`crate::soft_body::resolve_node_collision`] against every entry of
    /// `rigid_colliders` (so the CPU part of the step is O(nodes × colliders)); on a hit the
    /// swept position and reflected velocity replace the GPU result, and note the sweep
    /// starts from the position the GPU has *already* integrated. Fixed nodes are skipped
    /// entirely and keep their CPU state — the integrate pass does not move them either.
    /// Independently of `rigid_colliders`, the integrate pass holds every free node it writes
    /// at y ≥ 0.1 m, with restitution (`velocity.y *= -0.1`) and friction
    /// (`velocity.x/z *= 0.8`) applied on that contact.
    ///
    /// # Errors
    ///
    /// Returns [`crate::SoftBodyError::NodeOffsetOverflow`] if the node count summed over
    /// all bodies exceeds `u32::MAX`, and [`crate::SoftBodyError::DeviceRequestFailed`] if
    /// the buffer-mapping callback is dropped without reporting, leaving the readback
    /// channel closed. In both cases the CPU meshes are left as they were.
    pub fn step_soft_bodies(
        &mut self,
        soft_bodies: &mut [(
            BodyHandle,
            crate::soft_body::SoftBodyMesh,
            gizmo_physics_core::components::Transform,
        )],
        rigid_colliders: &[(
            BodyHandle,
            gizmo_physics_core::components::Transform,
            gizmo_physics_core::components::Collider,
        )],
        dt: f32,
        gravity: gizmo_math::Vec3,
    ) -> Result<(), crate::SoftBodyError> {
        if soft_bodies.is_empty() {
            return Ok(());
        }

        #[cfg(debug_assertions)]
        if soft_bodies
            .iter()
            .any(|(_, sb, _)| sb.mu != soft_bodies[0].1.mu)
        {
            tracing::error!("Warning: Mixed soft body materials not supported on GPU path");
        }

        // --- GPU PATHWAY ---
        // 1. Flatten all nodes and elements
        let mut gpu_nodes = Vec::new();
        let mut gpu_elements = Vec::new();
        let mut node_offsets = Vec::new();

        let mut current_node_offset: u32 = 0;
        for (_, sb, _) in soft_bodies.iter() {
            node_offsets.push(current_node_offset);

            for node in &sb.nodes {
                gpu_nodes.push(GpuSoftBodyNode {
                    position: node.position.into(),
                    mass: node.mass,
                    velocity: node.velocity.into(),
                    is_fixed: if node.is_fixed { 1 } else { 0 },
                });
            }

            for elem in &sb.elements {
                gpu_elements.push(GpuTetrahedron {
                    indices: [
                        elem.node_indices[0] + current_node_offset,
                        elem.node_indices[1] + current_node_offset,
                        elem.node_indices[2] + current_node_offset,
                        elem.node_indices[3] + current_node_offset,
                    ],
                    inv_rest_matrix_col0: elem.inv_rest_matrix.x_axis.into(),
                    pad0: 0.0,
                    inv_rest_matrix_col1: elem.inv_rest_matrix.y_axis.into(),
                    pad1: 0.0,
                    inv_rest_matrix_col2: elem.inv_rest_matrix.z_axis.into(),
                    pad2: 0.0,
                    rest_volume: elem.rest_volume,
                    pad3: [0.0; 3],
                });
            }

            // u32 taşması (aşırı sayıda düğüm) artık sessiz log+skip yerine somut
            // bir hata olarak yayılır.
            current_node_offset = current_node_offset
                .checked_add(sb.nodes.len() as u32)
                .ok_or(crate::SoftBodyError::NodeOffsetOverflow)?;
        }

        let params = GpuParameters {
            dt,
            mu: soft_bodies[0].1.mu, // We assume uniform material for now
            lambda: soft_bodies[0].1.lambda,
            damping: soft_bodies[0].1.damping,
            gravity: gravity.into(),
            num_elements: gpu_elements.len() as u32,
            num_nodes: gpu_nodes.len() as u32,
            pad0: 0.0,
            pad1: 0.0,
            pad2: 0.0,
        };

        // 2. Create GPU Buffers
        let node_count = gpu_nodes.len();
        let element_count = gpu_elements.len();

        if self.node_capacity < node_count || self.nodes_buffer.is_none() {
            let capacity = node_count.max(self.node_capacity * 2).max(1024);
            self.node_capacity = capacity;

            self.nodes_buffer = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Nodes Buffer"),
                size: (capacity * std::mem::size_of::<GpuSoftBodyNode>()) as u64,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }));

            self.forces_x_buffer = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Forces X Buffer"),
                size: (capacity * 4) as u64,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));

            self.forces_y_buffer = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Forces Y Buffer"),
                size: (capacity * 4) as u64,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));

            self.forces_z_buffer = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Forces Z Buffer"),
                size: (capacity * 4) as u64,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));

            self.staging_nodes_buffer = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Staging Nodes Buffer"),
                size: (capacity * std::mem::size_of::<GpuSoftBodyNode>()) as u64,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }

        if self.element_capacity < element_count || self.elements_buffer.is_none() {
            let capacity = element_count.max(self.element_capacity * 2).max(1024);
            self.element_capacity = capacity;

            self.elements_buffer = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Elements Buffer"),
                size: (capacity * std::mem::size_of::<GpuTetrahedron>()) as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }

        if self.params_buffer.is_none() {
            self.params_buffer = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Params Buffer"),
                size: std::mem::size_of::<GpuParameters>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }

        // Write data to persistent buffers
        self.queue.write_buffer(
            self.nodes_buffer.as_ref().unwrap(),
            0,
            bytemuck::cast_slice(&gpu_nodes),
        );
        self.queue.write_buffer(
            self.elements_buffer.as_ref().unwrap(),
            0,
            bytemuck::cast_slice(&gpu_elements),
        );
        self.queue.write_buffer(
            self.params_buffer.as_ref().unwrap(),
            0,
            bytemuck::cast_slice(&[params]),
        );

        // Clear forces buffers
        let zero_forces = vec![0u8; node_count * 4];
        self.queue
            .write_buffer(self.forces_x_buffer.as_ref().unwrap(), 0, &zero_forces);
        self.queue
            .write_buffer(self.forces_y_buffer.as_ref().unwrap(), 0, &zero_forces);
        self.queue
            .write_buffer(self.forces_z_buffer.as_ref().unwrap(), 0, &zero_forces);

        // 3. Bind Group
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Soft Body Compute Bind Group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.nodes_buffer.as_ref().unwrap().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.elements_buffer.as_ref().unwrap().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.params_buffer.as_ref().unwrap().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.forces_x_buffer.as_ref().unwrap().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.forces_y_buffer.as_ref().unwrap().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: self.forces_z_buffer.as_ref().unwrap().as_entire_binding(),
                },
            ],
        });

        // 4. Encode Commands
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: None,
                timestamp_writes: None,
            });
            cpass.set_bind_group(0, &bind_group, &[]);

            // Pass 1: Compute Forces
            cpass.set_pipeline(&self.compute_pipeline);
            let workgroups_forces = params.num_elements.div_ceil(64);
            cpass.dispatch_workgroups(workgroups_forces, 1, 1);

            // Pass 2: Integrate Positions
            cpass.set_pipeline(&self.integrate_pipeline);
            let workgroups_integrate = params.num_nodes.div_ceil(64);
            cpass.dispatch_workgroups(workgroups_integrate, 1, 1);
        }

        // Copy nodes back to CPU for rendering/CPU collision
        let nodes_byte_size = (node_count * std::mem::size_of::<GpuSoftBodyNode>()) as u64;
        encoder.copy_buffer_to_buffer(
            self.nodes_buffer.as_ref().unwrap(),
            0,
            self.staging_nodes_buffer.as_ref().unwrap(),
            0,
            nodes_byte_size,
        );

        self.queue.submit(Some(encoder.finish()));

        // 5. Read back the mapped buffers
        let staging_nodes = self.staging_nodes_buffer.as_ref().unwrap();
        let slice = staging_nodes.slice(0..nodes_byte_size);

        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |v| {
            let _ = tx.send(v);
        });

        let device_clone = self.device.clone();
        std::thread::spawn(move || {
            let _ = device_clone.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            });
        });

        if rx.recv().is_err() {
            return Err(crate::SoftBodyError::DeviceRequestFailed(
                "GPU readback channel closed before mapping completed".to_string(),
            ));
        }

        let mapped = slice.get_mapped_range();
        let output_nodes: &[GpuSoftBodyNode] = bytemuck::cast_slice(&mapped);

        // 6. Update CPU models and apply CPU collisions
        for (sb_idx, (_, sb, _)) in soft_bodies.iter_mut().enumerate() {
            let offset = node_offsets[sb_idx] as usize;
            for (i, node) in sb.nodes.iter_mut().enumerate() {
                if node.is_fixed {
                    continue;
                }

                let global_idx = offset + i;
                let out_node = &output_nodes[global_idx];

                let integrated_pos = gizmo_math::Vec3::from(out_node.position);
                let integrated_vel = gizmo_math::Vec3::from(out_node.velocity);

                let (new_pos, new_vel, collided) = crate::soft_body::resolve_node_collision(
                    integrated_pos,
                    integrated_vel,
                    dt,
                    rigid_colliders,
                );

                if collided {
                    node.position = new_pos;
                    node.velocity = new_vel;
                    // Note: This modifies the CPU side. Next substep will upload this fixed position to GPU.
                } else {
                    node.position = integrated_pos;
                    node.velocity = integrated_vel;
                }
            }
        }

        // Unmap
        drop(mapped);
        staging_nodes.unmap();

        Ok(())
    }
}

/// One soft-body node in the layout the `SoftBodyNode` struct of `soft_body.wgsl` expects.
///
/// 32 bytes: `position` at offset 0, `mass` at 12, `velocity` at 16, `is_fixed` at 28. The
/// scalars are deliberately interleaved between the two vectors so each `vec3<f32>` lands on
/// a 16-byte boundary without extra padding fields.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuSoftBodyNode {
    /// World-space position in metres. Read as the current configuration by the stress pass
    /// and overwritten in place by the integrate pass; this is the value copied back to the
    /// CPU mesh.
    pub position: [f32; 3],

    /// Node mass in kilograms. The shader uses `1/mass` only when `mass > 0` and otherwise
    /// substitutes an inverse mass of 0 — so a zero or negative mass makes the node ignore
    /// elastic forces while *still* accelerating under gravity. (The CPU integrator
    /// [`crate::soft_body::SoftBodyMesh::step`] instead skips such nodes entirely, leaving
    /// them frozen.)
    pub mass: f32,

    /// World-space velocity in m/s, advanced in place by the integrate pass and copied back
    /// to the CPU mesh.
    pub velocity: [f32; 3],

    /// Non-zero pins the node: the integrate pass returns immediately for it, leaving its
    /// position and velocity untouched (it still clears that node's force accumulators).
    /// Mirrors [`crate::soft_body::SoftBodyNode::is_fixed`], encoded as 1 / 0.
    pub is_fixed: u32,
}

/// One tetrahedral element, as uploaded to the `elements` storage binding of
/// `soft_body.wgsl`.
///
/// The `pad*` members carry no data: they are written as zero and the shader never reads
/// them. `pad0` and `pad1` keep the three `inv_rest_matrix_col*` vectors 16 bytes apart, as
/// the WGSL `vec3<f32>` alignment rule asks for; `pad2` and `pad3` fill out the words after
/// the last column and after `rest_volume`. Removing one shifts every field after it.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuTetrahedron {
    /// The four corner nodes, as indices into the *flattened* node buffer:
    /// [`GpuCompute::step_soft_bodies`] adds each body's base offset to the per-body
    /// [`crate::soft_body::Tetrahedron::node_indices`] before upload, so these are global,
    /// not body-local. [`crate::soft_body::SoftBodyMesh::add_element`] does bounds-check the
    /// per-body indices it accepts, and rebasing preserves that — but the upload path does not
    /// RE-check the rebased values against the flattened node count, so a caller that pushed
    /// into the public `nodes`/`elements` vectors directly can still get out-of-range indices
    /// here.
    pub indices: [u32; 4],

    /// First column of the inverse reference shape matrix `Dm⁻¹`, in units of 1/metre. The
    /// three columns together give the deformation gradient `F = Ds · Dm⁻¹`, where `Ds` is
    /// built in the shader from the current edge vectors `p1-p0`, `p2-p0`, `p3-p0`.
    pub inv_rest_matrix_col0: [f32; 3],

    /// Unused word at bytes 28..32. Its only effect is to start `inv_rest_matrix_col1` at
    /// offset 32 rather than 28.
    pub pad0: f32,

    /// Second column of `Dm⁻¹`, in units of 1/metre.
    pub inv_rest_matrix_col1: [f32; 3],

    /// Unused word at bytes 44..48, which is what puts `inv_rest_matrix_col2` at offset 48.
    pub pad1: f32,

    /// Third column of `Dm⁻¹`, in units of 1/metre.
    pub inv_rest_matrix_col2: [f32; 3],

    /// Unused word at bytes 60..64. Unlike `pad0`/`pad1` it does not precede a vector — it
    /// closes the third column's 16-byte slot, leaving `rest_volume` at offset 64.
    pub pad2: f32,

    /// Undeformed volume `V₀` of the element in cubic metres.
    /// [`crate::soft_body::Tetrahedron::calculate_rest_data`] produces it as `|det(Dm)| / 6`,
    /// so it is unsigned regardless of node winding. It scales the nodal forces the stress
    /// pass emits: a zero here silences the element completely.
    pub rest_volume: f32,

    /// Unused trailing words after `rest_volume`, occupying bytes 68..80. Written as zero
    /// and never read; nothing follows them, so they only affect the element stride.
    pub pad3: [f32; 3],
}

/// The per-step uniform block, matching the `Parameters` struct of `soft_body.wgsl`
/// (48 bytes, with `gravity` on the 16-byte boundary at offset 16).
///
/// Exactly one record is uploaded per call to [`GpuCompute::step_soft_bodies`]; both compute
/// passes read the same copy. The material terms are global to the dispatch, which is why
/// that method can only simulate one material at a time.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuParameters {
    /// Timestep in seconds for the single semi-implicit Euler update the integrate pass
    /// performs. It is *not* substepped: one call advances the bodies by exactly this much.
    pub dt: f32,

    /// Lamé second parameter (shear modulus) μ in pascals, entering the Neo-Hookean stress
    /// as `μ·(F − F⁻ᵀ)`. It governs resistance to shape change; larger is stiffer.
    pub mu: f32,

    /// Lamé first parameter λ in pascals, entering the stress as `λ·log(J)·F⁻ᵀ`. It is the
    /// volume term — it penalizes `J = det(F)` straying from 1.
    pub lambda: f32,

    /// Velocity damping *rate* in 1/s: the integrate pass scales velocity by
    /// `max(1 − damping·dt, 0)` once per step, so `damping ≥ 1/dt` stops the body dead and
    /// 0 leaves it undamped. [`GpuCompute::step_soft_bodies`] copies
    /// [`crate::soft_body::SoftBodyMesh::damping`] in verbatim, but the CPU integrator reads
    /// that same field as a per-second *retention factor* (`v *= damping.powf(dt)`) — with
    /// the default 0.99 the two paths therefore damp very differently.
    pub damping: f32,

    /// World-space gravitational acceleration in m/s², added to every free node's
    /// acceleration irrespective of its mass.
    pub gravity: [f32; 3],

    /// Number of live [`GpuTetrahedron`] entries. Stress-pass invocations at or beyond this
    /// index return immediately, which is what makes an over-allocated element buffer safe.
    pub num_elements: u32,

    /// Number of live [`GpuSoftBodyNode`] entries. Same early-out bound for the integrate
    /// pass, and hence also the range of force accumulator slots that get cleared.
    pub num_nodes: u32,

    /// Unused word at bytes 36..40, the first of the three that carry the record from its
    /// 36 bytes of live data up to a size the 16-byte uniform-block alignment accepts.
    pub pad0: f32,

    /// Unused word at bytes 40..44. Never written with anything but zero.
    pub pad1: f32,

    /// Unused word at bytes 44..48 — the last word of the record. Dropping any of the three
    /// pads would shorten the uploaded record below the 48 bytes the shader's uniform block
    /// occupies.
    pub pad2: f32,
}
