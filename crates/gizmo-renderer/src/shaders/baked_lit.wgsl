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
#import gizmo::common::{SceneUniforms, height_fog, instance_keeps}
#ifdef SHADOWS
#import gizmo::common::{receiver_axes, sun_shadow_visibility, shadow_distance_fade}
#endif

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
    // `InstanceRaw::optics`: xyz = 1 − specular tint, w = 0xFFFFFF − a glass's volume (8 bits a channel).
    optics: vec4<f32>,
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
    @location(6) inst_emissive: vec4<f32>,  // w < 0: receives no shadows
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
    out.inst_emissive = inst.emissive;
    out.inst_cutoff = inst.ambient.w;
    // Only for the shadow's normal offset — nothing here shades with it.
    out.world_normal = normalize((model * vec4<f32>(input.normal, 0.0)).xyz);
    return out;
}

// The sun's shadow, on the paths that draw shadow maps. A path that does not (the forward-only one: a
// browser's, a phone's) compiles none of it — not even a placeholder: it built a zero `mat2x3` here and
// passed it to a `sun_visibility` that returned 1 without reading it, and the Adreno 618's Vulkan
// compiler dereferenced a null pointer building this pipeline (2026-11-12, every game on that phone
// failed to start).
#ifdef SHADOWS
fn select_cascade(view_depth: f32) -> u32 {
    if (view_depth < scene.cascade_splits.x) { return 0u; }
    if (view_depth < scene.cascade_splits.y) { return 1u; }
    if (view_depth < scene.cascade_splits.z) { return 2u; }
    return 3u;
}

// How much sun reaches this fragment: 1.0 lit, 0.0 fully shadowed — `sun_shadow_visibility` in
// `common.wgsl`, as every lit path takes it: softened by the sun's size, pushed off the surface by both
// `ShadowSettings` multipliers, faded out over the end of the covered range. `axes` is the surface's
// own plane (`receiver_axes`), which a soft shadow filters along.
//
// This path had a lookup of its own until 2026-11-11: a 3×3 filter whatever the sun's size, so a soft
// sun cast a sharp shadow on every baked-lit surface, and a flat 16 cm depth bias — what an older NDC
// constant had been worth, restored rather than retuned when the cascades' depth range grew. A sun
// with no size still costs the nine taps this path was written to keep, on every pixel of a world that
// fills the screen; a soft one costs here what it costs everywhere.
fn sun_visibility(world_pos: vec3<f32>, world_normal: vec3<f32>, axes: mat2x3<f32>, view_depth: f32) -> f32 {
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
    let vis = sun_shadow_visibility(
        t_shadow, s_shadow, scene.light_view_proj[ci], i32(ci), scene.cascade_params.y, world_pos, world_normal,
        axes, normalize(-scene.sun_direction.xyz), scene.shadow_bias.xy, scene.camera_forward.w,
    );
    // Walk the sampled term back to "lit" over the last stretch of the covered range, so the
    // end of the shadow maps is a gradient rather than a line.
    return mix(1.0, vis, fade);
}
#endif

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // The surface's normal and how it runs across neighbouring pixels, for the sun's shadow
    // (`receiver_axes`) — taken first, before any pixel of the quad discards.
#ifdef SHADOWS
    let n = normalize(in.world_normal);
    let surface_axes = receiver_axes(dpdx(in.world_pos), dpdy(in.world_pos), n);
#endif
    let tex = textureSample(t_diffuse, s_diffuse, in.tex_coords);

    // **Cut-outs.** Foliage on a quad, a chain-link fence, a pierced railing: opaque geometry
    // with holes in it. Discarding the holes keeps the draw in the opaque pass — depth written,
    // order irrelevant — where blending it instead would lose anything coplanar underneath it,
    // because the sorted pass cannot hold a decal against its own surface. Same threshold and
    // same test as `gbuffer.wgsl`; zero disables it, which is every material that does not ask.
    let final_alpha = in.color.a * in.inst_albedo.a * tex.a;
    if (!instance_keeps(in.inst_cutoff, final_alpha, in.clip_position.xy)) {
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
#ifdef SHADOWS
    let view_depth = dot(in.world_pos - scene.camera_pos.xyz, scene.camera_forward.xyz);
    // `MeshRenderer::receives_shadows == false` keeps the sun's share whole.
    let vis = select(sun_visibility(in.world_pos, n, surface_axes, view_depth), 1.0, in.inst_emissive.w < 0.0);
#else
    let vis = 1.0;
#endif

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
    let colour = (lit + in.inst_ambient) * base + in.inst_emissive.xyz;

    // Vertex alpha multiplies through with the instance's and the texture's. It only reaches the
    // framebuffer on the blended (transparent) variant of this pipeline — the opaque variant
    // writes it to the HDR target's unread alpha channel, exactly as it did before.
    let fog = height_fog(in.world_pos, scene.camera_pos.xyz, scene.fog, scene.fog_shape);
    return vec4<f32>(mix(colour, fog.rgb, fog.a), final_alpha);
}
