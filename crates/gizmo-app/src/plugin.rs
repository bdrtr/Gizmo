use gizmo_core::system::Schedule;
use gizmo_core::world::World;

/// What a plugin is allowed to reach for: the world and the two schedules.
///
/// # Why this exists
///
/// [`Plugin::build`] used to take `&mut App<State>`, and `App` is whichever runtime the feature
/// flags selected. The two runtimes are **not** mutually exclusive — a windowed build still has
/// [`headless::App`](crate::headless::App) — so binding the trait to one of them meant the other
/// could not accept plugins at all: `headless::App::add_plugin` was `#[cfg]`-ed out of existence
/// in any build with `window` + `render`, and a headless test inside a graphical application had
/// to register systems by hand.
///
/// The fix is this trait rather than a second type parameter: every plugin in the workspace
/// touched exactly three things, so naming those three is enough, and it keeps [`Plugin`]
/// object-safe (`&mut dyn AppLike`) instead of pushing a runtime parameter into thirteen impls
/// and every caller's turbofish.
pub trait AppLike {
    /// Borrow the world and both schedules at once, as separate fields.
    ///
    /// One method rather than three accessors, and that is not a style choice: a plugin that
    /// wants the world *and* a schedule in the same call — `gizmo-ui`'s does — cannot get them
    /// from two `&mut self` methods, because the second borrow overlaps the first. Handing back
    /// one struct of disjoint field borrows is what a plugin had with direct field access, and
    /// what it keeps here.
    fn parts_mut(&mut self) -> AppParts<'_>;
}

/// The three pieces of an app a plugin may touch, borrowed disjointly — see [`AppLike::parts_mut`].
pub struct AppParts<'a> {
    /// The ECS world: resources, entities, components.
    pub world: &'a mut World,
    /// The **fixed-timestep** schedule — physics and anything that must not vary with frame rate.
    pub schedule: &'a mut Schedule,
    /// The **per-frame** schedule — input, cameras, UI, anything tied to the rendered frame.
    pub update_schedule: &'a mut Schedule,
}

/// A reusable bundle of application setup logic.
///
/// Plugins encapsulate world/resource/system registration so it can be added to an app in one
/// call via `add_plugin` — on either runtime, since [`build`](Plugin::build) speaks
/// [`AppLike`] rather than a concrete `App`.
///
/// This trait is a deliberate **extension point**: downstream crates and applications are
/// expected to implement it for their own plugin types, so it is intentionally *not* sealed.
/// Future methods, if needed, will be added with default implementations to remain
/// backwards-compatible.
///
/// # A plugin that needs more than [`AppLike`]
///
/// Then it is not a plugin: take the concrete `App` in an ordinary function and call it. The
/// state-typed hooks (`set_setup`, `set_update`, `set_render`) are deliberately outside this
/// trait — a plugin that installed one would decide, for the whole application, what `State` is.
pub trait Plugin {
    /// Applies this plugin's setup to the given app.
    fn build(&self, app: &mut dyn AppLike);
}
