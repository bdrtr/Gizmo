use wgpu::util::DeviceExt;
use crate::gpu_types::Vertex;
use super::types::*;
use super::pipeline::{create_fluid_pipelines, FluidPipelines};

pub struct GpuFluidSystem {
    pub num_particles: u32,
    pub particles_buffer: wgpu::Buffer,
    pub params_buffer: wgpu::Buffer,
    pub grid_buffer: wgpu::Buffer,
    pub colliders_buffer: wgpu::Buffer,
    pub sort_buffer: wgpu::Buffer,
    pub sort_params_buffer: wgpu::Buffer,
    
    pub pipelines: FluidPipelines,
    
    pub mesh_vertices: wgpu::Buffer,
    pub index_count: u32,
    pub vertex_count: u32,
}

impl GpuFluidSystem {
    pub fn new(
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        num_particles: u32,
        global_bind_group_layout: &wgpu::BindGroupLayout,
        output_format: wgpu::TextureFormat,
    ) -> Self {
        // 3D SPH - XZ düzleminde hacimli bir su bloğu
        let mut initial_particles = Vec::with_capacity(num_particles as usize);
        let spacing = 0.5_f32;
        
        let cols_x = 30u32;
        let cols_z = 30u32;
        let mut i = 0u32;
        
        while i < num_particles {
            let ix = (i % cols_x) as f32;
            let iz = ((i / cols_x) % cols_z) as f32;
            let iy = (i / (cols_x * cols_z)) as f32; // Y ekseninde yükseliyor
            
            let jx = ((i * 7 + 3) % 5) as f32 * 0.01 - 0.025;
            let jy = ((i * 13 + 7) % 5) as f32 * 0.01 - 0.025;
            let jz = ((i * 17 + 11) % 5) as f32 * 0.01 - 0.025;
            
            initial_particles.push(FluidParticle {
                position: [
                    -7.5 + ix * spacing + jx,
                    5.0 + iy * spacing + jy,
                    -7.5 + iz * spacing + jz,
                ],
                density: 1000.0,
                velocity: [0.0, 0.0, 0.0],
                pressure: 0.0,
                phase: 0,
                pad1: 0,
                pad2: 0,
                pad3: 0,
            });
            i += 1;
        }

        let particles_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Fluid Particles Buffer"),
            contents: bytemuck::cast_slice(&initial_particles),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        // Gerçek 3D Tank boyutları
        let bounds_min = [-15.0, 0.0, -15.0];
        let bounds_max = [15.0, 100.0, 15.0];
        let cell_size = 1.0_f32; // Standart smoothing radius

        let grid_size_x = f32::ceil((bounds_max[0] - bounds_min[0]) / cell_size) as u32;
        let grid_size_y = f32::ceil((bounds_max[1] - bounds_min[1]) / cell_size) as u32;
        let grid_size_z = f32::ceil((bounds_max[2] - bounds_min[2]) / cell_size) as u32;
        let total_cells = grid_size_x * grid_size_y * grid_size_z;

        // Initialize grid buffer (2 x u32 per cell: start_index, count)
        let grid_initial = vec![0_u32; (total_cells * 2) as usize];
        let grid_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Fluid Grid Buffer"),
            contents: bytemuck::cast_slice(&grid_initial),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        // Initialize sort buffer (ParticleHash: hash, index)
        // Bitonic Sort requires a power of two buffer size!
        let sort_capacity = num_particles.next_power_of_two() as usize;
        let mut sort_initial = Vec::with_capacity(sort_capacity);
        for i in 0..num_particles {
            sort_initial.push(ParticleHash { hash: 0, index: i });
        }
        // Pad with MAX hash so they sort to the very end
        for _ in num_particles..sort_capacity as u32 {
            sort_initial.push(ParticleHash { hash: 0xFFFFFFFF, index: 0 });
        }
        let sort_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Fluid Sort Buffer"),
            contents: bytemuck::cast_slice(&sort_initial),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        // Initialize sort params buffer
        let sort_params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Fluid Sort Params Buffer"),
            contents: bytemuck::cast_slice(&[SortParams { j: 0, k: 0 }]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let params = FluidParams {
            dt: 1.0 / 60.0,
            gravity: 9.81,
            rest_density: 1000.0,
            gas_constant: 10.0,   // Tait Equation (3D) requires smaller stiffness constant to avoid explosion
            viscosity: 0.1,         // 3D viscosity tuning
            mass: 125.0,            // 3D Volume mass = density * spacing^3 = 1000 * (0.5^3) = 125.0
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

        let pipelines = create_fluid_pipelines(
            device,
            global_bind_group_layout,
            output_format,
            &params_buffer,
            &particles_buffer,
            &grid_buffer,
            &colliders_buffer,
            &sort_buffer,
            &sort_params_buffer,
        );

        // Simple Sphere for rendering (Smaller)
        let sphere_mesh = crate::asset::AssetManager::create_sphere(device, 0.25, 12, 12);
        let mesh_vertices = wgpu::util::DeviceExt::create_buffer_init(device, &wgpu::util::BufferInitDescriptor {
            label: Some("Fluid Sphere Verts"),
            contents: bytemuck::cast_slice(&alloc_sphere_verts(0.25, 12, 12)),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Self {
            num_particles, particles_buffer, params_buffer, grid_buffer, colliders_buffer, sort_buffer, sort_params_buffer,
            pipelines,
            mesh_vertices, index_count: 0, vertex_count: sphere_mesh.vertex_count,
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

    pub fn compute_pass(&self, encoder: &mut wgpu::CommandEncoder, queue: &wgpu::Queue) {
        let workgroups_parts = self.num_particles.div_ceil(64);
        let workgroups_cells = u32::div_ceil(100 * 100 * 10, 64); // Safe over-approximation for clearing

        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Fluid Compute Pass"), timestamp_writes: None,
        });
        
        cpass.set_bind_group(0, &self.pipelines.compute_bind_group, &[]);
        
        cpass.set_pipeline(&self.pipelines.pipeline_hash);
        cpass.dispatch_workgroups(workgroups_parts, 1, 1);
        
        // Bitonic Sort Dispatch
        cpass.set_pipeline(&self.pipelines.pipeline_sort);
        let num_elements = self.num_particles.next_power_of_two();
        
        // O(log^2 N) bitonic sort passes
        let mut k = 2u32;
        while k <= num_elements {
            let mut j = k >> 1;
            while j > 0 {
                drop(cpass);
                queue.write_buffer(&self.sort_params_buffer, 0, bytemuck::cast_slice(&[SortParams { j, k }]));
                
                cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Fluid Sort Pass"), timestamp_writes: None,
                });
                cpass.set_bind_group(0, &self.pipelines.compute_bind_group, &[]);
                cpass.set_pipeline(&self.pipelines.pipeline_sort);
                cpass.dispatch_workgroups(num_elements.div_ceil(64), 1, 1);
                
                j >>= 1;
            }
            k <<= 1;
        }
        
        cpass.set_pipeline(&self.pipelines.pipeline_clear);
        cpass.dispatch_workgroups(workgroups_cells, 1, 1);
        
        cpass.set_pipeline(&self.pipelines.pipeline_offsets);
        cpass.dispatch_workgroups(workgroups_parts, 1, 1);
        
        cpass.set_pipeline(&self.pipelines.pipeline_density);
        cpass.dispatch_workgroups(workgroups_parts, 1, 1);
        
        cpass.set_pipeline(&self.pipelines.pipeline_integrate);
        cpass.dispatch_workgroups(workgroups_parts, 1, 1);
    }

    pub fn render_pass<'a>(&'a self, rpass: &mut wgpu::RenderPass<'a>, global_scene_bind_group: &'a wgpu::BindGroup) {
        rpass.set_pipeline(&self.pipelines.render_pipeline);
        rpass.set_bind_group(0, global_scene_bind_group, &[]);
        rpass.set_bind_group(1, &self.pipelines.render_bind_group, &[]);
        
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
