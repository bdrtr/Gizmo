//! Script Command Queue — Lua scriptlerden gelen değişiklik isteklerinin biriktirildiği kuyruk
//!
//! Lua scriptleri doğrudan World'ü mutate edemez (Rust borrow kuralları).
//! Bunun yerine komutlar bu kuyrukta birikir ve frame sonunda `flush()` ile uygulanır.

use gizmo_math::{Quat, Vec3};
use std::sync::Mutex;
/// Lua'dan gelen tüm değişiklik istekleri
#[derive(Debug, Clone)]
#[non_exhaustive]
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


/// Thread-safe queue of pending [`ScriptCommand`]s, accessible from Lua callbacks.
///
/// Lua callbacks cannot mutate the `World` directly, so they push commands here;
/// the engine later drains and applies them at a controlled point in the frame.
#[derive(Debug, Default)]
pub struct CommandQueue {
    /// Pending commands, guarded by a mutex so Lua callbacks can push concurrently.
    pub commands: Mutex<Vec<ScriptCommand>>,
}

impl CommandQueue {
    /// Creates an empty command queue.
    pub fn new() -> Self {
        Self {
            commands: Mutex::new(Vec::new()),
        }
    }

    /// Appends a command to the queue.
    pub fn push(&self, cmd: ScriptCommand) {
        // Poison-recovery: bir thread lock tutarken panic etse bile kuyruk
        // kullanılabilir kalır (FFI/Lua callback sınırında panic-free).
        self.commands
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(cmd);
    }

    /// Removes and returns all currently queued commands, leaving the queue empty.
    pub fn drain(&self) -> Vec<ScriptCommand> {
        // Poison-recovery: zehirlenmiş mutex'i kurtar, panic etme.
        self.commands
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
            .collect()
    }

    /// Returns `true` if no commands are currently queued.
    pub fn is_empty(&self) -> bool {
        // Poison-recovery: zehirlenmiş mutex'i kurtar, panic etme.
        self.commands
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
    }

    /// Returns the number of currently queued commands.
    pub fn len(&self) -> usize {
        // Poison-recovery: zehirlenmiş mutex'i kurtar, panic etme.
        self.commands
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// `drain` FIFO sırayı korumalı ve push edilen HER komutu döndürmeli.
    #[test]
    fn drain_preserves_push_order() {
        let q = CommandQueue::new();
        q.push(ScriptCommand::SetPosition(1, Vec3::new(1.0, 0.0, 0.0)));
        q.push(ScriptCommand::DestroyEntity(2));
        q.push(ScriptCommand::StartRace);

        let drained = q.drain();
        assert_eq!(drained.len(), 3);
        assert!(matches!(drained[0], ScriptCommand::SetPosition(1, _)));
        assert!(matches!(drained[1], ScriptCommand::DestroyEntity(2)));
        assert!(matches!(drained[2], ScriptCommand::StartRace));
    }

    /// `new()` ve `default()` her ikisi de boş kuyruk üretmeli; len/is_empty tutarlı olmalı.
    #[test]
    fn new_and_default_start_empty_and_agree() {
        for q in [CommandQueue::new(), CommandQueue::default()] {
            assert!(q.is_empty());
            assert_eq!(q.len(), 0);
        }
    }

    /// `drain` kuyruğu boşaltmalı: ilk drain komutları döndürür, ikincisi boş döner.
    #[test]
    fn drain_empties_queue() {
        let q = CommandQueue::new();
        q.push(ScriptCommand::HideDialogue);
        assert_eq!(q.len(), 1);
        assert!(!q.is_empty());

        let first = q.drain();
        assert_eq!(first.len(), 1);

        // Boşaldı: len/is_empty tutarlı, ikinci drain boş.
        assert_eq!(q.len(), 0);
        assert!(q.is_empty());
        assert!(q.drain().is_empty());
    }

    /// Eşzamanlı push'lar: N thread × M komut = tam olarak N*M komut kaybolmadan birikmeli.
    /// (Mutex'in sağladığı toplam-koruma invariant'ı.)
    #[test]
    fn concurrent_pushes_are_all_recorded() {
        let q = Arc::new(CommandQueue::new());
        let threads = 8;
        let per_thread = 250;

        let handles: Vec<_> = (0..threads)
            .map(|_| {
                let q = q.clone();
                std::thread::spawn(move || {
                    for i in 0..per_thread {
                        q.push(ScriptCommand::DestroyEntity(i));
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(q.len(), threads * per_thread as usize);
        assert_eq!(q.drain().len(), threads * per_thread as usize);
    }

    /// Zehirlenmiş mutex (bir thread lock tutarken panic etti) kuyruğu kullanılamaz
    /// bırakmamalı — poison-recovery ile push/drain/len panic-free çalışmaya devam etmeli.
    #[test]
    fn survives_poisoned_mutex() {
        let q = Arc::new(CommandQueue::new());
        q.push(ScriptCommand::StartRace);

        // Lock'u tutarken panic ederek mutex'i zehirle.
        let q2 = q.clone();
        let joined = std::thread::spawn(move || {
            let _guard = q2.commands.lock().unwrap();
            panic!("mutex'i kasıtlı zehirle");
        })
        .join();
        assert!(joined.is_err(), "thread panic etmeliydi");

        // Zehirli olsa da kuyruk hâlâ çalışmalı.
        assert_eq!(q.len(), 1);
        q.push(ScriptCommand::EndCutscene);
        assert_eq!(q.len(), 2);
        let drained = q.drain();
        assert_eq!(drained.len(), 2);
        assert!(q.is_empty());
    }
}
