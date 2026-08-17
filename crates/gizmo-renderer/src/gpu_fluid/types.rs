// ═══════════════════════════════════════════════════════════════════════
//  AAA Fluid GPU Types — Rust ↔ WGSL mirrored structs
// ═══════════════════════════════════════════════════════════════════════

/// One SPH particle, exactly as `fluid_compute.wgsl` declares it.
///
/// Each `vec3` is paired with the scalar that fills its 16-byte WGSL slot, so the struct is 64
/// bytes with no wasted space — at the hundred-thousand-particle scale this simulation runs at,
/// the padding would otherwise cost more memory than the velocities do.
///
/// The solver is position-based (PBF): it predicts a position, then corrects it over several
/// iterations, and only then reads a velocity back out of the correction. That is why both a
/// position and a predicted position live here.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FluidParticle {
    /// World position at the end of the last step.
    pub position: [f32; 3],
    /// Density from the SPH kernel sum over this particle's neighbours.
    pub density: f32,
    /// Velocity, m/s.
    pub velocity: [f32; 3],
    /// The particle's PBF constraint multiplier for this iteration.
    pub lambda: f32,
    /// The position under correction; the solver iterates on this and commits it at the end.
    pub predicted_position: [f32; 3],
    /// Which fluid this particle belongs to. Particles of different phases interact but do not
    /// mix.
    pub phase: u32,
    /// Curl of the velocity field at this particle, for vorticity confinement — SPH damps small
    /// eddies away, and this is what feeds their energy back.
    pub vorticity: [f32; 3],
    /// Padding, completing the vorticity slot's 16 bytes.
    pub _pad_vort: f32,
}

/// One entry of the spatial-hash table: which grid cell a particle is in, and which particle it
/// is.
///
/// The table is sorted by `hash`, which is what turns "find my neighbours" from a scan of every
/// particle into a lookup of a handful of adjacent cells.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ParticleHash {
    /// The particle's grid cell hash — the sort key.
    pub hash: u32,
    /// Its index in the particle buffer.
    pub index: u32,
}

/// One stage of the bitonic sort over [`ParticleHash`].
///
/// A bitonic sort is a fixed sequence of compare-exchange passes; the GPU runs one dispatch per
/// `(k, j)` pair, and this block is how each dispatch is told which pair it is.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SortParams {
    /// The distance between the two elements each thread compares in this pass.
    pub j: u32,
    /// The size of the bitonic sequence being merged.
    pub k: u32,
    /// Padding to the uniform block's 16-byte alignment.
    pub _pad0: u32,
    /// Padding, as above.
    pub _pad1: u32,
}

/// An obstacle the fluid is pushed out of — a sphere or an axis-aligned box.
///
/// Carries its own velocity, so a moving collider drags the fluid with it rather than teleporting
/// particles to its surface.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FluidCollider {
    /// World-space centre.
    pub position: [f32; 3],
    /// Sphere radius. Read only when [`Self::shape_type`] is 0.
    pub radius: f32,
    /// How fast the collider is moving, m/s.
    pub velocity: [f32; 3],
    /// 0 = sphere, 1 = AABB.
    pub shape_type: u32,
    /// Half extents of the box. Read only when [`Self::shape_type`] is 1.
    pub half_extents: [f32; 3],
    /// Padding, completing the last 16-byte slot.
    pub _pad: f32,
}

/// Every dial the fluid solver reads, in the one uniform block each of its passes binds.
///
/// The passes (predict, hash, sort, density, solve, viscosity, integrate) all bind this same
/// block, so a field is meaningful to some passes and ignored by others.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FluidParams {
    /// The timestep, in seconds.
    pub dt: f32,
    /// Downward acceleration, m/s² (a scalar: this fluid's gravity is always -Y).
    pub gravity: f32,
    /// The density the fluid relaxes towards; the PBF constraint drives every particle here.
    pub rest_density: f32,
    /// Stiffness relating a density error to a pressure.
    pub gas_constant: f32,
    /// Velocity-diffusion strength — how much a particle is dragged along by its neighbours.
    pub viscosity: f32,
    /// The mass of a single particle.
    pub mass: f32,
    /// The SPH kernel's support radius: past this, particles do not see each other. It is also
    /// what [`Self::cell_size`] must match, or the neighbour search misses particles the kernel
    /// wanted.
    pub smoothing_radius: f32,
    /// How many entries of the particle buffer are live.
    pub num_particles: u32,
    /// Cells along X in the neighbour-search grid.
    pub grid_size_x: u32,
    /// Cells along Y.
    pub grid_size_y: u32,
    /// Cells along Z.
    pub grid_size_z: u32,
    /// The side length of one grid cell.
    pub cell_size: f32,
    /// Minimum corner of the simulation box; particles are clamped inside it.
    pub bounds_min: [f32; 3],
    /// Padding, completing that slot.
    pub bounds_padding1: f32,
    /// Maximum corner of the simulation box.
    pub bounds_max: [f32; 3],
    /// Padding, completing that slot.
    pub bounds_padding2: f32,

    /// Where the interaction cursor is, in world space.
    pub mouse_pos: [f32; 3],
    /// Non-zero while the cursor is pushing the fluid.
    pub mouse_active: f32,
    /// The direction the cursor pushes.
    pub mouse_dir: [f32; 3],
    /// How far from [`Self::mouse_pos`] that push reaches.
    pub mouse_radius: f32,

    /// How many entries of the collider buffer are live (at most [`MAX_FLUID_COLLIDERS`]).
    pub num_colliders: u32,
    /// Attraction between neighbouring particles — what makes a stream hold together instead of
    /// dispersing.
    pub cohesion: f32,
    /// Elapsed seconds, for anything the solver animates.
    pub time: f32,
    /// Vorticity-confinement strength (ε): how much of the damped-away rotation is fed back.
    pub vorticity_strength: f32,

    /// Surface-tension coefficient (γ) — what rounds a droplet.
    pub surface_tension: f32,
    /// Laplacian viscosity coefficient (μ), the second viscosity term.
    pub viscosity_laplacian: f32,
    /// XSPH velocity-smoothing factor (c): how far each particle's velocity is blended towards its
    /// neighbourhood average.
    pub xsph_factor: f32,
    /// How many times the density constraint is solved per step. More is stiffer and slower.
    pub solver_iterations: u32,
}

/// The collider buffer's fixed capacity. [`FluidParams::num_colliders`] says how much of it is
/// live.
pub const MAX_FLUID_COLLIDERS: usize = 256;
