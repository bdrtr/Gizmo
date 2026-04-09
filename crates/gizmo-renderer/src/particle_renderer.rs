use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuParticle {
    pub position: [f32; 3],
    pub life: f32,
    pub velocity: [f32; 3],
    pub max_life: f32,
    pub color: [f32; 4],
    pub size_start: f32,
    pub size_end: f32,
    pub _padding: [f32; 2],
}

impl GpuParticle {
    pub fn desc<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuParticle>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute { offset: 0, shader_location: 0, format: wgpu::VertexFormat::Float32x4 }, // pos + life
                wgpu::VertexAttribute { offset: 16, shader_location: 1, format: wgpu::VertexFormat::Float32x4 }, // vel + max_life
                wgpu::VertexAttribute { offset: 32, shader_location: 2, format: wgpu::VertexFormat::Float32x4 }, // color
                wgpu::VertexAttribute { offset: 48, shader_location: 3, format: wgpu::VertexFormat::Float32x4 }, // sizes + padding
            ],
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ParticleSimParams {
    pub dt: f32,
    pub global_gravity: f32,
    pub global_drag: f32,
    pub _padding: f32,
}

pub struct GpuParticleSystem {
    pub max_particles: u32,
    pub particles_buffer: wgpu::Buffer,
    pub params_buffer: wgpu::Buffer,
    pub compute_pipeline: wgpu::ComputePipeline,
    pub compute_bind_group: wgpu::BindGroup,
    pub render_pipeline: wgpu::RenderPipeline,
    pub quad_vertex_buffer: wgpu::Buffer,
    pub active_particles: u32,
    pub ring_head: std::sync::atomic::AtomicU32, // CPU offset head
}

impl GpuParticleSystem {
    pub fn new(
        device: &wgpu::Device, 
        max_particles: u32,
        global_bind_group_layout: &wgpu::BindGroupLayout,
        output_format: wgpu::TextureFormat,
    ) -> Self {
        let mut initial_particles = Vec::with_capacity(max_particles as usize);
        for _ in 0..max_particles {
            initial_particles.push(GpuParticle {
                position: [0.0, 0.0, 0.0],
                life: 999.0, // Başlangıçta hepsi ÖLÜ
                velocity: [0.0, 0.0, 0.0],
                max_life: 0.1,
                color: [0.0, 0.0, 0.0, 0.0],
                size_start: 0.0, size_end: 0.0, _padding: [0.0; 2]
            });
        }

        let particles_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("GPU Particles Buffer"),
            contents: bytemuck::cast_slice(&initial_particles),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });

        let params = ParticleSimParams { dt: 0.0, global_gravity: 0.0, global_drag: 0.0, _padding: 0.0 };
        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("GPU Particle Params Buffer"),
            contents: bytemuck::cast_slice(&[params]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Compute Layout & Pipeline
        let compute_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
            label: Some("particle_compute_layout"),
        });

        let compute_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &compute_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: params_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: particles_buffer.as_entire_binding() },
            ],
            label: Some("particle_compute_bind_group"),
        });

        let compute_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Particle Compute Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("particle_compute.wgsl").into()),
        });

        let compute_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Particle Compute Pipeline Layout"),
            bind_group_layouts: &[&compute_bind_group_layout],
            push_constant_ranges: &[],
        });

        let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Particle Compute Pipeline"),
            layout: Some(&compute_pipeline_layout),
            module: &compute_shader,
            entry_point: "main",
        });

        // Render Pipeline
        let render_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Particle Render Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("particle_render.wgsl").into()),
        });

        let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Particle Render Pipeline Layout"),
            bind_group_layouts: &[global_bind_group_layout],
            push_constant_ranges: &[],
        });

        // Simple Quad (2 triangles for billboard)
        let quad_vertices: [[f32; 2]; 4] = [
            [-0.5, -0.5],
            [ 0.5, -0.5],
            [-0.5,  0.5],
            [ 0.5,  0.5],
        ];
        
        let quad_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Particle Quad Vertex Buffer"),
            contents: bytemuck::cast_slice(&quad_vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Particle Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &render_shader,
                entry_point: "vs_main",
                buffers: &[
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![4 => Float32x2], // Location 4 avoids conflict
                    },
                    GpuParticle::desc()
                ],
            },
            fragment: Some(wgpu::FragmentState {
                module: &render_shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: output_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING), // Alpha blending for particles
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip, // 4 vertices for a quad
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: false, // Particles don't write depth (soft particles)
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        Self {
            max_particles,
            particles_buffer,
            params_buffer,
            compute_pipeline,
            compute_bind_group,
            render_pipeline,
            quad_vertex_buffer,
            active_particles: max_particles,
            ring_head: std::sync::atomic::AtomicU32::new(0),
        }
    }

    pub fn update_params(&self, queue: &wgpu::Queue, dt: f32) {
        let params = ParticleSimParams { 
            dt, 
            global_gravity: 9.81, 
            global_drag: 0.8, 
            _padding: 0.0 
        };
        queue.write_buffer(&self.params_buffer, 0, bytemuck::cast_slice(&[params]));
    }

    pub fn spawn_particles(&self, queue: &wgpu::Queue, new_particles: &[GpuParticle]) {
        if new_particles.is_empty() { return; }

        let count = new_particles.len() as u32;
        let mut head = self.ring_head.fetch_add(count, std::sync::atomic::Ordering::Relaxed) % self.max_particles;
        
        let mut remaining = count;
        let mut offset = 0;

        while remaining > 0 {
            let to_write = remaining.min(self.max_particles - head);
            let slice = &new_particles[offset as usize..(offset + to_write) as usize];
            
            queue.write_buffer(
                &self.particles_buffer, 
                (head as usize * std::mem::size_of::<GpuParticle>()) as wgpu::BufferAddress, 
                bytemuck::cast_slice(slice)
            );

            head = (head + to_write) % self.max_particles;
            offset += to_write;
            remaining -= to_write;
        }
    }

    pub fn compute_pass(&self, encoder: &mut wgpu::CommandEncoder) {
        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Particle Compute Pass"),
            timestamp_writes: None,
        });
        cpass.set_pipeline(&self.compute_pipeline);
        cpass.set_bind_group(0, &self.compute_bind_group, &[]);
        let workgroups = self.max_particles.div_ceil(64);
        cpass.dispatch_workgroups(workgroups, 1, 1);
    }
}
