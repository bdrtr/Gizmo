// Auto exposure: one step of adaptation, on the GPU.
//
// `sums[0]` is this frame's mean HDR luminance, written by `luminance_reduce.wgsl` just before.
// The exposure that would bring it to `key` is `key / mean`; the current exposure moves toward it
// in log2 space (stops — equal ratios are equal steps, as the eye judges them) by
// `1 - exp(-dt * speed)`, a frame-rate-independent fraction. Darkening the picture (the scene got
// brighter) and brightening it (the scene got darker) have separate speeds, the way eyes adapt
// to light faster than to dark. `reset` snaps straight to the target: the first frame after the
// effect is enabled starts converged instead of fading in from 1.0.
//
// The result stays in `exposure[0]` for the composite pass to multiply by; the CPU never reads it.

struct Params {
    key: f32,
    adapt_to_bright: f32,
    adapt_to_dark: f32,
    min_exposure: f32,
    max_exposure: f32,
    dt: f32,
    reset: f32,
    _pad: f32,
};

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> sums: array<f32>;
@group(0) @binding(2) var<storage, read_write> exposure: array<f32>;

@compute @workgroup_size(1)
fn adapt() {
    let mean = max(sums[0], 1e-5);
    let target_exposure = clamp(p.key / mean, p.min_exposure, p.max_exposure);
    let current = exposure[0];
    // A non-positive or NaN current (never written, or poisoned) cannot be adapted from in log
    // space; take the target.
    if (p.reset > 0.5 || !(current > 0.0)) {
        exposure[0] = target_exposure;
        return;
    }
    // Exposure going down means the scene got brighter.
    let speed = select(p.adapt_to_dark, p.adapt_to_bright, target_exposure < current);
    let t = 1.0 - exp(-max(p.dt, 0.0) * speed);
    exposure[0] = exp2(mix(log2(current), log2(target_exposure), t));
}
