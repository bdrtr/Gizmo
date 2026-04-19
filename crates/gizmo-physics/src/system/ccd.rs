use gizmo_math::Vec3;

/// Sürekli Çarpışma Tespiti (CCD) — bisection yöntemi ile TOI (Time of Impact) bulur.
///
/// Mermi hızındaki nesnelerin bir frame'de tünel geçmesini önler.
/// `ccd_offset_a` / `ccd_offset_b` çıkışları, TOI anından itibaren pozisyon offsetidir.
///
/// Alt aralık `[t_low, t_mid]` için: A dünya konumunda `pos_a + v_a*t_low` (sabit), B aynı anda
/// `pos_b + v_b*t_low`; bu alt süre boyunca **göreli** yer değiştirme `(v_b - v_a) * (t_mid - t_low)`.
/// Yani süpürme vektörü her zaman **o aralığın başındaki** konumlara göre; `t=0` ile karıştırılmaz.
pub fn ccd_bisect(
    shape_a: &crate::shape::ColliderShape, pos_a: Vec3, rot_a: gizmo_math::Quat,
    shape_b: &crate::shape::ColliderShape, pos_b: Vec3, rot_b: gizmo_math::Quat,
    v_a_lin: Vec3, v_b_lin: Vec3, dt: f32,
    ccd_offset_a: &mut Option<Vec3>,
    ccd_offset_b: &mut Option<Vec3>,
) -> crate::collision::CollisionManifold {
    // Göreli lineer hız (A'nın t_low anındaki dünya çerçevesinde B'nin görünen hızı).
    let rel_v = v_b_lin - v_a_lin;

    // Ön test: [0, dt] — konumlar kare başı (t=0), süpürme rel_v * dt.
    let swept_b_full = crate::shape::ColliderShape::Swept {
        base: Box::new(shape_b.clone()),
        sweep_vector: rel_v * dt,
    };
    let (hit_any, _) = crate::gjk::gjk_intersect(shape_a, pos_a, rot_a, &swept_b_full, pos_b, rot_b);
    if !hit_any {
        return crate::collision::CollisionManifold {
            is_colliding: false,
            normal: Vec3::ZERO,
            penetration: 0.0,
            contact_points: vec![],
        };
    }

    let mut t_low  = 0.0_f32;
    let mut t_high = dt;

    for _ in 0..16 {
        let t_mid = (t_low + t_high) * 0.5;
        // Aralık başı t_low: dünya uzayında o anki merkezler.
        let pos_a_at_t_low = pos_a + v_a_lin * t_low;
        let pos_b_at_t_low = pos_b + v_b_lin * t_low;
        // Bisection HER ZAMAN aralığın sadece İLK YARISINI test etmeli. ([t_low, t_mid])
        let rel_disp = rel_v * (t_mid - t_low);
        let sweep_mid = crate::shape::ColliderShape::Swept {
            base: Box::new(shape_b.clone()),
            sweep_vector: rel_disp,
        };
        let (hit_first, _) = crate::gjk::gjk_intersect(
            shape_a,
            pos_a_at_t_low,
            rot_a,
            &sweep_mid,
            pos_b_at_t_low,
            rot_b,
        );
        if hit_first { t_high = t_mid; } else { t_low = t_mid; }
    }

    // TOI: Kesin çarpışma anı t_high civarındadır.
    // Çok mermi hızında t_high anında objeler henüz dokunuyor olabilir. İç içe geçmeyi t_high ile sağlarız.
    // Sabit hızla çok küçük bir saniye ekleyerek minimal penetrasyon (EPA için) garanti edelim.
    // Ancak çok hızlı objelerin içinden geçmesini önlemek için mesafe bazlı limitleme yapalım (Maks 1 cm).
    let speed = rel_v.length();
    let safe_offset = if speed > 1.0 { (0.01 / speed).min(dt * 0.05) } else { dt * 0.001 };
    let t_hit  = (t_high + safe_offset).min(dt);
    
    let pa_hit = pos_a + v_a_lin * t_hit;
    let pb_hit = pos_b + v_b_lin * t_hit;

    let (hit, sim) = crate::gjk::gjk_intersect(shape_a, pa_hit, rot_a, shape_b, pb_hit, rot_b);
    if !hit {
        // Eğer çok hızlılarsa ve offset onları tamamen dışarı attıysa, mecburi çarpışmayı üret.
        let normal = if speed > 0.001 { -rel_v.normalize() } else { Vec3::new(0.0, 1.0, 0.0) };
        return crate::collision::CollisionManifold {
            is_colliding: true, // CCD bisection ile çarpıştığını biliyoruz
            normal,
            penetration: 0.01,
            contact_points: vec![((pa_hit + pb_hit) * 0.5, 0.01)],
        };
    }

    let mut manifold = crate::epa::epa_solve(sim, shape_a, pa_hit, rot_a, shape_b, pb_hit, rot_b);
    if manifold.is_colliding {
        // Kalan süre boyunca penetrasyonu yapay artır (tünellemeyi önle)
        let remaining_t = dt - t_hit;
        let vn = rel_v.dot(manifold.normal);
        if vn < 0.0 {
            manifold.penetration += -vn * remaining_t;
        }
        // Temas noktalarını TOI anına geri taşı (A'nın referans çerçevesine)
        let cp_offset = v_a_lin * t_hit;
        for cp in &mut manifold.contact_points {
            cp.0 -= cp_offset;
        }
        *ccd_offset_a = Some(pa_hit - pos_a);
        *ccd_offset_b = Some(pb_hit - pos_b);
    }
    manifold
}
