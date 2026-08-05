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
