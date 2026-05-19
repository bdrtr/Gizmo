fn main() {
    let proj = gizmo_math::Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 1.0, 0.1, 100.0);
    println!("proj: {:?}", proj);
    let v_near = proj * gizmo_math::Vec4::new(0.0, 0.0, -0.1, 1.0);
    println!("z_near NDC: {}", v_near.z / v_near.w);
    let v_far = proj * gizmo_math::Vec4::new(0.0, 0.0, -100.0, 1.0);
    println!("z_far NDC: {}", v_far.z / v_far.w);
}
