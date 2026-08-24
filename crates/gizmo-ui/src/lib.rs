#![warn(missing_docs)]
//! (`missing_docs` is a RATCHET — see the note in `gizmo-math`. This crate is at zero.)
#![deny(clippy::undocumented_unsafe_blocks)]
//! (`undocumented_unsafe_blocks` is a RATCHET: this crate carries no `unsafe` block without a
//! `// SAFETY:` line stating why it is sound, and the lint keeps it that way. Every crate in the
//! workspace except `gizmo-core` is at zero and denies it; `gizmo-core`'s ECS internals are the
//! measured remainder — see docs/ENGINE.md.)
//! **Experimental.** Flexbox UI *layout* and pointer *hit-testing* for the
//! Gizmo ECS — this crate computes geometry and interaction state, and draws
//! nothing at all.
//!
//! `gizmo-ui` builds on [`taffy`] for layout and integrates with the Gizmo ECS.
//! UI elements are entities carrying components such as [`Style`] (layout),
//! [`Node`] (computed geometry), [`BackgroundColor`] and [`Interaction`].
//! Spawn them via the [`NodeBundle`] and [`ButtonBundle`] bundles.
//!
//! [`taffy`] is an **implementation detail**: no taffy type appears in this
//! crate's API. [`Style`] is our own plain-old-data type built from [`Val`]
//! lengths and [`UiRect`]s, and it is translated into a taffy style in exactly
//! one place, inside [`UiContext`]. (Before 0.9 `Style` was a newtype that
//! deref'd to `taffy::style::Style`, and the prelude glob-re-exported taffy's
//! `style` and `geometry` modules; both are gone.)
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
//!   into the layout tree held by [`UiContext`], computes layout for each root
//!   against the current `WindowInfo` size, and writes the result back into
//!   [`Node`] as **absolute** window-pixel `position` + `size` (ancestor offsets
//!   are accumulated; the engine's own locations are parent-relative). Entities
//!   that lose their [`Style`] have their layout node reclaimed.
//! - `ui_interaction_system` (`interaction.rs`) hit-tests the mouse position
//!   from `Input` against each [`Node`]'s half-open `[pos, pos + size)` box and
//!   sets [`Interaction`] to `None` / `Hovered` / `Pressed`.
//!
//! The crate carries 27 unit tests covering exactly those two things plus the
//! [`Style`] → taffy conversion: layout write-back, node lifecycle, length
//! conversion, the hit-test predicate and the interaction state machine. That
//! is the part you can rely on.
//!
//! # What this crate does not do itself — and what now reads it
//!
//! **This crate still emits no vertices and no draw calls.** Its dependencies
//! are `gizmo-core`, `gizmo-math`, `taffy` and (optionally, `app` feature)
//! `gizmo-app` with default features off — no renderer, no `wgpu`. Grepping
//! `src/` for `wgpu`, `vertex`, `draw`, `shader`, `texture`, `font` or `glyph`
//! matches nothing but this paragraph.
//!
//! What changed on 2026-08-24 is that something else reads the output.
//! `gizmo-renderer` gained fonts, a glyph atlas and a `Text` component, and the
//! engine's facade paints what this crate computes: [`BackgroundColor`] becomes
//! a solid quad over the [`Node`]'s box, and a `Text` on the same entity is
//! placed in that box, its anchor choosing which corner. The bridge lives in the
//! facade and not here for a layering reason that will not change: this crate
//! sits **above** `gizmo-app`, so the renderer cannot see a [`Node`].
//!
//! Used standalone — [`register`] on a bare `World`, no engine renderer — this
//! crate is what it always was: geometry and hover state, and the drawing is
//! yours.
//!
//! Concretely still missing:
//!
//! - **No rich text, wrapping, shaping, bidi or font fallback.** What the engine
//!   draws is one font per label, breaking only on `\n`; a string longer than its
//!   box overflows it, because there is no clipping either (below).
//! - **No z-order or occlusion.** The hit-test is a flat loop over every
//!   [`Node`], so two overlapping elements both report `Hovered` — and the
//!   drawing has no per-element order either: every background is painted, then
//!   every glyph, so a label is always above every panel. Right for a button,
//!   wrong for two overlapping windows, and there is no `z` on [`Node`] to sort
//!   by. The hit-test half is noted inline in `interaction.rs`.
//! - **No click/focus events, no keyboard handling, no scrolling, no clipping,
//!   no text input.** [`Interaction`] is a per-frame recomputed state, not an
//!   event stream.
//! - **No CSS Grid.** [`Style`] models the flexbox/block surface only; taffy's
//!   grid algorithm is compiled in but unreachable, because there is no way to
//!   express `display: grid` or a track template. The full list of taffy
//!   properties this crate does not model is in the [`Style`] docs.
//! - A UI entity whose `Parent` is *not* itself a styled UI entity gets no
//!   layout: root selection is "has no `Parent` component", and only styled
//!   parents get their children attached to the layout tree, so such a subtree
//!   is in no root's layout pass and its [`Node`] stays stale. Read from the
//!   code, not measured — no test covers it.
//!
//! # What it is good for, and what to use instead
//!
//! Under `gizmo-engine`, spawn a [`NodeBundle`] or [`ButtonBundle`], give it a
//! [`BackgroundColor`] and hang a `Text` on the same entity: the engine paints
//! both. `demo/src/bin/ui_layout.rs` is a row of buttons doing exactly that,
//! including one whose label overflows its box on purpose.
//!
//! Standalone — [`register`] on a bare `World` with no engine renderer — use
//! this crate when you want box geometry and hover/press state solved for you
//! and you intend to do the drawing yourself: read [`Node`] and
//! [`BackgroundColor`] from your own render pass.
//!
//! For a **debug/editor** overlay — panels, sliders, an immediate-mode inspector
//! — the `egui` integration in `gizmo-engine` is still the shorter path, and it
//! does not go through this crate. What this one gives you that `egui` does not
//! is a UI made of ECS entities: a button is an entity, so it can carry your own
//! components and be driven by your own systems.
//!
//! Note that `gizmo-engine` enables its `ui` feature **by default**, so these
//! types arrive in `gizmo::prelude::*` whether or not you asked for them.
//!
//! # Stability
//!
//! Experimental, in the 0.x sense: expect the component set to change when
//! rendering lands (a `Text` component and a draw-list output are the obvious
//! additions). Nothing here is deprecated and nothing is scheduled for removal —
//! the label is about how much of a UI toolkit this is, not about its lifespan.
/// The UI components themselves: rectangles, styles, anchors — what a widget *is*.
pub mod components;
/// The layout pass: turns the component tree into resolved rectangles (taffy under the hood).
pub mod layout;
/// The scheduled systems that run layout and interaction each frame.
pub mod system;
/// Pointer hit-testing and the hover/press state a widget reads.
pub mod interaction;
/// Bundles that spawn a complete widget in one call.
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
impl gizmo_app::Plugin for UiPlugin {
    fn build(&self, app: &mut dyn gizmo_app::AppLike) {
        let app = app.parts_mut();
        // Per-frame, not per fixed step. Layout resolves against the window size and
        // interaction reads the mouse position — both are presentation state refreshed once
        // per rendered frame. On the fixed-timestep schedule these ran `0..N` times per
        // frame depending on the physics accumulator, so with vsync off a hover would
        // register on roughly one frame in ten and a resize could take several frames to
        // reflow. Neither has anything to do with the simulation rate.
        register(&mut *app.world, &mut *app.update_schedule);
    }
}

/// Re-exports of the most commonly used UI types.
///
/// Everything here is defined by this crate. `taffy`'s style and geometry
/// modules used to be glob-re-exported from this prelude, which made a
/// third-party layout engine part of the public API; the POD [`Style`] type
/// replaced them and the globs are gone.
pub mod prelude {
    pub use crate::{
        components::{
            AlignContent, AlignItems, AlignSelf, BackgroundColor, Display, FlexDirection,
            FlexWrap, Interaction, JustifyContent, Node, PositionType, Style, UiRect, UiRoot, Val,
        },
        bundles::{NodeBundle, ButtonBundle},
    };
    #[cfg(feature = "app")]
    pub use crate::UiPlugin;
}
