use gizmo_math::Vec3;
use crate::components::{Transform, RigidBody, Velocity};
use crate::shape::Collider;
use super::types::Interval;


/// FAZ 1 — Broad-Phase: Sweep & Prune (active-list, dinamik eksen seçimi)
///
/// Her entity'nin dünya-uzayı AABB'sini hesaplar (CCD sweep dahil),
/// her karede **her eksen için tüm AABB uçları** (`min`/`max` köşe projeksiyonları)
/// üzerinden varyans hesaplanır; en yüksek varyanslı eksende sıralama yapılır.
/// (Yalnızca merkez varyansı, X'te ince ama Y/Z'te geniş düzenlerde zayıf kalabilir.)
/// Aktif liste, seçilen eksende `max[j] < min[i]` ile güvenli biçimde budanır; çift testi tam 3B AABB.
pub fn broad_phase(
    transforms: &gizmo_core::SparseSet<Transform>,
    colliders: &gizmo_core::SparseSet<Collider>,
    rigidbodies: &gizmo_core::SparseSet<RigidBody>,
    velocities: &gizmo_core::SparseSet<Velocity>,
    dt: f32,
) -> Vec<(u32, u32)> {
    use crate::shape::ColliderShape;

    let entities: Vec<u32> = transforms.iter().map(|(e, _)| e).collect();
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
                let is_identity = t.rotation.x.abs() < 0.001 && t.rotation.y.abs() < 0.001 && t.rotation.z.abs() < 0.001 && t.rotation.w.abs() > 0.999;
                if is_identity {
                    (t.position - he, t.position + he)
                } else {
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
                    let mut mn = Vec3::new(f32::INFINITY, f32::INFINITY, f32::INFINITY);
                    let mut mx = Vec3::new(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
                    for v in &corners {
                        let wv = t.position + t.rotation.mul_vec3(*v);
                        mn.x = mn.x.min(wv.x); mn.y = mn.y.min(wv.y); mn.z = mn.z.min(wv.z);
                        mx.x = mx.x.max(wv.x); mx.y = mx.y.max(wv.y); mx.z = mx.z.max(wv.z);
                    }
                    (mn, mx)
                }
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
                let mut mn = Vec3::new(f32::INFINITY, f32::INFINITY, f32::INFINITY);
                let mut mx = Vec3::new(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
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
                let is_identity = t.rotation.x.abs() < 0.001 && t.rotation.y.abs() < 0.001 && t.rotation.z.abs() < 0.001 && t.rotation.w.abs() > 0.999;
                if is_identity {
                    let center = t.position + off;
                    (center - he, center + he)
                } else {
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
                    let mut mn = Vec3::new(f32::INFINITY, f32::INFINITY, f32::INFINITY);
                    let mut mx = Vec3::new(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
                    for v in &corners {
                        let wv = t.position + t.rotation.mul_vec3(*v);
                        mn.x = mn.x.min(wv.x); mn.y = mn.y.min(wv.y); mn.z = mn.z.min(wv.z);
                        mx.x = mx.x.max(wv.x); mx.y = mx.y.max(wv.y); mx.z = mx.z.max(wv.z);
                    }
                    (mn, mx)
                }
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

    // Eksen başına: o eksendeki tüm min/max uçları (2N örnek) üzerinden varyans.
    // Böylece Y/Z'de geniş, X'te dar kutular da doğru eksende sıralanır.
    let axis_endpoint_variance = |axis: u8| -> f32 {
        let mut sum = 0.0f32;
        let mut sum_sq = 0.0f32;
        for iv in &intervals {
            let (lo, hi) = match axis {
                0 => (iv.min.x, iv.max.x),
                1 => (iv.min.y, iv.max.y),
                _ => (iv.min.z, iv.max.z),
            };
            sum += lo + hi;
            sum_sq += lo * lo + hi * hi;
        }
        let count = (intervals.len() * 2) as f32;
        let mean = sum / count;
        (sum_sq / count - mean * mean).max(0.0)
    };

    let vx = axis_endpoint_variance(0);
    let vy = axis_endpoint_variance(1);
    let vz = axis_endpoint_variance(2);

    let axis: u8 = if vy >= vx && vy >= vz {
        1
    } else if vz >= vx && vz >= vy {
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
