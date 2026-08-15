//! Physics pipeline — internal sub-steps of `PhysicsWorld::step`.
//!
//! The original monolithic 700-line `step_internal` has been split into
//! focused, individually-traceable functions.  Each function is responsible
//! for exactly one pipeline stage and returns a typed result so the caller
//! can handle errors at the right boundary.

use crate::{
    components::Velocity,
    world::{PhysicsWorld, ZoneShape},
};
use gizmo_physics_core::{CollisionEvent, CollisionEventType, ContactManifold, ContactPoints, TriggerEvent};
use gizmo_physics_core::narrowphase::NarrowPhase;
use gizmo_physics_core::BodyHandle;
use gizmo_math::Vec3;
#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;
#[cfg(target_arch = "wasm32")]
use crate::parallel_compat::*;
use rustc_hash::{FxHashMap, FxHashSet};


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

        // Parallel iteration over all entities.
        // `try_for_each` short-circuits on the first `Err` and propagates it.
        self.entities
            .par_iter()
            .zip(self.rigid_bodies.par_iter_mut())
            .zip(self.transforms.par_iter())
            .zip(self.velocities.par_iter_mut())
            .zip(self.colliders.par_iter())
            .try_for_each(
                |((((&entity, rb), transform), vel), collider)| -> Result<(), gizmo_physics_core::GizmoError> {
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
                    for zone in fluid_zones {
                        if !zone.shape.contains(pos) {
                            continue;
                        }

                        let extents_y = collider.extents_y();
                        let surface_y = match zone.shape {
                            ZoneShape::Box { max, .. }         => max.y,
                            ZoneShape::Sphere { center, radius } => center.y + radius,
                        };

                        let submerged_depth =
                            (surface_y - (pos.y - extents_y)).clamp(0.0, extents_y * 2.0);
                        let denom = (extents_y * 2.0).max(f32::EPSILON);
                        let submerged_ratio = submerged_depth / denom;

                        if submerged_ratio > 0.0 {
                            let submerged_volume = collider.volume() * submerged_ratio;
                            let buoyancy_force   = -active_gravity * (submerged_volume * zone.density);

                            let speed = vel.linear.length();
                            let drag_dir = if speed > 1e-4 {
                                vel.linear / speed
                            } else {
                                gizmo_math::Vec3::ZERO
                            };
                            let drag_mag = zone.linear_drag    * speed
                                         + zone.quadratic_drag * speed * speed;
                            let drag_force = -drag_dir * drag_mag * submerged_ratio;

                            vel.linear +=
                                (buoyancy_force + drag_force) * rb.inv_mass() * dt;
                        }
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
                },
            )
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

    /// Rebuild the spatial hash with swept AABBs and insert soft-body bounds.
    #[tracing::instrument(skip_all, name = "broadphase")]
    pub(crate) fn broadphase_step(
        &mut self,
        
        dt: f32,
    ) {
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
        for i in 0..self.entities.len() {
            let entity = self.entities[i];
            let rb = &self.rigid_bodies[i];
            let transform = &self.transforms[i];
            let vel = &self.velocities[i];
            let collider = &self.colliders[i];

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

            self.spatial_hash.insert(entity, aabb);
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

    /// Detect actual collisions (parallel narrowphase), emit collision /
    /// trigger events, and handle soft-body contacts (sequential).
    ///
    /// Returns the set of [`ContactManifold`]s to be fed into the solver.
    #[tracing::instrument(skip_all, name = "narrowphase_collision")]
    pub(crate) fn narrowphase_and_collision_step(
        &mut self,
        
        dt: f32,
    ) -> Vec<ContactManifold> {
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
                None => true, // rigid değil (soft cisim) → aktif say (ayrı yol işler)
            }
        };
        let mut dormant_pairs: Vec<(BodyHandle, BodyHandle)> = Vec::new();
        let active_pairs: Vec<(BodyHandle, BodyHandle)> = potential_pairs
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

        // ── Parallel narrowphase ──────────────────────────────────────────
        // Each entry: (entity_a, entity_b, contacts, is_trigger_a, is_trigger_b,
        //              mat_a, mat_b, is_soft_pair)
        type NpResult = (
            BodyHandle,
            BodyHandle,
            Vec<gizmo_physics_core::ContactPoint>,
            bool,
            bool,
            gizmo_physics_core::PhysicsMaterial,
            gizmo_physics_core::PhysicsMaterial,
            bool, // is_soft_pair
            // Warm-start outcome, computed in parallel with the collision maths rather than in the
            // sequential assembly below: `Some(lifetime)` when this pair persisted from the last
            // frame, and the impulses in `contacts` have already been carried across. It is pure
            // per-pair work over immutable state (`contact_cache`, the match tolerance), so it
            // belongs on the worker that produced the contacts, not on the one thread that has to
            // build the cache and the event list in order.
            Option<u32>,
        );

        let default_material = gizmo_physics_core::PhysicsMaterial::default();

        // Precompute the set of entity pairs joined by a collision-disabled joint ONCE per
        // frame (keyed by ordered id pair), instead of re-scanning every joint for every
        // candidate contact pair inside the parallel narrowphase — that was O(pairs × joints).
        // Empty (the common jointless case) ⇒ the per-pair check below short-circuits for free.
        let disabled_joint_pairs: FxHashSet<(u32, u32)> = self
            .joints
            .iter()
            .filter(|j| !j.collision_enabled)
            .map(|j| {
                let (a, b) = (j.entity_a.id(), j.entity_b.id());
                (a.min(b), a.max(b))
            })
            .collect();

        // Kapsam paralel bölümün TAMAMINI sarar, çift başına değil. Eskiden çift başınaydı:
        // kare başına ~7300 `Instant::now()` + paylaşılan tek atomiğe `fetch_add`, hem
        // `profile.rs`'in kendi sözleşmesini ("asla temas ya da cisim başına") ihlal ediyor hem de
        // ölçtüğü şeye maliyet ekliyordu. Ölçüldü: dar fazın %3'ü. Küçük, ama gönderilen bir
        // motorda kimsenin okumadığı bir sayaç için kalıcı bir vergi, ve buradan okunan sayı
        // (duvar saati) zaten daha kullanışlı — CPU-ms yerine fazın gerçekte ne kadar sürdüğü.
        let _t = crate::profile::Scope::new(&crate::profile::PHASES.dispatch);
        let narrowphase_results: Vec<NpResult> = active_pairs
            .par_iter()
            .filter_map(|&(entity_a, entity_b)| {
                let is_a_rigid = entity_map.contains_key(&entity_a.id());
                let is_b_rigid = entity_map.contains_key(&entity_b.id());

                match (is_a_rigid, is_b_rigid) {
                    // ── Rigid vs Rigid ────────────────────────────────────
                    (true, true) => {
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

                        Some((
                            entity_a,
                            entity_b,
                            contacts,
                            collider_a.is_trigger,
                            collider_b.is_trigger,
                            collider_a.material,
                            collider_b.material,
                            false,
                            warm,
                        ))
                    }

                    // ── Mixed rigid / soft ─────────────────────────────────
                    // Return a marker; the actual work happens sequentially below.
                    (true, false) | (false, true) => Some((
                        entity_a,
                        entity_b,
                        Vec::new(),
                        false,
                        false,
                        default_material,
                        default_material,
                        true,
                        None,
                    )),

                    // ── Soft vs Soft ──────────────────────────────────────
                    (false, false) => Some((
                        entity_a,
                        entity_b,
                        Vec::new(),
                        false,
                        false,
                        default_material,
                        default_material,
                        true,
                        None,
                    )),
                }
            })
            .collect();

        // ── Sequential post-processing ────────────────────────────────────
        // Pre-size to last frame's counts: the contact set is frame-coherent, so this skips the
        // ~log2(N) grow-and-rehash reallocations the map/vec otherwise do every frame while
        // filling to tens of thousands of entries.
        let mut manifolds = Vec::with_capacity(narrowphase_results.len());
        let mut current_cache =
            FxHashMap::with_capacity_and_hasher(self.contact_cache.len(), Default::default());
        let _mani = crate::profile::Scope::new(&crate::profile::PHASES.manifold);
        let mut soft_rigid_pairs = Vec::new();
        let mut soft_soft_pairs = Vec::new();

        for (
            entity_a,
            entity_b,
            contacts,
            is_trigger_a,
            is_trigger_b,
            mat_a,
            mat_b,
            is_soft,
            warm,
        ) in narrowphase_results
        {
            if is_soft {
                let is_a_rigid = entity_map.contains_key(&entity_a.id());
                let is_b_rigid = entity_map.contains_key(&entity_b.id());

                match (is_a_rigid, is_b_rigid) {
                    (true, false) => soft_rigid_pairs.push((entity_a, entity_b)),
                    (false, true) => soft_rigid_pairs.push((entity_b, entity_a)),
                    _ => soft_soft_pairs.push((entity_a, entity_b)),
                }
                continue;
            }

            let pair = (entity_a, entity_b);

            if is_trigger_a || is_trigger_b {
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
            } else {
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

                manifolds.push(manifold);
            }
        }

        // ── Dormant çiftlerin cache'ini KORU ──────────────────────────────
        // Narrowphase atlandı (her iki cisim dormant). Önceki cache girdisini current_cache'e
        // taşı ki aşağıdaki ended-collision döngüsü bunları "bitti" sanıp UYANDIRMASIN.
        // Manifold solver'a EKLENMEZ (iki cisim de dormant → island zaten çözülmez); bu yalnız
        // temas-cache sürekliliği. Bir cisim uyanınca çift "aktif" olur → narrowphase döner.
        for &pair in &dormant_pairs {
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

        // ── Ended collisions ──────────────────────────────────────────────
        for (pair, (is_trigger, _)) in &self.contact_cache {
            if current_cache.contains_key(pair) {
                continue;
            }

            // Wake both bodies so they aren't frozen mid-air after losing support.
            for entity in [pair.0, pair.1] {
                if let Some(&idx) = entity_map.get(&entity.id()) {
                    if self.rigid_bodies[idx].is_dynamic() {
                        self.rigid_bodies[idx].wake_up();
                    }
                }
            }

            if *is_trigger {
                self.trigger_events.push(TriggerEvent {
                    trigger_entity: pair.0,
                    other_entity: pair.1,
                    event_type: CollisionEventType::Ended,
                });
            } else {
                self.collision_events.push(CollisionEvent {
                    entity_a: pair.0,
                    entity_b: pair.1,
                    event_type: CollisionEventType::Ended,
                    contact_points: ContactPoints::new(),
                });
            }
        }

        self.contact_cache = current_cache;

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

    // ================================================================== //
    //  Stage 4–4.5 — Constraint solver                                    //
    // ================================================================== //

    /// Solve collision constraints (via island-parallel PGS) and explicit
    /// joints (hinges, springs, …).
    #[tracing::instrument(skip_all, name = "constraint_solve")]
    pub(crate) fn constraint_solve_step(&mut self, manifolds: Vec<ContactManifold>, dt: f32) {
        let entity_map = &self.entity_index_map;

        // Dynamic bodies the island sleep rule got to decide about this substep. Declared out
        // here because the sweep for everything else has to run even when there are no contacts
        // at all — a pair of bodies joined only by a joint has no island and must still be able
        // to sleep.
        let mut in_island: FxHashSet<u32> = FxHashSet::default();

        // ── Collision constraints ─────────────────────────────────────────
        if !manifolds.is_empty() {
            let is_dynamic = |entity: BodyHandle| -> bool {
                entity_map
                    .get(&entity.id())
                    .is_some_and(|&idx| self.rigid_bodies[idx].is_dynamic())
            };

            let islands = crate::island::IslandManager::build_islands(&manifolds, &is_dynamic);
            let island_count = islands.len();
            let island_groups = crate::island::IslandManager::split_manifolds(manifolds, &islands);

            let rigid_bodies = &self.rigid_bodies;
            let transforms = &self.transforms;
            let velocities = &self.velocities;
            let solver = &self.solver;
            let entities_arr = &self.entities;

            type IslandResult = (
                Vec<(BodyHandle, Velocity)>,      // velocity updates
                Vec<ContactManifold>,         // solved manifolds (warm-start data)
                Vec<BodyHandle>,                  // entities to wake up
                Vec<gizmo_physics_core::FractureEvent>,
                Vec<(BodyHandle, Vec3, Vec3)>,    // split-impulse position corrections (Δlin, Δscaled-axis)
                crate::solver::SolveStats,        // adaptive sweep count + support depth
                bool,                             // every dynamic member is eligible to sleep
            );

            let results: Vec<IslandResult> = island_groups
                .into_par_iter()
                .map(|mut island_manifolds| -> IslandResult {
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
                        return (
                            Vec::new(),
                            island_manifolds,
                            Vec::new(),
                            Vec::new(),
                            Vec::new(),
                            crate::solver::SolveStats::default(),
                            false,
                        );
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

                    // Thread-local buffers to avoid per-island allocations.
                    thread_local! {
                        static VEL_CACHE: std::cell::RefCell<Vec<Velocity>> =
                            const { std::cell::RefCell::new(Vec::new()) };
                        static POS_CACHE: std::cell::RefCell<Vec<(Vec3, Vec3)>> =
                            const { std::cell::RefCell::new(Vec::new()) };
                    }

                    let mut velocity_updates = Vec::with_capacity(island_indices.len());
                    let mut position_updates = Vec::with_capacity(island_indices.len());

                    let solve_stats = VEL_CACHE.with(|cache| {
                        POS_CACHE.with(|pos_cache| {
                            let mut buf = cache.borrow_mut();
                            let mut pos_buf = pos_cache.borrow_mut();

                            // Grow to fit the full velocity array if needed.
                            if buf.len() < velocities.len() {
                                buf.resize(velocities.len(), Velocity::default());
                            }
                            if pos_buf.len() < velocities.len() {
                                pos_buf.resize(velocities.len(), (Vec3::ZERO, Vec3::ZERO));
                            }

                            // Copy only this island's velocities in.
                            for &idx in &island_indices {
                                buf[idx] = velocities[idx];
                            }

                            // Island-local body index list: sizes the solver's per-body
                            // scratch/loops to THIS island instead of the whole world.
                            let island_body_vec: Vec<usize> =
                                island_indices.iter().copied().collect();

                            let stats = solver.solve_contacts(
                                &mut island_manifolds,
                                rigid_bodies,
                                transforms,
                                &mut buf,
                                &mut pos_buf,
                                entity_map,
                                &island_body_vec,
                                dt,
                            );

                            // Collect results.
                            for &idx in &island_indices {
                                if rigid_bodies[idx].is_dynamic() {
                                    velocity_updates.push((entities_arr[idx], buf[idx]));
                                    let (dlin, dang) = pos_buf[idx];
                                    if dlin != Vec3::ZERO || dang != Vec3::ZERO {
                                        position_updates.push((entities_arr[idx], dlin, dang));
                                    }
                                }
                            }
                            stats
                        })
                    });

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

                    (
                        velocity_updates,
                        island_manifolds,
                        wake_updates,
                        fractures,
                        position_updates,
                        solve_stats,
                        island_can_sleep,
                    )
                })
                .collect();

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
            for (
                island_vels,
                island_manifolds,
                wake_ups,
                local_fractures,
                pos_corrections,
                stats,
                island_can_sleep,
            ) in results
            {
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
                for (entity, vel) in island_vels {
                    in_island.insert(entity.id());
                    if let Some(&idx) = entity_map.get(&entity.id()) {
                        self.velocities[idx] = vel;
                    }
                }
                // Split-impulse pozisyon düzeltmesini doğrudan transform'a uygula
                // (hız kanalına dokunmadan → resting jitter yok, cisimler uyuyabilir).
                for (entity, dlin, dang) in pos_corrections {
                    if let Some(&idx) = entity_map.get(&entity.id()) {
                        let t = &mut self.transforms[idx];
                        t.position += dlin;
                        if dang.length_squared() > 1e-12 {
                            t.rotation = (gizmo_math::Quat::from_scaled_axis(dang) * t.rotation).normalize();
                        }
                        t.update_local_matrix();
                    }
                }
                total_wakes += wake_ups.len();
                for entity in wake_ups {
                    if let Some(&idx) = entity_map.get(&entity.id()) {
                        self.rigid_bodies[idx].wake_up();
                    }
                }
                // After the wake pass, so the two can never fight: an island with a mover is
                // by construction not all-eligible, since a mover's counter was just reset.
                for entity in sleepers {
                    if let Some(&idx) = entity_map.get(&entity.id()) {
                        self.rigid_bodies[idx].is_sleeping = true;
                    }
                }
                total_fractures += local_fractures.len();
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

        // ── Joints ────────────────────────────────────────────────────────
        if !self.joints.is_empty() {
            // Joint-coupled uyandırma: joint_solver `&[RigidBody]` alır → uyuyan bir cismi
            // uyandıramaz; ucu hareketli bir eklemin diğer (uyuyan) ucunun hızını sessizce
            // değiştirir ama is_sleeping'i bırakır → position_integration onu atlar, eklem
            // düzeltmesi YUTULUR (mekanizma kopuk görünür).
            //
            // Bu, eklemlerle bağlı cisim BİLEŞENİ üzerinde çalışır, tek tek eklemler üzerinde
            // değil. Eskiden `self.joints` üzerinde TEK GEÇİŞ vardı ve yayılım dizi sırasına
            // bağlıydı: 12 halkalı bir zincirin DERİN ucu sarsıldığında bir adımda yalnız 5
            // halka uyanıyordu (substep başına bir halka), kalan 7'si hiç entegre etmedikleri
            // eklem düzeltmelerini yutmaya devam ediyordu — zincir yarısından çivilenmiş gibi
            // davranıyordu. Fiziksel olarak incelik değil: uzamaz bir zincirin alt halkasını
            // rahatsız etmek üstündeki HER halkayı aynı anda yükler.
            //
            // Temaslarda bunu island'lar çözüyor (`island.rs` manifoldları union-find'lar,
            // yığın topluca uyanır); eklemler island kurulumuna hiç girmiyordu. Aynı yapı,
            // eklem grafı üzerinde: bileşende TEK BİR mover varsa bileşenin tamamı uyanır.
            // İki uç da uykudaysa mekanizma dinlenmededir → dokunma.
            //
            // Maliyet eklem sayısına bağlı, cisim sayısına değil: yalnız eklemlerin DOKUNDUĞU
            // cisimler yerel bir indeks uzayına toplanır.
            {
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

        self.entities
            .par_iter()
            .zip(self.rigid_bodies.par_iter_mut())
            .zip(self.transforms.par_iter_mut())
            .zip(self.velocities.par_iter_mut())
            .try_for_each(
                |(((&entity, rb), transform), vel)| -> Result<(), gizmo_physics_core::GizmoError> {
                    if rb.is_sleeping {
                        return Ok(());
                    }

                    // Apply axis locks before position integration.
                    rb.enforce_locks(vel);

                    integrator.integrate_positions(entity, rb, transform, vel, dt)?;

                    Ok(())
                },
            )
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
