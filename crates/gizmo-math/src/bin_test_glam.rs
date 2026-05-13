use gizmo_math::{Mat4, Vec4};

fn main() {
    let proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 1.0, 0.1, 100.0);
    let near_pt = proj * Vec4::new(0.0, 0.0, -0.1, 1.0);
    let far_pt = proj * Vec4::new(0.0, 0.0, -100.0, 1.0);
    
    println!("Near Z: {}", near_pt.z / near_pt.w);
    println!("Far Z: {}", far_pt.z / far_pt.w);
}
