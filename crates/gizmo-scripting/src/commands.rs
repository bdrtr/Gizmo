//! Script Command Queue — Lua scriptlerden gelen değişiklik isteklerinin biriktirildiği kuyruk
//!
//! Lua scriptleri doğrudan World'ü mutate edemez (Rust borrow kuralları).
//! Bunun yerine komutlar bu kuyrukta birikir ve frame sonunda `flush()` ile uygulanır.

use gizmo_math::{Quat, Vec3};
use std::sync::Mutex;
/// Lua'dan gelen tüm değişiklik istekleri
#[derive(Debug, Clone)]
pub enum ScriptCommand {
    // Transform
    SetPosition(u32, Vec3),
    SetRotation(u32, Quat),
    SetScale(u32, Vec3),

    // Velocity
    SetVelocity(u32, Vec3),
    SetAngularVelocity(u32, Vec3),

    // Physics
    ApplyForce(u32, Vec3),
    ApplyImpulse(u32, Vec3),
    AddRigidBody {
        id: u32,
        mass: f32,
        restitution: f32,
        friction: f32,
        use_gravity: bool,
    },
    AddBoxCollider {
        id: u32,
        hx: f32,
        hy: f32,
        hz: f32,
    },
    AddSphereCollider {
        id: u32,
        radius: f32,
    },

    // Vehicle
    SetVehicleEngineForce(u32, f32),
    SetVehicleSteering(u32, f32),
    SetVehicleBrake(u32, f32),

    // Entity Lifecycle
    SpawnEntity {
        name: String,
        position: Vec3,
    },
    SpawnPrefab {
        name: String,
        prefab_type: String,
        position: Vec3,
    },
    DestroyEntity(u32),

    // Audio
    PlaySound(String),
    PlaySound3D(String, Vec3),
    StopSound(String),

    // Scene
    LoadScene(String),
    SaveScene(String),

    // Diyalog Sistemi
    ShowDialogue {
        speaker: String,
        text: String,
        duration: f32,
    },
    HideDialogue,

    // Ara Sahne (Cutscene)
    TriggerCutscene(String), // cutscene adı/id
    EndCutscene,

    // Yarış Sistemi
    StartRace,
    AddCheckpoint {
        id: u32,
        position: Vec3,
        radius: f32,
    },
    ActivateCheckpoint(u32),
    FinishRace {
        winner_name: String,
    },
    ResetRace,

    // Kamera
    SetCameraTarget(u32), // hangi entity'yi takip etsin
    SetCameraFov(f32),
    /// İki dövüşçüyü aynı anda takip eden fighting camera
    SetFightCamera {
        p1_id: u32,
        p2_id: u32,
        height: f32,     // Kamera yüksekliği (Y offset)
        distance: f32,   // Minimum uzaklık (Z offset)
    },

// Component
    SetEntityName(u32, String),
PlayAnimation {
        id: u32,
        name: String,
        blend: f32,
        loop_anim: bool,
    },
    SetAnimationSpeed(u32, f32),


    // AI
    AddNavAgent(u32),
    SetAiTarget(u32, Vec3),
    ClearAiTarget(u32),

    // Fighter
    SetFighterMove {
        id: u32,
        name: String,
        startup: u32,
        active: u32,
        recovery: u32,
        damage: f32,
    },
    ApplyHitstop(u32, u32),
    ApplyHitstun(u32, u32),
}


/// Thread-local komut kuyruğu (Lua callback'leri içinden erişilebilir)
pub struct CommandQueue {
    pub commands: Mutex<Vec<ScriptCommand>>,
}

impl CommandQueue {
    pub fn new() -> Self {
        Self {
            commands: Mutex::new(Vec::new()),
        }
    }

    pub fn push(&self, cmd: ScriptCommand) {
        self.commands.lock().unwrap().push(cmd);
    }

    pub fn drain(&self) -> Vec<ScriptCommand> {
        self.commands.lock().unwrap().drain(..).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.commands.lock().unwrap().is_empty()
    }

    pub fn len(&self) -> usize {
        self.commands.lock().unwrap().len()
    }
}

impl Default for CommandQueue {
    fn default() -> Self {
        Self::new()
    }
}
