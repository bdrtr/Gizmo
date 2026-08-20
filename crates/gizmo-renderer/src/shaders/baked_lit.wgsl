// Baked lighting, plus the sun's shadow.
//
// For geometry that already carries its lighting in the vertex colour — a static level, a
// lightmapped world, anything authored lit — and needs only one thing the file cannot know: where
// the dynamic objects in front of it are casting. So this is `unlit.wgsl` (albedo × texture ×
// vertex colour) with the directional cascade term multiplied in, plus the two knobs a scene that
// was authored dark cannot do without: an ambient floor and an emissive term. Still no G-buffer,
// no point lights, no PBR: one forward draw per batch instead of the eleven a lit one costs.
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
    // xyz = ambient floor, **w = the cut-out threshold**; xyz = emissive. Mirrors
    // `gpu_types::InstanceRaw`; every shader that indexes this buffer must declare all eight
    // slots or the stride is wrong.
    ambient: vec4<f32>,
    emissive: vec4<f32>,
};

@group(#{INSTANCE_GROUP}) @binding(0)
var<storage, read> instances: array<InstanceRaw>;

struct VertexInput {
    @location(0) position: vec3<f32>,
    // RGBA. The alpha is authored data — a decal's soft edge lives here — not padding.
    @location(1) color: vec4<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) tex_coords: vec2<f32>,
    @location(4) joint_indices: vec4<u32>,
    @location(5) joint_weights: vec4<f32>,
    @location(6) tangent: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) tex_coords: vec2<f32>,
    @location(2) inst_albedo: vec4<f32>,
    @location(3) world_pos: vec3<f32>,
    @location(4) world_normal: vec3<f32>,
    @location(5) inst_ambient: vec3<f32>,
    @location(6) inst_emissive: vec3<f32>,
    @location(7) inst_cutoff: f32,
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
    out.inst_ambient = inst.ambient.rgb;
    out.inst_emissive = inst.emissive.rgb;
    out.inst_cutoff = inst.ambient.w;
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

// Mirror of `csm::SHADOW_FADE_FRACTION` / `csm::shadow_distance_fade` — see the Rust side for
// why the band exists and what the alternative costs. `csm.rs`'s
// `shader_shadow_fade_matches_the_rust_mirror` fails if this constant drifts.
const SHADOW_FADE_FRACTION: f32 = 0.15;

fn shadow_distance_fade(view_depth: f32, shadow_far: f32) -> f32 {
    let far = max(shadow_far, 1e-4);
    let band = max(far * SHADOW_FADE_FRACTION, 1e-4);
    return 1.0 - smoothstep(far - band, far, view_depth);
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
    // cascade_splits.w is the far distance of the last cascade — the whole range the shadow
    // maps cover (min(camera far, csm::SHADOW_DISTANCE)).
    let fade = shadow_distance_fade(view_depth, scene.cascade_splits.w);
    if (fade <= 0.0) {
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
    // Outside the cascade there is nothing to say, so say lit rather than guessing. Inside the
    // covered range this is now only reachable for a fragment that falls laterally out of the
    // cascade box; the DISTANCE case — which used to make crossing the range boundary a 1.82x
    // brightness step across the whole width of the world — is handled by the fade below.
    if (shadow_uv.x < 0.0 || shadow_uv.x > 1.0 || shadow_uv.y < 0.0 || shadow_uv.y > 1.0 || ndc.z > 1.0) {
        return 1.0;
    }

    let texel = scene.cascade_params.y;
    // **Metres, converted with the cascade's own z gradient** — the same treatment the deferred and
    // forward paths got, and this one needed it more than either. It was a flat `0.0015` in NDC,
    // which is a different world distance in every cascade and moves whenever the projection's
    // depth range does. Widening `CASTER_REACH` from 60 m to 500 m (so tall buildings would stop
    // losing their shadows at noon) quadrupled the range, and with it this bias: **0.16 m of
    // peter-panning became 0.82 m**, in the material path the city itself is drawn with. The third
    // row of the cascade matrix is NDC-z per world metre; 0.16 is what the old constant was worth
    // at the range it was chosen under, so this restores that behaviour rather than retuning it.
    let sz = length(vec3<f32>(m[0][2], m[1][2], m[2][2]));
    let bias = 0.16 * sz;
    var sum = 0.0;
    for (var x = -1; x <= 1; x++) {
        for (var y = -1; y <= 1; y++) {
            let o = vec2<f32>(f32(x), f32(y)) * texel;
            sum += textureSampleCompareLevel(t_shadow, s_shadow, shadow_uv + o, ci, ndc.z - bias);
        }
    }
    // Walk the sampled term back to "lit" over the last stretch of the covered range, so the
    // end of the shadow maps is a gradient rather than a line.
    return mix(1.0, sum / 9.0, fade);
#else
    return 1.0;
#endif
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let tex = textureSample(t_diffuse, s_diffuse, in.tex_coords);

    // **Cut-outs.** Foliage on a quad, a chain-link fence, a pierced railing: opaque geometry
    // with holes in it. Discarding the holes keeps the draw in the opaque pass — depth written,
    // order irrelevant — where blending it instead would lose anything coplanar underneath it,
    // because the sorted pass cannot hold a decal against its own surface. Same threshold and
    // same test as `gbuffer.wgsl`; zero disables it, which is every material that does not ask.
    let final_alpha = in.color.a * in.inst_albedo.a * tex.a;
    if (in.inst_cutoff > 0.0 && final_alpha < in.inst_cutoff) {
        discard;
    }

    let base = in.inst_albedo.rgb * tex.rgb;

    // The vertex colour is taken at face value. It used to be second-guessed here — a
    // near-zero length was rewritten to white, so that an importer which never set the
    // attribute could not black out the model. But whether the attribute EXISTS is a property
    // of the vertex layout, not of a pixel value, and this layout always has it: a source with
    // no colours is normalised to opaque white at import (`gpu_types::Vertex::default`). Under
    // the old test a surface the author painted black came out white instead.
    let baked = in.color.rgb;

    // Distance along the camera's forward axis — the same measure `cascade_splits` is expressed
    // in, and what `select_cascade` and the fade both assume. (Radial `length(...)` overstates
    // the depth of anything off-axis, so it picked a cascade too far out at the screen edges
    // and put the range boundary on a sphere instead of a plane.)
    let view_depth = dot(in.world_pos - scene.camera_pos.xyz, scene.camera_forward.xyz);
    let vis = sun_visibility(in.world_pos, in.world_normal, view_depth);

    // The shadow darkens rather than replaces. Baked lighting already includes the sky and the
    // bounce; a shadowed patch of it is dimmer, not black, and multiplying the whole thing by the
    // visibility would make every shadow a hole. 0.45 is how much of the surface's own light the
    // sun is taken to be responsible for.
    let sun_share = 0.45;
    let lit = baked * (1.0 - sun_share + sun_share * vis);

    // `ambient` is incident light: it joins the baked term BEFORE the surface's own colour, so a
    // lifted scene still shows its albedo instead of flooding to grey. `emissive` is the surface
    // emitting, so it is added after and owes nothing to albedo. Both default to zero, which
    // leaves this expression exactly `lit * base`.
    let colour = (lit + in.inst_ambient) * base + in.inst_emissive;

    // Vertex alpha multiplies through with the instance's and the texture's. It only reaches the
    // framebuffer on the blended (transparent) variant of this pipeline — the opaque variant
    // writes it to the HDR target's unread alpha channel, exactly as it did before.
    return vec4<f32>(colour, final_alpha);
}
