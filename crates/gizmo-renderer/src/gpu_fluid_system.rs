use wgpu::util::DeviceExt;
use crate::gpu_types::Vertex;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FluidParticle {
    pub position: [f32; 3],
    pub density: f32,
    pub velocity: [f32; 3],
    pub pressure: f32,
    pub force: [f32; 3],
    pub next_index: i32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FluidCollider {
    pub position: [f32; 3],
    pub radius: f32,
    pub velocity: [f32; 3],
    pub padding: f32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FluidParams {
    pub dt: f32,
    pub gravity: f32,
    pub rest_density: f32,
    pub gas_constant: f32,
    pub viscosity: f32,
    pub mass: f32,
    pub smoothing_radius: f32,
    pub num_particles: u32,
    pub grid_size_x: u32,
    pub grid_size_y: u32,
    pub grid_size_z: u32,
    pub cell_size: f32,
    pub bounds_min: [f32; 3],
    pub bounds_padding1: f32,
    pub bounds_max: [f32; 3],
    pub bounds_padding2: f32,

    pub mouse_pos: [f32; 3],
    pub mouse_active: f32,
    pub mouse_dir: [f32; 3],
    pub mouse_radius: f32,
    
    pub num_colliders: u32,
    pub pad1: f32,
    pub pad2: f32,
    pub pad3: f32,
}

pub const MAX_FLUID_COLLIDERS: usize = 64;

pub struct GpuFluidSystem {
    pub num_particles: u32,
    pub particles_buffer: wgpu::Buffer,
    pub params_buffer: wgpu::Buffer,
    pub grid_buffer: wgpu::Buffer,
    pub colliders_buffer: wgpu::Buffer,
    
    // Compute pipelines
    pub compute_bind_group: wgpu::BindGroup,
    pub pipeline_clear: wgpu::ComputePipeline,
    pub pipeline_hash: wgpu::ComputePipeline,
    pub pipeline_density: wgpu::ComputePipeline,
    pub pipeline_forces: wgpu::ComputePipeline,
    pub pipeline_integrate: wgpu::ComputePipeline,
    
    // Render pipeline
    pub render_bind_group: wgpu::BindGroup,
    pub render_pipeline: wgpu::RenderPipeline,
    pub mesh_vertices: wgpu::Buffer,
    pub index_count: u32,
    pub vertex_count: u32,
}

impl GpuFluidSystem {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        num_particles: u32,
        global_bind_group_layout: &wgpu::BindGroupLayout,
        output_format: wgpu::TextureFormat,
    ) -> Self {
        // Build initial particles (e.g. water tank inside a frame)
        let mut initial_particles = Vec::with_capacity(num_particles as usize);
        let spacing = 0.5_f32;
        let mut x = -15.0;
        let mut y = 0.0;
        let mut z = -5.0; // Narrow Z so it looks like a 2.5D tank!
        let mut i = 0;
        
        // Spawn them closely packed
        while i < num_particles {
            let offset_x = (i % 2) as f32 * 0.1;
            let offset_z = (i % 3) as f32 * 0.1;
            initial_particles.push(FluidParticle {
                position: [x + offset_x, y + 2.0, z + offset_z],
                density: 1000.0,
                velocity: [0.0, 0.0, 0.0],
                pressure: 0.0,
                force: [0.0, 0.0, 0.0],
                next_index: -1,
            });
            i += 1;
            
            x += spacing;
            if x > 15.0 {
                x = -15.0;
                z += spacing;
                if z > 5.0 {
                    z = -5.0;
                    y += spacing;
                }
            }
        }

        let particles_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Fluid Particles Buffer"),
            contents: bytemuck::cast_slice(&initial_particles),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        // Grid hashing parameters
        // Example bounds: aquarium tank!
        let bounds_min = [-16.0, 0.0, -6.0];
        let bounds_max = [16.0, 100.0, 6.0];
        let cell_size = 1.0; // Smaller smooth radius

        let grid_size_x = f32::ceil((bounds_max[0] - bounds_min[0]) / cell_size) as u32;
        let grid_size_y = f32::ceil((bounds_max[1] - bounds_min[1]) / cell_size) as u32;
        let grid_size_z = f32::ceil((bounds_max[2] - bounds_min[2]) / cell_size) as u32;
        let total_cells = grid_size_x * grid_size_y * grid_size_z;

        // Initialize grid buffer with -1
        let mut grid_initial = vec![-1_i32; total_cells as usize];
        let grid_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Fluid Grid Buffer"),
            contents: bytemuck::cast_slice(&grid_initial),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        let params = FluidParams {
            dt: 1.0 / 60.0, // base dt but we'll update it
            gravity: 9.81,
            rest_density: 1000.0,
            gas_constant: 2000.0,
            viscosity: 0.1, // Increased viscosity for more water-like behavior
            mass: 1.0,
            smoothing_radius: cell_size,
            num_particles,
            grid_size_x,
            grid_size_y,
            grid_size_z,
            cell_size,
            bounds_min,
            bounds_padding1: 0.0,
            bounds_max,
            bounds_padding2: 0.0,
            mouse_pos: [0.0; 3],
            mouse_active: 0.0,
            mouse_dir: [0.0; 3],
            mouse_radius: 5.0,
            num_colliders: 0,
            pad1: 0.0,
            pad2: 0.0,
            pad3: 0.0,
        };
        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Fluid Params Buffer"),
            contents: bytemuck::cast_slice(&[params]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Initialize Colliders buffer
        let empty_colliders = vec![FluidCollider { position: [0.0; 3], radius: 0.0, velocity: [0.0; 3], padding: 0.0 }; MAX_FLUID_COLLIDERS];
        let colliders_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Fluid Colliders Buffer"),
            contents: bytemuck::cast_slice(&empty_colliders),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        // Compute Layout
        let compute_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
                    wgpu::BindGroupLayoutEntry {
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
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
                label: Some("fluid_compute_layout"),
            });

        let compute_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &compute_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: params_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: particles_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: grid_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: colliders_buffer.as_entire_binding() },
            ],
            label: Some("fluid_compute_bg"),
        });

        let compute_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Fluid Compute Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("fluid_compute.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Fluid Compute Pipeline Layout"),
            bind_group_layouts: &[&compute_bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline_clear = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("fluid_clear"), layout: Some(&pipeline_layout), module: &compute_shader, entry_point: "clear_grid",
        });
        let pipeline_hash = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("fluid_hash"), layout: Some(&pipeline_layout), module: &compute_shader, entry_point: "hash_particles",
        });
        let pipeline_density = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("fluid_density"), layout: Some(&pipeline_layout), module: &compute_shader, entry_point: "calc_density",
        });
        let pipeline_forces = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("fluid_forces"), layout: Some(&pipeline_layout), module: &compute_shader, entry_point: "calc_forces",
        });
        let pipeline_integrate = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("fluid_integrate"), layout: Some(&pipeline_layout), module: &compute_shader, entry_point: "integrate",
        });

        // Setup Render pipeline
        let render_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Fluid Render BG Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                }
            ],
        });
        let render_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Fluid Render BG"),
            layout: &render_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 1, resource: particles_buffer.as_entire_binding() },
            ],
        });

        let render_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Fluid Render Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("fluid_render.wgsl").into()),
        });

        let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Fluid Render Pipeline Layout"),
            bind_group_layouts: &[global_bind_group_layout, &render_bind_group_layout],
            push_constant_ranges: &[],
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Fluid Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &render_shader, entry_point: "vs_main",
                buffers: &[Vertex::desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &render_shader, entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: output_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
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
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(), multiview: None,
        });

        // Simple Sphere for rendering (Smaller)
        let sphere_mesh = crate::asset::AssetManager::create_sphere(device, 0.25, 12, 12);
        let mesh_vertices = wgpu::util::DeviceExt::create_buffer_init(device, &wgpu::util::BufferInitDescriptor {
            label: Some("Fluid Sphere Verts"),
            contents: bytemuck::cast_slice(&alloc_sphere_verts(0.25, 12, 12)),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Self {
            num_particles, particles_buffer, params_buffer, grid_buffer, colliders_buffer,
            compute_bind_group, pipeline_clear, pipeline_hash, pipeline_density, pipeline_forces, pipeline_integrate,
            render_bind_group, render_pipeline, mesh_vertices, index_count: 0, vertex_count: sphere_mesh.vertex_count,
        }
    }

    pub fn update_parameters(
        &self,
        queue: &wgpu::Queue,
        mouse_pos: [f32; 3],
        mouse_dir: [f32; 3],
        mouse_active: bool,
        colliders: &[FluidCollider],
    ) {
        // Upload dynamic colliders
        let num_colliders = (colliders.len().min(MAX_FLUID_COLLIDERS)) as u32;
        if num_colliders > 0 {
            queue.write_buffer(&self.colliders_buffer, 0, bytemuck::cast_slice(&colliders[0..num_colliders as usize]));
        }

        // We only overwrite the changing parts or write a fresh struct. Let's do a fast partial write using offsets...
        // Actually, let's just write the second half of the struct!
        // params_buffer has an exact layout. We know mouse_pos starts at 80 (since 16 * 4 + 16 bytes = 80).
        // Let's create a struct just for the update:
        #[repr(C)]
        #[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
        struct DynamicFluidParams {
            mouse_pos: [f32; 3],
            mouse_active: f32,
            mouse_dir: [f32; 3],
            mouse_radius: f32,
            num_colliders: u32,
            pad1: f32,
            pad2: f32,
            pad3: f32,
        }

        let dyn_params = DynamicFluidParams {
            mouse_pos,
            mouse_active: if mouse_active { 1.0 } else { 0.0 },
            mouse_dir,
            mouse_radius: 10.0, // Large mouse influence
            num_colliders,
            pad1: 0.0,
            pad2: 0.0,
            pad3: 0.0,
        };

        queue.write_buffer(&self.params_buffer, 80, bytemuck::cast_slice(&[dyn_params]));
    }

    pub fn compute_pass(&self, encoder: &mut wgpu::CommandEncoder) {
        let workgroups_parts = self.num_particles.div_ceil(64);
        let workgroups_cells = u32::div_ceil(50 * 100 * 50, 64); // approximate

        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Fluid Compute Pass"), timestamp_writes: None,
        });
        
        cpass.set_bind_group(0, &self.compute_bind_group, &[]);
        
        cpass.set_pipeline(&self.pipeline_clear);
        cpass.dispatch_workgroups(workgroups_cells, 1, 1);
        
        cpass.set_pipeline(&self.pipeline_hash);
        cpass.dispatch_workgroups(workgroups_parts, 1, 1);
        
        cpass.set_pipeline(&self.pipeline_density);
        cpass.dispatch_workgroups(workgroups_parts, 1, 1);
        
        cpass.set_pipeline(&self.pipeline_forces);
        cpass.dispatch_workgroups(workgroups_parts, 1, 1);
        
        cpass.set_pipeline(&self.pipeline_integrate);
        cpass.dispatch_workgroups(workgroups_parts, 1, 1);
    }

    pub fn render_pass<'a>(&'a self, rpass: &mut wgpu::RenderPass<'a>, global_scene_bind_group: &'a wgpu::BindGroup) {
        rpass.set_pipeline(&self.render_pipeline);
        rpass.set_bind_group(0, global_scene_bind_group, &[]);
        rpass.set_bind_group(1, &self.render_bind_group, &[]);
        
        rpass.set_vertex_buffer(0, self.mesh_vertices.slice(..));
        rpass.draw(0..self.vertex_count, 0..self.num_particles);
    }
}

// Temporary workaround since we can't easily extract VBuf arrays from Arc<Buffer>
fn alloc_sphere_verts(radius: f32, stacks: u32, slices: u32) -> Vec<Vertex> {
    let mut vertices = Vec::new();
    let pi = std::f32::consts::PI;

    for i in 0..stacks {
        let theta1 = (i as f32 / stacks as f32) * pi;
        let theta2 = ((i + 1) as f32 / stacks as f32) * pi;
        for j in 0..slices {
            let phi1 = (j as f32 / slices as f32) * 2.0 * pi;
            let phi2 = ((j + 1) as f32 / slices as f32) * 2.0 * pi;
            let p1 = [radius * theta1.sin() * phi1.cos(), radius * theta1.cos(), radius * theta1.sin() * phi1.sin()];
            let p2 = [radius * theta2.sin() * phi1.cos(), radius * theta2.cos(), radius * theta2.sin() * phi1.sin()];
            let p3 = [radius * theta2.sin() * phi2.cos(), radius * theta2.cos(), radius * theta2.sin() * phi2.sin()];
            let p4 = [radius * theta1.sin() * phi2.cos(), radius * theta1.cos(), radius * theta1.sin() * phi2.sin()];
            let n1 = [theta1.sin() * phi1.cos(), theta1.cos(), theta1.sin() * phi1.sin()];
            let n2 = [theta2.sin() * phi1.cos(), theta2.cos(), theta2.sin() * phi1.sin()];
            let n3 = [theta2.sin() * phi2.cos(), theta2.cos(), theta2.sin() * phi2.sin()];
            let n4 = [theta1.sin() * phi2.cos(), theta1.cos(), theta1.sin() * phi2.sin()];
            
            vertices.push(Vertex { position: p1, color: [1.0; 3], normal: n1, tex_coords: [0.0; 2], joint_indices: [0;4], joint_weights: [0.0;4] });
            vertices.push(Vertex { position: p2, color: [1.0; 3], normal: n2, tex_coords: [0.0; 2], joint_indices: [0;4], joint_weights: [0.0;4] });
            vertices.push(Vertex { position: p3, color: [1.0; 3], normal: n3, tex_coords: [0.0; 2], joint_indices: [0;4], joint_weights: [0.0;4] });
            vertices.push(Vertex { position: p1, color: [1.0; 3], normal: n1, tex_coords: [0.0; 2], joint_indices: [0;4], joint_weights: [0.0;4] });
            vertices.push(Vertex { position: p3, color: [1.0; 3], normal: n3, tex_coords: [0.0; 2], joint_indices: [0;4], joint_weights: [0.0;4] });
            vertices.push(Vertex { position: p4, color: [1.0; 3], normal: n4, tex_coords: [0.0; 2], joint_indices: [0;4], joint_weights: [0.0;4] });
        }
    }
    vertices
}
