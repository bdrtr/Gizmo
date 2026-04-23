use wgpu::util::DeviceExt;

use super::pipeline::{create_particle_pipelines, ParticlePipelines};
use super::types::*;

pub struct GpuParticleSystem {
    pub max_particles: u32,
    pub particles_buffer: wgpu::Buffer,
    pub params_buffer: wgpu::Buffer,
    pub pipelines: ParticlePipelines,
    pub quad_vertex_buffer: wgpu::Buffer,
    pub active_particles: u32,
    pub ring_head: std::sync::atomic::AtomicU32,
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
                size_start: 0.0,
                size_end: 0.0,
                _padding: [0.0; 2],
            });
        }

        let particles_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("GPU Particles Buffer"),
            contents: bytemuck::cast_slice(&initial_particles),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_DST,
        });

        let params = ParticleSimParams {
            dt: 0.0,
            global_gravity: 0.0,
            global_drag: 0.0,
            _padding: 0.0,
        };
        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("GPU Particle Params Buffer"),
            contents: bytemuck::cast_slice(&[params]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let quad_vertices: [[f32; 2]; 4] = [[-0.5, -0.5], [0.5, -0.5], [-0.5, 0.5], [0.5, 0.5]];

        let quad_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Particle Quad Vertex Buffer"),
            contents: bytemuck::cast_slice(&quad_vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let pipelines = create_particle_pipelines(
            device,
            global_bind_group_layout,
            output_format,
            &params_buffer,
            &particles_buffer,
        );

        Self {
            max_particles,
            particles_buffer,
            params_buffer,
            pipelines,
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
            _padding: 0.0,
        };
        queue.write_buffer(&self.params_buffer, 0, bytemuck::cast_slice(&[params]));
    }

    pub fn spawn_particles(&self, queue: &wgpu::Queue, new_particles: &[GpuParticle]) {
        if new_particles.is_empty() {
            return;
        }

        let count = new_particles.len() as u32;
        let mut head = self
            .ring_head
            .fetch_add(count, std::sync::atomic::Ordering::Relaxed)
            % self.max_particles;

        let mut remaining = count;
        let mut offset = 0;

        while remaining > 0 {
            let to_write = remaining.min(self.max_particles - head);
            let slice = &new_particles[offset as usize..(offset + to_write) as usize];

            queue.write_buffer(
                &self.particles_buffer,
                (head as usize * std::mem::size_of::<GpuParticle>()) as wgpu::BufferAddress,
                bytemuck::cast_slice(slice),
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
        cpass.set_pipeline(&self.pipelines.compute_pipeline);
        cpass.set_bind_group(0, &self.pipelines.compute_bind_group, &[]);
        let workgroups = self.max_particles.div_ceil(64);
        cpass.dispatch_workgroups(workgroups, 1, 1);
    }
}
