use gizmo_math::Vec3;

/// A terrain surface generated from a greyscale heightmap image.
///
/// The component holds the *recipe*, not the mesh: the path and the three extents are what a scene
/// file stores, and the mesh is rebuilt from them on load. That is why it is `Serialize` while
/// [`Mesh`](super::mesh::Mesh), which owns GPU buffers, is not.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Terrain {
    /// Path to the heightmap image. Its red channel is read as the height, 0 → black, 1 → white.
    pub heightmap_path: String,
    /// The terrain's size along X, in metres.
    pub width: f32,
    /// Its size along Z, in metres.
    pub depth: f32,
    /// What a fully white texel means, in metres above the terrain's origin.
    pub max_height: f32,
}

impl Terrain {
    /// A terrain of the given size, sampled from `heightmap_path`.
    ///
    /// # Panics
    ///
    /// If any of the three extents is zero or negative — a degenerate terrain is a usage error,
    /// and it would otherwise surface as an invisible or infinitely-tall mesh much later.
    pub fn new(heightmap_path: String, width: f32, depth: f32, max_height: f32) -> Self {
        assert!(
            width > 0.0,
            "Kullanım hatası: Terrain width sıfırdan büyük olmalıdır."
        );
        assert!(
            depth > 0.0,
            "Kullanım hatası: Terrain depth sıfırdan büyük olmalıdır."
        );
        assert!(
            max_height > 0.0,
            "Kullanım hatası: Terrain max_height sıfırdan büyük olmalıdır."
        );
        Self {
            heightmap_path,
            width,
            depth,
            max_height,
        }
    }
}

/// Several meshes for one entity, chosen by distance — a level-of-detail group.
///
/// Attach it alongside the entity's own [`Mesh`](super::mesh::Mesh); the group **overrides** that
/// mesh rather than competing with it. See [`LodGroup::pick`], which is the single place both
/// render paths ask which mesh to draw.
#[derive(Clone)]
pub struct LodGroup {
    /// The levels, finest first — [`LodGroup::new`] sorts them by [`LodLevel::max_distance`].
    pub levels: Vec<LodLevel>,
}

/// One level of a [`LodGroup`]: a mesh, and the distance out to which it is used.
#[derive(Clone)]
pub struct LodLevel {
    /// The mesh drawn while the camera is within [`Self::max_distance`].
    pub mesh: super::mesh::Mesh,
    /// The far edge of this level's band, in metres. The band is closed at the top: a camera
    /// exactly this far away still gets this level.
    pub max_distance: f32,
}

impl LodLevel {
    /// A level drawn out to `max_distance` metres.
    ///
    /// # Panics
    ///
    /// If `max_distance` is negative — such a level can never be selected, so it is a silent
    /// invisibility bug rather than a configuration.
    pub fn new(mesh: super::mesh::Mesh, max_distance: f32) -> Self {
        assert!(
            max_distance >= 0.0,
            "Kullanım hatası: LodLevel max_distance negatif olamaz."
        );
        Self { mesh, max_distance }
    }
}

impl LodGroup {
    /// A group over `levels`, sorted into ascending distance order.
    ///
    /// The sort is done here so the selection can stop at the first level that covers a distance,
    /// and so a caller listing its levels coarsest-first still gets correct behaviour.
    ///
    /// # Panics
    ///
    /// If `levels` is empty — an empty group culls at every distance, which reads as the entity
    /// having vanished.
    pub fn new(levels: Vec<LodLevel>) -> Self {
        assert!(
            !levels.is_empty(),
            "Kullanım hatası: LodGroup en az 1 LodLevel içermelidir (Boş liste görünmezlik hatalarına yol açar)."
        );
        let mut levels = levels;
        levels.sort_by(|a, b| a.max_distance.total_cmp(&b.max_distance));
        Self { levels }
    }

    /// The mesh for `distance`, or `None` past the last level (cull).
    pub fn select_mesh(&self, distance: f32) -> Option<&super::mesh::Mesh> {
        self.select_level(distance).map(|i| &self.levels[i].mesh)
    }

    /// Which mesh an entity draws this frame: the LOD level for `distance` if it has a group,
    /// its own [`Mesh`](super::mesh::Mesh) if it does not, and `None` — cull — past the last
    /// level.
    ///
    /// Both render paths ask this question and each answered it inline, in eight lines that had
    /// to agree on all three cases: that a group **overrides** the entity's own mesh rather than
    /// competing with it, and that running off the end of the levels means "do not draw" rather
    /// than "draw the coarsest". They did agree, which is the good case and not a durable one —
    /// `LodGroup` was honoured only by the editor for a long time, so the engine's copy of this
    /// is younger than the feature.
    ///
    /// Takes the distance rather than the positions: the two paths reach the entity's world
    /// translation by different routes (a `GlobalTransform` here, the assembled model matrix
    /// there) and that part is genuinely theirs.
    pub fn pick<'a>(
        group: Option<&'a Self>,
        own_mesh: &'a super::mesh::Mesh,
        distance: f32,
    ) -> Option<&'a super::mesh::Mesh> {
        match group {
            Some(lod) => lod.select_mesh(distance),
            None => Some(own_mesh),
        }
    }

    /// Which level a distance falls in, or `None` past the last one.
    ///
    /// The policy on its own, separated from the mesh it picks. A [`super::mesh::Mesh`] owns
    /// `wgpu::Buffer`s and cannot be built without a device, so [`Self::select_mesh`] is not
    /// testable off a GPU — and this decision is worth testing, because the engine's own render
    /// pass now depends on it: the bounds decide what is drawn, and the `None` decides what is
    /// not drawn at all.
    #[must_use]
    pub fn select_level(&self, distance: f32) -> Option<usize> {
        Self::level_for(self.levels.iter().map(|l| l.max_distance), distance)
    }

    /// [`Self::select_level`] over bare bounds, so it can be tested without a GPU.
    ///
    /// The bounds are ascending — [`Self::new`] sorts them — so the first one the distance is
    /// within is the finest level that still covers it.
    fn level_for(bounds: impl Iterator<Item = f32>, distance: f32) -> Option<usize> {
        bounds
            .enumerate()
            .find(|(_, bound)| distance <= *bound)
            .map(|(i, _)| i)
    }
}

#[cfg(test)]
mod lod_tests {
    use super::*;

    fn pick(bounds: &[f32], distance: f32) -> Option<usize> {
        LodGroup::level_for(bounds.iter().copied(), distance)
    }

    /// Bands are closed at the top: a distance exactly on a bound belongs to that level, not the
    /// next one.
    #[test]
    fn a_distance_inside_a_band_picks_that_band() {
        let bands = [10.0, 50.0, 200.0];
        assert_eq!(pick(&bands, 0.0), Some(0));
        assert_eq!(pick(&bands, 9.9), Some(0));
        assert_eq!(pick(&bands, 10.0), Some(0), "a bound belongs to its own level");
        assert_eq!(pick(&bands, 10.1), Some(1));
        assert_eq!(pick(&bands, 200.0), Some(2));
    }

    #[test]
    fn past_the_last_band_is_culled_rather_than_drawn_coarsest() {
        assert_eq!(
            pick(&[10.0, 50.0, 200.0], 200.1),
            None,
            "past the last bound the object is culled — drawing the coarsest level instead would \
             make a LOD group a thing that can never disappear"
        );
    }
}

/// A CPU-driven particle emitter: how fast to spawn, and what each particle starts and ends as.
///
/// Distinct from the GPU particle system in [`crate::gpu_particles`] — this one spawns ECS
/// entities, so each particle is inspectable and scriptable, at a scale of hundreds rather than
/// hundreds of thousands.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct ParticleEmitter {
    /// Particles spawned per second.
    pub spawn_rate: f32,
    accumulator: f32,
    /// Where particles appear, relative to the emitter entity.
    pub local_offset: Vec3,
    /// The velocity a particle starts with, before randomisation.
    pub initial_velocity: Vec3,
    /// How far the initial velocity is randomised, as a fraction of itself. 0 = every particle
    /// launches identically.
    pub velocity_randomness: f32,
    /// How long a particle lives, in seconds.
    pub lifespan: f32,
    /// How far that lifespan is randomised, as a fraction. 0 = every particle lives exactly
    /// [`Self::lifespan`].
    pub lifespan_randomness: f32,
    /// Billboard size at spawn.
    pub size_start: f32,
    /// Billboard size at death; sizes interpolate over the particle's age.
    pub size_end: f32,
    /// Tint at spawn, RGBA.
    pub color_start: gizmo_math::Vec4,
    /// Tint at death. Fading out is done by ending at zero alpha.
    pub color_end: gizmo_math::Vec4,
    /// The texture each particle is drawn with; `None` draws an untextured billboard.
    pub texture_source: Option<String>,
    /// While false the emitter spawns nothing. Existing particles still live out their lifespan.
    pub is_active: bool,
}

impl Default for ParticleEmitter {
    fn default() -> Self {
        Self::new()
    }
}

impl ParticleEmitter {
    /// An emitter with the default fire-like preset: 10 particles a second, rising, fading from
    /// orange to transparent red over two seconds.
    pub fn new() -> Self {
        Self {
            spawn_rate: 10.0,
            accumulator: 0.0,
            local_offset: Vec3::ZERO,
            initial_velocity: Vec3::new(0.0, 1.0, 0.0),
            velocity_randomness: 0.5,
            lifespan: 2.0,
            lifespan_randomness: 0.5,
            size_start: 0.5,
            size_end: 0.1,
            color_start: gizmo_math::Vec4::new(1.0, 0.5, 0.1, 1.0),
            color_end: gizmo_math::Vec4::new(1.0, 0.2, 0.0, 0.0),
            texture_source: None,
            is_active: true,
        }
    }

    /// Creates a new particle emitter, validating its parameters.
    pub fn with_rate(spawn_rate: f32) -> Self {
        assert!(
            spawn_rate > 0.0,
            "Kullanım hatası: ParticleEmitter spawn_rate sıfırdan büyük olmalıdır."
        );
        Self {
            spawn_rate,
            ..Self::default()
        }
    }

    /// The spawn accumulator: seconds of emission owed but not yet spawned.
    ///
    /// Spawning is fractional — at 10 particles a second a 16 ms frame owes 0.16 of a particle —
    /// and this is where that fraction is kept, so the rate stays right at any frame rate instead
    /// of rounding down to zero every frame.
    pub fn get_accumulator(&self) -> f32 {
        self.accumulator
    }

    /// Adds a frame's worth of time to the spawn accumulator.
    pub fn add_time(&mut self, dt: f32) {
        self.accumulator += dt;
    }

    /// Takes one particle's worth of time back out of the accumulator, after spawning it.
    pub fn consume_time(&mut self, dt: f32) {
        self.accumulator -= dt;
    }
}

/// An off-screen texture a camera renders into, instead of the window.
///
/// The view is behind an `Arc` because the same target is held by the renderer that writes it and
/// by whatever displays it (the editor's Game panel binds this exact view as an egui texture).
#[derive(Clone)]
pub struct RenderTarget {
    /// The texture view rendered into.
    pub view: std::sync::Arc<wgpu::TextureView>,
    /// Its width in pixels.
    pub width: u32,
    /// Its height in pixels.
    pub height: u32,
}

/// The scene view's render target — what the editor's own camera draws into.
///
/// A newtype rather than a bare [`RenderTarget`] so the two editor viewports can be told apart as
/// resources: one world holds both, and they must not be confused.
#[derive(Clone)]
pub struct EditorRenderTarget(pub RenderTarget);

/// The game view's render target — what the scene's own camera draws into while the editor is
/// playing. See [`EditorRenderTarget`] for why this is a newtype.
#[derive(Clone)]
pub struct GameRenderTarget(pub RenderTarget);

/// What the last frame actually cost, published by whichever render path drew it.
///
/// The editor's viewport overlay reads this. It exists because the numbers it shows have to be
/// *measured*: `StudioState::draw_call_count` was a field nothing ever wrote, and an overlay that
/// prints a plausible-looking zero is worse than one that prints nothing.
///
/// Frame time is deliberately absent — that belongs to `gizmo_core::FrameProfiler`, which already
/// measures it for every configuration, editor or not.
// No `Eq`: the sample timestamp is a float. `PartialEq` is enough for the one comparison anyone
// makes of these — "did the frame change" in a test.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RenderStats {
    /// Draw calls recorded for the main pass — one per batch that reached the pass.
    pub draw_calls: u32,
    /// Triangles submitted, counting instances: `Σ (indices / 3) × instances`.
    pub triangles: u32,
    /// Instances uploaded this frame, across all batches.
    pub instances: u32,
    /// GPU memory wgpu has allocated, in bytes — `None` when the backend does not report it.
    ///
    /// This is the allocator's view (`Device::generate_allocator_report`), not the driver's total
    /// VRAM: it counts what this process has sub-allocated, which is the number an engine can
    /// actually act on. Labelled accordingly wherever it is shown, because "VRAM" would imply the
    /// card's usage including every other process.
    ///
    /// `None` is a real answer and must stay one: some backends never report, and a zero would
    /// read as "nothing allocated".
    pub gpu_allocated_bytes: Option<u64>,
    /// When the sample above was taken, in seconds of `Time::elapsed`.
    ///
    /// The report walks every live allocation, so it is sampled about once a second rather than
    /// per frame; this is what the sampler compares against.
    pub gpu_sampled_at: f32,
}


/// Which fluid a particle belongs to. Phases interact but do not mix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FluidPhaseType {
    /// The bulk liquid.
    Water,
    /// Aerated surface foam — lighter, and shorter-lived.
    Foam,
    /// A gas bubble inside the liquid, which rises rather than falls.
    Bubble,
}

/// The ECS marker component saying an entity is a fluid particle.
/// In the simulation it can be used together with FluidHandle, FluidPhase and
/// FluidInteractor.
#[derive(Clone)]
pub struct FluidParticle;

/// An ECS entity's link to its particle in the GPU fluid buffer.
#[derive(Clone)]
pub struct FluidHandle {
    /// The particle's index in that buffer.
    pub gpu_index: u32,
}

/// Which [`FluidPhaseType`] a fluid particle entity belongs to.
#[derive(Clone)]
pub struct FluidPhase {
    /// The phase.
    pub phase: FluidPhaseType,
}

/// Makes an entity interact with the fluid: it displaces particles, and they push back.
///
/// Both directions, which is what separates this from a plain collider — the entity is pushed out
/// of the fluid by [`Self::buoyancy_factor`] while pushing the fluid out of its own volume.
#[derive(Clone)]
pub struct FluidInteractor {
    /// This entity's slot in the fluid solver's collider buffer.
    pub collider_gpu_index: u32,
    /// How strongly the fluid lifts this entity. 0 = sinks, 1 = neutral, above = floats up.
    pub buoyancy_factor: f32,
    /// The radius of the sphere the fluid is pushed out of.
    pub radius: f32,
    /// How fast the interactor is moving, so it drags the fluid along instead of teleporting
    /// through it.
    pub velocity: gizmo_math::Vec3,
}
