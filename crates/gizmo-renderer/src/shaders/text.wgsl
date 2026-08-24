// One quad per glyph, from an instance buffer. No vertex buffer: the six corners are a constant
// table indexed by `vertex_index`, so a glyph costs 64 bytes of instance and nothing else.
//
// The two spaces share this shader and are told apart by `origin.w`: negative means the rect is
// already in clip space (screen text, positioned on the CPU where the window size is known), and
// non-negative means the rect is a local offset in world units around `origin.xyz`, turned to face
// the camera here.
#import gizmo::common::{SceneUniforms}

@group(0) @binding(0)
var<uniform> scene: SceneUniforms;

@group(1) @binding(0)
var atlas: texture_2d<f32>;
@group(1) @binding(1)
var atlas_sampler: sampler;

struct GlyphInstance {
    // Screen: the quad in NDC. World: the quad in world units, relative to `origin`, y up.
    @location(0) rect: vec4<f32>,
    // The glyph's box in the atlas: (u0, v0, u1, v1).
    @location(1) uv: vec4<f32>,
    @location(2) color: vec4<f32>,
    // World: xyz = the anchor in world space, w >= 0. Screen: w < 0.
    @location(3) origin: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, inst: GlyphInstance) -> VertexOutput {
    // Two triangles, wound counter-clockwise. Culling is off for this pipeline, so the winding is
    // documentation rather than load-bearing — but a quad whose diagonal is wrong shows up as half
    // a glyph, which is worth not having to diagnose.
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let c = corners[vi];

    var out: VertexOutput;
    out.uv = mix(inst.uv.xy, inst.uv.zw, c);
    out.color = inst.color;
    let p = mix(inst.rect.xy, inst.rect.zw, c);

    if (inst.origin.w < 0.0) {
        out.clip = vec4<f32>(p, 0.0, 1.0);
    } else {
        // A screen-aligned basis from the camera's forward vector. The engine's camera is a
        // yaw/pitch camera and never rolls, so `cross(forward, world_up)` IS the camera's right
        // vector rather than an approximation of it — and the label stays upright, which is what a
        // reader wants even when a rolled camera would say otherwise.
        let fwd = normalize(scene.camera_forward.xyz);
        var right = cross(fwd, vec3<f32>(0.0, 1.0, 0.0));
        // Looking straight up or straight down leaves that cross product at zero. Falling back to
        // world X keeps the label facing *somewhere*; without this it collapses to a point.
        if (length(right) < 1e-4) {
            right = vec3<f32>(1.0, 0.0, 0.0);
        }
        right = normalize(right);
        let up = normalize(cross(right, fwd));
        let world = inst.origin.xyz + right * p.x + up * p.y;
        out.clip = scene.view_proj * vec4<f32>(world, 1.0);
    }
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // The atlas is single-channel coverage, not colour: it says how much of this pixel the glyph
    // covers, and the text's own colour says what to paint there.
    let coverage = textureSample(atlas, atlas_sampler, in.uv).r;
    return vec4<f32>(in.color.rgb, in.color.a * coverage);
}
