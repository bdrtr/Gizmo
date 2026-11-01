//! A resource holding one application state value plus a pending transition.
//!
//! [`State<S>`] stores the current value and, separately, whatever was requested via
//! [`State::set`]; the swap happens at [`State::apply_transitions`], not at the call. That
//! delay is the point — a system can request a state change without the systems running
//! beside it in the same batch seeing a torn view.
//!
//! [`in_state`] turns a state value into a run condition.
use crate::world::World;

/// The State struct used to manage the logical states in the game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State<S: Clone + PartialEq + Eq + Send + Sync + 'static> {
    current: S,
    next: Option<S>,
}

impl<S: Clone + PartialEq + Eq + Send + Sync + 'static> State<S> {
    /// Starts the machine already *in* `initial`, with no transition queued.
    ///
    /// Entering the initial state is therefore never reported: the first
    /// [`State::apply_transitions`] returns `false`, and any one-off setup for `initial` has
    /// to be run by hand.
    ///
    /// This only builds a value; it registers nothing. [`in_state`] looks `State<S>` up as a
    /// world resource and answers `false` when there is none, so until the machine is inserted
    /// as a resource every gated system is switched *off*, not on.
    ///
    /// A bare `State` is driven by nobody: the application owns the
    /// [`State::apply_transitions`] call. [`add_state`] makes it a driven machine instead —
    /// applied once a frame by the app loop, with `on_enter` / `on_exit` hooks and the initial
    /// state reported.
    pub fn new(initial: S) -> Self {
        Self {
            current: initial,
            next: None,
        }
    }

    /// The state that is active *now*.
    ///
    /// A [`State::set`] made earlier is not visible here — this keeps answering the old value
    /// until [`State::apply_transitions`] runs. That delay is the point: between two
    /// `apply_transitions` calls every reader sees the same state, whichever of them asked
    /// for the switch and in whatever order they run.
    pub fn get(&self) -> &S {
        &self.current
    }

    /// Queues a switch to `state`, to take effect at the next [`State::apply_transitions`].
    ///
    /// Ignored when `state` already equals the current one, so re-asserting the state you are
    /// in never produces a spurious transition. The comparison is against the *current* state
    /// only, never against an already queued one, which has two consequences: calling this
    /// twice before a transition is applied simply keeps the last value (the intermediate
    /// state is never observed by anyone), and setting the current state back does *not*
    /// cancel a queued switch — the queued switch still happens.
    pub fn set(&mut self, state: S) {
        if self.current != state {
            self.next = Some(state);
        }
    }

    /// Switches the next state over to being the active one. (Usually run in the PreUpdate phase).
    pub fn apply_transitions(&mut self) -> bool {
        if let Some(next) = self.next.take() {
            self.current = next;
            true
        } else {
            false
        }
    }
}

/// The "Run Condition" function that makes a system run only while in a particular state.
pub fn in_state<S>(state: S) -> impl FnMut(&World) -> bool + Send + Sync + 'static
where
    S: Clone + PartialEq + Eq + Send + Sync + 'static,
{
    move |world: &World| {
        if let Some(current_state) = world.get_resource::<State<S>>() {
            *current_state.get() == state
        } else {
            false
        }
    }
}

/// A callback run on entering or leaving a state. It gets the whole world, mutably: spawning a
/// level, despawning a menu, resetting a score.
pub type StateHook = Box<dyn FnMut(&mut World) + Send + Sync>;

/// What runs on entering and leaving each value of `S` — the `OnEnter` / `OnExit` half of a state
/// machine, which [`State`] alone leaves to the application.
///
/// Built by [`add_state`]; filled by [`on_enter`] and [`on_exit`]; run by [`run_state_drivers`],
/// which the application loop calls once a frame before any schedule. The value is a world
/// resource, taken out while its hooks run so that they can have `&mut World`.
pub struct StateHooks<S: Clone + PartialEq + Eq + Send + Sync + 'static> {
    enter: Vec<(S, StateHook)>,
    exit: Vec<(S, StateHook)>,
    /// Whether the initial state's `on_enter` hooks have run. [`State::new`] starts *in* its
    /// state, which no `apply_transitions` reports — so the driver reports it, once.
    entered_initial: bool,
}

impl<S: Clone + PartialEq + Eq + Send + Sync + 'static> Default for StateHooks<S> {
    fn default() -> Self {
        Self { enter: Vec::new(), exit: Vec::new(), entered_initial: false }
    }
}

/// Marks an entity that belongs to one state: when that state is left, [`run_state_drivers`]
/// despawns it, with its children. A menu's buttons, a level's enemies, a pause overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DespawnOnExit<S>(pub S);

impl<S: Clone + PartialEq + Eq + Send + Sync + 'static> crate::Component for DespawnOnExit<S> {}

/// The per-type drivers [`add_state`] registered: one function per state type, kept as a world
/// resource so the loop that calls them needs to know no state type.
#[derive(Default)]
pub struct StateDrivers(Vec<(std::any::TypeId, fn(&mut World))>);

/// Makes `S` a driven state machine starting in `initial`: inserts [`State<S>`] and an empty
/// [`StateHooks<S>`], and registers its driver with [`run_state_drivers`]. Calling it again for
/// the same `S` resets the state to `initial` and keeps the hooks already added.
///
/// From then on, once a frame: the first run enters `initial` (its `on_enter` hooks run — the
/// initial state *is* reported, unlike with a bare [`State`]); every later run applies a queued
/// [`State::set`], and if it switched, runs the old state's `on_exit` hooks, despawns its
/// [`DespawnOnExit`] entities, then runs the new state's `on_enter` hooks. One switch a frame: a
/// `set` made inside a hook is applied on the next.
pub fn add_state<S: Clone + PartialEq + Eq + Send + Sync + 'static>(world: &mut World, initial: S) {
    world.insert_resource(State::new(initial));
    if world.get_resource::<StateHooks<S>>().is_none() {
        world.insert_resource(StateHooks::<S>::default());
    }
    if world.get_resource::<StateDrivers>().is_none() {
        world.insert_resource(StateDrivers::default());
    }
    let mut drivers = world.get_resource_mut::<StateDrivers>().expect("just ensured");
    let id = std::any::TypeId::of::<S>();
    if !drivers.0.iter().any(|(t, _)| *t == id) {
        drivers.0.push((id, drive_state::<S>));
    }
}

/// Runs `hook` every time `state` is entered — including the initial state, on the first frame.
/// Hooks for one state run in the order they were added. Needs [`add_state`] first.
pub fn on_enter<S, F>(world: &mut World, state: S, hook: F)
where
    S: Clone + PartialEq + Eq + Send + Sync + 'static,
    F: FnMut(&mut World) + Send + Sync + 'static,
{
    hooks_of::<S>(world).enter.push((state, Box::new(hook)));
}

/// Runs `hook` every time `state` is left, before the next state's `on_enter`.
pub fn on_exit<S, F>(world: &mut World, state: S, hook: F)
where
    S: Clone + PartialEq + Eq + Send + Sync + 'static,
    F: FnMut(&mut World) + Send + Sync + 'static,
{
    hooks_of::<S>(world).exit.push((state, Box::new(hook)));
}

fn hooks_of<S: Clone + PartialEq + Eq + Send + Sync + 'static>(
    world: &mut World,
) -> crate::world::resources::ResourceWriteGuard<'_, StateHooks<S>> {
    if world.get_resource::<StateHooks<S>>().is_none() {
        world.insert_resource(StateHooks::<S>::default());
    }
    world.get_resource_mut::<StateHooks<S>>().expect("just ensured")
}

/// Runs every driver [`add_state`] registered, in registration order. The application loop calls
/// this once a frame, before the fixed-step and update schedules, so every system of the frame
/// sees the frame's state; a world with no driven state does nothing.
pub fn run_state_drivers(world: &mut World) {
    let Some(drivers) = world.get_resource::<StateDrivers>().map(|d| d.0.iter().map(|(_, f)| *f).collect::<Vec<_>>())
    else {
        return;
    };
    for drive in drivers {
        drive(world);
    }
}

fn drive_state<S: Clone + PartialEq + Eq + Send + Sync + 'static>(world: &mut World) {
    let Some(mut hooks) = world.remove_resource::<StateHooks<S>>() else { return };
    let Some(current) = world.get_resource::<State<S>>().map(|s| s.get().clone()) else {
        world.insert_resource(hooks);
        return;
    };
    if !hooks.entered_initial {
        hooks.entered_initial = true;
        run_hooks(&mut hooks.enter, &current, world);
    } else {
        let switched = world.get_resource_mut::<State<S>>().map(|mut s| s.apply_transitions()).unwrap_or(false);
        if switched {
            let next = world.get_resource::<State<S>>().map(|s| s.get().clone()).expect("checked above");
            run_hooks(&mut hooks.exit, &current, world);
            despawn_scoped(world, &current);
            run_hooks(&mut hooks.enter, &next, world);
        }
    }
    // A hook may have added hooks for this type (through `on_enter`), which created a fresh
    // resource while ours was out; keep both.
    if let Some(added) = world.remove_resource::<StateHooks<S>>() {
        hooks.enter.extend(added.enter);
        hooks.exit.extend(added.exit);
    }
    world.insert_resource(hooks);
}

fn run_hooks<S: PartialEq>(hooks: &mut [(S, StateHook)], state: &S, world: &mut World) {
    for (s, hook) in hooks.iter_mut() {
        if s == state {
            hook(world);
        }
    }
}

fn despawn_scoped<S: Clone + PartialEq + Eq + Send + Sync + 'static>(world: &mut World, left: &S) {
    use crate::hierarchy::HierarchyExt;
    let ids: Vec<u32> = match world.query::<&DespawnOnExit<S>>() {
        Some(q) => q.iter().filter(|(_, d)| d.0 == *left).map(|(id, _)| id).collect(),
        None => return,
    };
    for id in ids {
        // A child despawned with its parent above is already gone.
        if let Some(entity) = world.entity(id) {
            world.despawn_recursive(entity);
        }
    }
}

#[cfg(test)]
mod tests {
    //! Every claim this module's docs make, checked.
    //!
    //! Nothing in the workspace drives `State` — that is a recorded decision (the application
    //! owns the `apply_transitions` call), not an accident. But it also had no test, so five
    //! documented behaviours rested on prose alone, and one of them is genuinely surprising.

    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Screen {
        Menu,
        Playing,
        Paused,
    }

    /// The whole reason the switch is deferred: between two `apply_transitions` every reader
    /// sees the same value, whichever system asked for the change.
    #[test]
    fn a_requested_switch_is_not_visible_until_it_is_applied() {
        let mut state = State::new(Screen::Menu);
        state.set(Screen::Playing);
        assert_eq!(*state.get(), Screen::Menu, "the switch must not happen at the call");
        assert!(state.apply_transitions());
        assert_eq!(*state.get(), Screen::Playing);
    }

    #[test]
    fn the_initial_state_is_never_reported_as_a_transition() {
        let mut state = State::new(Screen::Menu);
        assert!(
            !state.apply_transitions(),
            "entering the initial state is not a transition — one-off setup for it has to be run \
             by hand, which is only true if this returns false"
        );
    }

    #[test]
    fn asking_for_the_state_you_are_already_in_is_not_a_transition() {
        let mut state = State::new(Screen::Menu);
        state.set(Screen::Menu);
        assert!(!state.apply_transitions(), "re-asserting the current state must be a no-op");
    }

    /// Two `set`s before an apply keep the last, and the intermediate is never observed by
    /// anyone — which is what makes the deferral safe for systems that disagree.
    #[test]
    fn two_requests_before_an_apply_keep_the_last() {
        let mut state = State::new(Screen::Menu);
        state.set(Screen::Playing);
        state.set(Screen::Paused);
        assert!(state.apply_transitions());
        assert_eq!(*state.get(), Screen::Paused);
        assert!(!state.apply_transitions(), "and the queue is empty afterwards");
    }

    /// The surprising one, and therefore the one most worth pinning: `set` compares against the
    /// **current** state, never against an already queued one. So asking to go back to where you
    /// are does not cancel a queued switch — it is ignored, and the switch still happens.
    ///
    /// A reader who assumed otherwise would write `state.set(current)` as a cancel and get a
    /// transition anyway, one frame later, with nothing to point at.
    #[test]
    fn setting_the_current_state_back_does_not_cancel_a_queued_switch() {
        let mut state = State::new(Screen::Menu);
        state.set(Screen::Playing);
        state.set(Screen::Menu); // "cancel" — ignored, because Menu IS the current state
        assert!(state.apply_transitions());
        assert_eq!(
            *state.get(),
            Screen::Playing,
            "the queued switch still happened; `set(current)` is not a cancel"
        );
    }

    /// With no `State<S>` resource in the world, a gated system is switched **off**, not on.
    /// The other way round would run menu systems during play on any world that forgot to
    /// insert the machine.
    #[test]
    fn a_state_condition_is_false_when_the_machine_is_not_in_the_world() {
        let world = World::new();
        let mut condition = in_state(Screen::Menu);
        assert!(!condition(&world), "no machine means no state, so nothing gated may run");
    }

    #[test]
    fn a_state_condition_follows_the_applied_value_not_the_requested_one() {
        let mut world = World::new();
        let mut state = State::new(Screen::Menu);
        state.set(Screen::Playing);
        world.insert_resource(state);

        let mut in_menu = in_state(Screen::Menu);
        let mut in_play = in_state(Screen::Playing);
        assert!(in_menu(&world), "before the apply the machine is still in Menu");
        assert!(!in_play(&world));

        world
            .get_resource_mut::<State<Screen>>()
            .expect("inserted above")
            .apply_transitions();
        assert!(!in_menu(&world));
        assert!(in_play(&world));
    }

    /// The order of everything a driven switch does, written to a log the hooks share.
    #[derive(Default)]
    struct Log(Vec<String>);

    fn log(world: &mut World, line: &str) {
        world.get_resource_mut::<Log>().expect("log").0.push(line.to_string());
    }

    fn driven() -> World {
        let mut world = World::new();
        world.insert_resource(Log::default());
        add_state(&mut world, Screen::Menu);
        on_enter(&mut world, Screen::Menu, |w| log(w, "enter menu"));
        on_exit(&mut world, Screen::Menu, |w| log(w, "exit menu"));
        on_enter(&mut world, Screen::Playing, |w| log(w, "enter playing"));
        world
    }

    fn lines(world: &World) -> Vec<String> {
        world.get_resource::<Log>().expect("log").0.clone()
    }

    /// The gap a bare `State` leaves: entering the initial state. Driven, it is reported on the
    /// first run and never again.
    #[test]
    fn a_driven_machine_enters_its_initial_state_once() {
        let mut world = driven();
        run_state_drivers(&mut world);
        run_state_drivers(&mut world);
        run_state_drivers(&mut world);
        assert_eq!(lines(&world), ["enter menu"]);
    }

    /// A switch: the old state's exit, its scoped entities gone (children included), then the new
    /// state's enter — and an entity scoped to another state, or to none, left alone.
    #[test]
    fn a_switch_exits_despawns_then_enters() {
        use crate::hierarchy::HierarchyExt;
        let mut world = driven();
        run_state_drivers(&mut world);
        let button = world.spawn();
        world.add_component(button, DespawnOnExit(Screen::Menu));
        let label = world.spawn();
        world.add_child(button, label);
        let hud = world.spawn();
        world.add_component(hud, DespawnOnExit(Screen::Playing));
        let level = world.spawn();

        world.get_resource_mut::<State<Screen>>().unwrap().set(Screen::Playing);
        assert_eq!(lines(&world), ["enter menu"], "nothing happens at the call");
        run_state_drivers(&mut world);

        assert_eq!(lines(&world), ["enter menu", "exit menu", "enter playing"]);
        assert_eq!(*world.get_resource::<State<Screen>>().unwrap().get(), Screen::Playing);
        assert!(world.entity(button.id()).is_none(), "the menu's button went with the menu");
        assert!(world.entity(label.id()).is_none(), "and its child with it");
        assert!(world.entity(hud.id()).is_some(), "the playing HUD stays");
        assert!(world.entity(level.id()).is_some(), "an unscoped entity stays");
    }

    /// A `set` made inside a hook is the next frame's switch, not this one's; and a hook added
    /// from inside a hook is kept rather than lost while the hooks are out of the world.
    #[test]
    fn a_hook_can_queue_a_switch_and_add_hooks() {
        let mut world = driven();
        on_enter(&mut world, Screen::Playing, |w| {
            w.get_resource_mut::<State<Screen>>().unwrap().set(Screen::Paused);
            on_enter(w, Screen::Paused, |w| log(w, "enter paused"));
        });
        run_state_drivers(&mut world);
        world.get_resource_mut::<State<Screen>>().unwrap().set(Screen::Playing);
        run_state_drivers(&mut world);
        assert_eq!(*world.get_resource::<State<Screen>>().unwrap().get(), Screen::Playing, "one switch a frame");
        run_state_drivers(&mut world);
        assert_eq!(*world.get_resource::<State<Screen>>().unwrap().get(), Screen::Paused);
        assert_eq!(lines(&world), ["enter menu", "exit menu", "enter playing", "enter paused"]);
    }

    /// No driven state, nothing to do — the app loop calls this every frame for every world.
    #[test]
    fn a_world_without_a_driven_state_is_untouched() {
        let mut world = World::new();
        run_state_drivers(&mut world);
        assert!(world.get_resource::<StateDrivers>().is_none());
    }
}
