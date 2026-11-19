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
#import gizmo::common::{SceneUniforms, height_fog, instance_keeps, BAKED_SUN_SHARE}
#import gizmo::surface::{rain_on, wet_darkening}
#ifdef RAIN_MAP
#import gizmo::common::{rain_exposure}
#endif
#ifdef REFLECTION_INPUTS
#import gizmo::common::{BAKED_SURFACE, dielectric_f0}
#import gizmo::surface::{wet_roughness}
#endif
#ifdef SUN_SHADOWS
#ifdef SUN_SHADOW_PER_VERTEX
#import gizmo::common::{sun_shadow_coord, sun_shadow_lean, sun_shadow_tap, shadow_distance_fade}
#else
#import gizmo::common::{ReceiverAxes, receiver_axes, sun_shadow_visibility, shadow_distance_fade}
#endif
#endif

@group(0) @binding(0)
var<uniform> scene: SceneUniforms;

@group(1) @binding(0)
var t_diffuse: texture_2d<f32>;
@group(1) @binding(1)
var s_diffuse: sampler;
// The texture pool's array (`texture_pool`): a pooled material's base colour is one of its layers.
@group(1) @binding(8)
var t_pool: texture_2d_array<f32>;

// The base colour at `uv`: the material's own texture, or, for `layer` above zero, layer `layer - 1`
// of its texture pool's array (`InstanceRaw::texture`) — `gizmo::material::base_colour`, for a shader
// that declares group 1 itself. Both sampled, one chosen: a branch on a per-instance value would put
// the sample in non-uniform control flow, and the one not wanted is a 1×1 placeholder.
fn base_colour(uv: vec2<f32>, layer: u32) -> vec4<f32> {
    let own = textureSample(t_diffuse, s_diffuse, uv);
    let pooled = textureSample(t_pool, s_diffuse, uv, i32(max(layer, 1u) - 1u));
    return select(own, pooled, layer > 0u);
}
// The material's constants (`gpu_types::MaterialParams`; only the lightmap's strength, `uv_scale.w`,
// is read here) and its lightmap: a baked-lit surface whose light is a picture rather than its
// vertex colours.
struct BakedMaterialParams {
    emissive_and_normal_scale: vec4<f32>,
    occlusion_uv_rot_offset: vec4<f32>,
    uv_scale: vec4<f32>,
};
@group(1) @binding(6)
var<uniform> material: BakedMaterialParams;
@group(1) @binding(7)
var t_lightmap: texture_2d<f32>;

// The sun's cascades, where `shader.wgsl` reads them: the shadow group's on the desktop, group 0's on
// the forward-only path while the scene has a sun that casts.
#ifdef SUN_SHADOWS
@group(#{SUN_SHADOW_GROUP}) @binding(#{SUN_SHADOW_TEXTURE_BINDING}) var t_shadow: texture_depth_2d_array;
@group(#{SUN_SHADOW_GROUP}) @binding(#{SUN_SHADOW_SAMPLER_BINDING}) var s_shadow: sampler_comparison;
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
    // `InstanceRaw::texture`: x = 0, the material's own base colour, or one more than its layer
    // in the texture pool's array (`texture_pool`).
    texture: vec4<f32>,
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
    @location(7) lightmap_uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    // xy: the material's UV; zw: the lightmap's (`Vertex::lightmap_uv`).
    @location(1) tex_coords: vec4<f32>,
    @location(2) inst_albedo: vec4<f32>,
    @location(3) world_pos: vec3<f32>,
    @location(4) world_normal: vec3<f32>,
    @location(5) inst_ambient: vec3<f32>,
    @location(6) inst_emissive: vec4<f32>,  // w < 0: receives no shadows
    // x: the cut-out threshold (`instance_keeps`); y: the roughness, which `fs_reflective` hands the
    // screen-space reflections. One location for both, on a path that counts them (WebGL2's fifteen).
    @location(7) inst_cutoff_roughness: vec2<f32>,
    // Where the base colour is (`InstanceRaw::texture.x`, `base_colour`).
    @location(9) @interpolate(flat) base_layer: u32,
#ifdef SUN_SHADOWS
#ifdef SUN_SHADOW_PER_VERTEX
    // Where the sun's one cascade sees this point (`sun_shadow_coord`).
    @location(8) sun_shadow: vec4<f32>,
#endif
#endif
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
    out.tex_coords = vec4<f32>(input.tex_coords, input.lightmap_uv);
    out.inst_albedo = inst.albedo_color;
    out.inst_ambient = inst.ambient.rgb;
    out.inst_emissive = inst.emissive;
    out.inst_cutoff_roughness = vec2<f32>(inst.ambient.w, inst.pbr.x);
    out.base_layer = u32(inst.texture.x);
    // Only for the shadow's normal offset — nothing here shades with it.
    out.world_normal = normalize((model * vec4<f32>(input.normal, 0.0)).xyz);
#ifdef SUN_SHADOWS
#ifdef SUN_SHADOW_PER_VERTEX
    // Pushed along the normal it shades with, as the desktop's baked-lit lookup is.
    out.sun_shadow = sun_shadow_coord(scene.light_view_proj[0], world.xyz)
        + vec4<f32>(sun_shadow_lean(scene.light_view_proj[0], scene.cascade_params.y, scene.shadow_bias.y, out.world_normal), 0.0);
#endif
#endif
    return out;
}

// The sun's shadow, on the paths that draw shadow maps. A path that does not (the forward-only one: a
// browser's, a phone's) compiles none of it — not even a placeholder: it built a zero `mat2x3` here and
// passed it to a `sun_visibility` that returned 1 without reading it, and the Adreno 618's Vulkan
// compiler dereferenced a null pointer building this pipeline (2026-11-12, every game on that phone
// failed to start).
#ifdef SUN_SHADOWS
#ifndef SUN_SHADOW_PER_VERTEX
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
fn sun_visibility(world_pos: vec3<f32>, world_normal: vec3<f32>, axes: ReceiverAxes, view_depth: f32) -> f32 {
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
#endif

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return shade(in, rain_at(in.world_pos, in.world_normal));
}

// The scene's rain on a fragment (`rain_on`), kept off what is under cover where there is a rain map —
// looked up only on what faces up enough to take rain at all.
fn rain_at(world_position: vec3<f32>, normal: vec3<f32>) -> vec2<f32> {
    let up = normalize(normal).y;
    var exposure = 1.0;
#ifdef RAIN_MAP
    if (scene.wetness.x > 0.0 && up > 0.2) {
        exposure = rain_exposure(t_shadow, s_shadow, i32(#{RAIN_LAYER}), scene.rain_view_proj, scene.rain_params, world_position);
    }
#endif
    return rain_on(scene.wetness, world_position, up, exposure);
}

#ifdef REFLECTION_INPUTS
// What screen-space reflections read of a surface, written beside its colour when they are on and
// nothing else wants this surface in the G-buffer (`DeferredState::baked_reflective_pipelines`): its
// normal and roughness (RT1), where it is with the baked mark (RT2), and its reflectance (RT5's `w`) —
// as `gbuffer.wgsl`'s `fs_baked` writes them, so the reflections cannot tell the two routes apart. The
// lighting pass has been by now, so RT5's colour is nobody's, and stays black.
struct ReflectionInputs {
    @location(0) colour: vec4<f32>,
    @location(1) normal_roughness: vec4<f32>,
    @location(2) position_rel_camera: vec4<f32>,
    @location(3) reflectance: vec4<f32>,
};

@fragment
fn fs_reflective(in: VertexOutput, @builtin(front_facing) front: bool) -> ReflectionInputs {
    let rain = rain_at(in.world_pos, in.world_normal);
    var out: ReflectionInputs;
    out.colour = shade(in, rain);
    let n = normalize(select(-in.world_normal, in.world_normal, front));
    out.normal_roughness = vec4<f32>(
        normalize(mix(n, vec3<f32>(0.0, 1.0, 0.0), rain.y)),
        wet_roughness(in.inst_cutoff_roughness.y, rain),
    );
    out.position_rel_camera = vec4<f32>(in.world_pos - scene.camera_pos.xyz, BAKED_SURFACE);
    let f0 = dielectric_f0(in.inst_emissive.w);
    out.reflectance = vec4<f32>(0.0, 0.0, 0.0, select(f0, -f0, in.inst_emissive.w < 0.0));
    return out;
}
#endif

// The surface's colour, fogged, with its alpha; `rain` is `rain_on`'s.
fn shade(in: VertexOutput, rain: vec2<f32>) -> vec4<f32> {
    // The surface's normal and how it runs across neighbouring pixels, for the sun's shadow
    // (`receiver_axes`) — taken first, before any pixel of the quad discards.
#ifdef SUN_SHADOWS
    let n = normalize(in.world_normal);
#ifndef SUN_SHADOW_PER_VERTEX
    let surface_axes = receiver_axes(dpdx(in.world_pos), dpdy(in.world_pos), n);
#endif
#endif
    let tex = base_colour(in.tex_coords.xy, in.base_layer);

    // **Cut-outs.** Foliage on a quad, a chain-link fence, a pierced railing: opaque geometry
    // with holes in it. Discarding the holes keeps the draw in the opaque pass — depth written,
    // order irrelevant — where blending it instead would lose anything coplanar underneath it,
    // because the sorted pass cannot hold a decal against its own surface. Same threshold and
    // same test as `gbuffer.wgsl`; zero disables it, which is every material that does not ask.
    let final_alpha = in.color.a * in.inst_albedo.a * tex.a;
    if (!instance_keeps(in.inst_cutoff_roughness.x, final_alpha, in.clip_position.xy)) {
        discard;
    }

    // The scene's rain (`Renderer::wetness`) darkens what it soaks; its smoothness shows in what reflects
    // it — the screen-space reflections, which `fs_reflective` hands it to.
    let base = in.inst_albedo.rgb * tex.rgb * wet_darkening(rain);

    // The vertex colour is taken at face value. It used to be second-guessed here — a
    // near-zero length was rewritten to white, so that an importer which never set the
    // attribute could not black out the model. But whether the attribute EXISTS is a property
    // of the vertex layout, not of a pixel value, and this layout always has it: a source with
    // no colours is normalised to opaque white at import (`gpu_types::Vertex::default`). Under
    // the old test a surface the author painted black came out white instead.
    // With a lightmap (`MaterialMaps::lightmap`) the baked light is the lightmap's, times the vertex
    // colour — which an importer leaves white, so a lightmapped mesh is lit by its map alone.
    var baked = in.color.rgb;
    let lightmap_strength = material.uv_scale.w;
    if (lightmap_strength > 0.0) {
        baked = baked * textureSample(t_lightmap, s_diffuse, in.tex_coords.zw).rgb * lightmap_strength;
    }

    // Distance along the camera's forward axis — the same measure `cascade_splits` is expressed
    // in, and what `select_cascade` and the fade both assume. (Radial `length(...)` overstates
    // the depth of anything off-axis, so it picked a cascade too far out at the screen edges
    // and put the range boundary on a sphere instead of a plane.)
#ifdef SUN_SHADOWS
    let view_depth = dot(in.world_pos - scene.camera_pos.xyz, scene.camera_forward.xyz);
#ifdef SUN_SHADOW_PER_VERTEX
    // The forward-only path's: one cascade, a coordinate from the vertices, one tap (`sun_shadow_tap`).
    var vis = 1.0;
    let fade = shadow_distance_fade(view_depth, scene.cascade_splits.w);
    if (scene.sun_direction.w > 0.5 && fade > 0.0 && in.inst_emissive.w >= 0.0) {
        vis = mix(1.0, sun_shadow_tap(t_shadow, s_shadow, in.sun_shadow, n, normalize(-scene.sun_direction.xyz), scene.shadow_bias.x), fade);
    }
#else
    // `MeshRenderer::receives_shadows == false` keeps the sun's share whole.
    let vis = select(sun_visibility(in.world_pos, n, surface_axes, view_depth), 1.0, in.inst_emissive.w < 0.0);
#endif
#else
    let vis = 1.0;
#endif

    // The shadow darkens rather than replaces. Baked lighting already includes the sky and the
    // bounce; a shadowed patch of it is dimmer, not black, and multiplying the whole thing by the
    // visibility would make every shadow a hole. 0.45 is how much of the surface's own light the
    // sun is taken to be responsible for.
    let sun_share = BAKED_SUN_SHARE;
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
