//! The drawable size of the window, published to systems as an ECS resource.
//!
//! [`WindowInfo`] is a plain value type with no tie to any windowing backend — `gizmo-core`
//! has no window dependency. Whoever owns the real window is responsible for refreshing this
//! resource on every resize; nothing here observes the platform on its own.

/// Current drawable size of the window, held as an ECS resource.
///
/// Sizes are *meant* to be physical pixels — the same units as the render surface — so
/// `width/height` is directly usable as a camera aspect ratio and as the denominator when
/// mapping cursor coordinates into normalized device space. Nothing here enforces that unit:
/// the resource holds whatever the window owner last wrote, and a host may seed it with a
/// placeholder before the first real resize arrives. It is also not guaranteed to equal the
/// raw window size — the windowed app loop feeds this from the renderer's *effective* surface
/// extent, which the web backend may cap below the window.
///
/// Nothing validates the values either; treat the fields as untrusted when you divide by
/// them. [`WindowInfo::aspect_ratio`] is the only accessor here that guards against a
/// non-positive height. [`Default`] is 1280x720, a placeholder for headless use.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowInfo {
    /// Width in physical pixels — the numerator of [`WindowInfo::aspect_ratio`].
    ///
    /// Unlike `height` it is not guarded anywhere: a zero width alongside a positive height
    /// yields an aspect ratio of `0.0`, passed on to the caller as-is.
    pub width: f32,
    /// Height in physical pixels — the divisor of [`WindowInfo::aspect_ratio`].
    ///
    /// Being the divisor is the whole reason it is the field `aspect_ratio` special-cases; see
    /// there for what a non-positive height yields.
    pub height: f32,
}

impl WindowInfo {
    /// Builds a `WindowInfo` from an explicit physical-pixel size, width first.
    ///
    /// This is the constructor for a size you already know — typically a surface extent in
    /// `u32` pixels cast to `f32`, which is exact up to 2^24 and so lossless for any real
    /// display. When no real size is known yet, prefer [`Default`], whose 1280x720 at least
    /// yields a sane aspect ratio.
    pub fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    /// Returns `(width, height)` — in that order, physical pixels.
    ///
    /// A by-value copy of the two fields, not a view: a later resize does not show up in a
    /// tuple already taken. To divide the two, use [`WindowInfo::aspect_ratio`].
    pub fn size(&self) -> (f32, f32) {
        (self.width, self.height)
    }

    /// Width divided by height.
    ///
    /// Returns `1.0` whenever `height <= 0.0` (zero *or* negative), so a minimised or
    /// not-yet-sized window yields a harmless square aspect instead of `inf`/`NaN`
    /// propagating into a projection matrix. `width == 0.0` is deliberately *not* special
    /// cased and still returns `0.0`.
    pub fn aspect_ratio(&self) -> f32 {
        if self.height > 0.0 {
            self.width / self.height
        } else {
            1.0
        }
    }
}

impl Default for WindowInfo {
    fn default() -> Self {
        Self {
            width: 1280.0,
            height: 720.0,
        }
    }
}

/// How many frame pixels one *logical* pixel is — the unit a `Val::Px`, a HUD's font size and a touch
/// stick's radius are written in. An ECS resource the windowed app keeps current; without one the
/// scale is 1.
///
/// The engine works in the frame's pixels (`WindowInfo`), and a pixel is not the same size
/// everywhere: a 22 px HUD line is 22 px on a desktop monitor, but a phone packs 2.75 physical
/// pixels into the space of one, and the frame may be rendered below the screen's resolution on
/// top of that. Measured on a Redmi Note 12 Pro (1080 × 2400, scale factor 2.75, frame 656 wide):
/// 1.67 frame pixels per logical pixel, so a 22 px line came out as 13 logical pixels — too small
/// to read comfortably, and a different size again in the browser.
///
/// On an ordinary desktop window it is exactly 1 and nothing changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayScale(f32);

impl DisplayScale {
    /// The scale for a frame `frame_width` pixels wide shown on a window `physical_width` physical
    /// pixels wide at the platform's `scale_factor` (physical pixels per logical pixel). Anything
    /// not positive and finite gives 1: a minimised window has no meaningful scale.
    #[must_use]
    pub fn from_window(frame_width: f32, physical_width: f32, scale_factor: f32) -> Self {
        let k = frame_width * scale_factor / physical_width;
        Self(if k.is_finite() && k > 0.0 { k } else { 1.0 })
    }

    /// Frame pixels per logical pixel.
    #[must_use]
    pub fn get(self) -> f32 {
        self.0
    }
}

impl Default for DisplayScale {
    fn default() -> Self {
        Self(1.0)
    }
}

/// How much of each window edge the system covers, in frame pixels — on a phone, the navigation
/// bar over the bottom of the app. An ECS resource; without one nothing is covered. A control a
/// thumb has to reach belongs inside it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SafeArea {
    /// Covered at the left edge.
    pub left: f32,
    /// Covered at the top edge.
    pub top: f32,
    /// Covered at the right edge.
    pub right: f32,
    /// Covered at the bottom edge.
    pub bottom: f32,
}

/// How the mouse cursor behaves over the window — what a game asks of it, as an ECS resource.
///
/// A first-person or third-person game turns the camera with the mouse and must keep the cursor
/// from leaving the window or showing over the scene; nothing in the engine could ask for that.
/// The windowed app applies this resource to the real window whenever it changes (it compares
/// with what it last applied, so writing it every frame costs nothing):
///
/// ```
/// use gizmo_core::window::{CursorGrab, CursorOptions};
/// # let mut world = gizmo_core::World::new();
/// // Mouse-look on: the cursor is held in place and hidden; motion still arrives as
/// // `Input::mouse_delta`, which is fed from raw device motion, not the cursor position.
/// world.insert_resource(CursorOptions::mouse_look());
/// // A menu opened: give the cursor back.
/// world.insert_resource(CursorOptions::default());
/// # assert_eq!(world.get_resource::<CursorOptions>().map(|c| c.grab), Some(CursorGrab::None));
/// ```
///
/// No resource means the default: free and visible. `gizmo-core` has no window, so nothing here
/// applies it — a headless world simply holds the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct CursorOptions {
    /// Whether the cursor is held.
    pub grab: CursorGrab,
    /// Whether it is drawn.
    pub visible: bool,
}

impl CursorOptions {
    /// Free and visible — the window's own behaviour.
    #[must_use]
    pub const fn free() -> Self {
        Self { grab: CursorGrab::None, visible: true }
    }

    /// Held in place and hidden: what mouse-look wants.
    #[must_use]
    pub const fn mouse_look() -> Self {
        Self { grab: CursorGrab::Locked, visible: false }
    }

    /// Replaces the grab mode.
    #[must_use]
    pub const fn with_grab(mut self, grab: CursorGrab) -> Self {
        self.grab = grab;
        self
    }

    /// Replaces the visibility.
    #[must_use]
    pub const fn with_visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }
}

impl Default for CursorOptions {
    fn default() -> Self {
        Self::free()
    }
}

/// How far a [`CursorOptions`] holds the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum CursorGrab {
    /// Free to leave the window.
    #[default]
    None,
    /// Kept inside the window, still moving.
    Confined,
    /// Held where it is. Where the platform cannot lock (X11), the window falls back to
    /// [`Confined`](Self::Confined) — mouse-look works either way, because it reads raw motion.
    Locked,
}

/// Whether the window fills the screen — what a settings menu or an Alt+Enter handler asks of it,
/// as an ECS resource.
///
/// Nothing in the engine could switch a running game to fullscreen: the window was created
/// windowed and stayed so. The windowed app applies this resource whenever it changes, like
/// [`CursorOptions`]; no resource reads as [`Windowed`](Self::Windowed).
///
/// ```
/// use gizmo_core::window::WindowMode;
/// # let mut world = gizmo_core::World::new();
/// let next = match world.get_resource::<WindowMode>().map(|m| *m).unwrap_or_default() {
///     WindowMode::Windowed => WindowMode::Fullscreen,
///     _ => WindowMode::Windowed,
/// };
/// world.insert_resource(next);
/// # assert_eq!(world.get_resource::<WindowMode>().map(|m| *m), Some(WindowMode::Fullscreen));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum WindowMode {
    /// A normal window.
    #[default]
    Windowed,
    /// Borderless fullscreen on the monitor the window is on — the desktop's resolution, no mode
    /// switch, so Alt+Tab is instant.
    Fullscreen,
}

/// Insert this resource to end the application — the "Quit" button in a menu.
///
/// A game could not ask to quit: the windowed loop exited only when the window was closed, and the
/// headless loop never. Both now check for this after each frame's update and commands, and leave
/// by the same path closing the window takes (the input recording is written, everything drops in
/// order).
///
/// ```
/// # let mut world = gizmo_core::World::new();
/// world.insert_resource(gizmo_core::window::AppExit);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AppExit;

#[cfg(test)]
mod display_scale_tests {
    use super::*;

    /// A desktop window is 1; the phone it was measured on is 1.67; a frame stretched up in a
    /// browser is under 1; a minimised window does not divide by zero.
    #[test]
    fn the_scale_is_frame_pixels_per_logical_pixel() {
        assert_eq!(DisplayScale::from_window(1280.0, 1280.0, 1.0).get(), 1.0);
        assert!((DisplayScale::from_window(656.0, 1080.0, 2.75).get() - 1.67).abs() < 0.01);
        assert!((DisplayScale::from_window(328.0, 1080.0, 2.75).get() - 0.835).abs() < 0.01);
        assert_eq!(DisplayScale::from_window(100.0, 0.0, 1.0).get(), 1.0);
        assert_eq!(DisplayScale::default().get(), 1.0);
    }
}
