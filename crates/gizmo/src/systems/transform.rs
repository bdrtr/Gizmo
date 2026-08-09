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
        if let Some(mut transforms) = unsafe {
            world.query_unchecked::<gizmo_core::query::Mut<gizmo_physics_core::Transform>>()
        } {
            for (_, mut trans) in transforms.iter_mut() {
                trans.update_local_matrix();
                updated += 1;
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

pub struct TransformPropagateSystem;

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
        let mut visited: std::collections::HashSet<u32> = std::collections::HashSet::new();

        if let Some(mut roots) = root_query {
            let mut children_query = world.query::<&gizmo_core::component::Children>();
            for (id, (local, mut global, _)) in roots.iter_mut() {
                // Compare-then-write. `Mut::deref_mut` stamps `ticks.changed` on EVERY use,
                // so writing unconditionally marked every node of the hierarchy as changed
                // every single frame — `Changed<GlobalTransform>` then matched 100% of the
                // scene and was worth exactly nothing as a filter. Writing the identical
                // value through `bypass_change_detection` keeps the field correct and
                // leaves the tick alone. See T16–T18.
                set_global_matrix(&mut global, local.local_matrix);
                visited.insert(id);
                if let Some(children_q) = &mut children_query {
                    if let Some(children) = children_q.get(id) {
                        for &child_id in &children.0 {
                            if visited.insert(child_id) {
                                queue.push((global.matrix, child_id));
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
                            for &child_id in &children.0 {
                                if visited.insert(child_id) {
                                    queue.push((global.matrix, child_id));
                                }
                            }
                        }
                    }
                }
            }
        }

        // Aggregate: bu frame'de dünya-matrisi güncellenen düğüm sayısı (kök + çocuklar).
        tracing::trace!(nodes = visited.len(), "transform_propagate: hiyerarşi dünya-matrisleri güncellendi");
    }
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
        if let Some(query) = world.query::<&gizmo_renderer::components::BoneAttachment>() {
            let mut skeletons = world.query::<&gizmo_renderer::components::Skeleton>();
            // SAFETY: scheduled system; scheduler guarantees disjoint mutable access.
            let mut transforms = unsafe {
                world.query_unchecked::<gizmo_core::query::Mut<gizmo_physics_core::Transform>>()
            };

            for (id, attachment) in query.iter() {
                if let Some(sq) = &mut skeletons {
                    if let Some(skeleton) = sq.get(attachment.target_entity.id()) {
                        if let Some(global_matrix) = skeleton.global_poses.get(attachment.bone_index) {
                            if let Some(tq) = &mut transforms {
                                if let Some(mut trans) = tq.get_mut(id) {
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
    use super::*;
    use gizmo_core::component::Children;
    use gizmo_core::system::System;
    use gizmo_core::world::World;
    use gizmo_physics_core::components::GlobalTransform;
    use gizmo_physics_core::Transform;

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
        world.add_component(a, Children(vec![b.id()]));
        world.add_component(b, Children(vec![a.id()]));

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
    fn spawn_chain(world: &mut World, roots: usize) -> Vec<[u32; 3]> {
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
                if let Some(mut t) = q.get_mut(r.id()) {
                    t.position = gizmo_math::Vec3::new(i as f32, 1.0, 2.0);
                    t.update_local_matrix();
                }
                if let Some(mut t) = q.get_mut(c.id()) {
                    t.position = gizmo_math::Vec3::new(0.0, 3.0, 0.0);
                    t.update_local_matrix();
                }
                if let Some(mut t) = q.get_mut(g.id()) {
                    t.position = gizmo_math::Vec3::new(0.0, 0.0, 4.0);
                    t.update_local_matrix();
                }
            }
            world.add_component(r, Children(vec![c.id()]));
            world.add_component(c, Children(vec![g.id()]));
            // `Parent` is what keeps the child off the ROOT query — without it the child is
            // both a root and a child and gets written twice per frame.
            world.add_component(c, gizmo_core::component::Parent(r.id()));
            world.add_component(g, gizmo_core::component::Parent(c.id()));
            out.push([r.id(), c.id(), g.id()]);
        }
        out
    }

    fn changed_globals(world: &World) -> Vec<u32> {
        let mut ids: Vec<u32> = world
            .query::<gizmo_core::query::Changed<GlobalTransform>>()
            .map(|q| q.iter().map(|(id, _)| id).collect())
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
            Vec::<u32>::new(),
            "an unmoved hierarchy must not stamp GlobalTransform: `Changed` matching 100% of \
             the scene is what makes a change-driven refresh cost the same as no filter at all"
        );
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
