use super::pipeline::{create_fluid_pipelines, FluidPipelines};
use super::types::*;
use crate::gpu_types::Vertex;
use wgpu::util::DeviceExt;

mod geometry;
mod passes;
mod ssfr;

use geometry::alloc_sphere_verts;
use ssfr::create_ssfr_sized;

pub struct GpuFluidSystem {
    pub num_particles: u32,
    pub total_cells: u32,
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

    // SSFR resources
    pub ssfr_particle_bg: wgpu::BindGroup,
    pub ssfr_blur_x_bg: wgpu::BindGroup,
    pub ssfr_blur_y_bg: wgpu::BindGroup,
    pub ssfr_composite_bg: wgpu::BindGroup,
    pub depth_texture_view: wgpu::TextureView,
    pub raw_depth_texture: wgpu::Texture,
    pub raw_depth_texture_view: wgpu::TextureView,
    pub blur_texture_view: wgpu::TextureView,
    pub thickness_texture_view: wgpu::TextureView,
    pub opaque_bg_texture: wgpu::Texture,
    pub opaque_bg_texture_view: wgpu::TextureView,
}

/// All screen-space-fluid-rendering (SSFR) resources whose size depends on the
/// render target. Recreated on window resize so the fluid always matches the
/// swapchain (previously created once in `new` and never rebuilt → the fluid was
/// confined to a stale sub-rectangle after any resize). Built by
/// [`ssfr::create_ssfr_sized`]; consumed by `new`/`resize` below.
struct SsfrSized {
    depth_texture_view: wgpu::TextureView,
    raw_depth_texture: wgpu::Texture,
    raw_depth_texture_view: wgpu::TextureView,
    blur_texture_view: wgpu::TextureView,
    thickness_texture_view: wgpu::TextureView,
    opaque_bg_texture: wgpu::Texture,
    opaque_bg_texture_view: wgpu::TextureView,
    ssfr_particle_bg: wgpu::BindGroup,
    ssfr_blur_x_bg: wgpu::BindGroup,
    ssfr_blur_y_bg: wgpu::BindGroup,
    ssfr_composite_bg: wgpu::BindGroup,
}

impl GpuFluidSystem {
    pub fn new(
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        num_particles: u32,
        global_bind_group_layout: &wgpu::BindGroupLayout,
        output_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Self {
        let n = num_particles as usize;
        // Okyanus için geniş ve sığ bir spawn alanı (10x10 metre)
        let spacing = 0.077_f32;
        let nx = ((10.0 - 0.2) / spacing).floor() as usize; // 10 metrelik havuz (-5..5), biraz pay bırakıyoruz
        let nz = ((10.0 - 0.2) / spacing).floor() as usize;

        let mut initial_particles = Vec::with_capacity(n);
        for i in 0..n {
            let xi = (i % nx) as f32;
            let zi = ((i / nx) % nz) as f32;
            let yi = (i / (nx * nz)) as f32;

            let offset_x = -4.9; // Havuzun sol köşesinden başla
            let offset_z = -4.9;

            let x = offset_x + xi * spacing;
            let y = 0.1 + yi * spacing; // Suları dipten başlat
            let z = offset_z + zi * spacing;

            initial_particles.push(FluidParticle {
                position: [x, y, z],
                density: 1000.0,
                velocity: [0.0, 0.0, 0.0],
                lambda: 0.0,
                predicted_position: [x, y, z],
                phase: 0xFFFFFFFF, // Marked as uninitialized
                vorticity: [0.0, 0.0, 0.0],
                _pad_vort: 0.0,
            });
        }

        let particles_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Fluid Particles Buffer"),
            contents: bytemuck::cast_slice(&initial_particles),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        // Domain'i büyütelim, Ocean simülasyonu için daha geniş bir alan
        let bounds_min = [-5.0, 0.0, -5.0];
        let bounds_max = [5.0, 10.0, 5.0];
        let cell_size = 0.1_f32; // DİKKAT: Bunu değiştirmek SPH Kernel formüllerini bozuyordu, 0.1'de kalmalı!

        let grid_size_x = f32::ceil((bounds_max[0] - bounds_min[0]) / cell_size) as u32 + 1;
        let grid_size_y = f32::ceil((bounds_max[1] - bounds_min[1]) / cell_size) as u32 + 1;
        let grid_size_z = f32::ceil((bounds_max[2] - bounds_min[2]) / cell_size) as u32 + 1;
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
            sort_initial.push(ParticleHash {
                hash: 0xFFFFFFFF,
                index: 0,
            });
        }
        let sort_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Fluid Sort Buffer"),
            contents: bytemuck::cast_slice(&sort_initial),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        // Initialize sort params buffer (pre-calculate all passes and pad to 256 bytes)
        let num_elements = num_particles.next_power_of_two();
        let mut sort_params_data = Vec::new();
        let mut k = 2u32;
        while k <= num_elements {
            let mut j = k >> 1;
            while j > 0 {
                let params = SortParams {
                    j,
                    k,
                    _pad0: 0,
                    _pad1: 0,
                };
                sort_params_data.extend_from_slice(bytemuck::cast_slice(&[params]));
                sort_params_data.extend_from_slice(&[0u8; 256 - 16]); // Pad to 256 bytes alignment
                j >>= 1;
            }
            k <<= 1;
        }

        let sort_params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Fluid Sort Params Buffer"),
            contents: &sort_params_data,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let params = FluidParams {
            dt: 0.016, // PBF timestep (1/60 sec)
            gravity: 9.81,
            rest_density: 1000.0,
            gas_constant: 10000.0,
            viscosity: 1.0,
            mass: 0.457,
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
            cohesion: 0.008,
            time: 0.0,
            // AAA Physics Parameters
            vorticity_strength: 0.35,   // Vorticity Confinement epsilon
            surface_tension: 0.5,       // Akinci surface tension gamma
            viscosity_laplacian: 0.005, // Laplacian viscosity mu
            xsph_factor: 0.05,          // XSPH smoothing factor
            solver_iterations: 6,       // PBF solver iterations
        };
        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Fluid Params Buffer"),
            contents: bytemuck::cast_slice(&[params]),
            usage: wgpu::BufferUsages::UNIFORM
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC, // readback (debug/tests)
        });

        // Initialize Colliders buffer
        let empty_colliders = vec![
            FluidCollider {
                position: [0.0; 3],
                radius: 0.0,
                velocity: [0.0; 3],
                shape_type: 0,
                half_extents: [0.0; 3],
                _pad: 0.0,
            };
            MAX_FLUID_COLLIDERS
        ];
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
        let mesh_vertices = wgpu::util::DeviceExt::create_buffer_init(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("Fluid Sphere Verts"),
                contents: bytemuck::cast_slice(&alloc_sphere_verts(0.25, 12, 12)),
                usage: wgpu::BufferUsages::VERTEX,
            },
        );

        // SSFR Textures & Bindings
        let ssfr = create_ssfr_sized(
            device,
            &pipelines,
            &particles_buffer,
            output_format,
            width,
            height,
        );

        Self {
            num_particles,
            total_cells,
            particles_buffer,
            params_buffer,
            grid_buffer,
            colliders_buffer,
            sort_buffer,
            sort_params_buffer,
            pipelines,
            mesh_vertices,
            index_count: 0,
            vertex_count: sphere_mesh.vertex_count,
            ssfr_particle_bg: ssfr.ssfr_particle_bg,
            ssfr_blur_x_bg: ssfr.ssfr_blur_x_bg,
            ssfr_blur_y_bg: ssfr.ssfr_blur_y_bg,
            ssfr_composite_bg: ssfr.ssfr_composite_bg,
            depth_texture_view: ssfr.depth_texture_view,
            raw_depth_texture: ssfr.raw_depth_texture,
            raw_depth_texture_view: ssfr.raw_depth_texture_view,
            blur_texture_view: ssfr.blur_texture_view,
            thickness_texture_view: ssfr.thickness_texture_view,
            opaque_bg_texture: ssfr.opaque_bg_texture,
            opaque_bg_texture_view: ssfr.opaque_bg_texture_view,
        }
    }

    /// Recreate the size-dependent SSFR render targets after a window resize.
    /// Without this the fluid's depth/thickness/blur/background textures keep
    /// their original dimensions and the fluid renders into a stale sub-rectangle
    /// (or the composite copy uses the wrong extent) once the window changes size.
    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        output_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) {
        if width == 0 || height == 0 {
            return;
        }
        let ssfr = create_ssfr_sized(
            device,
            &self.pipelines,
            &self.particles_buffer,
            output_format,
            width,
            height,
        );
        self.ssfr_particle_bg = ssfr.ssfr_particle_bg;
        self.ssfr_blur_x_bg = ssfr.ssfr_blur_x_bg;
        self.ssfr_blur_y_bg = ssfr.ssfr_blur_y_bg;
        self.ssfr_composite_bg = ssfr.ssfr_composite_bg;
        self.depth_texture_view = ssfr.depth_texture_view;
        self.raw_depth_texture = ssfr.raw_depth_texture;
        self.raw_depth_texture_view = ssfr.raw_depth_texture_view;
        self.blur_texture_view = ssfr.blur_texture_view;
        self.thickness_texture_view = ssfr.thickness_texture_view;
        self.opaque_bg_texture = ssfr.opaque_bg_texture;
        self.opaque_bg_texture_view = ssfr.opaque_bg_texture_view;
    }

    pub fn update_colliders_count(&self, queue: &wgpu::Queue, count: u32) {
        // num_colliders offset is 108:
        // params layout:
        // dt(0), gravity(4), rest_density(8), gas_constant(12), viscosity(16), mass(20), smoothing_radius(24), num_particles(28)
        // grid_size_x(32), grid_size_y(36), grid_size_z(40), cell_size(44)
        // bounds_min(48,52,56), padding1(60)
        // bounds_max(64,68,72), padding2(76)
        // mouse_pos(80,84,88), mouse_active(92)
        // mouse_dir(96,100,104), mouse_radius(108)
        // num_colliders(112)
        queue.write_buffer(&self.params_buffer, 112, bytemuck::cast_slice(&[count]));
    }

    pub fn update_parameters(
        &self,
        queue: &wgpu::Queue,
        mouse_pos: [f32; 3],
        mouse_dir: [f32; 3],
        mouse_active: bool,
        colliders: &[FluidCollider],
        time: f32,
        active_particles: u32,
    ) {
        // Upload dynamic colliders
        let num_colliders = (colliders.len().min(MAX_FLUID_COLLIDERS)) as u32;
        if num_colliders > 0 {
            queue.write_buffer(
                &self.colliders_buffer,
                0,
                bytemuck::cast_slice(&colliders[0..num_colliders as usize]),
            );
        }

        #[repr(C)]
        #[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
        struct DynamicFluidParams {
            mouse_pos: [f32; 3],
            mouse_active: f32,
            mouse_dir: [f32; 3],
            mouse_radius: f32,
            num_colliders: u32,
            cohesion: f32,
            time: f32,
            // AAA: Vorticity confinement strength
            vorticity_strength: f32,
            // AAA: Surface tension, Laplacian viscosity, XSPH, solver iterations
            surface_tension: f32,
            viscosity_laplacian: f32,
            xsph_factor: f32,
            solver_iterations: u32,
        }

        let dyn_params = DynamicFluidParams {
            mouse_pos,
            mouse_active: if mouse_active { 1.0 } else { 0.0 },
            mouse_dir,
            mouse_radius: 10.0, // Large mouse influence
            num_colliders,
            cohesion: 0.008, // Cohesion coefficient (surface tension molecular)
            time,
            vorticity_strength: 0.35,   // Vorticity Confinement epsilon
            surface_tension: 0.5,       // Akinci surface tension gamma
            viscosity_laplacian: 0.005, // Laplacian viscosity mu
            xsph_factor: 0.05,          // XSPH smoothing factor
            solver_iterations: 6,       // PBF solver iterations per frame
        };

        queue.write_buffer(&self.params_buffer, 80, bytemuck::cast_slice(&[dyn_params]));
        queue.write_buffer(
            &self.params_buffer,
            28,
            bytemuck::cast_slice(&[active_particles]),
        );
    }
}

#[cfg(test)]
mod gpu_dispatch_tests {
    use super::*;

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

    async fn read_u32s(device: &wgpu::Device, queue: &wgpu::Queue, buffer: &wgpu::Buffer) -> Vec<u32> {
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

    // Regression: `params.num_particles` (params offset 28) was never updated at
    // runtime, so under LOD < 1.0 the hash pass treated particles in [active, N)
    // as REAL and inserted them into the neighbor grid, while grid_offsets ran
    // over only `active` — silently dropping those particles from every neighbor
    // scan. compute_pass must now write the active count so the hashed set and the
    // offset-mapped set are one population.
    #[test]
    fn test_compute_pass_syncs_active_particle_count() {
        pollster::block_on(async {
            let Some((device, queue)) = setup_headless_gpu().await else {
                tracing::info!("Skipping GPU test: no wgpu adapter found");
                return;
            };
            let global_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("test_fluid_global_layout"),
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
            });

            let n = 1024u32;
            let system = GpuFluidSystem::new(
                &device,
                &queue,
                n,
                &global_layout,
                wgpu::TextureFormat::Rgba16Float,
                256,
                256,
            );

            // Reduced active count (as the render loop passes under LOD < 1.0).
            let active = 300u32;
            let mut encoder =
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
            system.compute_pass(&mut encoder, &queue, true, active);
            queue.submit(Some(encoder.finish()));

            // params.num_particles lives at byte offset 28 == u32 index 7.
            let params_words = read_u32s(&device, &queue, &system.params_buffer).await;
            assert_eq!(
                params_words[7], active,
                "compute_pass must sync params.num_particles (offset 28) to the active LOD count"
            );
        });
    }

    // Regression: the SSFR render targets were created once in `new` and never
    // rebuilt, so after a window resize the fluid rendered into a stale-sized
    // sub-rectangle. `resize` must recreate them at the new dimensions.
    #[test]
    fn test_resize_recreates_ssfr_textures() {
        pollster::block_on(async {
            let Some((device, queue)) = setup_headless_gpu().await else {
                tracing::info!("Skipping GPU test: no wgpu adapter found");
                return;
            };
            let global_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("test_fluid_global_layout"),
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
            });
            let fmt = wgpu::TextureFormat::Rgba16Float;
            let mut system = GpuFluidSystem::new(&device, &queue, 256, &global_layout, fmt, 256, 256);
            assert_eq!(
                (system.raw_depth_texture.width(), system.raw_depth_texture.height()),
                (256, 256)
            );
            assert_eq!(
                (system.opaque_bg_texture.width(), system.opaque_bg_texture.height()),
                (256, 256)
            );

            system.resize(&device, fmt, 512, 384);

            assert_eq!(
                (system.raw_depth_texture.width(), system.raw_depth_texture.height()),
                (512, 384),
                "resize must recreate raw_depth_texture at the new size"
            );
            assert_eq!(
                (system.opaque_bg_texture.width(), system.opaque_bg_texture.height()),
                (512, 384),
                "resize must recreate opaque_bg_texture at the new size"
            );
        });
    }
}
