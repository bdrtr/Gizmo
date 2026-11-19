// Local exposure: the frame's luminance as a bilateral grid — what the composite reads each pixel's
// neighbourhood average from, among the pixels as bright as it (`crate::local_exposure`).
//
// The grid is the screen in cells of `CELL`² pixels, by `params.grid.z` bins of log2 luminance from
// `params.range.x` to `params.range.y`. A texel holds `(w · m, w)`: `w` the share of its cell's pixels
// whose luminance falls in its bin, `m` their mean log2 luminance. Kept premultiplied so that a blur — and
// the composite's trilinear lookup — weighs each bin by how many pixels are in it, and an empty bin by
// nothing: `m = (w·m) / w` after either.
//
// `build`: one workgroup a cell, its pixels sorted into the bins with workgroup atomics (integers: the
// log2 in 1/256ths, which a cell of 1024 pixels at ±20 stops keeps well inside an i32).
// `blur`: a Gaussian along one axis of the grid, `params.axis` saying which; run three times.

struct LocalParams {
    // x, y: the grid's cells across and down; z: its bins; w: a cell's side in pixels.
    grid: vec4<u32>,
    // x, y: the log2 luminance the bins span; z: the blur's sigma along x and y, in cells; w: along the
    // bins, in bins.
    range: vec4<f32>,
    // The composite's: x, y highlight and shadow contrast; z log2 of the pivot; w the detail kept.
    shape: vec4<f32>,
    // x: on (1) or off; y, z: the pixel size over the grid's (`uv` → grid); w: unused.
    flags: vec4<f32>,
}

struct BlurAxis {
    // The axis this pass blurs along: (1,0,0), (0,1,0) or (0,0,1).
    axis: vec4<i32>,
}

@group(0) @binding(0) var<uniform> params: LocalParams;

// ── build ────────────────────────────────────────────────────────────────────────────────────────

const MAX_BINS: u32 = 32u;
var<workgroup> bin_sums: array<atomic<i32>, 32>;
var<workgroup> bin_counts: array<atomic<u32>, 32>;

@group(1) @binding(0) var hdr: texture_2d<f32>;
@group(1) @binding(1) var grid_out: texture_storage_3d<rgba16float, write>;

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// The log2 luminance of `c`, held to the grid's span (a NaN to its bottom).
fn log_luminance(c: vec3<f32>) -> f32 {
    let l = luma(c);
    return clamp(select(params.range.x, log2(max(l, 1e-30)), l == l && l > 0.0), params.range.x, params.range.y);
}

fn bin_of(l2: f32) -> u32 {
    let t = (l2 - params.range.x) / (params.range.y - params.range.x);
    return u32(clamp(floor(t * f32(params.grid.z)), 0.0, f32(params.grid.z - 1u)));
}

@compute @workgroup_size(16, 16)
fn build(
    @builtin(workgroup_id) cell: vec3<u32>,
    @builtin(local_invocation_id) lid: vec3<u32>,
    @builtin(local_invocation_index) li: u32,
) {
    if (li < MAX_BINS) {
        atomicStore(&bin_sums[li], 0);
        atomicStore(&bin_counts[li], 0u);
    }
    workgroupBarrier();
    let size = vec2<u32>(textureDimensions(hdr));
    let side = params.grid.w;
    // Each invocation takes a square of the cell's pixels: a cell of 32² over 16² invocations, 2² each.
    let each = max(side / 16u, 1u);
    let origin = cell.xy * side + lid.xy * each;
    for (var dy = 0u; dy < each; dy++) {
        for (var dx = 0u; dx < each; dx++) {
            let p = origin + vec2<u32>(dx, dy);
            if (p.x < size.x && p.y < size.y) {
                let l2 = log_luminance(textureLoad(hdr, p, 0).rgb);
                let bin = bin_of(l2);
                atomicAdd(&bin_sums[bin], i32(round(l2 * 256.0)));
                atomicAdd(&bin_counts[bin], 1u);
            }
        }
    }
    workgroupBarrier();
    if (li < params.grid.z) {
        let n = atomicLoad(&bin_counts[li]);
        let w = f32(n) / f32(side * side);
        let m = select(0.0, f32(atomicLoad(&bin_sums[li])) / 256.0 / f32(max(n, 1u)), n > 0u);
        textureStore(grid_out, vec3<u32>(cell.x, cell.y, li), vec4<f32>(w * m, w, 0.0, 0.0));
    }
}

// ── blur ─────────────────────────────────────────────────────────────────────────────────────────

// Bindings of their own (2–4): an entry point's layout holds only what it reads.
@group(1) @binding(2) var grid_in: texture_3d<f32>;
@group(1) @binding(3) var blurred: texture_storage_3d<rgba16float, write>;
@group(1) @binding(4) var<uniform> blur: BlurAxis;

@compute @workgroup_size(4, 4, 4)
fn blur_axis(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = vec3<i32>(textureDimensions(grid_in));
    let at = vec3<i32>(id);
    if (any(at >= dims)) {
        return;
    }
    let along_bins = blur.axis.z != 0;
    let sigma = select(params.range.z, params.range.w, along_bins);
    let reach = i32(ceil(2.5 * sigma));
    var sum = vec2<f32>(0.0);
    var weights = 0.0;
    for (var k = -reach; k <= reach; k++) {
        let q = at + blur.axis.xyz * k;
        if (any(q < vec3<i32>(0)) || any(q >= dims)) {
            continue;
        }
        let g = exp(-0.5 * f32(k * k) / max(sigma * sigma, 1e-4));
        sum += textureLoad(grid_in, q, 0).xy * g;
        weights += g;
    }
    // Normalised over the taps inside the grid: the edge of the screen is not a darker neighbourhood.
    textureStore(blurred, vec3<u32>(id), vec4<f32>(sum / max(weights, 1e-6), 0.0, 0.0));
}
