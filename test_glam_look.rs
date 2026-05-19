use glam::{Mat4, Vec3, Vec4};
fn main() {
    let pos = Vec3::new(2.0, 2.0, 2.0);
    // +X face
    let view = Mat4::look_to_rh(pos, Vec3::X, Vec3::Y);
    // Point at 3, 2, 2 (which is +1 in X direction from pos)
    let p = Vec4::new(3.0, 2.0, 2.0, 1.0);
    let p_v = view * p;
    println!("+X view applied to (3,2,2) = {:?}", p_v);
    
    // -X face
    let view_nx = Mat4::look_to_rh(pos, Vec3::NEG_X, Vec3::Y);
    let p_nx = Vec4::new(1.0, 2.0, 2.0, 1.0);
    let p_vnx = view_nx * p_nx;
    println!("-X view applied to (1,2,2) = {:?}", p_vnx);
}
