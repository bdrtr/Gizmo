// One quad per sprite, from an instance buffer. No vertex buffer and no scene uniforms: the CPU
// has already put the four corners in clip space (with the UNJITTERED view-projection, so a
// sprite drawn after TAA does not shimmer by the jitter), and this shader only picks one.
//
// The corners are in image order — top-left, top-right, bottom-left, bottom-right — so the corner
// index alone says where in the image a vertex samples; flipping is already folded into `uv`.

@group(0) @binding(0)
var image: texture_2d<f32>;
@group(0) @binding(1)
var image_sampler: sampler;

struct SpriteInstance {
    @location(0) top_left: vec4<f32>,
    @location(1) top_right: vec4<f32>,
    @location(2) bottom_left: vec4<f32>,
    @location(3) bottom_right: vec4<f32>,
    // (u0, v0, u1, v1): u0/v0 at the top-left corner, u1/v1 at the bottom-right.
    @location(4) uv: vec4<f32>,
    @location(5) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, inst: SpriteInstance) -> VertexOutput {
    // Two triangles over corners 0 1 2 / 2 1 3. Culling is off, so the winding is documentation;
    // a mirrored sprite (negative scale) is still a sprite.
    var order = array<u32, 6>(0u, 1u, 2u, 2u, 1u, 3u);
    let k = order[vi];
    var corners = array<vec4<f32>, 4>(inst.top_left, inst.top_right, inst.bottom_left, inst.bottom_right);
    let c = vec2<f32>(f32(k & 1u), f32(k >> 1u));

    var out: VertexOutput;
    out.clip = corners[k];
    out.uv = mix(inst.uv.xy, inst.uv.zw, c);
    out.color = inst.color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(image, image_sampler, in.uv) * in.color;
}
