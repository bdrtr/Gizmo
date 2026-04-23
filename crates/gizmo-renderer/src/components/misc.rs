use gizmo_math::Vec3;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Terrain {
    pub heightmap_path: String,
    pub width: f32,
    pub depth: f32,
    pub max_height: f32,
}

impl Terrain {
    pub fn new(heightmap_path: String, width: f32, depth: f32, max_height: f32) -> Self {
        assert!(width > 0.0, "Kullanım hatası: Terrain width sıfırdan büyük olmalıdır.");
        assert!(depth > 0.0, "Kullanım hatası: Terrain depth sıfırdan büyük olmalıdır.");
        assert!(max_height > 0.0, "Kullanım hatası: Terrain max_height sıfırdan büyük olmalıdır.");
        Self {
            heightmap_path,
            width,
            depth,
            max_height,
        }
    }
}

#[derive(Clone)]
pub struct LodGroup {
    pub levels: Vec<LodLevel>,
}

#[derive(Clone)]
pub struct LodLevel {
    pub mesh: super::mesh::Mesh,
    pub max_distance: f32,
}

impl LodLevel {
    pub fn new(mesh: super::mesh::Mesh, max_distance: f32) -> Self {
        assert!(max_distance >= 0.0, "Kullanım hatası: LodLevel max_distance negatif olamaz.");
        Self { mesh, max_distance }
    }
}

impl LodGroup {
    pub fn new(levels: Vec<LodLevel>) -> Self {
        assert!(
            !levels.is_empty(),
            "Kullanım hatası: LodGroup en az 1 LodLevel içermelidir (Boş liste görünmezlik hatalarına yol açar)."
        );
        let mut levels = levels;
        levels.sort_by(|a, b| a.max_distance.total_cmp(&b.max_distance));
        Self { levels }
    }

    pub fn select_mesh(&self, distance: f32) -> Option<&super::mesh::Mesh> {
        for level in &self.levels {
            if distance <= level.max_distance {
                return Some(&level.mesh);
            }
        }
        self.levels.last().map(|l| &l.mesh)
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct ParticleEmitter {
    pub spawn_rate: f32,
    accumulator: f32,
    pub local_offset: Vec3,
    pub initial_velocity: Vec3,
    pub velocity_randomness: f32,
    pub lifespan: f32,
    pub lifespan_randomness: f32,
    pub size_start: f32,
    pub size_end: f32,
    pub color_start: gizmo_math::Vec4,
    pub color_end: gizmo_math::Vec4,
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
            color_start: gizmo_math::Vec4::new(1.0, 0.5, 0.1, 1.0),
            color_end: gizmo_math::Vec4::new(1.0, 0.2, 0.0, 0.0),
            texture_source: None,
            is_active: true,
        }
    }

    /// Yeni bir parçacık yayıcı oluşturur ve doğrulama yapar.
    pub fn with_rate(spawn_rate: f32) -> Self {
        assert!(spawn_rate > 0.0, "Kullanım hatası: ParticleEmitter spawn_rate sıfırdan büyük olmalıdır.");
        Self {
            spawn_rate,
            ..Self::default()
        }
    }

    pub fn get_accumulator(&self) -> f32 {
        self.accumulator
    }

    pub fn add_time(&mut self, dt: f32) {
        self.accumulator += dt;
    }

    pub fn consume_time(&mut self, dt: f32) {
        self.accumulator -= dt;
    }
}

#[derive(Clone)]
pub struct RenderTarget {
    pub view: std::sync::Arc<wgpu::TextureView>,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone)]
pub struct EditorRenderTarget(pub RenderTarget);

#[derive(Clone)]
pub struct GameRenderTarget(pub RenderTarget);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FluidPhaseType {
    Water,
    Foam,
    Bubble,
}

/// Bir entity'nin sıvı parçacığı olduğunu belirten ECS marker bileşenidir.
/// Simülasyonda FluidHandle, FluidPhase, FluidInteractor ile birlikte kullanılabilir.
#[derive(Clone)]
pub struct FluidParticle;

#[derive(Clone)]
pub struct FluidHandle {
    pub gpu_index: u32,
}

#[derive(Clone)]
pub struct FluidPhase {
    pub phase: FluidPhaseType,
}

#[derive(Clone)]
pub struct FluidInteractor {
    pub collider_gpu_index: u32,
    pub buoyancy_factor: f32,
    pub radius: f32,
    pub velocity: gizmo_math::Vec3,
}
