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
    /// Note that nothing in the engine drives this type: no scheduling phase in this workspace
    /// calls [`State::apply_transitions`], so the application owns that call — typically once
    /// a frame, before the systems that branch on the state.
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
}
