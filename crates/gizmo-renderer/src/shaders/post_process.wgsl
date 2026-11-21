// ============================================================
// Yelbegen Engine — Post-Processing Shader
// Bloom (Bright Extract + Gaussian Blur) ve ACES Tone Mapping + Sinematikler


struct FullscreenVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) vertex_index: u32) -> FullscreenVertexOutput {
    var out: FullscreenVertexOutput;
    let x = f32(i32(vertex_index) / 2) * 4.0 - 1.0;
    let y = f32(i32(vertex_index) % 2) * 4.0 - 1.0;
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return out;
}

@group(0) @binding(0) var t_source: texture_2d<f32>;
@group(0) @binding(1) var s_source: sampler;


const DOF_TAPS: i32 = 24;
const DOF_SEARCH_TAPS: i32 = 12;

// Depth of field at this distance: x = blur radius in texels, y = its share of the largest. From the
// focus band, or through a lens (`PostProcess::with_lens`). `params.motion.w` is the farthest a
// distance may count as, for a sky that has none of its own.
fn dof_radius_at(distance: f32) -> vec2<f32> {
    if (params.dof_blur_size <= 0.0) {
        return vec2<f32>(0.0);
    }
    let d = select(distance, min(distance, params.motion.w), params.motion.w > 0.0);
    if (params.motion.z > 0.0) {
        let radius = min(params.motion.z * abs(d - params.dof_focus_dist) / max(d, 0.001), params.dof_blur_size);
        return vec2<f32>(radius, radius / params.dof_blur_size);
    }
    let coc = clamp(abs(d - params.dof_focus_dist) / params.dof_focus_range, 0.0, 1.0);
    return vec2<f32>(coc * params.dof_blur_size, coc);
}

// Camera motion blur. A pixel's depth puts it back in clip space; `reproject` says where that
// point was on screen last frame, and the difference is how far the CAMERA moved it. The HDR frame
// is averaged along that path, centred on the pixel (the shutter opens before and closes after the
// instant drawn), over `shutter` of the frame interval. Only camera motion: an object moving in a
// still view has no velocity here. Sky pixels sit at depth 1 and reproject at the far plane, so a
// turning camera smears the sky too, and a translating one barely does — as with a real lens.
fn camera_motion_blur(uv: vec2<f32>, base: vec3<f32>, depth: f32) -> vec3<f32> {
    let shutter = params.motion.x;
    if (shutter <= 0.0) {
        return base;
    }
    var velocity = vec2<f32>(0.0);
    // **Neighbour max** (after McGuire et al. 2012, without the tiles): velocity exists only where a
    // moving object IS, so the background it just uncovered — where its trail belongs — reads zero,
    // and the smear would stay inside the silhouette. Look along both axes as far as a blur can
    // reach and take the fastest motion found: a pixel next to a moving object blurs with it.
    let dims = vec2<f32>(textureDimensions(t_velocity));
    let reach = params.motion.y;
    var neighbour = vec2<f32>(0.0);
    for (var k = -2; k <= 2; k = k + 1) {
        if (k == 0) { continue; }
        let step = f32(k) * 0.5 * reach;
        for (var axis = 0; axis < 2; axis = axis + 1) {
            let offset = select(vec2<f32>(0.0, step), vec2<f32>(step, 0.0), axis == 0);
            let p = vec2<i32>(clamp((uv + offset) * dims, vec2<f32>(0.0), dims - 1.0));
            let v = textureLoad(t_velocity, p, 0).xy;
            if (dot(v, v) > dot(neighbour, neighbour)) {
                neighbour = v;
            }
        }
    }
    if (depth < 1.0) {
        // Geometry: the G-buffer's velocity — object motion as well as the camera's.
        let pixel = vec2<i32>(clamp(uv * dims, vec2<f32>(0.0), dims - 1.0));
        velocity = textureLoad(t_velocity, pixel, 0).xy;
        if (dot(neighbour, neighbour) > dot(velocity, velocity)) {
            velocity = neighbour;
        }
        velocity = velocity * shutter;
    } else if (dot(neighbour, neighbour) > 0.0) {
        // Sky right beside moving geometry: its trail.
        velocity = neighbour * shutter;
    } else {
        // Sky: nothing in the G-buffer, so the camera's own reprojection.
        let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
        let prev_clip = params.reproject * vec4<f32>(ndc, depth, 1.0);
        if (prev_clip.w <= 1e-6) {
            return base;
        }
        let prev_ndc = prev_clip.xy / prev_clip.w;
        velocity = (ndc - prev_ndc) * vec2<f32>(0.5, -0.5) * shutter;
    }
    let len = length(velocity);
    let max_len = params.motion.y;
    if (len > max_len) {
        velocity = velocity * (max_len / len);
    }
    if (length(velocity * vec2<f32>(textureDimensions(t_source))) < 0.5) {
        return base;
    }
    let samples = 12;
    var sum = vec3<f32>(0.0);
    for (var i = 0; i < samples; i = i + 1) {
        let t = (f32(i) + 0.5) / f32(samples) - 0.5;
        sum += textureSampleLevel(t_source, s_source, uv - velocity * t, 0.0).rgb;
    }
    return sum / f32(samples);
}


// `PostProcessParams`, `params` (group 2), the tone curves, the vignette and the dither are in
// `post_common.wgsl`, which `post_process::post_shader_source` puts in front of this file.

// The exposure auto exposure computed this frame (`auto_exposure.wgsl`), or exactly 1.0 while it is
// off — so the product below is bit-identical then. On top of `params.exposure`, the camera's.
@group(3) @binding(0)
var<storage, read> auto_exposure: array<f32>;

// The G-buffer's per-pixel motion since last frame, in screen UV (`crate::motion`): object and
// camera motion together, zero where the G-buffer drew nothing (the sky).
@group(4) @binding(0)
var t_velocity: texture_2d<f32>;

// Local exposure (`crate::local_exposure`): the frame's luminance as a bilateral grid, and how its
// contrast is compressed. A one-texel grid and `flags.x` 0 while it is off.
struct LocalParams {
    grid: vec4<u32>,
    range: vec4<f32>,
    shape: vec4<f32>,
    flags: vec4<f32>,
}
@group(5) @binding(0) var t_local: texture_3d<f32>;
@group(5) @binding(1) var s_local: sampler;
@group(5) @binding(2) var<uniform> local: LocalParams;

// `colour` (before exposure) exposed for its own neighbourhood: the neighbourhood's log2 luminance `b`,
// among the pixels about as bright as this one, from the grid; pulled towards the pivot by the contrast
// on its side, and this pixel's difference from it kept by `detail` — all in log2 of the luminance as
// `exposure` exposes it. A factor on the colour, so its hue is kept.
fn local_exposure(colour: vec3<f32>, uv: vec2<f32>, exposure: f32) -> vec3<f32> {
    if (local.flags.x < 0.5) {
        return colour;
    }
    let l = dot(colour, vec3<f32>(0.2126, 0.7152, 0.0722));
    if (!(l > 0.0)) {
        return colour;
    }
    let l2 = clamp(log2(l), local.range.x, local.range.y);
    let grid = vec3<f32>(textureDimensions(t_local));
    let covered = grid.xy * f32(local.grid.w);
    let at = vec3<f32>(uv * vec2<f32>(textureDimensions(t_source)) / covered, (l2 - local.range.x) / (local.range.y - local.range.x));
    let cell = textureSampleLevel(t_local, s_local, at, 0.0).xy;
    let b = select(l2, cell.x / cell.y, cell.y > 1e-4);
    let e = log2(max(exposure, 1e-8));
    let pivot = local.shape.z;
    let be = b + e;
    let contrast = select(local.shape.y, local.shape.x, be > pivot);
    let out = pivot + (be - pivot) * contrast + (l2 - b) * local.shape.w;
    return colour * exp2(out - (l2 + e));
}


// ============================
// Bloom: a chain of levels, each half the size of the one above (`post_process::BloomChain`)
// ============================
// Down the chain the frame's bright part is filtered into ever smaller levels; up it, each level takes
// the one below it in, spread by a tent, in the share `Renderer::bloom_scatter` gives (the up pass's
// blend). The first level, half the frame's size, is the bloom the composite adds: the bright part
// blurred at every scale at once, wide or tight as the scatter has it — the bloom of Unreal's and of Call
// of Duty: Advanced Warfare's (Jimenez, "Next Generation Post Processing", 2014).

// The share of a colour that blooms.
fn bright_part(color: vec3<f32>) -> vec3<f32> {
    let luminance = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
    let threshold = params.bloom_threshold;
    let soft_threshold = threshold * (1.0 - params.tonemap.w); // `Renderer::bloom_softness`
    let knee = max(luminance - soft_threshold, 0.0) / (threshold - soft_threshold + 0.0001);
    return color * clamp(knee * knee, 0.0, 1.0);
}

// Jimenez's 13 taps, each a bilinear tap between four texels of `t_source`, around `uv`.
fn taps13(uv: vec2<f32>) -> array<vec3<f32>, 13> {
    let t = 1.0 / vec2<f32>(textureDimensions(t_source));
    return array<vec3<f32>, 13>(
        textureSampleLevel(t_source, s_source, uv + t * vec2<f32>(-2.0, -2.0), 0.0).rgb,
        textureSampleLevel(t_source, s_source, uv + t * vec2<f32>(0.0, -2.0), 0.0).rgb,
        textureSampleLevel(t_source, s_source, uv + t * vec2<f32>(2.0, -2.0), 0.0).rgb,
        textureSampleLevel(t_source, s_source, uv + t * vec2<f32>(-1.0, -1.0), 0.0).rgb,
        textureSampleLevel(t_source, s_source, uv + t * vec2<f32>(1.0, -1.0), 0.0).rgb,
        textureSampleLevel(t_source, s_source, uv + t * vec2<f32>(-2.0, 0.0), 0.0).rgb,
        textureSampleLevel(t_source, s_source, uv, 0.0).rgb,
        textureSampleLevel(t_source, s_source, uv + t * vec2<f32>(2.0, 0.0), 0.0).rgb,
        textureSampleLevel(t_source, s_source, uv + t * vec2<f32>(-1.0, 1.0), 0.0).rgb,
        textureSampleLevel(t_source, s_source, uv + t * vec2<f32>(1.0, 1.0), 0.0).rgb,
        textureSampleLevel(t_source, s_source, uv + t * vec2<f32>(-2.0, 2.0), 0.0).rgb,
        textureSampleLevel(t_source, s_source, uv + t * vec2<f32>(0.0, 2.0), 0.0).rgb,
        textureSampleLevel(t_source, s_source, uv + t * vec2<f32>(2.0, 2.0), 0.0).rgb,
    );
}

// The 13 taps weighed: the 4×4 box in the middle 0.5, the four 2×2 boxes round it 0.125 each — a 6×6
// footprint whose weights add to one, so a level holds the light of the one above it.
fn weigh13(taps: array<vec3<f32>, 13>) -> vec3<f32> {
    let middle = (taps[3] + taps[4] + taps[8] + taps[9]) * 0.125;
    let corners = (taps[0] + taps[1] + taps[5] + taps[6]) + (taps[1] + taps[2] + taps[6] + taps[7])
        + (taps[5] + taps[6] + taps[10] + taps[11]) + (taps[6] + taps[7] + taps[11] + taps[12]);
    return middle + corners * 0.03125;
}

// The first level, from the frame: its bright part, each tap's, filtered down to half its size.
@fragment
fn fs_bloom_prefilter(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    var taps = taps13(in.uv);
    for (var k = 0; k < 13; k++) {
        taps[k] = bright_part(taps[k]);
    }
    return vec4<f32>(weigh13(taps), 1.0);
}

// A level from the one above it.
@fragment
fn fs_bloom_down(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(weigh13(taps13(in.uv)), 1.0);
}

// A level's share of the one below it, `t_source`, spread by a 3×3 tent a texel of it wide: what the up
// pass's blend weighs by the scatter against what the level holds.
@fragment
fn fs_bloom_up(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let t = 1.0 / vec2<f32>(textureDimensions(t_source));
    var sum = textureSampleLevel(t_source, s_source, in.uv, 0.0).rgb * 4.0;
    sum += (textureSampleLevel(t_source, s_source, in.uv + vec2<f32>(t.x, 0.0), 0.0).rgb
        + textureSampleLevel(t_source, s_source, in.uv - vec2<f32>(t.x, 0.0), 0.0).rgb
        + textureSampleLevel(t_source, s_source, in.uv + vec2<f32>(0.0, t.y), 0.0).rgb
        + textureSampleLevel(t_source, s_source, in.uv - vec2<f32>(0.0, t.y), 0.0).rgb) * 2.0;
    sum += textureSampleLevel(t_source, s_source, in.uv + t, 0.0).rgb
        + textureSampleLevel(t_source, s_source, in.uv - t, 0.0).rgb
        + textureSampleLevel(t_source, s_source, in.uv + vec2<f32>(t.x, -t.y), 0.0).rgb
        + textureSampleLevel(t_source, s_source, in.uv + vec2<f32>(-t.x, t.y), 0.0).rgb;
    return vec4<f32>(sum / 16.0, 1.0);
}

// ============================
// Lens flare: ghosts and a halo of what is brightest in the frame (`post_process::LensFlare`)
// ============================
// Unreal's image-based lens flare, after John Chapman's ("Pseudo Lens Flare", 2013): a lens's elements
// reflect a bright light between them, and those reflections land on the image as copies of it about its
// middle — the ghosts — and as a ring, the halo, each blurred by the aperture. What flares is taken from
// the bloom's first level, half the frame's size, texel by texel (`fs_flare_extract`) — a lamp a few
// pixels across keeps its brightness there, where a quarter-size level spread it as thin as a lit window
// — and only then filtered down a chain of its own to a sixteenth (`post_process::BloomChain`), whose last
// level the ghosts and the halo are copies of: soft, as a lens's are.
struct FlareParams {
    intensity: f32,
    threshold: f32,
    halo: f32,
    chromatic: f32,
};
@group(1) @binding(0) var<uniform> flare: FlareParams;

// The eight ghosts: each the bright part scaled about the frame's middle by `w` — a negative scale mirrors
// it through the middle, where a lens's ghosts lie, on the line from the light through the centre — and
// tinted by `rgb`: Unreal's `LensFlareTints` defaults, their alpha folded in.
const GHOSTS = array<vec4<f32>, 8>(
    vec4<f32>(0.60, 0.48, 0.24, -1.00),
    vec4<f32>(0.53, 0.53, 0.32, -0.62),
    vec4<f32>(0.37, 0.37, 0.46, -0.38),
    vec4<f32>(0.20, 0.39, 0.16, -0.22),
    vec4<f32>(0.16, 0.25, 0.31, 0.30),
    vec4<f32>(0.24, 0.27, 0.22, 0.55),
    vec4<f32>(0.22, 0.18, 0.09, 0.85),
    vec4<f32>(0.14, 0.11, 0.11, 1.30),
);

// What of `c` flares: its luminance past `flare.threshold`, its hue kept.
fn flare_part(c: vec3<f32>) -> vec3<f32> {
    let l = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    return c * (max(l - flare.threshold, 0.0) / max(l, 1e-4));
}

// The flare's source, a quarter of the frame's size, from the bloom's first level (`t_source`, half of it):
// each texel the mean of what of the four under it flares, each thresholded before they are averaged.
@fragment
fn fs_flare_extract(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(t_source));
    let base = vec2<i32>(in.position.xy) * 2;
    var sum = vec3<f32>(0.0);
    for (var k = 0; k < 4; k++) {
        let texel = min(base + vec2<i32>(k & 1, k >> 1), size - 1);
        sum += flare_part(textureLoad(t_source, texel, 0).rgb);
    }
    return vec4<f32>(sum * 0.25, 1.0);
}

// The anamorphic streaks (`post_process::LensFlare::streak`): a light drawn out sideways, as an
// anamorphic lens draws it, by Kawase's streak filter ("Frame Buffer Postprocessing Effects in DOUBLE-S.T.E.A.L",
// GDC 2003). Four passes along one way of X over the flare's quarter-size source, each of four taps 0, 1,
// 2 and 3 times `step` texels on, `step` 1, 4, 16 and 64: every distance up to 255 texels is reached by one
// choice of tap in each pass — its digits in base four — with the product of their weights, `attenuation`
// to the power of the distance. So the four passes are a kernel that falls off exactly as that power, and
// normalised pass by pass it keeps the light it spreads. Drawn once each way (`direction`); a symmetric
// pass, taps either side, reaches a distance more ways than one, and beaded the streak. The last pass of
// each way adds `scale` of it, the tint times the strength (halved, the light's own texel being in both).
// The first pass of each way takes only what passes `threshold` more than the flare's source already
// passed (`LensFlare::streak_threshold`): the brightest points streak, a car's lit roof does not.
struct StreakPass {
    step: f32,
    attenuation: f32,
    direction: f32,
    threshold: f32,
    scale: vec4<f32>,
};
@group(1) @binding(0) var<uniform> streak_pass: StreakPass;

@fragment
fn fs_flare_streak(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let texel = 1.0 / f32(textureDimensions(t_source).x);
    var sum = vec3<f32>(0.0);
    var weight = 0.0;
    for (var k = 0; k < 4; k++) {
        let w = pow(streak_pass.attenuation, streak_pass.step * f32(k));
        let at = in.uv - vec2<f32>(streak_pass.direction * f32(k) * streak_pass.step * texel, 0.0);
        let c = textureSampleLevel(t_source, s_source, at, 0.0).rgb;
        let l = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
        sum += c * (max(l - streak_pass.threshold, 0.0) / max(l, 1e-4)) * w;
        weight += w;
    }
    return vec4<f32>(sum / weight * streak_pass.scale.rgb, 1.0);
}

// The flare's soft source (`t_source`, the last level of its chain, a sixteenth of the frame) at `uv`,
// through a cubic B-spline: four bilinear taps weighted as the spline's sixteen texels are (Sigg and
// Hadwiger, GPU Gems 2, ch. 20). The flare is drawn at four times the source's size, and a bilinear tap
// magnified that much leaves its texels' squares and diamonds in every ghost; the spline leaves a round one.
fn flare_bright(uv: vec2<f32>) -> vec3<f32> {
    let size = vec2<f32>(textureDimensions(t_source));
    let p = uv * size - 0.5;
    let i = floor(p);
    let f = p - i;
    let w0 = (1.0 - f) * (1.0 - f) * (1.0 - f) / 6.0;
    let w1 = (3.0 * f * f * f - 6.0 * f * f + 4.0) / 6.0;
    let w2 = (-3.0 * f * f * f + 3.0 * f * f + 3.0 * f + 1.0) / 6.0;
    let w3 = f * f * f / 6.0;
    let g0 = w0 + w1;
    let g1 = w2 + w3;
    // Where each pair of texels' bilinear tap falls, in texels, and then in UV.
    let x0 = (i - 1.0 + w1 / g0 + 0.5) / size;
    let x1 = (i + 1.0 + w3 / g1 + 0.5) / size;
    return g0.y * (g0.x * flare_tap(vec2<f32>(x0.x, x0.y)) + g1.x * flare_tap(vec2<f32>(x1.x, x0.y)))
        + g1.y * (g0.x * flare_tap(vec2<f32>(x0.x, x1.y)) + g1.x * flare_tap(vec2<f32>(x1.x, x1.y)));
}

// One bilinear tap of the flare's source.
fn flare_tap(at: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(t_source, s_source, at, 0.0).rgb;
}

// How much a ghost sampled at `p` counts: all of it in the middle of the frame, nothing at its edges — no
// ghost of a light outside the frame, and none cut off where the frame ends.
fn flare_window(p: vec2<f32>) -> f32 {
    let d = max(abs(p.x - 0.5), abs(p.y - 0.5)) * 2.0;
    return 1.0 - smoothstep(0.6, 1.0, d);
}

@fragment
fn fs_lens_flare(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    var sum = vec3<f32>(0.0);
    for (var i = 0; i < 8; i++) {
        let g = GHOSTS[i];
        let p = (in.uv - 0.5) * g.w + 0.5;
        sum += flare_bright(p) * g.xyz * flare_window(p);
    }
    // The halo: a ring `flare.halo` of the frame across, round the middle, on the side away from the light,
    // its colours spread apart along the ring's radius (`flare.chromatic`).
    let mirror = vec2<f32>(1.0) - in.uv;
    let towards = vec2<f32>(0.5) - mirror;
    let length_towards = length(towards);
    if (length_towards > 1e-4) {
        let direction = towards / length_towards;
        let at = mirror + direction * flare.halo;
        let w = pow(1.0 - min(length(vec2<f32>(0.5) - at) / 0.7071, 1.0), 5.0);
        let spread = direction * flare.chromatic;
        sum += vec3<f32>(flare_bright(at - spread).r, flare_bright(at).g, flare_bright(at + spread).b) * w;
    }
    return vec4<f32>(sum * flare.intensity, 1.0);
}

// ============================
// Pass 3: Composite + Tone Mapping
// ============================
@group(1) @binding(0) var t_bloom: texture_2d<f32>;
@group(1) @binding(2) var t_depth: texture_depth_2d;
// The lens flare (`fs_lens_flare`), its strength already in it: black while there is none.
@group(1) @binding(4) var t_flare: texture_2d<f32>;

@fragment
fn fs_composite(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    // 1. Chromatic Aberration
    let center_dist = distance(in.uv, vec2<f32>(0.5, 0.5));
    let ca_offset = params.chromatic_aberration * center_dist * 0.05;
    
    let r = textureSample(t_source, s_source, in.uv + vec2<f32>(ca_offset, 0.0)).r;
    let g = textureSample(t_source, s_source, in.uv).g;
    let b = textureSample(t_source, s_source, in.uv - vec2<f32>(ca_offset, 0.0)).b;
    let hdr_color_raw = vec3<f32>(r, g, b);

    // 2. Depth of Field (DoF)
    let depth_dims = textureDimensions(t_depth, 0);
    let depth_uv = vec2<i32>(i32(in.uv.x * f32(depth_dims.x)), i32(in.uv.y * f32(depth_dims.y)));
    let depth_val = textureLoad(t_depth, depth_uv, 0);
    // Linearize depth. wgpu/glam perspective_rh writes NDC depth in [0,1] (NOT the
    // OpenGL [-1,1] range), so the [0,1] reconstruction must be used:
    //   view_dist = n*f / (f - d*(f - n))   →  d=0 → n, d=1 → f.
    // (The old (2n)/(f+n-d(f-n)) was the OpenGL [-1,1] formula and was wrong here.)
    let n = params.cam_near;
    let f = params.cam_far;
    let view_dist = (n * f) / (f - depth_val * (f - n));
    let hdr_color = camera_motion_blur(in.uv, hdr_color_raw, depth_val);
    
    let dof = dof_radius_at(view_dist);
    var dof_color = hdr_color;
    if (params.dof_blur_size > 0.0) {
        let texel = 1.0 / vec2<f32>(textureDimensions(t_source));
        let last_depth = vec2<i32>(depth_dims) - vec2<i32>(1);
        let tap_scale = vec2<f32>(depth_dims);

        // Something blurred in front of this pixel spreads over it, however sharp this pixel is, so a
        // sharp pixel has to go looking. How far it gathers is the most any such neighbour reaches.
        var gather = dof.x;
        for (var i = 0; i < DOF_SEARCH_TAPS; i++) {
            let t = (f32(i) + 0.5) / f32(DOF_SEARCH_TAPS);
            let angle = f32(i) * 2.399963;
            let reach = sqrt(t) * params.dof_blur_size;
            let uv = clamp(in.uv + vec2<f32>(cos(angle), sin(angle)) * reach * texel, vec2<f32>(0.0), vec2<f32>(1.0));
            let tap_dist = (n * f) / (f - textureLoad(t_depth, min(vec2<i32>(uv * tap_scale), last_depth), 0) * (f - n));
            if (tap_dist < view_dist) {
                let tap_radius = dof_radius_at(tap_dist).x;
                if (tap_radius >= reach) {
                    gather = max(gather, tap_radius);
                }
            }
        }

        if (gather > 0.01 * params.dof_blur_size) {
            // A disc of taps on the golden angle, so a wide blur does not show as eight copies of the
            // picture. In texels of this frame: these were fractions of a 1920x1080 one, so the blur
            // was narrower the smaller the frame, and wider across than tall on any other shape.
            // The blur's shape (`DofBlur`): a bokeh is the aperture — each tap as heavy as the next, out
            // to a disc or to a polygon of `blades` sides — and a gaussian weighs the near taps most.
            let gaussian = params.dof_shape.x > 0.5;
            let blades = params.dof_shape.y;
            var blurred = vec3<f32>(0.0);
            var kept = 0.0;
            for (var i = 0; i < DOF_TAPS; i++) {
                let t = (f32(i) + 0.5) / f32(DOF_TAPS);
                let angle = f32(i) * 2.399963;
                var reach = sqrt(t) * gather;
                if (!gaussian && blades >= 3.0) {
                    // The polygon's radius toward this tap, its corners on the disc: cos(π/n) over the
                    // cosine of the angle from the middle of the nearest side.
                    let side = 6.2831853 / blades;
                    let a = angle - params.dof_shape.z;
                    let from_side = a - side * floor(a / side) - side * 0.5;
                    reach *= cos(side * 0.5) / cos(from_side);
                }
                let weight = select(1.0, exp(-4.5 * t), gaussian);
                let uv = clamp(in.uv + vec2<f32>(cos(angle), sin(angle)) * reach * texel, vec2<f32>(0.0), vec2<f32>(1.0));
                let tap_dist = (n * f) / (f - textureLoad(t_depth, min(vec2<i32>(uv * tap_scale), last_depth), 0) * (f - n));
                let tap_reaches = dof_radius_at(tap_dist).x >= reach;
                if (tap_dist < view_dist) {
                    // Something sharper in front stays out unless its own blur reaches this far: a
                    // sharp object does not glow onto the blurred background behind it.
                    if (!tap_reaches) {
                        continue;
                    }
                    blurred += textureSampleLevel(t_source, s_source, uv, 0.0).rgb * weight;
                } else if (reach <= dof.x) {
                    blurred += textureSampleLevel(t_source, s_source, uv, 0.0).rgb * weight;
                } else {
                    // Further than this pixel's own blur, and only here for the sake of what is in
                    // front: what is behind stays as sharp as this pixel is.
                    blurred += hdr_color * weight;
                }
                kept += weight;
            }
            if (kept > 0.0) {
                let amount = max(dof.y, gather / params.dof_blur_size);
                dof_color = mix(hdr_color, blurred / kept, min(amount * 4.0, 1.0));
            }
        }
    }

    // 3. Bloom Addition
    let bloom_color = textureSample(t_bloom, s_bloom, in.uv).rgb;
    // Added, the default, a bloom can only brighten. Conserving, the glow comes out of the light that
    // made it: what spreads is taken from where it was, and the total stays what it was.
    var with_bloom = dof_color + bloom_color * params.bloom_intensity;
    if (params.bloom_and_lut.x > 0.5) {
        with_bloom = dof_color + (bloom_color - bright_part(dof_color)) * min(params.bloom_intensity, 1.0);
    }
    with_bloom += textureSample(t_flare, s_bloom, in.uv).rgb;
    // In the order it always was, so that a frame without local exposure is bit-identical.
    let combined = local_exposure(with_bloom, in.uv, params.exposure * auto_exposure[0]) * params.exposure * auto_exposure[0];
    
    // 3. Tone mapping — ACES by default, `TonemapCurve` chooses.
    let mapped = display_mapping(
        combined, params.bloom_and_lut.y, params.bloom_and_lut.z, params.bloom_and_lut.w, params.tonemap.x,
        params.tonemap.y,
    );
    
    // Swapchain is configured as an sRGB format (Bgra8UnormSrgb/Rgba8UnormSrgb)
    // Hardware automatically applies gamma correction upon writing.
    var final_color = mapped;

    // ── Su-altı atmosferi: kamera bir su hacmindeyken derinlik-bazlı sis (Beer-Lambert) ──
    // view_dist (lineer sahne derinliği) kullanılır; uzak geometri sis rengine gömülür → mavi-yeşil
    // su hissi. underwater=0 iken tamamen atlanır (etkisiz).
    if (params.underwater > 0.5) {
        let fog_amount = clamp(1.0 - exp(-view_dist * params.fog.a), 0.0, 1.0);
        final_color = mix(final_color, params.fog.rgb, fog_amount);
    }
    
    // 5. Vignette
    final_color *= vignette(in.uv, params.vignette_intensity);
    
    // 6. Film Grain — piksel-koordinatı tabanlı düzgün hash (statik diagonal bant yok).
    let noise = hash12(in.position.xy) - 0.5;
    final_color += final_color * noise * params.film_grain_intensity;

    // 7. Deband dither (`Renderer::deband_dither`): ±half an 8-bit step, added where the steps are
    // — in the display encoding, which the target applies after this — so a dark gradient whose
    // neighbouring pixels would round to the same code no longer shows as flat bands.
    if (params.tonemap.z > 0.5) {
        final_color = deband(final_color, in.position.xy);
    }
    
    return vec4<f32>(final_color, 1.0);
}
