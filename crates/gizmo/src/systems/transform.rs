use gizmo_core::Entity;

/// Propagates the transform hierarchy: every entity's `GlobalTransform` is rebuilt from its
/// local `Transform` and its parents', so a child follows what it is attached to.
pub struct TransformSyncSystem;

impl gizmo_core::system::System for TransformSyncSystem {
    fn access_info(&self) -> gizmo_core::system::AccessInfo {
        let mut info = gizmo_core::system::AccessInfo::new();
        // Since we borrow components, we can mark it as exclusive to be safe, or specify component access.
        // For simplicity and safety during hierarchy traversal, we'll make it exclusive.
        info.is_exclusive = true;
        info
    }

    #[tracing::instrument(skip_all, level = "trace", name = "transform_sync")]
    fn run(&mut self, world: &gizmo_core::world::World, _dt: f32) {
        // SAFETY: scheduled system; the scheduler guarantees no other system mutably
        // aliases Transform while this runs (see `World::query_unchecked`).
        let mut updated = 0usize;
        // SAFETY: see the note above — scheduled system, no other system aliases Transform.
        if let Some(mut transforms) = unsafe {
            world.query_unchecked::<gizmo_core::query::Mut<gizmo_physics_core::Transform>>()
        } {
            // Rebuilt only where it moved, and written WITHOUT a change mark: `local_matrix` is a
            // cache derived from the fields, not a change of its own. Writing it through the
            // change-tracked path marked every `Transform` in the world changed every frame,
            // moving or not — the same mistake `set_global_matrix` below was written to undo for
            // `GlobalTransform` — and on a static scene of 60 000 entities it was most of this
            // pass's cost (measured 2026-11-03).
            for mut trans in transforms.iter_mut() {
                let next = crate::math::Mat4::from_scale_rotation_translation(trans.scale, trans.rotation, trans.position);
                if trans.local_matrix != next {
                    trans.bypass_change_detection().local_matrix = next;
                    updated += 1;
                }
            }
        }
        tracing::trace!(updated, "transform_sync: yerel matrisler güncellendi");
    }
}

/// Write a world matrix into a `GlobalTransform` **without** stamping the change tick when
/// the value is bit-identical to what is already there.
///
/// `Mut::deref_mut` sets `ticks.changed = current_tick` on every single use, so propagation —
/// which runs every frame over every hierarchy node — used to mark the whole scene changed
/// whether or not anything moved. That made `Changed<GlobalTransform>` a filter that matched
/// everything, i.e. no filter at all, and any consumer built on it (the renderer's visibility
/// index above all) paid full price for a scene that was standing still.
///
/// The comparison is exact `Mat4` equality, deliberately: a stale box is a mesh that silently
/// stops being drawn, so the only safe suppression is "the bytes did not move". No epsilon.
#[inline]
fn set_global_matrix(
    global: &mut gizmo_core::query::Mut<'_, gizmo_physics_core::components::GlobalTransform>,
    next: crate::math::Mat4,
) {
    if global.matrix == next {
        // Already correct — touch the value, not the tick.
        global.bypass_change_detection().matrix = next;
    } else {
        global.matrix = next;
    }
}

/// Walks the hierarchy root-down and writes each entity's `GlobalTransform`, so a parent's
/// movement reaches its children in the same frame it happened.
pub struct TransformPropagateSystem;

/// Entity ids already placed this pass — a bitset indexed by id (ids are dense slot indices).
#[derive(Default)]
struct VisitedIds {
    bits: Vec<u64>,
    count: usize,
}

impl VisitedIds {
    /// Marks `id`; `true` if it was not marked yet — `HashSet::insert`'s answer.
    fn insert(&mut self, id: u32) -> bool {
        let (word, bit) = ((id / 64) as usize, 1u64 << (id % 64));
        if word >= self.bits.len() {
            self.bits.resize(word + 1, 0);
        }
        let fresh = self.bits[word] & bit == 0;
        self.bits[word] |= bit;
        self.count += usize::from(fresh);
        fresh
    }
}

impl gizmo_core::system::System for TransformPropagateSystem {
    fn access_info(&self) -> gizmo_core::system::AccessInfo {
        let mut info = gizmo_core::system::AccessInfo::new();
        info.is_exclusive = true; // Safe fallback for complex queries
        info
    }

    #[tracing::instrument(skip_all, level = "trace", name = "transform_propagate")]
    fn run(&mut self, world: &gizmo_core::world::World, _dt: f32) {
        // Query to get root transforms (no Parent)
        // SAFETY: scheduled system; scheduler guarantees disjoint mutable access.
        let root_query = unsafe {
            world.query_unchecked::<(
                Entity,
                &gizmo_physics_core::Transform,
                gizmo_core::query::Mut<gizmo_physics_core::components::GlobalTransform>,
                gizmo_core::query::Without<gizmo_core::component::Parent>,
            )>()
        };

        let mut queue = Vec::new();
        // A `Children` cycle (reachable if the editor reparents an entity onto its own
        // descendant) would otherwise grow `queue` forever — this system runs EVERY
        // frame, so a single cyclic edit hangs the whole app. Track visited ids and
        // never enqueue one twice. For a valid tree this is a no-op (each node has one
        // parent → is enqueued once).
        // A bit per entity id, not a `HashSet`: every root goes in here every frame (the guard
        // against a `Children` list naming a root), and a SipHash insert per entity was most of
        // this pass on a big flat scene.
        let mut visited = VisitedIds::default();

        if let Some(mut roots) = root_query {
            let mut children_query = world.query::<&gizmo_core::component::Children>();
            for (entity, local, mut global, _) in roots.iter_mut() {
                // Compare-then-write. `Mut::deref_mut` stamps `ticks.changed` on EVERY use,
                // so writing unconditionally marked every node of the hierarchy as changed
                // every single frame — `Changed<GlobalTransform>` then matched 100% of the
                // scene and was worth exactly nothing as a filter. Writing the identical
                // value through `bypass_change_detection` keeps the field correct and
                // leaves the tick alone. See T16–T18.
                set_global_matrix(&mut global, local.local_matrix);
                visited.insert(entity.id());
                if let Some(children_q) = &mut children_query {
                    if let Some(children) = children_q.get(entity) {
                        for &child in &children.0 {
                            if world.is_alive(child) && visited.insert(child.id()) {
                                queue.push((global.matrix, child));
                            }
                        }
                    }
                }
            }
        }

        // Processing children (we need random access, so we do individual queries)
        let mut local_query = world.query::<&gizmo_physics_core::Transform>();
        // SAFETY: scheduled system; scheduler guarantees disjoint mutable access.
        let mut global_query = unsafe {
            world.query_unchecked::<gizmo_core::query::Mut<gizmo_physics_core::components::GlobalTransform>>()
        };
        let mut children_query = world.query::<&gizmo_core::component::Children>();

        let mut head = 0;
        while head < queue.len() {
            let (parent_matrix, current_id) = queue[head];
            head += 1;

            if let (Some(lq), Some(gq)) = (&mut local_query, &mut global_query) {
                if let (Some(local), Some(mut global)) = (lq.get(current_id), gq.get_mut(current_id)) {
                    // Same compare-then-write as the root loop. Note the compare is on the
                    // COMPOSED world matrix, not the local one: a child whose own `Transform`
                    // never moves still lands somewhere new when its parent does, and must be
                    // stamped for it (T18).
                    set_global_matrix(&mut global, parent_matrix * local.local_matrix);

                    if let Some(cq) = &mut children_query {
                        if let Some(children) = cq.get(current_id) {
                            for &child in &children.0 {
                                if world.is_alive(child) && visited.insert(child.id()) {
                                    queue.push((global.matrix, child));
                                }
                            }
                        }
                    }
                }
            }
        }

        // Aggregate: bu frame'de dünya-matrisi güncellenen düğüm sayısı (kök + çocuklar).
        tracing::trace!(nodes = visited.count, "transform_propagate: hiyerarşi dünya-matrisleri güncellendi");
    }
}

/// An entity's world matrix **now**, from its own `Transform` and its parents' — what its
/// `GlobalTransform` will hold once this frame's propagation has run.
///
/// `GlobalTransform` is written by [`TransformPropagateSystem`], which runs after the update, so
/// in game code it is a frame behind: an entity spawned this frame still holds the default (the
/// origin), and a child of something that moved this frame is where it was. A collision test
/// against freshly spawned obstacles reads them all at the origin. This walks up the [`Parent`]
/// links and composes the local transforms from their fields, so it sees this frame's writes.
///
/// `None` when the entity is dead or has no `Transform`, or when the chain up from it cannot be
/// followed the way propagation would: an ancestor with no `Transform`, a `Parent` naming a dead
/// id, or a cycle. Cost: one lookup per ancestor; for many entities a frame, read
/// `GlobalTransform` instead and accept the frame.
///
/// [`Parent`]: gizmo_core::component::Parent
///
/// ```
/// # use gizmo::prelude::*;
/// # use gizmo::core::hierarchy::HierarchyExt;
/// let mut world = World::new();
/// let board = world.spawn_bundle((Transform::new(Vec3::new(10.0, 0.0, 0.0)), GlobalTransform::default()));
/// let peg = world.spawn_bundle((Transform::new(Vec3::new(0.0, 1.0, 0.0)), GlobalTransform::default()));
/// world.add_child(board, peg);
/// // Spawned this frame: no propagation has run, but its place is known.
/// assert_eq!(gizmo::systems::world_position(&world, peg), Some(Vec3::new(10.0, 1.0, 0.0)));
/// ```
pub fn world_matrix(world: &gizmo_core::world::World, entity: gizmo_core::entity::Entity) -> Option<crate::math::Mat4> {
    use gizmo_core::component::Parent;
    use gizmo_physics_core::Transform;
    /// Deeper than any real hierarchy; past it the chain is taken for a cycle (which is how a
    /// cycle ends: no visited set, since a real chain is a handful of links).
    const MAX_DEPTH: usize = 1024;
    let local = |t: &Transform| crate::math::Mat4::from_scale_rotation_translation(t.scale, t.rotation, t.position);
    let mut matrix = local(world.get::<Transform>(entity)?);
    let mut current = entity;
    for _ in 0..MAX_DEPTH {
        let Some(parent) = world.get::<Parent>(current) else {
            return Some(matrix);
        };
        let parent = Some(parent.0).filter(|p| world.is_alive(*p))?;
        matrix = local(world.get::<Transform>(parent)?) * matrix;
        current = parent;
    }
    None
}

/// The translation of [`world_matrix`]: where the entity is in the world now.
pub fn world_position(world: &gizmo_core::world::World, entity: gizmo_core::entity::Entity) -> Option<crate::math::Vec3> {
    world_matrix(world, entity).map(|m| m.w_axis.truncate())
}

/// Drives entities that follow a skeleton bone.
///
/// Gated on `render`: `BoneAttachment`/`Skeleton` are renderer components (skinning lives on
/// the GPU side), so this system has nothing to read without the renderer. Transform sync and
/// hierarchy propagation above are renderer-independent and stay available headless.
#[cfg(feature = "render")]
pub struct BoneAttachmentSystem;

#[cfg(feature = "render")]
impl gizmo_core::system::System for BoneAttachmentSystem {
    fn access_info(&self) -> gizmo_core::system::AccessInfo {
        let mut info = gizmo_core::system::AccessInfo::new();
        info.is_exclusive = true;
        info
    }

    #[tracing::instrument(skip_all, level = "trace", name = "bone_attachment")]
    fn run(&mut self, world: &gizmo_core::world::World, _dt: f32) {
        let mut attached = 0usize;
        if let Some(query) = world.query::<(Entity, &gizmo_renderer::components::BoneAttachment)>() {
            let mut skeletons = world.query::<&gizmo_renderer::components::Skeleton>();
            // SAFETY: scheduled system; scheduler guarantees disjoint mutable access.
            let mut transforms = unsafe {
                world.query_unchecked::<gizmo_core::query::Mut<gizmo_physics_core::Transform>>()
            };

            for (entity, attachment) in query.iter() {
                if let Some(sq) = &mut skeletons {
                    if let Some(skeleton) = sq.get(attachment.target_entity) {
                        if let Some(global_matrix) = skeleton.global_poses.get(attachment.bone_index) {
                            if let Some(tq) = &mut transforms {
                                if let Some(mut trans) = tq.get_mut(entity) {
                                    let final_mat = *global_matrix * attachment.offset;
                                    let (t, r, s) = gizmo_renderer::decompose_mat4(final_mat);
                                    trans.position = t;
                                    trans.rotation = r;
                                    trans.scale = s;
                                    trans.update_local_matrix();
                                    attached += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
        if attached > 0 {
            tracing::trace!(attached, "bone_attachment: kemiğe bağlı varlıklar güncellendi");
        }
    }
}

#[cfg(test)]
mod tests {
    use gizmo_core::Entity;
    use super::*;
    use gizmo_core::component::Children;
    use gizmo_core::system::System;
    use gizmo_core::world::World;
    use gizmo_physics_core::components::GlobalTransform;
    use gizmo_physics_core::Transform;

    /// The world matrix through the parents, before any propagation has run: a child spawned
    /// this frame under a moved, turned, scaled parent is where propagation will put it — and
    /// after propagation `GlobalTransform` agrees exactly.
    #[test]
    fn world_matrix_is_this_frames_place_through_the_parents() {
        use gizmo_core::hierarchy::HierarchyExt;
        use gizmo_math::{Quat, Vec3};
        let mut world = World::new();
        let root = world.spawn();
        let arm = world.spawn();
        let hand = world.spawn();
        let at = |p: Vec3| Transform::new(p);
        world.add_component(root, at(Vec3::new(10.0, 0.0, 0.0)).with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)).with_scale(Vec3::splat(2.0)));
        world.add_component(arm, at(Vec3::new(1.0, 0.0, 0.0)));
        world.add_component(hand, at(Vec3::new(0.0, 0.5, 0.0)));
        for e in [root, arm, hand] {
            world.add_component(e, GlobalTransform::default());
        }
        world.add_child(root, arm);
        world.add_child(arm, hand);

        // Not propagated yet: GlobalTransform is the default, world_position already knows.
        let hand_now = world_position(&world, hand).expect("a live chain");
        // root turns +x into -z and doubles: arm at (10, 0, -2), hand 1 m above it.
        assert!((hand_now - Vec3::new(10.0, 1.0, -2.0)).length() < 1e-5, "{hand_now:?}");
        assert_eq!(world.get::<GlobalTransform>(hand).map(|g| g.matrix), Some(gizmo_math::Mat4::IDENTITY));

        TransformSyncSystem.run(&world, 0.0);
        TransformPropagateSystem.run(&world, 0.0);
        for e in [root, arm, hand] {
            let g = world.get::<GlobalTransform>(e).expect("global").matrix;
            let m = world_matrix(&world, e).expect("matrix");
            assert!(g.abs_diff_eq(m, 1e-5), "propagation and the walk disagree on {e:?}: {g:?} vs {m:?}");
        }

        // A parent moved this frame: the walk sees it, GlobalTransform is a frame behind.
        if let Some(mut q) = world.query_mut::<gizmo_core::query::Mut<Transform>>() {
            if let Some(mut t) = q.get_mut(root) {
                t.position.y = 5.0;
            }
        }
        assert!((world_position(&world, hand).unwrap().y - 6.0).abs() < 1e-5);
        assert!((world.get::<GlobalTransform>(hand).unwrap().matrix.w_axis.y - 1.0).abs() < 1e-5);
    }

    /// No answer where propagation would give none: no `Transform`, an ancestor without one, a
    /// `Parent` naming a dead id, a cycle, a dead entity.
    #[test]
    fn world_matrix_is_none_where_the_chain_breaks() {
        use gizmo_core::component::Parent;
        let mut world = World::new();
        let bare = world.spawn();
        assert_eq!(world_matrix(&world, bare), None, "no Transform");

        let (a, b) = (world.spawn(), world.spawn());
        world.add_component(a, Transform::default());
        world.add_component(b, Transform::default());
        world.add_component(a, Parent(bare));
        assert_eq!(world_matrix(&world, a), None, "an ancestor without a Transform");

        world.add_component(a, Parent(b));
        world.add_component(b, Parent(a));
        assert_eq!(world_matrix(&world, a), None, "a cycle through the entity");
        let c = world.spawn();
        world.add_component(c, Transform::default());
        world.add_component(c, Parent(a));
        assert_eq!(world_matrix(&world, c), None, "a cycle above the entity");

        // `d` first: spawned after the despawn it would recycle the dead id and name itself.
        let d = world.spawn();
        let gone = world.spawn();
        world.despawn(gone);
        world.add_component(d, Transform::default());
        world.add_component(d, Parent(gone));
        assert_eq!(world_matrix(&world, d), None, "a Parent naming a dead id");

        world.despawn(d);
        assert_eq!(world_matrix(&world, d), None, "a dead entity");
    }

    #[test]
    fn transform_propagate_terminates_on_children_cycle() {
        let mut world = World::new();
        let a = world.spawn();
        let b = world.spawn();
        world.add_component(a, Transform::default());
        world.add_component(a, GlobalTransform::default());
        world.add_component(b, Transform::default());
        world.add_component(b, GlobalTransform::default());
        // A `Children` cycle with neither node parented (both are propagation roots).
        // The old BFS had no visited set → its queue grew forever and this per-frame
        // system hung the whole app. Completing at all is the assertion.
        world.add_component(a, Children(vec![b]));
        world.add_component(b, Children(vec![a]));

        let mut sys = TransformPropagateSystem;
        sys.run(&world, 0.0);
    }

    // ── Change detection (T16–T18) ───────────────────────────────────────────
    //
    // `Mut::deref_mut` stamps `ticks.changed` on EVERY write, so an unconditional
    // `global.matrix = …` here made `Changed<GlobalTransform>` match 100% of the scene
    // every frame — the filter matched everything instead of matching the movers, which
    // makes every `Changed`-driven consumer (the render visibility index above all) pay
    // full price. These three pin the compare-then-write that fixes it, from both sides:
    // T16 that it suppresses, T17/T18 that it does not over-suppress. A missed stamp is
    // geometry that never refreshes its indexed box, i.e. an object that silently
    // disappears — so the two directions are not equally cheap to get wrong.

    /// A hierarchy of `roots` roots, each with one child and one grandchild.
    fn spawn_chain(world: &mut World, roots: usize) -> Vec<[Entity; 3]> {
        let mut out = Vec::with_capacity(roots);
        for i in 0..roots {
            let r = world.spawn();
            let c = world.spawn();
            let g = world.spawn();
            for e in [r, c, g] {
                world.add_component(e, Transform::default());
                world.add_component(e, GlobalTransform::default());
            }
            // Offset each root so the chain is not all-identity (identity would make the
            // "unchanged" compare trivially true for the wrong reason).
            if let Some(mut q) = world.query_mut::<gizmo_core::query::Mut<Transform>>() {
                if let Some(mut t) = q.get_mut(r) {
                    t.position = gizmo_math::Vec3::new(i as f32, 1.0, 2.0);
                    t.update_local_matrix();
                }
                if let Some(mut t) = q.get_mut(c) {
                    t.position = gizmo_math::Vec3::new(0.0, 3.0, 0.0);
                    t.update_local_matrix();
                }
                if let Some(mut t) = q.get_mut(g) {
                    t.position = gizmo_math::Vec3::new(0.0, 0.0, 4.0);
                    t.update_local_matrix();
                }
            }
            world.add_component(r, Children(vec![c]));
            world.add_component(c, Children(vec![g]));
            // `Parent` is what keeps the child off the ROOT query — without it the child is
            // both a root and a child and gets written twice per frame.
            world.add_component(c, gizmo_core::component::Parent(r));
            world.add_component(g, gizmo_core::component::Parent(c));
            out.push([r, c, g]);
        }
        out
    }

    fn changed_globals(world: &World) -> Vec<Entity> {
        let mut ids: Vec<Entity> = world
            .query::<(Entity, gizmo_core::query::Changed<GlobalTransform>)>()
            .map(|q| q.iter().map(|(entity, _)| entity).collect())
            .unwrap_or_default();
        ids.sort_unstable();
        ids
    }

    /// Advance to a fresh change frame, then run propagation inside it.
    fn propagate_in_new_frame(world: &mut World) {
        let prev = world.tick;
        world.begin_change_frame(prev);
        let mut sys = TransformPropagateSystem;
        sys.run(world, 0.0);
    }

    /// T16 — the prerequisite the whole visibility index rests on: a scene that did not move
    /// must produce ZERO `Changed<GlobalTransform>` rows on the next frame.
    ///
    /// Before the compare-then-write this returned every node, every frame.
    #[test]
    fn propagate_does_not_stamp_an_unmoved_global_transform() {
        let mut world = World::new();
        let chains = spawn_chain(&mut world, 40);
        let total = chains.len() * 3;

        // Frame 1 — first propagation genuinely writes every node.
        propagate_in_new_frame(&mut world);
        assert_eq!(
            changed_globals(&world).len(),
            total,
            "premise: the first propagation must stamp everything (the matrices really do change)"
        );

        // Frame 2 — nothing moved.
        propagate_in_new_frame(&mut world);
        assert_eq!(
            changed_globals(&world),
            Vec::<Entity>::new(),
            "an unmoved hierarchy must not stamp GlobalTransform: `Changed` matching 100% of \
             the scene is what makes a change-driven refresh cost the same as no filter at all"
        );
    }

    /// The same compare-then-write, for the sync: a `Transform` nobody touched is not stamped
    /// changed by the pass that rebuilds its cached matrix (it was, every frame, for every
    /// entity) — and one whose fields moved still gets its matrix rebuilt.
    #[test]
    fn sync_rebuilds_a_moved_matrix_and_does_not_stamp_an_unmoved_transform() {
        let mut world = World::new();
        let still = world.spawn();
        let moving = world.spawn();
        world.add_component(still, Transform::new(gizmo_math::Vec3::new(1.0, 2.0, 3.0)));
        world.add_component(moving, Transform::new(gizmo_math::Vec3::ZERO));
        let changed = |w: &World| -> Vec<u32> {
            let mut v: Vec<u32> = w
                .query::<(Entity, gizmo_core::query::Changed<Transform>)>()
                .map(|q| q.iter().map(|(entity, _)| entity.id()).collect())
                .unwrap_or_default();
            v.sort_unstable();
            v
        };
        let prev = world.tick;
        world.begin_change_frame(prev);
        TransformSyncSystem.run(&world, 0.0);
        // Frame 2: move one by its fields only — the matrix is stale until the sync.
        let prev = world.tick;
        world.begin_change_frame(prev);
        if let Some(mut q) = world.query_mut::<gizmo_core::query::Mut<Transform>>() {
            q.get_mut(moving).unwrap().position = gizmo_math::Vec3::new(5.0, 0.0, 0.0);
        }
        TransformSyncSystem.run(&world, 0.0);
        assert_eq!(changed(&world), vec![moving.id()], "the sync stamped a transform nobody moved");
        let m = world.borrow::<Transform>().get(moving).unwrap().local_matrix;
        assert_eq!(m.w_axis.truncate(), gizmo_math::Vec3::new(5.0, 0.0, 0.0), "the moved matrix was not rebuilt");
    }

    /// The visited bitset answers like `HashSet::insert`: fresh once, then not; ids in different
    /// words and far apart do not disturb each other; the count is of distinct ids.
    #[test]
    fn the_visited_bitset_answers_like_a_set() {
        let mut v = VisitedIds::default();
        assert!(v.insert(3));
        assert!(!v.insert(3));
        assert!(v.insert(64));
        assert!(v.insert(100_000));
        assert!(!v.insert(64));
        assert!(v.insert(63));
        assert_eq!(v.count, 4);
    }

    /// T17 — the other direction: a node that DID move must still be stamped, and its
    /// siblings must not be. Over-eager suppression here is the invisible-geometry bug.
    #[test]
    fn propagate_stamps_a_moved_global_transform() {
        let mut world = World::new();
        let chains = spawn_chain(&mut world, 5);
        propagate_in_new_frame(&mut world);
        propagate_in_new_frame(&mut world);
        assert!(changed_globals(&world).is_empty(), "premise: settled");

        // Move ONE leaf (a grandchild — no children of its own, so exactly one node moves).
        let moved = chains[2][2];
        if let Some(mut q) = world.query_mut::<gizmo_core::query::Mut<Transform>>() {
            if let Some(mut t) = q.get_mut(moved) {
                t.position.x += 10.0;
                t.update_local_matrix();
            }
        }

        propagate_in_new_frame(&mut world);
        assert_eq!(
            changed_globals(&world),
            vec![moved],
            "exactly the entity that moved must be stamped — no more, and above all no fewer"
        );
    }

    /// T18 — the compare is on the COMPUTED WORLD matrix, not the local one: a child whose
    /// own `Transform` is untouched still lands somewhere new when its parent moves.
    ///
    /// Comparing locals instead would leave every descendant of a moving object stale.
    #[test]
    fn a_moved_parent_restamps_its_unmoved_children() {
        let mut world = World::new();
        let chains = spawn_chain(&mut world, 3);
        propagate_in_new_frame(&mut world);
        propagate_in_new_frame(&mut world);
        assert!(changed_globals(&world).is_empty(), "premise: settled");

        let [root, child, grand] = chains[1];
        if let Some(mut q) = world.query_mut::<gizmo_core::query::Mut<Transform>>() {
            if let Some(mut t) = q.get_mut(root) {
                t.position.z -= 25.0;
                t.update_local_matrix();
            }
        }

        propagate_in_new_frame(&mut world);
        let mut expect = vec![root, child, grand];
        expect.sort_unstable();
        assert_eq!(
            changed_globals(&world),
            expect,
            "a moving root must restamp its whole subtree: the child's LOCAL matrix is \
             unchanged, its WORLD matrix is not"
        );
    }
}
