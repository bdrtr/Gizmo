// SSGI Temporal Accumulation
//
// Denoises the 1-spp raymarched GI by blending the current (frame-varying) noisy
// estimate with a world-reprojected history — the same scheme as taa.wgsl, but run
// at SSGI's HALF resolution and on the raw GI signal (before the spatial blur).
//
// The raymarch (ssgi.wgsl) now rotates its ray seed every frame, so each frame is an
// independent Monte-Carlo sample of the same lighting; averaging ~1/alpha frames of
// reprojected history converges the salt-and-pepper grain to a smooth solution.

struct SsgiTemporalParams {
    prev_view_proj: mat4x4<f32>,  // previous frame's UNJITTERED view-projection
    alpha:          f32,          // blend weight: 0 = full history, 1 = full current
    // NOTE: three scalar pads (NOT a vec3) — a vec3 has align-16 and would push the
    // struct to 96 bytes while the Rust `[f32; 3]` mirror is 80, tripping the uniform
    // min_binding_size validation. Scalars keep both sides at 80 bytes.
    _pad0:          f32,
    _pad1:          f32,
    _pad2:          f32,
};

@group(0) @binding(0) var<uniform> params: SsgiTemporalParams;
@group(0) @binding(1) var t_current:  texture_2d<f32>;  // raw SSGI this frame     (half-res)
@group(0) @binding(2) var t_history:  texture_2d<f32>;  // accumulated SSGI last frame (half-res)
@group(0) @binding(3) var t_position: texture_2d<f32>;  // world-position G-buffer (full-res)
@group(0) @binding(4) var s_linear:   sampler;          // bilinear — for history reprojection

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    var p = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4(p[vi], 0.0, 1.0);
}

@fragment
fn fs_resolve(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    let half_dims = vec2<f32>(textureDimensions(t_current));
    let iuv_half  = vec2<i32>(i32(frag_coord.x), i32(frag_coord.y));

    let current = textureLoad(t_current, iuv_half, 0).rgb;

    // World position for THIS half-res pixel comes from the full-res G-buffer at the
    // matching texel — the raymarch reads it the same way (iuv = frag_coord * 2).
    let iuv_full = iuv_half * 2;
    let pos_samp = textureLoad(t_position, iuv_full, 0);

    // ── Reproject: where did this world point project last frame? ────────────────
    // NO early returns before the textureSample below — WGSL requires derivative-taking
    // samples to run in uniform control flow. Track validity in a flag instead and fold
    // it into the blend weight at the end.
    var history_uv = frag_coord.xy / half_dims;   // fallback: same pixel
    var history_valid = pos_samp.w >= 0.5;        // sky / unwritten → no GI history
    let prev_clip = params.prev_view_proj * vec4(pos_samp.xyz, 1.0);
    if (prev_clip.w > 0.001) {
        let ndc = prev_clip.xy / prev_clip.w;
        history_uv = vec2(ndc.x * 0.5 + 0.5, ndc.y * -0.5 + 0.5);
    } else {
        history_valid = false;                    // behind previous camera
    }
    if (history_uv.x < 0.0 || history_uv.x > 1.0 || history_uv.y < 0.0 || history_uv.y > 1.0) {
        history_valid = false;                    // off-screen last frame (disocclusion)
    }

    // Re-center the reprojected UV onto the half-res write grid. World position is read
    // at the 2×2 block's top-left full-res texel (iuv_full = iuv_half*2), so its back-
    // projection lands a quarter-texel toward top-left of where THIS half-res pixel writes;
    // without this the feedback loop bakes in a permanent quarter-texel blur/shift.
    history_uv += 0.25 / half_dims;

    // Unconditional sample (uniform control flow); uv is clamped so an invalid reproject
    // reads a safe edge texel — its contribution is discarded via history_valid below.
    var history = textureSample(t_history, s_linear, clamp(history_uv, vec2(0.0), vec2(1.0))).rgb;

    // ── History rejection ────────────────────────────────────────────────────────
    // A neighbourhood colour-clamp (TAA-style) actively FIGHTS convergence for a 1-spp
    // GI signal: every frame it drags the accumulated (smooth) history back toward the
    // current frame's biased local diagonal band, so the bands never average out. For a
    // primarily-static scene the correct denoiser is a straight exponential accumulation,
    // rejecting history ONLY on true disocclusion (reprojection off-screen / behind
    // camera), which `history_valid` already encodes. Motion ghosting on the low-frequency
    // half-res GI is minor and is the acceptable trade for actually converging.
    //
    // (Deliberately NO colour clamp here — see git history for the variance-clip variant.)

    // Invalid history → blend weight 1.0 (take the fresh estimate, ignore history).
    let a = select(1.0, params.alpha, history_valid);
    let resolved = mix(history, current, a);
    return vec4(resolved, 1.0);
}
