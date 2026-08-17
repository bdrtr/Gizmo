//! The script command queue — where the change requests coming from Lua scripts accumulate.
//!
//! Lua scripts cannot mutate the World directly (Rust's borrow rules). Commands accumulate in
//! this queue instead and are applied by `flush()` at the end of the frame.

use gizmo_math::{Quat, Vec3};
use std::sync::Mutex;
/// Every change request coming from Lua.
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
    /// The fighting camera, which follows both fighters at once
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


impl ScriptCommand {
    /// Does every number this command carries have a finite value?
    ///
    /// # Why the queue asks
    ///
    /// Lua arithmetic produces NaN and infinity quietly — `0/0`, `math.huge`, a division by a
    /// velocity that happened to be zero — and a script has no reason to notice. Downstream,
    /// nothing recovers: a NaN position makes an entity vanish and every comparison against it
    /// false, a NaN velocity poisons the integrator for that body and then, through contacts, for
    /// whatever it touches, and the determinism hash goes with it. `sanitize_dim` covered collider
    /// dimensions — one of the eleven variants that carry floats — because that was the one that
    /// had bitten someone.
    ///
    /// The match is **exhaustive on purpose**: no `_` arm, so a variant added with a float in it
    /// fails to compile here rather than becoming the next one that was not covered. Variants
    /// carrying no numbers answer `true` by naming themselves, which is the price of that.
    #[must_use]
    pub fn is_finite(&self) -> bool {
        use ScriptCommand::*;
        match self {
            SetPosition(_, v) | SetScale(_, v) | SetVelocity(_, v) | SetAngularVelocity(_, v)
            | ApplyForce(_, v) | ApplyImpulse(_, v) | SetAiTarget(_, v) | PlaySound3D(_, v) => {
                v.is_finite()
            }
            SetRotation(_, q) => q.is_finite(),
            AddRigidBody { mass, .. } => mass.is_finite(),
            AddBoxCollider { hx, hy, hz, .. } => {
                hx.is_finite() && hy.is_finite() && hz.is_finite()
            }
            AddSphereCollider { radius, .. } => radius.is_finite(),
            SetVehicleEngineForce(_, f) | SetVehicleSteering(_, f) | SetVehicleBrake(_, f) => {
                f.is_finite()
            }
            SpawnEntity { position, .. } | SpawnPrefab { position, .. } => position.is_finite(),
            ShowDialogue { duration, .. } => duration.is_finite(),
            AddCheckpoint { position, radius, .. } => position.is_finite() && radius.is_finite(),
            SetCameraFov(f) | SetAnimationSpeed(_, f) => f.is_finite(),
            SetFightCamera { height, distance, .. } => height.is_finite() && distance.is_finite(),
            PlayAnimation { blend, .. } => blend.is_finite(),
            SetFighterMove { damage, .. } => damage.is_finite(),

            // Carry no floating-point numbers. Listed rather than wildcarded — see above.
            DestroyEntity(_)
            | PlaySound(_)
            | StopSound(_)
            | LoadScene(_)
            | SaveScene(_)
            | HideDialogue
            | TriggerCutscene(_)
            | EndCutscene
            | StartRace
            | ActivateCheckpoint(_)
            | FinishRace { .. }
            | ResetRace
            | SetCameraTarget(_)
            | SetEntityName(_, _)
            | AddNavAgent(_)
            | ClearAiTarget(_)
            | ApplyHitstop(_, _)
            | ApplyHitstun(_, _) => true,
        }
    }
}


/// Thread-safe queue of pending [`ScriptCommand`]s, accessible from Lua callbacks.
///
/// Lua callbacks cannot mutate the `World` directly, so they push commands here;
/// the engine later drains and applies them at a controlled point in the frame.
#[derive(Debug, Default)]
pub struct CommandQueue {
    /// Pending commands, guarded by a mutex so Lua callbacks can push concurrently.
    pub commands: Mutex<Vec<ScriptCommand>>,
    /// How many commands [`push`](CommandQueue::push) has refused for carrying NaN or infinity.
    /// Cumulative for the life of the queue; a caller that wants a per-frame figure takes
    /// differences.
    rejected: std::sync::atomic::AtomicU64,
}

impl CommandQueue {
    /// Creates an empty command queue.
    pub fn new() -> Self {
        Self {
            commands: Mutex::new(Vec::new()),
            rejected: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Appends a command to the queue, unless it carries a non-finite number.
    ///
    /// Dropped rather than clamped: a clamped force is a wrong answer the frame accepts silently,
    /// while a dropped one is a no-op with a log line and a counter behind it. A script that
    /// produced NaN has a bug, and the useful thing to do is say so, not to guess what it meant.
    pub fn push(&self, cmd: ScriptCommand) {
        if !cmd.is_finite() {
            tracing::warn!(
                command = ?cmd,
                "[Scripting] command dropped: carries NaN or infinity"
            );
            self.rejected
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return;
        }
        // Poison-recovery: bir thread lock tutarken panic etse bile kuyruk
        // kullanılabilir kalır (FFI/Lua callback sınırında panic-free).
        self.commands
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(cmd);
    }

    /// How many commands have been refused for carrying NaN or infinity, since this queue was
    /// created.
    #[must_use]
    pub fn rejected_count(&self) -> u64 {
        self.rejected.load(std::sync::atomic::Ordering::Relaxed)
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

    /// The queue is the one place every command passes, so it is the one place that has to ask.
    #[test]
    fn a_command_carrying_nan_never_reaches_the_queue() {
        let q = CommandQueue::new();
        q.push(ScriptCommand::SetPosition(1, Vec3::new(f32::NAN, 0.0, 0.0)));
        q.push(ScriptCommand::ApplyForce(1, Vec3::new(0.0, f32::INFINITY, 0.0)));
        q.push(ScriptCommand::SetCameraFov(f32::NAN));
        q.push(ScriptCommand::SetAnimationSpeed(1, f32::NEG_INFINITY));
        q.push(ScriptCommand::SetRotation(1, Quat::from_xyzw(f32::NAN, 0.0, 0.0, 1.0)));

        assert_eq!(q.rejected_count(), 5, "every one of these should have been refused");
        assert!(q.drain().is_empty(), "a non-finite command reached the queue");
    }

    /// …and the guard must not cost the ordinary case anything.
    #[test]
    fn finite_commands_pass_through_untouched() {
        let q = CommandQueue::new();
        q.push(ScriptCommand::SetPosition(1, Vec3::new(1.0, 2.0, 3.0)));
        q.push(ScriptCommand::DestroyEntity(2));
        q.push(ScriptCommand::PlaySound("hit".into()));
        q.push(ScriptCommand::AddCheckpoint {
            id: 3,
            position: Vec3::ZERO,
            radius: 4.0,
        });

        assert_eq!(q.rejected_count(), 0);
        assert_eq!(q.drain().len(), 4);
    }

    /// A NaN in ONE field is enough, whichever field it is — the multi-float variants are where a
    /// per-field check gets written for two of three and then forgotten.
    #[test]
    fn one_bad_field_condemns_the_whole_command() {
        let bad_z = ScriptCommand::AddBoxCollider { id: 1, hx: 1.0, hy: 1.0, hz: f32::NAN };
        assert!(!bad_z.is_finite(), "hz was not checked");
        let bad_distance =
            ScriptCommand::SetFightCamera { p1_id: 1, p2_id: 2, height: 3.0, distance: f32::NAN };
        assert!(!bad_distance.is_finite(), "distance was not checked");
        let bad_position = ScriptCommand::SpawnPrefab {
            name: "x".into(),
            prefab_type: "y".into(),
            position: Vec3::new(0.0, 0.0, f32::INFINITY),
        };
        assert!(!bad_position.is_finite(), "position.z was not checked");
    }
    use super::*;
    use std::sync::Arc;

    /// `drain` must preserve FIFO order and return EVERY command that was pushed.
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

    /// `new()` and `default()` must both produce an empty queue, with len/is_empty
    /// consistent.
    #[test]
    fn new_and_default_start_empty_and_agree() {
        for q in [CommandQueue::new(), CommandQueue::default()] {
            assert!(q.is_empty());
            assert_eq!(q.len(), 0);
        }
    }

    /// `drain` must empty the queue: the first drain returns the commands, the second returns
    /// nothing.
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

    /// Concurrent pushes: N threads × M commands must accumulate as exactly N*M commands with
    /// none lost. (The total-conservation invariant the mutex provides.)
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

    /// A poisoned mutex (a thread panicked while holding the lock) must not leave the queue
    /// unusable — with poison recovery, push/drain/len keep working panic-free.
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
