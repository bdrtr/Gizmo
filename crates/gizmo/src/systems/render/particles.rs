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

use gizmo_core::Entity;
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

/// How many particles one emitter's [`burst`](ParticleEmitter::burst) may put out in a frame.
/// Larger than the rate cap — a burst is meant to be big and happens once — but bounded, so a
/// mistyped count does not stall a frame on a giant upload.
const MAX_BURST: u32 = 4096;

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
    let emitter_entities: Vec<u32> = world.query::<(Entity, &ParticleEmitter)>().unwrap().iter().map(|(entity, _)| entity.id()).collect();
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
        if !emitter.is_active {
            continue;
        }

        // An emitter with no transform still emits, at its own local offset. It is a worse answer
        // than a position but a much better one than silence, and it is what a caller who spawned
        // a bare emitter almost certainly meant.
        let origin = match transforms.get(id) {
            Some(t) => t.position + t.rotation.mul_vec3(emitter.local_offset),
            None => emitter.local_offset,
        };

        // One stream per (frame, emitter): the same frame replays the same sparks, and neighbouring
        // emitters do not emit in lockstep.
        let mut rng = gizmo_math::Rng::with_stream(frame, u64::from(id));
        emit(&mut emitter, origin, dt, &mut rng, &mut spawned);
    }

    if !spawned.is_empty() {
        particles.spawn_particles(queue, &spawned);
    }
}

/// What one emitter puts out this frame, appended to `out`: its pending
/// [`burst`](ParticleEmitter::burst) all at once, then what its `spawn_rate` owes for `dt`.
///
/// The one place the emission rule lives, so the game's draw path ([`spawn_from_emitters`]) and the
/// studio's agree. They used to be two copies of the loop — the studio's comment said "the two
/// must change together" — and a burst added to one would have been a burst in one picture only.
pub fn emit(
    emitter: &mut ParticleEmitter,
    origin: Vec3,
    dt: f32,
    rng: &mut gizmo_math::Rng,
    out: &mut Vec<GpuParticle>,
) {
    for _ in 0..emitter.take_burst().min(MAX_BURST) {
        out.push(particle(emitter, origin, rng));
    }
    if emitter.spawn_rate <= 0.0 {
        return;
    }
    emitter.add_time(dt);
    let interval = 1.0 / emitter.spawn_rate;
    let mut this_frame = 0;
    while emitter.get_accumulator() >= interval && this_frame < MAX_SPAWNS_PER_FRAME {
        emitter.consume_time(interval);
        this_frame += 1;
        out.push(particle(emitter, origin, rng));
    }
}

/// One particle as `emitter` makes them, at `origin`.
fn particle(emitter: &ParticleEmitter, origin: Vec3, rng: &mut gizmo_math::Rng) -> GpuParticle {
    let jitter = Vec3::new(
        rng.signed() * emitter.velocity_randomness,
        rng.signed() * emitter.velocity_randomness,
        rng.signed() * emitter.velocity_randomness,
    );
    let velocity = emitter.initial_velocity + jitter;
    let life = (emitter.lifespan + rng.signed() * emitter.lifespan_randomness).max(0.1);
    GpuParticle {
        position: [origin.x, origin.y, origin.z],
        life: 0.0,
        velocity: [velocity.x, velocity.y, velocity.z],
        max_life: life,
        color: emitter.color_start.into(),
        size_start: emitter.size_start,
        size_end: emitter.size_end,
        _padding: [0.0; 2],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-shot emitter puts its whole burst out on its first frame and nothing after; a rate
    /// emitter's burst comes on top of its rate.
    #[test]
    fn a_burst_comes_out_at_once_and_only_once() {
        let mut rng = gizmo_math::Rng::new(1);
        let mut boom = ParticleEmitter::one_shot(250);
        let mut out = Vec::new();
        emit(&mut boom, Vec3::ZERO, 1.0 / 60.0, &mut rng, &mut out);
        assert_eq!(out.len(), 250);
        out.clear();
        emit(&mut boom, Vec3::ZERO, 1.0 / 60.0, &mut rng, &mut out);
        assert!(out.is_empty(), "a one-shot emitter emitted again");

        let mut stream = ParticleEmitter::with_rate(60.0).with_burst(10);
        emit(&mut stream, Vec3::new(1.0, 2.0, 3.0), 1.0 / 60.0, &mut rng, &mut out);
        assert_eq!(out.len(), 11, "ten from the burst and one from a frame at 60 a second");
        assert!(out.iter().all(|p| p.position == [1.0, 2.0, 3.0]));
        out.clear();
        stream.burst(5000);
        emit(&mut stream, Vec3::ZERO, 0.0, &mut rng, &mut out);
        assert_eq!(out.len(), MAX_BURST as usize, "a burst is capped");
    }
}
