// Volumetric light apply pass: the march's air, at the march's resolution, added over the HDR target.
//
// A depth-aware upsample: each pixel takes the four march texels around it, weighted as bilinear
// filtering would and by how near the surface each texel's ray stopped at (its alpha, `MarchOut`)
// stands to the pixel's own. Filtered plainly, the air marched past a lamp post spilled onto the post's
// pixels, more the coarser the march (`VolumetricState::set_march_divisor`).
#import gizmo::common::{SceneUniforms}

@group(0) @binding(0) var t_vol: texture_2d<f32>;
@group(0) @binding(1) var s_linear: sampler;
@group(0) @binding(2) var t_depth: texture_depth_2d;
@group(1) @binding(0) var<uniform> scene: SceneUniforms;

// What the march reports for a ray to the sky (`SKY_DISTANCE` in `volumetric.wgsl`).
const SKY_DISTANCE: f32 = 10000.0;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VertexOutput {
    var pos = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    var uv = array<vec2<f32>, 3>(vec2(0.0, 1.0), vec2(2.0, 1.0), vec2(0.0, -1.0));
    var out: VertexOutput;
    out.position = vec4(pos[vi], 0.0, 1.0);
    out.uv = uv[vi];
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let full = vec2<f32>(textureDimensions(t_depth));
    let pixel = vec2<i32>(in.position.xy);
    let uv = (vec2<f32>(pixel) + 0.5) / full;
    // How far this pixel's surface stands, as the march measures its texels'.
    var here = SKY_DISTANCE;
    let depth = textureLoad(t_depth, pixel, 0);
    if (depth < 1.0) {
        let h = scene.inv_view_proj * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
        here = length(h.xyz / h.w - scene.camera_pos.xyz);
    }
    let low = vec2<i32>(textureDimensions(t_vol));
    let p = uv * vec2<f32>(low) - 0.5;
    let base = vec2<i32>(floor(p));
    let f = p - floor(p);
    var sum = vec3<f32>(0.0);
    var weight = 0.0;
    for (var k = 0; k < 4; k++) {
        let corner = vec2<i32>(k & 1, k >> 1);
        let texel = textureLoad(t_vol, clamp(base + corner, vec2<i32>(0), low - 1), 0);
        let bilinear = select(1.0 - f.x, f.x, corner.x == 1) * select(1.0 - f.y, f.y, corner.y == 1);
        // A texel whose surface stands a tenth of the way off from this pixel's counts a tenth as much.
        let w = bilinear / (0.01 + abs(texel.a - here) / max(here, 0.1));
        sum += texel.rgb * w;
        weight += w;
    }
    return vec4(sum / max(weight, 1e-6), 1.0);
}
