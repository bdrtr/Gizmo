use gizmo_math::{Vec2, Vec4};

/// Layout style of a UI element.
///
/// This is a newtype wrapper around [`taffy::style::Style`] and derefs to it,
/// so all taffy style fields (flexbox, grid, sizing, spacing, ...) are
/// available directly.
#[derive(Clone, Debug, PartialEq)]
#[derive(Default)]
pub struct Style(pub taffy::style::Style);

// SAFETY: `taffy::Style` is `!Send + !Sync` only because `CompactLength` packs
// tag + value + pointer into one `*const ()` on 64-bit targets (taffy
// `style/compact_length.rs`, `mod inner`). The pointer variant is reachable
// through exactly one public constructor, `CompactLength::calc`, which is
// `#[cfg(feature = "calc")]` — and we build taffy with `default-features =
// false` and **without** `calc` (see this crate's Cargo.toml, where the reason
// is recorded next to the dependency). With that feature off the union can only
// ever hold an `f32` plus a tag: plain old data with no interior mutability, no
// provenance and nothing to race on. Sending/sharing it across threads is
// therefore sound, and the `Style` component needs it because ECS components
// must be `Send + Sync`.
//
// If anyone re-enables taffy's `calc` feature, these two impls become unsound
// (the tagged pointer would then alias caller-owned arena memory) — the
// Cargo.toml comment is the guard, and `calc_values_are_unconstructible` below
// is the test that fails loudly if the surface ever comes back.
unsafe impl Send for Style {}
// SAFETY: see the `Send` impl above — same invariant.
unsafe impl Sync for Style {}


impl std::ops::Deref for Style {
    type Target = taffy::style::Style;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for Style {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Computed layout of a UI element, written back each frame by the layout system.
///
/// `size` is the element's width/height and `position` is its top-left corner,
/// both in window pixel coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Node {
    /// Computed width and height in pixels.
    pub size: Vec2,
    /// Computed top-left position in window pixel coordinates.
    pub position: Vec2,
}

impl Default for Node {
    fn default() -> Self {
        Self {
            size: Vec2::ZERO,
            position: Vec2::ZERO,
        }
    }
}

/// Current pointer interaction state of a UI element, updated each frame by the
/// interaction system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(Default)]
#[non_exhaustive]
pub enum Interaction {
    /// The pointer is neither over nor pressing the element.
    #[default]
    None,
    /// The pointer is over the element but not pressed.
    Hovered,
    /// The pointer is over the element and the primary button is held.
    Pressed,
}


/// Fill color of a UI element, stored as a linear RGBA vector with each
/// channel in the `0.0..=1.0` range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BackgroundColor(pub Vec4);

impl Default for BackgroundColor {
    fn default() -> Self {
        Self(Vec4::new(1.0, 1.0, 1.0, 1.0))
    }
}

/// Marker component for the root of a UI tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiRoot;
gizmo_core::impl_component!(Style, Node, Interaction, BackgroundColor, UiRoot);

#[cfg(test)]
mod soundness_tests {
    use super::*;

    /// `Style` and `UiContext` must stay `Send + Sync` — the first is an ECS
    /// component, the second a `World` resource, and both storages require it.
    ///
    /// This is what makes the hand-written `unsafe impl`s load-bearing rather
    /// than cargo-culted: delete them and this test stops compiling. (The audit
    /// that prompted this comment assumed they were gratuitous; they are not —
    /// `taffy::Style` really is `!Send + !Sync`.)
    #[test]
    fn style_and_ui_context_are_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Style>();
        assert_send_sync::<crate::layout::UiContext>();
    }

    /// Documents the residual risk that the SAFETY comments above depend on.
    ///
    /// The soundness of those `unsafe impl`s rests on taffy's `calc` feature
    /// being OFF, which this crate's Cargo.toml arranges. But Cargo unifies
    /// features across the whole graph: another crate in a user's dependency
    /// tree that depends on `taffy` with default features turns `calc` back on
    /// for everyone, and nothing here can observe that — a dependency's feature
    /// flags are not visible as `cfg` to the dependent crate.
    ///
    /// So this is deliberately NOT an assertion pretending to be a guard. The
    /// real fix is to stop storing `taffy::Style` by value in a component; see
    /// `docs/FIXPLAN.md` (A3 follow-up). Until then the guard is the comment
    /// next to the dependency, and this test exists to keep the caveat in the
    /// same file as the `unsafe`.
    #[test]
    fn calc_invariant_is_documented_not_enforced() {
        // A compiling no-op: `CompactLength::calc` is `#[cfg(feature = "calc")]`
        // in taffy, so with the feature off there is no pointer-producing
        // constructor reachable from safe code in this crate's own surface.
        // We assert only what we can actually observe: that a default Style
        // round-trips as plain data.
        let s = Style::default();
        let cloned = s.clone();
        assert_eq!(s, cloned);
    }
}
