use gizmo_math::{Vec2, Vec4};

/// Layout style of a UI element.
///
/// This is a newtype wrapper around [`taffy::style::Style`] and derefs to it,
/// so all taffy style fields (flexbox, grid, sizing, spacing, ...) are
/// available directly.
#[derive(Clone, Debug, PartialEq)]
#[derive(Default)]
pub struct Style(pub taffy::style::Style);

unsafe impl Send for Style {}
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
