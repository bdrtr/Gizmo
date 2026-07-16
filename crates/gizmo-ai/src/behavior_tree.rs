use gizmo_core::World;

/// Return status of a Behavior Tree node
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BtStatus {
    Success,
    Failure,
    Running,
}

/// A node in the Behavior Tree.
///
/// This trait is a deliberate **extension point**: downstream crates and users
/// are expected to implement their own custom `BtNode` types alongside the
/// built-in [`Sequence`], [`Selector`], [`Inverter`], [`Action`] and
/// [`Condition`] nodes. It is therefore intentionally **not** sealed.
pub trait BtNode: Send + Sync {
    /// Executes the node's logic.
    fn tick(&mut self, entity: u32, world: &mut World, dt: f32) -> BtStatus;
}

/// Sequence: Runs children in order until one Fails or Runs.
/// Returns Success if ALL children succeed.
pub struct Sequence {
    children: Vec<Box<dyn BtNode>>,
    current_idx: usize,
}

impl Sequence {
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

/// Selector (Fallback): Runs children in order until one Succeeds or Runs.
/// Returns Failure if ALL children fail.
pub struct Selector {
    children: Vec<Box<dyn BtNode>>,
    current_idx: usize,
}

impl Selector {
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

/// Inverter: Inverts Success and Failure. Running stays Running.
pub struct Inverter {
    child: Box<dyn BtNode>,
}

impl Inverter {
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

/// Action Node: A leaf node that performs a concrete action.
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

/// Condition Node: A leaf node that checks a condition (returns Success or Failure).
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

/// BehaviorTree Component attached to an Entity
pub struct BehaviorTree {
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
    pub fn new(root: Box<dyn BtNode>) -> Self {
        Self { root: Some(root) }
    }

    pub fn tick(&mut self, entity: u32, world: &mut World, dt: f32) -> BtStatus {
        if let Some(root) = &mut self.root {
            root.tick(entity, world, dt)
        } else {
            BtStatus::Failure
        }
    }
}

/// The System that ticks all BehaviorTrees
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
