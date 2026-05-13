use gizmo_math::{Ray, Vec3, Vec2, Mat4, Quat};

fn main() {
    let aspect = 1600.0 / 900.0;
    let camera_pos = Vec3::new(0.0, 8.0, 18.0);
    
    let q_yaw = Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), -std::f32::consts::FRAC_PI_2);
    let q_pitch = Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), -0.4);
    let rotation = q_yaw * q_pitch;
    
    // Simulating get_view:
    let pitch = -0.4f32;
    let yaw = -std::f32::consts::FRAC_PI_2;
    let fx = yaw.cos() * pitch.cos();
    let fy = pitch.sin();
    let fz = yaw.sin() * pitch.cos();
    let front = Vec3::new(fx, fy, fz).normalize();
    let right = Vec3::new(-yaw.sin(), 0.0, yaw.cos());
    let up = right.cross(front);
    let view = Mat4::look_at_rh(camera_pos, camera_pos + front, up);
    let proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, aspect, 0.1, 100.0);
    
    let inv_vp = (proj * view).inverse();
    let near_vec = inv_vp.project_point3(Vec3::new(0.0, 0.0, 0.0));
    let far_vec = inv_vp.project_point3(Vec3::new(0.0, 0.0, 1.0));
    
    let world_dir = (far_vec - near_vec).normalize();
    let ray = Ray::new(near_vec, world_dir);
    
    println!("Ray origin: {:?}", ray.origin);
    println!("Ray direction: {:?}", ray.direction);
    
    let hit = ray.intersect_obb(Vec3::ZERO, Vec3::ONE, Quat::IDENTITY);
    println!("Hit: {:?}", hit);
}
