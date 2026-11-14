// Temporal Anti-Aliasing
//
// Two entry points:
//   fs_resolve — blend jittered current frame with clamped history → taa output
//   fs_blit    — copy taa output → hdr texture (so post-process is unchanged)

struct TaaParams {
    jitter:         vec2<f32>,   // current frame subpixel offset (NDC)
    alpha:          f32,         // 1 = no history to keep (first frame, after a reset)
    _pad:           f32,
};

@group(0) @binding(0) var<uniform> params:     TaaParams;
@group(0) @binding(1) var t_current:  texture_2d<f32>;  // jittered current HDR frame
@group(0) @binding(2) var t_history:  texture_2d<f32>;  // previous TAA output
@group(0) @binding(3) var t_position_rel_camera: texture_2d<f32>;  // world-position G-buffer
@group(0) @binding(4) var s_linear:   sampler;          // bilinear — for history
@group(0) @binding(5) var s_nearest:  sampler;          // nearest — for current / position
// Where each pixel's surface was last frame, in screen UV: now − before, camera and object motion
// together (`gbuffer.wgsl`, `crate::motion`).
@group(0) @binding(6) var t_velocity: texture_2d<f32>;

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

    // Un-jitter the current frame: the jittered projection moved the picture by +jitter in NDC (half
    // that in UV, y flipped), so texel q holds what belongs at q − jitter, and what belongs here is
    // read at uv + jitter. It was read at uv − jitter, which put twice the jitter into every frame
    // instead of none: a still scene shimmered along every edge (`taa_settles_on_a_still_scene`).
    let current_uv = uv + params.jitter * vec2<f32>(0.5, -0.5);
    let current = textureSample(t_current, s_linear, current_uv).rgb;

    // ── Reproject: where this pixel's surface was last frame ──────────────────
    // The velocity is that of the surface the jittered raster found here, between two unjittered
    // projections of it; the jitter offsets of "where it is" and "where it was" cancel, so the history
    // is read at uv − velocity. It reprojected through the previous view-projection alone until
    // 2026-11-14, which knows the camera's motion and not an object's: a piece sliding across the
    // chess board left a dark ghost where it had been (`taa_does_not_trail_a_moving_object`).
    //
    // The velocity is the nearest surface's in the 3×3 around this pixel: where a moving object has
    // just uncovered the floor, the floor pixel takes the object's motion and reads its history from
    // behind where the object came from — the floor — instead of from its own place, which last frame
    // was the object (a bright band trailing a moving piece).
    let last_px = vec2<i32>(textureDimensions(t_current)) - vec2<i32>(1);
    var history_uv = uv; // fallback: no surface here or around (the sky)
    var nearest = 1e30;
    for (var vy = -1; vy <= 1; vy++) {
        for (var vx = -1; vx <= 1; vx++) {
            let at = clamp(iuv + vec2(vx, vy), vec2<i32>(0), last_px);
            let p = textureLoad(t_position_rel_camera, at, 0);
            let d = dot(p.xyz, p.xyz);
            if (p.w >= 0.5 && d < nearest) {
                nearest = d;
                history_uv = uv - textureLoad(t_velocity, at, 0).xy;
            }
        }
    }

    // ── Sample history ────────────────────────────────────────────────────────
    var history = textureSample(t_history, s_linear, history_uv).rgb;

    // ── Neighbourhood AABB clamp (prevents ghosting from disoccluded regions) ─
    let last = vec2<i32>(textureDimensions(t_current)) - vec2<i32>(1);
    var c_min = current;
    var c_max = current;
    for (var dx = -1; dx <= 1; dx++) {
        for (var dy = -1; dy <= 1; dy++) {
            // Clamped: a load past the edge reads zero, which would keep black in range here.
            let n = textureLoad(t_current, clamp(iuv + vec2(dx, dy), vec2<i32>(0), last), 0).rgb;
            c_min = min(c_min, n);
            c_max = max(c_max, n);
        }
    }
    history = clamp(history, c_min, c_max);

    // ── Temporal blend ────────────────────────────────────────────────────────
    // How much of this frame goes in follows how far the point moved: a still pixel keeps more of its
    // history (1/20), so the jitter cycle's edge samples average out instead of showing through as a
    // shimmer (at 1/10 an edge kept about an eighth of the cycle's swing); a moving one takes up to
    // 1/5, so it does not trail. `params.alpha` is 1 when there is no history to keep.
    let moved = length((history_uv - uv) * dims);
    let weight = select(mix(0.05, 0.2, clamp(moved * 0.5, 0.0, 1.0)), 1.0, params.alpha >= 1.0);
    let resolved = mix(history, current, weight);
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
