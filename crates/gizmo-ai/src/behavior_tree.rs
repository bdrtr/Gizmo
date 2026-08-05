//! Behaviour trees: composable, stateless decision nodes.
//!
//! A tree is built from [`Sequence`], [`Selector`] and [`Action`] nodes and evaluated by
//! calling [`BtNode::tick`] on the root, once per decision update. Each tick walks the tree
//! from the top and returns a [`BtStatus`]; no node keeps a cursor between ticks, so a
//! long-running branch re-walks its already-succeeded children every time.
//!
//! Nodes take `&World` and a [`gizmo_core::entity::Entity`], so a tree can read the world but
//! cannot mutate it during the tick — an action that wants to change state records its intent
//! and lets a system apply it.

use gizmo_core::World;

/// Return status of a Behavior Tree node
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BtStatus {
    /// The node completed its work during this tick. A [`Sequence`] advances to
    /// its next child; a [`Selector`] stops there and reports success.
    Success,
    /// The node could not complete. A [`Sequence`] aborts and restarts from its
    /// first child on the next tick; a [`Selector`] moves on to its next child.
    /// This is also what a [`BehaviorTree`] with no root reports.
    Failure,
    /// The node has not finished and wants to be ticked again next frame.
    /// [`Sequence`] and [`Selector`] remember which child was running and resume
    /// there rather than re-ticking earlier siblings; [`Inverter`] passes this
    /// status through unchanged instead of flipping it.
    Running,
}

/// A node in the Behavior Tree.
///
/// This trait is a deliberate **extension point**: downstream crates and users
/// are expected to implement their own custom `BtNode` types alongside the
/// built-in [`Sequence`], [`Selector`], [`Inverter`], [`Action`] and
/// [`Condition`] nodes. It is therefore intentionally **not** sealed.
pub trait BtNode: Send + Sync {
    /// Advances this node by one frame and reports what happened.
    ///
    /// `entity` is the id of the agent that owns the tree. `world` is borrowed
    /// exclusively, which lets a node read and mutate any component while it runs.
    /// `dt` is the time elapsed since the previous tick, in seconds.
    ///
    /// Returning [`BtStatus::Running`] means "call me again next tick"; any
    /// progress worth keeping must be stored inside the node itself, as nothing
    /// re-creates the tree between ticks. Ticking is synchronous and recurses into
    /// children, so an implementation is expected to return promptly and tree
    /// depth is bounded only by the stack.
    fn tick(&mut self, entity: u32, world: &mut World, dt: f32) -> BtStatus;
}

/// Sequence (logical AND): ticks its children front to back until one fails or is
/// still running.
///
/// Reports [`BtStatus::Success`] once every child has succeeded, so an empty
/// sequence succeeds immediately. A child's [`BtStatus::Failure`] aborts the whole
/// sequence and rewinds it, so the next tick starts again from the first child.
///
/// This is a *memory* sequence: when a child reports [`BtStatus::Running`] the
/// position is kept and the next tick resumes at that child, without re-ticking
/// the children that already succeeded. There is no reset method, so a sequence
/// abandoned mid-run keeps its resume position until it next fails or completes.
pub struct Sequence {
    children: Vec<Box<dyn BtNode>>,
    current_idx: usize,
}

impl Sequence {
    /// Builds a sequence over `children`, ticked in slice order. The child list is
    /// fixed here: `Sequence` offers no way to add, remove, replace or inspect a child
    /// afterwards, so reordering the steps means constructing a new node.
    ///
    /// Starts at the first child. An empty `children` vector produces a node that
    /// succeeds on every tick without doing anything.
    pub fn new(children: Vec<Box<dyn BtNode>>) -> Self {
        Self {
            children,
            current_idx: 0,
        }
    }
}

impl BtNode for Sequence {
    fn tick(&mut self, entity: u32, world: &mut World, dt: f32) -> BtStatus {
        while self.current_idx < self.children.len() {
            let status = self.children[self.current_idx].tick(entity, world, dt);
            match status {
                BtStatus::Success => {
                    self.current_idx += 1; // Move to next child
                }
                BtStatus::Failure => {
                    self.current_idx = 0; // Reset for next tick
                    return BtStatus::Failure;
                }
                BtStatus::Running => {
                    return BtStatus::Running;
                }
            }
        }
        self.current_idx = 0; // Reset
        BtStatus::Success
    }
}

/// Selector / fallback (logical OR): ticks its children front to back until one
/// succeeds or is still running.
///
/// Reports [`BtStatus::Failure`] only after every child has failed, so an empty
/// selector fails immediately. Like [`Sequence`] it remembers a running child and
/// resumes there on the next tick — which means it is **not** a reactive
/// selector: while a lower-priority child is running, the earlier, higher-priority
/// children are not re-evaluated, so a condition that becomes true again cannot
/// preempt the running branch.
pub struct Selector {
    children: Vec<Box<dyn BtNode>>,
    current_idx: usize,
}

impl Selector {
    /// Builds a selector over `children`, ticked in slice order — earlier children
    /// are the higher-priority alternatives.
    ///
    /// Starts at the first child. An empty `children` vector produces a node that
    /// fails on every tick.
    pub fn new(children: Vec<Box<dyn BtNode>>) -> Self {
        Self {
            children,
            current_idx: 0,
        }
    }
}

impl BtNode for Selector {
    fn tick(&mut self, entity: u32, world: &mut World, dt: f32) -> BtStatus {
        while self.current_idx < self.children.len() {
            let status = self.children[self.current_idx].tick(entity, world, dt);
            match status {
                BtStatus::Success => {
                    self.current_idx = 0; // Reset
                    return BtStatus::Success;
                }
                BtStatus::Failure => {
                    self.current_idx += 1; // Try next child
                }
                BtStatus::Running => {
                    return BtStatus::Running;
                }
            }
        }
        self.current_idx = 0; // Reset
        BtStatus::Failure
    }
}

/// Decorator with exactly one child that swaps [`BtStatus::Success`] and
/// [`BtStatus::Failure`]. [`BtStatus::Running`] passes through untouched, so
/// inverting a long-running action does not invert its "not finished yet" signal.
///
/// The inverter keeps no state of its own; any resume behaviour belongs to the
/// child.
pub struct Inverter {
    child: Box<dyn BtNode>,
}

impl Inverter {
    /// Takes ownership of `child`. The child cannot be inspected or replaced
    /// afterwards — rebuild the node to change it.
    pub fn new(child: Box<dyn BtNode>) -> Self {
        Self { child }
    }
}

impl BtNode for Inverter {
    fn tick(&mut self, entity: u32, world: &mut World, dt: f32) -> BtStatus {
        match self.child.tick(entity, world, dt) {
            BtStatus::Success => BtStatus::Failure,
            BtStatus::Failure => BtStatus::Success,
            BtStatus::Running => BtStatus::Running,
        }
    }
}

/// Leaf node that runs a closure and reports the closure's [`BtStatus`] verbatim.
///
/// The closure is `FnMut`, so it may keep state between ticks — a timer, a
/// progress counter — which is the usual way to write a long-running action that
/// returns [`BtStatus::Running`] until it is done. It receives the same
/// `(entity, &mut World, dt)` triple as [`BtNode::tick`], with `dt` in seconds.
pub struct Action<F>
where
    F: FnMut(u32, &mut World, f32) -> BtStatus + Send + Sync,
{
    func: F,
}

impl<F> Action<F>
where
    F: FnMut(u32, &mut World, f32) -> BtStatus + Send + Sync,
{
    /// Wraps `func` by value. `Action<F>` is generic over the closure rather than
    /// boxing it, so putting one into a composite's `Vec<Box<dyn BtNode>>` needs an
    /// explicit `Box::new` at the call site. The `Send + Sync` bound sits on the
    /// closure and therefore on everything it captures — a captured `Rc` or `RefCell`
    /// will not compile.
    pub fn new(func: F) -> Self {
        Self { func }
    }
}

impl<F> BtNode for Action<F>
where
    F: FnMut(u32, &mut World, f32) -> BtStatus + Send + Sync,
{
    fn tick(&mut self, entity: u32, world: &mut World, dt: f32) -> BtStatus {
        (self.func)(entity, world, dt)
    }
}

/// Leaf node that evaluates a predicate: `true` becomes [`BtStatus::Success`],
/// `false` becomes [`BtStatus::Failure`].
///
/// A condition can never report [`BtStatus::Running`] — it always resolves within
/// the tick. Its closure is given `&mut World`, so side effects are possible even
/// though a condition is meant to be a test; keeping it side-effect free is
/// advisable, since composites re-tick it on later frames.
pub struct Condition<F>
where
    F: FnMut(u32, &mut World) -> bool + Send + Sync,
{
    func: F,
}

impl<F> Condition<F>
where
    F: FnMut(u32, &mut World) -> bool + Send + Sync,
{
    /// Wraps the predicate by value.
    ///
    /// Note the closure signature takes no `dt`: the tick's delta time is dropped
    /// here. A test that needs elapsed time must read it from the world or be
    /// written as an [`Action`] instead.
    pub fn new(func: F) -> Self {
        Self { func }
    }
}

impl<F> BtNode for Condition<F>
where
    F: FnMut(u32, &mut World) -> bool + Send + Sync,
{
    fn tick(&mut self, entity: u32, world: &mut World, _dt: f32) -> BtStatus {
        if (self.func)(entity, world) {
            BtStatus::Success
        } else {
            BtStatus::Failure
        }
    }
}

/// ECS component holding one agent's behavior tree.
///
/// Ticked either in bulk by [`behavior_tree_system`] or directly through
/// [`BehaviorTree::tick`]. Cloning this component does **not** copy the tree; see
/// the `Clone` implementation below.
pub struct BehaviorTree {
    /// Root node of the tree, or `None` for an empty tree, which ticks to
    /// [`BtStatus::Failure`].
    ///
    /// [`behavior_tree_system`] moves the root *out* of this field for the
    /// duration of a tick and puts it back afterwards, so a node that reaches its
    /// own entity's `BehaviorTree` while running observes `None` here. If the
    /// component or its entity disappears during that tick, the root is dropped
    /// rather than restored.
    pub root: Option<Box<dyn BtNode>>,
}

impl gizmo_core::component::Component for BehaviorTree {}

impl Clone for BehaviorTree {
    /// A `BehaviorTree` holds boxed trait objects (`Box<dyn BtNode>`) whose leaf
    /// nodes can capture arbitrary non-`Clone` closures, so a deep clone is not
    /// generally possible. Instead of panicking (which would violate the `Clone`
    /// contract and could be triggered silently by ECS entity/prefab cloning via
    /// the `Component: Clone` bound), cloning yields a fresh, empty tree.
    ///
    /// An empty tree is handled gracefully by [`BehaviorTree::tick`], which
    /// returns [`BtStatus::Failure`] when `root` is `None`. Callers that need the
    /// behavior preserved on the cloned entity should re-attach a tree explicitly.
    fn clone(&self) -> Self {
        Self { root: None }
    }
}

impl BehaviorTree {
    /// Wraps an already-built node as the tree's root, taking ownership of it and
    /// of the whole subtree hanging off it.
    ///
    /// Nodes carry their own resume state, so a node that was already ticked keeps
    /// that state when it becomes a root here — it is not reset.
    pub fn new(root: Box<dyn BtNode>) -> Self {
        Self { root: Some(root) }
    }

    /// Ticks the root once and returns its status, or [`BtStatus::Failure`] when
    /// this tree is empty — a missing tree is reported as a failed one, never as
    /// success.
    ///
    /// `dt` is the frame time in seconds and is handed down unchanged to every
    /// node. The call recurses through the tree; nothing here bounds the depth or
    /// catches a panic raised by a node.
    pub fn tick(&mut self, entity: u32, world: &mut World, dt: f32) -> BtStatus {
        if let Some(root) = &mut self.root {
            root.tick(entity, world, dt)
        } else {
            BtStatus::Failure
        }
    }
}

/// Ticks every entity that carries a [`BehaviorTree`] exactly once.
///
/// The entity list is snapshotted before any node runs, so trees attached during
/// this call are not ticked until the next one, and an entity whose component
/// vanished in the meantime is skipped silently. Iteration order is whatever the
/// ECS query yields; treat it as unspecified.
///
/// For each entity the root node is *moved out* of the component, ticked with
/// exclusive access to the world, then moved back. That is what allows a node to
/// mutate arbitrary components, but it has two consequences: during its own tick
/// the entity's [`BehaviorTree::root`] reads as `None`, and if the node removes
/// that component or despawns its own entity, the root is dropped instead of
/// restored and the tree is lost.
///
/// `dt` is the frame time in seconds, passed unchanged to every node. Each
/// entity's status is emitted at `trace` level and also folded into per-frame
/// counters logged at `debug`; the system itself returns nothing, reports no
/// errors, and a node's panic propagates out of it.
#[tracing::instrument(skip_all, name = "behavior_tree_system")]
pub fn behavior_tree_system(world: &mut World, dt: f32) {
    let entities: Vec<u32> = {
        let trees = world.borrow::<BehaviorTree>();
        trees.entities().collect()
    };

    let tree_count = entities.len();
    // Frame-başı toplu sayaçlar — iç döngüde per-tick log yerine çıkışta AGGREGATE.
    let mut success = 0u32;
    let mut failure = 0u32;
    let mut running = 0u32;
    let mut empty = 0u32;

    for entity in entities {
        let mut root_opt = None;
        if let Some(mut tree) = world.borrow_mut::<BehaviorTree>().get_mut(entity) {
            root_opt = tree.root.take();
        }

        if let Some(mut root) = root_opt {
            // Kök karar döngüsünün sonucu — per-entity SICAK yol, bu yüzden trace!.
            let status = root.tick(entity, world, dt);
            match status {
                BtStatus::Success => success += 1,
                BtStatus::Failure => failure += 1,
                BtStatus::Running => running += 1,
            }
            tracing::trace!(entity, ?status, "[AI] Davranış ağacı köke kadar tick'lendi");

            if let Some(mut tree) = world.borrow_mut::<BehaviorTree>().get_mut(entity) {
                tree.root = Some(root);
            }
        } else {
            // Boş ağaç (root None) — tick Failure döner; sık olabileceğinden yalnız say.
            empty += 1;
        }
    }

    if tree_count > 0 {
        tracing::debug!(
            tree_count,
            success,
            failure,
            running,
            empty,
            "[AI] Davranış ağaçları tick'lendi"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn leaf(status: BtStatus) -> Box<dyn BtNode> {
        Box::new(Action::new(move |_, _, _| status))
    }

    fn tick(node: &mut dyn BtNode) -> BtStatus {
        let mut w = World::new();
        node.tick(0, &mut w, 0.0)
    }

    #[test]
    fn sequence_succeeds_only_when_all_children_succeed() {
        assert_eq!(
            tick(&mut Sequence::new(vec![leaf(BtStatus::Success), leaf(BtStatus::Success)])),
            BtStatus::Success
        );
        assert_eq!(
            tick(&mut Sequence::new(vec![leaf(BtStatus::Success), leaf(BtStatus::Failure)])),
            BtStatus::Failure
        );
        // Vacuous: an empty sequence succeeds.
        assert_eq!(tick(&mut Sequence::new(vec![])), BtStatus::Success);
    }

    #[test]
    fn selector_succeeds_on_first_success_else_fails() {
        assert_eq!(
            tick(&mut Selector::new(vec![leaf(BtStatus::Failure), leaf(BtStatus::Success)])),
            BtStatus::Success
        );
        assert_eq!(
            tick(&mut Selector::new(vec![leaf(BtStatus::Failure), leaf(BtStatus::Failure)])),
            BtStatus::Failure
        );
        // Empty selector fails (no fallback succeeded).
        assert_eq!(tick(&mut Selector::new(vec![])), BtStatus::Failure);
    }

    #[test]
    fn inverter_flips_success_and_failure_but_not_running() {
        assert_eq!(tick(&mut Inverter::new(leaf(BtStatus::Success))), BtStatus::Failure);
        assert_eq!(tick(&mut Inverter::new(leaf(BtStatus::Failure))), BtStatus::Success);
        assert_eq!(tick(&mut Inverter::new(leaf(BtStatus::Running))), BtStatus::Running);
    }

    #[test]
    fn empty_behavior_tree_ticks_to_failure() {
        let mut w = World::new();
        let mut bt = BehaviorTree { root: None };
        assert_eq!(bt.tick(0, &mut w, 0.0), BtStatus::Failure);
        // Clone yields an empty tree (documented), which also ticks to Failure.
        let mut cloned = bt.clone();
        assert_eq!(cloned.tick(0, &mut w, 0.0), BtStatus::Failure);
    }

    #[test]
    fn sequence_resumes_at_running_child_without_reticking_earlier_ones() {
        // child0 succeeds (count its ticks); child1 runs on the first tick, then
        // succeeds. The memory sequence must NOT re-tick child0 on resume.
        let c0 = Arc::new(AtomicUsize::new(0));
        let c0c = c0.clone();
        let child0 = Box::new(Action::new(move |_, _, _| {
            c0c.fetch_add(1, Ordering::SeqCst);
            BtStatus::Success
        }));
        let mut runs = 0u32;
        let child1 = Box::new(Action::new(move |_, _, _| {
            runs += 1;
            if runs == 1 {
                BtStatus::Running
            } else {
                BtStatus::Success
            }
        }));
        let mut seq = Sequence::new(vec![child0, child1]);
        let mut w = World::new();

        assert_eq!(seq.tick(0, &mut w, 0.0), BtStatus::Running, "first tick: child1 is running");
        assert_eq!(seq.tick(0, &mut w, 0.0), BtStatus::Success, "second tick: child1 completes");
        assert_eq!(
            c0.load(Ordering::SeqCst),
            1,
            "memory sequence re-ticked an already-succeeded child on resume"
        );
    }
}
