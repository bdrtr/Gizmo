//! **Experimental.** Flexbox/grid UI *layout* and pointer *hit-testing* for the
//! Gizmo ECS — this crate computes geometry and interaction state, and draws
//! nothing at all.
//!
//! `gizmo-ui` builds on [`taffy`] for layout and integrates with the Gizmo ECS.
//! UI elements are entities carrying components such as [`Style`] (layout),
//! [`Node`] (computed geometry), [`BackgroundColor`] and [`Interaction`].
//! Spawn them via the [`NodeBundle`] and [`ButtonBundle`] bundles.
//!
//! Add `UiPlugin` to an `App` to register the components and run the layout
//! and interaction systems each frame, or call [`register`] to do the same on a
//! bare `World`/`Schedule` without `gizmo-app`. Common types are re-exported
//! from the [`prelude`] module.
//!
//! (`UiPlugin` is behind the non-default `app` feature, so it is deliberately not
//! an intra-doc link here — the link would not resolve in a default `cargo doc`.)
//!
//! # What works
//!
//! Two systems, and they do what their names say:
//!
//! - `ui_layout_system` (`system.rs`) mirrors every entity carrying a [`Style`]
//!   into a [`taffy`] tree, computes layout for each root against the current
//!   `WindowInfo` size, and writes the result back into [`Node`] as **absolute**
//!   window-pixel `position` + `size` (ancestor offsets are accumulated; taffy's
//!   own locations are parent-relative). Entities that lose their [`Style`] have
//!   their taffy node reclaimed.
//! - `ui_interaction_system` (`interaction.rs`) hit-tests the mouse position
//!   from `Input` against each [`Node`]'s half-open `[pos, pos + size)` box and
//!   sets [`Interaction`] to `None` / `Hovered` / `Pressed`.
//!
//! The crate carries 18 unit tests covering exactly those two things: layout
//! write-back, taffy node lifecycle, the hit-test predicate and the interaction
//! state machine. That is the part you can rely on.
//!
//! # What does NOT work
//!
//! **This crate emits no vertices and no draw calls.** Its dependencies are
//! `gizmo-core`, `gizmo-math`, `taffy` and (optionally, `app` feature)
//! `gizmo-app` with default features off — no renderer, no `wgpu`. Grepping
//! `src/` for `wgpu`, `vertex`, `draw`, `shader`, `texture`, `font` or `glyph`
//! matched nothing but this paragraph. Concretely:
//!
//! - **There is no text rendering** — no `Text` component, no font loading, no
//!   glyph rasterisation, neither here nor in `gizmo-renderer`. This is the
//!   single largest missing piece and the reason this crate is labelled
//!   experimental (tracked as D7 in the repository's `docs/FIXPLAN.md`).
//! - **[`BackgroundColor`] is written and never read.** The bundles attach it,
//!   and no crate in this workspace consumes it. The only consumer of
//!   `gizmo-ui` in the workspace at all is the facade's `pub use gizmo_ui as
//!   ui` re-export.
//! - **No z-order or occlusion.** The hit-test is a flat loop over every
//!   [`Node`], so two overlapping elements both report `Hovered`. The
//!   limitation is noted inline in `interaction.rs`.
//! - **No click/focus events, no keyboard handling, no scrolling, no clipping,
//!   no text input.** [`Interaction`] is a per-frame recomputed state, not an
//!   event stream.
//! - A UI entity whose `Parent` is *not* itself a styled UI entity gets no
//!   layout: root selection is "has no `Parent` component", and only styled
//!   parents get their children attached to the taffy tree, so such a subtree is
//!   in no root's layout pass and its [`Node`] stays stale. Read from the code,
//!   not measured — no test covers it.
//!
//! # What it is good for, and what to use instead
//!
//! Use `gizmo-ui` when you want the engine to solve box geometry and hover/press
//! state for you and you intend to do the drawing yourself: read [`Node`] and
//! [`BackgroundColor`] from your own render pass. Layout and hit-testing are the
//! product here; pixels are your problem.
//!
//! If you want a HUD that is actually visible today, use the `egui` integration
//! in `gizmo-engine` (the `egui` feature, and the `editor` feature on top of
//! it). That path does render, text included, and it does not go through this
//! crate.
//!
//! Note that `gizmo-engine` enables its `ui` feature **by default**, so these
//! types arrive in `gizmo::prelude::*` whether or not you asked for them. Their
//! presence in the prelude is not evidence that anything is being drawn.
//!
//! # Stability
//!
//! Experimental, in the 0.x sense: expect the component set to change when
//! rendering lands (a `Text` component and a draw-list output are the obvious
//! additions). Nothing here is deprecated and nothing is scheduled for removal —
//! the label is about how much of a UI toolkit this is, not about its lifespan.
pub mod components;
pub mod layout;
pub mod system;
pub mod interaction;
pub mod bundles;

use gizmo_core::system::{IntoSystemConfig, Schedule};
use gizmo_core::world::World;
pub use components::*;
pub use bundles::*;
pub use layout::*;

/// Registers the UI components + [`UiContext`] resource and schedules the layout
/// and interaction systems on a [`World`]/[`Schedule`] directly.
///
/// This is the **dependency-light** entry point — it needs only `gizmo-core`, so
/// `gizmo-ui` works as a pure ECS-UI layer without `gizmo-app`. The `app`-feature
/// `UiPlugin` (feature `app`) is a thin wrapper over this.
pub fn register(world: &mut World, schedule: &mut Schedule) {
    world.register_component_type::<Style>();
    world.register_component_type::<Node>();
    world.register_component_type::<Interaction>();
    world.register_component_type::<BackgroundColor>();
    world.register_component_type::<UiRoot>();

    world.insert_resource(UiContext::new());
    // Ensure a WindowInfo exists so `ui_layout_system`'s `Res<WindowInfo>` always
    // resolves (a missing resource would skip the whole system). Under gizmo-app the
    // resize handler keeps this up to date; standalone users can set it directly.
    let _ = world.get_resource_mut_or_default::<gizmo_core::window::WindowInfo>();

    schedule.add_di_system(
        system::ui_layout_system
            .into_config()
            .label("ui_layout"),
    );
    schedule.add_di_system(
        interaction::ui_interaction_system
            .into_config()
            .label("ui_interaction")
            .after("ui_layout"),
    );
}

/// Plugin that registers the UI components and schedules the layout and
/// interaction systems (via [`register`]). Requires the `app` feature.
#[cfg(feature = "app")]
pub struct UiPlugin;

#[cfg(feature = "app")]
impl<State: 'static> gizmo_app::Plugin<State> for UiPlugin {
    fn build(&self, app: &mut gizmo_app::App<State>) {
        // Per-frame, not per fixed step. Layout resolves against the window size and
        // interaction reads the mouse position — both are presentation state refreshed once
        // per rendered frame. On the fixed-timestep schedule these ran `0..N` times per
        // frame depending on the physics accumulator, so with vsync off a hover would
        // register on roughly one frame in ten and a resize could take several frames to
        // reflow. Neither has anything to do with the simulation rate.
        register(&mut app.world, &mut app.update_schedule);
    }
}

/// Re-exports of the most commonly used UI types, including the relevant
/// `taffy` style and geometry items.
pub mod prelude {
    pub use crate::{
        components::{Style, Node, Interaction, BackgroundColor, UiRoot},
        bundles::{NodeBundle, ButtonBundle},
    };
    #[cfg(feature = "app")]
    pub use crate::UiPlugin;
    pub use taffy::style::*;
    pub use taffy::geometry::*;
}
