use std::sync::Arc;

pub struct GpuCompute {
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    pub compute_pipeline: wgpu::ComputePipeline,
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

        let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Soft Body Compute Pipeline"),
            layout: None,
            module: &shader,
            entry_point: "main",
            compilation_options: Default::default(),
        });

        Some(Self {
            device: Arc::new(device),
            queue: Arc::new(queue),
            compute_pipeline,
        })
    }

    pub fn step_soft_bodies(&self, soft_bodies: &mut [(gizmo_core::entity::Entity, crate::soft_body::SoftBodyMesh, crate::components::Transform)], rigid_colliders: &[(gizmo_core::entity::Entity, crate::components::Transform, crate::components::Collider)], dt: f32, gravity: gizmo_math::Vec3) {
        if soft_bodies.is_empty() {
            return;
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
            
            current_node_offset += sb.nodes.len() as u32;
        }
        
        let params = GpuParameters {
            dt,
            mu: soft_bodies[0].1.mu, // We assume uniform material for now
            lambda: soft_bodies[0].1.lambda,
            damping: soft_bodies[0].1.damping,
            gravity: gravity.into(),
            num_elements: gpu_elements.len() as u32,
        };
        
        // 2. Create GPU Buffers
        use wgpu::util::DeviceExt;
        
        let nodes_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Nodes Buffer"),
            contents: bytemuck::cast_slice(&gpu_nodes),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });

        let elements_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Elements Buffer"),
            contents: bytemuck::cast_slice(&gpu_elements),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let params_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Params Buffer"),
            contents: bytemuck::cast_slice(&[params]),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        let zero_forces: Vec<i32> = vec![0; gpu_nodes.len()];
        let forces_x_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Forces X Buffer"),
            contents: bytemuck::cast_slice(&zero_forces),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
        let forces_y_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Forces Y Buffer"),
            contents: bytemuck::cast_slice(&zero_forces),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
        let forces_z_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Forces Z Buffer"),
            contents: bytemuck::cast_slice(&zero_forces),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });

        // 3. Create Bind Group
        let bind_group_layout = self.compute_pipeline.get_bind_group_layout(0);
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Soft Body Bind Group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: nodes_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: elements_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: params_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: forces_x_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: forces_y_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: forces_z_buffer.as_entire_binding() },
            ],
        });

        // 4. Encode Commands
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: None,
                timestamp_writes: None,
            });
            cpass.set_pipeline(&self.compute_pipeline);
            cpass.set_bind_group(0, &bind_group, &[]);
            let workgroups = (params.num_elements + 63) / 64;
            cpass.dispatch_workgroups(workgroups, 1, 1);
        }

        // Copy forces to a staging buffer
        let staging_forces_x = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Staging Forces X"),
            size: (gpu_nodes.len() * 4) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let staging_forces_y = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Staging Forces Y"),
            size: (gpu_nodes.len() * 4) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let staging_forces_z = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Staging Forces Z"),
            size: (gpu_nodes.len() * 4) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        encoder.copy_buffer_to_buffer(&forces_x_buffer, 0, &staging_forces_x, 0, (gpu_nodes.len() * 4) as u64);
        encoder.copy_buffer_to_buffer(&forces_y_buffer, 0, &staging_forces_y, 0, (gpu_nodes.len() * 4) as u64);
        encoder.copy_buffer_to_buffer(&forces_z_buffer, 0, &staging_forces_z, 0, (gpu_nodes.len() * 4) as u64);

        self.queue.submit(Some(encoder.finish()));

        // 5. Read back the mapped buffers
        let slice_x = staging_forces_x.slice(..);
        let slice_y = staging_forces_y.slice(..);
        let slice_z = staging_forces_z.slice(..);

        let (tx, rx) = std::sync::mpsc::channel();
        let tx1 = tx.clone();
        let tx2 = tx.clone();
        
        slice_x.map_async(wgpu::MapMode::Read, move |v| tx.send(v).unwrap());
        slice_y.map_async(wgpu::MapMode::Read, move |v| tx1.send(v).unwrap());
        slice_z.map_async(wgpu::MapMode::Read, move |v| tx2.send(v).unwrap());
        
        // Wait for GPU
        self.device.poll(wgpu::Maintain::Wait);
        
        rx.recv().unwrap().unwrap();
        rx.recv().unwrap().unwrap();
        rx.recv().unwrap().unwrap();

        let mapped_x = slice_x.get_mapped_range();
        let data_x: &[i32] = bytemuck::cast_slice(&mapped_x);
        let mapped_y = slice_y.get_mapped_range();
        let data_y: &[i32] = bytemuck::cast_slice(&mapped_y);
        let mapped_z = slice_z.get_mapped_range();
        let data_z: &[i32] = bytemuck::cast_slice(&mapped_z);

        // 6. Apply forces back to the CPU models!
        const FIXED_POINT_MULTIPLIER: f32 = 100000.0;
        
        for (sb_idx, (_, sb, _)) in soft_bodies.iter_mut().enumerate() {
            let offset = node_offsets[sb_idx] as usize;
            for (i, node) in sb.nodes.iter_mut().enumerate() {
                if node.is_fixed { continue; }
                
                let global_idx = offset + i;
                let force = gizmo_math::Vec3::new(
                    data_x[global_idx] as f32 / FIXED_POINT_MULTIPLIER,
                    data_y[global_idx] as f32 / FIXED_POINT_MULTIPLIER,
                    data_z[global_idx] as f32 / FIXED_POINT_MULTIPLIER,
                );
                
                let acceleration = force * (if node.mass > 0.0 { 1.0 / node.mass } else { 0.0 }) + gravity;
                node.velocity += acceleration * dt;
                
                // Add damping
                node.velocity *= params.damping;
                
                let next_pos = node.position + node.velocity * dt;
                let mut collided = false;
                
                // Extremely simple collision handling against rigid bodies
                let ray = crate::raycast::Ray::new(node.position, node.velocity.normalize_or_zero());
                let dist = node.velocity.length() * dt;
                
                if dist > 1e-5 {
                    for (_, col_trans, col) in rigid_colliders {
                        if let Some((d, n)) = crate::raycast::Raycast::ray_shape(&ray, &col.shape, col_trans) {
                            if d <= dist + 0.1 { // small radius
                                // Resolve collision
                                let bounce = 0.5;
                                let friction = 0.8;
                                
                                let vn = node.velocity.dot(n);
                                if vn < 0.0 {
                                    let vt = node.velocity - n * vn;
                                    node.velocity = vt * (1.0 - friction) - n * (vn * bounce);
                                }
                                
                                node.position += ray.direction * (d - 0.1).max(0.0);
                                collided = true;
                                break;
                            }
                        }
                    }
                }
                
                if !collided {
                    node.position = next_pos;
                }
            }
        }
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
}
