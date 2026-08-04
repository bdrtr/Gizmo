// Baked lighting, plus the sun's shadow.
//
// For geometry that already carries its lighting in the vertex colour — a static level, a lightmapped
// world, anything authored lit — and needs only one thing the file cannot know: where the dynamic
// objects in front of it are casting. So this is `unlit.wgsl` (albedo × texture × vertex colour) with
// the directional cascade term multiplied in, and nothing else. No G-buffer, no point lights, no
// PBR: one forward draw per batch instead of the eleven a lit one costs.
//
// It samples the shadow group the forward pass already binds at slot 2, so it needs no plumbing of
// its own. On the web schema that group does not exist (the 4-bind-group limit), and there the
// shadow term is 1.0 — the same reduction the rest of the web pipeline already makes.
#import gizmo::common::{SceneUniforms}

@group(0) @binding(0)
var<uniform> scene: SceneUniforms;

@group(1) @binding(0)
var t_diffuse: texture_2d<f32>;
@group(1) @binding(1)
var s_diffuse: sampler;

#ifdef SHADOWS
@group(2) @binding(0) var t_shadow: texture_depth_2d_array;
@group(2) @binding(1) var s_shadow: sampler_comparison;
#endif

struct SkeletonData {
    joints: array<mat4x4<f32>, 128>,
};
@group(#{SKELETON_GROUP}) @binding(0)
var<uniform> skeleton: SkeletonData;

struct InstanceRaw {
    model_matrix_0: vec4<f32>,
    model_matrix_1: vec4<f32>,
    model_matrix_2: vec4<f32>,
    model_matrix_3: vec4<f32>,
    albedo_color: vec4<f32>,
    pbr: vec4<f32>,
};

@group(#{INSTANCE_GROUP}) @binding(0)
var<storage, read> instances: array<InstanceRaw>;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) tex_coords: vec2<f32>,
    @location(4) joint_indices: vec4<u32>,
    @location(5) joint_weights: vec4<f32>,
    @location(6) tangent: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) tex_coords: vec2<f32>,
    @location(2) inst_albedo: vec4<f32>,
    @location(3) world_pos: vec3<f32>,
    @location(4) world_normal: vec3<f32>,
};

@vertex
fn vs_main(input: VertexInput, @builtin(instance_index) i: u32) -> VertexOutput {
    let inst = instances[i];
    let model = mat4x4<f32>(
        inst.model_matrix_0,
        inst.model_matrix_1,
        inst.model_matrix_2,
        inst.model_matrix_3,
    );

    var out: VertexOutput;
    let world = model * vec4<f32>(input.position, 1.0);
    out.world_pos = world.xyz;
    out.clip_position = scene.view_proj * world;
    out.color = input.color;
    out.tex_coords = input.tex_coords;
    out.inst_albedo = inst.albedo_color;
    // Only for the shadow's normal offset — nothing here shades with it.
    out.world_normal = normalize((model * vec4<f32>(input.normal, 0.0)).xyz);
    return out;
}

fn select_cascade(view_depth: f32) -> u32 {
    if (view_depth < scene.cascade_splits.x) { return 0u; }
    if (view_depth < scene.cascade_splits.y) { return 1u; }
    if (view_depth < scene.cascade_splits.z) { return 2u; }
    return 3u;
}

// How much sun reaches this fragment: 1.0 lit, 0.0 fully shadowed.
//
// A 3x3 PCF rather than the deferred path's 5x5. This runs on geometry that is already lit, so the
// term is a multiply on top of a correct image rather than the whole of it, and the difference
// between nine taps and twenty-five is not visible on a baked surface — while the cost is paid on
// every pixel of a world that fills the screen.
fn sun_visibility(world_pos: vec3<f32>, world_normal: vec3<f32>, view_depth: f32) -> f32 {
#ifdef SHADOWS
    if (scene.sun_direction.w < 0.5) {
        return 1.0;
    }
    let ci = select_cascade(view_depth);
    let m = scene.light_view_proj[ci];

    // Normal offset, scaled to the cascade's own world texel size: a fixed offset only suits the
    // nearest cascade, where the texel is smallest.
    let sx = length(vec3<f32>(m[0][0], m[1][0], m[2][0]));
    let world_texel = 2.0 * scene.cascade_params.y / max(sx, 1e-6);
    let offset_pos = world_pos + world_normal * world_texel * 2.0;

    let light_clip = m * vec4<f32>(offset_pos, 1.0);
    let ndc = light_clip.xyz / max(light_clip.w, 1e-6);
    let shadow_uv = ndc.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5, 0.5);
    // Outside the cascade there is nothing to say, so say lit rather than guessing.
    if (shadow_uv.x < 0.0 || shadow_uv.x > 1.0 || shadow_uv.y < 0.0 || shadow_uv.y > 1.0 || ndc.z > 1.0) {
        return 1.0;
    }

    let texel = scene.cascade_params.y;
    let bias = 0.0015;
    var sum = 0.0;
    for (var x = -1; x <= 1; x++) {
        for (var y = -1; y <= 1; y++) {
            let o = vec2<f32>(f32(x), f32(y)) * texel;
            sum += textureSampleCompare(t_shadow, s_shadow, shadow_uv + o, ci, ndc.z - bias);
        }
    }
    return sum / 9.0;
#else
    return 1.0;
#endif
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let tex = textureSample(t_diffuse, s_diffuse, in.tex_coords);
    let base = in.inst_albedo.rgb * tex.rgb;

    // A mesh with no vertex colour is white, not black: `Vertex::default()` is white and an
    // importer that does not set the attribute must not black out the model.
    var baked = in.color;
    if (length(baked) < 0.0001) {
        baked = vec3<f32>(1.0, 1.0, 1.0);
    }

    let view_depth = length(in.world_pos - scene.camera_pos.xyz);
    let vis = sun_visibility(in.world_pos, in.world_normal, view_depth);

    // The shadow darkens rather than replaces. Baked lighting already includes the sky and the
    // bounce; a shadowed patch of it is dimmer, not black, and multiplying the whole thing by the
    // visibility would make every shadow a hole. 0.45 is how much of the surface's own light the
    // sun is taken to be responsible for.
    let sun_share = 0.45;
    let lit = baked * (1.0 - sun_share + sun_share * vis);

    return vec4<f32>(lit * base, in.inst_albedo.a * tex.a);
}
