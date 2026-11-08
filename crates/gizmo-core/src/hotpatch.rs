//! Hot-patching: a system's body edited while the game runs, and its next call running the new code.
//!
//! With the `hotpatching` feature on, every place this crate calls code a game wrote — a system, a run
//! condition, a `pipe` stage, an observer, a [`DynamicSystem`](crate::dynamic::DynamicSystem)'s body —
//! calls it through [subsecond](https://docs.rs/subsecond)'s jump table, the mechanism Bevy uses. The
//! Dioxus CLI rebuilds what changed, links it against the running program and sends the new table;
//! from then on the call lands in the new code. The world, its entities and resources are untouched:
//! only code changes.
//!
//! ```sh
//! cargo install dioxus-cli --version 0.7.10 --locked   # `dx`, the same 0.7 line as `subsecond`
//! dx serve --hot-patch -p demo --bin hot_systems --features hotpatching
//! ```
//!
//! `hot_systems` prints a line a system returns once a second; edit the line and save, and the next one
//! is the new text while its counter carries on (measured 2026-11-09: the patch took 1.7 s, the world
//! kept its state). A game of its own turns the feature on through the engine:
//! `hotpatching = ["gizmo-engine/hotpatching"]` in its `[features]`.
//!
//! What it covers and what it does not, from subsecond's own limits:
//!
//! - **Debug builds only.** Subsecond compiles to a plain call when `debug_assertions` are off, so a
//!   release build pays nothing and patches nothing.
//! - **The crate with `main` only.** A system written in the game's binary crate is patched; one in a
//!   library the binary uses, this engine included, is not.
//! - **Bodies, not shapes.** Changing what a system does is a patch; changing its parameters, or the
//!   layout of a component or resource it touches, needs a restart — the scheduler's access lists and the
//!   world's storage were built for the old ones.
//! - **What is handed over as a closure is reached; a box pushed by hand is not.** `register_on_add`
//!   and its siblings take the closure itself since 2026-11-09 and box it after wrapping the call, as
//!   `add_observer` does. A `Box<dyn FnMut>` pushed straight onto
//!   [`World::component_hooks_mut`](crate::world::World::component_hooks_mut)'s lists has lost its type,
//!   so it runs the code it was built with.
//!
//! The feature is off by default and off on the browser: it adds subsecond, and the app layer adds the
//! connection to the CLI.

/// Calls the generic function item `$f` with `$args`: through subsecond's jump table with the
/// `hotpatching` feature, directly without it.
///
/// `$f` must be a function item — `body::<F, ..>`, never a closure value — whose code includes the
/// game's: subsecond looks the item up in the table, and the item is monomorphised in the game's crate,
/// so editing the game's function gives it a new entry.
macro_rules! hot_call {
    ($f:expr $(, $arg:expr)* $(,)?) => {{
        #[cfg(feature = "hotpatching")]
        {
            match ::subsecond::HotFn::current($f).try_call(($($arg,)*)) {
                Ok(out) => out,
                // Subsecond's own signal that code *above* this call changed too, and the frame it is
                // in is stale: nothing short of a restart can run it.
                Err(_) => panic!("a hot patch changed code that calls this system; restart the program"),
            }
        }
        #[cfg(not(feature = "hotpatching"))]
        {
            ($f)($($arg),*)
        }
    }};
}
pub(crate) use hot_call;
