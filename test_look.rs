fn main() {
    let proj = gizmo_math::Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 1.0, 0.1, 100.0);
    let view = gizmo_math::Mat4::look_to_rh(gizmo_math::Vec3::new(4.0, 8.0, 4.0), gizmo_math::Vec3::NEG_X, gizmo_math::Vec3::Y);
    let view_proj = proj * view;
    
    let object_pos = gizmo_math::Vec3::new(0.0, 0.5, 0.0);
    let clip_pos = view_proj * object_pos.extend(1.0);
    
    println!("Face -X clip_pos: {:?}", clip_pos);
    println!("Face -X ndc: {:?}", clip_pos / clip_pos.w);
}
