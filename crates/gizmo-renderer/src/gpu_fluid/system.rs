use super::pipeline::{create_fluid_pipelines, FluidPipelines};
use super::types::*;
use crate::gpu_types::Vertex;
use wgpu::util::DeviceExt;

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
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
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
        let depth_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("SSFR Depth Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let depth_texture_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let raw_depth_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("SSFR Raw Depth Color Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let raw_depth_texture_view =
            raw_depth_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let blur_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("SSFR Blur Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let blur_texture_view = blur_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let thickness_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("SSFR Thickness Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R16Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let thickness_texture_view =
            thickness_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let blur_temp_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("SSFR Blur Temp Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let blur_temp_texture_view =
            blur_temp_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let blur_params_buffer_x = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Blur Params X"),
            contents: bytemuck::cast_slice(&[1u32, 0u32, 16u32, 1.0f32.to_bits()]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let blur_params_buffer_y = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Blur Params Y"),
            contents: bytemuck::cast_slice(&[0u32, 1u32, 16u32, 1.0f32.to_bits()]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let ssfr_particle_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("SSFR Particle BG"),
            layout: &pipelines.particle_render_bg_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 1,
                resource: particles_buffer.as_entire_binding(),
            }],
        });

        let ssfr_blur_x_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("SSFR Blur X BG"),
            layout: &pipelines.blur_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&raw_depth_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&blur_temp_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: blur_params_buffer_x.as_entire_binding(),
                },
            ],
        });

        let ssfr_blur_y_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("SSFR Blur Y BG"),
            layout: &pipelines.blur_bind_group_layout,
            entries: &[
                // We must sample from blur_temp_texture_view. Wait, fluid_blur expects depth format?
                // No, fluid_blur expects a texture_2d<f32>. depth_texture_view is Depth32Float.
                // Wait! Can we sample from R32Float in WGSL using texture_2d<f32>? YES!
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&blur_temp_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&blur_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: blur_params_buffer_y.as_entire_binding(),
                },
            ],
        });

        let opaque_bg_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Opaque Background Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: output_format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let opaque_bg_texture_view =
            opaque_bg_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let _linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let ssfr_composite_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("SSFR Composite BG"),
            layout: &pipelines.composite_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&blur_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&thickness_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&opaque_bg_texture_view),
                },
            ],
        });

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
            ssfr_particle_bg,
            ssfr_blur_x_bg,
            ssfr_blur_y_bg,
            ssfr_composite_bg,
            depth_texture_view,
            raw_depth_texture,
            raw_depth_texture_view,
            blur_texture_view,
            thickness_texture_view,
            opaque_bg_texture,
            opaque_bg_texture_view,
        }
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

    pub fn compute_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        _queue: &wgpu::Queue,
        update_grid: bool,
        active_particles: u32,
    ) {
        if active_particles == 0 {
            return;
        }
        let workgroups_parts = active_particles.div_ceil(64);

        // 1. PBF PREDICT PASS
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Fluid Predict Pass"),
                timestamp_writes: None,
            });
            cpass.set_bind_group(0, &self.pipelines.compute_bind_group, &[0]);
            cpass.set_pipeline(&self.pipelines.pipeline_predict);
            cpass.dispatch_workgroups(workgroups_parts, 1, 1);
        }

        // 2. SPATIAL HASHING (Based on predicted positions)
        if update_grid {
            {
                let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Fluid Clear Pass"),
                    timestamp_writes: None,
                });
                cpass.set_bind_group(0, &self.pipelines.compute_bind_group, &[0]);
                cpass.set_pipeline(&self.pipelines.pipeline_clear);
                cpass.dispatch_workgroups(self.total_cells.div_ceil(64), 1, 1);
            }

            let num_elements = active_particles.next_power_of_two();

            {
                let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Fluid Hash Pass"),
                    timestamp_writes: None,
                });
                cpass.set_bind_group(0, &self.pipelines.compute_bind_group, &[0]);
                cpass.set_pipeline(&self.pipelines.pipeline_hash);
                // MUST run over num_elements so padded elements get their hashes set to 0xFFFFFFFF
                cpass.dispatch_workgroups(num_elements.div_ceil(64), 1, 1);
            }

            // O(log^2 N) bitonic sort passes
            let mut offset_idx = 0;
            let mut k = 2u32;
            while k <= num_elements {
                let mut j = k >> 1;
                while j > 0 {
                    let offset = offset_idx * 256;
                    let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("Fluid Sort Pass"),
                        timestamp_writes: None,
                    });
                    cpass.set_bind_group(
                        0,
                        &self.pipelines.compute_bind_group,
                        &[offset as wgpu::DynamicOffset],
                    );
                    cpass.set_pipeline(&self.pipelines.pipeline_sort);
                    cpass.dispatch_workgroups(num_elements.div_ceil(64), 1, 1);

                    offset_idx += 1;
                    j >>= 1;
                }
                k <<= 1;
            }

            {
                let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Fluid Offsets Pass"),
                    timestamp_writes: None,
                });
                cpass.set_bind_group(0, &self.pipelines.compute_bind_group, &[0]);
                cpass.set_pipeline(&self.pipelines.pipeline_offsets);
                cpass.dispatch_workgroups(workgroups_parts, 1, 1);
            }
        }

        // 3. PBF SOLVER ITERATIONS (AAA: increased from 4 to 6 for better convergence)
        for _ in 0..6 {
            {
                let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Fluid Calc Lambda"),
                    timestamp_writes: None,
                });
                cpass.set_bind_group(0, &self.pipelines.compute_bind_group, &[0]);
                cpass.set_pipeline(&self.pipelines.pipeline_calc_lambda);
                cpass.dispatch_workgroups(workgroups_parts, 1, 1);
            }
            {
                let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Fluid Apply Delta P"),
                    timestamp_writes: None,
                });
                cpass.set_bind_group(0, &self.pipelines.compute_bind_group, &[0]);
                cpass.set_pipeline(&self.pipelines.pipeline_apply_delta_p);
                cpass.dispatch_workgroups(workgroups_parts, 1, 1);
            }
        }

        // 4. AAA: VORTICITY CONFINEMENT — ω = ∇ × v (curl of velocity)
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Fluid Compute Vorticity"),
                timestamp_writes: None,
            });
            cpass.set_bind_group(0, &self.pipelines.compute_bind_group, &[0]);
            cpass.set_pipeline(&self.pipelines.pipeline_compute_vorticity);
            cpass.dispatch_workgroups(workgroups_parts, 1, 1);
        }

        // 5. AAA: UPDATE VELOCITY — Vorticity Confinement + Surface Tension + Viscosity
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Fluid Update Velocity Pass"),
                timestamp_writes: None,
            });
            cpass.set_bind_group(0, &self.pipelines.compute_bind_group, &[0]);
            cpass.set_pipeline(&self.pipelines.pipeline_update_velocity);
            cpass.dispatch_workgroups(workgroups_parts, 1, 1);
        }

        // 6. AAA: CLASSIFY PARTICLES — Foam / Spray / Droplet detection
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Fluid Classify Particles"),
                timestamp_writes: None,
            });
            cpass.set_bind_group(0, &self.pipelines.compute_bind_group, &[0]);
            cpass.set_pipeline(&self.pipelines.pipeline_classify);
            cpass.dispatch_workgroups(workgroups_parts, 1, 1);
        }
    }

    pub fn render_pass<'a>(
        &'a self,
        _rpass: &mut wgpu::RenderPass<'a>,
        _global_scene_bind_group: &'a wgpu::BindGroup,
    ) {
        // Fallback for compatibility, not used directly by SSFR loop
    }
    pub fn render_ssfr(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target_texture: &wgpu::Texture,
        target_view: &wgpu::TextureView,
        scene_depth_view: &wgpu::TextureView,
        global_scene_bind_group: &wgpu::BindGroup,
        active_particles: u32,
    ) {
        if active_particles == 0 {
            return;
        }
        // Copy the opaque background before rendering fluid on top
        encoder.copy_texture_to_texture(
            wgpu::ImageCopyTexture {
                texture: target_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyTexture {
                texture: &self.opaque_bg_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: self.opaque_bg_texture.width(),
                height: self.opaque_bg_texture.height(),
                depth_or_array_layers: 1,
            },
        );

        // 1. Depth Pass
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("SSFR Depth"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.raw_depth_texture_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_texture_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rpass.set_pipeline(&self.pipelines.pipeline_depth);
            rpass.set_bind_group(0, global_scene_bind_group, &[]);
            rpass.set_bind_group(1, &self.ssfr_particle_bg, &[]);
            rpass.draw(0..4, 0..active_particles);
        }

        // 2. Blur Pass (Ping-Pong)
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("SSFR Blur"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&self.pipelines.pipeline_blur);

            let target_width = target_texture.width();
            let target_height = target_texture.height();

            // X Pass
            cpass.set_bind_group(0, &self.ssfr_blur_x_bg, &[]);
            cpass.dispatch_workgroups(target_width.div_ceil(16), target_height.div_ceil(16), 1);

            // Y Pass
            cpass.set_bind_group(0, &self.ssfr_blur_y_bg, &[]);
            cpass.dispatch_workgroups(target_width.div_ceil(16), target_height.div_ceil(16), 1);
        }

        // 3. Thickness Pass
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("SSFR Thickness"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.thickness_texture_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rpass.set_pipeline(&self.pipelines.pipeline_thickness);
            rpass.set_bind_group(0, global_scene_bind_group, &[]);
            rpass.set_bind_group(1, &self.ssfr_particle_bg, &[]);
            rpass.draw(0..4, 0..active_particles);
        }

        // 4. Composite Pass
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("SSFR Composite"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: scene_depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rpass.set_pipeline(&self.pipelines.pipeline_composite);
            rpass.set_bind_group(0, global_scene_bind_group, &[]);
            rpass.set_bind_group(1, &self.ssfr_composite_bg, &[]);
            rpass.draw(0..3, 0..1); // Fullscreen triangle
        }

        // 5. AAA: Foam/Spray/Droplet Render Pass
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("SSFR Foam/Spray"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load, // Preserve composite result
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: scene_depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rpass.set_pipeline(&self.pipelines.pipeline_foam);
            rpass.set_bind_group(0, global_scene_bind_group, &[]);
            rpass.set_bind_group(1, &self.ssfr_particle_bg, &[]);
            rpass.draw(0..4, 0..active_particles); // Only foam/spray survive in vertex shader
        }
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
            let p1 = [
                radius * theta1.sin() * phi1.cos(),
                radius * theta1.cos(),
                radius * theta1.sin() * phi1.sin(),
            ];
            let p2 = [
                radius * theta2.sin() * phi1.cos(),
                radius * theta2.cos(),
                radius * theta2.sin() * phi1.sin(),
            ];
            let p3 = [
                radius * theta2.sin() * phi2.cos(),
                radius * theta2.cos(),
                radius * theta2.sin() * phi2.sin(),
            ];
            let p4 = [
                radius * theta1.sin() * phi2.cos(),
                radius * theta1.cos(),
                radius * theta1.sin() * phi2.sin(),
            ];
            let n1 = [
                theta1.sin() * phi1.cos(),
                theta1.cos(),
                theta1.sin() * phi1.sin(),
            ];
            let n2 = [
                theta2.sin() * phi1.cos(),
                theta2.cos(),
                theta2.sin() * phi1.sin(),
            ];
            let n3 = [
                theta2.sin() * phi2.cos(),
                theta2.cos(),
                theta2.sin() * phi2.sin(),
            ];
            let n4 = [
                theta1.sin() * phi2.cos(),
                theta1.cos(),
                theta1.sin() * phi2.sin(),
            ];

            vertices.push(Vertex {
                position: p1,
                color: [1.0; 3],
                normal: n1,
                tex_coords: [0.0; 2],
                joint_indices: [0; 4],
                joint_weights: [0.0; 4],
            });
            vertices.push(Vertex {
                position: p2,
                color: [1.0; 3],
                normal: n2,
                tex_coords: [0.0; 2],
                joint_indices: [0; 4],
                joint_weights: [0.0; 4],
            });
            vertices.push(Vertex {
                position: p3,
                color: [1.0; 3],
                normal: n3,
                tex_coords: [0.0; 2],
                joint_indices: [0; 4],
                joint_weights: [0.0; 4],
            });
            vertices.push(Vertex {
                position: p1,
                color: [1.0; 3],
                normal: n1,
                tex_coords: [0.0; 2],
                joint_indices: [0; 4],
                joint_weights: [0.0; 4],
            });
            vertices.push(Vertex {
                position: p3,
                color: [1.0; 3],
                normal: n3,
                tex_coords: [0.0; 2],
                joint_indices: [0; 4],
                joint_weights: [0.0; 4],
            });
            vertices.push(Vertex {
                position: p4,
                color: [1.0; 3],
                normal: n4,
                tex_coords: [0.0; 2],
                joint_indices: [0; 4],
                joint_weights: [0.0; 4],
            });
        }
    }
    vertices
}
