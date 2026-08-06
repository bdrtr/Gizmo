//! Test-only wgpu device helpers, shared by every GPU test in this crate.
//!
//! # Why this exists
//!
//! `cargo test -p gizmo-renderer --lib` died with a **SIGSEGV** — not a Rust panic, so no test
//! name was ever reported and the whole binary (and with it the workspace run) went down.
//! Nineteen tests, spread over **seven** test modules in six files, each requested their own
//! wgpu adapter and device, and `cargo test` runs a binary's tests in parallel. The module
//! count is worth stating precisely: `gpu_fluid::fluid_tests` is pure CPU and stays out, but
//! `gpu_fluid::system::gpu_dispatch_tests` is a DIFFERENT module in the same subtree and is
//! GPU — it was nearly missed for exactly that reason.
//!
//! Measured on this machine (16 cores, so the default is 16 test threads):
//!
//! | `--test-threads` | result |
//! |---|---|
//! | 1 | no segfault |
//! | 2 | **clean, 147/147 in 6.4 s** |
//! | 4 | SIGSEGV |
//! | 8 | SIGSEGV |
//! | 16 (default) | SIGSEGV, 3/3 runs |
//!
//! Two concurrent devices are fine and four are not, and no individual test changed between
//! those runs — so it is the concurrency, not any one test.
//!
//! # What was tried first, and REFUTED
//!
//! `gizmo`'s `golden_render_tests::gpu_lock` (the same class of crash in the sibling crate) is
//! documented as being about racing *device creation*, so the first fix here serialised only the
//! creation call and let the tests run their GPU work concurrently. **It did not work: 5/5 runs
//! still crashed.** The narrower claim is wrong — with creation serialised, four threads still
//! bring the driver down, so what the driver cannot take is several devices being *alive and in
//! use* at once, not the moment they are made.
//!
//! Hence the guard is bound for the whole test body, which is what `gizmo` does too (its doc
//! attributes that to creation; on this evidence that attribution is too narrow, though the fix
//! it chose is the right one).
//!
//! # Scope
//!
//! `gpu_fluid::fluid_tests` deliberately does NOT take the lock: those ~20 tests are pure CPU
//! math and never touch a device, so serialising them would cost wall-clock for nothing.

/// Serialises every test in this crate that owns a wgpu device.
///
/// Bind it for the WHOLE test — `let _gpu = gpu_lock();` as the first statement — not just
/// around device creation. See the module docs: creation-only was measured and refuted.
///
/// Poisoning is deliberately ignored: if one GPU test panics, the rest should still get to run
/// and report their own results rather than cascading into a poisoned-mutex failure.
pub(crate) fn gpu_lock() -> std::sync::MutexGuard<'static, ()> {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    GPU.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A headless device + queue with default limits.
///
/// Returns `None` when the machine has no usable adapter, which is how every caller skips
/// rather than fails on a GPU-less box (CI containers included).
///
/// **Does not lock** — the caller must already hold [`gpu_lock`]. `std::sync::Mutex` is not
/// reentrant, so locking here as well would deadlock every test that holds the guard.
pub(crate) async fn headless_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        flags: wgpu::InstanceFlags::default(),
        memory_budget_thresholds: Default::default(),
        backend_options: Default::default(),
        display: None,
    });
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        })
        .await
        .ok()?;
    adapter
        .request_device(&wgpu::DeviceDescriptor::default())
        .await
        .ok()
}
