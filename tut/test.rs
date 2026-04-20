use std::f32::consts::PI;

fn main() {
    // We want to map Local Z (0, 0, 1) to Tangent (+X) -> (1, 0, 0).
    let local_z = (0.0_f32, 0.0_f32, 1.0_f32);
    
    // Test dir_angle logic
    let dx = 1.0_f32; // Tangent goes right
    let dz = 0.0_f32; 
    let dir_angle = dx.atan2(dz);
    
    let sin_a = dir_angle.sin();
    let cos_a = dir_angle.cos();
    
    // Rotate local_z by dir_angle around Y
    // Quat rotation: x' = x*cos + z*sin, z' = -x*sin + z*cos
    let rx = local_z.0 * cos_a + local_z.2 * sin_a;
    let rz = -local_z.0 * sin_a + local_z.2 * cos_a;
    
    println!("Tangent: ({}, {}) -> Angle: {}", dx, dz, dir_angle);
    println!("Rotated Z: ({}, {})", rx, rz);
}
