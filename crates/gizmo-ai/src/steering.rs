use gizmo_math::Vec3;

/// Bir objeyi hedef vektörüne doğru yönlendiren kuvveti hesaplar (Seek Steering)
pub fn seek(current_pos: Vec3, target_pos: Vec3, current_vel: Vec3, max_speed: f32, max_force: f32) -> Vec3 {
    let desired_velocity = (target_pos - current_pos).normalize_or_zero() * max_speed;
    let steering = desired_velocity - current_vel;
    
    // Uygulanacak gücü sınırla (Manevra kabiliyeti)
    if steering.length() > max_force {
        steering.normalize() * max_force
    } else {
        steering
    }
}

/// Hedefe yaklaşınca yavaşlayarak duran Steering (Arrive)
pub fn arrive(current_pos: Vec3, target_pos: Vec3, current_vel: Vec3, max_speed: f32, max_force: f32, slowing_radius: f32) -> Vec3 {
    let to_target = target_pos - current_pos;
    let distance = to_target.length();
    
    if distance < 0.01 {
        return Vec3::ZERO;
    }

    // Yavaşlama çemberi içinde hızımızı mesafeye orantılı düşürüyoruz
    let desired_speed = if distance < slowing_radius {
        max_speed * (distance / slowing_radius)
    } else {
        max_speed
    };

    let desired_velocity = (to_target / distance) * desired_speed;
    let steering = desired_velocity - current_vel;

    if steering.length() > max_force {
        steering.normalize() * max_force
    } else {
        steering
    }
}
