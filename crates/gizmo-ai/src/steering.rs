use gizmo_math::Vec3;

/// Uygulanacak gücü sınırla (Manevra kabiliyeti için performanslı kare kök limiti)
#[inline]
pub fn clamp_force(v: Vec3, max_force: f32) -> Vec3 {
    if v.length_squared() > max_force * max_force {
        v.normalize() * max_force
    } else {
        v
    }
}

/// Bir objeyi hedef vektörüne doğru yönlendiren kuvveti hesaplar (Seek Steering)
pub fn seek(
    current_pos: Vec3,
    target_pos: Vec3,
    current_vel: Vec3,
    max_speed: f32,
    max_force: f32,
) -> Vec3 {
    let to_target = target_pos - current_pos;
    if to_target.length_squared() < f32::EPSILON {
        return Vec3::ZERO;
    }

    let desired_velocity = to_target.normalize() * max_speed;
    let steering = desired_velocity - current_vel;
    clamp_force(steering, max_force)
}

/// Hedefe yaklaşınca yavaşlayarak duran Steering (Arrive)
pub fn arrive(
    current_pos: Vec3,
    target_pos: Vec3,
    current_vel: Vec3,
    max_speed: f32,
    max_force: f32,
    slowing_radius: f32,
) -> Vec3 {
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
    clamp_force(steering, max_force)
}

/// Engellerden kaçınma (Obstacle Avoidance) Steering
/// `obstacles`: tuple array of `(center_position, avoidance_radius)`
pub fn avoid_obstacles(
    current_pos: Vec3,
    current_vel: Vec3,
    obstacles: &[(Vec3, f32)],
    max_speed: f32,
    max_force: f32,
) -> Vec3 {
    let mut desired_velocity = Vec3::ZERO;
    let mut count = 0;

    for &(obs_pos, obs_radius) in obstacles {
        let diff = current_pos - obs_pos;
        let dist = diff.length();

        if dist > f32::EPSILON && dist < obs_radius {
            // Engele yaklaşıldıkça kaçış kuvveti artar
            let force = diff.normalize() / dist;
            desired_velocity += force;
            count += 1;
        }
    }

    if count > 0 {
        // Ortalamaya bölmek yönü bozmaz ama length()'i bozar. normalize() ederek saf yönü yakalarız.
        desired_velocity = desired_velocity.normalize_or_zero() * max_speed;
        let steering = desired_velocity - current_vel;
        return clamp_force(steering, max_force);
    }

    Vec3::ZERO
}

/// Grup dinamiği: Bireylerin birbirinden uzaklaşması (Separation)
pub fn separate(
    current_pos: Vec3,
    current_vel: Vec3,
    neighbors: &[Vec3],
    separation_radius: f32,
    max_speed: f32,
    max_force: f32,
) -> Vec3 {
    let mut desired_velocity = Vec3::ZERO;
    let mut count = 0;

    for &neighbor in neighbors {
        let diff = current_pos - neighbor;
        let dist = diff.length();

        if dist > f32::EPSILON && dist < separation_radius {
            let force = diff.normalize() / dist;
            desired_velocity += force;
            count += 1;
        }
    }

    if count > 0 {
        desired_velocity = desired_velocity.normalize_or_zero() * max_speed;
        let steering = desired_velocity - current_vel;
        return clamp_force(steering, max_force);
    }

    Vec3::ZERO
}

/// Grup dinamiği: Bireylerin çevresindeki grupların merkezine gitmesi (Cohesion)
pub fn cohesion(
    current_pos: Vec3,
    current_vel: Vec3,
    neighbors: &[Vec3],
    cohesion_radius: f32,
    max_speed: f32,
    max_force: f32,
) -> Vec3 {
    let mut center = Vec3::ZERO;
    let mut count = 0;

    for &neighbor in neighbors {
        let dist = (current_pos - neighbor).length();
        if dist > f32::EPSILON && dist < cohesion_radius {
            center += neighbor;
            count += 1;
        }
    }

    if count > 0 {
        center /= count as f32;
        // Merkez noktasına seek uygularız
        return seek(current_pos, center, current_vel, max_speed, max_force);
    }

    Vec3::ZERO
}

/// Grup dinamiği: Bireylerin çevresindeki grubun hız/gidişat yönüne ayak uydurması (Alignment)
pub fn alignment(
    current_pos: Vec3,
    current_vel: Vec3,
    neighbors: &[(Vec3, Vec3)], // (neighbor_pos, neighbor_vel)
    alignment_radius: f32,
    max_speed: f32,
    max_force: f32,
) -> Vec3 {
    let mut avg_vel = Vec3::ZERO;
    let mut count = 0;

    for &(neighbor_pos, neighbor_vel) in neighbors {
        let dist = (current_pos - neighbor_pos).length();
        if dist > f32::EPSILON && dist < alignment_radius {
            avg_vel += neighbor_vel;
            count += 1;
        }
    }

    if count > 0 {
        avg_vel /= count as f32;
        let desired_velocity = avg_vel.normalize_or_zero() * max_speed;
        let steering = desired_velocity - current_vel;
        return clamp_force(steering, max_force);
    }

    Vec3::ZERO
}

#[derive(Debug, Clone, PartialEq)]
pub struct SteeringWeights {
    pub seek: f32,
    pub arrive: f32,
    pub avoid: f32,
    pub separate: f32,
    pub cohesion: f32,
    pub alignment: f32,
}

impl Default for SteeringWeights {
    fn default() -> Self {
        Self {
            seek: 1.0,
            arrive: 1.0,
            avoid: 5.0,
            separate: 1.5,
            cohesion: 1.0,
            alignment: 1.0,
        }
    }
}

/// Bireysel davranışları dinamik olarak harmanlayan birleşik steering mekanizması
#[allow(clippy::too_many_arguments)]
pub fn combined_steering(
    current_pos: Vec3,
    current_vel: Vec3,
    target_pos: Option<Vec3>,
    obstacles: &[(Vec3, f32)],
    neighbors: &[(Vec3, Vec3)], // (pos, vel)
    weights: &SteeringWeights,
    max_speed: f32,
    max_force: f32,
    radii: (f32, f32, f32), // (separate, cohesion, alignment)
) -> Vec3 {
    let mut total_force = Vec3::ZERO;

    if let Some(target) = target_pos {
        total_force += seek(current_pos, target, current_vel, max_speed, max_force) * weights.seek;
    }

    if weights.avoid > 0.0 && !obstacles.is_empty() {
        total_force += avoid_obstacles(
            current_pos,
            current_vel,
            obstacles,
            max_speed,
            max_force,
        ) * weights.avoid;
    }

    if !neighbors.is_empty() {
        let (sep_r, coh_r, align_r) = radii;
        if weights.separate > 0.0 || weights.cohesion > 0.0 {
            let neighbor_positions: Vec<Vec3> = neighbors.iter().map(|n| n.0).collect();
            if weights.separate > 0.0 {
                total_force += separate(
                    current_pos,
                    current_vel,
                    &neighbor_positions,
                    sep_r,
                    max_speed,
                    max_force,
                ) * weights.separate;
            }
            if weights.cohesion > 0.0 {
                total_force += cohesion(
                    current_pos,
                    current_vel,
                    &neighbor_positions,
                    coh_r,
                    max_speed,
                    max_force,
                ) * weights.cohesion;
            }
        }
        if weights.alignment > 0.0 {
            total_force += alignment(
                current_pos,
                current_vel,
                neighbors,
                align_r,
                max_speed,
                max_force,
            ) * weights.alignment;
        }
    }

    clamp_force(total_force, max_force)
}
