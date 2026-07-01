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
            // Engele yaklaşıldıkça kaçış kuvveti artar (ters-kare yasası ile yakınlık ölçeklenir)
            let force = diff / (dist * dist);
            desired_velocity += force;
            count += 1;
        }
    }

    if count > 0 {
        // Biriken kuvvet hem yön hem yakınlık büyüklüğünü taşır; yönü normalize edip
        // büyüklüğü max_speed'e ORANTILI ölçekleyerek yakınlık bilgisini koruyoruz.
        let raw = desired_velocity.length();
        if raw > f32::EPSILON {
            // raw büyüdükçe (engel yakınlaştıkça) istenen kaçış hızı max_speed'e kadar artar.
            let scaled_speed = raw.min(1.0) * max_speed;
            desired_velocity = (desired_velocity / raw) * scaled_speed;
        }
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
            // Ters-kare yasası: komşu yakınlaştıkça itme büyüklüğü artar.
            let force = diff / (dist * dist);
            desired_velocity += force;
            count += 1;
        }
    }

    if count > 0 {
        // Yönü koru, büyüklüğü yakınlığa ORANTILI olarak max_speed'e kadar ölçekle.
        let raw = desired_velocity.length();
        if raw > f32::EPSILON {
            let scaled_speed = raw.min(1.0) * max_speed;
            desired_velocity = (desired_velocity / raw) * scaled_speed;
        }
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
#[non_exhaustive]
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
        total_force += avoid_obstacles(current_pos, current_vel, obstacles, max_speed, max_force)
            * weights.avoid;
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

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_math::Vec3;

    /// Zero başlangıç hızıyla üretilen steering büyüklüğü, arzu edilen kaçış
    /// hızının büyüklüğüne eşittir; bu yüzden farklı mesafeler farklı büyüklük vermelidir.
    #[test]
    fn avoid_obstacles_scales_with_proximity() {
        let obs = Vec3::ZERO;
        let radius = 4.0;
        let max_speed = 10.0;
        let max_force = 1000.0; // clamp devreye girmesin

        // Yakın ajan (dist=1) ile uzak ajan (dist=3) aynı engelden kaçıyor.
        let near = avoid_obstacles(
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::ZERO,
            &[(obs, radius)],
            max_speed,
            max_force,
        );
        let far = avoid_obstacles(
            Vec3::new(3.0, 0.0, 0.0),
            Vec3::ZERO,
            &[(obs, radius)],
            max_speed,
            max_force,
        );

        // Yakın ajan kesinlikle daha güçlü itilmeli (eski kod eşit büyüklük veriyordu).
        assert!(
            near.length() > far.length() + 1.0,
            "yakın={} uzak={} yakınlık ölçeklemesi kaybolmuş",
            near.length(),
            far.length()
        );
    }

    #[test]
    fn separate_scales_with_proximity() {
        let separation_radius = 1.5;
        let max_speed = 10.0;
        let max_force = 1000.0;

        // Çok yakın komşu (dist=0.5) ile daha uzak komşu (dist=1.4).
        let near = separate(
            Vec3::ZERO,
            Vec3::ZERO,
            &[Vec3::new(0.5, 0.0, 0.0)],
            separation_radius,
            max_speed,
            max_force,
        );
        let far = separate(
            Vec3::ZERO,
            Vec3::ZERO,
            &[Vec3::new(1.4, 0.0, 0.0)],
            separation_radius,
            max_speed,
            max_force,
        );

        assert!(
            near.length() > far.length() + 1.0,
            "yakın={} uzak={} ayrılma yakınlık ölçeklemesi kaybolmuş",
            near.length(),
            far.length()
        );
    }
}
