fn main() {
    let proj = glam::Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 1.0, 0.1, 100.0);
    println!("proj: {:?}", proj);
    let p = glam::Vec4::new(0.0, 0.0, -10.0, 1.0);
    let clip = proj * p;
    println!("Z=-10.0 clip: {:?}", clip);
    println!("Z=-10.0 ndc: {:?}", clip.z / clip.w);
}
