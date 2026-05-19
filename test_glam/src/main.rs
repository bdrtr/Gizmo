fn main() {
    // Generate right-handed matrices for a cubemap
    // We want the resulting face mapping to match WebGPU's textureSampleCube
    
    // Face 0: +X. Right: -Z, Top: +Y (Wait, is Top +Y or -Y?)
    // Let's print out the axes.
    let up = glam::Vec3::Y;
    let faces = [
        (glam::Vec3::X, up),
        (glam::Vec3::NEG_X, up),
        (glam::Vec3::Y, glam::Vec3::Z),
        (glam::Vec3::NEG_Y, glam::Vec3::NEG_Z),
        (glam::Vec3::Z, up),
        (glam::Vec3::NEG_Z, up),
    ];
    for (i, (dir, up)) in faces.iter().enumerate() {
        let right = dir.cross(*up);
        println!("Face {}: dir={:?}, up={:?}, right={:?}", i, dir, up, right);
    }
}
