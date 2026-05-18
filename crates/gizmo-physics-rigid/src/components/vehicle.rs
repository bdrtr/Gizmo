use gizmo_math::Vec3;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Wheel {
    /// Local attachment point of the wheel suspension to the chassis
    pub local_position: Vec3,
    /// Direction the suspension ray is cast (usually -Y, relative to chassis)
    pub direction: Vec3,
    /// Radius of the tire
    pub radius: f32,
    /// Suspension rest length (maximum droop)
    pub suspension_rest_length: f32,
    /// Suspension stiffness (spring constant k)
    pub suspension_stiffness: f32,
    /// Suspension damping (shock absorber c)
    pub suspension_damping: f32,
    
    /// Does this wheel steer?
    pub is_steering: bool,
    /// Does this wheel receive engine power?
    pub is_drive: bool,
    
    /// Base grip factor (how much lateral force before slipping)
    pub base_grip: f32,
    pub drift_grip: f32,
    pub slip_threshold: f32,
    
    /// Coefficient of rolling resistance (Crr)
    pub rolling_resistance_coefficient: f32,
    
    // Internal state
    #[serde(skip)]
    pub is_grounded: bool,
    #[serde(skip)]
    pub suspension_compression: f32,
    #[serde(skip)]
    pub contact_point: Vec3,
    #[serde(skip)]
    pub contact_normal: Vec3,
    #[serde(skip)]
    pub slip_angle: f32,
}

impl Default for Wheel {
    fn default() -> Self {
        Self {
            local_position: Vec3::ZERO,
            direction: Vec3::new(0.0, -1.0, 0.0),
            radius: 0.4,
            suspension_rest_length: 0.5,
            suspension_stiffness: 40000.0,
            suspension_damping: 3000.0,
            is_steering: false,
            is_drive: false,
            base_grip: 15.0,
            drift_grip: 5.0,
            slip_threshold: 4.0,
            rolling_resistance_coefficient: 0.015,
            is_grounded: false,
            suspension_compression: 0.0,
            contact_point: Vec3::ZERO,
            contact_normal: Vec3::Y,
            slip_angle: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Gearbox {
    /// Gear ratios (e.g., [3.0, 2.0, 1.5, 1.0, 0.8])
    pub gears: Vec<f32>,
    /// Reverse gear ratio
    pub reverse_ratio: f32,
    /// Final drive multiplier
    pub final_drive: f32,
    /// Current gear index (0-based)
    pub current_gear: usize,
    /// Whether shifting happens automatically based on speed
    pub is_automatic: bool,
    /// Speeds at which to shift up (m/s)
    pub shift_up_speeds: Vec<f32>,
    /// Speeds at which to shift down (m/s)
    pub shift_down_speeds: Vec<f32>,
    /// True if the car is in reverse
    pub is_reversing: bool,
}

impl Default for Gearbox {
    fn default() -> Self {
        Self {
            gears: vec![3.0, 2.0, 1.5, 1.1, 0.85, 0.65],
            reverse_ratio: 3.0,
            final_drive: 3.5,
            current_gear: 0,
            is_automatic: true,
            shift_up_speeds: vec![15.0, 30.0, 45.0, 60.0, 75.0],
            shift_down_speeds: vec![10.0, 20.0, 35.0, 50.0, 65.0],
            is_reversing: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Vehicle {
    pub wheels: Vec<Wheel>,
    
    /// Engine power (throttle multiplier)
    pub engine_power: f32,
    /// Brake force multiplier
    pub brake_force: f32,
    /// Maximum steering angle (radians)
    pub max_steer_angle: f32,
    
    /// Vehicle Gearbox
    pub gearbox: Gearbox,
    
    /// Aerodynamic Drag coefficient (Cd)
    pub aerodynamic_drag: f32,
    /// Frontal Area (m^2)
    pub frontal_area: f32,
    /// Downforce coefficient (Cl)
    pub downforce_coefficient: f32,
    
    /// Current normalized throttle input [-1.0, 1.0]
    #[serde(skip)]
    pub current_throttle: f32,
    /// Current steering angle (radians)
    #[serde(skip)]
    pub current_steer: f32,
    /// Current brake input [0.0, 1.0]
    #[serde(skip)]
    pub current_brake: f32,
}

impl Default for Vehicle {
    fn default() -> Self {
        Self {
            wheels: Vec::new(),
            engine_power: 10000.0,
            brake_force: 5000.0,
            max_steer_angle: 0.5,
            gearbox: Gearbox::default(),
            aerodynamic_drag: 0.3,
            frontal_area: 2.2,
            downforce_coefficient: 0.5,
            current_throttle: 0.0,
            current_steer: 0.0,
            current_brake: 0.0,
        }
    }
}

gizmo_core::impl_component!(Vehicle);
