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
