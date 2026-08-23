// Average scene luminance, reduced on the GPU.
//
// Two dispatches over one buffer. Pass 0 has each workgroup sample a tile of the HDR target and
// write one partial sum; pass 1 has a single workgroup sum those partials into slot 0. Two passes
// rather than atomics because f32 atomics are not in core WebGPU, and rather than one pass because
// a single workgroup sampling the whole frame serialises it.
//
// The alternative this replaces is a CPU readback: correct, and measured at 10.1–10.9 ms a sample,
// all of it `map_async` + `poll(Wait)` stalling the GPU for one number.

struct ReduceParams {
    // Source size in texels.
    width: u32,
    height: u32,
    // How many partial sums pass 0 produced. Only pass 1 reads it.
    partial_count: u32,
    // 0 = tile pass, 1 = final sum. One shader, one pipeline layout, two dispatches.
    stage: u32,
}

@group(0) @binding(0) var<uniform> params: ReduceParams;
@group(0) @binding(1) var hdr: texture_2d<f32>;
// Slot 0 is the answer; slots 1.. are the partials. Kept in one buffer so the readback (when a
// game wants the number on the CPU) is 4 bytes from a known offset.
@group(0) @binding(2) var<storage, read_write> sums: array<f32>;

const WG: u32 = 64u;
var<workgroup> scratch: array<f32, 64>;

// Rec. 709 luma, the same weights the demos measure with.
fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

@compute @workgroup_size(64)
fn main(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(local_invocation_id) lid: vec3<u32>,
    @builtin(workgroup_id) wid: vec3<u32>,
) {
    if (params.stage == 0u) {
        // ── Pass 0: one workgroup per tile of the frame ──────────────────────
        //
        // Each invocation strides through its tile, so a tile larger than the workgroup is handled
        // without a second dispatch level.
        let total = params.width * params.height;
        let per_group = (total + params.partial_count - 1u) / params.partial_count;
        let start = wid.x * per_group;
        let end = min(start + per_group, total);

        var sum = 0.0;
        var count = 0.0;
        var i = start + lid.x;
        loop {
            if (i >= end) { break; }
            let x = i % params.width;
            let y = i / params.width;
            let c = textureLoad(hdr, vec2<u32>(x, y), 0).rgb;
            let l = luma(c);
            // A blown highlight must not take the mean with it. `textureLoad` on Rgba16Float can
            // hand back inf, and one inf makes every downstream sum inf.
            if (l == l && l < 1e6) {
                sum += l;
                count += 1.0;
            }
            i += WG;
        }

        scratch[lid.x] = sum;
        workgroupBarrier();

        // Tree reduction inside the workgroup.
        var stride = WG / 2u;
        loop {
            if (stride == 0u) { break; }
            if (lid.x < stride) {
                scratch[lid.x] += scratch[lid.x + stride];
            }
            workgroupBarrier();
            stride = stride / 2u;
        }

        if (lid.x == 0u) {
            // +1: slot 0 is reserved for the answer.
            sums[wid.x + 1u] = scratch[0];
        }
    } else {
        // ── Pass 1: one workgroup sums the partials ──────────────────────────
        var sum = 0.0;
        var i = lid.x;
        loop {
            if (i >= params.partial_count) { break; }
            sum += sums[i + 1u];
            i += WG;
        }
        scratch[lid.x] = sum;
        workgroupBarrier();

        var stride = WG / 2u;
        loop {
            if (stride == 0u) { break; }
            if (lid.x < stride) {
                scratch[lid.x] += scratch[lid.x + stride];
            }
            workgroupBarrier();
            stride = stride / 2u;
        }

        if (lid.x == 0u) {
            let total = f32(params.width * params.height);
            sums[0] = scratch[0] / max(total, 1.0);
        }
    }
}
