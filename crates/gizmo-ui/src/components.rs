use gizmo_math::{Vec2, Vec4};

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

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub size: Vec2,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(Default)]
pub enum Interaction {
    #[default]
    None,
    Hovered,
    Pressed,
}


#[derive(Clone, Debug, PartialEq)]
pub struct BackgroundColor(pub Vec4);

impl Default for BackgroundColor {
    fn default() -> Self {
        Self(Vec4::new(1.0, 1.0, 1.0, 1.0))
    }
}

// Marker component for the root of a UI tree
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiRoot;
gizmo_core::impl_component!(Style, Node, Interaction, BackgroundColor, UiRoot);
