use gizmo_math::Vec3;

/// Sürekli Çarpışma Tespiti (CCD) — bisection yöntemi ile TOI (Time of Impact) bulur.
/// Alt aralık `[t_low, t_mid]` için: A dünya konumunda `pos_a + v_a*t_low` (sabit), B aynı anda
/// `pos_b + v_b*t_low`; bu alt süre boyunca **göreli** yer değiştirme `(v_b - v_a) * (t_mid - t_low)`.
#[inline]
fn integrate_rot(rot: gizmo_math::Quat, ang_vel: Vec3, t: f32) -> gizmo_math::Quat {
    if ang_vel.length_squared() > 1e-6 {
        gizmo_math::Quat::from_axis_angle(ang_vel.normalize(), ang_vel.length() * t) * rot
    } else {
        rot
    }
}

pub struct CcdInput<'a> {
    pub shape: &'a crate::shape::ColliderShape,
    pub pos: Vec3,
    pub rot: gizmo_math::Quat,
    pub vel_lin: Vec3,
    pub vel_ang: Vec3,
}

pub struct CcdResult {
    pub manifold: crate::collision::CollisionManifold,
    pub remaining_time: f32,
    pub ccd_offset_a: Option<Vec3>,
    pub ccd_offset_b: Option<Vec3>,
}

pub fn ccd_bisect(a: CcdInput, b: CcdInput, dt: f32) -> CcdResult {
    let rel_v = b.vel_lin - a.vel_lin;

    let mut swept_b = crate::shape::ColliderShape::Swept {
        base: Box::new(b.shape.clone()),
        sweep_vector: rel_v * dt,
    };

    let (hit_any, _) = crate::gjk::gjk_intersect(a.shape, a.pos, a.rot, &swept_b, b.pos, b.rot);
    if !hit_any {
        return CcdResult {
            manifold: crate::collision::CollisionManifold {
                is_colliding: false,
                normal: Vec3::ZERO,
                penetration: 0.0,
                contact_points: arrayvec::ArrayVec::new(),
            },
            remaining_time: 0.0,
            ccd_offset_a: None,
            ccd_offset_b: None,
        };
    }

    let mut t_low  = 0.0_f32;
    let mut t_high = dt;

    for _ in 0..16 {
        let t_mid = (t_low + t_high) * 0.5;
        if (t_high - t_low) < 1e-4 { break; }

        let a_pos_mid = a.pos + a.vel_lin * t_low;
        let b_pos_mid = b.pos + b.vel_lin * t_low;
        let a_rot_mid = integrate_rot(a.rot, a.vel_ang, t_low);
        let b_rot_mid = integrate_rot(b.rot, b.vel_ang, t_low);

        let rel_disp = rel_v * (t_mid - t_low);
        if let crate::shape::ColliderShape::Swept { ref mut sweep_vector, .. } = swept_b {
            *sweep_vector = rel_disp;
        }

        let (hit_first, _) = crate::gjk::gjk_intersect(
            a.shape, a_pos_mid, a_rot_mid,
            &swept_b, b_pos_mid, b_rot_mid,
        );
        if hit_first { t_high = t_mid; } else { t_low = t_mid; }
    }

    let t_hit = t_low;
    
    let a_pos_hit = a.pos + a.vel_lin * t_hit;
    let b_pos_hit = b.pos + b.vel_lin * t_hit;
    let a_rot_hit = integrate_rot(a.rot, a.vel_ang, t_hit);
    let b_rot_hit = integrate_rot(b.rot, b.vel_ang, t_hit);

    let (hit, sim) = crate::gjk::gjk_intersect(a.shape, a_pos_hit, a_rot_hit, b.shape, b_pos_hit, b_rot_hit);
    if !hit {
        let normal = if rel_v.length_squared() > 1e-6 { -rel_v.normalize() } else { Vec3::new(0.0, 1.0, 0.0) };
        let sup_a = a.shape.support_point(a_pos_hit, a_rot_hit, -normal);
        let sup_b = b.shape.support_point(b_pos_hit, b_rot_hit, normal);
        
        return CcdResult {
            manifold: crate::collision::CollisionManifold {
                is_colliding: true,
                normal,
                penetration: 0.01,
                contact_points: { let mut v = arrayvec::ArrayVec::new(); v.push(((sup_a + sup_b) * 0.5, 0.01)); v },
            },
            remaining_time: dt - t_hit,
            ccd_offset_a: Some(a_pos_hit - a.pos),
            ccd_offset_b: Some(b_pos_hit - b.pos),
        };
    }

    let mut manifold = crate::epa::epa_solve(sim, a.shape, a_pos_hit, a_rot_hit, b.shape, b_pos_hit, b_rot_hit);
    let mut remaining_t = 0.0;
    let mut off_a = None;
    let mut off_b = None;

    if manifold.is_colliding {
        remaining_t = dt - t_hit;
        
        // ÖNEMLİ (Space Transformation): 
        // EPA'dan dönen `contact_points`, vurulma anındaki (t_hit) Dünya Uzayındadır (World Space).
        // Ancak narrow_phase algoritmaları ve warm-starting cache, temasları karenin BAŞINDAKİ (t=0)
        // Dünya Uzayı referans çerçevesine (World Space) göre saklar ve eşler.
        // Bu yüzden noktalara `a.vel_lin * t_hit` kadar ters bir offset uygulayarak noktaları 
        // A objesinin t=0 anındaki referans çerçevesine (Dünya Uzayı üzerine) geri çekiyoruz.
        let cp_offset = a.vel_lin * t_hit;
        for cp in &mut manifold.contact_points {
            cp.0 -= cp_offset;
        }
        off_a = Some(a_pos_hit - a.pos);
        off_b = Some(b_pos_hit - b.pos);
    }

    CcdResult {
        manifold,
        remaining_time: remaining_t,
        ccd_offset_a: off_a,
        ccd_offset_b: off_b,
    }
}
