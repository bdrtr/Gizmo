use gizmo_math::Vec3;

#[derive(Debug, Clone)]
pub struct FluidSimulation {
    pub target_density: f32,
    pub pressure_multiplier: f32,
    pub viscosity: f32,
    pub particle_radius: f32,
    pub bounds_min: Vec3,
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

gizmo_core::impl_component!(FluidSimulation);
