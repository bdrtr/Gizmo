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
    /// Simülasyon yerçekimi (m/s²), her frame parçacığın `velocity.y`'sinden düşülür.
    /// Default 9.81 (kıvılcım/duman düşer). Yatay akış/streamline için `0.0` yap —
    /// böylece parçacıklar sarkmadan düz akar (ör. rüzgar tüneli). Runtime'da değişir.
    pub gravity: f32,
    /// Simülasyon sürükleme katsayısı (üstel hız sönümü, `v -= v*drag*dt`). Default 0.8
    /// (parçacıklar hızla yavaşlar). Düz/uzun streamline için `≈0.0` yap ki hız korunsun.
    pub drag: f32,
    /// Engel küreleri (xyz = merkez dünya-uzayı, w = yarıçap). Parçacıklar bunlara
    /// çarpıp etrafından SAPAR (flow-around). En fazla `MAX_PARTICLE_OBSTACLES` kullanılır.
    /// Boş (default) → sapma kapalı. Rüzgar tüneli gibi "akış cisme çarpsın" için doldur.
    pub obstacles: Vec<[f32; 4]>,
    /// Nominal akış hızı (relaks hedefi). Parçacık hızı buna doğru yumuşakça çekilir →
    /// engelden sonra çizgiler tekrar paralelleşir. `flow_relax`=0 iken etkisiz.
    pub flow_target: [f32; 3],
    /// Akış relaks oranı (1/s). 0 = kapalı (default).
    pub flow_relax: f32,
    /// Türbülans gücü: relaks hedefine eklenen dalgalı (swirl) hız genliği → düz akış
    /// yerine duman gibi kıvrımlı filamentler. 0 = kapalı (default, düz çizgiler).
    pub turbulence: f32,
    /// Curl-noise gücü: parçacık hızına DOĞRUDAN eklenen diverjanssız (zamanla evrilen)
    /// swirl → duman/toz gerçekçi kıvrılır. 0 = kapalı (default). Duman için ~1.5–3.
    pub curl_strength: f32,
    /// Flipbook/SubUV atlas bind group'u (group 2). Default 1×1 (boş); `set_flipbook` /
    /// `set_procedural_smoke_flipbook` gerçek atlası yükler.
    pub flipbook_bind_group: wgpu::BindGroup,
    /// Flipbook config uniform'u (render FS: x=tiles/kenar, y=açık). update_params yazar.
    pub flipbook_params_buffer: wgpu::Buffer,
    /// Atlas'ın kenar-başına kare sayısı (ör. 4 → 4×4=16 kare). FS SubUV için.
    pub flipbook_tiles: f32,
    /// Flipbook açık mı (FS'te atlas mı yoksa prosedürel yuvarlak mı). Default kapalı →
    /// mevcut kıvılcım/toz efektleri değişmez; duman için demo açar.
    pub flipbook_on: bool,
    /// Işıklandırma (T4): küresel normal + half-lambert + ambient ile güneşe göre aydınlan.
    /// Default kapalı (kıvılcım/ateş emissive kalsın); duman/toz için demo açar.
    pub lit: bool,
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
            obstacle_count: 0.0,
            flow_target: [0.0; 4],
            misc: [0.0; 4],
            obstacles: [[0.0; 4]; crate::gpu_particles::types::MAX_PARTICLE_OBSTACLES],
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

        // Flipbook config uniform (x=tiles, y=on). Default: 4 kare, kapalı.
        let flipbook_params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("particle_flipbook_config"),
            contents: bytemuck::cast_slice(&[4.0f32, 0.0, 0.0, 0.0]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        // Default flipbook: 1×1 (yazılmamış) doku + linear sampler. flipbook_on=false iken
        // FS örneklemez (prosedürel yuvarlak kullanır) → içerik önemsiz.
        let flipbook_bind_group = Self::build_flipbook_bind_group(
            device,
            &pipelines.flipbook_bind_group_layout,
            &Self::empty_flipbook_texture(device)
                .create_view(&wgpu::TextureViewDescriptor::default()),
            &flipbook_params_buffer,
        );

        Self {
            max_particles,
            particles_buffer,
            params_buffer,
            pipelines,
            quad_vertex_buffer,
            active_particles: max_particles,
            ring_head: std::sync::atomic::AtomicU32::new(0),
            gravity: 9.81,
            drag: 0.8,
            obstacles: Vec::new(),
            flow_target: [0.0; 3],
            flow_relax: 0.0,
            turbulence: 0.0,
            curl_strength: 0.0,
            flipbook_bind_group,
            flipbook_params_buffer,
            flipbook_tiles: 4.0,
            flipbook_on: false,
            lit: false,
        }
    }

    fn empty_flipbook_texture(device: &wgpu::Device) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("particle_flipbook_default"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }

    fn build_flipbook_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        view: &wgpu::TextureView,
        config_buffer: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("particle_flipbook_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("particle_flipbook_bind_group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: config_buffer.as_entire_binding(),
                },
            ],
        })
    }

    /// RGBA8 atlas verisini flipbook olarak yükler (kenar-başına `tiles` kare) ve etkinleştirir.
    pub fn set_flipbook(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        rgba: &[u8],
        width: u32,
        height: u32,
        tiles: u32,
    ) {
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("particle_flipbook_atlas"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        self.flipbook_bind_group = Self::build_flipbook_bind_group(
            device,
            &self.pipelines.flipbook_bind_group_layout,
            &view,
            &self.flipbook_params_buffer,
        );
        self.flipbook_tiles = tiles as f32;
        self.flipbook_on = true;
    }

    /// PROSEDÜREL duman flipbook'u üretip yükler (dış varlık gerekmez): `tiles`×`tiles` kare,
    /// her kare fBm-modüle bir duman topağının bir animasyon fazı (büyür + dağılır + kıvrılır).
    pub fn set_procedural_smoke_flipbook(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let tiles: u32 = 4;
        let tile_px: u32 = 128;
        let (rgba, size) = generate_smoke_atlas(tiles, tile_px);
        self.set_flipbook(device, queue, &rgba, size, size, tiles);
    }

    pub fn update_params(&self, queue: &wgpu::Queue, dt: f32, time: f32) {
        let count = self.obstacles.len().min(crate::gpu_particles::types::MAX_PARTICLE_OBSTACLES);
        let mut obstacles = [[0.0f32; 4]; crate::gpu_particles::types::MAX_PARTICLE_OBSTACLES];
        obstacles[..count].copy_from_slice(&self.obstacles[..count]);
        let params = ParticleSimParams {
            dt,
            global_gravity: self.gravity,
            global_drag: self.drag,
            obstacle_count: count as f32,
            flow_target: [
                self.flow_target[0],
                self.flow_target[1],
                self.flow_target[2],
                self.flow_relax,
            ],
            // COMPUTE misc: x=türbülans, y=curl_strength (duman kıvrılması), z=mutlak zaman.
            // (flipbook config compute'ta kullanılmaz; ayrı flipbook_params_buffer'da.)
            misc: [self.turbulence, self.curl_strength, time, 0.0],
            obstacles,
        };
        queue.write_buffer(&self.params_buffer, 0, bytemuck::cast_slice(&[params]));
        // Flipbook render config (FS group 2): x=tiles, y=flipbook açık, z=ışıklandırma açık.
        let fb = [
            self.flipbook_tiles,
            if self.flipbook_on { 1.0f32 } else { 0.0 },
            if self.lit { 1.0f32 } else { 0.0 },
            0.0,
        ];
        queue.write_buffer(&self.flipbook_params_buffer, 0, bytemuck::cast_slice(&fb));
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

    /// Soft particles için group-1 (sahne derinliği) bind group'u güncel `depth_view` ile
    /// oluşturur. `depth_texture_view` resize'da değiştiğinden her frame çağrılmalı (ucuz).
    pub fn create_depth_bind_group(
        &self,
        device: &wgpu::Device,
        depth_view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("particle_depth_bind_group"),
            layout: &self.pipelines.depth_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(depth_view),
            }],
        })
    }

    pub fn render_pass<'a>(
        &'a self,
        rpass: &mut wgpu::RenderPass<'a>,
        global_bind_group: &'a wgpu::BindGroup,
        depth_bind_group: &'a wgpu::BindGroup,
        active_particles: u32,
    ) {
        if active_particles == 0 {
            return;
        }
        rpass.set_pipeline(&self.pipelines.render_pipeline);
        rpass.set_bind_group(0, global_bind_group, &[]);
        rpass.set_bind_group(1, depth_bind_group, &[]); // soft particles: sahne derinliği
        rpass.set_bind_group(2, &self.flipbook_bind_group, &[]); // flipbook/SubUV atlas
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

// ── Prosedürel duman flipbook üretimi (fBm) ──────────────────────────────────
fn hash2(x: i32, y: i32) -> f32 {
    let mut h = (x.wrapping_mul(374761393)).wrapping_add(y.wrapping_mul(668265263)) as u32;
    h = (h ^ (h >> 13)).wrapping_mul(1274126177);
    h ^= h >> 16;
    h as f32 / u32::MAX as f32
}

fn vnoise(px: f32, py: f32) -> f32 {
    let x0 = px.floor() as i32;
    let y0 = py.floor() as i32;
    let fx = px - x0 as f32;
    let fy = py - y0 as f32;
    let ux = fx * fx * (3.0 - 2.0 * fx);
    let uy = fy * fy * (3.0 - 2.0 * fy);
    let n00 = hash2(x0, y0);
    let n10 = hash2(x0 + 1, y0);
    let n01 = hash2(x0, y0 + 1);
    let n11 = hash2(x0 + 1, y0 + 1);
    let nx0 = n00 * (1.0 - ux) + n10 * ux;
    let nx1 = n01 * (1.0 - ux) + n11 * ux;
    nx0 * (1.0 - uy) + nx1 * uy
}

fn fbm(px: f32, py: f32) -> f32 {
    let mut v = 0.0;
    let mut amp = 0.5;
    let mut freq = 1.0;
    for _ in 0..4 {
        v += amp * vnoise(px * freq, py * freq);
        freq *= 2.0;
        amp *= 0.5;
    }
    v
}

fn sstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// `tiles`×`tiles` kare duman atlası üretir; her kare bir animasyon fazı (büyür + dağılır +
/// fBm ile kıvrılır). Beyaz duman, alpha = yoğunluk. Döner: (RGBA8 veri, atlas kenar-piksel).
pub fn generate_smoke_atlas(tiles: u32, tile_px: u32) -> (Vec<u8>, u32) {
    let size = tiles * tile_px;
    let frames = tiles * tiles;
    let mut data = vec![0u8; (size * size * 4) as usize];
    for f in 0..frames {
        let phase = f as f32 / frames as f32; // 0..1
        let tx = (f % tiles) * tile_px;
        let ty = (f / tiles) * tile_px;
        let ox = phase * 7.3; // fBm kayması → kare kare evrilir
        let oy = phase * 3.1;
        let radius = 0.35 + phase * 0.55; // topak büyür
        let dissip = 1.0 - phase * 0.65; // ömür sonuna doğru soluklaşır
        for py in 0..tile_px {
            for px in 0..tile_px {
                let u = (px as f32 / (tile_px - 1) as f32) * 2.0 - 1.0;
                let v = (py as f32 / (tile_px - 1) as f32) * 2.0 - 1.0;
                let r = (u * u + v * v).sqrt();
                let radial = 1.0 - sstep(radius * 0.35, radius, r); // merkez dolu, kenar boş
                let n = fbm(u * 2.3 + ox, v * 2.3 + oy); // ~0..1
                let density = (radial * (0.3 + 0.95 * n) * dissip).clamp(0.0, 1.0);
                let a = (density * 255.0) as u8;
                let idx = (((ty + py) * size + (tx + px)) * 4) as usize;
                data[idx] = 240;
                data[idx + 1] = 244;
                data[idx + 2] = 255;
                data[idx + 3] = a;
            }
        }
    }
    (data, size)
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
            system.update_params(&queue, 1.0 / 60.0, 0.0);
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
