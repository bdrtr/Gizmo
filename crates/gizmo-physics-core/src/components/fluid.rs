//! A particle fluid in the scene: what the engine's GPU fluid solver simulates.

use gizmo_math::Vec3;

/// **The scene's fluid.** An entity carrying one turns the GPU fluid on and says what it is; with
/// none, there is no water. `gizmo`'s frame preparation hands it to the solver every frame, so an
/// edit — in code or in the inspector — takes effect on the next step.
///
/// The fields are the solver's own dials (a position-based fluid, Macklin & Müller 2013), named
/// for what they do. Until 2026-11-14 this type carried an SPH vocabulary — a "pressure multiplier"
/// and a viscosity the solver has no term for — that nothing read: the inspector was its only
/// reader, and whether a fluid ran at all was a renderer flag two demos set.
///
/// A new box, particle size or count lays the water out again, at rest on the floor of the box;
/// the other dials change the water that is there. One fluid per scene: a second is ignored, and
/// said so once.
///
/// Being `#[non_exhaustive]`, it is built from [`FluidSimulation::default()`](Default) with the
/// fields set that matter.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct FluidSimulation {
    /// Rest density ρ₀ in kg/m³ — what every particle is driven towards. 1000 is fresh water; it
    /// also sets a particle's mass, so particles at rest spacing are at it.
    pub target_density: f32,
    /// The density constraint's relaxation ε. **Lower is stiffer**: less squash under its own
    /// weight, and less stable when stepped hard. 600 by default.
    pub relaxation: f32,
    /// Viscosity μ — how much neighbouring particles share their motion. Honey is high.
    pub viscosity: f32,
    /// XSPH smoothing — how far each particle's velocity is pulled towards its neighbours'.
    pub xsph: f32,
    /// Vorticity confinement — how much of the swirl the solver's damping took is given back.
    pub vorticity: f32,
    /// Surface tension — what rounds a droplet.
    pub surface_tension: f32,
    /// Cohesion — what holds a stream together.
    pub cohesion: f32,
    /// Density-constraint iterations per step; more is stiffer and slower.
    pub solver_iterations: u32,
    /// Downward acceleration, m/s².
    pub gravity: f32,
    /// A particle's radius in metres, and so the resolution: halving it needs roughly eight times
    /// the particles for the same volume.
    pub particle_radius: f32,
    /// Lower corner, in metres, of the world-space box the fluid is held in.
    pub bounds_min: Vec3,
    /// Upper corner of that box.
    pub bounds_max: Vec3,
    /// How many particles — at most the renderer's pool (a hundred thousand on the desktop), and
    /// laid out from the floor of the box up.
    pub particles: u32,
}

impl Default for FluidSimulation {
    /// Water standing in a 10 × 10 × 10 m tank whose floor is the ground: a hundred thousand
    /// particles of 0.1 m, about half a metre deep.
    fn default() -> Self {
        Self {
            target_density: 1000.0,
            relaxation: 600.0,
            viscosity: 0.005,
            xsph: 0.05,
            vorticity: 0.35,
            surface_tension: 0.5,
            cohesion: 0.008,
            solver_iterations: 6,
            gravity: 9.81,
            particle_radius: 0.1,
            bounds_min: Vec3::new(-5.0, 0.0, -5.0),
            bounds_max: Vec3::new(5.0, 10.0, 5.0),
            particles: 100_000,
        }
    }
}

#[cfg(feature = "ecs")]
gizmo_core::impl_component!(FluidSimulation);
