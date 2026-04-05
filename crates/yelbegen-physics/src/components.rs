use yelbegen_math::{Vec3, Quat, Mat4};

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
        self.global_matrix = Mat4::translation(self.position) * self.rotation.to_mat4() * Mat4::scale(self.scale);
    }

    /// Geriye dönük uyumluluk veya anlık model matrisi hesaplaması
    pub fn local_matrix(&self) -> Mat4 {
        Mat4::translation(self.position) * self.rotation.to_mat4() * Mat4::scale(self.scale)
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
        Self { linear, angular: Vec3::ZERO }
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

    // Island Sleeping (Uyku Süreci)
    pub is_sleeping: bool,
    #[serde(skip)]
    pub sleep_timer: f32,
}

impl RigidBody {
    pub fn new(mass: f32, restitution: f32, friction: f32, use_gravity: bool) -> Self {
        // Varsayılan olarak 1x1x1 bir küp olduğunu varsayarak Inertia hesaplayalım
        // İleride şekle (Shape) göre dinamik hesaplanacak
        let mut local_inertia = Vec3::new(1.0, 1.0, 1.0);
        let mut inverse_inertia = Vec3::ZERO;

        if mass > 0.0 {
            let i = (1.0 / 12.0) * mass * (1.0 * 1.0 + 1.0 * 1.0); // Kutu eylemsizlik tahmini
            local_inertia = Vec3::new(i, i, i);
            inverse_inertia = Vec3::new(1.0 / i, 1.0 / i, 1.0 / i);
        }

        Self { 
            mass, 
            restitution, 
            friction, 
            use_gravity,
            local_inertia,
            inverse_inertia,
            is_sleeping: false,
            sleep_timer: 0.0,
        }
    }

    pub fn new_static() -> Self {
        Self { 
            mass: 0.0, 
            restitution: 0.0, 
            friction: 1.0, 
            use_gravity: false,
            local_inertia: Vec3::ZERO,
            inverse_inertia: Vec3::ZERO,
            is_sleeping: true, // Statik objeler hep uyur
            sleep_timer: 0.0,
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
}
