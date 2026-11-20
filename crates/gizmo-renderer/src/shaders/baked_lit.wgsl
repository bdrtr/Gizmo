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
#import gizmo::surface::{rain_on, rain_ripples, ripple_footprint, water_normal, wet_darkening, wet_roughness, WATER_F0}
#import gizmo::common::{compute_direct_lighting, dielectric_f0}
#ifdef RAIN_MAP
#import gizmo::common::{rain_exposure}
#endif
#ifdef REFLECTION_INPUTS
#import gizmo::common::{BAKED_SURFACE}
#endif
#ifdef LOCAL_LIGHTS
#import gizmo::common::{LightData, light_falloff, spot_cone, spot_cookie_uv, gizmo_cluster_index}
#endif
#ifdef SIZED_LIGHTS
#import gizmo::common::{compute_sphere_lighting, compute_tube_lighting}
#endif
#ifdef RECT_LIGHTS
#import gizmo::common::{compute_rect_lighting}
#endif
#ifdef LOCAL_LIGHT_MAPS
#ifdef LOCAL_SHADOW_TAP
#import gizmo::common::{local_shadow_layer, local_shadow_tap}
#else
#import gizmo::common::{local_shadow_layer, local_shadow_visibility, light_source_radius}
#endif
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

#ifdef LOCAL_LIGHTS
// The clustered light lists (`gizmo_renderer::clustered`), on the global group as the lit shader has them:
// what the water on a baked-lit surface reflects, and the lights its bake does not hold (`baked_surface_lights`).
@group(0) @binding(1)
var<storage, read> cluster_table: array<vec2<u32>>;
@group(0) @binding(2)
var<storage, read> cluster_light_indices: array<u32>;
#endif
#ifdef LOCAL_LIGHT_MAPS
// The local lights' shadow layers and the spots' cookies, as the lit shader reads them.
@group(#{LOCAL_LIGHT_MAP_GROUP}) @binding(#{LOCAL_SHADOW_TEXTURE_BINDING})
var t_local_shadow: texture_depth_2d_array;
@group(#{LOCAL_LIGHT_MAP_GROUP}) @binding(#{COOKIE_TEXTURE_BINDING})
var t_cookie: texture_2d_array<f32>;
@group(#{LOCAL_LIGHT_MAP_GROUP}) @binding(#{COOKIE_SAMPLER_BINDING})
var s_cookie: sampler;
#ifndef SUN_SHADOWS
// Their comparison sampler is the sun's, declared with the sun's shadow when there is one.
@group(#{SUN_SHADOW_GROUP}) @binding(#{SUN_SHADOW_SAMPLER_BINDING})
var s_shadow: sampler_comparison;
#endif
#endif

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
    // Invariant: the z-prepass draws the same surfaces with this vertex stage alone
    // (`SceneState::baked_lit_depth_pipelines`), and the forward draw must meet its depth exactly.
    @invariant @builtin(position) clip_position: vec4<f32>,
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
    // x = the cut-out and LOD fade (`instance_keeps`), y = roughness, z = `Material::porosity`.
    @location(7) inst_cutoff_roughness: vec3<f32>,
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
    out.inst_cutoff_roughness = vec3<f32>(inst.ambient.w, inst.pbr.x, 1.0 - inst.texture.y);
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
fn fs_main(in: VertexOutput, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    // How wide a pixel is on the ground, for the rain's rings — taken before any pixel of the quad discards.
    let ripple_px = ripple_footprint(dpdx(in.world_pos), dpdy(in.world_pos));
    let rain = rain_at(in.world_pos, in.world_normal, in.inst_cutoff_roughness.z);
    return shade(in, rain, rain_ripples(scene.wetness, in.world_pos, scene.cascade_params.z, ripple_px, rain), front);
}

// The scene's rain on a fragment (`rain_on`), kept off what is under cover where there is a rain map —
// looked up only on what faces up enough to take rain at all.
fn rain_at(world_position: vec3<f32>, normal: vec3<f32>, porosity: f32) -> vec3<f32> {
    let up = normalize(normal).y;
    var exposure = 1.0;
#ifdef RAIN_MAP
    if (scene.wetness.x > 0.0 && up > 0.2) {
        exposure = rain_exposure(t_shadow, s_shadow, i32(#{RAIN_LAYER}), scene.rain_view_proj, scene.rain_params, world_position);
    }
#endif
    return rain_on(scene.wetness, world_position, up, exposure, porosity);
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
    let ripple_px = ripple_footprint(dpdx(in.world_pos), dpdy(in.world_pos));
    let rain = rain_at(in.world_pos, in.world_normal, in.inst_cutoff_roughness.z);
    let ripples = rain_ripples(scene.wetness, in.world_pos, scene.cascade_params.z, ripple_px, rain);
    var out: ReflectionInputs;
    out.colour = shade(in, rain, ripples, front);
    let n = normalize(select(-in.world_normal, in.world_normal, front));
    out.normal_roughness = vec4<f32>(
        water_normal(n, rain, ripples),
        wet_roughness(in.inst_cutoff_roughness.y, rain, ripples),
    );
    out.position_rel_camera = vec4<f32>(in.world_pos - scene.camera_pos.xyz, BAKED_SURFACE);
    let f0 = dielectric_f0(in.inst_emissive.w);
    out.reflectance = vec4<f32>(0.0, 0.0, 0.0, select(f0, -f0, in.inst_emissive.w < 0.0));
    return out;
}
#endif

// What the frame's lights add to a baked-lit surface, which its bake does not hold.
//
// The water on it (`water` of it, `rain_on`'s) reflects the sun and every light: water's reflectance
// (`WATER_F0`) at its roughness, view-dependent, so no bake holds it — the lit shader's lights, the same
// way, with no colour of its own beneath. And a light that is not `in_bake` lights the surface itself as a
// lit surface is lit — a car's headlights on a baked street: the first `scene.unbaked_lights` of the
// frame's lights, which every cluster's list (ascending) starts with, so the loop stops at the first light
// past them on a dry surface. `sun`: the sun's visibility here; `receives_shadows`: whether the local
// lights' shadows fall on it.
fn baked_surface_lights(
    world_pos: vec3<f32>,
    N: vec3<f32>,
    base: vec3<f32>,
    roughness: f32,
    f0: vec3<f32>,
    water: f32,
    water_n: vec3<f32>,
    water_roughness: f32,
    sun: f32,
    receives_shadows: bool,
#ifdef LOCAL_LIGHT_MAPS
#ifndef LOCAL_SHADOW_TAP
    axes: ReceiverAxes,
#endif
#endif
) -> vec3<f32> {
    let V = normalize(scene.camera_pos.xyz - world_pos);
    let f0w = vec3<f32>(WATER_F0);
    let rw = max(water_roughness, 0.05);
    var total = vec3<f32>(0.0);
    if (water > 0.0 && scene.sun_direction.w > 0.5) {
        total += water * compute_direct_lighting(
            water_n, V, normalize(-scene.sun_direction.xyz), vec3<f32>(0.0), rw, 0.0, f0w, scene.sun_color.rgb, scene.sun_color.w, sun,
        );
    }
#ifdef LOCAL_LIGHTS
    let cluster = gizmo_cluster_index(scene.view_proj, scene.camera_pos.xyz, scene.camera_forward.xyz, scene.cluster_dims, scene.cluster_depth, world_pos);
    let cluster_range = cluster_table[cluster];
    for (var ci = 0u; ci < cluster_range.y; ci++) {
        let i = cluster_light_indices[cluster_range.x + ci];
        let unbaked = i < scene.unbaked_lights;
        if (!unbaked && water <= 0.0) {
            break;
        }
        let light = scene.lights[i];
        let light_type = u32(light.params.y);
        let intensity = light.position.w;
        var L: vec3<f32>;
        var attenuation = 1.0;
        var window = 1.0;
        var cookie = vec3<f32>(1.0);
        if (light_type == 2u) {
            L = normalize(-light.direction.xyz);
        } else {
            let to_light = light.position.xyz - world_pos;
            L = normalize(to_light);
            let falloff = light_falloff(length(to_light), light.color.a);
            attenuation = falloff.x;
            window = falloff.y;
            if (light_type == 1u) {
                attenuation *= spot_cone(light, -L);
            }
            // Past its reach, or outside its cone, it gives nothing here: no cookie to read, no shadow to
            // look up. A rectangle is lit by its window alone. A clustered light is in every cluster its
            // sphere touches, behind a car's headlights as much as in front of them.
            if (select(attenuation, window, light_type == 3u) <= 0.0) {
                continue;
            }
#ifdef LOCAL_LIGHT_MAPS
            if (light_type == 1u && light.params.w > 0.0) {
                cookie = textureSampleLevel(t_cookie, s_cookie, spot_cookie_uv(light, world_pos), i32(light.params.w) - 1, 0.0).rgb;
            }
#endif
#ifdef LOCAL_LIGHT_MAPS
            let shadow_layer = local_shadow_layer(scene.light_shadows[i / 4u][i % 4u], world_pos - light.position.xyz);
            if (receives_shadows && shadow_layer >= 0 && max(attenuation, window) > 0.0) {
                // As the lit shader takes it: one tap on the forward-only path, filtered on the desktop.
#ifdef LOCAL_SHADOW_TAP
                let lit = local_shadow_tap(
                    t_local_shadow, s_shadow, scene.local_shadow_layers[shadow_layer].view_proj,
                    scene.local_shadow_layers[shadow_layer].params, shadow_layer, world_pos, N, to_light,
                    scene.shadow_bias.xy,
                );
#else
                let lit = local_shadow_visibility(
                    t_local_shadow, s_shadow, scene.local_shadow_layers[shadow_layer], shadow_layer,
                    world_pos, N, axes, to_light, scene.shadow_bias.xy, light_source_radius(light),
                );
#endif
                attenuation *= lit;
                window *= lit;
            }
#endif
        }
        let light_rgb = light.color.rgb * cookie;
        if (water > 0.0) {
            total += water * baked_light_term(light, light_type, world_pos, water_n, V, L, vec3<f32>(0.0), rw, f0w, light_rgb, intensity, attenuation, window);
        }
        if (unbaked) {
            total += baked_light_term(light, light_type, world_pos, N, V, L, base, roughness, f0, light_rgb, intensity, attenuation, window);
        }
    }
#endif
    return total;
}

#ifdef LOCAL_LIGHTS
// One light on a surface of `albedo` (black for the water's reflection alone), by its kind: a rectangle,
// a tube, a sphere, or a point.
fn baked_light_term(
    light: LightData,
    light_type: u32,
    world_pos: vec3<f32>,
    N: vec3<f32>,
    V: vec3<f32>,
    L: vec3<f32>,
    albedo: vec3<f32>,
    roughness: f32,
    f0: vec3<f32>,
    light_rgb: vec3<f32>,
    intensity: f32,
    attenuation: f32,
    window: f32,
) -> vec3<f32> {
#ifdef RECT_LIGHTS
    if (light_type == 3u) {
        return compute_rect_lighting(
            N, V, light.position.xyz - world_pos, light.direction, light.params.z, light.params.w,
            albedo, roughness, 0.0, f0, light_rgb, intensity, window,
        );
    }
#endif
#ifdef SIZED_LIGHTS
    if (light_type == 0u && light.params.w > 0.0) {
        return compute_tube_lighting(
            N, V, light.position.xyz - world_pos, light.direction.xyz, light.params.w, light.params.z,
            albedo, roughness, 0.0, f0, light_rgb, intensity, attenuation,
        );
    }
    if (light_type != 2u && light.params.z > 0.0) {
        return compute_sphere_lighting(
            N, V, light.position.xyz - world_pos, light.params.z, albedo, roughness, 0.0, f0, light_rgb, intensity, attenuation,
        );
    }
#endif
    return compute_direct_lighting(N, V, L, albedo, roughness, 0.0, f0, light_rgb, intensity, attenuation);
}
#endif

// The surface's colour, fogged, with its alpha; `rain` is `rain_on`'s, `ripples` `rain_ripples`'s.
// `front`: whether the fragment is its triangle's front, the side its normal is on — a double-sided surface
// seen from behind is lit on the side the camera sees, as the G-buffer route lights it (`fs_baked`).
fn shade(in: VertexOutput, rain: vec3<f32>, ripples: vec3<f32>, front: bool) -> vec4<f32> {
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

    // What the frame's lights add (`baked_surface_lights`): the rain on it reflects the sun and every light —
    // what no bake holds, since a reflection depends on where it is seen from — and a light that is not
    // `in_bake` lights it as it lights any surface. A dry surface under lights the bake holds is the bake's
    // alone.
    var shaded = colour;
    let water = max(rain.x, rain.y);
    if (water > 0.0 || scene.unbaked_lights > 0u) {
        let surface_n = normalize(select(-in.world_normal, in.world_normal, front));
        let water_n = water_normal(surface_n, rain, ripples);
        shaded += baked_surface_lights(
            in.world_pos, surface_n, base, wet_roughness(in.inst_cutoff_roughness.y, rain, ripples),
            vec3<f32>(dielectric_f0(in.inst_emissive.w)), water, water_n,
            wet_roughness(in.inst_cutoff_roughness.y, rain, ripples), vis, in.inst_emissive.w >= 0.0,
#ifdef LOCAL_LIGHT_MAPS
#ifndef LOCAL_SHADOW_TAP
            surface_axes,
#endif
#endif
        );
    }

    // Vertex alpha multiplies through with the instance's and the texture's. It only reaches the
    // framebuffer on the blended (transparent) variant of this pipeline — the opaque variant
    // writes it to the HDR target's unread alpha channel, exactly as it did before.
    let fog = height_fog(in.world_pos, scene.camera_pos.xyz, scene.fog, scene.fog_shape);
    return vec4<f32>(mix(shaded, fog.rgb, fog.a), final_alpha);
}
