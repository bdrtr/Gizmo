use wgpu::util::DeviceExt;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuBox {
    pub position: [f32; 3],
    pub mass: f32,
    pub velocity: [f32; 3],
    pub state: u32,
    pub rotation: [f32; 4],
    pub angular_velocity: [f32; 3],
    pub sleep_counter: u32,
    pub color: [f32; 4],
    pub half_extents: [f32; 3],
    pub _pad: u32,
}

impl GpuBox {
    pub fn desc<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuBox>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 6,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 16,
                    shader_location: 7,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 32,
                    shader_location: 8,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 48,
                    shader_location: 9,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 64,
                    shader_location: 10,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 80,
                    shader_location: 11,
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuCollider {
    pub shape_type: u32, // 0 = AABB, 1 = Plane
    pub _pad1: [u32; 3],
    pub data1: [f32; 4], // AABB: min, Plane: normal
    pub data2: [f32; 4], // AABB: max, Plane: [d, pad, pad, pad]
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PhysicsSimParams {
    pub dt: f32,
    pub _padding0: [u32; 3],
    pub _pad1: [f32; 3],
    pub _padding1: u32,
    pub gravity: [f32; 3],
    pub damping: f32,
    pub num_boxes: u32,
    pub num_colliders: u32,
    pub _pad2: [u32; 2],
}

pub struct GpuPhysicsSystem {
    pub max_boxes: u32,
    pub grid_size: u32,
    pub boxes_buffer: wgpu::Buffer,
    pub params_buffer: wgpu::Buffer,
    pub grid_heads_buffer: wgpu::Buffer,
    pub linked_nodes_buffer: wgpu::Buffer,
    pub colliders_buffer: wgpu::Buffer,
    pub awake_flags_buffer: wgpu::Buffer,

    pub pipeline_clear: wgpu::ComputePipeline,
    pub pipeline_build: wgpu::ComputePipeline,
    pub pipeline_solve: wgpu::ComputePipeline,
    pub pipeline_integrate: wgpu::ComputePipeline,

    pub compute_bind_group: wgpu::BindGroup,
    pub render_pipeline: wgpu::RenderPipeline,
    pub box_vertex_buffer: wgpu::Buffer,
    pub box_index_buffer: wgpu::Buffer,
    pub index_count: u32,

    pub readback_buffer: wgpu::Buffer,
    // 0 = Idle, 1 = Copied to buffer (awaiting map), 2 = Mapping, 3 = Mapped (ready to read)
    pub readback_state: Arc<AtomicU8>,
    
    pub indirect_buffer: wgpu::Buffer,
    pub culled_boxes_buffer: wgpu::Buffer,
    pub culling_bind_group: wgpu::BindGroup,
    pub pipeline_culling: wgpu::ComputePipeline,
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

        let colliders_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("GPU Static Colliders Buffer"),
            contents: bytemuck::cast_slice(&initial_colliders),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        // Uyandırma bayrakları (Her objeye 1 u32, başlangıç 0)
        let initial_awake_flags: Vec<u32> = vec![0; max_boxes as usize];
        let awake_flags_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("GPU Physics Awake Flags Buffer"),
            contents: bytemuck::cast_slice(&initial_awake_flags),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        let params = PhysicsSimParams {
            dt: 0.016,
            _padding0: [0; 3],
            _pad1: [0.0; 3],
            _padding1: 0,
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
        // int array initialized to -1 (though we will clear it in pass 1 anyway)
        let initial_heads = vec![-1i32; grid_size as usize];
        let grid_heads_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("GPU Physics Grid Heads Buffer"),
            contents: bytemuck::cast_slice(&initial_heads),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        // linked nodes, size = max_boxes
        let initial_nodes = vec![-1i32; max_boxes as usize];
        let linked_nodes_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("GPU Physics Linked Nodes Buffer"),
            contents: bytemuck::cast_slice(&initial_nodes),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        let compute_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        // params
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
                        // spheres
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        // grid_heads
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        // linked_nodes
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
                        // statik colliders
                        binding: 4,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        // awake_flags
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
                label: Some("physics_compute_layout"),
            });

        let compute_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &compute_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: boxes_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: grid_heads_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: linked_nodes_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: colliders_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: awake_flags_buffer.as_entire_binding(),
                },
            ],
            label: Some("physics_compute_bind_group"),
        });

        let compute_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Physics Compute Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("physics_compute.wgsl").into()),
        });

        let compute_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Physics Compute Pipeline Layout"),
                bind_group_layouts: &[&compute_bind_group_layout],
                push_constant_ranges: &[],
            });

        let pipeline_clear = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Physics Clear Pipeline"),
            layout: Some(&compute_pipeline_layout),
            module: &compute_shader,
            entry_point: "clear_grid",
        });

        let pipeline_build = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Physics Build Pipeline"),
            layout: Some(&compute_pipeline_layout),
            module: &compute_shader,
            entry_point: "build_grid",
        });

        let pipeline_solve = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Physics Solve Pipeline"),
            layout: Some(&compute_pipeline_layout),
            module: &compute_shader,
            entry_point: "solve_collisions_safe",
        });

        let pipeline_integrate = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Physics Integrate Pipeline"),
            layout: Some(&compute_pipeline_layout),
            module: &compute_shader,
            entry_point: "integrate",
        });

        let render_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Physics Render Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("physics_render.wgsl").into()),
        });

        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Physics Render Pipeline Layout"),
                bind_group_layouts: &[global_bind_group_layout],
                push_constant_ranges: &[],
            });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Physics Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &render_shader,
                entry_point: "vs_main",
                buffers: &[crate::gpu_types::Vertex::desc(), GpuBox::desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &render_shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: output_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
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

        let culling_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None },
                        count: None,
                    },
                ],
                label: Some("physics_culling_layout"),
            });

        let culling_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &culling_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: params_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: boxes_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: culled_boxes_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: indirect_buffer.as_entire_binding() },
            ],
            label: Some("physics_culling_bind_group"),
        });

        let culling_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Physics Culling Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("physics_culling.wgsl").into()),
        });

        let culling_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Physics Culling Pipeline Layout"),
                bind_group_layouts: &[global_bind_group_layout, &culling_bind_group_layout],
                push_constant_ranges: &[],
            });

        let pipeline_culling = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Physics Culling Pipeline"),
            layout: Some(&culling_pipeline_layout),
            module: &culling_shader,
            entry_point: "cull_main",
        });

        Self {
            max_boxes,
            grid_size,
            boxes_buffer,
            params_buffer,
            grid_heads_buffer,
            linked_nodes_buffer,
            colliders_buffer,
            awake_flags_buffer,
            pipeline_clear,
            pipeline_build,
            pipeline_solve,
            pipeline_integrate,
            compute_bind_group,
            render_pipeline,
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
            culling_bind_group,
            pipeline_culling,
        }
    }

    /// Update or Add a sphere at a specific index
    pub fn update_box(&self, queue: &wgpu::Queue, index: u32, box_struct: &GpuBox) {
        if index < self.max_boxes {
            let offset = (index as wgpu::BufferAddress) * std::mem::size_of::<GpuBox>() as wgpu::BufferAddress;
            queue.write_buffer(&self.boxes_buffer, offset, bytemuck::cast_slice(&[*box_struct]));
        }
    }

    /// Update or Add a static collider at a specific index
    pub fn update_collider(&self, queue: &wgpu::Queue, index: u32, collider: &GpuCollider) {
        // We only reserved 50 colliders usually. We need to be careful of max capacity.
        let offset = (index as wgpu::BufferAddress) * std::mem::size_of::<GpuCollider>() as wgpu::BufferAddress;
        queue.write_buffer(&self.colliders_buffer, offset, bytemuck::cast_slice(&[*collider]));
    }

    pub fn compute_pass(&self, encoder: &mut wgpu::CommandEncoder) {
        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Physics Compute Pass"),
            timestamp_writes: None,
        });
        cpass.set_bind_group(0, &self.compute_bind_group, &[]);

        // Pass 1: Integrate Positions (Prediction step)
        cpass.set_pipeline(&self.pipeline_integrate);
        cpass.dispatch_workgroups(self.max_boxes.div_ceil(256), 1, 1);

        // Jacobi Iterations for stability
        let num_iterations = 4;
        for _ in 0..num_iterations {
            // Pass 2: Clear Grid
            cpass.set_pipeline(&self.pipeline_clear);
            cpass.dispatch_workgroups(self.grid_size.div_ceil(256), 1, 1);

            // Pass 3: Build Grid Linked List
            cpass.set_pipeline(&self.pipeline_build);
            cpass.dispatch_workgroups(self.max_boxes.div_ceil(256), 1, 1);

            // Pass 4: Solve Collisions
            cpass.set_pipeline(&self.pipeline_solve);
            cpass.dispatch_workgroups(self.max_boxes.div_ceil(256), 1, 1);
        }
    }

    pub fn cull_pass(&self, encoder: &mut wgpu::CommandEncoder, global_bind_group: &wgpu::BindGroup) {
        encoder.clear_buffer(&self.indirect_buffer, 4, Some(4));

        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Physics Culling Pass"),
            timestamp_writes: None,
        });
        cpass.set_pipeline(&self.pipeline_culling);
        cpass.set_bind_group(0, global_bind_group, &[]);
        cpass.set_bind_group(1, &self.culling_bind_group, &[]);
        cpass.dispatch_workgroups(self.max_boxes.div_ceil(256), 1, 1);
    }

    pub fn render_pass<'a>(
        &'a self,
        rpass: &mut wgpu::RenderPass<'a>,
        global_bind_group: &'a wgpu::BindGroup,
    ) {
        rpass.set_pipeline(&self.render_pipeline);
        rpass.set_bind_group(0, global_bind_group, &[]);
        rpass.set_vertex_buffer(0, self.box_vertex_buffer.slice(..));
        rpass.set_vertex_buffer(1, self.culled_boxes_buffer.slice(..));
        rpass.set_index_buffer(
            self.box_index_buffer.slice(..),
            wgpu::IndexFormat::Uint32,
        );
        rpass.draw_indexed_indirect(&self.indirect_buffer, 0);
    }

    /// Asynchronously requests a readback of the GPU physics state to the CPU.
    pub fn request_readback(&self, encoder: &mut wgpu::CommandEncoder) {
        // Only request if Idle (0)
        if self.readback_state.compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
            let size = (self.max_boxes as wgpu::BufferAddress) * std::mem::size_of::<GpuBox>() as wgpu::BufferAddress;
            
            // Queue the copy from device to host readback buffer
            encoder.copy_buffer_to_buffer(
                &self.boxes_buffer,
                0,
                &self.readback_buffer,
                0,
                size,
            );
            // Notification: WGPU requires queue.submit BEFORE map_async on the same buffer can be used without failing
            // state becomes 1 ("Copied to buffer")
        }
    }

    /// Polls and unmaps the buffer. Should be called periodically on the CPU.
    pub fn poll_readback_data(&self, device: &wgpu::Device) -> Option<Vec<GpuBox>> {
        // If state is 1 (Copied), queue was already submitted in previous frame. Setup mapping.
        if self.readback_state.compare_exchange(1, 2, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
            let slice = self.readback_buffer.slice(..);
            let state_clone = self.readback_state.clone();
            slice.map_async(wgpu::MapMode::Read, move |result| {
                if result.is_ok() {
                    state_clone.store(3, Ordering::SeqCst); // Set to Mapped
                } else {
                    state_clone.store(0, Ordering::SeqCst); // Fallback to Idle
                }
            });
        }

        device.poll(wgpu::Maintain::Poll); // Advance map_async callbacks!

        if self.readback_state.load(Ordering::SeqCst) == 3 { // Mapped
            let slice = self.readback_buffer.slice(..);
            let view = slice.get_mapped_range();
            
            let data: &[GpuBox] = bytemuck::cast_slice(&view);
            let vec_data = data.to_vec();
            
            drop(view);
            self.readback_buffer.unmap();
            
            self.readback_state.store(0, Ordering::SeqCst); // Reset to Idle
            
            return Some(vec_data);
        }
        None
    }
}

fn create_cube() -> (Vec<crate::gpu_types::Vertex>, Vec<u32>) {
    let s = 1.0f32;
    let positions = [
        [-s, -s, s],
        [s, -s, s],
        [s, s, s],
        [-s, s, s],
        [-s, -s, -s],
        [s, -s, -s],
        [s, s, -s],
        [-s, s, -s],
    ];
    let indices_u32 = vec![
        0, 1, 2, 2, 3, 0, // front
        1, 5, 6, 6, 2, 1, // right
        5, 4, 7, 7, 6, 5, // back
        4, 0, 3, 3, 7, 4, // left
        3, 2, 6, 6, 7, 3, // top
        4, 5, 1, 1, 0, 4, // bottom
    ];

    let mut vertices = Vec::new();
    for p in positions {
        let sum: f32 = p[0] * p[0] + p[1] * p[1] + p[2] * p[2];
        let n_len = sum.sqrt();
        let n = [p[0] / n_len, p[1] / n_len, p[2] / n_len];
        vertices.push(crate::gpu_types::Vertex {
            position: p,
            color: [1.0, 1.0, 1.0],
            normal: n,
            tex_coords: [0.0, 0.0],
            joint_indices: [0; 4],
            joint_weights: [0.0; 4],
        });
    }

    (vertices, indices_u32)
}
