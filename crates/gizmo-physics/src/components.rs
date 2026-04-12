use gizmo_math::{Mat4, Quat, Vec3};

fn default_mat4() -> Mat4 {
    Mat4::IDENTITY
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Transform {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
    // Eklediğimiz Global Matrix. ECS update sisteminde hepsi traverse edilip güncellenir.
    #[serde(skip, default = "default_mat4")]
    pub global_matrix: Mat4,
}

impl Transform {
    pub fn new(position: Vec3) -> Self {
        let mut t = Self {
            position,
            rotation: Quat::IDENTITY,
            scale: Vec3::new(1.0, 1.0, 1.0),
            global_matrix: Mat4::IDENTITY,
        };
        t.update_local_matrix();
        t
    }

    pub fn with_scale(mut self, scale: Vec3) -> Self {
        self.scale = scale;
        self.update_local_matrix();
        self
    }

    pub fn with_rotation(mut self, rotation: Quat) -> Self {
        self.rotation = rotation;
        self.update_local_matrix();
        self
    }

    /// Update metodu, sistemden önce başlangıç değerleri için kullanılabilir
    pub fn update_local_matrix(&mut self) {
        self.global_matrix =
            Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.position);
    }

    /// Geriye dönük uyumluluk veya anlık model matrisi hesaplaması
    pub fn local_matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.position)
    }

    pub fn model_matrix(&self) -> Mat4 {
        self.global_matrix
    }
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Velocity {
    pub linear: Vec3,
    pub angular: Vec3, // Açısal hız (Radyan/s)
}

impl Velocity {
    pub fn new(linear: Vec3) -> Self {
        Self {
            linear,
            angular: Vec3::ZERO,
        }
    }
}

// Fiziksel ağırlık ve dış güçlerin nasıl etki edeceğini belirten kütle özellikleri
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RigidBody {
    pub mass: f32, // Eğer mass == 0.0 ise bu obje sabittir (Duvar/Zemin) ve itilemez!
    pub restitution: f32, // Sekme katsayısı (0.0 = sekmez, 1.0 = sonsuz teper)
    pub friction: f32, // Sürtünme katsayısı
    pub use_gravity: bool, // Yerçekiminden etkileniyor mu?

    // Eylemsizlik Temsili (Inertia Tensor) - objenin kendi ekseni etrafında dönmeye direncini temsil eder
    pub local_inertia: Vec3,
    pub inverse_inertia: Vec3,

    // Island Sleeping (Uyku Süreci) - Fix #12: Rolling Average ile stabil uyku
    pub is_sleeping: bool,
    #[serde(skip)]
    pub sleep_timer: f32,
    #[serde(skip)]
    pub avg_linear_sq: f32, 
    #[serde(skip)]
    pub avg_angular_sq: f32,
    
    /// Continuous Collision Detection aktif mi? (hızlı objeler için)
    #[serde(default)]
    pub ccd_enabled: bool,

    // ─── Çarpışma Katmanı (Collision Layer/Mask) — Fix #35 ───────────────
    //
    // Her bit bir katmanı temsil eder (0-31).
    // A ile B çarpışabilir ancak ve ancak:
    //   (a.collision_layer & b.collision_mask) != 0
    //   (b.collision_layer & a.collision_mask) != 0
    //
    // Varsayılan: layer=1, mask=0xFFFF_FFFF (tüm katmanlarla çarpışır)
    /// Bu objenin ait olduğu katman bitleri
    #[serde(default = "default_collision_layer")]
    pub collision_layer: u32,
    /// Bu objenin çarpışabileceği katman bitleri
    #[serde(default = "default_collision_mask")]
    pub collision_mask: u32,
}

fn default_collision_layer() -> u32 { 1 }
fn default_collision_mask()  -> u32 { 0xFFFF_FFFF }

impl RigidBody {
    /// Yeni rigid body oluştur. ⚠️ Varsayılan 1x1x1 küp eylemsizliği hesaplanır!
    /// Doğru eylemsizlik için oluşturduktan sonra `calculate_box_inertia()`,
    /// `calculate_sphere_inertia()` veya `calculate_capsule_inertia()` çağırılmalı.
    pub fn new(mass: f32, restitution: f32, friction: f32, use_gravity: bool) -> Self {
        let mut rb = Self {
            mass,
            restitution,
            friction,
            use_gravity,
            local_inertia: Vec3::new(1.0, 1.0, 1.0),
            inverse_inertia: Vec3::ZERO,
            is_sleeping: false,
            sleep_timer: 0.0,
            avg_linear_sq: 0.0,
            avg_angular_sq: 0.0,
            ccd_enabled: false,
            collision_layer: 1,
            collision_mask: 0xFFFF_FFFF,
        };
        if mass > 0.0 {
            rb.calculate_box_inertia(1.0, 1.0, 1.0);
        }
        rb
    }

    pub fn new_static() -> Self {
        Self {
            mass: 0.0,
            restitution: 0.0,
            friction: 1.0,
            use_gravity: false,
            local_inertia: Vec3::ZERO,
            inverse_inertia: Vec3::ZERO,
            is_sleeping: true,
            sleep_timer: 0.0,
            avg_linear_sq: 0.0,
            avg_angular_sq: 0.0,
            ccd_enabled: false,
            collision_layer: 1,
            collision_mask: 0xFFFF_FFFF,
        }
    }

    /// Objeyi uyandırır
    pub fn wake_up(&mut self) {
        self.is_sleeping = false;
        self.sleep_timer = 0.0;
    }

    /// Dinamik objenin Inverse Inertia Tensor'unu boyutlarına (AABB) göre baştan hesaplar
    pub fn calculate_box_inertia(&mut self, width: f32, height: f32, depth: f32) {
        if self.mass > 0.0 {
            self.local_inertia = Vec3::new(
                (1.0 / 12.0) * self.mass * (height * height + depth * depth),
                (1.0 / 12.0) * self.mass * (width * width + depth * depth),
                (1.0 / 12.0) * self.mass * (width * width + height * height),
            );
            self.inverse_inertia = Vec3::new(
                1.0 / self.local_inertia.x,
                1.0 / self.local_inertia.y,
                1.0 / self.local_inertia.z,
            );
        }
    }

    pub fn calculate_sphere_inertia(&mut self, radius: f32) {
        if self.mass <= 0.0 {
            return;
        }
        // I = 2/5 * m * r^2
        let inertia = (2.0 / 5.0) * self.mass * (radius * radius);
        self.local_inertia = Vec3::new(inertia, inertia, inertia);
        self.inverse_inertia = Vec3::new(1.0 / inertia, 1.0 / inertia, 1.0 / inertia);
    }

    /// Kapsül için eylemsizlik tensörü hesaplar (silindir + iki yarıküre)
    pub fn calculate_capsule_inertia(&mut self, radius: f32, half_height: f32) {
        if self.mass > 0.0 {
            let r2 = radius * radius;
            let h = half_height * 2.0; // Toplam silindir yüksekliği
                                       // Silindir kısmının eylemsizliği
            let cyl_mass = self.mass * h / (h + (4.0 / 3.0) * radius);
            let sphere_mass = self.mass - cyl_mass;

            // Y ekseni etrafında (uzun eksen)
            let iy = 0.5 * cyl_mass * r2 + (2.0 / 5.0) * sphere_mass * r2;
            // X ve Z ekseni etrafında
            let ix = cyl_mass * (3.0 * r2 + h * h) / 12.0
                + sphere_mass * (2.0 * r2 / 5.0 + h * h / 4.0 + 3.0 * h * radius / 8.0);

            self.local_inertia = Vec3::new(ix, iy, ix);
            self.inverse_inertia = Vec3::new(1.0 / ix, 1.0 / iy, 1.0 / ix);
        }
    }

    /// ColliderShape verisine bakarak eylemsizlik tensörünü otomatik hesaplar
    pub fn update_inertia_from_shape(&mut self, shape: &crate::shape::ColliderShape) {
        if self.mass <= 0.0 {
            return;
        }
        match shape {
            crate::shape::ColliderShape::Aabb(aabb) => {
                self.calculate_box_inertia(
                    aabb.half_extents.x * 2.0,
                    aabb.half_extents.y * 2.0,
                    aabb.half_extents.z * 2.0,
                );
            }
            crate::shape::ColliderShape::Sphere(sphere) => {
                self.calculate_sphere_inertia(sphere.radius);
            }
            crate::shape::ColliderShape::Capsule(capsule) => {
                self.calculate_capsule_inertia(capsule.radius, capsule.half_height);
            }
            crate::shape::ColliderShape::ConvexHull(_)
            | crate::shape::ColliderShape::HeightField { .. }
            | crate::shape::ColliderShape::Swept { .. } => {
                // Approximate fallback
                self.calculate_box_inertia(1.0, 1.0, 1.0);
            }
        }
    }
}

/// Global fizik konfigürasyonu — World resource olarak saklanır
#[derive(Debug, Clone, Copy)]
pub struct PhysicsConfig {
    /// Fallback zemin yüksekliği (collider yoksa) — varsayılan: -1.0
    pub ground_y: f32,
    /// Lineer hız üst limiti (m/s) — Fix #5. Varsayılan: 200.0
    pub max_linear_velocity: f32,
    /// Açısal hız üst limiti (rad/s) — Fix #5. Varsayılan: 100.0
    pub max_angular_velocity: f32,
    /// Çift başına maksimum warm-start cache girişi — Fix #6. Varsayılan: 4
    pub max_contact_points_per_pair: usize,
    /// Collision event'leri throttle: aynı çift için minimum frame aralığı
    /// 0 = devre dışı (tüm eventler fırlatılır). Fix #31. Varsayılan: 4
    pub collision_event_throttle_frames: u32,
}

impl Default for PhysicsConfig {
    fn default() -> Self {
        Self {
            ground_y:                       -1.0,
            max_linear_velocity:            200.0,
            max_angular_velocity:           100.0,
            max_contact_points_per_pair:    4,
            collision_event_throttle_frames: 4,
        }
    }
}
