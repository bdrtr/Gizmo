//! Physics pipeline — internal sub-steps of `PhysicsWorld::step`.
//!
//! The original monolithic 700-line `step_internal` has been split into
//! focused, individually-traceable functions.  Each function is responsible
//! for exactly one pipeline stage and returns a typed result so the caller
//! can handle errors at the right boundary.

use crate::{
    components::{RigidBody, Velocity},
    world::PhysicsWorld,
};
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_core::{CollisionEvent, CollisionEventType, ContactManifold, ContactPoints, TriggerEvent};
use gizmo_physics_core::narrowphase::NarrowPhase;
use gizmo_physics_core::BodyHandle;
use gizmo_math::Vec3;
#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;
#[cfg(target_arch = "wasm32")]
use crate::parallel_compat::*;
use rustc_hash::{FxHashMap, FxHashSet};

/// Awake dynamic bodies below which a per-body loop runs on the calling thread.
///
/// Handing a loop to the thread pool costs a wake-up and a wait on a latch, and splitting it costs
/// steals and yields across every worker — tens of microseconds, several times a step, whatever the
/// loop holds. With a few hundred bodies asleep and a handful awake that was nearly all of a step:
/// 513 resting boxes on terrain took 1.03 ms a step against the terrain's 0.10
/// (`many_sleeping_bodies`, `nfsu2-gizmo` MOTOR-NOTLARI #26). The result is the same either way:
/// the bodies are independent and the outputs keep their order.
const PARALLEL_MIN_AWAKE: usize = 256;

/// Whether a per-body loop over `bodies` is worth the thread pool — see [`PARALLEL_MIN_AWAKE`].
fn worth_parallel(bodies: &[RigidBody]) -> bool {
    bodies.iter().filter(|rb| !rb.is_sleeping && rb.is_dynamic()).take(PARALLEL_MIN_AWAKE).count() >= PARALLEL_MIN_AWAKE
}

/// Whether a loop over `items` independent jobs is worth the thread pool, given how many it takes
/// to pay for the handoff: a body's integration is a few multiplies (`PARALLEL_MIN_AWAKE`), a pair's
/// contacts a few microseconds (`PARALLEL_MIN_PAIRS`), and the islands' solve is counted in the
/// manifolds they hold (`PARALLEL_MIN_MANIFOLDS`), not in islands — one threshold for all three made
/// settling bodies slower, and an island count let six resting boxes' six islands go to the pool.
fn worth_parallel_jobs(items: usize, at_least: usize) -> bool {
    items >= at_least
}

/// See [`worth_parallel_jobs`].
const PARALLEL_MIN_PAIRS: usize = 64;
/// See [`worth_parallel_jobs`].
const PARALLEL_MIN_MANIFOLDS: usize = 32;


/// The fracture events one manifold's strongest contact produces.
///
/// Only the largest normal impulse in the manifold is considered: a manifold is one collision, and
/// four contact points sharing it should not fracture a body four times over. Each endpoint is
/// tested separately because they can carry different thresholds — a crate hitting a wall breaks
/// the crate.
///
/// Lifted out of `constraint_solve_step`, where it was the deepest code in the crate at nesting
/// level ten. It also carried a vestigial `(entity, entity)` tuple with a `let _ = …; // suppress
/// unused warning` next to it — a second element that was never anything but the first, silenced
/// rather than deleted.
fn collect_fracture_events(
    manifold: &ContactManifold,
    rigid_bodies: &[crate::RigidBody],
    entity_map: &crate::world::EntityIndexMap,
    out: &mut Vec<gizmo_physics_core::FractureEvent>,
) {
    let Some(contact) = manifold
        .contacts
        .iter()
        .max_by(|a, b| a.normal_impulse.partial_cmp(&b.normal_impulse).unwrap())
    else {
        return;
    };

    for entity in [manifold.entity_a, manifold.entity_b] {
        let Some(&idx) = entity_map.get(&entity.id()) else { continue };
        let Some(threshold) = rigid_bodies[idx].fracture_threshold else { continue };
        if contact.normal_impulse > threshold {
            out.push(gizmo_physics_core::FractureEvent {
                entity,
                impact_point: contact.point,
                impact_force: contact.normal_impulse,
            });
        }
    }
}

/// What the narrowphase found for one candidate pair, on the worker that tested it.
struct PairContacts {
    entity_a: BodyHandle,
    entity_b: BodyHandle,
    contacts: Vec<gizmo_physics_core::ContactPoint>,
    is_trigger_a: bool,
    is_trigger_b: bool,
    material_a: gizmo_physics_core::PhysicsMaterial,
    material_b: gizmo_physics_core::PhysicsMaterial,
    /// Warm-start outcome, computed in parallel with the collision maths rather than in the
    /// sequential assembly (`record_solid_contact`): `Some(lifetime)` when this pair persisted
    /// from the last frame, and the impulses in `contacts` have already been carried across. It
    /// is pure per-pair work over immutable state (`contact_cache`, the match tolerance), so it
    /// belongs on the worker that produced the contacts, not on the one thread that has to build
    /// the cache and the event list in order.
    warm: Option<u32>,
}

/// One island's solve, handed back from its worker for the sequential write-back
/// (`PhysicsWorld::write_back_island`).
#[derive(Default)]
struct IslandResult {
    /// The dynamic members' solved velocities.
    velocities: Vec<(BodyHandle, Velocity)>,
    /// The solved manifolds: the warm-start data, and the collision events' points.
    manifolds: Vec<ContactManifold>,
    /// The sleeping members a real mover in the island wakes.
    wake_ups: Vec<BodyHandle>,
    fractures: Vec<gizmo_physics_core::FractureEvent>,
    /// Split-impulse position corrections (Δlinear, Δscaled-axis).
    position_corrections: Vec<(BodyHandle, Vec3, Vec3)>,
    /// Speculative contacts' bounces, for after position integration.
    bounces: Vec<(BodyHandle, Vec3, Vec3)>,
    /// The adaptive sweep count and the support depth.
    stats: crate::solver::SolveStats,
    /// Every dynamic member is eligible to sleep.
    can_sleep: bool,
    /// What contacts gave kinematic bodies with a contact mass.
    kinematic_responses: Vec<(usize, Vec3)>,
}
impl PhysicsWorld {
    // ================================================================== //
    //  Stage 0 — Velocity integration                                     //
    // ================================================================== //

    /// Apply gravity fields, fluid buoyancy/drag, and integrate velocities
    /// for every non-sleeping rigid body (parallel).
    #[tracing::instrument(skip_all, name = "velocity_integration")]
    pub(crate) fn velocity_integration_step(
        &mut self,
        dt: f32,
    ) -> Result<(), gizmo_physics_core::GizmoError> {
        let default_gravity = self.integrator.gravity;
        let gravity_fields = &self.gravity_fields;
        let fluid_zones = &self.fluid_zones;
        let watchlist = &self.watchlist;

        // Over every body, in parallel when there is enough awake to be worth it (`worth_parallel`).
        // `try_for_each` short-circuits on the first `Err` and propagates it.
        let parallel = worth_parallel(&self.rigid_bodies);
        let integrate =
                |((((&entity, rb), transform), vel), collider): ((((&BodyHandle, &mut RigidBody), &Transform), &mut Velocity), &Collider)| -> Result<(), gizmo_physics_core::GizmoError> {
                    if rb.is_sleeping {
                        return Ok(());
                    }

                    // ── Cache pre-velocity for Heun's method (Diferansiyel hesap) ──
                    vel.pre_linear = vel.linear;
                    vel.pre_angular = vel.angular;

                    let pos = transform.position;

                    // ── Gravity field resolution (highest priority wins) ──
                    let active_gravity = gravity_fields
                        .iter()
                        .filter(|f| f.shape.contains(pos))
                        .max_by_key(|f| f.priority)
                        .map_or(default_gravity, |f| f.gravity);

                    // ── Fluid buoyancy & drag ─────────────────────────────
                    // Summed over cells of the body, each at its own point: see
                    // `FluidZone::push_on`. The torque is what rights a tipped hull.
                    if !fluid_zones.is_empty() {
                        let com = pos + transform.rotation * rb.center_of_mass;
                        let mut force = gizmo_math::Vec3::ZERO;
                        let mut torque = gizmo_math::Vec3::ZERO;
                        for zone in fluid_zones {
                            if let Some(push) = zone.push_on(
                                collider,
                                pos,
                                transform.rotation,
                                com,
                                vel.linear,
                                vel.angular,
                                active_gravity,
                            ) {
                                force += push.force;
                                torque += push.torque;
                            }
                        }
                        vel.linear += force * rb.inv_mass() * dt;
                        vel.angular += rb.inv_world_inertia_tensor(transform.rotation) * torque * dt;
                    }

                    // ── Velocity integration ──────────────────────────────
                    let local_integrator = crate::integrator::Integrator {
                        gravity: active_gravity,
                        air_density: self.integrator.air_density,
                        wind: self.integrator.wind,
                    };

                    local_integrator.integrate_velocities(entity, rb, transform.rotation, vel, dt)?;

                    // ── Watchlist debug logging ───────────────────────────
                    if !watchlist.is_empty() && watchlist.contains(&entity) {
                        tracing::debug!(
                            "WATCHLIST [{:?}]: pos={:.3?}  lin_vel={:.3?}",
                            entity, transform.position, vel.linear,
                        );
                    }

                    Ok(())
                };
        let result = if parallel {
            self.entities
                .par_iter()
                .zip(self.rigid_bodies.par_iter_mut())
                .zip(self.transforms.par_iter())
                .zip(self.velocities.par_iter_mut())
                .zip(self.colliders.par_iter())
                .try_for_each(integrate)
        } else {
            self.entities
                .iter()
                .zip(self.rigid_bodies.iter_mut())
                .zip(self.transforms.iter())
                .zip(self.velocities.iter_mut())
                .zip(self.colliders.iter())
                .try_for_each(&integrate)
        };
        result
            .inspect_err(|e| {
                tracing::error!("Velocity integration failed: {e:?}");
                if let Err(snap_err) =
                    self.trigger_snapshot("Velocity Integration Error (NaN/Overflow)")
                {
                    tracing::error!("Failed to write physics snapshot: {snap_err}");
                }
            })
    }



    // ================================================================== //
    //  Stage 2 — Broadphase                                               //
    // ================================================================== //

    /// Refit the spatial hash to this substep's bounds (swept over the step for a CCD body) and
    /// evict the bodies the world no longer holds. Only rigid bodies are in it.
    #[tracing::instrument(skip_all, name = "broadphase")]
    pub(crate) fn broadphase_step(&mut self, dt: f32) {
        // The tree is KEPT across substeps rather than cleared and refilled.
        //
        // It is a fat-margin dynamic AABB tree, so `insert` already early-outs for a body that
        // has not left the box stored for it — which is most bodies on most substeps, and all of
        // them in a sleeping pile. Clearing first threw that away and paid a full descent, node
        // allocation and refit for every body, four times per frame. Measured on this machine:
        // broadphase_no_contacts/1024 1.78 ms -> 574 us (-67.6%), full_step_mixed/512
        // 2.53 ms -> 1.65 ms (-34.9%).
        //
        // Holding it means the tree can outlive bodies, so the loop below is followed by a
        // reconciliation that evicts ids the world no longer has. `docs/ENGINE.md` records
        // pair-emission invariance — `support_ordering`, on by default — as the property that
        // makes this safe: the island solve does not depend on the order pairs come out in, and
        // a tree built incrementally emits them in a different order than a freshly rebuilt one.

        // Rigid bodies — sequential because `spatial_hash` needs `&mut self`.
        self.settled_bounds.resize(self.entities.len(), None);
        for i in 0..self.entities.len() {
            let entity = self.entities[i];
            let rb = &self.rigid_bodies[i];
            let transform = &self.transforms[i];
            let vel = &self.velocities[i];
            let collider = &self.colliders[i];

            // Static or asleep, and where it was when its bounds were last refreshed this step:
            // nothing to do (see `settled_bounds`). The id is compared too, so a body added or
            // removed mid-step — which moves others to new indices — is refreshed, not trusted.
            let settled = rb.is_static() || rb.is_sleeping;
            let pose = (entity.id(), transform.position, transform.rotation);
            if settled && self.settled_bounds[i] == Some(pose) {
                // What the skip relies on, checked wherever tests run (debug): the refresh it
                // saves would have found the body inside its box and changed nothing.
                debug_assert!(
                    self.spatial_hash.holds(entity, &collider.compute_aabb(transform.position, transform.rotation)),
                    "body {} skipped by the broadphase is not inside the box the tree holds for it",
                    entity.id()
                );
                continue;
            }
            self.settled_bounds[i] = settled.then_some(pose);

            let base_aabb = collider.compute_aabb(transform.position, transform.rotation);

            // For CCD bodies sweep the AABB over the full expected travel this frame.
            // `!is_static()` covers both dynamic AND kinematic movers — new_kinematic()
            // turns CCD on by default, so a fast scripted platform/blade must be swept
            // too (testing is_dynamic() here silently dropped every kinematic CCD body).
            let aabb = if rb.ccd_enabled && !rb.is_static() && !rb.is_sleeping {
                let sweep_dt = if vel.linear.length() > 100.0 {
                    // High-speed: guarantee at least one full 60 Hz frame is covered.
                    dt.max(1.0 / 60.0)
                } else {
                    dt
                };
                let swept_pos = transform.position + vel.linear * sweep_dt;
                let swept_aabb = collider.compute_aabb(swept_pos, transform.rotation);
                base_aabb.merge(swept_aabb)
            } else {
                base_aabb
            };

            if rb.is_static() {
                self.spatial_hash.insert_static(entity, aabb);
            } else {
                self.spatial_hash.insert(entity, aabb);
            }
        }

        // Evict bodies the world no longer has. Skipped on the common path, where the counts
        // agree because nothing was added or removed — and when they disagree only because
        // bodies were ADDED, the loop above has already inserted them, so the counts agree again
        // by the time we look.
        if self.spatial_hash.entity_count() != self.entities.len() {
            let live = &self.entity_index_map;
            self.spatial_hash.retain(|id| live.contains_key(&id));
        }

        // Soft bodies — insert bounding AABB over all nodes.
    }

    // ================================================================== //
    //  Stage 3 — Narrowphase & collision events                           //
    // ================================================================== //

    /// Detect actual collisions (parallel narrowphase) and emit the collision and trigger events.
    ///
    /// Returns the set of [`ContactManifold`]s to be fed into the solver.
    #[tracing::instrument(skip_all, name = "narrowphase_collision")]
    pub(crate) fn narrowphase_and_collision_step(&mut self, dt: f32) -> Vec<ContactManifold> {
        let (active_pairs, dormant_pairs) = self.partition_pairs();
        let disabled_joint_pairs = self.collision_disabled_joint_pairs();

        // Kapsam paralel bölümün TAMAMINI sarar, çift başına değil. Eskiden çift başınaydı:
        // kare başına ~7300 `Instant::now()` + paylaşılan tek atomiğe `fetch_add`, hem
        // `profile.rs`'in kendi sözleşmesini ("asla temas ya da cisim başına") ihlal ediyor hem de
        // ölçtüğü şeye maliyet ekliyordu. Ölçüldü: dar fazın %3'ü. Küçük, ama gönderilen bir
        // motorda kimsenin okumadığı bir sayaç için kalıcı bir vergi, ve buradan okunan sayı
        // (duvar saati) zaten daha kullanışlı — CPU-ms yerine fazın gerçekte ne kadar sürdüğü.
        let _t = crate::profile::Scope::new(&crate::profile::PHASES.dispatch);
        let pair_contacts = |&pair: &(BodyHandle, BodyHandle)| self.pair_contacts(pair, &disabled_joint_pairs, dt);
        // On the calling thread when there are few pairs (`worth_parallel_jobs`); order is kept.
        let narrowphase_results: Vec<PairContacts> = if worth_parallel_jobs(active_pairs.len(), PARALLEL_MIN_PAIRS) {
            active_pairs.par_iter().filter_map(pair_contacts).collect()
        } else {
            active_pairs.iter().filter_map(pair_contacts).collect()
        };

        // ── Contacts of bodies removed since the last step: ended ─────────
        // Reported first, before this step's own events: the body that took the id may be in the
        // same pair this very step, and its `Started` must come after the dead body's `Ended`.
        for (pair, is_trigger) in std::mem::take(&mut self.departed_contacts) {
            self.end_contact(pair, is_trigger);
        }

        // ── Sequential post-processing ────────────────────────────────────
        // Pre-size to last frame's counts: the contact set is frame-coherent, so this skips the
        // ~log2(N) grow-and-rehash reallocations the map/vec otherwise do every frame while
        // filling to tens of thousands of entries.
        let mut manifolds = Vec::with_capacity(narrowphase_results.len());
        let mut current_cache =
            FxHashMap::with_capacity_and_hasher(self.contact_cache.len(), Default::default());
        let _mani = crate::profile::Scope::new(&crate::profile::PHASES.manifold);
        for found in narrowphase_results {
            if found.is_trigger_a || found.is_trigger_b {
                self.record_trigger(&found, &mut current_cache);
            } else {
                manifolds.push(self.record_solid_contact(found, &mut current_cache));
            }
        }

        self.keep_dormant_cache(&dormant_pairs, &mut current_cache);

        // ── Ended collisions ──────────────────────────────────────────────
        // What the last step's cache holds and this step's does not.
        let previous = std::mem::replace(&mut self.contact_cache, current_cache);
        for (pair, &(is_trigger, _)) in &previous {
            if !self.contact_cache.contains_key(pair) {
                self.end_contact(*pair, is_trigger);
            }
        }

        // AGGREGATE: narrowphase outcome for this substep. `dormant_skipped` shows how many
        // candidate pairs the broad-scene dormant-pair optimisation dropped before the
        // expensive SAT/GJK test ran, so a sudden collapse in that ratio (everything active)
        // explains a narrowphase cost spike. Counts are O(1) reads.
        tracing::debug!(
            candidate_pairs = active_pairs.len() + dormant_pairs.len(),
            active_pairs = active_pairs.len(),
            dormant_skipped = dormant_pairs.len(),
            manifold_count = manifolds.len(),
            "narrowphase complete"
        );

        manifolds
    }

    /// This step's candidate pairs, split into the ones the narrowphase tests (in id order) and
    /// the dormant ones it skips.
    fn partition_pairs(&self) -> (Vec<(BodyHandle, BodyHandle)>, Vec<(BodyHandle, BodyHandle)>) {
        let entity_map = &self.entity_index_map;
        let potential_pairs = self.spatial_hash.query_pairs();

        // ── Dormant-çift atlama (geniş-sahne perf) ────────────────────────
        // İki cisim de DORMANT ise (statik VEYA uyuyan dinamik VEYA hareketsiz kinematik)
        // yeni temas ÜRETEMEZ → pahalı narrowphase ATLANIR. (Not: eski "~%82" ölçümü
        // dormant-skip/pre-size/FxHash/paralel-narrowphase ÖNCESİNE ait; ölçüldü 2026-07-14 →
        // narrowphase artık geniş sahnede ~%34, SAT compute alt-dilimi frame'in ~%3'ü. Bu yüzden
        // box-box SAT batch-SoA SIMD DÜŞÜRÜLDÜ; bkz. docs/ENGINE.md §7.)
        // Cache aşağıda KORUNUR (yoksa ended-
        // collision wake sahte tetiklenir). En az biri aktifse normal narrowphase çalışır
        // (temas + wake yakalanır), böylece düşen/itilen cisim uyuyan komşuyu uyandırır.
        let is_active_body = |e: BodyHandle| -> bool {
            match entity_map.get(&e.id()) {
                Some(&i) => {
                    let rb = &self.rigid_bodies[i];
                    (rb.is_dynamic() && !rb.is_sleeping)
                        || (rb.is_kinematic()
                            && (self.velocities[i].linear.length_squared() > 1e-8
                                || self.velocities[i].angular.length_squared() > 1e-8))
                }
                // Not a body this world holds: tested, and `pair_contacts` drops it.
                None => true,
            }
        };
        let mut dormant_pairs: Vec<(BodyHandle, BodyHandle)> = Vec::new();
        let mut active_pairs: Vec<(BodyHandle, BodyHandle)> = potential_pairs
            .into_iter()
            .filter(|&(a, b)| {
                if !is_active_body(a) && !is_active_body(b) {
                    dormant_pairs.push((a, b));
                    false
                } else {
                    true
                }
            })
            .collect();
        // **In id order, not the tree's.** The broadphase hands its pairs over in whatever order
        // its trees' shapes walk them, and that order became the collision events' order — which
        // what reacts to them (the fracture system spawning debris, a game's own handler) turns
        // into entity ids and onward into a different simulation. The island solve does not care
        // (`support_ordering`), the events did: splitting the broadphase into a static and a
        // moving tree changed which blocks `vinc`'s wrecking ball broke, and a rollback, which
        // restores bodies but not the tree's history, could replay a step's events in another
        // order. Sorted, they are a function of the bodies alone.
        active_pairs.sort_unstable_by_key(|&(a, b)| (a.id(), b.id()));
        (active_pairs, dormant_pairs)
    }

    /// The pairs of bodies joined by a collision-disabled joint, by ordered id pair.
    ///
    /// frame (keyed by ordered id pair), instead of re-scanning every joint for every
    /// candidate contact pair inside the parallel narrowphase — that was O(pairs × joints).
    /// Empty (the common jointless case) ⇒ the per-pair check below short-circuits for free.
    fn collision_disabled_joint_pairs(&self) -> FxHashSet<(u32, u32)> {
        self
            .joints
            .iter()
            .filter(|j| !j.collision_enabled)
            .map(|j| {
                let (a, b) = (j.entity_a.id(), j.entity_b.id());
                (a.min(b), a.max(b))
            })
            .collect()
    }

    /// The narrowphase for one candidate pair, on whichever worker drew it: its contacts (with a
    /// speculative one for a CCD body), the one-way surfaces' say, and the warm start. `None` for
    /// a pair that cannot collide or does not touch, and for one with a body the rigid world does
    /// not hold (the broadphase holds rigid bodies only).
    fn pair_contacts(
        &self,
        (entity_a, entity_b): (BodyHandle, BodyHandle),
        disabled_joint_pairs: &FxHashSet<(u32, u32)>,
        dt: f32,
    ) -> Option<PairContacts> {
        let entity_map = &self.entity_index_map;
        let idx_a = *entity_map.get(&entity_a.id())?;
        let idx_b = *entity_map.get(&entity_b.id())?;

        let collider_a = &self.colliders[idx_a];
        let collider_b = &self.colliders[idx_b];

        if !collider_a
            .collision_layer
            .can_collide_with(&collider_b.collision_layer)
        {
            return None;
        }

        // Entities connected by a joint with collision_enabled == false:
        // O(1) lookup in the precomputed set (empty ⇒ no work).
        let has_disabled_joint = !disabled_joint_pairs.is_empty() && {
            let (a, b) = (entity_a.id(), entity_b.id());
            disabled_joint_pairs.contains(&(a.min(b), a.max(b)))
        };

        if has_disabled_joint {
            return None;
        }

        let transform_a = &self.transforms[idx_a];
        let transform_b = &self.transforms[idx_b];

        let mut contacts = NarrowPhase::test_collision_manifold(
            &collider_a.shape,
            transform_a.position,
            transform_a.rotation,
            &collider_b.shape,
            transform_b.position,
            transform_b.rotation,
        );

        // Speculative contacts for CCD bodies.
        if contacts.is_empty() {
            let rb_a = &self.rigid_bodies[idx_a];
            let rb_b = &self.rigid_bodies[idx_b];

            if rb_a.ccd_enabled || rb_b.ccd_enabled {
                let vel_a = &self.velocities[idx_a];
                let vel_b = &self.velocities[idx_b];

                if let Some(sc) = gizmo_physics_core::Gjk::speculative_contact(
                    &collider_a.shape,
                    transform_a.position,
                    transform_a.rotation,
                    vel_a.linear,
                    rb_a.inv_mass(),
                    &collider_b.shape,
                    transform_b.position,
                    transform_b.rotation,
                    vel_b.linear,
                    rb_b.inv_mass(),
                    dt,
                ) {
                    contacts.push(sc);
                }
            }
        }

        // One-way surfaces: drop the contacts a platform should not stop. Only
        // pairs with a one-way collider pay anything here.
        if collider_a.one_way.is_some() || collider_b.one_way.is_some() {
            let vel_a = self.velocities[idx_a].linear;
            let vel_b = self.velocities[idx_b].linear;
            if let Some(up) = collider_a.one_way {
                let up = transform_a.rotation * up;
                contacts.retain(|c| gizmo_physics_core::Collider::one_way_keeps(up, c.normal, vel_b - vel_a));
            }
            if let Some(up) = collider_b.one_way {
                // B is the platform: flip the pair so the rule sees it as A.
                let up = transform_b.rotation * up;
                contacts.retain(|c| gizmo_physics_core::Collider::one_way_keeps(up, -c.normal, vel_a - vel_b));
            }
        }

        if contacts.is_empty() {
            return None;
        }

        // Warm-start here, on the worker. Triggers carry no impulses, so the same
        // condition the sequential loop used still gates it.
        let mut warm = None;
        if !collider_a.is_trigger && !collider_b.is_trigger {
            if let Some((_, Some(old_manifold))) =
                self.contact_cache.get(&(entity_a, entity_b))
            {
                warm = Some(old_manifold.lifetime + 1);
                let ws_tol_sq =
                    self.solver.warm_start_match_tolerance.powi(2);
                for contact in contacts.iter_mut() {
                    if let Some(old) = old_manifold.contacts.iter().find(|o| {
                        (o.point - contact.point).length_squared() < ws_tol_sq
                    }) {
                        contact.normal_impulse = old.normal_impulse;
                        contact.tangent_impulse = old.tangent_impulse;
                    }
                }
            }
        }

        Some(PairContacts {
            entity_a,
            entity_b,
            contacts,
            is_trigger_a: collider_a.is_trigger,
            is_trigger_b: collider_b.is_trigger,
            material_a: collider_a.material,
            material_b: collider_b.material,
            warm,
        })
    }

    /// A contact that ended — its pair stopped touching, or one of its bodies is gone. Both
    /// bodies wake, so neither is left frozen mid-air after losing its support, and the `Ended`
    /// event goes out.
    fn end_contact(&mut self, pair: (BodyHandle, BodyHandle), is_trigger: bool) {
        for entity in [pair.0, pair.1] {
            if let Some(&idx) = self.entity_index_map.get(&entity.id()) {
                if self.rigid_bodies[idx].is_dynamic() {
                    self.rigid_bodies[idx].wake_up();
                }
            }
        }
        if is_trigger {
            self.trigger_events.push(TriggerEvent { trigger_entity: pair.0, other_entity: pair.1, event_type: CollisionEventType::Ended });
        } else {
            self.collision_events.push(CollisionEvent {
                entity_a: pair.0,
                entity_b: pair.1,
                event_type: CollisionEventType::Ended,
                contact_points: ContactPoints::new(),
            });
        }
    }

    /// A trigger overlap: cached without a manifold, and its event.
    fn record_trigger(&mut self, found: &PairContacts, current_cache: &mut FxHashMap<(BodyHandle, BodyHandle), (bool, Option<ContactManifold>)>) {
        let PairContacts { entity_a, entity_b, is_trigger_a, .. } = *found;
        let pair = (entity_a, entity_b);
        // ── Trigger ───────────────────────────────────────────────
        current_cache.insert(pair, (true, None));

        let event_type = if self.contact_cache.contains_key(&pair) {
            CollisionEventType::Persisting
        } else {
            CollisionEventType::Started
        };
        self.trigger_events.push(TriggerEvent {
            trigger_entity: if is_trigger_a { entity_a } else { entity_b },
            other_entity: if is_trigger_a { entity_b } else { entity_a },
            event_type,
        });
    }

    /// A solid contact: its manifold with the pair's combined material and warm start, the cache's
    /// entry for the next step, and its event.
    fn record_solid_contact(&mut self, found: PairContacts, current_cache: &mut FxHashMap<(BodyHandle, BodyHandle), (bool, Option<ContactManifold>)>) -> ContactManifold {
        let PairContacts { entity_a, entity_b, contacts, material_a: mat_a, material_b: mat_b, warm, .. } = found;
        let pair = (entity_a, entity_b);
        // ── Solid contact ─────────────────────────────────────────
        let mut manifold = ContactManifold::new(entity_a, entity_b);
        // Combine the two materials via their declared combine modes
        // (`PhysicsMaterial::combine`) instead of hard-coding geometric-mean
        // friction + max restitution. The hard-coded form silently ignored
        // each material's `friction_combine`/`restitution_combine`, so presets
        // like ICE (Min) and RUBBER (Max) never combined as specified. For the
        // default material (GeometricMean friction, Max restitution) this is
        // identical, so default-material scenes are unaffected.
        let combined = gizmo_physics_core::PhysicsMaterial::combine(&mat_a, &mat_b);
        manifold.friction = combined.dynamic_friction;
        manifold.static_friction = combined.static_friction;
        manifold.restitution = combined.restitution;
        manifold.rolling_friction = combined.rolling_friction;

        // Snapshot the event's points from the RAW narrowphase output, BEFORE the
        // warm-start pass below. This used to be taken at the end, and was pristine for
        // free: the loop copied each point out with `iter().copied()` and pushed the
        // modified COPY into the manifold, never touching `contacts`. Now that the buffer
        // is moved into the manifold and warm-started in place, taking the snapshot late
        // would start shipping accumulated impulses in `CollisionEvent.contact_points`.
        let event_points: ContactPoints = contacts.iter().copied().take(4).collect();

        // Warm-start: the impulses were already carried across on the worker that found
        // these contacts (see `NpResult`'s last field). What is left is the part that
        // touches the manifold rather than the contacts.
        if let Some(lifetime) = warm {
            manifold.lifetime = lifetime;
            // Persisting contact ⇒ resting / stacking, not a fresh impact.
            // Restitution is only physically meaningful on the FIRST frame of
            // contact; re-applying it every frame (and, at 240 Hz, on each of
            // the 4 sub-steps) keeps pumping energy into a settled stack.
            // Suppress it once a contact has persisted so stacks can settle.
            manifold.restitution = 0.0;
        }

        // The narrowphase's buffer IS the manifold's buffer — moved, not copied. This is
        // one of the per-pair-per-substep heap allocations docs/AUDIT-2026-08.md counted
        // in the contact path: `ContactManifold::new` used to malloc a second Vec and this
        // arm memcpy'd the first one into it. Owning the Vec removes the allocation; no
        // inline container is involved, because the narrowphase's return type is public
        // and its count is deliberately uncapped.
        manifold.contacts = contacts;

        // **Kalıcı tampon.** Bu çiftin bir önceki girdisi hâlâ elimizde; onun
        // `Vec<ContactPoint>`'ini geri kullanmak, `manifold.clone()`'un her
        // alt-adımda yaptığı tahsisi kalıcı durumda tamamen ortadan kaldırır —
        // aynı çiftler devam ettiği sürece tahsisatçıya hiç inilmez. Kopyalanan
        // veri birebir aynı; değişen yalnız belleğin nereden geldiği.
        //
        // `remove` ÖNCE yapılamaz: aşağıdaki olay türü kararı önbellekte girdi
        // olup olmadığına bakıyor, o yüzden varlık burada yakalanır.
        let existed = self.contact_cache.contains_key(&pair);
        let mut cached = match self.contact_cache.remove(&pair) {
            Some((_, Some(mut old))) => {
                old.contacts.clear();
                old.contacts.extend_from_slice(&manifold.contacts);
                old
            }
            _ => manifold.clone(),
        };
        cached.entity_a = manifold.entity_a;
        cached.entity_b = manifold.entity_b;
        cached.friction = manifold.friction;
        cached.static_friction = manifold.static_friction;
        cached.restitution = manifold.restitution;
        cached.rolling_friction = manifold.rolling_friction;
        cached.lifetime = manifold.lifetime;
        current_cache.insert(pair, (false, Some(cached)));

        let event_type = if existed {
            CollisionEventType::Persisting
        } else {
            CollisionEventType::Started
        };
        self.collision_events.push(CollisionEvent {
            entity_a,
            entity_b,
            event_type,
            contact_points: event_points,
        });

        manifold
    }

    /// Keeps the dormant pairs' cache entries. The narrowphase skipped them (both bodies are
    /// dormant), so their previous entries move to `current_cache`; otherwise the ended-collision
    /// loop would take them for ended and WAKE them. No manifold goes to the solver (both bodies
    /// are dormant, so the island is not solved anyway): this is the contact cache's continuity
    /// only. Once a body wakes, the pair is active again and the narrowphase tests it.
    fn keep_dormant_cache(&mut self, dormant_pairs: &[(BodyHandle, BodyHandle)], current_cache: &mut FxHashMap<(BodyHandle, BodyHandle), (bool, Option<ContactManifold>)>) {
        for &pair in dormant_pairs {
            if current_cache.contains_key(&pair) {
                continue;
            }
            // **Taşı, kopyalama.** Girdi buradan değiştirilmeden ileri geçiyor ve
            // `self.contact_cache` birkaç satır aşağıda tümden `current_cache` ile
            // değiştiriliyor — yani kopyanın ömrü zaten bu blok kadar. `ContactManifold`
            // bir `Vec<ContactPoint>` sahibi olduğundan her `clone()` bir heap tahsisiydi:
            // dormant çift başına, ALT-ADIM başına. Oturmuş bir yığında çiftlerin neredeyse
            // tamamı dormant olduğu için temas yolundaki tahsislerin çoğu buradan geliyordu
            // (ölçüm: 8.160/kare, temassız taban çizgisi 20/kare).
            //
            // Aşağıdaki "biten çarpışma" döngüsü `self.contact_cache`'i tarayıp
            // `current_cache`'te olmayanları "bitti" sayar; buradan çıkardığımız her girdi
            // aynı anda `current_cache`'e girdiğinden o döngü onları zaten atlıyordu.
            // Kaldırmak sonucu değiştirmez.
            if let Some(entry) = self.contact_cache.remove(&pair) {
                current_cache.insert(pair, entry);
            } else if let Some(entry) = self.contact_cache.remove(&(pair.1, pair.0)) {
                current_cache.insert((pair.1, pair.0), entry);
            }
        }
    }

    // ================================================================== //
    //  Stage 4–4.5 — Constraint solver                                    //
    // ================================================================== //

    /// Solves the contacts (island by island, in parallel when it pays) and the explicit joints
    /// (hinges, springs, …) of the substep. Returns the bounces of speculative contacts
    /// (`solve_contacts`' `bounces`), which the step adds after integrating positions.
    #[tracing::instrument(skip_all, name = "constraint_solve")]
    pub(crate) fn constraint_solve_step(&mut self, manifolds: Vec<ContactManifold>, dt: f32) -> Vec<(BodyHandle, Vec3, Vec3)> {
        let mut bounces: Vec<(BodyHandle, Vec3, Vec3)> = Vec::new();

        // Dynamic bodies the island sleep rule got to decide about this substep. Declared out
        // here because the sweep for everything else has to run even when there are no contacts
        // at all — a pair of bodies joined only by a joint has no island and must still be able
        // to sleep.
        let mut in_island: FxHashSet<u32> = FxHashSet::default();

        // ── Collision constraints ─────────────────────────────────────────
        if !manifolds.is_empty() {
            self.solve_islands(manifolds, dt, &mut in_island, &mut bounces);
        }

        // ── Joints ────────────────────────────────────────────────────────
        if !self.joints.is_empty() {
            self.wake_joint_components();
            self.joint_solver.solve_joints(
                &mut self.joints,
                &self.entity_index_map,
                &self.rigid_bodies,
                &self.transforms,
                &mut self.velocities,
                dt,
            );

            // A joint that snapped this substep (exceeded its break force/torque) is a
            // structural change to the scene — surface how many are now broken so a
            // collapsing mechanism is traceable without stepping through the solver.
            let broken_joints = self.joints.iter().filter(|j| j.is_broken).count();
            tracing::debug!(
                joint_count = self.joints.len(),
                broken_joints,
                "joints solved"
            );
        }

        // ── Sleep, for everything the island rule did not decide ──────────
        // A dynamic body with no contacts this substep belongs to no island, so the island rule
        // never reaches it. It sleeps on its own counter, exactly as it always did: the
        // inconsistency that rule exists to prevent can only arise through a contact, and a pair
        // of bodies joined only by a joint has to stay able to settle.
        //
        // After the joint pass on purpose. A joint-coupled wake calls `wake_up`, which zeroes the
        // counter, so a body woken there is not eligible and cannot be put straight back to sleep
        // in the same substep.
        for i in 0..self.rigid_bodies.len() {
            let id = self.entities[i].id();
            let rb = &mut self.rigid_bodies[i];
            if rb.is_dynamic() && !rb.is_sleeping && rb.sleep_eligible() && !in_island.contains(&id) {
                rb.is_sleeping = true;
            }
        }

        // Sync pre-velocities with the solver-corrected velocities so that Heun's method
        // integrates the corrected state without uncorrected gravity/force drift.
        for vel in &mut self.velocities {
            vel.pre_linear = vel.linear;
            vel.pre_angular = vel.angular;
        }
        bounces
    }

    /// The contacts of the substep, island by island, and the sequential write-back of what each
    /// island's solve found (`write_back_island`). `in_island` collects the dynamic bodies the
    /// island sleep rule decided about; `bounces`, the speculative contacts' bounces.
    fn solve_islands(
        &mut self,
        manifolds: Vec<ContactManifold>,
        dt: f32,
        in_island: &mut FxHashSet<u32>,
        bounces: &mut Vec<(BodyHandle, Vec3, Vec3)>,
    ) {
        let entity_map = &self.entity_index_map;
        let is_dynamic = |entity: BodyHandle| -> bool {
            entity_map
                .get(&entity.id())
                .is_some_and(|&idx| self.rigid_bodies[idx].is_dynamic())
        };

        let islands = crate::island::IslandManager::build_islands(&manifolds, &is_dynamic);
        let island_count = islands.len();
        let island_groups = crate::island::IslandManager::split_manifolds(manifolds, &islands);

        let solve_island = |island_manifolds: Vec<ContactManifold>| self.solve_island(island_manifolds, dt);
        // On the calling thread when there are few islands (`worth_parallel_jobs`); order is kept.
        let results: Vec<IslandResult> = if island_groups.len() > 1
            && worth_parallel_jobs(island_groups.iter().map(Vec::len).sum(), PARALLEL_MIN_MANIFOLDS)
        {
            island_groups.into_par_iter().map(solve_island).collect()
        } else {
            island_groups.into_iter().map(solve_island).collect()
        };

        // Write-back: velocities, wake-ups, fractures, warm-start data.
        // Build a lookup from entity pair → event index for O(1) updates.
        let event_index: FxHashMap<(BodyHandle, BodyHandle), usize> = self
            .collision_events
            .iter()
            .enumerate()
            .map(|(i, e)| ((e.entity_a, e.entity_b), i))
            .collect();

        let mut total_wakes = 0usize;
        let mut total_fractures = 0usize;
        for result in results {
            let (wakes, fractures) = self.write_back_island(result, &event_index, in_island, bounces);
            total_wakes += wakes;
            total_fractures += fractures;
        }

        // AGGREGATE: island solve outcome for this substep. island_count feeds the
        // per-frame metrics; bodies_woken / fractures surface the two behaviours that
        // most often explain "why did that pile suddenly move / shatter".
        self.metrics.island_count += island_count;
        tracing::debug!(
            island_count,
            bodies_woken = total_wakes,
            fractures = total_fractures,
            "islands solved"
        );
    }

    /// One island's solve, on whichever worker drew it: whether it is solved at all and whether it
    /// wakes its sleepers, the contact solve itself (`solve_island_contacts`), the fractures it
    /// caused, and whether the island as a whole may sleep.
    fn solve_island(&self, mut island_manifolds: Vec<ContactManifold>, dt: f32) -> IslandResult {
        let entity_map = &self.entity_index_map;
        let rigid_bodies = &self.rigid_bodies;
        let velocities = &self.velocities;
        // Adayı bir "mover" içeriyorsa uyanık say: uyanık bir dinamik gövde
        // VEYA hareket eden bir kinematik gövde. (Eskiden yalnızca dinamik-uyanık
        // sayılıyordu; hareket eden bir kinematik platformun üstündeki uyuyan
        // dinamik cisim hiç uyandırılmıyor, içinden geçiliyordu.) Mover varsa
        // aşağıdaki wake_updates döngüsü adadaki uyuyan dinamikleri uyandırır.
        // İki AYRI kavram:
        //  • island_active — bu adayı ÇÖZ müyüz? Herhangi uyanık dinamik VEYA
        //    hareketli kinematik varsa (yerleşmekte olan ada çözülmeye devam eder).
        //  • island_has_mover — uyuyan üyeleri UYANDIRIR mıyız? Yalnızca GERÇEK bir
        //    hareketli (uyku eşiğinin ÜSTÜNDE hızlı dinamik VEYA hareketli kinematik)
        //    varsa. Eskiden bu ayrım yoktu: uyanık-ama-yerleşen bir komşu (eşik altı)
        //    "awake" sayılıp uyuyan komşusunu geri uyandırıyordu → ada ASLA topluca
        //    uyuyamıyordu (ping-pong; |v|=0 olsa bile). Bu ada-uyumsuzluğu bug'ı.
        let kinematic_moving = |i: usize| -> bool {
            rigid_bodies[i].is_kinematic()
                && (velocities[i].linear.length_squared() > 1e-8
                    || velocities[i].angular.length_squared() > 1e-8)
        };
        let island_active = island_manifolds.iter().any(|m| {
            [m.entity_a, m.entity_b].iter().any(|&e| {
                entity_map.get(&e.id()).is_some_and(|&i| {
                    (rigid_bodies[i].is_dynamic() && !rigid_bodies[i].is_sleeping)
                        || kinematic_moving(i)
                })
            })
        });

        if !island_active {
            return IslandResult { manifolds: island_manifolds, ..IslandResult::default() };
        }

        // Gerçek hareketli: eşik üstü hızlı uyanık dinamik VEYA hareketli kinematik.
        let island_has_mover = island_manifolds.iter().any(|m| {
            [m.entity_a, m.entity_b].iter().any(|&e| {
                entity_map.get(&e.id()).is_some_and(|&i| {
                    let rb = &rigid_bodies[i];
                    (rb.is_dynamic()
                        && !rb.is_sleeping
                        && !rb.can_sleep(&velocities[i]))
                        || kinematic_moving(i)
                })
            })
        });

        // Collect island indices and bodies that need waking.
        let mut island_indices: FxHashSet<usize> = FxHashSet::default();
        let mut wake_updates = Vec::new();

        for m in &island_manifolds {
            for &e in &[m.entity_a, m.entity_b] {
                if let Some(&idx) = entity_map.get(&e.id()) {
                    island_indices.insert(idx);
                    // Uyuyanı YALNIZ gerçek bir hareketli varsa uyandır (yerleşen
                    // komşu uyuyanı geri uyandırmasın → ada topluca uyuyabilsin).
                    if island_has_mover
                        && rigid_bodies[idx].is_dynamic()
                        && rigid_bodies[idx].is_sleeping
                    {
                        wake_updates.push(e);
                    }
                }
            }
        }

        let mut result = IslandResult {
            velocities: Vec::with_capacity(island_indices.len()),
            position_corrections: Vec::with_capacity(island_indices.len()),
            ..IslandResult::default()
        };
        let stats = self.solve_island_contacts(&mut island_manifolds, &island_indices, &mut result, dt);
        result.stats = stats;

        // Fracture detection. Its own function: it is a question about the solved
        // impulses ("did this break anything?") rather than a step of solving them,
        // and answering it inline put five `if`s at nesting depth ten inside the
        // engine's hottest loop — the deepest point in the crate.
        let mut fractures = Vec::new();
        for m in &island_manifolds {
            collect_fracture_events(m, rigid_bodies, entity_map, &mut fractures);
        }

        // Sleep is decided for the island as a whole, not per body. A body that
        // stops being integrated in the middle of a stack is still solved against —
        // the contact solver reads its mass and exchanges impulses with it — so a
        // partially-asleep island stops conserving momentum across the interfaces
        // that touch the sleepers. Requiring the whole island keeps that from
        // arising. Eligibility itself is still per body and still counted in
        // `advance_sleep_counter`.
        let island_can_sleep = island_indices.iter().all(|&idx| {
            !rigid_bodies[idx].is_dynamic() || rigid_bodies[idx].sleep_eligible()
        });

        result.manifolds = island_manifolds;
        result.wake_ups = wake_updates;
        result.fractures = fractures;
        result.can_sleep = island_can_sleep;
        result
    }

    /// The contact solve of one island, through the worker's own scratch buffers (sized to the
    /// world, written only at the island's bodies). What each body came out with goes to `out`: a
    /// dynamic body's velocity, position correction and bounce, a kinematic body's response.
    fn solve_island_contacts(
        &self,
        island_manifolds: &mut [ContactManifold],
        island_indices: &FxHashSet<usize>,
        out: &mut IslandResult,
        dt: f32,
    ) -> crate::solver::SolveStats {
        let entity_map = &self.entity_index_map;
        let rigid_bodies = &self.rigid_bodies;
        let transforms = &self.transforms;
        let velocities = &self.velocities;
        let solver = &self.solver;
        let entities_arr = &self.entities;
        let velocity_updates = &mut out.velocities;
        let kinematic_responses = &mut out.kinematic_responses;
        let position_updates = &mut out.position_corrections;
        let bounce_updates = &mut out.bounces;

        // Thread-local buffers to avoid per-island allocations.
        thread_local! {
            static VEL_CACHE: std::cell::RefCell<Vec<Velocity>> =
                const { std::cell::RefCell::new(Vec::new()) };
            static POS_CACHE: std::cell::RefCell<Vec<(Vec3, Vec3)>> =
                const { std::cell::RefCell::new(Vec::new()) };
            static BOUNCE_CACHE: std::cell::RefCell<Vec<(Vec3, Vec3)>> =
                const { std::cell::RefCell::new(Vec::new()) };
        }

        VEL_CACHE.with(|cache| {
            POS_CACHE.with(|pos_cache| { BOUNCE_CACHE.with(|bounce_cache| {
                let mut buf = cache.borrow_mut();
                let mut pos_buf = pos_cache.borrow_mut();
                let mut bounce_buf = bounce_cache.borrow_mut();
                if bounce_buf.len() < velocities.len() {
                    bounce_buf.resize(velocities.len(), (Vec3::ZERO, Vec3::ZERO));
                }

                // Grow to fit the full velocity array if needed.
                if buf.len() < velocities.len() {
                    buf.resize(velocities.len(), Velocity::default());
                }
                if pos_buf.len() < velocities.len() {
                    pos_buf.resize(velocities.len(), (Vec3::ZERO, Vec3::ZERO));
                }

                // Copy only this island's velocities in.
                for &idx in island_indices {
                    buf[idx] = velocities[idx];
                }

                // Island-local body index list: sizes the solver's per-body
                // scratch/loops to THIS island instead of the whole world.
                let island_body_vec: Vec<usize> =
                    island_indices.iter().copied().collect();

                let stats = solver.solve_contacts(
                    island_manifolds,
                    rigid_bodies,
                    transforms,
                    &mut buf,
                    &mut pos_buf,
                    &mut bounce_buf,
                    entity_map,
                    &island_body_vec,
                    dt,
                );

                // Collect results.
                for &idx in island_indices {
                    // A kinematic body with a contact mass took impulses in the solve;
                    // its velocity stays the game's, and what it took is reported.
                    // Its bounce too: the restitution of a speculative contact goes
                    // to the bounce buffer, not into the solved velocity.
                    if !rigid_bodies[idx].is_dynamic() && rigid_bodies[idx].responds_to_contacts() {
                        let bounce = bounce_buf[idx].0;
                        kinematic_responses.push((idx, buf[idx].linear - velocities[idx].linear + bounce));
                    }
                    if rigid_bodies[idx].is_dynamic() {
                        velocity_updates.push((entities_arr[idx], buf[idx]));
                        let (dlin, dang) = pos_buf[idx];
                        if dlin != Vec3::ZERO || dang != Vec3::ZERO {
                            position_updates.push((entities_arr[idx], dlin, dang));
                        }
                        let (blin, bang) = bounce_buf[idx];
                        if blin != Vec3::ZERO || bang != Vec3::ZERO {
                            bounce_updates.push((entities_arr[idx], blin, bang));
                        }
                    }
                }
                stats
            }) })
        })
    }

    /// One island's solve, written back to the world: the kinematic responses and the bounces, the
    /// wake-ups before the velocities (see below), the split-impulse corrections, the island's
    /// sleep, the fractures, and the solved manifolds to their events and the warm-start cache.
    /// Returns how many bodies it woke and how many fractures it found.
    fn write_back_island(
        &mut self,
        result: IslandResult,
        event_index: &FxHashMap<(BodyHandle, BodyHandle), usize>,
        in_island: &mut FxHashSet<u32>,
        bounces: &mut Vec<(BodyHandle, Vec3, Vec3)>,
    ) -> (usize, usize) {
        let entity_map = &self.entity_index_map;
        let IslandResult {
            velocities: island_vels,
            manifolds: island_manifolds,
            wake_ups,
            fractures: local_fractures,
            position_corrections: pos_corrections,
            bounces: island_bounces,
            stats,
            can_sleep: island_can_sleep,
            kinematic_responses,
        } = result;
        let counts = (wake_ups.len(), local_fractures.len());
        // A kinematic body touching several islands took a share in each. It moves by it
        // for the rest of the step, as a body of its contact mass would — a later substep
        // that still saw it standing would push it again — and it is reported.
        for (idx, dv) in kinematic_responses {
            self.rigid_bodies[idx].contact_response += dv;
            self.velocities[idx].linear += dv;
        }
        bounces.extend(island_bounces);
        // Captured before `island_vels` is consumed below; empty unless the whole island
        // qualifies, so this allocates only on the substep a pile actually settles.
        let sleepers: Vec<BodyHandle> = if island_can_sleep {
            island_vels.iter().map(|(e, _)| *e).collect()
        } else {
            Vec::new()
        };
        // Effective sweep count, summed over islands (and, by the caller's accumulation,
        // over substeps): the number that says whether the adaptive policy was engaged
        // at all on this scene. Depth is a max, not a sum — it is a property of the
        // deepest island, and summing it would mean nothing.
        self.metrics.solver_sweeps += stats.iterations;
        self.metrics.max_island_depth = self.metrics.max_island_depth.max(stats.island_depth);
        // The wake pass runs BEFORE the velocities are written back, and that order is the
        // whole of the fix below.
        //
        // The solver has no sleep check anywhere (a sleeping dynamic still has finite
        // inverse mass, so it takes real impulses), and this loop used to write its result
        // onto every dynamic island member. Position integration then skips the sleeping
        // ones — so a body that stays asleep banks a velocity it never spends, and fires it
        // whenever something wakes it later. That is the defect `docs/ENGINE.md` records
        // against this write-back.
        //
        // Waking first is what keeps this from changing the ordinary case: an island with a
        // real mover wakes its sleepers here, so they are awake by the time the guard below
        // runs and still take the impact impulse on this substep. Only the half-awake
        // island — woken by something that is not a mover, e.g. the unconditional
        // ended-collision wake — loses the write, which is exactly the case that was wrong.
        for entity in wake_ups {
            if let Some(&idx) = entity_map.get(&entity.id()) {
                self.rigid_bodies[idx].wake_up();
            }
        }
        for (entity, vel) in island_vels {
            // Unconditional: `in_island` is read further down to decide sleep eligibility,
            // and a body that was solved belongs to an island whether or not it woke.
            in_island.insert(entity.id());
            if let Some(&idx) = entity_map.get(&entity.id()) {
                if !self.rigid_bodies[idx].is_sleeping {
                    self.velocities[idx] = vel;
                }
            }
        }
        // Split-impulse pozisyon düzeltmesini doğrudan transform'a uygula
        // (hız kanalına dokunmadan → resting jitter yok, cisimler uyuyabilir).
        for (entity, dlin, dang) in pos_corrections {
            if let Some(&idx) = entity_map.get(&entity.id()) {
                // Axis locks hold here too: this moves the transform without a
                // velocity, so the velocity-level `enforce_locks` cannot see it.
                let (dlin, dang) = self.rigid_bodies[idx].mask_locked(dlin, dang);
                // A sleeping member of an island with no real mover is not woken, and
                // takes its share of the correction all the same: the step moves a body
                // that stays asleep. Noted for whoever mirrors the world (`moved_asleep`).
                let rb = &self.rigid_bodies[idx];
                if rb.is_dynamic() && rb.is_sleeping && (dlin != gizmo_math::Vec3::ZERO || dang != gizmo_math::Vec3::ZERO) {
                    self.moved_asleep.push(entity.id());
                }
                let t = &mut self.transforms[idx];
                t.position += dlin;
                if dang.length_squared() > 1e-12 {
                    t.rotation = (gizmo_math::Quat::from_scaled_axis(dang) * t.rotation).normalize();
                }
                t.update_local_matrix();
            }
        }
        // After the wake pass, so the two can never fight: an island with a mover is
        // by construction not all-eligible, since a mover's counter was just reset.
        for entity in sleepers {
            if let Some(&idx) = entity_map.get(&entity.id()) {
                self.rigid_bodies[idx].is_sleeping = true;
            }
        }
        self.fracture_events.extend(local_fractures);

        // Update solved contact points on the matching collision event.
        for manifold in island_manifolds {
            let solved: ContactPoints =
                manifold.contacts.iter().copied().take(4).collect();

            // Try both orderings (broadphase may report either).
            let key_ab = (manifold.entity_a, manifold.entity_b);
            let key_ba = (manifold.entity_b, manifold.entity_a);

            if let Some(&idx) = event_index
                .get(&key_ab)
                .or_else(|| event_index.get(&key_ba))
            {
                self.collision_events[idx].contact_points = solved;
            }

            // WARM-START FIX: Save the solved manifold back to the cache!
            if let Some(entry) = self.contact_cache.get_mut(&key_ab) {
                entry.1 = Some(manifold);
            } else if let Some(entry) = self.contact_cache.get_mut(&key_ba) {
                entry.1 = Some(manifold);
            }
        }
        counts
    }

    /// Joint-coupled waking. The joint solver takes `&[RigidBody]` and cannot wake a sleeping
    /// body: a joint with a moving end silently changes the velocity of its other, sleeping end
    /// but leaves `is_sleeping` set, position integration skips that end, and the joint's
    /// correction is SWALLOWED (the mechanism looks broken apart).
    ///
    /// This works on the COMPONENT of bodies the joints connect, not joint by joint. There used
    /// to be a SINGLE PASS over `self.joints`, and the wake spread in array order: shaking the
    /// DEEP end of a 12-link chain woke only 5 links in a step (one link per substep), and the
    /// other 7 went on swallowing joint corrections they never integrated — the chain behaved as
    /// if it were pinned halfway. Not a nicety, physically: disturbing the bottom link of an
    /// inextensible chain loads EVERY link above it at once.
    ///
    /// For contacts the islands do this (`island.rs` union-finds the manifolds, and a stack wakes
    /// together); joints never entered the island build. The same structure, over the joint
    /// graph: ONE mover in a component wakes the whole component. With both ends asleep the
    /// mechanism is at rest, and nothing is touched.
    ///
    /// The cost scales with the joints, not the bodies: only the bodies joints TOUCH are gathered
    /// into a local index space.
    fn wake_joint_components(&mut self) {
        let mut local: FxHashMap<usize, usize> = FxHashMap::default();
        let mut members: Vec<usize> = Vec::new();
        let mut edges: Vec<(usize, usize)> = Vec::new();

        for j in self.joints.iter().filter(|j| !j.is_broken) {
            let ia = self.entity_index_map.get(&j.entity_a.id()).copied();
            let ib = self.entity_index_map.get(&j.entity_b.id()).copied();
            let (Some(ia), Some(ib)) = (ia, ib) else { continue };
            if ia == ib {
                continue;
            }
            let intern = |g: usize,
                              local: &mut FxHashMap<usize, usize>,
                              members: &mut Vec<usize>| {
                *local.entry(g).or_insert_with(|| {
                    members.push(g);
                    members.len() - 1
                })
            };
            let la = intern(ia, &mut local, &mut members);
            let lb = intern(ib, &mut local, &mut members);
            edges.push((la, lb));
        }

        if !members.is_empty() {
            let mut parent: Vec<usize> = (0..members.len()).collect();
            fn find(parent: &mut [usize], mut i: usize) -> usize {
                while parent[i] != i {
                    parent[i] = parent[parent[i]]; // path splitting
                    i = parent[i];
                }
                i
            }
            for &(a, b) in &edges {
                let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
                if ra != rb {
                    parent[ra] = rb;
                }
            }

            // Bileşen başına: mover var mı?
            let mut has_mover: Vec<bool> = vec![false; members.len()];
            for (li, &gi) in members.iter().enumerate() {
                let rb = &self.rigid_bodies[gi];
                let mover = (rb.is_dynamic() && !rb.is_sleeping)
                    || (rb.is_kinematic()
                        && (self.velocities[gi].linear.length_squared() > 1e-8
                            || self.velocities[gi].angular.length_squared() > 1e-8));
                if mover {
                    let r = find(&mut parent, li);
                    has_mover[r] = true;
                }
            }
            for (li, &gi) in members.iter().enumerate() {
                let r = find(&mut parent, li);
                if has_mover[r]
                    && self.rigid_bodies[gi].is_dynamic()
                    && self.rigid_bodies[gi].is_sleeping
                {
                    self.rigid_bodies[gi].wake_up();
                }
            }
        }
    }

    /// Adds the speculative contacts' bounces (from [`constraint_solve_step`](Self::constraint_solve_step))
    /// once positions are integrated: the bodies have reached each other, and leave from there.
    pub(crate) fn apply_bounces(&mut self, bounces: &[(BodyHandle, Vec3, Vec3)]) {
        for &(entity, linear, angular) in bounces {
            let Some(&idx) = self.entity_index_map.get(&entity.id()) else { continue };
            if self.rigid_bodies[idx].is_sleeping || !self.rigid_bodies[idx].is_dynamic() {
                continue;
            }
            let vel = &mut self.velocities[idx];
            vel.linear += linear;
            vel.angular += angular;
            vel.pre_linear = vel.linear;
            vel.pre_angular = vel.angular;
        }
    }

    // ================================================================== //
    //  Stage 5-6 — Position integration & sleep update                    //
    // ================================================================== //

    /// Integrate positions from velocities (parallel) and update sleep state.
    #[tracing::instrument(skip_all, name = "position_integration")]
    pub(crate) fn position_integration_step(
        &mut self,
        dt: f32,
    ) -> Result<(), gizmo_physics_core::GizmoError> {
        let integrator = &self.integrator;

        let parallel = worth_parallel(&self.rigid_bodies);
        let integrate =
                |(((&entity, rb), transform), vel): (((&BodyHandle, &mut RigidBody), &mut Transform), &mut Velocity)| -> Result<(), gizmo_physics_core::GizmoError> {
                    if rb.is_sleeping {
                        return Ok(());
                    }

                    // Apply axis locks before position integration.
                    rb.enforce_locks(vel);

                    integrator.integrate_positions(entity, rb, transform, vel, dt)?;

                    Ok(())
                };
        let result = if parallel {
            self.entities
                .par_iter()
                .zip(self.rigid_bodies.par_iter_mut())
                .zip(self.transforms.par_iter_mut())
                .zip(self.velocities.par_iter_mut())
                .try_for_each(integrate)
        } else {
            self.entities
                .iter()
                .zip(self.rigid_bodies.iter_mut())
                .zip(self.transforms.iter_mut())
                .zip(self.velocities.iter_mut())
                .try_for_each(&integrate)
        };
        result
            .inspect_err(|e| {
                tracing::error!("Position integration failed: {e:?}");
                if let Err(snap_err) =
                    self.trigger_snapshot("Position Integration Error (NaN/Overflow)")
                {
                    tracing::error!("Failed to write physics snapshot: {snap_err}");
                }
            })
    }

    /// CCD backstop — runs right after position integration.
    ///
    /// The speculative-contact path lets a CCD body close the gap to an obstacle and
    /// stop one frame later, but it relies on the GJK separation distance being
    /// accurate at near-contact. Against **thin** geometry (a box whose face is tiny
    /// next to its other extents) the GJK distance can degenerate to a far corner, so
    /// the speculative contact bails and a Mach-scale body sails straight through.
    ///
    /// This pass is a robust geometric guard: for any CCD body that travelled farther
    /// than its own radius this substep (i.e. fast enough that discrete detection could
    /// miss it), sweep its centre against each static / sleeping collider's AABB
    /// inflated by the body's half-extents. If the swept segment enters one, the body
    /// is clamped a hair short of that face and its inward velocity is removed.
    ///
    /// Deliberately scoped: slow / resting bodies (`travel <= radius`) are left
    /// entirely to the discrete + speculative path, so ordinary contacts and resting
    /// stacks are byte-for-byte unchanged; two fast *dynamic* bodies remain the
    /// documented out-of-scope case (handled, imperfectly, by the speculative path).
    #[tracing::instrument(skip_all, name = "ccd_resolve")]
    pub(crate) fn ccd_resolve_step(&mut self, dt: f32) {
        use gizmo_math::{Aabb, Vec3};
        use gizmo_physics_core::raycast::{Ray, Raycast};
        const SKIN: f32 = 0.01;

        let n = self.entities.len();
        let mut clamps = 0usize;
        for i in 0..n {
            let rb_i = &self.rigid_bodies[i];
            // `!is_static()` so kinematic CCD movers (new_kinematic ⇒ ccd on) are
            // also backstopped — testing is_dynamic() left them tunnelling.
            if !rb_i.ccd_enabled || rb_i.is_static() || rb_i.is_sleeping {
                continue;
            }
            let vel = self.velocities[i].linear;
            let delta = vel * dt;
            let travel = delta.length();
            if travel < 1e-5 {
                continue;
            }
            // Own half-extents; only engage once the body outruns discrete detection.
            let self_aabb = self.colliders[i].compute_aabb(Vec3::ZERO, self.transforms[i].rotation);
            let half = (Vec3::from(self_aabb.max) - Vec3::from(self_aabb.min)) * 0.5;
            let min_half = half.x.min(half.y).min(half.z).max(1e-4);
            if travel <= min_half {
                continue;
            }
            let dir = delta / travel;
            let new_pos = self.transforms[i].position;
            let old_pos = new_pos - delta;

            let mut best_toi = f32::INFINITY;
            let mut best_normal = Vec3::ZERO;
            for j in 0..n {
                if j == i {
                    continue;
                }
                // Triggers are pass-through by definition; everything solid is a valid
                // target, awake dynamic bodies included.
                //
                // This used to also skip `rb_j.is_dynamic() && !rb_j.is_sleeping`, which is
                // why `ccd_hole_fast_body_vs_dynamic_awake_thin_plate` tunnelled while the
                // otherwise-identical Rung 7 (same plate, asleep) passed. The exclusion had
                // a real reason — for a *moving* target the sweep was frame-mismatched,
                // since the bullet's `old_pos` is pre-integration while `compute_aabb` reads
                // the target's post-integration transform — but the gate was far wider than
                // the reason. The fix below is to sweep the target's own volume instead of
                // snapshotting it, which removes the mismatch and makes the exclusion
                // unnecessary.
                if self.colliders[j].is_trigger {
                    continue;
                }
                // Respect collision-layer filtering, exactly like narrowphase — else the
                // backstop phantom-stops the body against geometry it is masked to pass through.
                if !self.colliders[i]
                    .collision_layer
                    .can_collide_with(&self.colliders[j].collision_layer)
                {
                    continue;
                }
                // Sweep the TARGET too. `compute_aabb` gives its post-integration pose, but
                // the ray starts from the bullet's pre-integration position, so a moving
                // target must be covered over the whole substep or the two are a frame
                // apart. Merging the current box with where it was at the start of the
                // substep closes that gap; for a static or sleeping target the delta is
                // zero and this is exactly the old behaviour.
                let other_now = self.colliders[j]
                    .compute_aabb(self.transforms[j].position, self.transforms[j].rotation);
                let other_delta = self.velocities[j].linear * dt;
                let other_then = Aabb::new(
                    Vec3::from(other_now.min) - other_delta,
                    Vec3::from(other_now.max) - other_delta,
                );
                let other = other_now.merge(other_then);
                let infl = Aabb::new(Vec3::from(other.min) - half, Vec3::from(other.max) + half);
                // Already overlapping at the start of the substep ⇒ leave it to the
                // discrete solver (this guard is only for clean pass-through).
                if infl.contains_point(old_pos) {
                    continue;
                }
                let ray = Ray::new(old_pos, dir);
                if let Some(t) = Raycast::ray_aabb(&ray, &infl) {
                    if (0.0..=travel).contains(&t) && t < best_toi {
                        best_toi = t;
                        best_normal = Self::aabb_face_normal(&infl, old_pos + dir * t);
                    }
                }
            }

            if best_toi.is_finite() {
                clamps += 1;
                let entity = self.entities[i];
                self.transforms[i].position = old_pos + dir * (best_toi - SKIN).max(0.0);
                // Rebuild the cached local matrix so the clamp is visible to an
                // end-of-frame snapshot (integrator + split-impulse paths do the same).
                self.transforms[i].update_local_matrix();
                let vn = self.velocities[i].linear.dot(best_normal);
                if vn < 0.0 {
                    self.velocities[i].linear -= best_normal * vn;
                }
                // A fast CCD body was about to tunnel through static/sleeping geometry this
                // substep; the geometric backstop clamped it to the surface. Rare and
                // per-trigger → trace with toi/travel/normal is exactly what a tunnelling
                // investigation needs.
                tracing::trace!(
                    entity = ?entity,
                    toi = best_toi,
                    travel,
                    normal = ?best_normal,
                    "CCD backstop clamped a fast body short of tunnelling"
                );
            }
        }

        if clamps > 0 {
            tracing::debug!(ccd_clamps = clamps, "CCD backstop engaged this substep");
        }
    }

    /// Outward normal of the AABB face nearest to `hit` (a point on its surface).
    fn aabb_face_normal(aabb: &gizmo_math::Aabb, hit: Vec3) -> Vec3 {
        let min = Vec3::from(aabb.min);
        let max = Vec3::from(aabb.max);
        let dmin = (hit - min).abs();
        let dmax = (hit - max).abs();
        let faces = [
            (dmin.x, Vec3::new(-1.0, 0.0, 0.0)),
            (dmax.x, Vec3::new(1.0, 0.0, 0.0)),
            (dmin.y, Vec3::new(0.0, -1.0, 0.0)),
            (dmax.y, Vec3::new(0.0, 1.0, 0.0)),
            (dmin.z, Vec3::new(0.0, 0.0, -1.0)),
            (dmax.z, Vec3::new(0.0, 0.0, 1.0)),
        ];
        let mut best = f32::INFINITY;
        let mut normal = Vec3::X;
        for (d, nrm) in faces {
            if d < best {
                best = d;
                normal = nrm;
            }
        }
        normal
    }
}
