//! Script Command Queue — Lua scriptlerden gelen değişiklik isteklerinin biriktirildiği kuyruk
//!
//! Lua scriptleri doğrudan World'ü mutate edemez (Rust borrow kuralları).
//! Bunun yerine komutlar bu kuyrukta birikir ve frame sonunda `flush()` ile uygulanır.

use gizmo_math::{Vec3, Quat};
use std::cell::RefCell;

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
    
    // Entity Lifecycle
    SpawnEntity { name: String, position: Vec3 },
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
    ShowDialogue { speaker: String, text: String, duration: f32 },
    HideDialogue,

    // Ara Sahne (Cutscene)
    TriggerCutscene(String), // cutscene adı/id
    EndCutscene,

    // Yarış Sistemi
    AddCheckpoint { id: u32, position: Vec3, radius: f32 },
    ActivateCheckpoint(u32),
    FinishRace { winner_name: String },
    ResetRace,

    // Kamera
    SetCameraTarget(u32),    // hangi entity'yi takip etsin
    SetCameraFov(f32),

    // Component
    SetEntityName(u32, String),
}

/// Thread-local komut kuyruğu (Lua callback'leri içinden erişilebilir)
pub struct CommandQueue {
    pub commands: RefCell<Vec<ScriptCommand>>,
}

impl CommandQueue {
    pub fn new() -> Self {
        Self {
            commands: RefCell::new(Vec::new()),
        }
    }

    pub fn push(&self, cmd: ScriptCommand) {
        self.commands.borrow_mut().push(cmd);
    }

    pub fn drain(&self) -> Vec<ScriptCommand> {
        self.commands.borrow_mut().drain(..).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.commands.borrow().is_empty()
    }

    pub fn len(&self) -> usize {
        self.commands.borrow().len()
    }
}

impl Default for CommandQueue {
    fn default() -> Self {
        Self::new()
    }
}
