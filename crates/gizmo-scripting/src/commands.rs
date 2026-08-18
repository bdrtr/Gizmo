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
    // ── Transform ────────────────────────────────────────────────────────────
    /// Move an entity to a world position, in metres.
    SetPosition(u32, Vec3),
    /// Set an entity's world rotation. Not normalised on the way through: a quaternion built by
    /// hand in Lua arrives as it was written.
    SetRotation(u32, Quat),
    /// Set an entity's scale. Collider geometry does not follow it — see `Collider`.
    SetScale(u32, Vec3),

    // ── Velocity ─────────────────────────────────────────────────────────────
    /// Set a body's linear velocity outright, in m/s. Overwrites what the solver produced this
    /// frame rather than adding to it.
    SetVelocity(u32, Vec3),
    /// Set a body's angular velocity outright, in rad/s.
    SetAngularVelocity(u32, Vec3),

    // ── Physics ──────────────────────────────────────────────────────────────
    /// Apply a force (N) for this frame — an acceleration once divided by mass. A zero-mass body
    /// is skipped rather than accelerated infinitely.
    ApplyForce(u32, Vec3),
    /// Apply an impulse (N·s) — an instantaneous velocity change, mass accounted for.
    ApplyImpulse(u32, Vec3),
    /// Give an entity a rigid body, and a `Velocity` with it: without one it could not move.
    AddRigidBody {
        /// Entity to add the body to.
        id: u32,
        /// Mass in kilograms.
        mass: f32,
        /// Whether gravity acts on it.
        use_gravity: bool,
    },
    /// Give an entity a box collider, sized by half-extents in metres.
    AddBoxCollider {
        /// Entity to add the collider to.
        id: u32,
        /// Half-extent on X. Clamped away from zero and non-finite values before use.
        hx: f32,
        /// Half-extent on Y.
        hy: f32,
        /// Half-extent on Z.
        hz: f32,
    },
    /// Give an entity a sphere collider.
    AddSphereCollider {
        /// Entity to add the collider to.
        id: u32,
        /// Radius in metres, clamped away from zero and non-finite values.
        radius: f32,
    },

    // ── Vehicle ──────────────────────────────────────────────────────────────
    /// Set a vehicle's engine force. Negative drives it backwards.
    SetVehicleEngineForce(u32, f32),
    /// Set a vehicle's steering input, −1 (left) to 1 (right).
    SetVehicleSteering(u32, f32),
    /// Set a vehicle's brake input, 0 to 1.
    SetVehicleBrake(u32, f32),

    // ── Entity lifecycle ─────────────────────────────────────────────────────
    /// Spawn a named entity carrying a `Transform` at `position`, and nothing else.
    SpawnEntity {
        /// The `EntityName` it is spawned with.
        name: String,
        /// Where it appears, in world space.
        position: Vec3,
    },
    /// Spawn an entity from a prefab.
    SpawnPrefab {
        /// The `EntityName` the instance is given.
        name: String,
        /// Which prefab to instantiate.
        prefab_type: String,
        /// Where it appears, in world space.
        position: Vec3,
    },
    /// Despawn an entity. A stale id is a no-op, not an error.
    DestroyEntity(u32),

    // ── Audio ────────────────────────────────────────────────────────────────
    /// Play a sound by name, without a position (2D).
    PlaySound(String),
    /// Play a sound positioned in the world, so the listener hears it spatially.
    PlaySound3D(String, Vec3),
    /// Stop a sound by name.
    StopSound(String),

    // ── Scene ────────────────────────────────────────────────────────────────
    /// Load a scene file. Handed back to the host: this crate does not own scene loading.
    LoadScene(String),
    /// Save the current scene to a file. Also the host's to carry out.
    SaveScene(String),

    // ── Dialogue ─────────────────────────────────────────────────────────────
    /// Show a line of dialogue.
    ShowDialogue {
        /// Who is speaking, for the name plate.
        speaker: String,
        /// The line itself.
        text: String,
        /// How long to leave it up, in seconds.
        duration: f32,
    },
    /// Hide the dialogue box now, whatever time it had left.
    HideDialogue,

    // ── Cutscene ─────────────────────────────────────────────────────────────
    /// Start the named cutscene.
    TriggerCutscene(String),
    /// End the running cutscene and hand control back.
    EndCutscene,

    // ── Race ─────────────────────────────────────────────────────────────────
    /// Start the race — the timer runs and checkpoints begin counting.
    StartRace,
    /// Register a checkpoint the racer has to pass through.
    AddCheckpoint {
        /// Checkpoint number, which is also the order they must be taken in.
        id: u32,
        /// Its centre, in world space.
        position: Vec3,
        /// How close counts as passing through it, in metres.
        radius: f32,
    },
    /// Mark a checkpoint as reached.
    ActivateCheckpoint(u32),
    /// End the race.
    FinishRace {
        /// Who won, for the results screen.
        winner_name: String,
    },
    /// Reset the race: the timer, the checkpoints and the standings.
    ResetRace,

    // ── Camera ───────────────────────────────────────────────────────────────
    /// Make the camera follow an entity.
    SetCameraTarget(u32),
    /// Set the camera's vertical field of view, in degrees.
    SetCameraFov(f32),
    /// The fighting camera, which follows both fighters at once
    SetFightCamera {
        /// First fighter's entity.
        p1_id: u32,
        /// Second fighter's entity.
        p2_id: u32,
        /// How far above the pair the camera sits (Y offset, metres).
        height: f32,
        /// How far back it sits at minimum (Z offset, metres) — it pulls further out as the
        /// fighters separate, never closer than this.
        distance: f32,
    },

    // ── Components ───────────────────────────────────────────────────────────
    /// Rename an entity, i.e. overwrite its `EntityName`.
    SetEntityName(u32, String),
    /// Play an animation clip on an entity.
    PlayAnimation {
        /// Entity to animate.
        id: u32,
        /// Clip name.
        name: String,
        /// Blend time into the clip, in seconds.
        blend: f32,
        /// Whether the clip loops.
        loop_anim: bool,
    },
    /// Scale an entity's animation playback rate — 1.0 is normal speed.
    SetAnimationSpeed(u32, f32),

    // ── AI ───────────────────────────────────────────────────────────────────
    /// Give an entity a navigation agent, so it can be sent places.
    AddNavAgent(u32),
    /// Send an agent to a world position.
    SetAiTarget(u32, Vec3),
    /// Clear an agent's destination — the target *and* the path it had planned.
    ClearAiTarget(u32),

    // ── Fighter ──────────────────────────────────────────────────────────────
    /// Start a fighting-game move, with its frame data.
    SetFighterMove {
        /// The fighter's entity.
        id: u32,
        /// The move's name.
        name: String,
        /// Startup frames before the hitbox is live.
        startup: u32,
        /// Frames the hitbox is live for.
        active: u32,
        /// Recovery frames after it, during which the fighter cannot act.
        recovery: u32,
        /// Damage dealt on hit.
        damage: f32,
        /// Frames of stun this move inflicts on the fighter it hits.
        ///
        /// Carried here rather than left to `FrameData::default()` because it is the move's own
        /// number and the script authoring the move is the only thing that knows it. Until this
        /// field existed every Lua-authored move silently inherited a 20-frame stun, and the
        /// script could neither set it nor read it back.
        hitstun: u32,
        /// Frames of hit-freeze this move inflicts on connect — the weight of the blow. Same
        /// story as `hitstun`: it used to be a constant 5 for every move Lua could start.
        hitstop: u32,
    },
    /// Set a fighter's health outright.
    ///
    /// The write that makes `HitEvent` spendable: the engine resolves a hit and reports its
    /// damage, and the game (here, a script) decides what that costs. An assignment rather than a
    /// subtraction, because clamping, armour, a block that halves it and death are all the game's.
    SetFighterHealth(u32, f32),
    /// Freeze a fighter for a number of frames on impact — the hit-stop that gives a blow its
    /// weight.
    ApplyHitstop(u32, u32),
    /// Lock a fighter out of acting for a number of frames after being hit.
    ApplyHitstun(u32, u32)
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
            SetFighterHealth(_, health) => health.is_finite(),

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
