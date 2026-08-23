//! Writing the frame that was actually presented to a PNG.
//!
//! # Why this exists
//!
//! The engine had no way to look at its own output. Every visual question — "is the editor
//! viewport washed out?", "did that post-process change alter the picture?" — had to be answered by
//! a human in front of the window, because the alternative, grabbing the screen from outside,
//! does not work here: this desktop runs Xwayland in **rootless** mode, where the X root window
//! holds no content and every screen grab comes back black. Nothing about that is going to change,
//! and it would not help on a CI box with no display at all.
//!
//! So the frame gets read back from the GPU instead, on the engine's side of the compositor. What
//! this captures is the surface texture after every pass and after egui, i.e. exactly the pixels
//! `present()` is about to hand over — not an approximation rendered a second time with different
//! state.
//!
//! # The two things that make a readback wrong
//!
//! Both are handled here, and both are the kind of bug that produces a *plausible* image rather
//! than an obviously broken one, so they are worth naming:
//!
//! - **Row alignment.** `copy_texture_to_buffer` requires `bytes_per_row` to be a multiple of 256.
//!   At 1600 px wide that is already satisfied (6400); at 1590 it is not, and copying with the
//!   unpadded stride either fails validation or, worse, reads the image back with a progressive
//!   horizontal skew. The padded stride is computed, and the padding is dropped row by row.
//! - **Channel order.** Surfaces are usually `Bgra8UnormSrgb`, not RGBA. Ignoring that swaps red
//!   and blue: a picture that still looks like a picture, so nobody notices until a screenshot is
//!   used as evidence about colour — which is the whole point of this module.

use std::path::Path;

/// What went wrong, in terms the caller can print. Capture is a diagnostic, so every failure is a
/// message rather than a panic: a screenshot that cannot be taken must never take the frame with
/// it.
#[derive(Debug)]
pub enum CaptureError {
    /// The texture's format is not one of the 8-bit-per-channel BGRA/RGBA surface formats.
    UnsupportedFormat(wgpu::TextureFormat),
    /// The texture was not created with `COPY_SRC`, so the GPU cannot copy out of it.
    NotCopyable,
    /// `map_async` failed, or the device was lost mid-readback.
    MapFailed,
    /// The pixels were read, but writing the file did not work.
    Write(String),
}

impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedFormat(fmt) => {
                write!(f, "capture: unsupported surface format {fmt:?} (need 8-bit BGRA/RGBA)")
            }
            Self::NotCopyable => write!(
                f,
                "capture: the texture lacks COPY_SRC usage — the surface must be configured with it"
            ),
            Self::MapFailed => write!(f, "capture: mapping the readback buffer failed"),
            Self::Write(e) => write!(f, "capture: writing the file failed: {e}"),
        }
    }
}

impl std::error::Error for CaptureError {}

/// Is this format one this module knows how to unpack, and does it need a red/blue swap?
///
/// Returns `None` for anything else rather than guessing — a wrong guess here is a silently
/// miscoloured screenshot.
fn channel_order(format: wgpu::TextureFormat) -> Option<bool> {
    use wgpu::TextureFormat::*;
    match format {
        Bgra8Unorm | Bgra8UnormSrgb => Some(true),
        Rgba8Unorm | Rgba8UnormSrgb => Some(false),
        _ => None,
    }
}

/// Read `texture` back and write it to `path` as an 8-bit RGBA PNG.
///
/// Call this **after** the frame's commands are submitted and **before** `present()`: the surface
/// texture is still alive at that point and carries the finished frame. The function submits its
/// own copy command and blocks until the readback lands, which costs a frame — it is a diagnostic
/// path, not something to run every frame.
pub fn texture_to_png(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    path: &Path,
) -> Result<(), CaptureError> {
    let format = texture.format();
    let swap_rb = channel_order(format).ok_or(CaptureError::UnsupportedFormat(format))?;
    if !texture.usage().contains(wgpu::TextureUsages::COPY_SRC) {
        return Err(CaptureError::NotCopyable);
    }

    let width = texture.width();
    let height = texture.height();
    let unpadded_row = width * 4;
    // The alignment rule that silently skews the image when ignored.
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded_row = unpadded_row.div_ceil(align) * align;

    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("capture-readback"),
        size: u64::from(padded_row) * u64::from(height),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("capture") });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    );
    queue.submit(std::iter::once(encoder.finish()));

    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    // The copy has to finish before the mapping callback can fire.
    let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
    match rx.recv() {
        Ok(Ok(())) => {}
        _ => return Err(CaptureError::MapFailed),
    }

    let mut pixels = Vec::with_capacity((unpadded_row * height) as usize);
    {
        // wgpu 30 made this fallible: mapping can now report a range error rather than
        // panicking. The capture path already has an error for "the readback did not come back",
        // so it reuses it instead of unwrapping on the caller's behalf.
        let mapped = slice
            .get_mapped_range()
            .map_err(|_| CaptureError::MapFailed)?;
        for row in 0..height as usize {
            let start = row * padded_row as usize;
            let row_bytes = &mapped[start..start + unpadded_row as usize];
            if swap_rb {
                for px in row_bytes.as_chunks::<4>().0 {
                    pixels.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
                }
            } else {
                pixels.extend_from_slice(row_bytes);
            }
        }
    }
    staging.unmap();

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| CaptureError::Write(e.to_string()))?;
        }
    }
    image::RgbaImage::from_raw(width, height, pixels)
        .ok_or_else(|| CaptureError::Write("pixel buffer size did not match the image".into()))?
        .save(path)
        .map_err(|e| CaptureError::Write(e.to_string()))
}

/// The environment-driven capture request the windowed loop honours.
///
/// Env vars rather than CLI flags on purpose: every binary in this workspace (studio, the 39 demos,
/// anything downstream built on `SimpleApp`) gets the capability without touching its argument
/// parsing, and a capture can be requested of a *running* configuration without editing code.
///
/// - `GIZMO_SCREENSHOT` — path to write. Absent means no capture, and nothing in this module runs.
/// - `GIZMO_SCREENSHOT_FRAME` — which frame to grab, default 90. The first frames of any scene are
///   not representative: assets stream in, TAA has not converged, and the editor's layout settles.
/// - `GIZMO_SCREENSHOT_EXIT` — `1` to close the window after writing, which is what a script wants.
#[derive(Debug, Clone)]
pub struct CaptureRequest {
    /// Where the PNG is written.
    pub path: std::path::PathBuf,
    /// Which frame to capture. Early frames are unrepresentative — assets are still streaming in
    /// and the layout has not settled — so this defaults well past the first.
    pub frame: u64,
    /// Whether the process exits once the frame is written, which is what makes this usable from
    /// a script.
    pub exit_after: bool,
}

impl CaptureRequest {
    /// Reads the request from the environment, or `None` if `GIZMO_SCREENSHOT` is unset.
    pub fn from_env() -> Option<Self> {
        let path = std::env::var_os("GIZMO_SCREENSHOT")?;
        let frame = std::env::var("GIZMO_SCREENSHOT_FRAME")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(90);
        let exit_after = std::env::var("GIZMO_SCREENSHOT_EXIT").is_ok_and(|v| v == "1");
        Some(Self { path: path.into(), frame, exit_after })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The red/blue decision, stated as a table so a new surface format cannot quietly default to
    /// "no swap" — the unsupported arm makes it a hard error instead of a miscoloured file.
    #[test]
    fn bgra_swaps_rgba_does_not_and_everything_else_is_refused() {
        use wgpu::TextureFormat::*;
        assert_eq!(channel_order(Bgra8UnormSrgb), Some(true));
        assert_eq!(channel_order(Bgra8Unorm), Some(true));
        assert_eq!(channel_order(Rgba8UnormSrgb), Some(false));
        assert_eq!(channel_order(Rgba8Unorm), Some(false));
        assert_eq!(channel_order(Rgba16Float), None, "HDR formats need tone mapping, not a memcpy");
        assert_eq!(channel_order(Depth32Float), None);
    }

    /// The alignment arithmetic, checked at the widths that actually occur — including the ones
    /// where `bytes_per_row` is already aligned and padding must NOT be added.
    #[test]
    fn row_padding_rounds_up_to_256_only_when_needed() {
        let padded = |w: u32| (w * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * 256;
        assert_eq!(padded(1600), 6400, "1600*4 is already a multiple of 256");
        assert_eq!(padded(64), 256);
        assert_eq!(padded(1590), 6400, "6360 rounds up to 6400");
        assert_eq!(padded(1), 256);
        assert_eq!(padded(1920), 7680);
    }

    #[test]
    fn no_env_var_means_no_capture() {
        // Guards the property the whole feature depends on: an engine build with this compiled in
        // does nothing at all unless asked.
        temp_env_unset("GIZMO_SCREENSHOT");
        assert!(CaptureRequest::from_env().is_none());
    }

    fn temp_env_unset(key: &str) {
        // SAFETY: single-threaded test, and the variable is not read concurrently.
        unsafe { std::env::remove_var(key) };
    }
}
