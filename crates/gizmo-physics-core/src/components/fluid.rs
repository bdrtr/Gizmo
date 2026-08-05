//! Authoring parameters for a smoothed-particle-hydrodynamics (SPH) fluid volume.

use gizmo_math::Vec3;

/// The tunables of an SPH fluid volume attached to an entity.
///
/// The defaults describe water at rest: 1000 kg/m³, 0.1 m particles, a light viscosity, and a
/// 20 × 10 × 20 m box whose floor sits on the `y = 0` plane. Nothing here is validated or
/// clamped — the type is a parameter block, not a guard.
///
/// This is authoring data at the moment: in this workspace the editor's inspector is the only
/// code that reads these fields, so changing them does not steer a running solver.
///
/// Being `#[non_exhaustive]`, downstream crates cannot build it with a struct literal (not
/// even with `..Default::default()`); start from [`FluidSimulation::default()`](Default) and
/// assign the fields you care about.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct FluidSimulation {
    /// Rest density ρ₀ in kg/m³ — the density the fluid is meant to settle at, and the zero
    /// point that pressure is measured against rather than a cap. 1000 is fresh water.
    pub target_density: f32,
    /// Stiffness scaling the pressure response to a density error. Higher values resist
    /// compression harder at the cost of a stiffer system that needs smaller steps to stay
    /// stable. A tuning knob, not a physical constant in Pa.
    pub pressure_multiplier: f32,
    /// Viscosity coefficient — a tuning knob, not a dynamic viscosity calibrated in Pa·s.
    /// `0.0` is inviscid: neighbouring particles never exchange momentum through shear.
    pub viscosity: f32,
    /// Radius of a single particle, in metres, and so the resolution knob: halving it
    /// multiplies the particle count needed to fill the same volume by roughly eight.
    pub particle_radius: f32,
    /// Lower corner, in metres, of the world-space AABB the fluid is confined to. The
    /// default `y` of 0 puts the floor of the domain on the ground plane.
    pub bounds_min: Vec3,
    /// Upper corner of that same AABB. Expected to be componentwise ≥
    /// [`bounds_min`](Self::bounds_min); an inverted or flat box describes an empty domain
    /// and is neither rejected nor normalised here.
    pub bounds_max: Vec3,
}

impl Default for FluidSimulation {
    fn default() -> Self {
        Self {
            target_density: 1000.0,
            pressure_multiplier: 100.0,
            viscosity: 0.01,
            particle_radius: 0.1,
            bounds_min: Vec3::new(-10.0, 0.0, -10.0),
            bounds_max: Vec3::new(10.0, 10.0, 10.0),
        }
    }
}

#[cfg(feature = "ecs")]
gizmo_core::impl_component!(FluidSimulation);
