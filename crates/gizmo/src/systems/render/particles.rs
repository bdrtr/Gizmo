//! The missing link between `ParticleEmitter` and the GPU particle system.
//!
//! # Why this file exists
//!
//! Everything else on the particle path was already here. The draw path calls `update_params` and
//! `compute_pass` on `renderer.gpu_particles` every frame, and `passes/forward` draws the result.
//! What nothing did was **put anything in it**: the engine simulated and drew a particle set that
//! no scene ever populated, because the step that reads `ParticleEmitter` entities and spawns from
//! them lived only inside `gizmo-studio`'s own render pipeline.
//!
//! So an entity carrying a `ParticleEmitter` emitted nothing, for anyone using the engine's
//! out-of-the-box pass. Same shape as `LodGroup` (visible only to studio) and as skeletal
//! animation (systems written, exported, never scheduled) — see `docs/ENGINE.md`.
//!
//! # Two halves
//!
//! Stepping an emitter writes it — its burst is taken, its timer runs — so that is frame
//! preparation's ([`step_particle_emitters`], from `prepare_frame`), and what it emits is kept as
//! the [`ParticleSpawns`] resource, replaced every frame. The draw uploads what that resource
//! holds and writes nothing back: it used to step the emitters itself, which made it a writer of
//! the world it draws.
//!
//! # Why it is in the facade and not in `gizmo-renderer`
//!
//! It needs both halves and they live in different crates: `GpuParticleSystem` is in
//! `gizmo-renderer`, and `Transform` — which is where an emitter *is* — is in
//! `gizmo-physics-core`, which `gizmo-renderer` does not depend on. This is the lowest crate that
//! can see both. [`emit`] is `pub` because studio calls it instead of keeping its own copy of the
//! rule, so the two paths cannot drift.
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
use crate::renderer::gpu_particles::GpuParticle;
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

/// The particles this frame's emitters put out, for the draw to upload into the GPU particle
/// system. Written whole by [`step_particle_emitters`] every frame, never appended to, and numbered:
/// the draw cannot clear it (it reads the world, it does not write it), so the GPU particle system
/// remembers the last batch it uploaded and skips it when it comes round again — a frame drawn
/// twice, or drawn from a copy of the world that was not refreshed, spawns nothing twice.
#[derive(Clone, Debug)]
pub struct ParticleSpawns {
    batch: u64,
    particles: Vec<GpuParticle>,
}

impl ParticleSpawns {
    fn new(particles: Vec<GpuParticle>) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self { batch: NEXT.fetch_add(1, Ordering::Relaxed), particles }
    }

    /// This frame's new particles, in emitter order.
    #[must_use]
    pub fn particles(&self) -> &[GpuParticle] {
        &self.particles
    }

    /// Which batch this is: new every [`step_particle_emitters`], never 0, unique in the process.
    #[must_use]
    pub fn batch(&self) -> u64 {
        self.batch
    }
}

/// Advance every active emitter by `dt` and keep what it owes as this frame's [`ParticleSpawns`],
/// replacing last frame's.
///
/// Call once per frame, before the draw — `prepare_frame` does. The draw uploads the spawns before
/// its particle compute pass, so they are stepped in the same frame.
pub fn step_particle_emitters(world: &mut World, dt: f32) {
    // Where each emitter is, read before the emitters are borrowed for writing.
    let placed: Vec<(Entity, Option<(Vec3, gizmo_math::Quat)>)> = {
        let transforms = world.query::<&Transform>();
        world
            .query::<(Entity, &ParticleEmitter)>()
            .iter()
            .map(|(entity, _)| (entity, transforms.get(entity).map(|t| (t.position, t.rotation))))
            .collect()
    };
    let mut spawned = Vec::new();
    if !placed.is_empty() {
        // Seeded from the frame so successive frames differ, and mixed with the entity below so
        // two emitters in the same frame do not emit in lockstep.
        let frame = world.get_resource::<gizmo_core::time::Time>().map_or(0, |t| t.frame());
        let mut emitters = world.query_mut::<&mut ParticleEmitter>();
        for (id, place) in placed {
            let Some(mut emitter) = emitters.get_mut(id) else {
                continue;
            };
            if !emitter.is_active {
                continue;
            }
            // An emitter with no transform still emits, at its own local offset. It is a worse
            // answer than a position but a much better one than silence, and it is what a caller
            // who spawned a bare emitter almost certainly meant.
            let origin = match place {
                Some((position, rotation)) => position + rotation.mul_vec3(emitter.local_offset),
                None => emitter.local_offset,
            };
            // One stream per (frame, emitter): the same frame replays the same sparks, and
            // neighbouring emitters do not emit in lockstep.
            let mut rng = gizmo_math::Rng::with_stream(frame, u64::from(id.id()));
            emit(&mut emitter, origin, dt, &mut rng, &mut spawned);
        }
    }
    world.insert_resource(ParticleSpawns::new(spawned));
}

/// What one emitter puts out this frame, appended to `out`: its pending
/// [`burst`](ParticleEmitter::burst) all at once, then what its `spawn_rate` owes for `dt`.
///
/// The one place the emission rule lives, so the game's frame ([`step_particle_emitters`]) and the
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
