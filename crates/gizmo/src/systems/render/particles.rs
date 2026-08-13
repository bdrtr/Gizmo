//! The missing link between `ParticleEmitter` and the GPU particle system.
//!
//! # Why this file exists
//!
//! Everything else on the particle path was already here. `default_render_pass` calls
//! `update_params` and `compute_pass` on `renderer.gpu_particles` every frame, and `passes/forward`
//! draws the result. What nothing did was **put anything in it**: the engine simulated and drew a
//! particle set that no scene ever populated, because the step that reads `ParticleEmitter`
//! entities and spawns from them lived only inside `gizmo-studio`'s own render pipeline.
//!
//! So an entity carrying a `ParticleEmitter` emitted nothing, for anyone using the engine's
//! out-of-the-box pass. Same shape as `LodGroup` (visible only to studio) and as skeletal
//! animation (systems written, exported, never scheduled) — see `docs/ENGINE.md`.
//!
//! # Why it is in the facade and not in `gizmo-renderer`
//!
//! It needs both halves and they live in different crates: `GpuParticleSystem` is in
//! `gizmo-renderer`, and `Transform` — which is where an emitter *is* — is in
//! `gizmo-physics-core`, which `gizmo-renderer` does not depend on. This is the lowest crate that
//! can see both. Being `pub` is deliberate: studio calls this instead of keeping its own copy, so
//! the two paths cannot drift.
//!
//! # Randomness
//!
//! The code this replaces called `rand::rng()`. Here the jitter comes from a four-line xorshift
//! seeded per emitter per frame instead, for two reasons that both happen to point the same way.
//! The facade would otherwise gain a dependency — every one of those is a compile-time and
//! supply-chain cost paid by everyone who builds `gizmo-engine`, and cosmetic jitter is a thin
//! reason to charge it. And a deterministic engine emitting nondeterministic sparks is a small
//! lie: a replay now reproduces its particles as exactly as it reproduces its physics. Nothing
//! here writes simulation state either way, so this was never a contract violation — only an
//! avoidable inconsistency.

use crate::renderer::components::ParticleEmitter;
use crate::renderer::gpu_particles::{GpuParticle, GpuParticleSystem};
use gizmo_core::World;
use gizmo_math::Vec3;
use gizmo_physics_core::components::Transform;

/// How many particles one emitter may spawn in a single frame.
///
/// A frame drop makes `dt` large, which makes the accumulator hand out hundreds of spawns at once
/// and turns one stutter into a sustained one. The cap converts that into dropped particles, which
/// nobody sees, instead of a frame-rate collapse, which everybody does.
const MAX_SPAWNS_PER_FRAME: u32 = 100;

/// Advance every active emitter and spawn what it owes into the GPU particle system.
///
/// Call once per frame, before the particle compute pass — the particles spawned here are stepped
/// by that pass in the same frame.
pub fn spawn_from_emitters(
    world: &mut World,
    particles: &GpuParticleSystem,
    queue: &wgpu::Queue,
    dt: f32,
) {
    // Collected through a read borrow that ends with this statement, so the mutable borrow below
    // never coexists with a read borrow of the same storage.
    let emitter_entities: Vec<u32> = world.borrow::<ParticleEmitter>().entities().collect();
    if emitter_entities.is_empty() {
        return;
    }

    // SAFETY: exclusive `&mut World`; `ParticleEmitter` is a distinct component type from the
    // read-only `Transform` borrow below, and the read borrow above is already dropped, so this
    // mutable view never aliases another live access to the same storage.
    let mut emitters = unsafe { world.borrow_mut_unchecked::<ParticleEmitter>() };
    let transforms = world.borrow::<Transform>();

    // Seeded from the frame so successive frames differ, and mixed with the entity below so two
    // emitters in the same frame do not emit in lockstep.
    let frame = world
        .get_resource::<gizmo_core::time::Time>()
        .map_or(0, |t| t.frame());
    let mut spawned = Vec::new();

    for id in emitter_entities {
        let Some(mut emitter) = emitters.get_mut(id) else {
            continue;
        };
        if !emitter.is_active || emitter.spawn_rate <= 0.0 {
            continue;
        }

        // An emitter with no transform still emits, at its own local offset. It is a worse answer
        // than a position but a much better one than silence, and it is what a caller who spawned
        // a bare emitter almost certainly meant.
        let origin = match transforms.get(id) {
            Some(t) => t.position + t.rotation.mul_vec3(emitter.local_offset),
            None => emitter.local_offset,
        };

        emitter.add_time(dt);
        let interval = 1.0 / emitter.spawn_rate;
        let mut this_frame = 0;
        let mut rng = Xorshift::seeded(frame, id);
        while emitter.get_accumulator() >= interval && this_frame < MAX_SPAWNS_PER_FRAME {
            emitter.consume_time(interval);
            this_frame += 1;

            let jitter = Vec3::new(
                rng.signed() * emitter.velocity_randomness,
                rng.signed() * emitter.velocity_randomness,
                rng.signed() * emitter.velocity_randomness,
            );
            let velocity = emitter.initial_velocity + jitter;
            let life = (emitter.lifespan + rng.signed() * emitter.lifespan_randomness).max(0.1);

            spawned.push(GpuParticle {
                position: [origin.x, origin.y, origin.z],
                life: 0.0,
                velocity: [velocity.x, velocity.y, velocity.z],
                max_life: life,
                color: emitter.color_start.into(),
                size_start: emitter.size_start,
                size_end: emitter.size_end,
                _padding: [0.0; 2],
            });
        }
    }

    if !spawned.is_empty() {
        particles.spawn_particles(queue, &spawned);
    }
}

/// A four-line xorshift, enough for spawn jitter and nothing else.
///
/// Not a general-purpose RNG and not offered as one: it is private, it is used for cosmetic
/// velocity and lifetime spread, and its only real requirement is that the same frame produces the
/// same sparks. Anything needing statistical quality should reach for `rand`.
struct Xorshift(u32);

impl Xorshift {
    fn seeded(frame: u64, entity: u32) -> Self {
        // Mix so that neighbouring frames and neighbouring entity ids do not give neighbouring
        // streams — a raw `frame + entity` seed makes two adjacent emitters emit near-identically.
        let mixed = (frame as u32)
            .wrapping_mul(0x9E37_79B9)
            .wrapping_add(entity.wrapping_mul(0x85EB_CA6B));
        // Zero is a fixed point of xorshift, so it can never be the state.
        Self(mixed | 1)
    }

    fn next_u32(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }

    /// Uniform in `[-1, 1]`.
    fn signed(&mut self) -> f32 {
        (self.next_u32() as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::Xorshift;

    #[test]
    fn the_jitter_stays_in_range_and_moves() {
        let mut rng = Xorshift::seeded(7, 3);
        let mut seen = [0.0f32; 64];
        for slot in &mut seen {
            *slot = rng.signed();
            assert!((-1.0..=1.0).contains(slot), "out of range: {slot}");
        }
        // A generator stuck on one value would pass the range check and produce no jitter at all.
        assert!(
            seen.windows(2).any(|w| (w[0] - w[1]).abs() > 1e-6),
            "the sequence never changed"
        );
    }

    #[test]
    fn two_emitters_in_one_frame_do_not_emit_in_lockstep() {
        let (mut a, mut b) = (Xorshift::seeded(7, 3), Xorshift::seeded(7, 4));
        assert_ne!(a.signed(), b.signed());
    }

    #[test]
    fn the_same_frame_and_entity_replays_identically() {
        let (mut a, mut b) = (Xorshift::seeded(12, 5), Xorshift::seeded(12, 5));
        for _ in 0..8 {
            assert_eq!(a.signed(), b.signed());
        }
    }
}
