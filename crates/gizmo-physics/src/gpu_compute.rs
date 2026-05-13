use std::sync::Arc;

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct GpuPhysicsLink {
    pub id: u32,
}
gizmo_core::impl_component!(GpuPhysicsLink);

pub struct GpuCompute {
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    pub compute_pipeline: wgpu::ComputePipeline,
    pub integrate_pipeline: wgpu::ComputePipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,

    pub nodes_buffer: Option<wgpu::Buffer>,
    pub elements_buffer: Option<wgpu::Buffer>,
    pub params_buffer: Option<wgpu::Buffer>,
    pub forces_x_buffer: Option<wgpu::Buffer>,
    pub forces_y_buffer: Option<wgpu::Buffer>,
    pub forces_z_buffer: Option<wgpu::Buffer>,
    pub staging_nodes_buffer: Option<wgpu::Buffer>,

    pub node_capacity: usize,
    pub element_capacity: usize,
}

impl GpuCompute {
    pub async fn new() -> Option<Self> {
        let instance = wgpu::Instance::default();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await?;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("Gizmo Physics Compute Device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                },
                None,
            )
            .await
            .ok()?;

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
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Soft Body Forces Compute Pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: "main",
            compilation_options: Default::default(),
        });

        let integrate_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Soft Body Integrate Compute Pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: "integrate",
            compilation_options: Default::default(),
        });

        Some(Self {
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

    pub fn step_soft_bodies(
        &mut self,
        soft_bodies: &mut [(
            gizmo_core::entity::Entity,
            crate::soft_body::SoftBodyMesh,
            crate::components::Transform,
        )],
        rigid_colliders: &[(
            gizmo_core::entity::Entity,
            crate::components::Transform,
            crate::components::Collider,
        )],
        dt: f32,
        gravity: gizmo_math::Vec3,
    ) {
        if soft_bodies.is_empty() {
            return;
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

        let mut current_node_offset = 0;
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

            current_node_offset = current_node_offset
                .checked_add(sb.nodes.len() as u32)
                .expect("Soft body node offset overflow. Too many nodes!");
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
            device_clone.poll(wgpu::Maintain::Wait);
        });

        if rx.recv().is_err() {
            return;
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
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuSoftBodyNode {
    pub position: [f32; 3],
    pub mass: f32,
    pub velocity: [f32; 3],
    pub is_fixed: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuTetrahedron {
    pub indices: [u32; 4],
    pub inv_rest_matrix_col0: [f32; 3],
    pub pad0: f32,
    pub inv_rest_matrix_col1: [f32; 3],
    pub pad1: f32,
    pub inv_rest_matrix_col2: [f32; 3],
    pub pad2: f32,
    pub rest_volume: f32,
    pub pad3: [f32; 3],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuParameters {
    pub dt: f32,
    pub mu: f32,
    pub lambda: f32,
    pub damping: f32,
    pub gravity: [f32; 3],
    pub num_elements: u32,
    pub num_nodes: u32,
    pub pad0: f32,
    pub pad1: f32,
    pub pad2: f32,
}
