//! The spatial index, as a scene resource the draw path will use if it is there.
//!
//! # Why this is opt-in
//!
//! [`RenderAabbTree`](gizmo_renderer::visibility::RenderAabbTree) has been in the tree, complete
//! and tested, with no render path calling it — `CAPABILITY_GAPS.md` §F1 recorded it as unwired,
//! and the obvious fix was to put it in front of the batcher's per-object walk, which
//! `occlusion_culling` measures at **+8.46 ms** over 60 000 entities.
//!
//! Measuring it first said no, at least not unconditionally. Same demo, 60 000 cubes, tree built
//! once:
//!
//! | case | index query | linear walk | |
//! |---|---|---|---|
//! | nothing culled | **5.020 ms** | 1.864 ms | index **2.7× slower** |
//! | everything culled | **0.001 ms** | 1.241 ms | index **1240× faster** |
//!
//! A factor of a million between the ends. When the frustum rejects the root the query is over in
//! one plane test; when it accepts everything, the tree has to *write 60 000 keys into a `Vec`*,
//! and writing costs more than testing.
//!
//! Keeping it current costs too. Rebuilt every frame, even the fully-culled case falls to a 14 %
//! win (1.415 ms against the walk's 1.652 ms), and at 12 000 cubes the tree loses in both cases.
//! Those cubes never move, so that 1.4 ms is not re-insertion — it is 60 000 calls to
//! `insert`'s own "is the new box still inside the old leaf" shortcut.
//!
//! # And the shadow cascades decide it
//!
//! The batcher does not ask "what does the camera see". It asks for the union of the camera
//! frustum **and the four shadow cascades**, because an off-screen shadow caster still has to be
//! drawn into the shadow maps. Traced on the same 60 000-cube scene:
//!
//! | scene | candidates | query |
//! |---|---|---|
//! | sun on, all cubes in view | 60 000 | 12.3 ms |
//! | sun on, all cubes **behind the camera** | **60 000** | 5.4 ms |
//! | sun off, all cubes behind the camera | **0** | **0.005 ms** |
//!
//! With a shadow-casting sun the index culls **nothing**, in either case — the cascades cover the
//! scene, so their union accepts everything the camera rejected. It then pays to write 60 000 keys
//! into a `Vec` and sort them, which is why wiring it in made the frame *slower*: 18.84 → 35.75 ms
//! with everything visible, 19.73 → 27.23 ms with everything behind the camera.
//!
//! Without cascades the same scene culls completely and the frame goes **12.72 → 10.14 ms**, a
//! 20 % win.
//!
//! So the honest shape is a door with its conditions written on it. It pays when the scene is
//! static, the cull rate is high, **and the sun is not casting cascades over the whole of it**.
//! That last condition is the one that surprised me, and it is the one that decides most scenes —
//! `CAPABILITY_GAPS.md` §F1 previously read "an index in front of it is precisely what removes
//! that walk for culled objects", which is true of the camera frustum and false of the union the
//! batcher actually queries.
//!
//! # Use
//!
//! ```no_run
//! # use gizmo::systems::render::visibility_index::VisibilityIndex;
//! # use gizmo::prelude::*;
//! # fn demo(world: &mut World) {
//! // Once, after the static geometry is spawned:
//! let mut index = VisibilityIndex::default();
//! index.rebuild_from(world);
//! world.insert_resource(index);
//! # }
//! ```
//!
//! With the resource present, the batcher walks the query's candidates instead of every mesh.
//! **Anything not in the index is not drawn** — that is what makes it a cull — so an entity spawned
//! after the rebuild must be inserted, or it disappears. That is the cost of the door being a door;
//! see [`VisibilityIndex::rebuild_from`].

use crate::core::World;
use gizmo_physics_core::components::GlobalTransform;
use gizmo_renderer::components::Mesh;
use gizmo_renderer::visibility::RenderAabbTree;

/// A spatial index over the scene's renderable bounds, consulted by the batcher when present.
#[derive(Default)]
pub struct VisibilityIndex {
    /// The tree itself. Public so a game can `insert`/`remove` incrementally rather than rebuild.
    pub tree: RenderAabbTree,
}

crate::core::impl_component!(VisibilityIndex);

impl Clone for VisibilityIndex {
    fn clone(&self) -> Self {
        // `Component` requires `Clone` and a BVH is not cheap to copy. Nothing clones this — it is
        // inserted once and mutated in place — so the honest implementation is the one that says
        // so rather than one that silently costs a rebuild.
        unreachable!("VisibilityIndex is a resource and is never cloned")
    }
}

impl VisibilityIndex {
    /// Rebuilds the index from every entity that currently has a `Mesh` and a `GlobalTransform`.
    ///
    /// Call it after spawning static geometry, and again whenever that set changes. **An entity
    /// absent from the index is not drawn**, because the batcher treats the index's answer as the
    /// complete candidate list — that is what makes it a cull rather than a hint.
    ///
    /// Not called automatically anywhere: an index the engine kept current for you would pay the
    /// per-frame update cost this exists to avoid, which the module docs measure.
    pub fn rebuild_from(&mut self, world: &World) {
        self.tree.clear();
        if let Some(q) = world.query::<(&Mesh, &GlobalTransform)>() {
            for (e, (mesh, trans)) in q.iter() {
                let world_aabb = mesh.bounds.transform(&trans.matrix);
                self.tree.insert(e, world_aabb);
            }
        }
    }

    /// How many entities the index holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tree.len()
    }

    /// Whether the index is empty — in which case the batcher ignores it and walks everything,
    /// rather than drawing nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tree.is_empty()
    }
}
