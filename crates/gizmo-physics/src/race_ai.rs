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
    pub fn new(waypoints: Vec<Vec3>, speed_mult: f32) -> Self {
        Self {
            waypoints,
            current_wp: 0,
            reach_radius: 5.0,
            speed_mult,
            laps_completed: 0,
            total_wp_passed: 0,
        }
    }
}

/// AI araçlarını her fizik adımında günceleyen sistem.
/// Sonraki waypoint'e doğru `engine_force` ve `steering_angle` hesaplar.
pub fn race_ai_system(world: &gizmo_core::World, _dt: f32) {
    let transforms = match world.borrow::<crate::components::Transform>() {
        Some(t) => t,
        None => return,
    };
    let mut vehicles = match world.borrow_mut::<crate::vehicle::VehicleController>() {
        Some(v) => v,
        None => return,
    };
    let mut ais = match world.borrow_mut::<RaceAI>() {
        Some(a) => a,
        None => return,
    };

    let entity_ids = ais.entity_dense.clone();

    for &e in &entity_ids {
        let t = match transforms.get(e) { Some(t) => t, None => continue };
        let ai = match ais.get_mut(e) { Some(a) => a, None => continue };

        if ai.waypoints.is_empty() { continue; }

        // Hedef waypoint
        let target = ai.waypoints[ai.current_wp];
        let to_target = target - t.position;
        let dist_xz = Vec3::new(to_target.x, 0.0, to_target.z).length();

        // Waypoint'e ulaştı mı?
        if dist_xz < ai.reach_radius {
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

        // VehicleController'a yaz
        if let Some(vc) = vehicles.get_mut(e) {
            vc.engine_force = engine;
            vc.steering_angle = steer;
            vc.brake_force = brake;
        }
    }
}
