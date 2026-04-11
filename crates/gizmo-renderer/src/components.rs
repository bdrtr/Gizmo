use std::sync::Arc;
use gizmo_math::Vec3;

#[derive(Clone)]
pub struct Mesh {
    pub vbuf: Arc<wgpu::Buffer>,
    pub vertex_count: u32,
    pub center_offset: Vec3,
    pub source: String,
    pub bounds: gizmo_math::Aabb,
}

impl Mesh {
    pub fn new(vbuf: Arc<wgpu::Buffer>, vertex_count: u32, center_offset: Vec3, source: String, bounds: gizmo_math::Aabb) -> Self {
        Self { vbuf, vertex_count, center_offset, source, bounds }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MaterialType {
    Pbr,
    Unlit,
    Water,
}

#[derive(Clone)]
pub struct Material {
    pub bind_group: Arc<wgpu::BindGroup>,
    pub albedo: gizmo_math::Vec4,
    pub roughness: f32,
    pub metallic: f32,
    pub unlit: f32,
    pub texture_source: Option<String>,
    pub material_type: MaterialType,
    pub is_transparent: bool,
    pub is_double_sided: bool,
}

impl Material {
    pub fn new(bind_group: Arc<wgpu::BindGroup>) -> Self {
        Self {
            bind_group,
            albedo: gizmo_math::Vec4::new(1.0, 1.0, 1.0, 1.0),
            roughness: 0.5,
            metallic: 0.0,
            unlit: 0.0,
            texture_source: None,
            material_type: MaterialType::Pbr,
            is_transparent: false,
            is_double_sided: false,
        }
    }

    pub fn with_pbr(mut self, albedo: gizmo_math::Vec4, roughness: f32, metallic: f32) -> Self {
        self.albedo = albedo;
        self.roughness = roughness;
        self.metallic = metallic;
        self.unlit = 0.0;
        self.material_type = MaterialType::Pbr;
        self
    }

    pub fn with_transparent(mut self, transparent: bool) -> Self {
        self.is_transparent = transparent;
        self
    }

    pub fn with_double_sided(mut self, double_sided: bool) -> Self {
        self.is_double_sided = double_sided;
        self
    }

    pub fn with_unlit(mut self, albedo: gizmo_math::Vec4) -> Self {
        self.albedo = albedo;
        self.unlit = 1.0;
        self.material_type = MaterialType::Unlit;
        self
    }

    pub fn with_skybox(mut self) -> Self {
        self.unlit = 2.0;
        self.material_type = MaterialType::Unlit;
        self
    }
    
    pub fn with_water(mut self, base_albedo: gizmo_math::Vec4) -> Self {
        self.albedo = base_albedo;
        self.material_type = MaterialType::Water;
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
    pub local_poses: Vec<gizmo_math::Mat4>,
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

    pub fn get_projection(&self, aspect: f32) -> gizmo_math::Mat4 {
        gizmo_math::Mat4::perspective_rh(self.fov, aspect, self.near, self.far)
    }

    pub fn get_view(&self, position: Vec3) -> gizmo_math::Mat4 {
        let front = self.get_front();
        gizmo_math::Mat4::look_at_rh(position, position + front, Vec3::new(0.0, 1.0, 0.0))
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
    pub fn get_projection(&self, width: f32, height: f32) -> gizmo_math::Mat4 {
        let hw = (width / 2.0) / self.zoom;
        let hh = (height / 2.0) / self.zoom;
        gizmo_math::Mat4::orthographic_rh(-hw, hw, -hh, hh, -1000.0, 1000.0)
    }
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct PointLight {
    pub color: gizmo_math::Vec3,
    pub intensity: f32,
    pub radius: f32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Terrain {
    pub heightmap_path: String,
    pub width: f32,
    pub depth: f32,
    pub max_height: f32,
}

impl PointLight {
    pub fn new(color: gizmo_math::Vec3, intensity: f32) -> Self {
        Self { color, intensity, radius: 10.0 }
    }
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
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

// --- PARTICLE SYSTEM ---

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct ParticleEmitter {
    pub spawn_rate: f32, // How many particles to spawn per second
    pub accumulator: f32, // Used over frame deltas
    
    // Extents
    pub local_offset: Vec3, // Local to the entity transform
    
    // Defaults for new spawns
    pub initial_velocity: Vec3,
    pub velocity_randomness: f32,
    pub lifespan: f32,
    pub lifespan_randomness: f32,
    pub size_start: f32,
    pub size_end: f32,
    pub color_start: gizmo_math::Vec4,
    
    // Appearance bindings
    pub texture_source: Option<String>,
    pub is_active: bool,
}

impl Default for ParticleEmitter {
    fn default() -> Self {
        Self::new()
    }
}

impl ParticleEmitter {
    pub fn new() -> Self {
        Self {
            spawn_rate: 10.0,
            accumulator: 0.0,
            local_offset: Vec3::ZERO,
            initial_velocity: Vec3::new(0.0, 1.0, 0.0),
            velocity_randomness: 0.5,
            lifespan: 2.0,
            lifespan_randomness: 0.5,
            size_start: 0.5,
            size_end: 0.1,
            color_start: gizmo_math::Vec4::new(1.0, 0.5, 0.1, 1.0), // Fire/spark default
            texture_source: None,
            is_active: true,
        }
    }
}

pub struct EditorRenderTarget {
    pub view: std::sync::Arc<wgpu::TextureView>,
    pub width: u32,
    pub height: u32,
}
