use std::collections::HashMap;
use gizmo_math::{Vec3, Mat3};
use crate::components::{Transform, Velocity};

// ─── Veri Yapıları ────────────────────────────────────────────────────────────

/// Kalıcı temas noktası — frame'ler arası eşleme için konum bilgisi taşır
#[derive(Clone, Debug)]
pub struct CachedContact {
    /// Dünya koordinatlarında temas noktası (eşleme anahtarı)
    pub world_point: Vec3,
    pub accumulated_normal: f32,
}

/// Broad-phase için entity'nin dünya-uzayı AABB sınırları
pub struct Interval {
    pub entity: u32,
    pub min: Vec3,
    pub max: Vec3,
}

/// Narrow-phase çözücüsünün bir adım için gereken tüm veriler (17 alan)
pub struct StoredContact {
    pub ent_a: u32,
    pub ent_b: u32,
    pub normal: Vec3,
    pub inv_mass_a: f32,
    pub inv_mass_b: f32,
    pub inv_inertia_a: Mat3,
    pub inv_inertia_b: Mat3,
    pub restitution: f32,
    pub friction: f32,
    pub penetration: f32,
    pub r_a: Vec3,
    pub r_b: Vec3,
    pub rot_a: gizmo_math::Quat,
    pub rot_b: gizmo_math::Quat,
    pub accumulated_j: f32,
    pub accumulated_friction: Vec3,
    pub ccd_offset_a: Vec3,
    pub ccd_offset_b: Vec3,
    /// Döngü öncesi hesaplanan restitution (sekme) hedef hızı
    pub bias_bounce: f32,
    /// Dünya koordinatlarındaki temas noktası (warm-start eşleme için)
    pub world_point: Vec3,
    pub remaining_time: f32,
}

/// Paralel algılama adımının tek-çiftten dönen sonucu
pub struct DetectionResult {
    pub contacts: Vec<StoredContact>,
    pub wake_entities: Vec<u32>,
}

/// Birbirine temas eden dinamik entity'lerin grubu
pub struct Island {
    pub joints: Vec<(usize, crate::constraints::Joint, crate::constraints::JointBodies)>,
    pub contacts: Vec<StoredContact>,
    pub velocities: HashMap<u32, Velocity>,
    pub poses: HashMap<u32, Transform>,
}

// ─── Çözücü Durumu ───────────────────────────────────────────────────────────

/// Contact Point Matching eşik değeri (2cm yarıçap)
pub const MATCH_THRESHOLD_SQ: f32 = 0.1 * 0.1;

/// Warm-start sönümleme faktörü (%80 — patlama riskini azaltır)
pub const WARM_START_FACTOR: f32 = 0.4;  // 0.8 çok agresif → yapışma, 0.4 = dengeli. Newton sarkacı krizi (bias_bounce sırası) çözüldüğü için artık güvenle aktif!

/// Kalıcı Çözücü Durumu (Warm-Starting Cache için)
pub struct PhysicsSolverState {
    /// Önceki karedeki temas noktalarının entity-çifti bazlı cache'i
    pub contact_cache: HashMap<(u32, u32), Vec<CachedContact>>,
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
            frame_counter: 0,
        }
    }
}

#[inline]
pub fn is_near_identity(q: gizmo_math::Quat) -> bool {
    q.x.abs() < 1e-4 && q.y.abs() < 1e-4 && q.z.abs() < 1e-4
}

