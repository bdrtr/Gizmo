//! Test-only wgpu serialisation for this crate, mirroring `gizmo-renderer`'s `test_gpu`.
//!
//! # Why it moved here
//!
//! This lock used to live as a private `gpu_lock()` inside
//! `systems::render::golden_render_tests`, where only that module could reach it. That left
//! `systems::streaming`'s test building its own headless `Renderer` — a full wgpu device — with
//! no serialisation at all, free to run concurrently with a golden render test.
//!
//! That is a concrete explanation for a residual `docs/FIXPLAN.md` had recorded as roughly two
//! crashes in twelve full-workspace runs and attributed to "driver/system level, under sustained
//! load". Measured in `gizmo-renderer` (see `gizmo_renderer`'s own `test_gpu`), two concurrently
//! live devices are survivable and four are fatal — and one serialised device plus one unlocked
//! device is exactly two, sitting in the marginal band.
//!
//! # The trap this module exists to avoid
//!
//! Two independently declared `static Mutex`es do not serialise against each other. Pasting a
//! second lock into `streaming.rs` would have compiled, read like a fix, and changed nothing.
//! There is one static, here, and every GPU test in the crate takes it.

/// Serialises every test in this crate that owns a wgpu device.
///
/// Bind it for the WHOLE test — `let _gpu = gpu_lock();` as the first statement. Locking only
/// around device creation was measured in `gizmo-renderer` and does NOT work: the driver's limit
/// is on devices concurrently alive and in use, not on the moment they are created.
///
/// Poisoning is deliberately ignored: if one GPU test panics, the others should still get a
/// chance to run and report their own results rather than cascading.
pub(crate) fn gpu_lock() -> std::sync::MutexGuard<'static, ()> {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    GPU.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A headless [`Renderer`](gizmo_renderer::Renderer) built on **the one device this test binary
/// owns**.
///
/// Every golden test needs its own renderer — it carries TAA/GI history between frames, and the
/// tests are written as "one frame from a clean state" — but nothing needs its own *device*.
/// Creating one per renderer is what `docs/FIXPLAN.md` measured as the residual flake (a
/// driver-level crash with no panic and no test name), and this file had grown from ~5 such
/// devices to 15 as the render guards were added; the same shape kills a long sweep outright
/// (`radv/amdgpu: Not enough memory for command submission`, measured at ~17 devices).
///
/// The device is created once, on the first call, and lives for the process — which is what makes
/// the count 1 no matter how many renderers the suite builds.
pub(crate) async fn headless_renderer(width: u32, height: u32) -> gizmo_renderer::Renderer {
    use std::sync::OnceLock;
    static DEVICE: OnceLock<(wgpu::Device, wgpu::Queue)> = OnceLock::new();

    // `get_or_init` cannot await, and the acquisition is behind `gpu_lock()` at every call site
    // anyway, so the race this looks like cannot happen: only one test runs GPU work at a time.
    if DEVICE.get().is_none() {
        let dq = gizmo_renderer::Renderer::headless_device().await;
        let _ = DEVICE.set(dq);
    }
    let (device, queue) = DEVICE.get().expect("device just initialised");
    gizmo_renderer::Renderer::new_headless_with_device(
        device.clone(),
        queue.clone(),
        width,
        height,
        None,
    )
}
