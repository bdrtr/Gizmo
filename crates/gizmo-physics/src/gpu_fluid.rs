use std::sync::Arc;
use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct FluidParticle {
    pub position: [f32; 3],
    pub density: f32,
    pub velocity: [f32; 3],
    pub pressure: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct FluidParams {
    pub dt: f32,
    pub gravity: [f32; 3],
    pub particle_radius: f32,
    pub smoothing_radius: f32,
    pub target_density: f32,
    pub pressure_multiplier: f32,
    pub viscosity: f32,
    pub num_particles: u32,
    pub grid_size_x: u32,
    pub grid_size_y: u32,
    pub grid_size_z: u32,
    pub cell_size: f32,
}

pub struct GpuFluidCompute {
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    
    pub pipeline_clear: wgpu::ComputePipeline,
    pub pipeline_count: wgpu::ComputePipeline,
    pub pipeline_sort: wgpu::ComputePipeline,
    pub pipeline_density: wgpu::ComputePipeline,
    pub pipeline_forces: wgpu::ComputePipeline,
    pub pipeline_integrate: wgpu::ComputePipeline,
    
    pub bind_group_layout: wgpu::BindGroupLayout,
    
    pub particles_buffer: Option<wgpu::Buffer>,
    pub cell_counts_buffer: Option<wgpu::Buffer>,
    pub cell_offsets_buffer: Option<wgpu::Buffer>,
    pub sorted_indices_buffer: Option<wgpu::Buffer>,
    pub forces_buffer: Option<wgpu::Buffer>,
    pub params_buffer: Option<wgpu::Buffer>,
    
    pub staging_buffer: Option<wgpu::Buffer>,
    pub readback_state: Arc<std::sync::atomic::AtomicU8>,
    
    pub max_particles: usize,
    pub grid_size: (u32, u32, u32),
}

impl GpuFluidCompute {
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
                    label: Some("Gizmo Fluid Compute Device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                },
                None,
            )
            .await
            .ok()?;

        let shader = device.create_shader_module(wgpu::include_wgsl!("fluid.wgsl"));

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Fluid Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 4, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 5, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Fluid Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let device = Arc::new(device);
        let queue = Arc::new(queue);

        let create_pipeline = |entry: &str, device: &wgpu::Device, shader: &wgpu::ShaderModule, pipeline_layout: &wgpu::PipelineLayout| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(&format!("Fluid {} Pipeline", entry)),
                layout: Some(pipeline_layout),
                module: shader,
                entry_point: entry,
                compilation_options: Default::default(),
            })
        };

        Some(Self {
            pipeline_clear: create_pipeline("clear_counts", &device, &shader, &pipeline_layout),
            pipeline_count: create_pipeline("count_particles", &device, &shader, &pipeline_layout),
            pipeline_sort: create_pipeline("sort_particles", &device, &shader, &pipeline_layout),
            pipeline_density: create_pipeline("compute_density", &device, &shader, &pipeline_layout),
            pipeline_forces: create_pipeline("compute_forces", &device, &shader, &pipeline_layout),
            pipeline_integrate: create_pipeline("integrate", &device, &shader, &pipeline_layout),
            device,
            queue,
            bind_group_layout,
            particles_buffer: None,
            cell_counts_buffer: None,
            cell_offsets_buffer: None,
            sorted_indices_buffer: None,
            forces_buffer: None,
            params_buffer: None,
            staging_buffer: None,
            readback_state: Arc::new(std::sync::atomic::AtomicU8::new(0)),
            max_particles: 0,
            grid_size: (32, 32, 32),
        })
    }

    pub fn step_fluid(&mut self, particles: &mut Vec<FluidParticle>, dt: f32, gravity: [f32; 3]) {
        if particles.is_empty() { return; }

        let num_particles = particles.len() as u32;
        let grid_cells = self.grid_size.0 * self.grid_size.1 * self.grid_size.2;

        let params = FluidParams {
            dt,
            gravity,
            particle_radius: 0.1,
            smoothing_radius: 0.2,
            target_density: 1000.0,
            pressure_multiplier: 100.0,
            viscosity: 0.01,
            num_particles,
            grid_size_x: self.grid_size.0,
            grid_size_y: self.grid_size.1,
            grid_size_z: self.grid_size.2,
            cell_size: 0.2,
        };

        // Resize buffers if necessary
        if self.max_particles < num_particles as usize || self.particles_buffer.is_none() {
            let capacity = (num_particles as usize).max(self.max_particles * 2).max(1024);
            self.max_particles = capacity;

            self.particles_buffer = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Particles Buffer"),
                size: (capacity * std::mem::size_of::<FluidParticle>()) as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }));
            
            self.sorted_indices_buffer = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Sorted Indices Buffer"),
                size: (capacity * 4) as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));

            self.forces_buffer = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Forces Buffer"),
                size: (capacity * 12) as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));

            self.staging_buffer = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Staging Buffer"),
                size: (capacity * std::mem::size_of::<FluidParticle>()) as u64,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }

        if self.cell_counts_buffer.is_none() {
            let cells_size = (grid_cells * 4) as u64;
            self.cell_counts_buffer = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Cell Counts Buffer"),
                size: cells_size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }));

            self.cell_offsets_buffer = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Cell Offsets Buffer"),
                size: cells_size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));

            self.params_buffer = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Params Buffer"),
                size: std::mem::size_of::<FluidParams>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }

        // 1. Upload initial data
        self.queue.write_buffer(self.particles_buffer.as_ref().unwrap(), 0, bytemuck::cast_slice(particles));
        self.queue.write_buffer(self.params_buffer.as_ref().unwrap(), 0, bytemuck::cast_slice(&[params]));

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Fluid Bind Group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.particles_buffer.as_ref().unwrap().as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: self.cell_counts_buffer.as_ref().unwrap().as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: self.cell_offsets_buffer.as_ref().unwrap().as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: self.sorted_indices_buffer.as_ref().unwrap().as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: self.forces_buffer.as_ref().unwrap().as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: self.params_buffer.as_ref().unwrap().as_entire_binding() },
            ],
        });

        // 2. Clear counts & count particles
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: None, timestamp_writes: None });
            cpass.set_bind_group(0, &bind_group, &[]);
            
            cpass.set_pipeline(&self.pipeline_clear);
            cpass.dispatch_workgroups((grid_cells + 255) / 256, 1, 1);
            
            cpass.set_pipeline(&self.pipeline_count);
            cpass.dispatch_workgroups((num_particles + 255) / 256, 1, 1);
        }

        // Create a temporary buffer to read back counts
        let counts_staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Counts Staging"),
            size: (grid_cells * 4) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        encoder.copy_buffer_to_buffer(self.cell_counts_buffer.as_ref().unwrap(), 0, &counts_staging, 0, (grid_cells * 4) as u64);
        self.queue.submit(Some(encoder.finish()));

        // 3. Read back counts & compute prefix sum
        let slice = counts_staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |v| { let _ = tx.send(v); });
        self.device.poll(wgpu::Maintain::Wait);
        if rx.recv().is_err() { return; }

        let mapped = slice.get_mapped_range();
        let counts: &[u32] = bytemuck::cast_slice(&mapped);
        
        let mut offsets = vec![0u32; grid_cells as usize];
        let mut sum = 0;
        for i in 0..grid_cells as usize {
            offsets[i] = sum;
            sum += counts[i];
        }
        
        drop(mapped);
        counts_staging.unmap();

        // 4. Upload offsets and zero out counts again for sorting
        self.queue.write_buffer(self.cell_offsets_buffer.as_ref().unwrap(), 0, bytemuck::cast_slice(&offsets));
        self.queue.write_buffer(self.cell_counts_buffer.as_ref().unwrap(), 0, bytemuck::cast_slice(&vec![0u32; grid_cells as usize]));

        // 5. Sort, Density, Forces, Integrate
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: None, timestamp_writes: None });
            cpass.set_bind_group(0, &bind_group, &[]);
            
            let wg_particles = (num_particles + 255) / 256;
            
            cpass.set_pipeline(&self.pipeline_sort);
            cpass.dispatch_workgroups(wg_particles, 1, 1);
            
            cpass.set_pipeline(&self.pipeline_density);
            cpass.dispatch_workgroups(wg_particles, 1, 1);
            
            cpass.set_pipeline(&self.pipeline_forces);
            cpass.dispatch_workgroups(wg_particles, 1, 1);
            
            cpass.set_pipeline(&self.pipeline_integrate);
            cpass.dispatch_workgroups(wg_particles, 1, 1);
        }

        // 6. Read back particles
        let bytes = (num_particles as usize * std::mem::size_of::<FluidParticle>()) as u64;
        
        if self.readback_state.compare_exchange(0, 1, std::sync::atomic::Ordering::SeqCst, std::sync::atomic::Ordering::SeqCst).is_ok() {
            encoder.copy_buffer_to_buffer(self.particles_buffer.as_ref().unwrap(), 0, self.staging_buffer.as_ref().unwrap(), 0, bytes);
            self.queue.submit(Some(encoder.finish()));
            
            let staging = self.staging_buffer.as_ref().unwrap();
            let slice = staging.slice(0..bytes);
            let state_clone = self.readback_state.clone();
            slice.map_async(wgpu::MapMode::Read, move |result| {
                if result.is_ok() {
                    state_clone.store(3, std::sync::atomic::Ordering::SeqCst);
                } else {
                    state_clone.store(0, std::sync::atomic::Ordering::SeqCst);
                }
            });
        } else {
            self.queue.submit(Some(encoder.finish()));
        }

        self.device.poll(wgpu::Maintain::Poll);

        if self.readback_state.load(std::sync::atomic::Ordering::SeqCst) == 3 {
            let staging = self.staging_buffer.as_ref().unwrap();
            let slice = staging.slice(0..bytes);
            {
                let mapped = slice.get_mapped_range();
                particles.copy_from_slice(bytemuck::cast_slice(&mapped));
            }
            staging.unmap();
            self.readback_state.store(0, std::sync::atomic::Ordering::SeqCst);
        }
    }
}
