use crate::components::{RigidBody, Transform, Velocity};
use crate::integration::apply_inv_inertia;
use crate::shape::Collider;
use crate::vehicle::VehicleController;
use gizmo_core::World;
use gizmo_math::Vec3;
use std::collections::HashMap;

// ─── Veri Yapıları ────────────────────────────────────────────────────────────

/// Kalıcı temas noktası — frame'ler arası eşleme için konum bilgisi taşır
#[derive(Clone, Debug)]
pub struct CachedContact {
    /// Dünya koordinatlarında temas noktası (eşleme anahtarı)
    pub world_point: Vec3,
    /// Birikmiş normal impuls
    pub accumulated_normal: f32,
    /// Birikmiş sürtünme impulsu
    pub accumulated_friction: Vec3,
}

/// Broad-phase için entity'nin dünya-uzayı AABB sınırları
struct Interval {
    entity: u32,
    min: Vec3,
    max: Vec3,
}

/// Narrow-phase çözücüsünün bir adım için gereken tüm veriler (17 alan)
struct StoredContact {
    ent_a: u32,
    ent_b: u32,
    normal: Vec3,
    inv_mass_a: f32,
    inv_mass_b: f32,
    inv_inertia_a: Vec3,
    inv_inertia_b: Vec3,
    restitution: f32,
    friction: f32,
    penetration: f32,
    r_a: Vec3,
    r_b: Vec3,
    rot_a: gizmo_math::Quat,
    rot_b: gizmo_math::Quat,
    accumulated_j: f32,
    accumulated_friction: Vec3,
    ccd_offset_a: Vec3,
    ccd_offset_b: Vec3,
    /// Dünya koordinatlarındaki temas noktası (warm-start eşleme için)
    world_point: Vec3,
}

/// Paralel algılama adımının tek-çiftten dönen sonucu
struct DetectionResult {
    contacts: Vec<StoredContact>,
    wake_entities: Vec<u32>,
}

/// Birbirine temas eden dinamik entity'lerin grubu
struct Island {
    contacts: Vec<StoredContact>,
    velocities: HashMap<u32, Velocity>,
    poses: HashMap<u32, Transform>,
}

// ─── Çözücü Durumu ───────────────────────────────────────────────────────────

/// Contact Point Matching eşik değeri (2cm yarıçap)
const MATCH_THRESHOLD_SQ: f32 = 0.02 * 0.02;

/// Warm-start sönümleme faktörü (%80 — patlama riskini azaltır)
const WARM_START_FACTOR: f32 = 0.8;

/// Kalıcı Çözücü Durumu (Warm-Starting Cache için)
pub struct PhysicsSolverState {
    /// Önceki karedeki temas noktalarının entity-çifti bazlı cache'i
    pub contact_cache: HashMap<(u32, u32), Vec<CachedContact>>,
    /// Konfigüre edilebilir çözücü iterasyon sayısı (varsayılan: 8)
    pub solver_iterations: u32,
    /// Frame sayacı — contact shuffle için seed olarak kullanılır
    pub frame_counter: u64,
}

impl Default for PhysicsSolverState {
    fn default() -> Self {
        Self::new()
    }
}

impl PhysicsSolverState {
    pub fn new() -> Self {
        Self {
            contact_cache: HashMap::new(),
            solver_iterations: 8,
            frame_counter: 0,
        }
    }
}

// ─── Yardımcı Fonksiyonlar ───────────────────────────────────────────────────

/// Yeni frame'deki bir temas noktası için önceki frame'in cache'inde en yakın eşleşmeyi bul.
/// Eşleşme 2cm içindeyse (accumulated_j, accumulated_friction) döndürür, yoksa None.
fn match_cached_contact(new_point: Vec3, cached: &[CachedContact]) -> Option<(f32, Vec3)> {
    let mut best_dist_sq = f32::MAX;
    let mut best = None;
    for cc in cached {
        let d = (new_point - cc.world_point).length_squared();
        if d < best_dist_sq && d < MATCH_THRESHOLD_SQ {
            best_dist_sq = d;
            best = Some((cc.accumulated_normal, cc.accumulated_friction));
        }
    }
    best
}

/// Node'u haritaya ekler (idempotent — zaten varsa değişmez).
fn ensure_node(parent: &mut HashMap<u32, u32>, rank: &mut HashMap<u32, u8>, i: u32) {
    parent.entry(i).or_insert(i);
    rank.entry(i).or_insert(0);
}

/// Kökü döndürür — iki geçişli tam path compression uygular.
///
/// 1. Geçiş: root'a kadar olan zinciri `path` vektörüne topla.
/// 2. Geçiş: zincirdeki her node'u doğrudan root'a bağla.
///
/// Bir sonraki find_root çağrısında bu node'lar O(1) ile root'a ulaşır.
fn find_root(parent: &mut HashMap<u32, u32>, i: u32) -> u32 {
    let mut path = Vec::new();
    let mut cur = i;
    loop {
        let p = match parent.get(&cur) {
            Some(&p) => p,
            None => cur,
        };
        if p == cur {
            break;
        }
        path.push(cur);
        cur = p;
    }
    let root = cur;
    for node in path {
        parent.insert(node, root);
    }
    root
}

/// İki island'ı birleştirir; rank'ı düşük olan, yüksek olanın altına girer.
fn union_nodes(
    parent: &mut HashMap<u32, u32>,
    rank: &mut HashMap<u32, u8>,
    i: u32,
    j: u32,
) {
    let ri = find_root(parent, i);
    let rj = find_root(parent, j);
    if ri == rj {
        return;
    }
    let rank_i = *rank.get(&ri).unwrap_or(&0);
    let rank_j = *rank.get(&rj).unwrap_or(&0);
    match rank_i.cmp(&rank_j) {
        std::cmp::Ordering::Less => {
            parent.insert(ri, rj);
        }
        std::cmp::Ordering::Greater => {
            parent.insert(rj, ri);
        }
        std::cmp::Ordering::Equal => {
            parent.insert(rj, ri);
            *rank.entry(ri).or_insert(0) += 1;
        }
    }
}

// ─── Fizik Fazları ───────────────────────────────────────────────────────────

/// FAZ 1 — Broad-Phase: Sweep & Prune (active-list, dinamik eksen seçimi)
///
/// Her entity'nin dünya-uzayı AABB'sini hesaplar (CCD sweep dahil),
/// varyans bazlı eksen seçimiyle sıralayıp aktif-liste geçişiyle
/// olası çarpışma çiftlerini O(N log N + N·k) karmaşıklıkta döndürür.
fn broad_phase(
    transforms: &gizmo_core::SparseSet<Transform>,
    colliders: &gizmo_core::SparseSet<Collider>,
    rigidbodies: &gizmo_core::SparseSet<RigidBody>,
    velocities: &gizmo_core::SparseSet<Velocity>,
    dt: f32,
) -> Vec<(u32, u32)> {
    use crate::shape::ColliderShape;

    let entities: Vec<u32> = transforms.dense.iter().map(|e| e.entity).collect();
    let mut intervals = Vec::with_capacity(entities.len());

    for &e in &entities {
        let t = match transforms.get(e) {
            Some(t) => t,
            None => continue,
        };
        let col = match colliders.get(e) {
            Some(c) => c,
            None => continue,
        };

        let (mut min, mut max) = match &col.shape {
            ColliderShape::Aabb(a) => {
                let he = a.half_extents;
                let corners = [
                    Vec3::new( he.x,  he.y,  he.z),
                    Vec3::new( he.x,  he.y, -he.z),
                    Vec3::new( he.x, -he.y,  he.z),
                    Vec3::new( he.x, -he.y, -he.z),
                    Vec3::new(-he.x,  he.y,  he.z),
                    Vec3::new(-he.x,  he.y, -he.z),
                    Vec3::new(-he.x, -he.y,  he.z),
                    Vec3::new(-he.x, -he.y, -he.z),
                ];
                let mut mn = Vec3::new(f32::MAX, f32::MAX, f32::MAX);
                let mut mx = Vec3::new(f32::MIN, f32::MIN, f32::MIN);
                for v in &corners {
                    let wv = t.position + t.rotation.mul_vec3(*v);
                    mn.x = mn.x.min(wv.x); mn.y = mn.y.min(wv.y); mn.z = mn.z.min(wv.z);
                    mx.x = mx.x.max(wv.x); mx.y = mx.y.max(wv.y); mx.z = mx.z.max(wv.z);
                }
                (mn, mx)
            }
            ColliderShape::Sphere(s) => {
                let r = Vec3::new(s.radius, s.radius, s.radius);
                (t.position - r, t.position + r)
            }
            ColliderShape::Capsule(c) => {
                let up = t.rotation.mul_vec3(Vec3::new(0.0, c.half_height, 0.0));
                let top = t.position + up;
                let bot = t.position - up;
                let r = Vec3::new(c.radius, c.radius, c.radius);
                let mn = Vec3::new(top.x.min(bot.x), top.y.min(bot.y), top.z.min(bot.z)) - r;
                let mx = Vec3::new(top.x.max(bot.x), top.y.max(bot.y), top.z.max(bot.z)) + r;
                (mn, mx)
            }
            ColliderShape::ConvexHull(hull) => {
                let mut mn = Vec3::new(f32::MAX, f32::MAX, f32::MAX);
                let mut mx = Vec3::new(f32::MIN, f32::MIN, f32::MIN);
                for v in &hull.vertices {
                    let wv = t.position + t.rotation.mul_vec3(*v);
                    mn.x = mn.x.min(wv.x); mn.y = mn.y.min(wv.y); mn.z = mn.z.min(wv.z);
                    mx.x = mx.x.max(wv.x); mx.y = mx.y.max(wv.y); mx.z = mx.z.max(wv.z);
                }
                (mn, mx)
            }
            ColliderShape::Swept { .. } => {
                eprintln!("[Physics WARN] Swept shape found in ECS for entity {}! Skipping.", e);
                continue;
            }
            ColliderShape::HeightField { width, max_height, depth, .. } => {
                let he = Vec3::new(width * 0.5, max_height * 0.5, depth * 0.5);
                let off = Vec3::new(0.0, max_height * 0.5, 0.0);
                let corners = [
                    Vec3::new( he.x,  he.y,  he.z) + off,
                    Vec3::new( he.x,  he.y, -he.z) + off,
                    Vec3::new( he.x, -he.y,  he.z) + off,
                    Vec3::new( he.x, -he.y, -he.z) + off,
                    Vec3::new(-he.x,  he.y,  he.z) + off,
                    Vec3::new(-he.x,  he.y, -he.z) + off,
                    Vec3::new(-he.x, -he.y,  he.z) + off,
                    Vec3::new(-he.x, -he.y, -he.z) + off,
                ];
                let mut mn = Vec3::new(f32::MAX, f32::MAX, f32::MAX);
                let mut mx = Vec3::new(f32::MIN, f32::MIN, f32::MIN);
                for v in &corners {
                    let wv = t.position + t.rotation.mul_vec3(*v);
                    mn.x = mn.x.min(wv.x); mn.y = mn.y.min(wv.y); mn.z = mn.z.min(wv.z);
                    mx.x = mx.x.max(wv.x); mx.y = mx.y.max(wv.y); mx.z = mx.z.max(wv.z);
                }
                (mn, mx)
            }
        };

        // CCD: hızlı hareket eden objeler için AABB'yi hareket yönünde uzat
        if let Some(rb) = rigidbodies.get(e) {
            if rb.ccd_enabled {
                if let Some(v) = velocities.get(e) {
                    let sweep = v.linear * dt;
                    let offset_min: Vec3 = min + sweep;
                    let offset_max: Vec3 = max + sweep;
                    min = Vec3::new(min.x.min(offset_min.x), min.y.min(offset_min.y), min.z.min(offset_min.z));
                    max = Vec3::new(max.x.max(offset_max.x), max.y.max(offset_max.y), max.z.max(offset_max.z));
                }
            }
        }

        intervals.push(Interval { entity: e, min, max });
    }

    if intervals.is_empty() {
        return Vec::new();
    }

    // Merkezlerin varyansından en geniş dağılım eksenini seç
    let mut sum = Vec3::ZERO;
    let mut sum_sq = Vec3::ZERO;
    for iv in &intervals {
        let center = (iv.min + iv.max) * 0.5;
        sum += center;
        sum_sq.x += center.x * center.x;
        sum_sq.y += center.y * center.y;
        sum_sq.z += center.z * center.z;
    }
    let count = intervals.len() as f32;
    let mean = sum / count;
    let variance = Vec3::new(
        sum_sq.x / count - mean.x * mean.x,
        sum_sq.y / count - mean.y * mean.y,
        sum_sq.z / count - mean.z * mean.z,
    );
    let axis: u8 = if variance.y >= variance.x && variance.y >= variance.z {
        1
    } else if variance.z >= variance.x && variance.z >= variance.y {
        2
    } else {
        0
    };

    let min_on_axis = |iv: &Interval| -> f32 {
        if axis == 0 { iv.min.x } else if axis == 1 { iv.min.y } else { iv.min.z }
    };
    let max_on_axis = |iv: &Interval| -> f32 {
        if axis == 0 { iv.max.x } else if axis == 1 { iv.max.y } else { iv.max.z }
    };

    intervals.sort_unstable_by(|a, b| min_on_axis(a).total_cmp(&min_on_axis(b)));

    let len = intervals.len();
    let mut active_list: Vec<usize> = Vec::with_capacity(32);
    let mut pairs: Vec<(u32, u32)> = Vec::new();

    for i in 0..len {
        let cur_min = min_on_axis(&intervals[i]);
        active_list.retain(|&j| max_on_axis(&intervals[j]) >= cur_min);

        let a = &intervals[i];
        for &j in &active_list {
            let b = &intervals[j];
            let overlap = a.min.x <= b.max.x && a.max.x >= b.min.x
                && a.min.y <= b.max.y && a.max.y >= b.min.y
                && a.min.z <= b.max.z && a.max.z >= b.min.z;
            if overlap {
                let pair = if a.entity < b.entity {
                    (a.entity, b.entity)
                } else {
                    (b.entity, a.entity)
                };
                pairs.push(pair);
            }
        }
        active_list.push(i);
    }

    pairs.sort_unstable();
    pairs
}

/// FAZ 2 — Narrow-Phase: Her çarpışma çifti için GJK/EPA veya analitik test + CCD bisection.
///
/// Bağımsız hesaplamalar Rayon ile paralel çalışır; thread-safe immutable referanslar kullanır.
/// Her çift için `DetectionResult` döndürür (temas noktaları + uyandırılacak entityler).
fn detect_collisions(
    collision_pairs: &[(u32, u32)],
    transforms:  &gizmo_core::SparseSet<Transform>,
    colliders:   &gizmo_core::SparseSet<Collider>,
    rigidbodies: &gizmo_core::SparseSet<RigidBody>,
    velocities:  &gizmo_core::SparseSet<Velocity>,
    vehicle_entities: &std::collections::HashSet<u32>,
    has_vehicles: bool,
    dt: f32,
) -> Vec<DetectionResult> {
    use crate::shape::ColliderShape;
    use rayon::prelude::*;

    let t_dense  = &transforms.dense;
    let t_sparse = &transforms.sparse;
    let c_dense  = &colliders.dense;
    let c_sparse = &colliders.sparse;
    let rb_dense = &rigidbodies.dense;
    let rb_sparse = &rigidbodies.sparse;
    let v_dense  = &velocities.dense;
    let v_sparse = &velocities.sparse;
    let v_set    = vehicle_entities;

    collision_pairs
        .par_iter()
        .filter_map(|&(ent_a, ent_b)| {
            let rb_a = rb_sparse.get(&ent_a).map(|&i| &rb_dense[i])?;
            let rb_b = rb_sparse.get(&ent_b).map(|&i| &rb_dense[i])?;

            // İki statik obje asla etkileşemez
            if rb_a.data.mass == 0.0 && rb_b.data.mass == 0.0 {
                return None;
            }
            // İki dinamik uyuyan obje birbirini uyandıramaz
            let both_dynamic_sleeping = rb_a.data.mass > 0.0
                && rb_b.data.mass > 0.0
                && rb_a.data.is_sleeping
                && rb_b.data.is_sleeping;
            if both_dynamic_sleeping {
                return None;
            }
            // Araç ↔ statik çarpışmasını filtrele (raycast süspansiyon bunu yönetir)
            if has_vehicles
                && ((v_set.contains(&ent_a) && rb_b.data.mass == 0.0)
                    || (v_set.contains(&ent_b) && rb_a.data.mass == 0.0))
            {
                return None;
            }

            let col_a = c_sparse.get(&ent_a).map(|&i| &c_dense[i])?;
            let col_b = c_sparse.get(&ent_b).map(|&i| &c_dense[i])?;
            let t_a   = t_sparse.get(&ent_a).map(|&i| &t_dense[i])?;
            let t_b   = t_sparse.get(&ent_b).map(|&i| &t_dense[i])?;
            let (pos_a, rot_a) = (t_a.data.position, t_a.data.rotation);
            let (pos_b, rot_b) = (t_b.data.position, t_b.data.rotation);

            let mut ccd_pos_a = None;
            let mut ccd_pos_b = None;

            // Rotasyon kontrolü: AABB fast-path sadece eksen-hizalı kutular için güvenli
            let is_rot_a_identity = rot_a.x.abs() < 0.001 && rot_a.y.abs() < 0.001 && rot_a.z.abs() < 0.001;
            let is_rot_b_identity = rot_b.x.abs() < 0.001 && rot_b.y.abs() < 0.001 && rot_b.z.abs() < 0.001;

            let manifold = detect_pair(
                &col_a.data.shape, pos_a, rot_a, is_rot_a_identity,
                &col_b.data.shape, pos_b, rot_b, is_rot_b_identity,
            );

            // CCD bisection — sadece hızlı/mermi objeler için
            let manifold = if !manifold.is_colliding
                && (rb_a.data.ccd_enabled || rb_b.data.ccd_enabled)
            {
                let v_a_lin = v_sparse.get(&ent_a).map(|&i| v_dense[i].data.linear).unwrap_or(Vec3::ZERO);
                let v_b_lin = v_sparse.get(&ent_b).map(|&i| v_dense[i].data.linear).unwrap_or(Vec3::ZERO);
                let rel_v = v_b_lin - v_a_lin;

                if rel_v.length() * dt > 0.1 {
                    ccd_bisect(
                        &col_a.data.shape, pos_a, rot_a,
                        &col_b.data.shape, pos_b, rot_b,
                        v_a_lin, v_b_lin, rel_v, dt,
                        &mut ccd_pos_a, &mut ccd_pos_b,
                    )
                } else {
                    manifold
                }
            } else {
                manifold
            };

            if !manifold.is_colliding || manifold.contact_points.is_empty() {
                return None;
            }

            let inv_mass_a = if rb_a.data.mass == 0.0 { 0.0 } else { 1.0 / rb_a.data.mass };
            let inv_mass_b = if rb_b.data.mass == 0.0 { 0.0 } else { 1.0 / rb_b.data.mass };

            // Sadece zaten uyuyan dinamik objeleri uyandır
            let mut wakes = Vec::new();
            if rb_a.data.is_sleeping && rb_a.data.mass > 0.0 { wakes.push(ent_a); }
            if rb_b.data.is_sleeping && rb_b.data.mass > 0.0 { wakes.push(ent_b); }

            let mut result = DetectionResult { contacts: Vec::new(), wake_entities: wakes };

            for (contact_point, pen) in &manifold.contact_points {
                let mut r_a = *contact_point - pos_a;
                let mut r_b = *contact_point - pos_b;
                if let ColliderShape::Sphere(s) = &col_a.data.shape {
                    r_a = manifold.normal * s.radius;
                }
                if let ColliderShape::Sphere(s) = &col_b.data.shape {
                    r_b = manifold.normal * -s.radius;
                }
                result.contacts.push(StoredContact {
                    ent_a, ent_b,
                    normal: manifold.normal,
                    inv_mass_a, inv_mass_b,
                    inv_inertia_a: rb_a.data.inverse_inertia,
                    inv_inertia_b: rb_b.data.inverse_inertia,
                    restitution: rb_a.data.restitution.max(rb_b.data.restitution),
                    friction: (rb_a.data.friction * rb_b.data.friction).sqrt(),
                    penetration: *pen,
                    r_a, r_b,
                    rot_a: t_a.data.rotation,
                    rot_b: t_b.data.rotation,
                    accumulated_j: 0.0,
                    accumulated_friction: Vec3::ZERO,
                    ccd_offset_a: ccd_pos_a.unwrap_or(Vec3::ZERO),
                    ccd_offset_b: ccd_pos_b.unwrap_or(Vec3::ZERO),
                    world_point: *contact_point,
                });
            }

            Some(result)
        })
        .collect()
}

/// Tek bir çarpışma çifti için analitik veya GJK/EPA ile manifold üret.
fn detect_pair(
    shape_a: &crate::shape::ColliderShape, pos_a: Vec3, rot_a: gizmo_math::Quat, rot_a_identity: bool,
    shape_b: &crate::shape::ColliderShape, pos_b: Vec3, rot_b: gizmo_math::Quat, rot_b_identity: bool,
) -> crate::collision::CollisionManifold {
    use crate::shape::ColliderShape::*;

    match (shape_a, shape_b) {
        (Aabb(a1), Aabb(a2)) => {
            if rot_a_identity && rot_b_identity {
                crate::collision::check_aabb_aabb_manifold(pos_a, a1, pos_b, a2)
            } else {
                crate::collision::check_obb_obb_manifold(pos_a, rot_a, a1, pos_b, rot_b, a2)
            }
        }
        (Sphere(s), Aabb(a)) => {
            if rot_b_identity {
                crate::collision::check_sphere_aabb_manifold(pos_a, s, pos_b, a)
            } else {
                crate::collision::check_sphere_obb_manifold(pos_a, s, pos_b, rot_b, a)
            }
        }
        (Aabb(a), Sphere(s)) => {
            let mut m = if rot_a_identity {
                crate::collision::check_sphere_aabb_manifold(pos_b, s, pos_a, a)
            } else {
                crate::collision::check_sphere_obb_manifold(pos_b, s, pos_a, rot_a, a)
            };
            m.normal = -m.normal;
            m
        }
        (Capsule(c1), Capsule(c2)) => {
            crate::collision::check_capsule_capsule_manifold(pos_a, rot_a, c1, pos_b, rot_b, c2)
        }
        (Capsule(c), Sphere(s)) => {
            crate::collision::check_capsule_sphere_manifold(pos_a, rot_a, c, pos_b, s)
        }
        (Sphere(s), Capsule(c)) => {
            let mut m = crate::collision::check_capsule_sphere_manifold(pos_b, rot_b, c, pos_a, s);
            m.normal *= -1.0;
            m
        }
        (Capsule(c), Aabb(a)) => {
            crate::collision::check_capsule_aabb_manifold(pos_a, rot_a, c, pos_b, a)
        }
        (Aabb(a), Capsule(c)) => {
            let mut m = crate::collision::check_capsule_aabb_manifold(pos_b, rot_b, c, pos_a, a);
            m.normal *= -1.0;
            m
        }
        (Sphere(s1), Sphere(s2)) => {
            crate::collision::check_sphere_sphere_manifold(pos_a, s1, pos_b, s2)
        }
        _ => {
            // GJK + EPA fallback (ConvexHull ve karışık şekiller için)
            let (is_colliding, simplex) = crate::gjk::gjk_intersect(shape_a, pos_a, rot_a, shape_b, pos_b, rot_b);
            if is_colliding {
                crate::epa::epa_solve(simplex, shape_a, pos_a, rot_a, shape_b, pos_b, rot_b)
            } else {
                crate::collision::CollisionManifold {
                    is_colliding: false,
                    normal: Vec3::ZERO,
                    penetration: 0.0,
                    contact_points: vec![],
                }
            }
        }
    }
}

/// Sürekli Çarpışma Tespiti (CCD) — bisection yöntemi ile TOI (Time of Impact) bulur.
///
/// Mermi hızındaki nesnelerin bir frame'de tünel geçmesini önler.
/// `ccd_offset_a` / `ccd_offset_b` çıkışları, TOI anından itibaren pozisyon offsetidir.
fn ccd_bisect(
    shape_a: &crate::shape::ColliderShape, pos_a: Vec3, rot_a: gizmo_math::Quat,
    shape_b: &crate::shape::ColliderShape, pos_b: Vec3, rot_b: gizmo_math::Quat,
    v_a_lin: Vec3, v_b_lin: Vec3, rel_v: Vec3, dt: f32,
    ccd_offset_a: &mut Option<Vec3>,
    ccd_offset_b: &mut Option<Vec3>,
) -> crate::collision::CollisionManifold {
    // Ön test: [0, dt] boyunca hiç kesişme var mı?
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
        let t_mid  = (t_low + t_high) * 0.5;
        let pa_low = pos_a + v_a_lin * t_low;
        let pb_low = pos_b + v_b_lin * t_low;
        let sweep_half = crate::shape::ColliderShape::Swept {
            base: Box::new(shape_b.clone()),
            sweep_vector: (v_b_lin - v_a_lin) * (t_mid - t_low),
        };
        let (hit_first, _) = crate::gjk::gjk_intersect(shape_a, pa_low, rot_a, &sweep_half, pb_low, rot_b);
        if hit_first { t_high = t_mid; } else { t_low = t_mid; }
    }

    let t_hit  = (t_high + dt * 0.001).min(dt);
    let pa_hit = pos_a + v_a_lin * t_hit;
    let pb_hit = pos_b + v_b_lin * t_hit;

    let (hit, sim) = crate::gjk::gjk_intersect(shape_a, pa_hit, rot_a, shape_b, pb_hit, rot_b);
    if !hit {
        return crate::collision::CollisionManifold {
            is_colliding: false,
            normal: Vec3::ZERO,
            penetration: 0.0,
            contact_points: vec![],
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
        // Temas noktalarını TOI anına geri taşı
        let cp_offset = (v_a_lin + v_b_lin) * 0.5 * t_hit;
        for cp in &mut manifold.contact_points {
            cp.0 -= cp_offset;
        }
        *ccd_offset_a = Some(pa_hit - pos_a);
        *ccd_offset_b = Some(pb_hit - pos_b);
    }
    manifold
}

/// FAZ 3 — Island Generation: Union-Find (tam path compression + union-by-rank).
///
/// Birbirine temas eden dinamik entity'leri gruplara ayırır.
/// Her island bağımsız olarak çözülebilir → paralel solver mümkündür.
fn build_islands(
    detection_results: Vec<DetectionResult>,
    transforms:  &gizmo_core::SparseSet<Transform>,
    velocities:  &gizmo_core::SparseSet<Velocity>,
    entities_to_wake: &mut Vec<u32>,
) -> Vec<Island> {
    let mut parent_map: HashMap<u32, u32> = HashMap::new();
    let mut rank_map:   HashMap<u32, u8>  = HashMap::new();

    let mut all_contacts: Vec<StoredContact> = Vec::new();
    for result in detection_results {
        entities_to_wake.extend(result.wake_entities);
        for c in result.contacts {
            let a_dyn = c.inv_mass_a > 0.0;
            let b_dyn = c.inv_mass_b > 0.0;
            if a_dyn && b_dyn {
                ensure_node(&mut parent_map, &mut rank_map, c.ent_a);
                ensure_node(&mut parent_map, &mut rank_map, c.ent_b);
                union_nodes(&mut parent_map, &mut rank_map, c.ent_a, c.ent_b);
            } else if a_dyn {
                ensure_node(&mut parent_map, &mut rank_map, c.ent_a);
            } else if b_dyn {
                ensure_node(&mut parent_map, &mut rank_map, c.ent_b);
            }
            all_contacts.push(c);
        }
    }

    // Temasları island'lara dağıt
    let mut islands_map: HashMap<u32, Island> = HashMap::new();
    for c in all_contacts {
        let a_dyn = c.inv_mass_a > 0.0;
        let root = if a_dyn {
            find_root(&mut parent_map, c.ent_a)
        } else {
            find_root(&mut parent_map, c.ent_b)
        };
        let island = islands_map.entry(root).or_insert_with(|| Island {
            contacts: Vec::new(),
            velocities: HashMap::new(),
            poses: HashMap::new(),
        });
        island.contacts.push(c);
    }

    // Her island'a başlangıç hız ve pozisyon snapshot'larını aktar
    for island in islands_map.values_mut() {
        for c in &island.contacts {
            if c.inv_mass_a > 0.0 && !island.velocities.contains_key(&c.ent_a) {
                island.velocities.insert(
                    c.ent_a,
                    velocities.get(c.ent_a).cloned().unwrap_or(Velocity::new(Vec3::ZERO)),
                );
                let mut p = *transforms.get(c.ent_a).unwrap();
                p.position += c.ccd_offset_a;
                island.poses.insert(c.ent_a, p);
            }
            if c.inv_mass_b > 0.0 && !island.velocities.contains_key(&c.ent_b) {
                island.velocities.insert(
                    c.ent_b,
                    velocities.get(c.ent_b).cloned().unwrap_or(Velocity::new(Vec3::ZERO)),
                );
                let mut p = *transforms.get(c.ent_b).unwrap();
                p.position += c.ccd_offset_b;
                island.poses.insert(c.ent_b, p);
            }
        }
    }

    islands_map.into_values().collect()
}

/// FAZ 4 — Warm-Starting + SI Çözücü + Position Projection (paralel island başına).
///
/// Warm-start: önceki frame'in impulslarını %80 azaltarak başlangıç noktası olarak kullanır.
/// SI çözücü: Baumgarte stabilizasyonu + Coulomb sürtünme.
/// Position Projection: doğrudan pozisyon düzeltmesi (penetrasyon giderme).
fn solve_islands(
    islands: &mut Vec<Island>,
    contact_cache: &HashMap<(u32, u32), Vec<CachedContact>>,
    solver_iters: u32,
    frame_count: u64,
) {
    use rayon::prelude::*;

    // Warm-start: önceki frame'in impulslarını temas eşlemesiyle aktar
    for island in islands.iter_mut() {
        for c in island.contacts.iter_mut() {
            let key = if c.ent_a < c.ent_b { (c.ent_a, c.ent_b) } else { (c.ent_b, c.ent_a) };
            if let Some(cached) = contact_cache.get(&key) {
                if let Some((cached_j, cached_friction)) = match_cached_contact(c.world_point, cached) {
                    c.accumulated_j        = (cached_j * WARM_START_FACTOR).min(20.0);
                    c.accumulated_friction = cached_friction * WARM_START_FACTOR;
                }
            }
        }
    }

    // Warm-start impulslarını hızlara uygula
    for island in islands.iter_mut() {
        for c in island.contacts.iter() {
            if c.accumulated_j > 1e-6 {
                let impulse = c.normal * c.accumulated_j;
                if let Some(v_a) = island.velocities.get_mut(&c.ent_a) {
                    v_a.linear  -= impulse * c.inv_mass_a;
                    v_a.angular += apply_inv_inertia(c.r_a.cross(impulse * -1.0), c.inv_inertia_a, c.rot_a);
                }
                if let Some(v_b) = island.velocities.get_mut(&c.ent_b) {
                    v_b.linear  += impulse * c.inv_mass_b;
                    v_b.angular += apply_inv_inertia(c.r_b.cross(impulse), c.inv_inertia_b, c.rot_b);
                }
            }
            let fi = c.accumulated_friction;
            if fi.length_squared() > 1e-12 {
                if let Some(v) = island.velocities.get_mut(&c.ent_a) {
                    v.linear  -= fi * c.inv_mass_a;
                    v.angular += apply_inv_inertia(c.r_a.cross(fi * -1.0), c.inv_inertia_a, c.rot_a);
                }
                if let Some(v) = island.velocities.get_mut(&c.ent_b) {
                    v.linear  += fi * c.inv_mass_b;
                    v.angular += apply_inv_inertia(c.r_b.cross(fi), c.inv_inertia_b, c.rot_b);
                }
            }
        }
    }

    const MAX_ANG: f32 = 100.0;
    const MAX_LIN: f32 = 200.0;

    // Paralel island çözümü — Sequential Impulse + Coulomb Sürtünme + Position Projection
    islands.par_iter_mut().for_each(|island| {
        // Frame-seeded Fisher-Yates shuffle (çözüm bias'ını önler)
        let contacts_len = island.contacts.len();
        if contacts_len > 1 {
            let seed = frame_count as usize;
            for i in 0..(contacts_len - 1) {
                let range = contacts_len - i;
                let h = (i.wrapping_add(1).wrapping_mul(2654435761).wrapping_add(seed)) ^ seed;
                let swap_idx = i + (h % range);
                island.contacts.swap(i, swap_idx);
            }
        }

        // Sequential Impulse iterasyonları
        for _iter in 0..solver_iters {
            for c in island.contacts.iter_mut() {
                let va = island.velocities.get(&c.ent_a).cloned().unwrap_or(Velocity::new(Vec3::ZERO));
                let vb = island.velocities.get(&c.ent_b).cloned().unwrap_or(Velocity::new(Vec3::ZERO));

                let rel = (vb.linear + vb.angular.cross(c.r_b)) - (va.linear + va.angular.cross(c.r_a));
                let vn  = rel.dot(c.normal);

                let e = if vn.abs() < 0.2 { 0.0 } else { c.restitution };

                let ra_x_n = c.r_a.cross(c.normal);
                let rb_x_n = c.r_b.cross(c.normal);
                let ang_a  = apply_inv_inertia(ra_x_n, c.inv_inertia_a, c.rot_a).cross(c.r_a).dot(c.normal);
                let ang_b  = apply_inv_inertia(rb_x_n, c.inv_inertia_b, c.rot_b).cross(c.r_b).dot(c.normal);
                let eff_mass = c.inv_mass_a + c.inv_mass_b + ang_a + ang_b;
                if eff_mass == 0.0 { continue; }

                // Baumgarte stabilizasyonu
                let bias     = ((0.2 / 1.0) * (c.penetration - 0.005).max(0.0)).min(20.0);
                let j_new    = (-(1.0 + e) * vn + bias) / eff_mass;
                let old_acc  = c.accumulated_j;
                c.accumulated_j = (c.accumulated_j + j_new).max(0.0);
                let j = c.accumulated_j - old_acc;

                if j.abs() > 1e-8 {
                    let impulse = c.normal * j;
                    if let Some(v_a) = island.velocities.get_mut(&c.ent_a) {
                        v_a.linear  -= impulse * c.inv_mass_a;
                        v_a.linear.x = v_a.linear.x.clamp(-MAX_LIN, MAX_LIN);
                        v_a.linear.y = v_a.linear.y.clamp(-MAX_LIN, MAX_LIN);
                        v_a.linear.z = v_a.linear.z.clamp(-MAX_LIN, MAX_LIN);
                        v_a.angular += apply_inv_inertia(c.r_a.cross(impulse * -1.0), c.inv_inertia_a, c.rot_a);
                        v_a.angular.x = v_a.angular.x.clamp(-MAX_ANG, MAX_ANG);
                        v_a.angular.y = v_a.angular.y.clamp(-MAX_ANG, MAX_ANG);
                        v_a.angular.z = v_a.angular.z.clamp(-MAX_ANG, MAX_ANG);
                    }
                    if let Some(v_b) = island.velocities.get_mut(&c.ent_b) {
                        v_b.linear  += impulse * c.inv_mass_b;
                        v_b.linear.x = v_b.linear.x.clamp(-MAX_LIN, MAX_LIN);
                        v_b.linear.y = v_b.linear.y.clamp(-MAX_LIN, MAX_LIN);
                        v_b.linear.z = v_b.linear.z.clamp(-MAX_LIN, MAX_LIN);
                        v_b.angular += apply_inv_inertia(c.r_b.cross(impulse), c.inv_inertia_b, c.rot_b);
                        v_b.angular.x = v_b.angular.x.clamp(-MAX_ANG, MAX_ANG);
                        v_b.angular.y = v_b.angular.y.clamp(-MAX_ANG, MAX_ANG);
                        v_b.angular.z = v_b.angular.z.clamp(-MAX_ANG, MAX_ANG);
                    }
                }

                // Coulomb Sürtünme (statik/kinetik)
                let va2 = island.velocities.get(&c.ent_a).cloned().unwrap_or(Velocity::new(Vec3::ZERO));
                let vb2 = island.velocities.get(&c.ent_b).cloned().unwrap_or(Velocity::new(Vec3::ZERO));
                let rel2 = (vb2.linear + vb2.angular.cross(c.r_b)) - (va2.linear + va2.angular.cross(c.r_a));
                let tangent_vel = rel2 - c.normal * rel2.dot(c.normal);
                let ts = tangent_vel.length();

                if ts > 0.001 {
                    let tangent_dir = tangent_vel / ts;
                    let ra_cross_t  = c.r_a.cross(tangent_dir);
                    let rb_cross_t  = c.r_b.cross(tangent_dir);
                    let tangent_eff_mass = c.inv_mass_a + c.inv_mass_b
                        + apply_inv_inertia(ra_cross_t, c.inv_inertia_a, c.rot_a).cross(c.r_a).dot(tangent_dir)
                        + apply_inv_inertia(rb_cross_t, c.inv_inertia_b, c.rot_b).cross(c.r_b).dot(tangent_dir);

                    if tangent_eff_mass > 0.0 {
                        let jt            = -ts / tangent_eff_mass;
                        let max_friction  = c.accumulated_j * c.friction;
                        let old_friction  = c.accumulated_friction;
                        let mut new_friction = old_friction + tangent_dir * jt;
                        let friction_len  = new_friction.length();
                        if friction_len > max_friction {
                            let kinetic_limit = c.accumulated_j * (c.friction * 0.7);
                            new_friction *= kinetic_limit / friction_len;
                        }
                        let fi = new_friction - old_friction;
                        c.accumulated_friction = new_friction;

                        if let Some(v) = island.velocities.get_mut(&c.ent_a) {
                            v.linear  -= fi * c.inv_mass_a;
                            v.angular += apply_inv_inertia(c.r_a.cross(fi * -1.0), c.inv_inertia_a, c.rot_a);
                            v.angular.x = v.angular.x.clamp(-MAX_ANG, MAX_ANG);
                            v.angular.y = v.angular.y.clamp(-MAX_ANG, MAX_ANG);
                            v.angular.z = v.angular.z.clamp(-MAX_ANG, MAX_ANG);
                        }
                        if let Some(v) = island.velocities.get_mut(&c.ent_b) {
                            v.linear  += fi * c.inv_mass_b;
                            v.angular += apply_inv_inertia(c.r_b.cross(fi), c.inv_inertia_b, c.rot_b);
                            v.angular.x = v.angular.x.clamp(-MAX_ANG, MAX_ANG);
                            v.angular.y = v.angular.y.clamp(-MAX_ANG, MAX_ANG);
                            v.angular.z = v.angular.z.clamp(-MAX_ANG, MAX_ANG);
                        }
                    }
                }
            }
        }

        // Position Projection — nesneleri doğrudan penetrasyon derinliği kadar ayır
        for c in &island.contacts {
            let correction = (c.penetration - 0.005).max(0.0) * 0.4;
            if correction > 0.0 {
                let total_inv = c.inv_mass_a + c.inv_mass_b;
                if total_inv > 0.0 {
                    let push = c.normal * (correction / total_inv);
                    if let Some(p) = island.poses.get_mut(&c.ent_a) { p.position -= push * c.inv_mass_a; }
                    if let Some(p) = island.poses.get_mut(&c.ent_b) { p.position += push * c.inv_mass_b; }
                }
            }
        }
    });
}

/// FAZ 5 — Write-Back: çözüm sonuçlarını ECS'e yaz, cache'i güncelle, eventleri fırlat.
fn write_back(
    islands: Vec<Island>,
    transforms:       &mut gizmo_core::SparseSet<Transform>,
    velocities:       &mut gizmo_core::SparseSet<Velocity>,
    vehicle_entities: &std::collections::HashSet<u32>,
    solver_state:     &mut PhysicsSolverState,
    collision_events: &mut Vec<crate::CollisionEvent>,
) {
    solver_state.contact_cache.clear();

    for island in islands {
        let Island { contacts, velocities: island_vels, poses } = island;

        for (ent, vel) in &island_vels {
            // Araç entity'lerinin hızına dokunma — vehicle_system yönetir
            if vehicle_entities.contains(ent) { continue; }
            if let Some(v) = velocities.get_mut(*ent) { *v = *vel; }
        }
        for (ent, tbox) in &poses {
            if let Some(t) = transforms.get_mut(*ent) {
                *t = *tbox;
                t.update_local_matrix();
            }
        }
        for c in contacts {
            // Warm-start cache kaydı
            let wp  = poses.get(&c.ent_a).map(|p| p.position + c.r_a).unwrap_or(c.world_point);
            let key = if c.ent_a < c.ent_b { (c.ent_a, c.ent_b) } else { (c.ent_b, c.ent_a) };
            let entry = solver_state.contact_cache.entry(key).or_insert_with(Vec::new);
            if entry.len() < 4 {
                entry.push(CachedContact {
                    world_point: wp,
                    accumulated_normal: c.accumulated_j,
                    accumulated_friction: c.accumulated_friction,
                });
            }

            // Darbe/momentum event'i fırlat (adaptif eşik)
            let eff_mass  = 1.0 / (c.inv_mass_a + c.inv_mass_b).max(0.0001);
            let threshold = 0.05 * eff_mass + 0.01;
            if c.accumulated_j > threshold {
                let pos_a = poses.get(&c.ent_a).map(|t| t.position).unwrap_or(Vec3::ZERO);
                collision_events.push(crate::CollisionEvent {
                    entity_a: c.ent_a,
                    entity_b: c.ent_b,
                    position: pos_a + c.r_a,
                    normal:   c.normal,
                    impulse:  c.accumulated_j,
                });
            }
        }
    }
}

// ─── Ana Giriş Noktası ────────────────────────────────────────────────────────

/// Tek frame için tüm çarpışma pipeline'ını çalıştırır:
///
/// ```text
/// broad_phase → detect_collisions → build_islands → solve_islands → write_back
/// ```
///
/// Her aşama bağımsız, test edilebilir bir fonksiyon; yan etkiler yalnızca
/// `write_back` içinde ECS'e yansıtılır.
pub fn physics_collision_system(world: &mut World, dt: f32) {
    let mut entities_to_wake: Vec<u32> = Vec::new();
    let mut collision_events: Vec<crate::CollisionEvent> = Vec::new();

    {
        // Borrow scope — tüm ECS borrow'ları burada yaşar
        let mut transforms  = match world.borrow_mut::<Transform>()  { Some(t) => t, None => return };
        let mut velocities  = match world.borrow_mut::<Velocity>()   { Some(v) => v, None => return };
        let colliders       = match world.borrow::<Collider>()        { Some(c) => c, None => return };
        let rigidbodies     = match world.borrow::<RigidBody>()       { Some(r) => r, None => return };
        let vehicles        = world.borrow::<VehicleController>();

        let vehicle_entities: std::collections::HashSet<u32> = match &vehicles {
            Some(v) => v.dense.iter().map(|e| e.entity).collect(),
            None    => std::collections::HashSet::new(),
        };
        let has_vehicles = vehicles.is_some();

        // 1. Broad-phase — olası çarpışma çiftleri
        let collision_pairs = broad_phase(&transforms, &colliders, &rigidbodies, &velocities, dt);
        if collision_pairs.is_empty() {
            return;
        }

        // 2. Narrow-phase — gerçek temas tespiti (paralel)
        let detection_results = detect_collisions(
            &collision_pairs,
            &transforms, &colliders, &rigidbodies, &velocities,
            &vehicle_entities, has_vehicles, dt,
        );

        // 3. Island generation — Union-Find ile gruplama
        let mut islands = build_islands(detection_results, &transforms, &velocities, &mut entities_to_wake);

        // 4. Çözücü — warm-start + SI + position projection (paralel island başına)
        let (solver_iters, frame_count) =
            if let Some(state) = world.get_resource_mut::<PhysicsSolverState>() {
                (state.solver_iterations, state.frame_counter)
            } else {
                #[cfg(debug_assertions)]
                eprintln!(
                    "[Physics WARN] PhysicsSolverState bulunamadı. \
                     Warm-start devre dışı. world.insert_resource(PhysicsSolverState::new()) ekleyin."
                );
                (8, 0)
            };

        let contact_cache = if let Some(state) = world.get_resource_mut::<PhysicsSolverState>() {
            state.contact_cache.clone()
        } else {
            HashMap::new()
        };

        solve_islands(&mut islands, &contact_cache, solver_iters, frame_count);

        // 5. Write-back — ECS + cache + event
        if let Some(mut state) = world.get_resource_mut::<PhysicsSolverState>() {
            state.frame_counter += 1;
            write_back(
                islands,
                &mut transforms,
                &mut velocities,
                &vehicle_entities,
                &mut state,
                &mut collision_events,
            );
        } else {
            // State yoksa collision event'leri ve hız yazımlarını yine de yap
            let mut dummy_state = PhysicsSolverState::new();
            write_back(
                islands,
                &mut transforms,
                &mut velocities,
                &vehicle_entities,
                &mut dummy_state,
                &mut collision_events,
            );
        }
    } // Borrow scope sonu

    // Event kuyruğuna yaz
    if !collision_events.is_empty() {
        let mut evs = world.get_resource_mut_or_default::<gizmo_core::event::Events<crate::CollisionEvent>>();
        for ev in collision_events {
            evs.push(ev);
        }
    }

    // Uyuyan nesneleri uyandır
    if !entities_to_wake.is_empty() {
        if let Some(mut rbs) = world.borrow_mut::<RigidBody>() {
            for e in entities_to_wake {
                if let Some(rb) = rbs.get_mut(e) {
                    rb.wake_up();
                }
            }
        }
    }
}
