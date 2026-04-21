use gizmo_math::Vec3;

/// PS1 tarzı basit yarış AI'ı — Waypoint takip eden araç kontrolcüsü.
/// Her frame sonraki waypoint'e doğru steering ve throttle hesaplar.
#[derive(Debug, Clone)]
pub struct RaceAI {
    /// Pistteki waypoint'ler (sıralı, kapalı devre)
    pub waypoints: Vec<Vec3>,
    /// Şu anki hedef waypoint indeksi
    pub current_wp: usize,
    /// Waypoint'e ulaşma yarıçapı (metre)
    pub reach_radius: f32,
    /// Hız çarpanı (zorluk seviyesi: 0.5 = yavaş, 1.0 = normal, 1.5 = hızlı)
    pub speed_mult: f32,
    /// Tamamlanan tur sayısı
    pub laps_completed: u32,
    /// Toplam waypoint geçiş sayısı (sıralama için)
    pub total_wp_passed: u32,
}

impl RaceAI {
    pub fn new(waypoints: Vec<Vec3>, speed_mult: f32, reach_radius: f32) -> Self {
        Self {
            waypoints,
            current_wp: 0,
            reach_radius,
            speed_mult,
            laps_completed: 0,
            total_wp_passed: 0,
        }
    }
}

/// AI araçlarını her fizik adımında günceleyen sistem.
/// Sonraki waypoint'e doğru `engine_force` ve `steering_angle` hesaplar.
pub fn race_ai_system(world: &gizmo_core::World, _dt: f32) {
    let transforms = world.borrow::<crate::components::Transform>();
    let mut vehicles = world.borrow_mut::<crate::vehicle::VehicleController>();
    let mut ais = world.borrow_mut::<RaceAI>();

    let entity_ids: Vec<u32> = ais.iter().map(|(id, _)| id).collect();

    for &e in &entity_ids {
        let t = match transforms.get(e) {
            Some(t) => t,
            None => continue,
        };
        let ai = match ais.get_mut(e) {
            Some(a) => a,
            None => continue,
        };

        if ai.waypoints.is_empty() {
            continue;
        }

        // Hedef waypoint
        let target = ai.waypoints[ai.current_wp];
        let to_target = target - t.position;
        let dist = to_target.length();

        // Waypoint'e ulaştı mı?
        if dist < ai.reach_radius {
            ai.current_wp += 1;
            ai.total_wp_passed += 1;

            // Tur tamamladı mı?
            if ai.current_wp >= ai.waypoints.len() {
                ai.current_wp = 0;
                ai.laps_completed += 1;
            }
            continue;
        }

        // Aracın ileri yönü (lokal Z)
        let forward = t.rotation.mul_vec3(Vec3::new(0.0, 0.0, 1.0));
        let forward_xz = Vec3::new(forward.x, 0.0, forward.z).normalize();
        let target_dir = Vec3::new(to_target.x, 0.0, to_target.z).normalize();

        // Steering: Cross product ile sağa/sola dönüş yönü
        let cross = forward_xz.cross(target_dir);
        let dot = forward_xz.dot(target_dir);

        // Steering açısı: cross.y pozitifse sola, negatifse sağa
        let steer = cross.y.clamp(-1.0, 1.0) * 0.8; // Max 0.8 rad (~45 deg)

        // Throttle: Hedefe bakıyorsak tam gaz, yana bakıyorsak yavaşla
        let base_engine = 12000.0 * ai.speed_mult;
        let engine = if dot > 0.5 {
            base_engine
        } else if dot > 0.0 {
            base_engine * 0.5 // Virajda yavaşla
        } else {
            base_engine * 0.3 // Ters yöne bakıyorsak çok yavaşla
        };

        // Brake: Hedefin tam tersine bakıyorsak frenle
        let brake = if dot < -0.3 { 15000.0 } else { 0.0 };

        // VehicleController'a yaz (Yumuşak geçiş / Interpolation ile frame-rate bağımsızlığı)
        if let Some(vc) = vehicles.get_mut(e) {
            // Saniyede %99 hedefe ulaşacak şekilde exponential decay (pürüzsüz lerp)
            let lerp_factor = 1.0 - (-10.0 * _dt).exp();

            vc.engine_force += (engine - vc.engine_force) * lerp_factor;
            vc.steering_angle += (steer - vc.steering_angle) * lerp_factor;
            vc.brake_force += (brake - vc.brake_force) * lerp_factor;
        }
    }
}

gizmo_core::impl_component!(RaceAI);
