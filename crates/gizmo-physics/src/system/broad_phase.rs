use gizmo_math::Vec3;
use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};
use std::collections::BinaryHeap;
use crate::components::{Transform, RigidBody, Velocity};
use crate::shape::Collider;
use super::types::Interval;

#[derive(Clone, Copy)]
struct ActiveItem {
    max_val: f32,
    index: usize,
}
impl PartialEq for ActiveItem {
    fn eq(&self, other: &Self) -> bool { self.max_val == other.max_val }
}
impl Eq for ActiveItem {}
impl PartialOrd for ActiveItem {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        // Min-heap için ters sıralama (küçük max_val en üstte olur)
        other.max_val.partial_cmp(&self.max_val)
    }
}
impl Ord for ActiveItem {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.partial_cmp(other).unwrap_or(std::cmp::Ordering::Equal)
    }
}
use crate::system::types::is_near_identity;

static LAST_AXIS: AtomicU8 = AtomicU8::new(0);
static FRAME_COUNT: AtomicU32 = AtomicU32::new(0);

/// FAZ 1 — Broad-Phase: Sweep & Prune (active-list, dinamik eksen seçimi)
/// 
/// CONTRACT / RETURNS:
/// Çarpışan her çift (Entity_A, Entity_B) şeklinde döndürülür ve HİÇBİR ZAMAN
/// tersi dönmez. Daima `Entity_A < Entity_B` garantisi vardır.
/// Narrow-phase, çarpışma algoritmaları ve warm-starting önbelleği manifoldları
/// doğru hash/eşleştirme yapabilmek için bu sıralamaya DAİMA güvenir.
///
/// Her entity'nin dünya-uzayı AABB'sini hesaplar (CCD sweep dahil),
/// her karede **her eksen için tüm AABB uçları** (`min`/`max` köşe projeksiyonları)
/// üzerinden varyans hesaplanır; en yüksek varyanslı eksende sıralama yapılır.
/// (Yalnızca merkez varyansı, X'te ince ama Y/Z'te geniş düzenlerde zayıf kalabilir.)
/// Aktif liste, seçilen eksende `max[j] < min[i]` ile güvenli biçimde budanır; çift testi tam 3B AABB.
pub fn broad_phase(
    transforms: &gizmo_core::StorageView<'_, Transform>,
    colliders: &gizmo_core::StorageView<'_, Collider>,
    rigidbodies: &gizmo_core::StorageView<'_, RigidBody>,
    velocities: &gizmo_core::StorageView<'_, Velocity>,
    dt: f32,
    parallel_physics: bool,
) -> Vec<(u32, u32)> {
    use crate::shape::ColliderShape;

    assert!(dt >= 0.0, "Timestep dt must be non-negative");

    let entities: Vec<u32> = transforms.iter().map(|(e, _)| e).collect();

    let map_fn = |&e: &u32| -> Option<Interval> {
        let t = transforms.get(e)?;
        let col = colliders.get(e)?;

        let (mut min, mut max) = match &col.shape {
            ColliderShape::Aabb(a) => {
                // Collider half_extents zaten dünya uzayında — scale UYGULANMAZ.
                // Narrow-phase de scale uygulamaz; burada tutarlı olmalıyız.
                let he = a.half_extents;
                let is_identity = is_near_identity(t.rotation);
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
                // Sphere radius zaten dünya uzayında — scale UYGULANMAZ.
                let r = Vec3::splat(s.radius);
                (t.position - r, t.position + r)
            }
            ColliderShape::Capsule(c) => {
                // Capsule boyutları zaten dünya uzayında — scale UYGULANMAZ.
                let up = t.rotation.mul_vec3(Vec3::new(0.0, c.half_height, 0.0));
                let top = t.position + up;
                let bot = t.position - up;
                let r = Vec3::splat(c.radius);
                let mn = Vec3::new(top.x.min(bot.x), top.y.min(bot.y), top.z.min(bot.z)) - r;
                let mx = Vec3::new(top.x.max(bot.x), top.y.max(bot.y), top.z.max(bot.z)) + r;
                (mn, mx)
            }
            ColliderShape::ConvexHull(hull) => {
                // Hull vertex'leri zaten dünya uzayında — scale UYGULANMAZ.
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
                static WARNED_SWEPT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
                if !WARNED_SWEPT.swap(true, Ordering::Relaxed) {
                    eprintln!("[Physics WARN] Swept shape found in ECS for entity {}! Skipping. (Further swept warnings suppressed)", e);
                }
                return None;
            }
            ColliderShape::HeightField { width, max_height, depth, .. } => {
                // HeightField boyutları zaten dünya uzayında — scale UYGULANMAZ.
                let is_identity = is_near_identity(t.rotation);
                if is_identity {
                    (t.position, t.position + Vec3::new(*width, *max_height, *depth))
                } else {
                    let corners = [
                        Vec3::new(0.0, 0.0, 0.0),
                        Vec3::new(*width, 0.0, 0.0),
                        Vec3::new(0.0, *max_height, 0.0),
                        Vec3::new(*width, *max_height, 0.0),
                        Vec3::new(0.0, 0.0, *depth),
                        Vec3::new(*width, 0.0, *depth),
                        Vec3::new(0.0, *max_height, *depth),
                        Vec3::new(*width, *max_height, *depth),
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
        let rb_opt = rigidbodies.get(e);
        if let Some(rb) = rb_opt {
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

        let (is_sleeping, is_static) = match rb_opt {
            Some(rb) => (rb.is_sleeping, rb.mass == 0.0),
            None => (false, true),
        };

        Some(Interval { entity: e, min, max, is_sleeping, is_static })
    };

    let mut intervals: Vec<Interval> = if parallel_physics {
        use rayon::prelude::*;
        entities.par_iter().filter_map(map_fn).collect()
    } else {
        entities.iter().filter_map(map_fn).collect()
    };

    if intervals.is_empty() {
        return Vec::new();
    }

    // Eksen başına: o eksendeki tüm min/max uçları (2N örnek) üzerinden varyans hesaplanır.
    // Performans için aralıklarla (örneğin 60 karede bir) tek döngüde yenilenir ve atomik olarak önbelleğe alınır.
    let frame = FRAME_COUNT.fetch_add(1, Ordering::Relaxed);
    let axis: u8;

    if frame % 60 == 0 || intervals.len() < 10 {
        let count = (intervals.len() * 2) as f32;
        let mut sum_x = 0.0; let mut sum_y = 0.0; let mut sum_z = 0.0;
        let mut sq_x = 0.0;  let mut sq_y = 0.0;  let mut sq_z = 0.0;

        for iv in &intervals {
            sum_x += iv.min.x + iv.max.x;
            sum_y += iv.min.y + iv.max.y;
            sum_z += iv.min.z + iv.max.z;

            sq_x += iv.min.x * iv.min.x + iv.max.x * iv.max.x;
            sq_y += iv.min.y * iv.min.y + iv.max.y * iv.max.y;
            sq_z += iv.min.z * iv.min.z + iv.max.z * iv.max.z;
        }

        let mean_x = sum_x / count;
        let mean_y = sum_y / count;
        let mean_z = sum_z / count;

        let vx = (sq_x / count - mean_x * mean_x).max(0.0);
        let vy = (sq_y / count - mean_y * mean_y).max(0.0);
        let vz = (sq_z / count - mean_z * mean_z).max(0.0);

        axis = if vy >= vx && vy >= vz {
            1
        } else if vz >= vx && vz >= vy {
            2
        } else {
            0
        };
        LAST_AXIS.store(axis, Ordering::Relaxed);
    } else {
        axis = LAST_AXIS.load(Ordering::Relaxed);
    }

    let min_on_axis = |iv: &Interval| -> f32 {
        if axis == 0 { iv.min.x } else if axis == 1 { iv.min.y } else { iv.min.z }
    };
    let max_on_axis = |iv: &Interval| -> f32 {
        if axis == 0 { iv.max.x } else if axis == 1 { iv.max.y } else { iv.max.z }
    };

    intervals.sort_unstable_by(|a, b| min_on_axis(a).total_cmp(&min_on_axis(b)));

    let len = intervals.len();
    let mut active_list: BinaryHeap<ActiveItem> = BinaryHeap::with_capacity(len);
    let mut pairs: Vec<(u32, u32)> = Vec::new();

    for i in 0..len {
        let cur_min = min_on_axis(&intervals[i]);
        
        while let Some(top) = active_list.peek() {
            if top.max_val < cur_min {
                active_list.pop();
            } else {
                break;
            }
        }

        let a = &intervals[i];
        for item in active_list.iter() {
            let j = item.index;
            let b = &intervals[j];
            let overlap = a.min.x <= b.max.x && a.max.x >= b.min.x
                && a.min.y <= b.max.y && a.max.y >= b.min.y
                && a.min.z <= b.max.z && a.max.z >= b.min.z;
            if overlap {
                // Erken eleme: her iki taraf da uyuyor veya her ikisi de statik → çift gereksiz.
                // NOT: sleeping+static filtrelenmez çünkü uyuyan cisim aynı frame'de
                // başka bir uyanık cisim tarafından uyandırılabilir ve zemin desteğine ihtiyaç duyar.
                let both_sleeping = a.is_sleeping && b.is_sleeping;
                let both_static = a.is_static && b.is_static;
                if both_sleeping || both_static {
                    continue;
                }

                // CONTRACT: Çiftler her zaman `a < b` (entity) şeklinde sıralıdır.
                let pair = if a.entity < b.entity {
                    (a.entity, b.entity)
                } else {
                    (b.entity, a.entity)
                };
                pairs.push(pair);
            }
        }
        active_list.push(ActiveItem { max_val: max_on_axis(&intervals[i]), index: i });
    }

    pairs.sort_unstable();
    pairs.dedup();
    pairs
}
