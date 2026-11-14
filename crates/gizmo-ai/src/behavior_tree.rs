//! Behaviour trees, as data.
//!
//! A [`BehaviorTree`] is a tree of [`BtNode`]s — composites ([`BtNode::Sequence`],
//! [`BtNode::Selector`]), decorators ([`BtNode::Inverter`], [`BtNode::Succeeder`],
//! [`BtNode::Repeat`]), a timer ([`BtNode::Wait`]) and leaves that **name** what they do
//! ([`BtNode::Action`], [`BtNode::Condition`]). The game registers what each name means once, in
//! a [`BtLibrary`] resource; the tree only holds the names.
//!
//! That split is the point (decided 2026-11-14). A tree that held closures could not be written
//! down: a scene lost every agent's behaviour on save, and the editor's ⏹ — which restores the
//! world from a scene snapshot — emptied every agent it had. A tree of names and numbers is plain
//! `serde` data, so it is saved, cloned with its entity, and shown or edited by a tool, and the
//! state a running tree carries (which child a sequence is on, how long a wait has waited, how
//! many times a repeat has gone round) is part of it: a tree saved mid-run goes on from where it
//! was.
//!
//! What a leaf remembers between ticks belongs in a component of its entity, not in the
//! function: the function is shared by every tree that names it, and a component is saved.
//!
//! ```
//! use gizmo_ai::behavior_tree::{behavior_tree_system, BehaviorTree, BtLibrary, BtNode, BtStatus};
//! use gizmo_core::World;
//!
//! let mut world = World::new();
//! world.insert_resource(
//!     BtLibrary::new()
//!         .with_condition("sees_player", |_entity, _world| false)
//!         .with_action("chase", |_entity, _world, _dt| BtStatus::Running)
//!         .with_action("patrol", |_entity, _world, _dt| BtStatus::Running),
//! );
//! let guard = world.spawn();
//! world.add_component(
//!     guard,
//!     BehaviorTree::new(BtNode::selector([
//!         BtNode::sequence([BtNode::condition("sees_player"), BtNode::action("chase")]),
//!         BtNode::action("patrol"),
//!     ])),
//! );
//! behavior_tree_system(&mut world, 1.0 / 60.0);
//! ```

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use gizmo_core::Entity;
use gizmo_core::World;
use serde::{Deserialize, Serialize};

/// What a node reports after a tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BtStatus {
    /// The node did what it is for. A [`BtNode::Sequence`] moves on to its next child; a
    /// [`BtNode::Selector`] stops and succeeds.
    Success,
    /// The node could not. A sequence fails and starts again from its first child next tick; a
    /// selector tries its next child. A leaf whose name the [`BtLibrary`] does not know fails.
    Failure,
    /// The node has not finished and wants to be ticked again next frame. Composites remember the
    /// running child and resume there.
    Running,
}

/// One node of a [`BehaviorTree`]: what it is, and the state it carries while it runs.
///
/// The state fields (`current`, `done`, `elapsed`) are part of the data on purpose — they are what
/// lets a saved tree resume — and start at zero; the constructors set them, and a file that leaves
/// them out reads them as zero.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum BtNode {
    /// Logical AND: ticks its children in order until one fails or is still running. Succeeds
    /// once every child has (an empty one at once). A failure rewinds it to the first child.
    ///
    /// A *memory* sequence: a running child is resumed next tick, the ones before it are not
    /// ticked again.
    Sequence {
        /// The steps, in order.
        children: Vec<BtNode>,
        /// The child it is on.
        #[serde(default)]
        current: usize,
    },
    /// Logical OR, a fallback: ticks its children in order until one succeeds or is still
    /// running. Fails once every child has (an empty one at once).
    ///
    /// Like the sequence it resumes a running child, so it is **not** reactive: while a later
    /// child runs, the earlier, higher-priority ones are not looked at again.
    Selector {
        /// The alternatives, highest priority first.
        children: Vec<BtNode>,
        /// The child it is on.
        #[serde(default)]
        current: usize,
    },
    /// Swaps success and failure; running passes through.
    Inverter(Box<BtNode>),
    /// Turns a failure into a success — an optional step in a sequence; running passes through.
    Succeeder(Box<BtNode>),
    /// Runs its child to success `times` times (or forever, for `None`), one success per tick so
    /// a child that succeeds at once cannot spin the tick. A failure of the child fails it and
    /// starts the count again.
    Repeat {
        /// The node repeated.
        child: Box<BtNode>,
        /// How many successes complete the repeat; `None` never completes.
        times: Option<u32>,
        /// Successes so far.
        #[serde(default)]
        done: u32,
    },
    /// Runs for `secs` seconds of the tick's `dt`, then succeeds.
    Wait {
        /// How long.
        secs: f32,
        /// How long it has waited so far.
        #[serde(default)]
        elapsed: f32,
    },
    /// Does what the [`BtLibrary`] action of this name does, and reports what it reports.
    Action(String),
    /// Succeeds when the [`BtLibrary`] condition of this name holds, fails when it does not.
    /// Never runs.
    Condition(String),
}

impl BtNode {
    /// A [`BtNode::Sequence`] over `children`.
    pub fn sequence(children: impl IntoIterator<Item = BtNode>) -> Self {
        Self::Sequence { children: children.into_iter().collect(), current: 0 }
    }

    /// A [`BtNode::Selector`] over `children`, highest priority first.
    pub fn selector(children: impl IntoIterator<Item = BtNode>) -> Self {
        Self::Selector { children: children.into_iter().collect(), current: 0 }
    }

    /// A [`BtNode::Inverter`] over `child`.
    pub fn inverter(child: BtNode) -> Self {
        Self::Inverter(Box::new(child))
    }

    /// A [`BtNode::Succeeder`] over `child`.
    pub fn succeeder(child: BtNode) -> Self {
        Self::Succeeder(Box::new(child))
    }

    /// A [`BtNode::Repeat`] of `child`, `times` successes long.
    pub fn repeat(child: BtNode, times: u32) -> Self {
        Self::Repeat { child: Box::new(child), times: Some(times), done: 0 }
    }

    /// A [`BtNode::Repeat`] of `child` that never completes.
    pub fn repeat_forever(child: BtNode) -> Self {
        Self::Repeat { child: Box::new(child), times: None, done: 0 }
    }

    /// A [`BtNode::Wait`] of `secs` seconds.
    pub fn wait(secs: f32) -> Self {
        Self::Wait { secs, elapsed: 0.0 }
    }

    /// A [`BtNode::Action`] naming `name`.
    pub fn action(name: impl Into<String>) -> Self {
        Self::Action(name.into())
    }

    /// A [`BtNode::Condition`] naming `name`.
    pub fn condition(name: impl Into<String>) -> Self {
        Self::Condition(name.into())
    }

    /// Ticks this node once for `entity`, its leaves looked up in `library`.
    pub fn tick(&mut self, entity: Entity, world: &mut World, dt: f32, library: &BtLibrary) -> BtStatus {
        match self {
            Self::Sequence { children, current } => {
                while *current < children.len() {
                    match children[*current].tick(entity, world, dt, library) {
                        BtStatus::Success => *current += 1,
                        BtStatus::Failure => {
                            *current = 0;
                            return BtStatus::Failure;
                        }
                        BtStatus::Running => return BtStatus::Running,
                    }
                }
                *current = 0;
                BtStatus::Success
            }
            Self::Selector { children, current } => {
                while *current < children.len() {
                    match children[*current].tick(entity, world, dt, library) {
                        BtStatus::Success => {
                            *current = 0;
                            return BtStatus::Success;
                        }
                        BtStatus::Failure => *current += 1,
                        BtStatus::Running => return BtStatus::Running,
                    }
                }
                *current = 0;
                BtStatus::Failure
            }
            Self::Inverter(child) => match child.tick(entity, world, dt, library) {
                BtStatus::Success => BtStatus::Failure,
                BtStatus::Failure => BtStatus::Success,
                BtStatus::Running => BtStatus::Running,
            },
            Self::Succeeder(child) => match child.tick(entity, world, dt, library) {
                BtStatus::Running => BtStatus::Running,
                _ => BtStatus::Success,
            },
            Self::Repeat { child, times, done } => match child.tick(entity, world, dt, library) {
                BtStatus::Success => {
                    *done += 1;
                    if times.is_some_and(|n| *done >= n) {
                        *done = 0;
                        BtStatus::Success
                    } else {
                        BtStatus::Running
                    }
                }
                BtStatus::Failure => {
                    *done = 0;
                    BtStatus::Failure
                }
                BtStatus::Running => BtStatus::Running,
            },
            Self::Wait { secs, elapsed } => {
                *elapsed += dt.max(0.0);
                if *elapsed >= *secs {
                    *elapsed = 0.0;
                    BtStatus::Success
                } else {
                    BtStatus::Running
                }
            }
            Self::Action(name) => match library.action(name) {
                Some(action) => action(entity, world, dt),
                None => {
                    library.report_missing("action", name);
                    BtStatus::Failure
                }
            },
            Self::Condition(name) => match library.condition(name) {
                Some(condition) => {
                    if condition(entity, world) {
                        BtStatus::Success
                    } else {
                        BtStatus::Failure
                    }
                }
                None => {
                    library.report_missing("condition", name);
                    BtStatus::Failure
                }
            },
        }
    }

    /// Clears the running state of this subtree — every sequence and selector back on its first
    /// child, every repeat at zero, every wait unwaited — so the next tick starts it afresh. What a
    /// tool does after changing a tree's shape, when a saved position may no longer point at the
    /// child it meant.
    pub fn reset(&mut self) {
        match self {
            Self::Sequence { children, current } | Self::Selector { children, current } => {
                *current = 0;
                children.iter_mut().for_each(BtNode::reset);
            }
            Self::Repeat { child, done, .. } => {
                *done = 0;
                child.reset();
            }
            Self::Inverter(child) | Self::Succeeder(child) => child.reset(),
            Self::Wait { elapsed, .. } => *elapsed = 0.0,
            Self::Action(_) | Self::Condition(_) => {}
        }
    }

    /// Every node of this subtree, this one first.
    pub fn walk(&self, visit: &mut impl FnMut(&BtNode)) {
        visit(self);
        match self {
            Self::Sequence { children, .. } | Self::Selector { children, .. } => {
                for child in children {
                    child.walk(visit);
                }
            }
            Self::Inverter(child) | Self::Succeeder(child) | Self::Repeat { child, .. } => child.walk(visit),
            Self::Wait { .. } | Self::Action(_) | Self::Condition(_) => {}
        }
    }
}

/// An action: what a [`BtNode::Action`] of its name does, for the entity whose tree is ticking.
pub type BtActionFn = dyn Fn(Entity, &mut World, f32) -> BtStatus + Send + Sync;
/// A condition: whether a [`BtNode::Condition`] of its name holds, for the entity whose tree is
/// ticking. A test, so it gets the world to read.
pub type BtConditionFn = dyn Fn(Entity, &World) -> bool + Send + Sync;

#[derive(Default)]
struct LibraryInner {
    actions: HashMap<String, Arc<BtActionFn>>,
    conditions: HashMap<String, Arc<BtConditionFn>>,
    /// Names a tree used and nobody registered, each warned about once.
    reported: Mutex<BTreeSet<String>>,
}

/// What the names in a game's behaviour trees mean: a resource the game fills once, at setup.
///
/// A leaf naming something this does not hold fails, and the first time each such name is met a
/// warning says so — a tree from a scene file may name an action the game has since renamed.
/// [`BehaviorTree::unresolved`] answers the same question without ticking, for an editor.
///
/// Cloning is cheap (one shared table), which is how [`behavior_tree_system`] holds it while the
/// trees get the world mutably.
#[derive(Clone, Default)]
pub struct BtLibrary {
    inner: Arc<LibraryInner>,
}

impl std::fmt::Debug for BtLibrary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BtLibrary")
            .field("actions", &self.action_names())
            .field("conditions", &self.condition_names())
            .finish()
    }
}

impl BtLibrary {
    /// An empty library.
    pub fn new() -> Self {
        Self::default()
    }

    fn inner_mut(&mut self) -> &mut LibraryInner {
        if Arc::get_mut(&mut self.inner).is_none() {
            let old = &self.inner;
            self.inner = Arc::new(LibraryInner {
                actions: old.actions.clone(),
                conditions: old.conditions.clone(),
                reported: Mutex::new(BTreeSet::new()),
            });
        }
        Arc::get_mut(&mut self.inner).expect("just made unique")
    }

    /// Registers `action` under `name`, replacing one of that name.
    pub fn add_action(
        &mut self,
        name: impl Into<String>,
        action: impl Fn(Entity, &mut World, f32) -> BtStatus + Send + Sync + 'static,
    ) -> &mut Self {
        self.inner_mut().actions.insert(name.into(), Arc::new(action));
        self
    }

    /// Registers `condition` under `name`, replacing one of that name.
    pub fn add_condition(
        &mut self,
        name: impl Into<String>,
        condition: impl Fn(Entity, &World) -> bool + Send + Sync + 'static,
    ) -> &mut Self {
        self.inner_mut().conditions.insert(name.into(), Arc::new(condition));
        self
    }

    /// [`add_action`](Self::add_action), by value.
    #[must_use]
    pub fn with_action(
        mut self,
        name: impl Into<String>,
        action: impl Fn(Entity, &mut World, f32) -> BtStatus + Send + Sync + 'static,
    ) -> Self {
        self.add_action(name, action);
        self
    }

    /// [`add_condition`](Self::add_condition), by value.
    #[must_use]
    pub fn with_condition(
        mut self,
        name: impl Into<String>,
        condition: impl Fn(Entity, &World) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.add_condition(name, condition);
        self
    }

    /// The action registered as `name`.
    pub fn action(&self, name: &str) -> Option<Arc<BtActionFn>> {
        self.inner.actions.get(name).cloned()
    }

    /// The condition registered as `name`.
    pub fn condition(&self, name: &str) -> Option<Arc<BtConditionFn>> {
        self.inner.conditions.get(name).cloned()
    }

    /// Every action name, sorted — what an editor offers for an action leaf.
    pub fn action_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.inner.actions.keys().map(String::as_str).collect();
        names.sort_unstable();
        names
    }

    /// Every condition name, sorted.
    pub fn condition_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.inner.conditions.keys().map(String::as_str).collect();
        names.sort_unstable();
        names
    }

    fn report_missing(&self, kind: &str, name: &str) {
        let key = format!("{kind} {name}");
        let first = self.inner.reported.lock().map(|mut seen| seen.insert(key)).unwrap_or(false);
        if first {
            tracing::warn!(
                kind,
                name,
                "[AI] a behaviour tree names a {kind} the BtLibrary does not hold — the leaf fails; \
                 register it with BtLibrary::add_{kind}"
            );
        }
    }
}

/// One agent's behaviour tree: a component, and scene data like any other.
///
/// Ticked in bulk by [`behavior_tree_system`] (which `ai_frame` runs, in the editor's ▶ and in an
/// exported game) or directly with [`tick`](Self::tick). Clones are whole trees, running state
/// included.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct BehaviorTree {
    /// The root, or `None` for an empty tree, which ticks to [`BtStatus::Failure`].
    ///
    /// [`behavior_tree_system`] moves the root *out* of this field for the duration of a tick and
    /// puts it back afterwards, so a leaf that reaches its own entity's `BehaviorTree` while
    /// running sees `None` here. If the component or its entity disappears during that tick, the
    /// root is dropped rather than restored.
    pub root: Option<BtNode>,
}

impl gizmo_core::component::Component for BehaviorTree {}

impl BehaviorTree {
    /// A tree with `root`.
    pub fn new(root: BtNode) -> Self {
        Self { root: Some(root) }
    }

    /// Ticks the root once for `entity`, its leaves looked up in the world's [`BtLibrary`] (an
    /// empty one when it has none). An empty tree fails.
    pub fn tick(&mut self, entity: Entity, world: &mut World, dt: f32) -> BtStatus {
        let library = world.get_resource::<BtLibrary>().map(|l| l.clone()).unwrap_or_default();
        match &mut self.root {
            Some(root) => root.tick(entity, world, dt, &library),
            None => BtStatus::Failure,
        }
    }

    /// The leaf names this tree uses that `library` does not hold, as `"action chase"` /
    /// `"condition sees_player"`, sorted and without repeats — what an editor shows next to a tree
    /// before anything has run.
    pub fn unresolved(&self, library: &BtLibrary) -> Vec<String> {
        let mut missing = BTreeSet::new();
        if let Some(root) = &self.root {
            root.walk(&mut |node| match node {
                BtNode::Action(name) if library.action(name).is_none() => {
                    missing.insert(format!("action {name}"));
                }
                BtNode::Condition(name) if library.condition(name).is_none() => {
                    missing.insert(format!("condition {name}"));
                }
                _ => {}
            });
        }
        missing.into_iter().collect()
    }
}

/// Ticks every entity that carries a [`BehaviorTree`] exactly once.
///
/// The entity list is snapshotted before any node runs, so trees attached during this call are not
/// ticked until the next one, and an entity whose component vanished in the meantime is skipped.
/// Iteration order is the query's; treat it as unspecified.
///
/// For each entity the root is *moved out* of the component, ticked with exclusive access to the
/// world, then moved back — which is what lets a leaf change any component, and has two
/// consequences: during its own tick the entity's [`BehaviorTree::root`] reads as `None`, and if a
/// leaf removes that component or despawns its own entity, the root is dropped instead of
/// restored.
#[tracing::instrument(skip_all, name = "behavior_tree_system")]
pub fn behavior_tree_system(world: &mut World, dt: f32) {
    let entities: Vec<Entity> = world
        .query::<(Entity, &BehaviorTree)>()
        .iter()
        .map(|(entity, _)| entity)
        .collect();
    if entities.is_empty() {
        return;
    }
    let library = world.get_resource::<BtLibrary>().map(|l| l.clone()).unwrap_or_default();

    let tree_count = entities.len();
    let (mut success, mut failure, mut running, mut empty) = (0u32, 0u32, 0u32, 0u32);
    for entity in entities {
        let root = world.query_mut::<&mut BehaviorTree>().get_mut(entity).and_then(|mut tree| tree.root.take());
        let Some(mut root) = root else {
            empty += 1;
            continue;
        };
        let status = root.tick(entity, world, dt, &library);
        match status {
            BtStatus::Success => success += 1,
            BtStatus::Failure => failure += 1,
            BtStatus::Running => running += 1,
        }
        tracing::trace!(entity = %entity, ?status, "[AI] behaviour tree ticked");
        if let Some(mut tree) = world.query_mut::<&mut BehaviorTree>().get_mut(entity) {
            tree.root = Some(root);
        }
    }
    tracing::debug!(tree_count, success, failure, running, empty, "[AI] behaviour trees ticked");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn e0() -> Entity {
        Entity::new(0, 0)
    }

    /// A library with `succeed`, `fail` and `run` — leaves that always report that.
    fn fixed() -> BtLibrary {
        BtLibrary::new()
            .with_action("succeed", |_, _, _| BtStatus::Success)
            .with_action("fail", |_, _, _| BtStatus::Failure)
            .with_action("run", |_, _, _| BtStatus::Running)
    }

    fn tick(node: &mut BtNode) -> BtStatus {
        node.tick(e0(), &mut World::new(), 0.0, &fixed())
    }

    fn a(name: &str) -> BtNode {
        BtNode::action(name)
    }

    #[test]
    fn sequence_succeeds_only_when_all_children_succeed() {
        assert_eq!(tick(&mut BtNode::sequence([a("succeed"), a("succeed")])), BtStatus::Success);
        assert_eq!(tick(&mut BtNode::sequence([a("succeed"), a("fail")])), BtStatus::Failure);
        assert_eq!(tick(&mut BtNode::sequence([])), BtStatus::Success, "an empty sequence succeeds");
    }

    #[test]
    fn selector_succeeds_on_first_success_else_fails() {
        assert_eq!(tick(&mut BtNode::selector([a("fail"), a("succeed")])), BtStatus::Success);
        assert_eq!(tick(&mut BtNode::selector([a("fail"), a("fail")])), BtStatus::Failure);
        assert_eq!(tick(&mut BtNode::selector([])), BtStatus::Failure, "an empty selector fails");
    }

    #[test]
    fn inverter_and_succeeder_change_the_result_but_not_running() {
        assert_eq!(tick(&mut BtNode::inverter(a("succeed"))), BtStatus::Failure);
        assert_eq!(tick(&mut BtNode::inverter(a("fail"))), BtStatus::Success);
        assert_eq!(tick(&mut BtNode::inverter(a("run"))), BtStatus::Running);
        assert_eq!(tick(&mut BtNode::succeeder(a("fail"))), BtStatus::Success);
        assert_eq!(tick(&mut BtNode::succeeder(a("run"))), BtStatus::Running);
    }

    #[test]
    fn a_wait_runs_for_its_seconds_then_succeeds_and_starts_over() {
        let mut wait = BtNode::wait(0.5);
        let lib = BtLibrary::new();
        let mut w = World::new();
        assert_eq!(wait.tick(e0(), &mut w, 0.3, &lib), BtStatus::Running);
        assert_eq!(wait.tick(e0(), &mut w, 0.3, &lib), BtStatus::Success, "0.6 s of 0.5");
        assert_eq!(wait.tick(e0(), &mut w, 0.3, &lib), BtStatus::Running, "and a fresh wait after");
    }

    #[test]
    fn a_repeat_succeeds_after_its_count_one_success_a_tick() {
        let mut rep = BtNode::repeat(a("succeed"), 3);
        assert_eq!(tick(&mut rep), BtStatus::Running);
        assert_eq!(tick(&mut rep), BtStatus::Running);
        assert_eq!(tick(&mut rep), BtStatus::Success, "the third success completes it");
        assert_eq!(tick(&mut rep), BtStatus::Running, "and it counts again");
        assert_eq!(tick(&mut BtNode::repeat(a("fail"), 3)), BtStatus::Failure);
        let mut forever = BtNode::repeat_forever(a("succeed"));
        for _ in 0..10 {
            assert_eq!(tick(&mut forever), BtStatus::Running);
        }
    }

    #[test]
    fn a_leaf_naming_nothing_fails_and_the_tree_says_which() {
        let lib = fixed().with_condition("ok", |_, _| true);
        let tree = BehaviorTree::new(BtNode::sequence([
            a("succeed"),
            a("chase"),
            BtNode::condition("ok"),
            BtNode::condition("sees_player"),
            a("chase"),
        ]));
        assert_eq!(tree.unresolved(&lib), vec!["action chase".to_string(), "condition sees_player".to_string()]);
        let mut root = tree.root.clone().unwrap();
        assert_eq!(root.tick(e0(), &mut World::new(), 0.0, &lib), BtStatus::Failure);
    }

    #[test]
    fn a_condition_reads_the_world_for_its_entity() {
        #[derive(Clone)]
        struct Alert;
        impl gizmo_core::component::Component for Alert {}
        let lib = BtLibrary::new().with_condition("alert", |e, w| w.query::<&Alert>().get(e).is_some());
        let mut w = World::new();
        let calm = w.spawn();
        let alert = w.spawn();
        w.add_component(alert, Alert);
        let mut node = BtNode::condition("alert");
        assert_eq!(node.tick(calm, &mut w, 0.0, &lib), BtStatus::Failure);
        assert_eq!(node.tick(alert, &mut w, 0.0, &lib), BtStatus::Success);
    }

    #[test]
    fn sequence_resumes_at_running_child_without_reticking_earlier_ones() {
        let first = Arc::new(AtomicUsize::new(0));
        let second = Arc::new(AtomicUsize::new(0));
        let (f, s) = (first.clone(), second.clone());
        let lib = BtLibrary::new()
            .with_action("first", move |_, _, _| {
                f.fetch_add(1, Ordering::SeqCst);
                BtStatus::Success
            })
            .with_action("second", move |_, _, _| {
                if s.fetch_add(1, Ordering::SeqCst) == 0 { BtStatus::Running } else { BtStatus::Success }
            });
        let mut seq = BtNode::sequence([a("first"), a("second")]);
        let mut w = World::new();
        assert_eq!(seq.tick(e0(), &mut w, 0.0, &lib), BtStatus::Running);
        assert_eq!(seq.tick(e0(), &mut w, 0.0, &lib), BtStatus::Success);
        assert_eq!(first.load(Ordering::SeqCst), 1, "the memory sequence re-ticked a finished child");
    }

    /// The reason the tree is data: written out mid-run and read back, it goes on from where it
    /// was — the sequence still on its wait, the wait still half done.
    #[test]
    fn a_tree_written_out_mid_run_goes_on_from_where_it_was() {
        let fired = Arc::new(AtomicUsize::new(0));
        let f = fired.clone();
        let lib = fixed().with_action("fire", move |_, _, _| {
            f.fetch_add(1, Ordering::SeqCst);
            BtStatus::Success
        });
        let mut tree = BehaviorTree::new(BtNode::sequence([a("succeed"), BtNode::wait(1.0), a("fire")]));
        let mut w = World::new();
        w.insert_resource(lib);
        assert_eq!(tree.tick(e0(), &mut w, 0.6), BtStatus::Running);

        let text = serde_json::to_string(&tree).expect("a tree is data");
        let mut back: BehaviorTree = serde_json::from_str(&text).expect("and reads back");
        assert_eq!(back, tree, "with its running state");
        assert_eq!(back.tick(e0(), &mut w, 0.5), BtStatus::Success, "0.6 + 0.5 s of a 1 s wait");
        assert_eq!(fired.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn reset_clears_every_running_state_and_keeps_the_shape() {
        let fresh = BtNode::sequence([a("succeed"), BtNode::repeat(BtNode::wait(1.0), 3)]);
        let mut node = fresh.clone();
        let (lib, mut w) = (fixed(), World::new());
        for _ in 0..4 {
            node.tick(e0(), &mut w, 0.7, &lib);
        }
        assert_ne!(node, fresh, "ticking moved it on");
        node.reset();
        assert_eq!(node, fresh);
    }

    #[test]
    fn a_cloned_tree_is_the_whole_tree() {
        let tree = BehaviorTree::new(BtNode::selector([a("fail"), BtNode::wait(2.0)]));
        assert_eq!(tree.clone(), tree, "a prefab copy keeps its behaviour");
    }

    #[test]
    fn empty_behavior_tree_ticks_to_failure() {
        let mut bt = BehaviorTree::default();
        assert_eq!(bt.tick(e0(), &mut World::new(), 0.0), BtStatus::Failure);
    }

    // ── `behavior_tree_system` ───────────────────────────────────────────────────────────────

    fn counting(world: &mut World, name: &str) -> Arc<AtomicUsize> {
        let ticks = Arc::new(AtomicUsize::new(0));
        let t = ticks.clone();
        let mut lib = world.remove_resource::<BtLibrary>().unwrap_or_default();
        lib.add_action(name, move |_, _, _| {
            t.fetch_add(1, Ordering::SeqCst);
            BtStatus::Success
        });
        world.insert_resource(lib);
        ticks
    }

    #[test]
    fn the_system_ticks_every_tree_in_the_world() {
        let mut world = World::new();
        let ticks = counting(&mut world, "count");
        for _ in 0..3 {
            let e = world.spawn();
            world.add_component(e, BehaviorTree::new(a("count")));
        }
        behavior_tree_system(&mut world, 1.0 / 60.0);
        assert_eq!(ticks.load(Ordering::SeqCst), 3, "one tick per tree");
    }

    /// The documented consequence of moving the root out: a leaf that reaches its OWN entity's
    /// `BehaviorTree` mid-tick sees `None` there, not itself.
    #[test]
    fn a_leaf_that_looks_at_its_own_tree_mid_tick_sees_an_empty_one() {
        let mut world = World::new();
        let observed = Arc::new(AtomicUsize::new(usize::MAX));
        let seen = observed.clone();
        world.insert_resource(BtLibrary::new().with_action("look", move |entity, w: &mut World, _| {
            let root_is_none = w.query::<&BehaviorTree>().get(entity).is_some_and(|t| t.root.is_none());
            seen.store(usize::from(root_is_none), Ordering::SeqCst);
            BtStatus::Success
        }));
        let e = world.spawn();
        world.add_component(e, BehaviorTree::new(a("look")));
        behavior_tree_system(&mut world, 1.0 / 60.0);
        assert_eq!(observed.load(Ordering::SeqCst), 1, "the root is out for the duration of the tick");
        assert!(world.query::<&BehaviorTree>().get(e).is_some_and(|t| t.root.is_some()), "and back after");
    }

    /// A leaf that removes its own `BehaviorTree` loses it: putting it back would resurrect a
    /// component the leaf deliberately removed.
    #[test]
    fn a_leaf_that_removes_its_own_tree_loses_it_rather_than_having_it_restored() {
        let mut world = World::new();
        world.insert_resource(BtLibrary::new().with_action("quit", |entity, w: &mut World, _| {
            w.remove_component::<BehaviorTree>(entity);
            BtStatus::Success
        }));
        let e = world.spawn();
        world.add_component(e, BehaviorTree::new(a("quit")));
        behavior_tree_system(&mut world, 1.0 / 60.0);
        assert!(world.query::<&BehaviorTree>().get(e).is_none());
    }

    #[test]
    fn an_empty_tree_is_skipped_without_complaint() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, BehaviorTree::default());
        behavior_tree_system(&mut world, 1.0 / 60.0);
        assert!(world.query::<&BehaviorTree>().get(e).is_some_and(|t| t.root.is_none()));
    }

    /// A library is shared by cheap clones; adding to one copy does not change another, and a
    /// clone the system holds sees what was registered before it was taken.
    #[test]
    fn adding_to_a_cloned_library_leaves_the_original_alone() {
        let original = fixed();
        let mut copy = original.clone();
        copy.add_action("extra", |_, _, _| BtStatus::Success);
        assert!(original.action("extra").is_none());
        assert!(copy.action("extra").is_some() && copy.action("succeed").is_some());
        assert_eq!(copy.action_names(), vec!["extra", "fail", "run", "succeed"]);
    }
}
