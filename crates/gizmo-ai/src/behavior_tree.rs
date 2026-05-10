use gizmo_core::World;

/// Return status of a Behavior Tree node
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BtStatus {
    Success,
    Failure,
    Running,
}

/// A node in the Behavior Tree
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
    fn clone(&self) -> Self {
        panic!("BehaviorTree cannot be cloned!");
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
pub fn behavior_tree_system(world: &mut World, dt: f32) {
    let entities: Vec<u32> = {
        let trees = world.borrow_mut::<BehaviorTree>();
        trees.entities().collect()
    };

    for entity in entities {
        let mut root_opt = None;
        if let Some(tree) = world.borrow_mut::<BehaviorTree>().get_mut(entity) {
            root_opt = tree.root.take();
        }

        if let Some(mut root) = root_opt {
            root.tick(entity, world, dt);

            if let Some(tree) = world.borrow_mut::<BehaviorTree>().get_mut(entity) {
                tree.root = Some(root);
            }
        }
    }
}
