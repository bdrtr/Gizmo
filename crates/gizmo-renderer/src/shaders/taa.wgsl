// Temporal Anti-Aliasing
//
// Two entry points:
//   fs_resolve — blend jittered current frame with clamped history → taa output
//   fs_blit    — copy taa output → hdr texture (so post-process is unchanged)

struct TaaParams {
    prev_view_proj: mat4x4<f32>,
    jitter:         vec2<f32>,   // current frame subpixel offset (NDC)
    alpha:          f32,         // temporal blend weight (0=full history, 1=full current)
    _pad:           f32,
};

@group(0) @binding(0) var<uniform> params:     TaaParams;
@group(0) @binding(1) var t_current:  texture_2d<f32>;  // jittered current HDR frame
@group(0) @binding(2) var t_history:  texture_2d<f32>;  // previous TAA output
@group(0) @binding(3) var t_position: texture_2d<f32>;  // world-position G-buffer
@group(0) @binding(4) var s_linear:   sampler;          // bilinear — for history
@group(0) @binding(5) var s_nearest:  sampler;          // nearest — for current / position

// Shared fullscreen-triangle vertex shader
@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    var p = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4(p[vi], 0.0, 1.0);
}

// ── Resolve pass ──────────────────────────────────────────────────────────────
@fragment
fn fs_resolve(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    let dims     = vec2<f32>(textureDimensions(t_current));
    let iuv      = vec2<i32>(i32(frag_coord.x), i32(frag_coord.y));
    let uv       = frag_coord.xy / dims;

    let current = textureLoad(t_current, iuv, 0).rgb;

    // ── Reproject: find where this world-space point was last frame ───────────
    var history_uv = uv; // fallback: no movement
    let pos_samp = textureLoad(t_position, iuv, 0);
    if (pos_samp.w >= 0.5) {
        let prev_clip = params.prev_view_proj * vec4(pos_samp.xyz, 1.0);
        if (prev_clip.w > 0.001) {
            let ndc = prev_clip.xy / prev_clip.w;
            history_uv = vec2(ndc.x * 0.5 + 0.5, ndc.y * -0.5 + 0.5);
        }
    }

    // ── Sample history ────────────────────────────────────────────────────────
    var history = textureSample(t_history, s_linear, history_uv).rgb;

    // ── Neighbourhood AABB clamp (prevents ghosting from disoccluded regions) ─
    var c_min = current;
    var c_max = current;
    for (var dx = -1; dx <= 1; dx++) {
        for (var dy = -1; dy <= 1; dy++) {
            let n = textureLoad(t_current, iuv + vec2(dx, dy), 0).rgb;
            c_min = min(c_min, n);
            c_max = max(c_max, n);
        }
    }
    history = clamp(history, c_min, c_max);

    // ── Temporal blend ────────────────────────────────────────────────────────
    let resolved = mix(history, current, params.alpha);
    return vec4(resolved, 1.0);
}

// ── Blit pass: copy TAA output back into the HDR texture ─────────────────────
@group(1) @binding(0) var t_taa_out: texture_2d<f32>;
@group(1) @binding(1) var s_blit:    sampler;

@fragment
fn fs_blit(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    let iuv = vec2<i32>(i32(frag_coord.x), i32(frag_coord.y));
    return textureLoad(t_taa_out, iuv, 0);
}
