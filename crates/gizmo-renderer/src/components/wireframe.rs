//! Wireframes one entity at a time: [`Wireframe`], [`NoWireframe`], [`WireframeColor`].
//!
//! `WireframeConfig::global` draws every line in the frame or none. These say it per entity, on the
//! same overlay and in the same pipeline, which the web does not have: there, lines are fills.

use gizmo_core::impl_component;

/// Draws this entity's wireframe over it, whatever `WireframeConfig::global` says.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Wireframe;

/// Leaves this entity out of `WireframeConfig::global`. It wins over [`Wireframe`] on one entity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoWireframe;

/// The colour of this entity's lines (linear RGB, 0..1), over `WireframeConfig::color`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WireframeColor(pub [f32; 3]);

impl_component!(Wireframe, NoWireframe, WireframeColor);

/// Whether an entity's wireframe is drawn, from the frame's flag and what the entity carries. Both
/// draw loops ask here.
#[must_use]
pub fn draws_wireframe(global: bool, wireframe: bool, no_wireframe: bool) -> bool {
    !no_wireframe && (global || wireframe)
}

#[cfg(test)]
mod tests {
    use super::draws_wireframe;

    #[test]
    fn the_flag_the_marker_and_the_opt_out() {
        assert!(!draws_wireframe(false, false, false));
        assert!(draws_wireframe(true, false, false));
        assert!(draws_wireframe(false, true, false));
        assert!(!draws_wireframe(true, false, true));
        assert!(!draws_wireframe(true, true, true));
    }
}
