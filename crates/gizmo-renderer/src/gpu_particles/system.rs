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
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC, // readback (debug/tests)
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

    pub fn compute_pass(&self, encoder: &mut wgpu::CommandEncoder, active_particles: u32) {
        if active_particles == 0 {
            return;
        }
        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Particle Compute Pass"),
            timestamp_writes: None,
        });
        cpass.set_pipeline(&self.pipelines.compute_pipeline);
        cpass.set_bind_group(0, &self.pipelines.compute_bind_group, &[]);
        let workgroups = active_particles.div_ceil(64);
        cpass.dispatch_workgroups(workgroups, 1, 1);
    }

    pub fn render_pass<'a>(
        &'a self,
        rpass: &mut wgpu::RenderPass<'a>,
        global_bind_group: &'a wgpu::BindGroup,
        active_particles: u32,
    ) {
        if active_particles == 0 {
            return;
        }
        rpass.set_pipeline(&self.pipelines.render_pipeline);
        rpass.set_bind_group(0, global_bind_group, &[]);
        rpass.set_vertex_buffer(0, self.quad_vertex_buffer.slice(..));
        rpass.set_vertex_buffer(1, self.particles_buffer.slice(..));
        rpass.draw(0..4, 0..active_particles);
    }

    /// Helper method to spawn an explosion of particles (e.g. dust or debris from a fracture).
    pub fn spawn_explosion(
        &self,
        queue: &wgpu::Queue,
        center: [f32; 3],
        count: u32,
        base_color: [f32; 4],
        force: f32,
    ) {
        let mut new_particles = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let u: f32 = rand::random();
            let v: f32 = rand::random();
            let theta = u * 2.0 * std::f32::consts::PI;
            let phi = (2.0 * v - 1.0).acos();
            let r = force * (0.5 + 0.5 * rand::random::<f32>());

            let vx = r * phi.sin() * theta.cos();
            let vy = r * phi.sin() * theta.sin() + force * 0.5; // Upward bias
            let vz = r * phi.cos();

            let life = 0.5 + rand::random::<f32>() * 1.5;

            new_particles.push(GpuParticle {
                position: center,
                // `life` is an AGE that starts at 0 and grows toward `max_life`;
                // the compute/render shaders treat `life >= max_life` as dead.
                // Spawning with `life == max_life` makes every particle born dead
                // (never integrated, never drawn), so explosion/dust bursts were
                // completely invisible. Start the age at 0.
                life: 0.0,
                velocity: [vx, vy, vz],
                max_life: life,
                color: base_color,
                size_start: 0.1 + rand::random::<f32>() * 0.2,
                size_end: 0.0,
                _padding: [0.0; 2],
            });
        }
        self.spawn_particles(queue, &new_particles);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu_particles::types::GpuParticle;

    async fn setup_headless_gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            flags: wgpu::InstanceFlags::default(),
            memory_budget_thresholds: Default::default(),
            backend_options: Default::default(),
            display: None,
        });
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .ok()?;
        adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .ok()
    }

    async fn read_buffer<T: bytemuck::Pod>(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        buffer: &wgpu::Buffer,
    ) -> Vec<T> {
        let size = buffer.size();
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Test Staging Buffer"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, size);
        queue.submit(Some(encoder.finish()));
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |v| tx.send(v).unwrap());
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        rx.recv().unwrap().unwrap();
        let data = slice.get_mapped_range();
        let out = bytemuck::cast_slice(&data).to_vec();
        drop(data);
        staging.unmap();
        out
    }

    fn dummy_global_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        // Matches particle_render.wgsl @group(0): a single uniform buffer.
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("test_particle_global_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        })
    }

    // Regression: spawn_explosion used to write `life == max_life`, so every
    // spawned particle was born dead — the compute/render shaders treat
    // `life >= max_life` as dead, so impact/fracture dust bursts were invisible
    // and never integrated. A born-alive particle must (a) satisfy life<max_life
    // and (b) have its age advanced by a compute step (the dead branch would
    // leave it untouched).
    #[test]
    fn test_spawn_explosion_particles_are_born_alive() {
        pollster::block_on(async {
            let Some((device, queue)) = setup_headless_gpu().await else {
                tracing::info!("Skipping GPU test: no wgpu adapter found");
                return;
            };
            let layout = dummy_global_layout(&device);
            let system =
                GpuParticleSystem::new(&device, 64, &layout, wgpu::TextureFormat::Rgba8UnormSrgb);

            // Explosion fills the ring buffer starting at head 0 → indices [0, 8).
            system.spawn_explosion(&queue, [0.0, 10.0, 0.0], 8, [1.0, 1.0, 1.0, 1.0], 2.0);
            let before: Vec<GpuParticle> =
                read_buffer(&device, &queue, &system.particles_buffer).await;
            for (i, p) in before.iter().take(8).enumerate() {
                assert!(
                    p.life < p.max_life,
                    "spawned particle {} born dead: life={} max_life={}",
                    i,
                    p.life,
                    p.max_life
                );
            }

            // One compute step must advance the age of the (live) spawned particles.
            // With the old life==max_life bug the shader's dead branch returns
            // immediately and life would be unchanged — a clean discriminator.
            system.update_params(&queue, 1.0 / 60.0);
            let mut encoder =
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
            system.compute_pass(&mut encoder, 64);
            queue.submit(Some(encoder.finish()));
            let after: Vec<GpuParticle> =
                read_buffer(&device, &queue, &system.particles_buffer).await;
            for i in 0..8 {
                assert!(
                    after[i].life > before[i].life,
                    "particle {} did not integrate (age {} -> {}); born-dead?",
                    i,
                    before[i].life,
                    after[i].life
                );
            }
        });
    }
}
