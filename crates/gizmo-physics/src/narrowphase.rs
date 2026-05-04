/// AAA Narrowphase — Box-Box SAT + 4-Noktalı Manifold + Gelişmiş Dispatcher
use crate::collision::ContactPoint;
use crate::components::ColliderShape;
use gizmo_math::{Quat, Vec3};
use crate::gjk::Gjk;

pub struct NarrowPhase;

impl NarrowPhase {
    // ─── Sphere-Sphere ────────────────────────────────────────────────────────
    pub fn sphere_sphere(pos_a: Vec3, r_a: f32, pos_b: Vec3, r_b: f32) -> Option<ContactPoint> {
        let d   = pos_b - pos_a;
        let d2  = d.length_squared();
        let sum = r_a + r_b;
        if d2 >= sum * sum || d2 < 1e-10 { return None; }
        let dist   = d2.sqrt();
        let normal = d / dist;
        Some(mk_contact(pos_a + normal * r_a, normal, sum - dist))
    }

    // ─── Sphere-Plane ─────────────────────────────────────────────────────────
    pub fn sphere_plane(sph_pos: Vec3, r: f32, n: Vec3, d: f32) -> Option<ContactPoint> {
        let dist = sph_pos.dot(n) - d;
        if dist >= r { return None; }
        Some(mk_contact(sph_pos - n * dist, n, r - dist))
    }

    // ─── Box-Plane — 4 köşe noktası üretir ───────────────────────────────────
    pub fn box_plane(
        bpos: Vec3, brot: Quat, half: Vec3, n: Vec3, d: f32,
    ) -> Vec<ContactPoint> {
        let corners = box_corners(bpos, brot, half);
        corners.iter().filter_map(|&c| {
            let dist = c.dot(n) - d;
            if dist < 0.0 { Some(mk_contact(c - n * dist, n, -dist)) } else { None }
        }).collect()
    }

    // ─── Box-Box SAT (Separating Axis Theorem) ────────────────────────────────
    /// 15 eksen (3+3+9) üzerinden minimum penetrasyon eksenini bul,
    /// ardından referans yüzünü kırparak 4 temas noktası üret.
    pub fn box_box(
        pos_a: Vec3, rot_a: Quat, ha: Vec3,
        pos_b: Vec3, rot_b: Quat, hb: Vec3,
    ) -> Vec<ContactPoint> {
        // Her kutunun 3 yerel ekseni
        let ax = [
            rot_a.mul_vec3(Vec3::X),
            rot_a.mul_vec3(Vec3::Y),
            rot_a.mul_vec3(Vec3::Z),
        ];
        let bx = [
            rot_b.mul_vec3(Vec3::X),
            rot_b.mul_vec3(Vec3::Y),
            rot_b.mul_vec3(Vec3::Z),
        ];

        let t   = pos_b - pos_a;
        let ha_ = [ha.x, ha.y, ha.z];
        let hb_ = [hb.x, hb.y, hb.z];

        // 15 aday eksen
        let mut axes: Vec<Vec3> = Vec::with_capacity(15);
        for &a in &ax { axes.push(a); }
        for &b in &bx { axes.push(b); }
        for &a in &ax {
            for &b in &bx {
                let c = a.cross(b);
                if c.length_squared() > 1e-6 { axes.push(c.normalize()); }
            }
        }

        let mut min_pen  = f32::MAX;
        let mut best_ax  = Vec3::Y;
        let mut best_flip = false;

        for axis in &axes {
            let pen = sat_penetration(axis, pos_a, &ax, &ha_, pos_b, &bx, &hb_, t);
            if pen < 0.0 { return vec![]; } // Ayrılma ekseni bulundu
            if pen < min_pen {
                min_pen  = pen;
                best_ax  = *axis;
                best_flip = t.dot(*axis) < 0.0;
            }
        }

        let normal = if best_flip { -best_ax } else { best_ax };

        // Referans yüzünü A'dan mı yoksa B'den mi al?
        // A yüzünün normali `normal`'e en yakın ise A referans yüz
        let (ref_pos, ref_rot, ref_h, inc_pos, inc_rot, inc_h) =
            if is_face_axis(normal, &ax) {
                (pos_a, rot_a, ha, pos_b, rot_b, hb)
            } else {
                (pos_b, rot_b, hb, pos_a, rot_a, ha)
            };

        clip_box_box(normal, min_pen, ref_pos, ref_rot, ref_h, inc_pos, inc_rot, inc_h)
    }

    // ─── Ana Dispatcher ───────────────────────────────────────────────────────
    pub fn test_collision(
        shape_a: &ColliderShape, pos_a: Vec3, rot_a: Quat,
        shape_b: &ColliderShape, pos_b: Vec3, rot_b: Quat,
    ) -> Option<ContactPoint> {
        // Compound dispatch (özyinelemeli)
        if let ColliderShape::Compound(c) = shape_a {
            return c.iter().filter_map(|(lt, ss)| {
                let wp = pos_a + rot_a.mul_vec3(lt.position);
                let wr = rot_a * lt.rotation;
                Self::test_collision(ss, wp, wr, shape_b, pos_b, rot_b)
            }).max_by(|a, b| a.penetration.total_cmp(&b.penetration));
        }
        if let ColliderShape::Compound(c) = shape_b {
            return c.iter().filter_map(|(lt, ss)| {
                let wp = pos_b + rot_b.mul_vec3(lt.position);
                let wr = rot_b * lt.rotation;
                Self::test_collision(shape_a, pos_a, rot_a, ss, wp, wr)
            }).max_by(|a, b| a.penetration.total_cmp(&b.penetration));
        }

        match (shape_a, shape_b) {
            (ColliderShape::Sphere(sa), ColliderShape::Sphere(sb)) =>
                Self::sphere_sphere(pos_a, sa.radius, pos_b, sb.radius),

            (ColliderShape::Sphere(s), ColliderShape::Plane(p)) =>
                Self::sphere_plane(pos_a, s.radius, p.normal, p.distance)
                    .map(|mut c| { c.normal = -c.normal; c }),

            (ColliderShape::Plane(p), ColliderShape::Sphere(s)) =>
                Self::sphere_plane(pos_b, s.radius, p.normal, p.distance),

            (ColliderShape::Box(b), ColliderShape::Plane(p)) =>
                Self::box_plane(pos_a, rot_a, b.half_extents, p.normal, p.distance)
                    .into_iter()
                    .max_by(|a, b| a.penetration.total_cmp(&b.penetration))
                    .map(|mut c| { c.normal = -c.normal; c }),

            (ColliderShape::Plane(p), ColliderShape::Box(b)) =>
                Self::box_plane(pos_b, rot_b, b.half_extents, p.normal, p.distance)
                    .into_iter()
                    .max_by(|a, b| a.penetration.total_cmp(&b.penetration)),

            (ColliderShape::Box(ba), ColliderShape::Box(bb)) =>
                Self::box_box(pos_a, rot_a, ba.half_extents, pos_b, rot_b, bb.half_extents)
                    .into_iter()
                    .max_by(|a, b| a.penetration.total_cmp(&b.penetration)),

            _ => Gjk::get_contact(shape_a, pos_a, rot_a, shape_b, pos_b, rot_b),
        }
    }

    /// Tüm temas noktalarını üret (manifold için) — Box-Box/Box-Plane'de 4 nokta
    pub fn test_collision_manifold(
        shape_a: &ColliderShape, pos_a: Vec3, rot_a: Quat,
        shape_b: &ColliderShape, pos_b: Vec3, rot_b: Quat,
    ) -> Vec<ContactPoint> {
        if let ColliderShape::Compound(c) = shape_a {
            return c.iter().flat_map(|(lt, ss)| {
                let wp = pos_a + rot_a.mul_vec3(lt.position);
                let wr = rot_a * lt.rotation;
                Self::test_collision_manifold(ss, wp, wr, shape_b, pos_b, rot_b)
            }).collect();
        }
        if let ColliderShape::Compound(c) = shape_b {
            return c.iter().flat_map(|(lt, ss)| {
                let wp = pos_b + rot_b.mul_vec3(lt.position);
                let wr = rot_b * lt.rotation;
                Self::test_collision_manifold(shape_a, pos_a, rot_a, ss, wp, wr)
            }).collect();
        }

        match (shape_a, shape_b) {
            (ColliderShape::Sphere(sa), ColliderShape::Sphere(sb)) =>
                Self::sphere_sphere(pos_a, sa.radius, pos_b, sb.radius).into_iter().collect(),

            (ColliderShape::Box(b), ColliderShape::Plane(p)) => {
                let mut contacts = Self::box_plane(pos_a, rot_a, b.half_extents, p.normal, p.distance);
                for c in &mut contacts { c.normal = -c.normal; }
                contacts
            }

            (ColliderShape::Plane(p), ColliderShape::Box(b)) =>
                Self::box_plane(pos_b, rot_b, b.half_extents, p.normal, p.distance),

            (ColliderShape::Box(ba), ColliderShape::Box(bb)) =>
                Self::box_box(pos_a, rot_a, ba.half_extents, pos_b, rot_b, bb.half_extents),

            _ => Self::test_collision(shape_a, pos_a, rot_a, shape_b, pos_b, rot_b)
                    .into_iter().collect(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SAT Yardımcıları
// ─────────────────────────────────────────────────────────────────────────────

/// Bir eksen boyunca iki kutu arasındaki penetrasyon miktarı.
/// Negatif → ayrılma ekseni bulundu.
fn sat_penetration(
    axis: &Vec3,
    _pos_a: Vec3, ax: &[Vec3; 3], ha: &[f32; 3],
    _pos_b: Vec3, bx: &[Vec3; 3], hb: &[f32; 3],
    t: Vec3,
) -> f32 {
    let proj_a: f32 = ax.iter().zip(ha).map(|(e, &h)| (e.dot(*axis)).abs() * h).sum();
    let proj_b: f32 = bx.iter().zip(hb).map(|(e, &h)| (e.dot(*axis)).abs() * h).sum();
    let dist       = t.dot(*axis).abs();
    proj_a + proj_b - dist
}

/// Verilen normal'in kutu A'nın yüz eksenlerinden biriyle hizalanıp hizalanmadığı
fn is_face_axis(normal: Vec3, ax: &[Vec3; 3]) -> bool {
    ax.iter().any(|a| a.dot(normal).abs() > 0.9)
}

// ─────────────────────────────────────────────────────────────────────────────
// Sutherland-Hodgman Clipping — 4 Noktalı Manifold
// ─────────────────────────────────────────────────────────────────────────────

fn box_corners(pos: Vec3, rot: Quat, h: Vec3) -> [Vec3; 8] {
    let signs = [
        Vec3::new( 1., 1., 1.), Vec3::new(-1., 1., 1.),
        Vec3::new( 1.,-1., 1.), Vec3::new(-1.,-1., 1.),
        Vec3::new( 1., 1.,-1.), Vec3::new(-1., 1.,-1.),
        Vec3::new( 1.,-1.,-1.), Vec3::new(-1.,-1.,-1.),
    ];
    signs.map(|s| pos + rot.mul_vec3(Vec3::new(s.x * h.x, s.y * h.y, s.z * h.z)))
}

/// Sutherland-Hodgman 3D clip: `poly` noktalarını `plane_n · x = plane_d` düzlemine göre kırp
#[allow(dead_code)]
fn clip_poly_by_plane(poly: &[Vec3], plane_n: Vec3, plane_d: f32) -> Vec<Vec3> {
    if poly.is_empty() { return vec![]; }
    let mut out = Vec::with_capacity(poly.len() + 1);
    let n = poly.len();
    for i in 0..n {
        let cur  = poly[i];
        let next = poly[(i + 1) % n];
        let dc = cur.dot(plane_n)  - plane_d;
        let dn = next.dot(plane_n) - plane_d;
        if dc >= 0.0 { out.push(cur); }
        if (dc >= 0.0) != (dn >= 0.0) {
            let t = dc / (dc - dn);
            out.push(cur + (next - cur) * t);
        }
    }
    out
}

/// Referans kutu yüzüne karşı incident kutunun tüm 8 köşesini test eder.
/// Referans yüzünün gerisindeki ve yüz sınırları içindeki köşeleri temas noktası olarak üretir.
fn clip_box_box(
    normal: Vec3, pen: f32,
    ref_pos: Vec3, ref_rot: Quat, ref_h: Vec3,
    inc_pos: Vec3, inc_rot: Quat, inc_h: Vec3,
) -> Vec<ContactPoint> {
    // Referans kutusunun yerel eksenleri
    let ref_axes = [
        ref_rot.mul_vec3(Vec3::X),
        ref_rot.mul_vec3(Vec3::Y),
        ref_rot.mul_vec3(Vec3::Z),
    ];
    let ref_h_arr = [ref_h.x, ref_h.y, ref_h.z];

    // Normal'e en yakın referans yüz eksenini bul
    let (face_idx, _) = ref_axes.iter().enumerate()
        .map(|(i, a)| (i, a.dot(normal).abs()))
        .fold((0, 0.0f32), |(bi, bv), (i, v)| if v > bv { (i, v) } else { (bi, bv) });

    let face_n     = ref_axes[face_idx];
    let face_n_sgn = if face_n.dot(normal) > 0.0 { 1.0_f32 } else { -1.0 };
    let face_n_dir = face_n * face_n_sgn;

    // Referans yüzünün düzlem sabiti
    let ref_face_d = (ref_pos + face_n_dir * ref_h_arr[face_idx]).dot(face_n_dir);

    // Incident kutunun tüm 8 köşesi
    let corners = box_corners(inc_pos, inc_rot, inc_h);

    let mut contacts: Vec<ContactPoint> = corners.iter().filter_map(|&p| {
        // 1. Köşe referans yüzünün gerisinde mi? (penetrasyon var mı?)
        let signed_dist = p.dot(face_n_dir) - ref_face_d;
        if signed_dist > 0.0 { return None; }  // yüzün önünde

        // 2. Diğer 2 eksende referans kutunun sınırları içinde mi?
        let local = p - ref_pos;
        let t0 = ref_axes[(face_idx + 1) % 3];
        let t1 = ref_axes[(face_idx + 2) % 3];
        let e0 = ref_h_arr[(face_idx + 1) % 3];
        let e1 = ref_h_arr[(face_idx + 2) % 3];
        if local.dot(t0).abs() > e0 + 1e-3 { return None; }
        if local.dot(t1).abs() > e1 + 1e-3 { return None; }

        let depth = -signed_dist; // pozitif penetrasyon
        let depth = depth.max(pen * 0.01);
        let contact_pt = p - face_n_dir * (depth * 0.5);
        Some(mk_contact(contact_pt, normal, depth))
    }).collect();

    // En derin 4 noktayı seç (solver için optimal)
    contacts.sort_unstable_by(|a, b| b.penetration.total_cmp(&a.penetration));
    contacts.truncate(4);
    contacts
}

// ─────────────────────────────────────────────────────────────────────────────
// Ortak yardımcı
// ─────────────────────────────────────────────────────────────────────────────

fn mk_contact(point: Vec3, normal: Vec3, penetration: f32) -> ContactPoint {
    ContactPoint {
        point,
        normal,
        penetration,
        local_point_a: Vec3::ZERO,
        local_point_b: Vec3::ZERO,
        normal_impulse: 0.0,
        tangent_impulse: Vec3::ZERO,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Testler
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_core::entity::Entity;

    #[test]
    fn test_sphere_sphere_hit() {
        let c = NarrowPhase::sphere_sphere(Vec3::ZERO, 1.0, Vec3::new(1.5, 0., 0.), 1.0);
        assert!(c.is_some());
        let c = c.unwrap();
        assert!(c.penetration > 0.0);
        assert!((c.normal.x - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_sphere_sphere_miss() {
        assert!(NarrowPhase::sphere_sphere(Vec3::ZERO, 1.0, Vec3::new(3., 0., 0.), 1.0).is_none());
    }

    #[test]
    fn test_box_plane_4_contacts() {
        // Kutu zeminde 4 köşesiyle de temas etmeli
        let contacts = NarrowPhase::box_plane(
            Vec3::new(0., 0.5, 0.), Quat::IDENTITY, Vec3::splat(1.0),
            Vec3::Y, 0.0,
        );
        // 4 alt köşe penetrasyonda
        assert_eq!(contacts.len(), 4, "Expected 4 contact points, got {}", contacts.len());
        for c in &contacts { assert!(c.penetration > 0.0); }
    }

    #[test]
    fn test_box_box_sat_overlap() {
        let contacts = NarrowPhase::box_box(
            Vec3::ZERO,          Quat::IDENTITY, Vec3::splat(1.0),
            Vec3::new(1.5,0.,0.), Quat::IDENTITY, Vec3::splat(1.0),
        );
        assert!(!contacts.is_empty(), "Box-box overlap should produce contacts");
    }

    #[test]
    fn test_box_box_sat_no_overlap() {
        let contacts = NarrowPhase::box_box(
            Vec3::ZERO,           Quat::IDENTITY, Vec3::splat(1.0),
            Vec3::new(5., 0., 0.), Quat::IDENTITY, Vec3::splat(1.0),
        );
        assert!(contacts.is_empty(), "Separated boxes should have no contacts");
    }

    #[test]
    fn test_box_box_sat_rotated() {
        let rot45 = Quat::from_rotation_y(std::f32::consts::FRAC_PI_4);
        let contacts = NarrowPhase::box_box(
            Vec3::ZERO,             Quat::IDENTITY, Vec3::splat(0.8),
            Vec3::new(1.0, 0., 0.), rot45,          Vec3::splat(0.8),
        );
        // 45° döndürülmüş ve yakın iki kutu temas etmeli
        assert!(!contacts.is_empty());
    }

    #[test]
    fn test_dispatcher_box_box() {
        use crate::components::BoxShape;
        let ba = ColliderShape::Box(BoxShape { half_extents: Vec3::splat(1.0) });
        let bb = ColliderShape::Box(BoxShape { half_extents: Vec3::splat(1.0) });
        let c = NarrowPhase::test_collision(
            &ba, Vec3::ZERO, Quat::IDENTITY,
            &bb, Vec3::new(1.5, 0., 0.), Quat::IDENTITY,
        );
        assert!(c.is_some());
    }
}
