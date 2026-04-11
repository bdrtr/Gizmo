use crate::components::{RigidBody, Transform, Velocity};
use crate::integration::apply_inv_inertia;
use crate::shape::Collider;
use crate::vehicle::VehicleController;
use gizmo_core::World;
use gizmo_math::Vec3;
use std::collections::HashMap;

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

pub fn physics_collision_system(world: &mut World, dt: f32) {
    // Wake-up listesi — collision scope dışında tanımlanıyor
    let mut entities_to_wake: Vec<u32> = Vec::new();

    let mut collision_events = Vec::new();
    {
        // --- Borrow Scope Başlangıcı (immutable rigidbodies + mutable transforms/velocities) ---
        let mut transforms = match world.borrow_mut::<Transform>() {
            Some(t) => t,
            None => {
                return;
            }
        };
        let mut velocities = match world.borrow_mut::<Velocity>() {
            Some(v) => v,
            None => {
                return;
            }
        };
        let colliders = match world.borrow::<Collider>() {
            Some(c) => c,
            None => {
                return;
            }
        };
        let rigidbodies = match world.borrow::<RigidBody>() {
            Some(r) => r,
            None => {
                return;
            }
        };

        // Collision Layer: VehicleController olan entity'ler statik objelerle çarpışmaz
        let vehicles = world.borrow::<VehicleController>();

        // 1. BROAD-PHASE: Sweep and Prune (3D Bounding Box / AABB Filtrelemesi)
        struct Interval {
            entity: u32,
            min: Vec3,
            max: Vec3,
        }
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

            use crate::shape::ColliderShape;
            let (mut min, mut max) = match &col.shape {
                ColliderShape::Aabb(a) => {
                    // AABB'yi gerçek rotasyona bağlı OBB gibi taramak (Broad-phase min-max sınırları)
                    let he = a.half_extents;
                    let corners = [
                        Vec3::new(he.x, he.y, he.z),
                        Vec3::new(he.x, he.y, -he.z),
                        Vec3::new(he.x, -he.y, he.z),
                        Vec3::new(he.x, -he.y, -he.z),
                        Vec3::new(-he.x, he.y, he.z),
                        Vec3::new(-he.x, he.y, -he.z),
                        Vec3::new(-he.x, -he.y, he.z),
                        Vec3::new(-he.x, -he.y, -he.z),
                    ];
                    let mut mn = Vec3::new(f32::MAX, f32::MAX, f32::MAX);
                    let mut mx = Vec3::new(f32::MIN, f32::MIN, f32::MIN);
                    for v in &corners {
                        let wv = t.position + t.rotation.mul_vec3(*v);
                        mn.x = mn.x.min(wv.x);
                        mn.y = mn.y.min(wv.y);
                        mn.z = mn.z.min(wv.z);
                        mx.x = mx.x.max(wv.x);
                        mx.y = mx.y.max(wv.y);
                        mx.z = mx.z.max(wv.z);
                    }
                    (mn, mx)
                }
                ColliderShape::Sphere(s) => {
                    let radius_vec = Vec3::new(s.radius, s.radius, s.radius);
                    (t.position - radius_vec, t.position + radius_vec)
                }
                ColliderShape::Capsule(c) => {
                    // Kapsülün rotasyonuna göre sıkı AABB hesapla
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
                        mn.x = mn.x.min(wv.x);
                        mn.y = mn.y.min(wv.y);
                        mn.z = mn.z.min(wv.z);
                        mx.x = mx.x.max(wv.x);
                        mx.y = mx.y.max(wv.y);
                        mx.z = mx.z.max(wv.z);
                    }
                    (mn, mx)
                }
                ColliderShape::Swept { .. } => {
                    eprintln!(
                        "[Physics WARN] Swept shape found in ECS for entity {}! Skipping.",
                        e
                    );
                    continue;
                }
                ColliderShape::HeightField {
                    width,
                    max_height,
                    depth,
                    ..
                } => {
                    let he = Vec3::new(*width * 0.5, *max_height * 0.5, *depth * 0.5);
                    let center_offset = Vec3::new(0.0, *max_height * 0.5, 0.0);
                    let corners = [
                        Vec3::new(he.x, he.y, he.z) + center_offset,
                        Vec3::new(he.x, he.y, -he.z) + center_offset,
                        Vec3::new(he.x, -he.y, he.z) + center_offset,
                        Vec3::new(he.x, -he.y, -he.z) + center_offset,
                        Vec3::new(-he.x, he.y, he.z) + center_offset,
                        Vec3::new(-he.x, he.y, -he.z) + center_offset,
                        Vec3::new(-he.x, -he.y, he.z) + center_offset,
                        Vec3::new(-he.x, -he.y, -he.z) + center_offset,
                    ];
                    let mut mn = Vec3::new(f32::MAX, f32::MAX, f32::MAX);
                    let mut mx = Vec3::new(f32::MIN, f32::MIN, f32::MIN);
                    for v in &corners {
                        let wv = t.position + t.rotation.mul_vec3(*v);
                        mn.x = mn.x.min(wv.x);
                        mn.y = mn.y.min(wv.y);
                        mn.z = mn.z.min(wv.z);
                        mx.x = mx.x.max(wv.x);
                        mx.y = mx.y.max(wv.y);
                        mx.z = mx.z.max(wv.z);
                    }
                    (mn, mx)
                }
            };

            // --- YENİ: CCD Modeli Broad-Phase Taraması (Sweeping) ---
            // Eğer obje çok hızlı (mermi) ise, gideceği yere kadar AABB kutusunu sosis gibi uzat.
            if let Some(rb) = rigidbodies.get(e) {
                if rb.ccd_enabled {
                    if let Some(v) = velocities.get(e) {
                        let sweep = v.linear * dt;
                        // Kutunun o anki durumuyla, son durumu (sweep'lenmiş) birleşimi yeni büyük kutuyu oluşturur
                        let offset_min = min + sweep;
                        let offset_max = max + sweep;
                        min = min.min(offset_min);
                        max = max.max(offset_max);
                    }
                }
            }

            intervals.push(Interval {
                entity: e,
                min,
                max,
            });
        }

        // =========================================================================
        // 1D Sweep & Prune Broadphase - O(N log N)
        // En yüksek varyansa sahip ekseni bularak dinamik sıralama/pruning yapar.
        // =========================================================================

        let mut collision_pairs: Vec<(u32, u32)> = Vec::new();
        if intervals.is_empty() {
            return;
        }

        // 1. Merkezlerin eksenlere göre varyansını (dağılımını) hesapla
        let mut sum = Vec3::ZERO;
        let mut sum_sq = Vec3::ZERO;
        for i in &intervals {
            let center = (i.min + i.max) * 0.5;
            sum += center;
            sum_sq += center * center;
        }
        let count = intervals.len() as f32;
        let mean = sum / count;
        let variance = sum_sq / count - mean * mean;

        // En geniş dağılım (variance) hangi eksendeyse o ekseni seç
        let mut axis = 0; // 0: X, 1: Y, 2: Z
        if variance.y > variance.x && variance.y > variance.z {
            axis = 1;
        } else if variance.z > variance.x && variance.z > variance.y {
            axis = 2;
        }

        // 2. Seçilen eksenin min değerine göre hızlıca (unstable_sort) sırala
        intervals.sort_unstable_by(|a, b| {
            if axis == 0 {
                a.min.x.total_cmp(&b.min.x)
            } else if axis == 1 {
                a.min.y.total_cmp(&b.min.y)
            } else {
                a.min.z.total_cmp(&b.min.z)
            }
        });

        // 3. Sıralanmış listeyi Linear olarak tara (Sweep)
        let len = intervals.len();
        for i in 0..len {
            let a = &intervals[i];
            for j in (i + 1)..len {
                let b = &intervals[j];

                // (Prune) Eğer b'nin sol kenarı (min), seçili eksende a'nın sağ kenarını geçiyorsa,
                // Liste sıralı olduğu için geri kalan HİÇBİR obje a ile kesişemez. Döngüyü kır.
                let prune = if axis == 0 {
                    b.min.x > a.max.x
                } else if axis == 1 {
                    b.min.y > a.max.y
                } else {
                    b.min.z > a.max.z
                };

                if prune {
                    break;
                }

                // Kalan tüm eksenlerde kesin örtüşme (overlap) kontrolü yap
                if a.min.x <= b.max.x
                    && a.max.x >= b.min.x
                    && a.min.y <= b.max.y
                    && a.max.y >= b.min.y
                    && a.min.z <= b.max.z
                    && a.max.z >= b.min.z
                {
                    // Benzersiz Çifti ekle (Sürülen j daima i'den ileride olduğu için kopya çakışması asla olmaz)
                    let pair = if a.entity < b.entity {
                        (a.entity, b.entity)
                    } else {
                        (b.entity, a.entity)
                    };
                    collision_pairs.push(pair);
                }
            }
        }

        // Determinizm için pairleri Entity ID lerine göre mutlak düzene oturtalım
        collision_pairs.sort_unstable();

        // =========================================================================
        // 2. NARROW-PHASE: Sequential Impulse (SI) Çözücü + Rayon Paralelleştirme
        //    Mimari: Erin Catto (Box2D) SI + Paralel Algılama
        // =========================================================================

        // ---- FAZ 1a: PARALEL ÇARPIŞMA ALGILAMA ----
        // Her çarpışma çiftinin GJK/EPA hesabı bağımsızdır → çekirdekler arası dağıtılır.
        // Immutable (paylaşılan) referanslar thread-safe: &[T] ve &HashMap<K,V> → Sync ✓

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

        // Paralel algılama sonucu — her iş parçacığı kendi sonuçlarını üretir
        struct DetectionResult {
            contacts: Vec<StoredContact>,
            wake_entities: Vec<u32>,
        }

        // Paylaşılan immutable referanslar — SparseSet.dense (&[T]) ve SparseSet.sparse (&HashMap) Sync ✓
        let t_dense = &transforms.dense;
        let t_sparse = &transforms.sparse;
        let c_dense = &colliders.dense;
        let c_sparse = &colliders.sparse;
        let rb_dense = &rigidbodies.dense;
        let rb_sparse = &rigidbodies.sparse;
        let v_dense = &velocities.dense;
        let v_sparse = &velocities.sparse;

        // Vehicle entity ID'lerini thread-safe HashSet'e çıkar
        // Ref<SparseSet> = !Sync (Cell içerir), ama entity_dense (&[u32]) = Sync
        let vehicle_entities: std::collections::HashSet<u32> = match &vehicles {
            Some(v) => v.dense.iter().map(|e| e.entity).collect(),
            None => std::collections::HashSet::new(),
        };
        let has_vehicles = vehicles.is_some();
        let v_set = &vehicle_entities;

        use crate::shape::ColliderShape;
        use rayon::prelude::*;

        // Paralel algılama: her çarpışma çifti bağımsız iş parçacığında işlenir
        let detection_results: Vec<DetectionResult> = collision_pairs
            .par_iter()
            .filter_map(|&(ent_a, ent_b)| {
                // Bileşen lookup (hash tabanlı O(1) — immutable, thread-safe)
                let rb_a = rb_sparse.get(&ent_a).map(|&i| &rb_dense[i])?;
                let rb_b = rb_sparse.get(&ent_b).map(|&i| &rb_dense[i])?;

                if (rb_a.data.mass == 0.0 && rb_b.data.mass == 0.0)
                    || (rb_a.data.is_sleeping && rb_b.data.is_sleeping)
                {
                    return None;
                }

                if has_vehicles
                    && ((v_set.contains(&ent_a) && rb_b.data.mass == 0.0)
                        || (v_set.contains(&ent_b) && rb_a.data.mass == 0.0))
                {
                    return None;
                }

                let col_a = c_sparse.get(&ent_a).map(|&i| &c_dense[i])?;
                let col_b = c_sparse.get(&ent_b).map(|&i| &c_dense[i])?;
                let t_a = t_sparse.get(&ent_a).map(|&i| &t_dense[i])?;
                let t_b = t_sparse.get(&ent_b).map(|&i| &t_dense[i])?;
                let (pos_a, rot_a) = (t_a.data.position, t_a.data.rotation);
                let (pos_b, rot_b) = (t_b.data.position, t_b.data.rotation);

                // Çarpışma algılama (analitik veya GJK/EPA) — saf hesaplama, yan etkisiz
                let mut is_fast_path = false;
                let mut ccd_pos_a = None;
                let mut ccd_pos_b = None;
                let mut manifold = crate::collision::CollisionManifold {
                    is_colliding: false,
                    normal: Vec3::ZERO,
                    penetration: 0.0,
                    contact_points: vec![],
                };

                // Rotasyon kontrolü: AABB fast-path'leri sadece rotasyonsuz kutular için güvenli
                let is_rot_a_identity =
                    rot_a.x.abs() < 0.001 && rot_a.y.abs() < 0.001 && rot_a.z.abs() < 0.001;
                let is_rot_b_identity =
                    rot_b.x.abs() < 0.001 && rot_b.y.abs() < 0.001 && rot_b.z.abs() < 0.001;

                // === BUG #3 FIX: AABB-AABB analitik fast-path + SAT OBB Geçişi ===
                if let (ColliderShape::Aabb(a1), ColliderShape::Aabb(a2)) =
                    (&col_a.data.shape, &col_b.data.shape)
                {
                    is_fast_path = true;
                    if is_rot_a_identity && is_rot_b_identity {
                        manifold = crate::collision::check_aabb_aabb_manifold(pos_a, a1, pos_b, a2);
                    } else {
                        // Herhangi biri rotasyonluysa Analitik OBB GJK'in yerini alıyor
                        manifold = crate::collision::check_obb_obb_manifold(
                            pos_a, rot_a, a1, pos_b, rot_b, a2,
                        );
                    }
                }

                // === BUG #4 FIX: Sphere-AABB rotasyon kontrolü ===
                if !is_fast_path {
                    if let (ColliderShape::Sphere(s), ColliderShape::Aabb(a)) =
                        (&col_a.data.shape, &col_b.data.shape)
                    {
                        is_fast_path = true;
                        if is_rot_b_identity {
                            manifold =
                                crate::collision::check_sphere_aabb_manifold(pos_a, s, pos_b, a);
                        } else {
                            manifold = crate::collision::check_sphere_obb_manifold(
                                pos_a, s, pos_b, rot_b, a,
                            );
                        }
                    } else if let (ColliderShape::Aabb(a), ColliderShape::Sphere(s)) =
                        (&col_a.data.shape, &col_b.data.shape)
                    {
                        is_fast_path = true;
                        if is_rot_a_identity {
                            manifold =
                                crate::collision::check_sphere_aabb_manifold(pos_b, s, pos_a, a);
                        } else {
                            manifold = crate::collision::check_sphere_obb_manifold(
                                pos_b, s, pos_a, rot_a, a,
                            );
                        }
                        manifold.normal = -manifold.normal; // A→B yönü
                    } else if let (ColliderShape::Capsule(c1), ColliderShape::Capsule(c2)) =
                        (&col_a.data.shape, &col_b.data.shape)
                    {
                        is_fast_path = true;
                        manifold = crate::collision::check_capsule_capsule_manifold(
                            pos_a, rot_a, c1, pos_b, rot_b, c2,
                        );
                    } else if let (ColliderShape::Capsule(c), ColliderShape::Sphere(s)) =
                        (&col_a.data.shape, &col_b.data.shape)
                    {
                        is_fast_path = true;
                        manifold = crate::collision::check_capsule_sphere_manifold(
                            pos_a, rot_a, c, pos_b, s,
                        );
                    } else if let (ColliderShape::Sphere(s), ColliderShape::Capsule(c)) =
                        (&col_a.data.shape, &col_b.data.shape)
                    {
                        is_fast_path = true;
                        manifold = crate::collision::check_capsule_sphere_manifold(
                            pos_b, rot_b, c, pos_a, s,
                        );
                        manifold.normal *= -1.0;
                    } else if let (ColliderShape::Capsule(c), ColliderShape::Aabb(a)) =
                        (&col_a.data.shape, &col_b.data.shape)
                    {
                        is_fast_path = true;
                        manifold = crate::collision::check_capsule_aabb_manifold(
                            pos_a, rot_a, c, pos_b, a,
                        );
                    } else if let (ColliderShape::Aabb(a), ColliderShape::Capsule(c)) =
                        (&col_a.data.shape, &col_b.data.shape)
                    {
                        is_fast_path = true;
                        manifold = crate::collision::check_capsule_aabb_manifold(
                            pos_b, rot_b, c, pos_a, a,
                        );
                        manifold.normal *= -1.0;
                    } else if let (ColliderShape::Sphere(s1), ColliderShape::Sphere(s2)) =
                        (&col_a.data.shape, &col_b.data.shape)
                    {
                        is_fast_path = true;
                        manifold =
                            crate::collision::check_sphere_sphere_manifold(pos_a, s1, pos_b, s2);
                    }
                } // if !is_fast_path (Bug #4 rotasyon guard kapanışı)

                if !is_fast_path {
                    let (is_colliding, simplex) = crate::gjk::gjk_intersect(
                        &col_a.data.shape,
                        pos_a,
                        rot_a,
                        &col_b.data.shape,
                        pos_b,
                        rot_b,
                    );
                    if is_colliding {
                        manifold = crate::epa::epa_solve(
                            simplex,
                            &col_a.data.shape,
                            pos_a,
                            rot_a,
                            &col_b.data.shape,
                            pos_b,
                            rot_b,
                        );
                    }
                }

                // --- CCD Bisection (Sürekli Çarpışma Tespiti) ---
                // t=0'da kesişme yoksa fakat hızlıysak TOI (Time of Impact) ara.
                if !manifold.is_colliding && (rb_a.data.ccd_enabled || rb_b.data.ccd_enabled) {
                    let v_a_lin = v_sparse
                        .get(&ent_a)
                        .map(|&i| v_dense[i].data.linear)
                        .unwrap_or(Vec3::ZERO);
                    let v_b_lin = v_sparse
                        .get(&ent_b)
                        .map(|&i| v_dense[i].data.linear)
                        .unwrap_or(Vec3::ZERO);
                    let rel_v = v_b_lin - v_a_lin;

                    // Sadece göreli hız bu frame içinde anlamlı mesafe kat ediyorsa testi yap
                    if rel_v.length() * dt > 0.1 {
                        // Ön test: tüm [0, dt] aralığında hiç çakışma var mı?
                        // B'yi dt boyunca sweep et, A sabittir.
                        let swept_b_full = crate::shape::ColliderShape::Swept {
                            base: Box::new(col_b.data.shape.clone()),
                            sweep_vector: rel_v * dt,
                        };
                        let (hit_any, _) = crate::gjk::gjk_intersect(
                            &col_a.data.shape,
                            pos_a,
                            rot_a,
                            &swept_b_full,
                            pos_b,
                            rot_b,
                        );

                        if hit_any {
                            // Bisection — her adımda MUTLAK pozisyonları kullan, birikmeli sweep değil.
                            // Böylece t_low güncellendikçe sweep vektörü sıfırdan yanlış hesaplanmaz.
                            let mut t_low = 0.0_f32;
                            let mut t_high = dt;

                            for _ in 0..16 {
                                let t_mid = (t_low + t_high) * 0.5;

                                // [t_low, t_mid] aralığında çarpışma: B'yi t_low'dan t_mid'e sweep et
                                let pa_low = pos_a + v_a_lin * t_low;
                                let pb_low = pos_b + v_b_lin * t_low;
                                let sweep_half = crate::shape::ColliderShape::Swept {
                                    base: Box::new(col_b.data.shape.clone()),
                                    // Doğru sweep: [t_low, t_mid] mesafesi, t_mid - t_low (MUTLAK aralık)
                                    sweep_vector: (v_b_lin - v_a_lin) * (t_mid - t_low),
                                };
                                let (hit_first, _) = crate::gjk::gjk_intersect(
                                    &col_a.data.shape,
                                    pa_low,
                                    rot_a,
                                    &sweep_half,
                                    pb_low,
                                    rot_b,
                                );

                                if hit_first {
                                    t_high = t_mid;
                                } else {
                                    t_low = t_mid;
                                }
                            }

                            // t_high = TOI. Mikro epsilon ile çakışmayı garantile.
                            let t_hit = (t_high + dt * 0.001).min(dt);
                            let pa_hit = pos_a + v_a_lin * t_hit;
                            let pb_hit = pos_b + v_b_lin * t_hit;

                            let (hit, sim) = crate::gjk::gjk_intersect(
                                &col_a.data.shape,
                                pa_hit,
                                rot_a,
                                &col_b.data.shape,
                                pb_hit,
                                rot_b,
                            );
                            if hit {
                                manifold = crate::epa::epa_solve(
                                    sim,
                                    &col_a.data.shape,
                                    pa_hit,
                                    rot_a,
                                    &col_b.data.shape,
                                    pb_hit,
                                    rot_b,
                                );
                                if manifold.is_colliding {
                                    // Kalan süre boyunca penetrasyonu yapay olarak artır (tünellemeyi önle)
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

                                    ccd_pos_a = Some(pa_hit - pos_a);
                                    ccd_pos_b = Some(pb_hit - pos_b);
                                }
                            }
                        }
                    }
                }

                if !manifold.is_colliding || manifold.contact_points.is_empty() {
                    return None;
                }

                // Sonuç yapıları oluştur (heap alloc yok — tüm veriler stack'te)
                let inv_mass_a = if rb_a.data.mass == 0.0 {
                    0.0
                } else {
                    1.0 / rb_a.data.mass
                };
                let inv_mass_b = if rb_b.data.mass == 0.0 {
                    0.0
                } else {
                    1.0 / rb_b.data.mass
                };

                // SADECE ZATEN UYUYAN OBJELERİ UYANDIR!
                // Eğer iki obje de zaten uyanıksa birbirlerini dürtüp sleep_timer'larını SIFIRLAMAMALILAR.
                // Yoksa üst üste binen objeler sonsuza dek titreşir ve asla uyumazlar.
                let mut wakes = Vec::new();
                if rb_a.data.is_sleeping {
                    wakes.push(ent_a);
                }
                if rb_b.data.is_sleeping {
                    wakes.push(ent_b);
                }

                let mut result = DetectionResult {
                    contacts: Vec::new(),
                    wake_entities: wakes,
                };

                // Temas noktaları
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
                        ent_a,
                        ent_b,
                        normal: manifold.normal,
                        inv_mass_a,
                        inv_mass_b,
                        inv_inertia_a: rb_a.data.inverse_inertia,
                        inv_inertia_b: rb_b.data.inverse_inertia,
                        restitution: rb_a.data.restitution.max(rb_b.data.restitution),
                        friction: (rb_a.data.friction * rb_b.data.friction).sqrt(), // SAF COULOMB KARIŞIMI (Toplama yerine karekök çarpım)
                        penetration: *pen,
                        r_a,
                        r_b,
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
            .collect();

        // ---- FAZ 1b: ISLAND GENERATION (Union-Find — Path-Halving + Union-by-Rank) ----
        //
        // parent_map[i] = i'nin parent node'u (başta her node kendi root'u)
        // rank_map[i]   = alt ağaç derinliği tahmini (union-by-rank için)
        //
        // find_root : iterative path-halving → O(α(N)) amortize, sonsuz döngü riski yok
        // union_nodes: rank küçük olan rank büyüğe bağlanır → dengeli ağaç, O(log N) derinlik

        let mut parent_map: std::collections::BTreeMap<u32, u32> =
            std::collections::BTreeMap::new();
        let mut rank_map: std::collections::BTreeMap<u32, u8> = std::collections::BTreeMap::new();

        // Node'u haritaya ekler (idempotent — zaten varsa değişmez).
        fn ensure_node(
            parent: &mut std::collections::BTreeMap<u32, u32>,
            rank: &mut std::collections::BTreeMap<u32, u8>,
            i: u32,
        ) {
            parent.entry(i).or_insert(i);
            rank.entry(i).or_insert(0);
        }

        // Kökü döndürür — her adımda i'yi büyük-ebeveynine (grandparent) bağlar (path-halving).
        // Tek pass'te hem arama hem sıkıştırma yapılır; entry+get çift erişim sorununu ortadan kaldırır.
        fn find_root(parent: &mut std::collections::BTreeMap<u32, u32>, mut i: u32) -> u32 {
            loop {
                // parent yoksa node kendi kendinin root'u demektir
                let p = match parent.get(&i) {
                    Some(&p) => p,
                    None => return i,
                };
                if p == i {
                    return i;
                }
                // grandparent'ı al (yoksa p'nin kendisi)
                let gp = match parent.get(&p) {
                    Some(&gp) => gp,
                    None => p,
                };
                // path-halving: i'yi grandparent'a bağla
                parent.insert(i, gp);
                i = gp;
            }
        }

        // İki island'ı birleştirir; rank'ı düşük olan, yüksek olanın altına girer.
        fn union_nodes(
            parent: &mut std::collections::BTreeMap<u32, u32>,
            rank: &mut std::collections::BTreeMap<u32, u8>,
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

        struct Island {
            contacts: Vec<StoredContact>,
            velocities: std::collections::HashMap<u32, Velocity>,
            poses: std::collections::HashMap<u32, Transform>,
        }

        let mut all_contacts = Vec::new();
        for result in detection_results {
            entities_to_wake.extend(result.wake_entities);
            for c in result.contacts {
                let a_dyn = c.inv_mass_a > 0.0;
                let b_dyn = c.inv_mass_b > 0.0;
                if a_dyn && b_dyn {
                    // Her iki uç da dinamik → iki node'u kaydet ve birleştir
                    ensure_node(&mut parent_map, &mut rank_map, c.ent_a);
                    ensure_node(&mut parent_map, &mut rank_map, c.ent_b);
                    union_nodes(&mut parent_map, &mut rank_map, c.ent_a, c.ent_b);
                } else if a_dyn {
                    // Yalnız dinamik node → kendi island key'i oluşsun diye kayıt
                    ensure_node(&mut parent_map, &mut rank_map, c.ent_a);
                } else if b_dyn {
                    ensure_node(&mut parent_map, &mut rank_map, c.ent_b);
                }
                all_contacts.push(c);
            }
        }

        let mut islands_map: std::collections::HashMap<u32, Island> =
            std::collections::HashMap::new();
        for c in all_contacts {
            let a_dyn = c.inv_mass_a > 0.0;
            let root = if a_dyn {
                find_root(&mut parent_map, c.ent_a)
            } else {
                find_root(&mut parent_map, c.ent_b)
            };
            let island = islands_map.entry(root).or_insert_with(|| Island {
                contacts: Vec::new(),
                velocities: std::collections::HashMap::new(),
                poses: std::collections::HashMap::new(),
            });
            island.contacts.push(c);
        }

        for island in islands_map.values_mut() {
            for c in &island.contacts {
                if c.inv_mass_a > 0.0 && !island.velocities.contains_key(&c.ent_a) {
                    island.velocities.insert(
                        c.ent_a,
                        velocities
                            .get(c.ent_a)
                            .cloned()
                            .unwrap_or(Velocity::new(Vec3::ZERO)),
                    );
                    let mut p = *transforms.get(c.ent_a).unwrap();
                    p.position += c.ccd_offset_a;
                    island.poses.insert(c.ent_a, p);
                }
                if c.inv_mass_b > 0.0 && !island.velocities.contains_key(&c.ent_b) {
                    island.velocities.insert(
                        c.ent_b,
                        velocities
                            .get(c.ent_b)
                            .cloned()
                            .unwrap_or(Velocity::new(Vec3::ZERO)),
                    );
                    let mut p = *transforms.get(c.ent_b).unwrap();
                    p.position += c.ccd_offset_b;
                    island.poses.insert(c.ent_b, p);
                }
            }
        }

        let mut islands: Vec<Island> = islands_map.into_values().collect();

        // === FAZ 1: WARM STARTING — Contact Point Matching ile Güvenli Önbellek İtmesi ===
        // Önceki karenin impuls sonuçlarını, temas noktası konumsal eşlemesi (2cm threshold)
        // ile yeni karenin başlangıç değerleri olarak kullan. %80 sönümleme uygulanır.
        let (solver_iters, frame_count) =
            if let Some(mut state) = world.get_resource_mut::<PhysicsSolverState>() {
                // Önceki frame'in cache'inden eşleşen temas noktalarının impulslarını aktar
                for island in islands.iter_mut() {
                    for c in island.contacts.iter_mut() {
                        let key = if c.ent_a < c.ent_b {
                            (c.ent_a, c.ent_b)
                        } else {
                            (c.ent_b, c.ent_a)
                        };
                        if let Some(cached_contacts) = state.contact_cache.get(&key) {
                            if let Some((cached_j, cached_friction)) =
                                match_cached_contact(c.world_point, cached_contacts)
                            {
                                // Güvenli warm-start: %80 sönümleme, max 20.0 impuls limiti
                                c.accumulated_j = (cached_j * WARM_START_FACTOR).min(20.0);
                                c.accumulated_friction = cached_friction * WARM_START_FACTOR;
                            }
                        }
                    }
                }

                // Warm-start impulslarını hızlara uygula (solver öncesi başlangıç noktası)
                for island in islands.iter_mut() {
                    for c in island.contacts.iter() {
                        if c.accumulated_j > 1e-6 {
                            let impulse = c.normal * c.accumulated_j;
                            if let Some(v_a) = island.velocities.get_mut(&c.ent_a) {
                                v_a.linear -= impulse * c.inv_mass_a;
                                let t = c.r_a.cross(impulse * -1.0);
                                v_a.angular += apply_inv_inertia(t, c.inv_inertia_a, c.rot_a);
                            }
                            if let Some(v_b) = island.velocities.get_mut(&c.ent_b) {
                                v_b.linear += impulse * c.inv_mass_b;
                                let t = c.r_b.cross(impulse);
                                v_b.angular += apply_inv_inertia(t, c.inv_inertia_b, c.rot_b);
                            }
                        }
                        // Sürtünme warm-start
                        let fi = c.accumulated_friction;
                        if fi.length_squared() > 1e-12 {
                            if let Some(v) = island.velocities.get_mut(&c.ent_a) {
                                v.linear -= fi * c.inv_mass_a;
                                let ft = c.r_a.cross(fi * -1.0);
                                v.angular += apply_inv_inertia(ft, c.inv_inertia_a, c.rot_a);
                            }
                            if let Some(v) = island.velocities.get_mut(&c.ent_b) {
                                v.linear += fi * c.inv_mass_b;
                                let ft = c.r_b.cross(fi);
                                v.angular += apply_inv_inertia(ft, c.inv_inertia_b, c.rot_b);
                            }
                        }
                    }
                }

                state.frame_counter += 1;
                (state.solver_iterations, state.frame_counter)
            } else {
                (8, 0)
            };

        const MAX_ANG: f32 = 100.0;
        const MAX_LIN: f32 = 200.0;

        // ---- FAZ 2: PARALEL ADA ÇÖZÜMÜ ----
        islands.par_iter_mut().for_each(|island| {
            // Çözüm bias'ını engellemek için frame-seeded pseudo-random contact shuffle:
            let contacts_len = island.contacts.len();
            if contacts_len > 1 {
                let seed = frame_count as usize;
                for i in 0..(contacts_len - 1) {
                    let swap_idx = (i * 37 + 11 + seed) % contacts_len;
                    island.contacts.swap(i, swap_idx);
                }
            }

            for _iter in 0..solver_iters {
                for c in island.contacts.iter_mut() {
                    let va = island
                        .velocities
                        .get(&c.ent_a)
                        .cloned()
                        .unwrap_or(Velocity::new(Vec3::ZERO));
                    let vb = island
                        .velocities
                        .get(&c.ent_b)
                        .cloned()
                        .unwrap_or(Velocity::new(Vec3::ZERO));

                    let vpa = va.linear + va.angular.cross(c.r_a);
                    let vpb = vb.linear + vb.angular.cross(c.r_b);
                    let rel = vpb - vpa;
                    let vn = rel.dot(c.normal);

                    let mut e = c.restitution;
                    if vn.abs() < 0.2 {
                        e = 0.0;
                    }

                    let ra_x_n = c.r_a.cross(c.normal);
                    let rb_x_n = c.r_b.cross(c.normal);
                    let it_a = apply_inv_inertia(ra_x_n, c.inv_inertia_a, c.rot_a);
                    let it_b = apply_inv_inertia(rb_x_n, c.inv_inertia_b, c.rot_b);
                    let ang_a = it_a.cross(c.r_a).dot(c.normal);
                    let ang_b = it_b.cross(c.r_b).dot(c.normal);
                    let eff_mass = c.inv_mass_a + c.inv_mass_b + ang_a + ang_b;
                    if eff_mass == 0.0 {
                        continue;
                    }

                    // BAUMGARTE STABİLİZASYONU (İç içe geçmeleri yaylandırarak çözer, zemin fırlatmalarını engeller)
                    let beta = 0.2; // Düzeltme şiddeti
                    let slop = 0.005; // Titreme payı (Micro-jitter)
                                      // Baumgarte bias'ına üst limit koyuyoruz (frame spike'larında orantısız patlamaları önlemek için)
                    let max_bias = 20.0;
                    let bias = ((beta / dt) * (c.penetration - slop).max(0.0)).min(max_bias);

                    let j_new = (-(1.0 + e) * vn + bias) / eff_mass;
                    let old_acc = c.accumulated_j;
                    c.accumulated_j = (c.accumulated_j + j_new).max(0.0);
                    let j = c.accumulated_j - old_acc;

                    if j.abs() > 1e-8 {
                        let impulse = c.normal * j;

                        if let Some(v_a) = island.velocities.get_mut(&c.ent_a) {
                            v_a.linear -= impulse * c.inv_mass_a;
                            v_a.linear.x = v_a.linear.x.clamp(-MAX_LIN, MAX_LIN);
                            v_a.linear.y = v_a.linear.y.clamp(-MAX_LIN, MAX_LIN);
                            v_a.linear.z = v_a.linear.z.clamp(-MAX_LIN, MAX_LIN);
                            let t = c.r_a.cross(impulse * -1.0);
                            v_a.angular += apply_inv_inertia(t, c.inv_inertia_a, c.rot_a);

                            if v_a.angular.length() > 50.0 {
                                // TAKLA ATIYOR printi, saniyede yüzlerce I/O yaratarak oyuna feci bir Frame Drop/Stutter yaptırdığı için kapatıldı.
                            }

                            v_a.angular.x = v_a.angular.x.clamp(-MAX_ANG, MAX_ANG);
                            v_a.angular.y = v_a.angular.y.clamp(-MAX_ANG, MAX_ANG);
                            v_a.angular.z = v_a.angular.z.clamp(-MAX_ANG, MAX_ANG);
                        }
                        if let Some(v_b) = island.velocities.get_mut(&c.ent_b) {
                            v_b.linear += impulse * c.inv_mass_b;
                            v_b.linear.x = v_b.linear.x.clamp(-MAX_LIN, MAX_LIN);
                            v_b.linear.y = v_b.linear.y.clamp(-MAX_LIN, MAX_LIN);
                            v_b.linear.z = v_b.linear.z.clamp(-MAX_LIN, MAX_LIN);
                            let t = c.r_b.cross(impulse);
                            v_b.angular += apply_inv_inertia(t, c.inv_inertia_b, c.rot_b);
                            v_b.angular.x = v_b.angular.x.clamp(-MAX_ANG, MAX_ANG);
                            v_b.angular.y = v_b.angular.y.clamp(-MAX_ANG, MAX_ANG);
                            v_b.angular.z = v_b.angular.z.clamp(-MAX_ANG, MAX_ANG);
                        }
                    }

                    // === COULOMB SÜRTÜNME ===
                    let va2 = island
                        .velocities
                        .get(&c.ent_a)
                        .cloned()
                        .unwrap_or(Velocity::new(Vec3::ZERO));
                    let vb2 = island
                        .velocities
                        .get(&c.ent_b)
                        .cloned()
                        .unwrap_or(Velocity::new(Vec3::ZERO));
                    let rel2 = (vb2.linear + vb2.angular.cross(c.r_b))
                        - (va2.linear + va2.angular.cross(c.r_a));
                    let tangent_vel = rel2 - c.normal * rel2.dot(c.normal);
                    let ts = tangent_vel.length();

                    if ts > 0.001 {
                        let tangent_dir = tangent_vel / ts;

                        let ra_cross_t = c.r_a.cross(tangent_dir);
                        let rb_cross_t = c.r_b.cross(tangent_dir);
                        let ita = apply_inv_inertia(ra_cross_t, c.inv_inertia_a, c.rot_a);
                        let itb = apply_inv_inertia(rb_cross_t, c.inv_inertia_b, c.rot_b);

                        let tangent_eff_mass = c.inv_mass_a
                            + c.inv_mass_b
                            + ita.cross(c.r_a).dot(tangent_dir)
                            + itb.cross(c.r_b).dot(tangent_dir);

                        if tangent_eff_mass > 0.0 {
                            let jt = -ts / tangent_eff_mass;
                            let mu_s = c.friction;
                            let mu_k = c.friction * 0.7; // Kinetik sürtünme her zaman statiğin biraz altındadır
                            let max_friction = c.accumulated_j * mu_s;
                            let old_friction = c.accumulated_friction;

                            let mut new_friction = old_friction + tangent_dir * jt;
                            let friction_len = new_friction.length();

                            // STATİK / KİNETİK SÜRTÜNME YASASI
                            if friction_len > max_friction {
                                // Statik limit aşıldı! Eşyalar sarsılmadan kaymalı (Kinetik moda geçer)
                                let kinetic_limit = c.accumulated_j * mu_k;
                                new_friction *= kinetic_limit / friction_len;
                            }

                            let fi = new_friction - old_friction;
                            c.accumulated_friction = new_friction;

                            if let Some(v) = island.velocities.get_mut(&c.ent_a) {
                                v.linear -= fi * c.inv_mass_a;
                                let ft = c.r_a.cross(fi * -1.0);
                                v.angular += apply_inv_inertia(ft, c.inv_inertia_a, c.rot_a);
                                v.angular.x = v.angular.x.clamp(-MAX_ANG, MAX_ANG);
                                v.angular.y = v.angular.y.clamp(-MAX_ANG, MAX_ANG);
                                v.angular.z = v.angular.z.clamp(-MAX_ANG, MAX_ANG);
                            }
                            if let Some(v) = island.velocities.get_mut(&c.ent_b) {
                                v.linear += fi * c.inv_mass_b;
                                let ft = c.r_b.cross(fi);
                                v.angular += apply_inv_inertia(ft, c.inv_inertia_b, c.rot_b);
                                v.angular.x = v.angular.x.clamp(-MAX_ANG, MAX_ANG);
                                v.angular.y = v.angular.y.clamp(-MAX_ANG, MAX_ANG);
                                v.angular.z = v.angular.z.clamp(-MAX_ANG, MAX_ANG);
                            }
                        }
                    }
                }
            }

            // === BUG #2 FIX: DOĞRUDAN POZİSYON DÜZELTMESİ (Position Projection) ===
            // SI çözücü hız tabanlı düzeltme yapar, ama damping onu zayıflatabilir.
            // Bu adım nesneleri doğrudan penetrasyon derinliği kadar ayırır.
            for c in &island.contacts {
                let slop = 0.005;
                let correction_factor = 0.4; // %40 anında düzelt (çok agresif olursa titreşim yapar)
                let correction = (c.penetration - slop).max(0.0) * correction_factor;

                if correction > 0.0 {
                    let total_inv = c.inv_mass_a + c.inv_mass_b;
                    if total_inv > 0.0 {
                        let push = c.normal * (correction / total_inv);
                        if let Some(p) = island.poses.get_mut(&c.ent_a) {
                            p.position -= push * c.inv_mass_a;
                        }
                        if let Some(p) = island.poses.get_mut(&c.ent_b) {
                            p.position += push * c.inv_mass_b;
                        }
                    }
                }
            }
        });

        // Yazımları ana array'e geri aktar (Sync phase)
        let mut sync_cache = Vec::new();
        for island in islands {
            for (ent, vel) in &island.velocities {
                // VehicleController entity'lerinin velocity'sini SI solver üzerine yazmasın —
                // bu entity'lerin fizik kuvvetleri physics_vehicle_system tarafından yönetilir.
                if vehicle_entities.contains(ent) {
                    continue;
                }
                if let Some(v) = velocities.get_mut(*ent) {
                    *v = *vel;
                }
            }
            for (ent, tbox) in &island.poses {
                if let Some(t) = transforms.get_mut(*ent) {
                    *t = *tbox;
                    t.update_local_matrix();
                }
            }
            for c in island.contacts {
                // Temas noktası dünya koordinatını hesapla (warm-start cache için)
                let wp = island
                    .poses
                    .get(&c.ent_a)
                    .map(|p| p.position + c.r_a)
                    .unwrap_or(c.world_point);
                sync_cache.push((
                    c.ent_a,
                    c.ent_b,
                    c.accumulated_j,
                    c.accumulated_friction,
                    wp,
                ));

                // DARBE/MOMENTUM EVENT'İ FIRLAT: Kütle-normalize edilmiş eşik (adaptif)
                let eff_mass = 1.0 / (c.inv_mass_a + c.inv_mass_b).max(0.0001);
                let threshold = (0.05 * eff_mass) + 0.01;
                if c.accumulated_j > threshold {
                    let pos_a = island
                        .poses
                        .get(&c.ent_a)
                        .map(|t| t.position)
                        .unwrap_or(Vec3::ZERO);
                    collision_events.push(crate::CollisionEvent {
                        entity_a: c.ent_a,
                        entity_b: c.ent_b,
                        position: pos_a + c.r_a,
                        normal: c.normal,
                        impulse: c.accumulated_j,
                    });
                }
            }
        }

        // === FAZ 3: WARM STARTING CACHE KAYDI ===
        // Contact point matching sayesinde güvenle etkinleştirildi.
        // Her temas noktasının dünya koordinatı ve birikmiş impulsu bir sonraki frame için saklanır.
        if let Some(mut state) = world.get_resource_mut::<PhysicsSolverState>() {
            state.contact_cache.clear();
            for (ent_a, ent_b, acc_j, acc_friction, world_point) in &sync_cache {
                // Cache key'i her zaman küçük entity ID önce olacak şekilde normalize et
                let key = if ent_a < ent_b {
                    (*ent_a, *ent_b)
                } else {
                    (*ent_b, *ent_a)
                };
                let entry = state.contact_cache.entry(key).or_insert_with(Vec::new);
                // Entity çifti başına max 4 temas noktası (bellek koruması)
                if entry.len() < 4 {
                    entry.push(CachedContact {
                        world_point: *world_point,
                        accumulated_normal: *acc_j,
                        accumulated_friction: *acc_friction,
                    });
                }
            }
        }
    } // --- Borrow Scope Sonu (transforms, velocities, colliders, rigidbodies drop ediliyor) ---

    if !collision_events.is_empty() {
        // Artık event kuyruğu yoksa "sessizce düşürmek" yerine `_or_default` ile anında oluşturuyoruz!
        // World üzerindeki tüm component borrow'ları bittiği için &mut world kullanabiliriz.
        let mut evs =
            world.get_resource_mut_or_default::<gizmo_core::event::Events<crate::CollisionEvent>>();
        for ev in collision_events {
            evs.push(ev);
        }
    }

    // Uyuyan ve dokunulan objeleri UYANDIR!
    // Tüm immutable borrow'lar scope dışına çıktı, güvenle borrow_mut yapabiliriz
    if !entities_to_wake.is_empty() {
        if let Some(mut rbs) = world.borrow_mut::<RigidBody>() {
            for e in entities_to_wake {
                if let Some(rb) = rbs.get_mut(e) {
                    rb.wake_up();
                }
            }
        }
    }
} // closes physics_collision_system
