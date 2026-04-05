use std::sync::Arc;
use yelbegen_math::vec3::Vec3;

#[derive(Clone)]
pub struct Mesh {
    pub vbuf: Arc<wgpu::Buffer>,
    pub vertex_count: u32,
    pub center_offset: Vec3,
    pub source: String,
    pub bounds: yelbegen_math::Aabb,
}

impl Mesh {
    pub fn new(vbuf: Arc<wgpu::Buffer>, vertex_count: u32, center_offset: Vec3, source: String, bounds: yelbegen_math::Aabb) -> Self {
        Self { vbuf, vertex_count, center_offset, source, bounds }
    }
}

#[derive(Clone)]
pub struct Material {
    pub bind_group: Arc<wgpu::BindGroup>,
    pub albedo: yelbegen_math::vec4::Vec4,
    pub roughness: f32,
    pub metallic: f32,
    pub unlit: f32,
    pub texture_source: Option<String>,
}

impl Material {
    pub fn new(bind_group: Arc<wgpu::BindGroup>) -> Self {
        Self {
            bind_group,
            albedo: yelbegen_math::vec4::Vec4::new(1.0, 1.0, 1.0, 1.0),
            roughness: 0.5,
            metallic: 0.0,
            unlit: 0.0,
            texture_source: None,
        }
    }

    pub fn with_pbr(mut self, albedo: yelbegen_math::vec4::Vec4, roughness: f32, metallic: f32) -> Self {
        self.albedo = albedo;
        self.roughness = roughness;
        self.metallic = metallic;
        self.unlit = 0.0;
        self
    }

    pub fn with_unlit(mut self, albedo: yelbegen_math::vec4::Vec4) -> Self {
        self.albedo = albedo;
        self.unlit = 1.0;
        self
    }

    pub fn with_skybox(mut self) -> Self {
        self.unlit = 2.0;
        self
    }

    pub fn with_texture_source(mut self, path: String) -> Self {
        self.texture_source = Some(path);
        self
    }
}

// --- SKELETAL ANIMATION EKLENTILERI ---

#[derive(Clone)]
pub struct Skeleton {
    pub bind_group: Arc<wgpu::BindGroup>,
    pub buffer: Arc<wgpu::Buffer>,
    pub hierarchy: Arc<crate::animation::SkeletonHierarchy>,
    pub local_poses: Vec<yelbegen_math::mat4::Mat4>,
}

#[derive(Clone)]
pub struct AnimationPlayer {
    pub current_time: f32,
    pub active_animation: usize,
    pub loop_anim: bool,
    pub animations: Arc<Vec<crate::animation::AnimationClip>>,
}
pub struct MeshRenderer;

impl MeshRenderer {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MeshRenderer {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Camera {
    pub fov: f32,
    pub near: f32,
    pub far: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub primary: bool,
}

impl Camera {
    pub fn new(fov: f32, near: f32, far: f32, yaw: f32, pitch: f32, primary: bool) -> Self {
        Self { fov, near, far, yaw, pitch, primary }
    }

    pub fn get_projection(&self, aspect: f32) -> yelbegen_math::mat4::Mat4 {
        yelbegen_math::mat4::Mat4::perspective(self.fov, aspect, self.near, self.far)
    }

    pub fn get_view(&self, position: Vec3) -> yelbegen_math::mat4::Mat4 {
        let front = self.get_front();
        yelbegen_math::mat4::Mat4::look_at_rh(position, position + front, Vec3::new(0.0, 1.0, 0.0))
    }
    
    pub fn get_front(&self) -> Vec3 {
        let fx = self.yaw.cos() * self.pitch.cos();
        let fy = self.pitch.sin();
        let fz = self.yaw.sin() * self.pitch.cos();
        Vec3::new(fx, fy, fz).normalize()
    }
    
    pub fn get_right(&self) -> Vec3 {
        self.get_front().cross(Vec3::new(0.0, 1.0, 0.0)).normalize()
    }
}

/// 2D Sprite bileşeni — texture atlas, UV region, layer ordering desteği
#[derive(Clone)]
pub struct Sprite {
    pub width: f32,
    pub height: f32,
    pub uv_min: [f32; 2], // Sprite sheet sol-üst UV (atlas desteği)
    pub uv_max: [f32; 2], // Sprite sheet sağ-alt UV
    pub layer: i32,        // Çizim sırası (Z-order, büyük = daha önde)
    pub flip_x: bool,
    pub flip_y: bool,
}

impl Sprite {
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            width, height,
            uv_min: [0.0, 0.0],
            uv_max: [1.0, 1.0],
            layer: 0,
            flip_x: false,
            flip_y: false,
        }
    }

    pub fn with_uv_region(mut self, min: [f32; 2], max: [f32; 2]) -> Self {
        self.uv_min = min;
        self.uv_max = max;
        self
    }

    pub fn with_layer(mut self, layer: i32) -> Self {
        self.layer = layer;
        self
    }
}

/// 2D Ortografik kamera — sprite/2D oyun rendering için
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Camera2D {
    pub zoom: f32,     // Yakınlaştırma (1.0 = normal)
    pub primary: bool,
}

impl Camera2D {
    pub fn new(zoom: f32) -> Self {
        Self { zoom, primary: true }
    }

    /// Ortografik projeksiyon matrisini döndürür (piksel birimi)
    pub fn get_projection(&self, width: f32, height: f32) -> yelbegen_math::mat4::Mat4 {
        let hw = (width / 2.0) / self.zoom;
        let hh = (height / 2.0) / self.zoom;
        yelbegen_math::mat4::Mat4::orthographic(-hw, hw, -hh, hh, -1000.0, 1000.0)
    }
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct PointLight {
    pub color: Vec3,
    pub intensity: f32,
}

impl PointLight {
    pub fn new(color: Vec3, intensity: f32) -> Self {
        Self { color, intensity }
    }
}

#[derive(Clone, Copy)]
pub struct DirectionalLight {
    pub color: Vec3,
    pub intensity: f32,
    pub is_sun: bool, // Sistemde "birinci ana güneş" olduğunu belirlemek için
}

impl DirectionalLight {
    /// Yeni bir DirectionalLight yaratır. `Transform` bileşeninin rotasyonunu (veya z yönelimini) baz alarak yön belirlenir.
    pub fn new(color: Vec3, intensity: f32, is_sun: bool) -> Self {
        Self {
            color,
            intensity,
            is_sun,
        }
    }
}

/// LOD (Level of Detail) — Kameraya olan mesafeye göre farklı detay seviyelerinde mesh seçimi
#[derive(Clone)]
pub struct LodGroup {
    pub levels: Vec<LodLevel>,
}

#[derive(Clone)]
pub struct LodLevel {
    pub mesh: Mesh,
    pub max_distance: f32, // Bu mesh'in kullanılacağı maksimum kamera mesafesi
}

impl LodGroup {
    pub fn new(levels: Vec<LodLevel>) -> Self {
        let mut levels = levels;
        levels.sort_by(|a, b| a.max_distance.partial_cmp(&b.max_distance).unwrap());
        Self { levels }
    }

    /// Kamera mesafesine göre uygun LOD seviyesinin mesh'ini döndürür
    pub fn select_mesh(&self, distance: f32) -> Option<&Mesh> {
        for level in &self.levels {
            if distance <= level.max_distance {
                return Some(&level.mesh);
            }
        }
        // Tüm LOD eşiklerini aştıysa en son (en düşük detay) mesh'i kullan
        self.levels.last().map(|l| &l.mesh)
    }
}
