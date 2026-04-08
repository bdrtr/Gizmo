use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuSphere {
    pub position: [f32; 3],
    pub radius: f32,
    pub velocity: [f32; 3],
    pub mass: f32,
    pub color: [f32; 4],
    pub _padding: [f32; 4],
}

impl GpuSphere {
    pub fn desc<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuSphere>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute { offset: 0, shader_location: 6, format: wgpu::VertexFormat::Float32x4 },
                wgpu::VertexAttribute { offset: 16, shader_location: 7, format: wgpu::VertexFormat::Float32x4 },
                wgpu::VertexAttribute { offset: 32, shader_location: 8, format: wgpu::VertexFormat::Float32x4 },
            ],
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PhysicsSimParams {
    pub dt: f32,
    pub _pad1: [f32; 3],
    pub gravity: [f32; 3],
    pub damping: f32,
    pub num_spheres: u32,
    pub _pad2: [f32; 3],
}

pub struct GpuPhysicsSystem {
    pub max_spheres: u32,
    pub grid_size: u32,
    pub spheres_buffer: wgpu::Buffer,
    pub params_buffer: wgpu::Buffer,
    pub grid_heads_buffer: wgpu::Buffer,
    pub linked_nodes_buffer: wgpu::Buffer,
    
    pub pipeline_clear: wgpu::ComputePipeline,
    pub pipeline_build: wgpu::ComputePipeline,
    pub pipeline_solve: wgpu::ComputePipeline,
    pub pipeline_integrate: wgpu::ComputePipeline,
    
    pub compute_bind_group: wgpu::BindGroup,
    pub render_pipeline: wgpu::RenderPipeline,
    pub sphere_vertex_buffer: wgpu::Buffer,
    pub sphere_index_buffer: wgpu::Buffer,
    pub index_count: u32,
}

impl GpuPhysicsSystem {
    pub fn new(
        device: &wgpu::Device, 
        max_spheres: u32, 
        global_bind_group_layout: &wgpu::BindGroupLayout,
        output_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat
    ) -> Self {
        let mut initial_spheres = Vec::with_capacity(max_spheres as usize);
        for i in 0..max_spheres {
            let x = ((i as f32 * 13.0) % 40.0) - 20.0;
            let y = 50.0 + (i as f32 % 50.0) * 2.0; 
            let z = ((i as f32 * 23.0) % 40.0) - 20.0;
            
            initial_spheres.push(GpuSphere {
                position: [x, y, z],
                radius: 1.0,
                velocity: [0.0, 0.0, 0.0],
                mass: 1.0,
                color: [0.8, 0.2, 0.2, 1.0],
                _padding: [0.0; 4],
            });
        }

        let spheres_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("GPU Physics Buffer"),
            contents: bytemuck::cast_slice(&initial_spheres),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });

        let params = PhysicsSimParams { 
            dt: 0.016, 
            _pad1: [0.0; 3],
            gravity: [0.0, -9.81, 0.0], 
            damping: 0.99,
            num_spheres: max_spheres,
            _pad2: [0.0; 3],
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

        // linked nodes, size = max_spheres
        let initial_nodes = vec![-1i32; max_spheres as usize];
        let linked_nodes_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("GPU Physics Linked Nodes Buffer"),
            contents: bytemuck::cast_slice(&initial_nodes),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        let compute_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[
                wgpu::BindGroupLayoutEntry { // params
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { // spheres
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { // grid_heads
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { // linked_nodes
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
            label: Some("physics_compute_layout"),
        });

        let compute_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &compute_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: params_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: spheres_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: grid_heads_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: linked_nodes_buffer.as_entire_binding() },
            ],
            label: Some("physics_compute_bind_group"),
        });

        let compute_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Physics Compute Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("physics_compute.wgsl").into()),
        });

        let compute_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
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

        let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
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
                buffers: &[
                    crate::gpu_types::Vertex::desc(),
                    GpuSphere::desc()
                ],
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

        let (vertices, indices) = create_ico_sphere(2);
        
        let sphere_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Sphere Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        
        let sphere_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Sphere Index Buffer"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        Self {
            max_spheres,
            grid_size,
            spheres_buffer,
            params_buffer,
            grid_heads_buffer,
            linked_nodes_buffer,
            pipeline_clear,
            pipeline_build,
            pipeline_solve,
            pipeline_integrate,
            compute_bind_group,
            render_pipeline,
            sphere_vertex_buffer,
            sphere_index_buffer,
            index_count: indices.len() as u32,
        }
    }

    pub fn compute_pass(&self, encoder: &mut wgpu::CommandEncoder) {
        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Physics Compute Pass"),
            timestamp_writes: None,
        });
        cpass.set_bind_group(0, &self.compute_bind_group, &[]);
        
        // Pass 1: Clear
        cpass.set_pipeline(&self.pipeline_clear);
        cpass.dispatch_workgroups(self.grid_size.div_ceil(256), 1, 1);
        
        // Pass 2: Build Grid Linked List
        cpass.set_pipeline(&self.pipeline_build);
        cpass.dispatch_workgroups(self.max_spheres.div_ceil(256), 1, 1);
        
        // Pass 3: Solve Collisions
        cpass.set_pipeline(&self.pipeline_solve);
        cpass.dispatch_workgroups(self.max_spheres.div_ceil(256), 1, 1);
        
        // Pass 4: Integrate Positions
        cpass.set_pipeline(&self.pipeline_integrate);
        cpass.dispatch_workgroups(self.max_spheres.div_ceil(256), 1, 1);
    }
    
    pub fn render_pass<'a>(&'a self, rpass: &mut wgpu::RenderPass<'a>, global_bind_group: &'a wgpu::BindGroup) {
        rpass.set_pipeline(&self.render_pipeline);
        rpass.set_bind_group(0, global_bind_group, &[]);
        rpass.set_vertex_buffer(0, self.sphere_vertex_buffer.slice(..));
        rpass.set_vertex_buffer(1, self.spheres_buffer.slice(..));
        rpass.set_index_buffer(self.sphere_index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        rpass.draw_indexed(0..self.index_count, 0, 0..self.max_spheres);
    }
}

fn create_ico_sphere(subdivisions: u32) -> (Vec<crate::gpu_types::Vertex>, Vec<u32>) {
    let s = 1.0f32;
    let positions = [
        [-s, -s,  s], [ s, -s,  s], [ s,  s,  s], [-s,  s,  s],
        [-s, -s, -s], [ s, -s, -s], [ s,  s, -s], [-s,  s, -s],
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
        let sum: f32 = p[0]*p[0] + p[1]*p[1] + p[2]*p[2];
        let n_len = sum.sqrt();
        let n = [p[0]/n_len, p[1]/n_len, p[2]/n_len];
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
