use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use wgpu::util::DeviceExt;

use super::types::*;
use super::pipeline::{create_physics_pipelines, PhysicsPipelines};

pub struct GpuPhysicsSystem {
    pub max_boxes: u32,
    pub grid_size: u32,
    pub boxes_buffer: wgpu::Buffer,
    pub params_buffer: wgpu::Buffer,
    pub grid_heads_buffer: wgpu::Buffer,
    pub linked_nodes_buffer: wgpu::Buffer,
    pub colliders_buffer: wgpu::Buffer,
    pub awake_flags_buffer: wgpu::Buffer,

    pub pipelines: PhysicsPipelines,

    pub box_vertex_buffer: wgpu::Buffer,
    pub box_index_buffer: wgpu::Buffer,
    pub index_count: u32,

    pub readback_buffer: wgpu::Buffer,
    // 0 = Idle, 1 = Copied to buffer (awaiting map), 2 = Mapping, 3 = Mapped (ready to read)
    pub readback_state: Arc<AtomicU8>,
    
    pub indirect_buffer: wgpu::Buffer,
    pub culled_boxes_buffer: wgpu::Buffer,
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
            let empty_col = GpuCollider { shape_type: 0, _pad1: [0; 3], data1: [0.0;4], data2: [0.0;4] };
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
            _pad2: [0; 2],
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
            0, // instance_count
            0, // first_index
            0, // base_vertex
            0, // first_instance
        ];
        
        let indirect_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Culling Indirect Buffer"),
            contents: bytemuck::cast_slice(&indirect_data),
            usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        let culled_boxes_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Culled Boxes Buffer"),
            size: (max_boxes as wgpu::BufferAddress) * std::mem::size_of::<GpuBox>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::VERTEX,
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
            colliders_buffer,
            awake_flags_buffer,
            pipelines,
            box_vertex_buffer,
            box_index_buffer,
            index_count: indices.len() as u32,
            
            readback_buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("GPU Physics Readback Buffer"),
                size: (max_boxes as wgpu::BufferAddress) * std::mem::size_of::<GpuBox>() as wgpu::BufferAddress,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            readback_state: Arc::new(AtomicU8::new(0)),
            
            indirect_buffer,
            culled_boxes_buffer,
        }
    }

    pub fn update_box(&self, queue: &wgpu::Queue, index: u32, box_struct: &GpuBox) {
        if index < self.max_boxes {
            let offset = (index as wgpu::BufferAddress) * std::mem::size_of::<GpuBox>() as wgpu::BufferAddress;
            queue.write_buffer(&self.boxes_buffer, offset, bytemuck::cast_slice(&[*box_struct]));
        }
    }

    pub fn update_collider(&self, queue: &wgpu::Queue, index: u32, collider: &GpuCollider) {
        if index < 100 {
            let offset = (index as wgpu::BufferAddress) * std::mem::size_of::<GpuCollider>() as wgpu::BufferAddress;
            queue.write_buffer(&self.colliders_buffer, offset, bytemuck::cast_slice(&[*collider]));
        }
    }

    pub fn compute_pass(&self, encoder: &mut wgpu::CommandEncoder) {
        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Physics Compute Pass"),
            timestamp_writes: None,
        });
        cpass.set_bind_group(0, &self.pipelines.compute_bind_group, &[]);

        let num_iterations = 4;
        for _ in 0..num_iterations {
            cpass.set_pipeline(&self.pipelines.pipeline_clear);
            cpass.dispatch_workgroups(self.grid_size.div_ceil(256), 1, 1);

            cpass.set_pipeline(&self.pipelines.pipeline_build);
            cpass.dispatch_workgroups(self.max_boxes.div_ceil(256), 1, 1);

            cpass.set_pipeline(&self.pipelines.pipeline_solve);
            cpass.dispatch_workgroups(self.max_boxes.div_ceil(256), 1, 1);
        }

        cpass.set_pipeline(&self.pipelines.pipeline_integrate);
        cpass.dispatch_workgroups(self.max_boxes.div_ceil(256), 1, 1);
    }

    pub fn cull_pass(&self, encoder: &mut wgpu::CommandEncoder, global_bind_group: &wgpu::BindGroup) {
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
        rpass.set_index_buffer(
            self.box_index_buffer.slice(..),
            wgpu::IndexFormat::Uint32,
        );
        rpass.draw_indexed_indirect(&self.indirect_buffer, 0);
    }

    pub fn request_readback(&self, encoder: &mut wgpu::CommandEncoder) {
        if self.readback_state.compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
            let size = (self.max_boxes as wgpu::BufferAddress) * std::mem::size_of::<GpuBox>() as wgpu::BufferAddress;
            encoder.copy_buffer_to_buffer(
                &self.boxes_buffer,
                0,
                &self.readback_buffer,
                0,
                size,
            );
        }
    }

    pub fn poll_readback_data(&self, device: &wgpu::Device) -> Option<Vec<GpuBox>> {
        if self.readback_state.compare_exchange(1, 2, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
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
        ([0.0, 0.0, 1.0],  [[-s,-s, s], [ s,-s, s], [ s, s, s], [-s, s, s]]),
        ([0.0, 0.0,-1.0],  [[ s,-s,-s], [-s,-s,-s], [-s, s,-s], [ s, s,-s]]),
        ([1.0, 0.0, 0.0],  [[ s,-s, s], [ s,-s,-s], [ s, s,-s], [ s, s, s]]),
        ([-1.0,0.0, 0.0],  [[-s,-s,-s], [-s,-s, s], [-s, s, s], [-s, s,-s]]),
        ([0.0, 1.0, 0.0],  [[-s, s, s], [ s, s, s], [ s, s,-s], [-s, s,-s]]),
        ([0.0,-1.0, 0.0],  [[-s,-s,-s], [ s,-s,-s], [ s,-s, s], [-s,-s, s]]),
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
