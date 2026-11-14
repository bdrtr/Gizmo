use super::pipeline::{create_fluid_pipelines, FluidPipelines};
use super::types::*;
use crate::gpu_types::Vertex;
use wgpu::util::DeviceExt;

mod geometry;
mod passes;
mod ssfr;

use geometry::alloc_sphere_verts;
use ssfr::create_ssfr_sized;

/// A GPU SPH fluid: the solver's buffers, its pipelines, and the screen-space surface it is drawn
/// as.
///
/// A renderer allocates one of these, holding **no fluid** until it is given one: its settings'
/// particle count starts at zero, and `gizmo`'s frame preparation sets them from the scene's
/// `FluidSimulation` — a scene without one has no water over it.
pub struct GpuFluidSystem {
    /// How many particles the simulation can run — its buffers' size.
    pub num_particles: u32,
    /// How many of them this frame's step simulated, and so how many every view of the frame
    /// draws. Written by the step (the game path's, once a frame and ahead of every view): the
    /// share of [`FluidSettings::particles`] the frame's level of detail asks for, and 0 before the
    /// first step.
    pub active_particles: u32,
    /// How many cells the neighbour-search grid has.
    pub total_cells: u32,
    /// The particles — [`FluidParticle`]s.
    pub particles_buffer: wgpu::Buffer,
    /// The solver's parameters.
    pub params_buffer: wgpu::Buffer,
    /// Each cell's range in the sorted hash list.
    pub grid_buffer: wgpu::Buffer,
    /// The obstacles.
    pub colliders_buffer: wgpu::Buffer,
    /// What the fluid is — set with [`Self::apply_settings`], read back with [`Self::settings`].
    settings: FluidSettings,
    /// The fluid's own clock: game time in, whole steps of [`FLUID_STEP`] out (see [`Self::advance`]).
    clock: gizmo_core::time::PhysicsTime,
    /// The clock's time still owed, shared with the scene's `FluidState` so a save holds it too.
    leftover: std::sync::Arc<std::sync::atomic::AtomicU32>,
    /// One `vec4` per particle a pass computes from its neighbours and a commit pass copies in —
    /// so no pass writes what its neighbours are still reading (see `fluid_bindings.wgsl`).
    pub scratch_buffer: wgpu::Buffer,
    /// Per obstacle, the particles against it — see [`Self::count_wetting`].
    pub wetting_buffer: wgpu::Buffer,
    /// The `(hash, index)` pairs the bitonic sort orders.
    pub sort_buffer: wgpu::Buffer,
    /// Which stage of that sort the next dispatch runs.
    pub sort_params_buffer: wgpu::Buffer,

    /// Every pipeline the fluid runs.
    pub pipelines: FluidPipelines,

    /// The impostor quad each particle is splatted through.
    pub mesh_vertices: wgpu::Buffer,
    /// Its index count.
    pub index_count: u32,
    /// Its vertex count.
    pub vertex_count: u32,

    // SSFR resources
    /// The particles, as the splat passes read them.
    pub ssfr_particle_bg: wgpu::BindGroup,
    /// The horizontal blur's inputs.
    pub ssfr_blur_x_bg: wgpu::BindGroup,
    /// The vertical blur's — separable, for the same reason bloom's blur is.
    pub ssfr_blur_y_bg: wgpu::BindGroup,
    /// The composite pass's inputs.
    pub ssfr_composite_bg: wgpu::BindGroup,
    /// The depth attachment the splat pass renders with.
    pub depth_texture_view: wgpu::TextureView,
    /// The splatted depth, before blurring.
    pub raw_depth_texture: wgpu::Texture,
    /// Its view.
    pub raw_depth_texture_view: wgpu::TextureView,
    /// The blurred depth — the surface the composite pass shades.
    pub blur_texture_view: wgpu::TextureView,
    /// The accumulated thickness, driving absorption.
    pub thickness_texture_view: wgpu::TextureView,
    /// A copy of the opaque scene behind the water, so the composite pass can refract it — a pass
    /// cannot sample the target it is writing.
    pub opaque_bg_texture: wgpu::Texture,
    /// Its view.
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
    /// Allocates every buffer and screen-space target and builds every pipeline.
    pub fn new(
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        num_particles: u32,
        global_bind_group_layout: &wgpu::BindGroupLayout,
        output_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Self {
        // Holding no fluid until it is given one: every particle not yet placed (see [`lattice`]).
        let settings = FluidSettings { particles: 0, ..FluidSettings::default() };
        let initial_particles = lattice(&settings, num_particles);
        let particles_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Fluid Particles Buffer"),
            contents: bytemuck::cast_slice(&initial_particles),
            // COPY_SRC: read back to be saved with a scene (`read_particles`).
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
        });

        let [grid_size_x, grid_size_y, grid_size_z] = settings.grid_size();
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

        let params = params_for(&settings, num_particles);
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

        let scratch_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Fluid Scratch Buffer"),
            size: (num_particles.max(1) as u64) * 16,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let wetting_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Fluid Wetting Buffer"),
            size: (MAX_FLUID_COLLIDERS * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
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
            &scratch_buffer,
            &wetting_buffer,
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
            active_particles: 0,
            total_cells,
            particles_buffer,
            params_buffer,
            grid_buffer,
            colliders_buffer,
            settings,
            clock: {
                let mut clock = gizmo_core::time::PhysicsTime::default();
                clock.set_fixed_dt(FLUID_STEP);
                clock
            },
            leftover: Default::default(),
            scratch_buffer,
            wetting_buffer,
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

    /// **Steps the water by `elapsed` seconds of the game's time**, in whole steps of
    /// [`FLUID_STEP`]: none while the game is paused, two in a frame of 1/30 s, at half the rate
    /// under a time scale of ½, and the remainder carried to the next frame — at most eight in one
    /// frame, the rest dropped, as the bodies' clocks do. `None` (a world with no clock) is one
    /// step. Each step simulates `active` particles. Returns how many steps it recorded.
    ///
    /// It stepped once for every frame drawn: the water ran on under ⏸ and twice as fast at 120 Hz.
    pub fn advance(&mut self, encoder: &mut wgpu::CommandEncoder, queue: &wgpu::Queue, elapsed: Option<f32>, active: u32) -> u32 {
        let steps = match elapsed {
            None => 1,
            Some(dt) => {
                self.clock.accumulate(dt);
                let mut n = 0;
                while self.clock.should_step() {
                    self.clock.consume_step();
                    n += 1;
                }
                n
            }
        };
        self.leftover.store(self.clock.accumulator().to_bits(), std::sync::atomic::Ordering::Relaxed);
        for _ in 0..steps {
            self.compute_pass(encoder, queue, true, active);
        }
        steps
    }

    /// The game time the clock still owes the water, seconds.
    pub fn leftover(&self) -> f32 {
        self.clock.accumulator()
    }

    /// Puts back the time owed — the water's saved clock, on reopening it.
    pub fn set_leftover(&mut self, seconds: f32) {
        self.clock.set_accumulator(seconds);
        self.leftover.store(self.clock.accumulator().to_bits(), std::sync::atomic::Ordering::Relaxed);
    }

    /// The shared cell the clock's owed time is mirrored into after every advance.
    pub fn leftover_cell(&self) -> std::sync::Arc<std::sync::atomic::AtomicU32> {
        self.leftover.clone()
    }

    /// What the fluid is now.
    pub fn settings(&self) -> FluidSettings {
        self.settings
    }

    /// Makes the fluid `settings`: the solver's dials take effect on the next step, and a new box
    /// or particle size rebuilds the neighbour grid over it. A new box, particle size or count also
    /// lays the water out again, at rest at the bottom of the box ([`lattice`]) — the particles that
    /// were there belonged to another fluid. `particles` past the pool is held to the pool.
    pub fn apply_settings(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, settings: FluidSettings) {
        let settings = FluidSettings { particles: settings.particles.min(self.num_particles), ..settings };
        if settings == self.settings {
            return;
        }
        let old = self.settings;
        self.settings = settings;
        let refill = old.bounds_min != settings.bounds_min
            || old.bounds_max != settings.bounds_max
            || old.particle_radius != settings.particle_radius
            || old.particles != settings.particles;
        if old.grid_size() != settings.grid_size() {
            let [x, y, z] = settings.grid_size();
            self.total_cells = x * y * z;
            self.grid_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Fluid Grid Buffer"),
                contents: bytemuck::cast_slice(&vec![0_u32; (self.total_cells * 2) as usize]),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            });
            self.pipelines.compute_bind_group = super::pipeline::create_compute_bind_group(
                device,
                &self.pipelines.compute_bind_group_layout,
                &self.params_buffer,
                &self.particles_buffer,
                &self.grid_buffer,
                &self.colliders_buffer,
                &self.sort_buffer,
                &self.sort_params_buffer,
                &self.scratch_buffer,
                &self.wetting_buffer,
            );
        }
        if refill {
            queue.write_buffer(&self.particles_buffer, 0, bytemuck::cast_slice(&lattice(&settings, self.num_particles)));
        }
        // Everything but the per-step block (pointer, colliders, time) the step writes itself.
        let params = params_for(&settings, self.num_particles);
        queue.write_buffer(&self.params_buffer, 0, &bytemuck::bytes_of(&params)[..80]);
        queue.write_buffer(&self.params_buffer, 116, &bytemuck::bytes_of(&params)[116..]);
    }

    /// The first `count` particles as they are on the GPU now — **waits** for the GPU to finish
    /// what it was given. For saving the water with a scene, and for tests; not for every frame.
    /// `None` when the GPU would not give the buffer back.
    pub fn read_particles(&self, device: &wgpu::Device, queue: &wgpu::Queue, count: u32) -> Option<Vec<FluidParticle>> {
        let size = (count.min(self.num_particles) as u64) * std::mem::size_of::<FluidParticle>() as u64;
        if size == 0 {
            return Some(Vec::new());
        }
        Some(bytemuck::cast_slice(&read_buffer(device, queue, &self.particles_buffer, size)?).to_vec())
    }

    /// The solver's parameter block as the GPU holds it — waits, like [`Self::read_particles`].
    pub fn read_params(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Option<FluidParams> {
        Some(bytemuck::pod_read_unaligned(&read_buffer(device, queue, &self.params_buffer, std::mem::size_of::<FluidParams>() as u64)?))
    }

    /// Puts `particles` in the pool from its start — water saved with a scene, come back where it
    /// was. The settings must already be the fluid's (a later change of box, size or count lays the
    /// water out again over these).
    pub fn write_particles(&self, queue: &wgpu::Queue, particles: &[FluidParticle]) {
        let n = particles.len().min(self.num_particles as usize);
        if n > 0 {
            queue.write_buffer(&self.particles_buffer, 0, bytemuck::cast_slice(&particles[..n]));
        }
    }

    /// For each of the first `colliders` obstacles, how much of it the water is against, `0..=1`:
    /// the particles in the shell around it (half a particle to two particles out) over how many a
    /// shell the water closes over holds at rest. Counted from the water as it is now, and
    /// **waits** for the GPU — once a frame, by the engine's fluid pass, and only when something in
    /// the water is lifted by it. `None` when the counts could not be read back.
    pub fn count_wetting(&self, device: &wgpu::Device, queue: &wgpu::Queue, colliders: &[FluidCollider]) -> Option<Vec<f32>> {
        let n = colliders.len().min(MAX_FLUID_COLLIDERS);
        if n == 0 || self.settings.particles == 0 {
            return Some(vec![0.0; n]);
        }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Fluid Wetting") });
        encoder.clear_buffer(&self.wetting_buffer, 0, None);
        self.dispatch(&mut encoder, "Fluid Count Wetting", &self.pipelines.pipeline_count_wetting, self.num_particles.div_ceil(64));
        queue.submit(Some(encoder.finish()));
        let bytes = read_buffer(device, queue, &self.wetting_buffer, (n * 4) as u64)?;
        let counts: &[u32] = bytemuck::cast_slice(&bytes);
        let h = self.settings.particle_radius;
        let (inner, outer) = (h * 0.5, h * 2.0);
        let per_particle = self.settings.spacing().powi(3);
        Some(
            colliders[..n]
                .iter()
                .zip(counts)
                .map(|(c, &count)| {
                    let shell = match c.shape_type {
                        0 => {
                            let ball = |r: f32| 4.0 / 3.0 * std::f32::consts::PI * r.powi(3);
                            ball(c.radius + outer) - ball(c.radius + inner)
                        }
                        _ => {
                            let cube = |g: f32| c.half_extents.iter().map(|e| 2.0 * (e + g)).product::<f32>();
                            cube(outer) - cube(inner)
                        }
                    };
                    (count as f32 * per_particle / shell.max(1e-9)).clamp(0.0, 1.0)
                })
                .collect(),
        )
    }

    /// The obstacles for the next step — at most [`MAX_FLUID_COLLIDERS`], the rest dropped.
    pub fn set_colliders(&self, queue: &wgpu::Queue, colliders: &[FluidCollider]) {
        let n = colliders.len().min(MAX_FLUID_COLLIDERS);
        if n > 0 {
            queue.write_buffer(&self.colliders_buffer, 0, bytemuck::cast_slice(&colliders[..n]));
        }
        self.update_colliders_count(queue, n as u32);
    }

    /// Sets how many entries of the collider buffer are live.
    pub fn update_colliders_count(&self, queue: &wgpu::Queue, count: u32) {
        // num_colliders offset is 108:
        // params layout:
        // dt(0), gravity(4), rest_density(8), relaxation(12), _reserved(16), mass(20), smoothing_radius(24), num_particles(28)
        // grid_size_x(32), grid_size_y(36), grid_size_z(40), cell_size(44)
        // bounds_min(48,52,56), padding1(60)
        // bounds_max(64,68,72), padding2(76)
        // mouse_pos(80,84,88), mouse_active(92)
        // mouse_dir(96,100,104), mouse_radius(108)
        // num_colliders(112)
        queue.write_buffer(&self.params_buffer, 112, bytemuck::cast_slice(&[count]));
    }

    /// Uploads the solver's parameters for this step.
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
            cohesion: self.settings.cohesion,
            time,
            vorticity_strength: self.settings.vorticity,
            surface_tension: self.settings.surface_tension,
            viscosity_laplacian: self.settings.viscosity,
            xsph_factor: self.settings.xsph,
            solver_iterations: self.settings.solver_iterations,
        };

        queue.write_buffer(&self.params_buffer, 80, bytemuck::cast_slice(&[dyn_params]));
        queue.write_buffer(
            &self.params_buffer,
            28,
            bytemuck::cast_slice(&[active_particles]),
        );
    }
}

/// One step of the water, seconds — the solver's `dt`, and what [`GpuFluidSystem::advance`] spends
/// the game's time in.
pub const FLUID_STEP: f32 = 1.0 / 60.0;

/// The first `size` bytes of `buffer`, copied out and waited for.
pub(crate) fn read_buffer(device: &wgpu::Device, queue: &wgpu::Queue, buffer: &wgpu::Buffer, size: u64) -> Option<Vec<u8>> {
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Fluid Readback"),
        size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Fluid Readback") });
    encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, size);
    queue.submit(Some(encoder.finish()));
    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |v| {
        let _ = tx.send(v);
    });
    let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
    if !matches!(rx.recv(), Ok(Ok(()))) {
        tracing::warn!(size, "[GpuFluid] a fluid buffer could not be read back");
        return None;
    }
    let out = slice.get_mapped_range().ok().map(|d| d.to_vec());
    staging.unmap();
    out
}

/// The solver's block for `settings`, over a pool of `pool` particles, with no pointer and no
/// colliders.
fn params_for(settings: &FluidSettings, pool: u32) -> FluidParams {
    let [grid_size_x, grid_size_y, grid_size_z] = settings.grid_size();
    FluidParams {
        dt: FLUID_STEP,
        gravity: settings.gravity,
        rest_density: settings.rest_density,
        relaxation: settings.relaxation,
        _reserved: 0.0,
        mass: settings.particle_mass(),
        smoothing_radius: settings.particle_radius,
        num_particles: pool,
        grid_size_x,
        grid_size_y,
        grid_size_z,
        cell_size: settings.particle_radius,
        bounds_min: settings.bounds_min,
        bounds_padding1: 0.0,
        bounds_max: settings.bounds_max,
        bounds_padding2: 0.0,
        mouse_pos: [0.0; 3],
        mouse_active: 0.0,
        mouse_dir: [0.0; 3],
        mouse_radius: 5.0,
        num_colliders: 0,
        cohesion: settings.cohesion,
        time: 0.0,
        vorticity_strength: settings.vorticity,
        surface_tension: settings.surface_tension,
        viscosity_laplacian: settings.viscosity,
        xsph_factor: settings.xsph,
        solver_iterations: settings.solver_iterations,
    }
}

/// `pool` particles of `settings`' fluid, at rest: the first [`FluidSettings::particles`] laid
/// out at the rest spacing from the bottom of the box up, filling it side to side — water standing
/// in its tank. The rest of the pool is **not yet placed** (`phase` `0xFFFFFFFF`): a caller that
/// steps more particles than the settings hold — the `fluid` demo pours that way — has the solver
/// place each on its first step, at the pointer while it is pushing, else over the middle of the
/// box near its top.
pub fn lattice(settings: &FluidSettings, pool: u32) -> Vec<FluidParticle> {
    let spacing = settings.spacing().max(1e-4);
    let pad = settings.particle_radius * 0.5 + spacing * 0.5;
    let (min, max) = (settings.bounds_min, settings.bounds_max);
    let across = |i: usize| (((max[i] - min[i]) - 2.0 * pad) / spacing).floor().max(1.0) as usize;
    let (nx, nz) = (across(0), across(2));
    (0..pool as usize)
        .map(|i| {
            let (xi, zi, yi) = (i % nx, (i / nx) % nz, i / (nx * nz));
            let p = [min[0] + pad + xi as f32 * spacing, min[1] + pad + yi as f32 * spacing, min[2] + pad + zi as f32 * spacing];
            let p = [p[0], p[1].min(max[1] - pad), p[2]];
            FluidParticle {
                position: p,
                density: settings.rest_density,
                velocity: [0.0; 3],
                lambda: 0.0,
                predicted_position: p,
                phase: if (i as u32) < settings.particles { 0 } else { 0xFFFF_FFFF },
                vorticity: [0.0; 3],
                _pad_vort: 0.0,
            }
        })
        .collect()
}

#[cfg(test)]
mod gpu_dispatch_tests {
    use super::*;

    async fn setup_headless_gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
        crate::test_gpu::headless_device().await
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
        let data = slice.get_mapped_range()
            // wgpu 30 made this fallible; the range is the whole buffer we just mapped, so a
            // failure here is a programming error rather than a runtime condition.
            .expect("a just-mapped buffer's full range is always valid");
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
        // Guard testin tamamı boyunca: aynı anda birden fazla canlı cihaz sürücüyü
        // düşürüyor (bkz. `crate::test_gpu`).
        let _gpu = crate::test_gpu::gpu_lock();
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
        // Guard testin tamamı boyunca: aynı anda birden fazla canlı cihaz sürücüyü
        // düşürüyor (bkz. `crate::test_gpu`).
        let _gpu = crate::test_gpu::gpu_lock();
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
