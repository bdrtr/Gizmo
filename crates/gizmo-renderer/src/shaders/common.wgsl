#define_import_path gizmo::common

// Shared shader library, composed into other shaders by `load_shader_composed`
// (naga_oil `#import gizmo::common::{...}`). This is the SINGLE source of truth for the
// scene uniform layout and the core BRDF, replacing the hand-copied duplicates that used
// to live in 20+ shaders and silently drift apart. Only put things here that are pure
// (no reference to a module-scope binding like `scene`): structs, constants, and helper
// functions that take everything they need as parameters.

const PI: f32 = 3.1415926535;
const INV_PI: f32 = 0.31830988618; // 1 / PI — Lambert diffuse normalization

// One dynamic light. Encoding (matches gpu_types::LightData on the CPU):
//   position.xyz = world pos,        position.w = intensity
//   color.rgb    = colour,           color.a    = radius
//   direction.xyz= spot/dir axis,    direction.w= inner cutoff cos
//   params.x     = outer cutoff cos, params.y   = light type (0=point,1=spot,2=dir,3=rect)
//   (rect: direction = orientation quaternion, params.zw = half width, half height)
//   (spot: params.w = cookie layer + 1, 0 = no cookie)
//   params.z     = source radius (metres; 0 = a point — see compute_sphere_lighting)
//   params.w     = tube length along direction.xyz (point lights only; see compute_tube_lighting)
struct LightData {
    position:  vec4<f32>,
    color:     vec4<f32>,
    direction: vec4<f32>,
    params:    vec4<f32>,
};

// One layer of the local shadow array (`gizmo_renderer::local_shadows`, gpu_types::LocalShadowLayerRaw):
// the matrix it was drawn through, and params = (near, far, a texel's width per metre of depth,
// 1 / resolution).
struct LocalShadowLayer {
    view_proj: mat4x4<f32>,
    params:    vec4<f32>,
};

// Global per-frame scene uniform. MUST stay byte-compatible with
// gpu_types::SceneUniforms on the CPU side.
struct SceneUniforms {
    view_proj:       mat4x4<f32>,
    camera_pos:      vec4<f32>,
    sun_direction:   vec4<f32>,   // xyz = sun dir, w = sun-present flag (1/0)
    sun_color:       vec4<f32>,   // rgb = colour, w = intensity
    lights:          array<LightData, 256>,
    light_view_proj: array<mat4x4<f32>, 4>,
    cascade_splits:  vec4<f32>,
    camera_forward:  vec4<f32>,
    cascade_params:  vec4<f32>,   // x=znear, y=1/shadowRes, z=time, w unused
    num_lights: u32,
    exposure: f32,
    // `WireframeConfig::color`, RGBA8; alpha 0 = none. Read by the WIREFRAME build of shader.wgsl.
    wireframe_color: u32,
    _pre_align_pad: u32,
    // The background: `Camera::clear_color`, or `frame_uniforms::DEFAULT_BACKGROUND`.
    clear_color: vec3<f32>,
    environment_blend_t: f32,
    environment_preset: u32,
    // Padding (`point_shadows_enabled` until 2026-11-11).
    _pad_shadow_flag: u32,
    // Named without a trailing digit: naga_oil reserves the `_<number>` suffix for naga's
    // WGSL writeback name-mangling and rejects composable-module identifiers that use it.
    // Only the byte layout must match gpu_types::SceneUniforms.environment_preset_2.
    environment_preset_b: u32,
    shading_mode: u32,
    // inverse(view_proj), computed once per frame on the CPU (gpu_types::SceneUniforms).
    // Fullscreen passes that unproject NDC→world read this instead of calling inverse_mat4
    // per fragment. Appended at the tail so all prior field offsets stay byte-compatible.
    inv_view_proj: mat4x4<f32>,
    // Clustered light culling (gizmo-renderer/src/clustered.rs). dims = (x, y, z, 0) tiles/slices;
    // depth = (z_scale, z_bias, 0, 0) for slice = floor(log(view_depth) * z_scale + z_bias).
    cluster_dims:  vec4<u32>,
    cluster_depth: vec4<f32>,
    // The frame's fog, a hand-set one or the preset's (`frame_uniforms::frame_fog`): rgb = colour,
    // a = density; shape = height falloff, base height, `FogMode` (0, 1, 2), a linear ramp's start.
    fog: vec4<f32>,
    fog_shape: vec4<f32>,
    // `ShadowSettings`: x = depth-bias multiplier, y = normal-offset multiplier.
    shadow_bias: vec4<f32>,
    // `Renderer::sky`: rgb = colour straight up / at the horizon / straight down; zenith.a =
    // intensity (< 0 = the preset's sky).
    sky_zenith: vec4<f32>,
    sky_horizon: vec4<f32>,
    sky_ground: vec4<f32>,
    // The local lights' shadow layers, and each light slot's entry in them, four to a row:
    // `local_shadow_layer` reads an entry. See `gizmo_renderer::local_shadows`.
    local_shadow_layers: array<LocalShadowLayer, 32>,
    light_shadows: array<vec4<u32>, 64>,
};

// Depth bias in METRES, for every shadow lookup that is a surface's: the cascades' and the local
// lights'. Calibrated to reproduce what the old NDC constants were worth at the depth range they were
// chosen under: 0.0004 NDC over a ~105 m cascade is 4.2 cm, and the 0.00004 floor is 4.2 mm.
const SHADOW_BIAS_METRES: f32 = 0.042;
const SHADOW_BIAS_METRES_MIN: f32 = 0.0042;

// Set on a light's `light_shadows` entry when its shadow is six layers, a cube around it
// (`local_shadows::LOCAL_SHADOW_CUBE`).
const LOCAL_SHADOW_CUBE: u32 = 0x80000000u;

// The local shadow layer a light's shadow is looked up in for a point `from_light` away from it, or
// -1 for a light that casts none. `entry` is the light's `light_shadows` slot. A cube's face is the
// axis `from_light` is longest along, numbered +X, −X, +Y, −Y, +Z, −Z as the CPU lays them out.
fn local_shadow_layer(entry: u32, from_light: vec3<f32>) -> i32 {
    if (entry == 0u) {
        return -1;
    }
    let first = i32(entry & ~LOCAL_SHADOW_CUBE) - 1;
    if ((entry & LOCAL_SHADOW_CUBE) == 0u) {
        return first;
    }
    let a = abs(from_light);
    if (a.x >= a.y && a.x >= a.z) {
        return first + select(1, 0, from_light.x >= 0.0);
    }
    if (a.y >= a.z) {
        return first + select(3, 2, from_light.y >= 0.0);
    }
    return first + select(5, 4, from_light.z >= 0.0);
}

// The widest a local light's penumbra is drawn, each side, in its layer's texels: 5 cm at 1 m from a
// cube face, 50 cm at 10 m.
const MAX_PENUMBRA_TEXELS: f32 = 48.0;

// Two directions along the surface a shadow falls on, for `local_shadow_grade`: the pixel's own
// neighbours (`dx`, `dy`: the screen-space derivatives of its world position), which lie on its
// geometry — where they agree with its normal `n` to within 75°; at an edge, where a neighbour is
// another surface, the plane square to `n`. Not `n`'s plane always: a normal map tilts `n` off the
// geometry, and a filter that follows it climbs out of the surface — 13 105 pixels of a floor shadowed
// themselves under an 80 cm sphere with its normal map tilting it 0.9 rad.
fn receiver_axes(dx: vec3<f32>, dy: vec3<f32>, n: vec3<f32>) -> mat2x3<f32> {
    let g = cross(dx, dy);
    let len = length(g);
    if (len > 1e-12 && abs(dot(g / len, n)) > 0.26) {
        return mat2x3<f32>(normalize(dx), normalize(dy));
    }
    let side = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(n.y) > 0.9);
    let t1 = normalize(cross(n, side));
    return mat2x3<f32>(t1, cross(n, t1));
}

// How a surface's depth runs across a local shadow layer: d(ndc z)/d(uv) of the plane through `at`
// along `axes` (`receiver_axes`), from two points `step` metres along it, against `here` = (uv, ndc z)
// at `at`. Zero where the plane is edge-on to the light (nothing of it is lit there) and capped where
// it is near to edge-on, so a tap is never moved further than the plane leaning 80° away from the
// light would.
fn local_shadow_grade(layer: LocalShadowLayer, at: vec3<f32>, axes: mat2x3<f32>, step: f32, here: vec3<f32>) -> vec2<f32> {
    let a = layer_point(layer.view_proj * vec4<f32>(at + axes[0] * step, 1.0)) - here;
    let b = layer_point(layer.view_proj * vec4<f32>(at + axes[1] * step, 1.0)) - here;
    let det = a.x * b.y - a.y * b.x;
    if (abs(det) < 1e-14) {
        return vec2<f32>(0.0);
    }
    let grade = vec2<f32>(a.z * b.y - b.z * a.y, b.z * a.x - a.z * b.x) / det;
    // At 80° a plane climbs tan 80° = 5.7 of its texel per texel across; in NDC that is 5.7 times what
    // a texel of depth is worth at this distance — `a.z` at a one-texel step face on.
    let cap = 5.7 * length(vec2<f32>(length(a.xy), length(b.xy))) * max(abs(a.z), abs(b.z)) / max(length(a.xy) * length(b.xy), 1e-12);
    let size = length(grade);
    return select(grade, grade * (cap / size), size > cap);
}

// A clip-space point as a layer's (uv, ndc z).
fn layer_point(clip: vec4<f32>) -> vec3<f32> {
    let p = clip.xyz / clip.w;
    return vec3<f32>(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5, p.z);
}

// The radius of a light's source, metres — what softens its shadow (`local_shadow_visibility`): a
// point's or a spot's `source_radius`; for a tube, its thickness and half its length averaged (its
// penumbra is long along the tube and narrow across it, and one disc stands in for both); for a
// rectangle, the geometric mean of its half extents. 0 for a point source.
fn light_source_radius(light: LightData) -> f32 {
    let kind = u32(light.params.y);
    if (kind == 3u) {
        return sqrt(max(light.params.z * light.params.w, 0.0));
    }
    // A point light's params.w is its tube length; a spot's is its cookie.
    if (kind == 0u && light.params.w > 0.0) {
        return 0.5 * (light.params.z + 0.5 * light.params.w);
    }
    if (kind <= 1u) {
        return max(light.params.z, 0.0);
    }
    return 0.0;
}

// How much of a local light reaches `world_pos` past its shadow: 1 lit, 0 shadowed, filtered between.
// `t`/`s` are the local shadow array and the comparison sampler, `layer` the layer `index` names, `n`
// the surface normal, `axes` two directions along its geometry (`receiver_axes`), `to_light` the
// vector from the surface to the light, `bias` = `scene.shadow_bias.xy` (the depth-bias and
// normal-offset multipliers) and `source_radius` the light's (`light_source_radius`).
//
// The same two biases the cascades take, in the same units: pushed off the surface by two texels'
// width at this distance, and compared past it by a slope-scaled depth bias in metres — turned into
// this layer's NDC at this depth, which is the step a perspective map needs and an orthographic one
// does not (a fixed NDC bias would grow with the square of the distance from the light).
//
// A point source's edge is filtered over 3×3 texels. A source with a size softens it as the sun's
// angular radius softens the sun's — percentage-closer soft shadows: the blockers between the surface
// and the light are found, and the edge is filtered as wide as the penumbra a disc of that radius
// throws past them, so a shadow is sharp where its caster touches and soft away from it.
fn local_shadow_visibility(
    t: texture_depth_2d_array,
    s: sampler_comparison,
    layer: LocalShadowLayer,
    index: i32,
    world_pos: vec3<f32>,
    n: vec3<f32>,
    axes: mat2x3<f32>,
    to_light: vec3<f32>,
    bias: vec2<f32>,
    source_radius: f32,
) -> f32 {
    let near = layer.params.x;
    let far = layer.params.y;
    // Depth along the layer's axis is a perspective clip's w.
    let depth = max((layer.view_proj * vec4<f32>(world_pos, 1.0)).w, near);
    let offset = n * (depth * layer.params.z * 2.0 * bias.y);
    let clip = layer.view_proj * vec4<f32>(world_pos + offset, 1.0);
    if (clip.w <= near) {
        return 1.0;
    }
    let ndc = clip.xyz / clip.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || ndc.z > 1.0) {
        return 1.0;
    }
    // d(ndc z)/d(depth) for a [0, 1] perspective: near·far / ((far − near)·depth²).
    let ndc_per_metre = near * far / max((far - near) * clip.w * clip.w, 1e-12);
    let slope = 1.0 - clamp(dot(n, normalize(to_light)), 0.0, 1.0);
    let reference = ndc.z - max(SHADOW_BIAS_METRES * slope, SHADOW_BIAS_METRES_MIN) * ndc_per_metre * bias.x;
    let texel = layer.params.w;
    if (source_radius <= 0.0) {
        var lit = 0.0;
        for (var x = -1; x <= 1; x++) {
            for (var y = -1; y <= 1; y++) {
                lit += textureSampleCompareLevel(t, s, uv + vec2<f32>(f32(x), f32(y)) * texel, index, reference);
            }
        }
        return lit / 9.0;
    }
    // The layer spans `span`·z metres at depth z (span = 2·tan of its half-angle), so `m` metres at
    // the receiver's depth are m / (span·z) of its uv.
    let span = layer.params.z / layer.params.w;
    let z_receiver = clip.w;
    // **The receiver's plane** (Isidoro's receiver-plane depth bias): a tap far from the centre is
    // compared against where the surface is there, not where it is here. Across a penumbra dozens
    // of texels wide, a floor at a slant from the light climbs further than any fixed bias, and the
    // filter shadowed the floor with the floor — everywhere past the angle where it outran the bias.
    // Two points a texel along the surface, projected, give how its depth runs across the layer.
    let grade = local_shadow_grade(layer, world_pos + offset, axes, depth * layer.params.z, vec3<f32>(uv, ndc.z));
    // **Blockers**: where the disc's light to this point can be stopped — at a third of the way
    // from the light, the disc seen from here is twice its radius across — searched on a 5×5 grid,
    // as far out as the widest penumbra the filter below draws: a point at a penumbra's outer edge
    // finds its blockers that far away, and a narrower search left it lit.
    let search = clamp(2.0 * source_radius / (span * z_receiver), 1.5 * texel, MAX_PENUMBRA_TEXELS * texel);
    let size = vec2<f32>(textureDimensions(t));
    var blockers = 0.0;
    var blocker_ndc = 0.0;
    for (var x = -2; x <= 2; x++) {
        for (var y = -2; y <= 2; y++) {
            let step = vec2<f32>(f32(x), f32(y)) * (search * 0.5);
            let p = clamp(uv + step, vec2<f32>(0.0), vec2<f32>(1.0));
            let stored = textureLoad(t, vec2<i32>(min(p * size, size - 1.0)), index, 0);
            if (stored < reference + dot(grade, step)) {
                blockers += 1.0;
                blocker_ndc += stored;
            }
        }
    }
    if (blockers < 0.5) {
        return 1.0;
    }
    // Their mean depth, linear: a [0, 1] perspective's NDC is near·far / (far − ndc·(far − near)).
    let z_blocker = near * far / max(far - (blocker_ndc / blockers) * (far - near), 1e-6);
    // **The penumbra** a disc of `source_radius` throws past them, each side: R·(z_r − z_b)/z_b.
    let radius = clamp(
        source_radius * max(z_receiver - z_blocker, 0.0) / (max(z_blocker, near) * span * z_receiver),
        texel,
        MAX_PENUMBRA_TEXELS * texel,
    );
    // Twenty-five taps that far apart would band, so the grid turns by a per-texel angle — banding
    // becomes fine noise, which TAA averages away, as in the sun's filter.
    let angle = fract(sin(dot(floor(uv / texel), vec2<f32>(12.9898, 78.233))) * 43758.5453) * 6.2831853;
    let turn = mat2x2<f32>(cos(angle), sin(angle), -sin(angle), cos(angle));
    var lit = 0.0;
    for (var x = -2; x <= 2; x++) {
        for (var y = -2; y <= 2; y++) {
            let offset_uv = turn * (vec2<f32>(f32(x), f32(y)) * (radius * 0.5));
            lit += textureSampleCompareLevel(t, s, uv + offset_uv, index, reference + dot(grade, offset_uv));
        }
    }
    return lit / 25.0;
}

// `Renderer::sky` along `dir`: the gradient, softened toward the hemisphere's average as
// `roughness` grows (a rough surface sees a wide cone of it). Pure: the colours come in.
fn sky_gradient(dir: vec3<f32>, zenith: vec3<f32>, horizon: vec3<f32>, ground: vec3<f32>, roughness: f32) -> vec3<f32> {
    let y = clamp(dir.y, -1.0, 1.0);
    let sharp = select(
        mix(horizon, ground, sqrt(max(-y, 0.0))),
        mix(horizon, zenith, sqrt(max(y, 0.0))),
        y >= 0.0,
    );
    let soft = mix(mix(ground, horizon, 0.5), mix(horizon, zenith, 0.5), y * 0.5 + 0.5);
    return mix(sharp, soft, clamp(roughness, 0.0, 1.0));
}

fn single_preset_environment(preset: u32, dir: vec3<f32>, roughness: f32, sun_dir: vec3<f32>, sun_dot: f32, zenith: f32, horizon_blend: f32) -> vec3<f32> {
    // Preset 0: Sunset Gold
    if (preset == 0u) {
        let sky_color = mix(vec3<f32>(0.05, 0.02, 0.08), vec3<f32>(0.1, 0.05, 0.15), max(zenith, 0.0));
        let horizon_color = vec3<f32>(0.9, 0.35, 0.1);
        let base_sky = mix(horizon_color, sky_color, pow(max(zenith, 0.0), 0.5));
        
        let sun_power = mix(4096.0, 16.0, roughness);
        let sun_glow = pow(sun_dot, sun_power) * vec3<f32>(20.0, 12.0, 3.0) * mix(1.0, 0.1, roughness);
        
        let rim_dir = normalize(vec3<f32>(-0.8, 0.3, -0.5));
        let rim_dot = max(dot(dir, rim_dir), 0.0);
        let rim_glow = pow(rim_dot, mix(256.0, 8.0, roughness)) * vec3<f32>(0.2, 0.3, 0.6) * mix(1.0, 0.15, roughness);
        
        let ground_color = vec3<f32>(0.12, 0.05, 0.03);
        
        let sky_val = base_sky * 0.5 + sun_glow * 0.4 + rim_glow;
        let ground_val = mix(ground_color, horizon_color * 0.3, pow(1.0 + clamp(zenith, -1.0, 0.0), 4.0)) + sun_glow * 0.1;
        return mix(ground_val, sky_val, horizon_blend);
    }
    
    // Preset 1: Studio Neutral
    if (preset == 1u) {
        let sky_color = vec3<f32>(0.1, 0.12, 0.15);
        let horizon_color = vec3<f32>(0.4, 0.42, 0.45);
        let base_sky = mix(horizon_color, sky_color, max(zenith, 0.0));
        
        let key_dir = normalize(vec3<f32>(0.6, 0.8, 0.5));
        let key_dot = max(dot(dir, key_dir), 0.0);
        let key_glow = pow(key_dot, mix(1024.0, 12.0, roughness)) * vec3<f32>(6.0, 5.8, 5.5) * mix(1.0, 0.2, roughness);
        
        let fill_dir = normalize(vec3<f32>(-0.8, 0.2, -0.6));
        let fill_dot = max(dot(dir, fill_dir), 0.0);
        let fill_glow = pow(fill_dot, mix(512.0, 8.0, roughness)) * vec3<f32>(1.5, 1.8, 2.5) * mix(1.0, 0.15, roughness);
        
        let rim_dir = normalize(vec3<f32>(0.0, 0.1, -1.0));
        let rim_dot = max(dot(dir, rim_dir), 0.0);
        let rim_glow = pow(rim_dot, mix(2048.0, 16.0, roughness)) * vec3<f32>(3.0, 2.5, 1.8) * mix(1.0, 0.2, roughness);
        
        let ground_color = vec3<f32>(0.03, 0.03, 0.035);
        
        let sky_val = base_sky * 0.3 + key_glow * 0.4 + fill_glow * 0.2 + rim_glow * 0.3;
        let ground_val = mix(ground_color, horizon_color * 0.15, pow(1.0 + clamp(zenith, -1.0, 0.0), 4.0)) + key_glow * 0.1;
        return mix(ground_val, sky_val, horizon_blend);
    }
    
    // Preset 2: Midnight Neon
    if (preset == 2u) {
        let sky_color = vec3<f32>(0.005, 0.005, 0.02);
        let horizon_color = vec3<f32>(0.01, 0.01, 0.03);
        let base_sky = mix(horizon_color, sky_color, max(zenith, 0.0));
        
        let neon1_dir = normalize(vec3<f32>(0.7, 0.2, 0.5));
        let neon1_dot = max(dot(dir, neon1_dir), 0.0);
        let neon1_glow = pow(neon1_dot, mix(2048.0, 8.0, roughness)) * vec3<f32>(18.0, 0.0, 12.0) * mix(1.0, 0.15, roughness);
        
        let neon2_dir = normalize(vec3<f32>(-0.7, 0.3, -0.5));
        let neon2_dot = max(dot(dir, neon2_dir), 0.0);
        let neon2_glow = pow(neon2_dot, mix(1024.0, 6.0, roughness)) * vec3<f32>(0.0, 14.0, 18.0) * mix(1.0, 0.12, roughness);
        
        let top_dir = vec3<f32>(0.0, 1.0, 0.0);
        let top_glow = pow(max(dot(dir, top_dir), 0.0), 3.0) * vec3<f32>(0.4, 0.0, 0.8);
        
        let ground_color = vec3<f32>(0.005, 0.005, 0.008);
        
        let sky_val = base_sky * 0.2 + neon1_glow + neon2_glow + top_glow * 0.15;
        let ground_val = mix(ground_color, horizon_color * 0.2, pow(1.0 + clamp(zenith, -1.0, 0.0), 4.0)) + (neon1_glow + neon2_glow) * 0.05;
        return mix(ground_val, sky_val, horizon_blend);
    }
    
    // Default Preset 3: Classic Daylight
    let sky_color = mix(vec3<f32>(0.5, 0.65, 1.0), vec3<f32>(0.05, 0.2, 0.6), max(zenith, 0.0));
    let base_sky = sky_color;
    
    let sun_power = mix(2048.0, 16.0, roughness);
    let sun_glow = pow(sun_dot, sun_power) * vec3<f32>(15.0, 12.0, 8.0) * mix(1.0, 0.2, roughness);
    
    let horizon_glow = pow(1.0 - max(zenith, 0.0), 4.0) * vec3<f32>(1.0, 0.6, 0.3) * max(dot(dir, sun_dir) * 0.5 + 0.5, 0.0);
    
    let ground_color = vec3<f32>(0.05, 0.05, 0.05);
    
    let sky_val = base_sky * 0.8 + sun_glow * 0.05 + horizon_glow;
    let ground_val = mix(ground_color, horizon_glow * 0.2, pow(1.0 + clamp(zenith, -1.0, 0.0), 4.0)) + sun_glow * 0.1;
    return mix(ground_val, sky_val, horizon_blend);
}

// What lights and is reflected by a surface along `dir`: `Renderer::sky` when one is set (`zenith.a`, its
// intensity, ≥ 0; < 0 = none), otherwise the presets. The forward and deferred shaders call it per pixel
// with the frame's uniforms, and `env_bake.wgsl` once per texel of the forward-only path's cube.
fn scene_environment(
    dir: vec3<f32>, roughness: f32, zenith: vec4<f32>, horizon: vec3<f32>, ground: vec3<f32>, preset: u32,
    preset_b: u32, blend_t: f32, sun_dir: vec3<f32>,
) -> vec3<f32> {
    if (zenith.a >= 0.0) {
        return sky_gradient(dir, zenith.rgb, horizon, ground, roughness) * zenith.a;
    }
    return preset_environment(dir, roughness, preset, preset_b, blend_t, sun_dir);
}

// The environment presets along `dir`, `blend_t` of the way from `preset` to `preset_b`: what lights
// a surface and what it reflects, for the deferred and the forward shader alike. Pure: `sun_dir` is
// the direction towards the sun.
fn preset_environment(dir: vec3<f32>, roughness: f32, preset: u32, preset_b: u32, blend_t: f32, sun_dir: vec3<f32>) -> vec3<f32> {
    let zenith = dir.y;
    let sun_dot = max(dot(dir, sun_dir), 0.0);

    // Smooth transition factor at the horizon (zenith = 0.0)
    let horizon_blend = smoothstep(0.0, 1.0, clamp(zenith * 10.0 + 0.5, 0.0, 1.0));

    let color1 = single_preset_environment(preset, dir, roughness, sun_dir, sun_dot, zenith, horizon_blend);

    if (blend_t > 0.001) {
        let color2 = single_preset_environment(preset_b, dir, roughness, sun_dir, sun_dot, zenith, horizon_blend);
        return mix(color1, color2, blend_t);
    }

    return color1;
}

// The height fog at a surface: rgb is its colour, a how much of the surface it covers. `fog` and
// `fog_shape` are what `frame_uniforms::frame_fog` settled on for this frame. Every shader that draws
// the world goes through this, so a surface stands in the same fog on any path.
//
// It decides nothing per fragment, on purpose: which fog and which `FogMode` are the same for the whole
// frame, and a phone pays for a branch here whether or not it is taken. Two `if`s for the mode, neither
// of them taken, cost an Adreno 618 filling the screen 6 fps of 62. See `frame_fog`.
//
// Measure before making this smaller. An exponential-only copy for the plain tier, with the camera's
// `exp` worked out once a frame, has strictly less to do and read 85 fps on that phone against this
// function's 103, by two ways of counting and for no reason found.
fn height_fog(world_pos: vec3<f32>, camera_pos: vec3<f32>, fog: vec4<f32>, fog_shape: vec4<f32>) -> vec4<f32> {
    let view_vec = world_pos - camera_pos;
    let dist = length(view_vec);
    let view_dir = view_vec / max(dist, 0.0001);
    let density = fog.a;
    let falloff = fog_shape.x;

    // Volumetric analytical scattering (height-decay fog integration): `(1 - e^(-rate·dist)) / rate`
    // along the ray. Its limit is `dist` whenever the rate is ~0: a level ray, or fog that does not thin
    // with height at all (`Fog::height_falloff == 0`), which the division would turn to NaN.
    let rate = falloff * view_dir.y;
    let level = abs(rate) < 0.000001;
    let along_ray = select((1.0 - exp(-rate * dist)) / select(rate, 1.0, level), dist, level);
    // Past 64 the exponential below is 1 in an f32 anyway, and an infinity would turn to NaN in the `mix`.
    let thickness = min(density * exp(-falloff * (camera_pos.y - fog_shape.y)) * along_ray, 64.0);

    // `FogMode`: `fog_shape.z` is 0, 1 or 2, so each of these is 1 in its own mode and 0 in the others.
    let squared = 1.0 - abs(fog_shape.z - 1.0);
    let linear = max(fog_shape.z - 1.0, 0.0);
    let exponential = 1.0 - clamp(exp(-thickness * mix(1.0, thickness, squared)), 0.0, 1.0);
    let ramp = clamp((dist - fog_shape.w) / max(density - fog_shape.w, 0.001), 0.0, 1.0);
    return vec4<f32>(fog.rgb, mix(exponential, ramp, linear));
}

// ── Clustered light lookup ───────────────────────────────────────────────────
//
// The cluster a world position belongs to, by the SAME mapping `clustered.rs` assigns lights with:
// screen tile from NDC, depth slice from the log of the view depth. A shader that wants a cluster's
// light list also needs the two storage bindings (group 0, bindings 1 and 2) — they are declared by
// the shaders that light, not here, because a pipeline whose group 0 lacks them must still be able
// to compose this module.
//
// It takes the five fields it reads, not `SceneUniforms`. It took the whole struct by value until
// 2026-11-05, and that struct carries `lights: array<LightData, 256>` — 17 040 bytes copied into
// function-local memory for every fragment. Desktop drivers fold the copy away; Adreno 618's
// (driver V@0502, Chrome 156) did not, and the lit forward shader came out miscompiled: every
// value that depends on a fragment input — varyings, even `@builtin(position).z` — was garbage in
// blocks and speckles, while a constant colour drew clean. It did so with the light loop never
// iterating (no point lights in the scene): the loop only has to be compiled. Measured on the
// phone by bisecting `fs_main`: returning before the loop was clean, everything after it was not,
// and passing these fields instead cleared it.
fn gizmo_cluster_index(
    view_proj: mat4x4<f32>,
    camera_pos: vec3<f32>,
    camera_forward: vec3<f32>,
    dims: vec4<u32>,
    cluster_depth: vec4<f32>,
    world_pos: vec3<f32>,
) -> u32 {
    let clip = view_proj * vec4<f32>(world_pos, 1.0);
    // Behind the camera (w <= 0) has no cluster; clamp into the volume rather than wrap.
    let inv_w = select(1.0 / clip.w, 1.0, abs(clip.w) < 1e-9);
    let uv = clamp(clip.xy * inv_w * 0.5 + vec2<f32>(0.5), vec2<f32>(0.0), vec2<f32>(0.999999));
    let tile = vec2<u32>(uv * vec2<f32>(f32(dims.x), f32(dims.y)));

    let depth = max(dot(world_pos - camera_pos, camera_forward), 1e-6);
    let raw = log(depth) * cluster_depth.x + cluster_depth.y;
    let slice = min(u32(max(floor(raw), 0.0)), dims.z - 1u);

    return min(tile.x, dims.x - 1u)
         + min(tile.y, dims.y - 1u) * dims.x
         + slice * dims.x * dims.y;
}

// ── Core BRDF (Cook-Torrance / GGX) ──────────────────────────────────────────

fn D_GGX(NoH: f32, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let denom = NoH * NoH * (a2 - 1.0) + 1.0;
    return a2 / (PI * denom * denom);
}

fn V_SmithJointGGX(NoV: f32, NoL: f32, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let lambdaV = NoL * sqrt(NoV * NoV * (1.0 - a2) + a2);
    let lambdaL = NoV * sqrt(NoL * NoL * (1.0 - a2) + a2);
    return 0.5 / max(lambdaV + lambdaL, 0.0001);
}

// Whether a pixel survives `InstanceRaw.ambient.w`. 0: yes. Positive: the cut-out threshold. Negative:
// a LOD crossfade, `-(n + threshold)`: n up to 255 keeps the share of the pattern below n/256, and
// n + 256 keeps the rest, so the two levels of a band split every pixel between them.
fn instance_keeps(packed: f32, alpha: f32, pixel: vec2<f32>) -> bool {
    if (packed == 0.0) {
        return true;
    }
    var threshold = packed;
    if (packed < 0.0) {
        let whole = floor(-packed);
        threshold = -packed - whole;
        let pattern = fract(52.9829189 * fract(dot(pixel, vec2<f32>(0.06711056, 0.00583715))));
        let below = pattern < whole / 256.0;
        let keep = select(below, pattern >= (whole - 256.0) / 256.0, whole > 256.0);
        if (!keep) {
            return false;
        }
    }
    return alpha >= threshold;
}

// What `InstanceRaw.emissive.w` (and RT5's alpha, its copy) says about a non-metal's F0: the
// magnitude, with 0 standing for "not set". Its sign is `receives_shadows`, tested by the callers.
fn dielectric_f0(packed: f32) -> f32 {
    let m = abs(packed);
    return select(m, 0.04, m == 0.0);
}

fn F_Schlick(VoH: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * pow(clamp(1.0 - VoH, 0.0, 1.0), 5.0);
}

// How much of a point or spot light reaches a surface `distance` metres away, given its `radius` of
// influence: the inverse square, windowed to nothing at the radius by `(1 - (d/r)^4)^2`. `x` is that
// attenuation and `y` the window alone. The deferred lighting pass and the forward shader both come here:
// the forward shader divided by `d² + 1` where the deferred pass divides by `d² + 0.01`, so a light a
// metre from a transparent surface, in the editor viewport, in a browser or on a phone gave half of what
// it gave the G-buffer's — 195.5 against 228.5 on a matte floor, where a 5 cm rectangle light, whose
// polygon integral is this inverse square, read 226.8 on both (2026-11-11).
fn light_falloff(distance: f32, radius: f32) -> vec2<f32> {
    let d_over_r = distance / max(radius, 0.001);
    let w = clamp(1.0 - d_over_r * d_over_r * d_over_r * d_over_r, 0.0, 1.0);
    let window = w * w;
    return vec2<f32>(window / (distance * distance + 0.01), window);
}

// Direct PBR lighting for one light. Diffuse carries the Lambert 1/PI so it is
// energy-consistent with the 1/PI already inside D_GGX.
fn compute_direct_lighting(
    N: vec3<f32>,
    V: vec3<f32>,
    L: vec3<f32>,
    albedo: vec3<f32>,
    roughness: f32,
    metallic: f32,
    f0: vec3<f32>,
    light_color: vec3<f32>,
    intensity: f32,
    atten: f32
) -> vec3<f32> {
    let H = normalize(V + L);
    let NoL = max(dot(N, L), 0.0);
    let NoV = max(dot(N, V), 0.001);
    let NoH = max(dot(N, H), 0.0);
    let VoH = max(dot(V, H), 0.0);

    if (NoL <= 0.0) {
        return vec3<f32>(0.0);
    }

    let D = D_GGX(NoH, roughness);
    let Vis = V_SmithJointGGX(NoV, NoL, roughness);
    let F = F_Schlick(VoH, f0);

    let kS = F;
    let kD = (vec3<f32>(1.0) - kS) * (1.0 - metallic);

    let diffuse = kD * albedo * NoL * INV_PI; // Lambert: albedo / PI
    let specular = D * Vis * F * NoL;

    return (diffuse + specular) * light_color * intensity * atten;
}

// A light with a SPHERICAL source of `source_radius` metres, centred `to_light` away. Diffuse
// uses the centre. Specular uses the representative point (Karis, "Real Shading in Unreal Engine
// 4", 2013): the point on the sphere nearest the reflected ray, so a glossy surface shows a highlight
// as big as the sphere's reflection rather than a pinpoint — with the distribution widened to match
// and renormalised by (α / α')², so a bigger source spreads the same energy rather than adding it.
fn compute_sphere_lighting(
    N: vec3<f32>,
    V: vec3<f32>,
    to_light: vec3<f32>,
    source_radius: f32,
    albedo: vec3<f32>,
    roughness: f32,
    metallic: f32,
    f0: vec3<f32>,
    light_color: vec3<f32>,
    intensity: f32,
    atten: f32
) -> vec3<f32> {
    let dist = max(length(to_light), 1e-4);
    let L = to_light / dist;
    let NoL = max(dot(N, L), 0.0);
    let r = reflect(-V, N);
    let center_to_ray = dot(to_light, r) * r - to_light;
    let closest = to_light + center_to_ray * clamp(source_radius / max(length(center_to_ray), 1e-5), 0.0, 1.0);
    let Ls = normalize(closest);
    let NoLs = max(dot(N, Ls), 0.0);
    if (NoL <= 0.0 && NoLs <= 0.0) {
        return vec3<f32>(0.0);
    }
    let H = normalize(V + Ls);
    let NoV = max(dot(N, V), 0.001);
    let NoH = max(dot(N, H), 0.0);
    let VoH = max(dot(V, H), 0.0);
    let alpha = roughness * roughness;
    let alpha_wide = clamp(alpha + source_radius / (2.0 * dist), 0.0, 1.0);
    let norm = (alpha / max(alpha_wide, 1e-4)) * (alpha / max(alpha_wide, 1e-4));

    let D = D_GGX(NoH, roughness) * norm;
    let Vis = V_SmithJointGGX(NoV, NoLs, roughness);
    let F = F_Schlick(VoH, f0);
    let kD = (vec3<f32>(1.0) - F) * (1.0 - metallic);
    let diffuse = kD * albedo * NoL * INV_PI;
    let specular = D * Vis * F * NoLs;
    return (diffuse + specular) * light_color * intensity * atten;
}

// A TUBE light: a segment `tube_length` metres long through `to_light` (the centre, relative to the
// surface) along `axis`, `source_radius` thick (Karis 2013, "tube lights"). Diffuse averages the
// cosine to the two ends. Specular takes the point on the segment nearest the reflected ray, then
// the sphere step around it, and renormalises the lobe once per widened dimension — (α/α'_line) for
// the length, (α/α'_sphere)² for the thickness — so a longer tube spreads its energy rather than
// adding it.
fn compute_tube_lighting(
    N: vec3<f32>,
    V: vec3<f32>,
    to_light: vec3<f32>,
    axis: vec3<f32>,
    tube_length: f32,
    source_radius: f32,
    albedo: vec3<f32>,
    roughness: f32,
    metallic: f32,
    f0: vec3<f32>,
    light_color: vec3<f32>,
    intensity: f32,
    atten: f32
) -> vec3<f32> {
    let half_axis = axis * (tube_length * 0.5);
    let l0 = to_light - half_axis;
    let l1 = to_light + half_axis;
    let dist = max(length(to_light), 1e-4);
    let NoL = clamp(0.5 * (dot(N, l0) / max(length(l0), 1e-4) + dot(N, l1) / max(length(l1), 1e-4)), 0.0, 1.0);

    let r = reflect(-V, N);
    let ld = l1 - l0;
    let r_ld = dot(r, ld);
    let t = clamp((dot(r, l0) * r_ld - dot(l0, ld)) / max(dot(ld, ld) - r_ld * r_ld, 1e-6), 0.0, 1.0);
    let on_line = l0 + ld * t;
    let center_to_ray = dot(on_line, r) * r - on_line;
    let closest = on_line + center_to_ray * clamp(source_radius / max(length(center_to_ray), 1e-5), 0.0, 1.0);
    let Ls = normalize(closest);
    let NoLs = max(dot(N, Ls), 0.0);
    if (NoL <= 0.0 && NoLs <= 0.0) {
        return vec3<f32>(0.0);
    }
    let H = normalize(V + Ls);
    let NoV = max(dot(N, V), 0.001);
    let NoH = max(dot(N, H), 0.0);
    let VoH = max(dot(V, H), 0.0);
    let alpha = roughness * roughness;
    let alpha_line = clamp(alpha + tube_length / (3.0 * dist), 0.0, 1.0);
    let alpha_sphere = clamp(alpha + source_radius / (2.0 * dist), 0.0, 1.0);
    let norm = (alpha / max(alpha_line, 1e-4)) * (alpha / max(alpha_sphere, 1e-4)) * (alpha / max(alpha_sphere, 1e-4));

    let D = D_GGX(NoH, roughness) * norm;
    let Vis = V_SmithJointGGX(NoV, NoLs, roughness);
    let F = F_Schlick(VoH, f0);
    let kD = (vec3<f32>(1.0) - F) * (1.0 - metallic);
    let diffuse = kD * albedo * NoL * INV_PI;
    let specular = D * Vis * F * NoLs;
    return (diffuse + specular) * light_color * intensity * atten;
}

// Where `world_pos` falls on a spot light's cookie (`gizmo_renderer::cookies`): the outer cone
// fills [0,1]², row 0 at the top. The cone's roll is not packed, so the image's up is world up as
// seen down the cone — world −Z for a cone pointing straight up or down.
fn spot_cookie_uv(light: LightData, world_pos: vec3<f32>) -> vec2<f32> {
    let d = normalize(light.direction.xyz);
    let up_ref = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(0.0, 0.0, -1.0), abs(d.y) > 0.99);
    let right = normalize(cross(d, up_ref));
    let up = cross(right, d);
    let v = world_pos - light.position.xyz;
    let z = max(dot(v, d), 1e-4);
    let c = clamp(light.params.x, 1e-4, 0.9999);
    let tan_outer = sqrt(1.0 - c * c) / c;
    let p = vec2<f32>(dot(v, right), dot(v, up)) / (z * tan_outer);
    return vec2<f32>(0.5 + 0.5 * p.x, 0.5 - 0.5 * p.y);
}

// `v` turned by the unit quaternion `q` (xyzw).
fn rotate_by_quat(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    let t = 2.0 * cross(q.xyz, v);
    return v + q.w * t + cross(q.xyz, t);
}

// One edge's share of the vector form factor of a polygon: θ · n̂, with θ the angle between the two
// corner directions and n̂ the normal of the plane through them. `atan2` rather than `acos` keeps
// it stable for the near-parallel edges a distant light has; θ / sin θ → 1 as they close.
fn rect_edge(v1: vec3<f32>, v2: vec3<f32>) -> vec3<f32> {
    let c = cross(v1, v2);
    let s = length(c);
    return c * (atan2(s, dot(v1, v2)) / max(s, 1e-7));
}

// A one-sided rectangular emitter (`RectLight`): centre `to_light` from the surface, oriented by the
// quaternion `orientation`, half extents `half_w` along its local X and `half_h` along its local Y,
// emitting along its local −Z. `window` is the light's range falloff alone — the inverse square is
// in the integral below, not in it.
//
// Diffuse is exact for a Lambertian receiver, before horizon clipping: the cosine-weighted solid
// angle of the rectangle is ½·Σ θ_e n̂_e · N over its edges (Lambert's polygon formula). `intensity`
// is the light's on-axis radiant intensity — the rectangle's radiance is intensity / area — so from
// far away, seen face on, it lights like a point light of the same intensity.
//
// Specular takes the representative point: where the reflected ray meets the light's plane,
// clamped into the rectangle, with the lobe widened across each extent and renormalised so the
// highlight spreads its energy instead of adding to it — the tube light's line step, twice.
fn compute_rect_lighting(
    N: vec3<f32>,
    V: vec3<f32>,
    to_light: vec3<f32>,
    orientation: vec4<f32>,
    half_w: f32,
    half_h: f32,
    albedo: vec3<f32>,
    roughness: f32,
    metallic: f32,
    f0: vec3<f32>,
    light_color: vec3<f32>,
    intensity: f32,
    window: f32
) -> vec3<f32> {
    let right = rotate_by_quat(orientation, vec3<f32>(1.0, 0.0, 0.0));
    let up = rotate_by_quat(orientation, vec3<f32>(0.0, 1.0, 0.0));
    let emit = -normalize(cross(right, up));
    let dist = max(length(to_light), 1e-4);
    // Behind the emitting face: nothing.
    let facing = dot(-to_light, emit) / dist;
    if (facing <= 0.0) {
        return vec3<f32>(0.0);
    }

    let a = right * half_w;
    let b = up * half_h;
    let p0 = normalize(to_light - a - b);
    let p1 = normalize(to_light + a - b);
    let p2 = normalize(to_light + a + b);
    let p3 = normalize(to_light - a + b);
    var form = 0.5 * (rect_edge(p0, p1) + rect_edge(p1, p2) + rect_edge(p2, p3) + rect_edge(p3, p0));
    // The winding's sign depends on which side the surface sees; the vector form factor points at
    // the light either way.
    if (dot(form, to_light) < 0.0) {
        form = -form;
    }
    let diffuse_integral = max(dot(form, N), 0.0);

    let r = reflect(-V, N);
    let denom = dot(r, emit);
    var on_plane: vec3<f32>;
    let t = dot(to_light, emit) / select(denom, -1e-6, abs(denom) < 1e-6);
    if (denom < -1e-6 && t > 0.0) {
        on_plane = r * t;
    } else {
        // The ray never meets the face: walk its in-plane direction far out and clamp back.
        on_plane = to_light + (r - emit * denom) * 1e4;
    }
    let local = on_plane - to_light;
    let closest = to_light + right * clamp(dot(local, right), -half_w, half_w)
                           + up * clamp(dot(local, up), -half_h, half_h);
    let Ls = normalize(closest);
    let NoLs = max(dot(N, Ls), 0.0);
    if (diffuse_integral <= 0.0 && NoLs <= 0.0) {
        return vec3<f32>(0.0);
    }

    let H = normalize(V + Ls);
    let NoV = max(dot(N, V), 0.001);
    let NoH = max(dot(N, H), 0.0);
    let VoH = max(dot(V, H), 0.0);
    let alpha = roughness * roughness;
    let alpha_w = clamp(alpha + (2.0 * half_w) / (3.0 * dist), 0.0, 1.0);
    let alpha_h = clamp(alpha + (2.0 * half_h) / (3.0 * dist), 0.0, 1.0);
    let norm = (alpha / max(alpha_w, 1e-4)) * (alpha / max(alpha_h, 1e-4));

    let D = D_GGX(NoH, roughness) * norm;
    let Vis = V_SmithJointGGX(NoV, NoLs, roughness);
    let F = F_Schlick(VoH, f0);
    let kD = (vec3<f32>(1.0) - F) * (1.0 - metallic);
    let area = max(4.0 * half_w * half_h, 1e-6);
    let diffuse = kD * albedo * INV_PI * (diffuse_integral / area);
    let specular = D * Vis * F * NoLs * facing / (dist * dist + 0.01);
    return (diffuse + specular) * light_color * intensity * window;
}

// Full 4x4 inverse (used for NDC → world unprojection in fullscreen passes).
fn inverse_mat4(m: mat4x4<f32>) -> mat4x4<f32> {
    let n11 = m[0][0]; let n12 = m[1][0]; let n13 = m[2][0]; let n14 = m[3][0];
    let n21 = m[0][1]; let n22 = m[1][1]; let n23 = m[2][1]; let n24 = m[3][1];
    let n31 = m[0][2]; let n32 = m[1][2]; let n33 = m[2][2]; let n34 = m[3][2];
    let n41 = m[0][3]; let n42 = m[1][3]; let n43 = m[2][3]; let n44 = m[3][3];

    let t11 = n23 * n34 * n42 - n24 * n33 * n42 + n24 * n32 * n43 - n22 * n34 * n43 - n23 * n32 * n44 + n22 * n33 * n44;
    let t12 = n14 * n33 * n42 - n13 * n34 * n42 - n14 * n32 * n43 + n12 * n34 * n43 + n13 * n32 * n44 - n12 * n33 * n44;
    let t13 = n13 * n24 * n42 - n14 * n23 * n42 + n14 * n22 * n43 - n12 * n24 * n43 - n13 * n22 * n44 + n12 * n23 * n44;
    let t14 = n14 * n23 * n32 - n13 * n24 * n32 - n14 * n22 * n33 + n12 * n24 * n33 + n13 * n22 * n34 - n12 * n23 * n34;

    let det = n11 * t11 + n21 * t12 + n31 * t13 + n41 * t14;

    if (abs(det) < 1e-6) {
        return mat4x4<f32>(
            vec4<f32>(1.0, 0.0, 0.0, 0.0),
            vec4<f32>(0.0, 1.0, 0.0, 0.0),
            vec4<f32>(0.0, 0.0, 1.0, 0.0),
            vec4<f32>(0.0, 0.0, 0.0, 1.0)
        );
    }

    let idet = 1.0 / det;

    let t21 = n24 * n33 * n41 - n24 * n31 * n43 - n23 * n34 * n41 + n21 * n34 * n43 + n23 * n31 * n44 - n21 * n33 * n44;
    let t22 = n13 * n34 * n41 - n14 * n33 * n41 + n14 * n31 * n43 - n11 * n34 * n43 - n13 * n31 * n44 + n11 * n33 * n44;
    let t23 = n14 * n23 * n41 - n13 * n24 * n41 - n14 * n21 * n43 + n11 * n24 * n43 + n13 * n21 * n44 - n11 * n23 * n44;
    let t24 = n13 * n24 * n31 - n14 * n23 * n31 + n14 * n21 * n33 - n11 * n24 * n33 - n13 * n21 * n34 + n11 * n23 * n34;

    let t31 = n22 * n34 * n41 - n24 * n32 * n41 + n24 * n31 * n42 - n21 * n34 * n42 - n22 * n31 * n44 + n21 * n32 * n44;
    let t32 = n14 * n32 * n41 - n12 * n34 * n41 - n14 * n31 * n42 + n11 * n34 * n42 + n12 * n31 * n44 - n11 * n32 * n44;
    let t33 = n12 * n24 * n41 - n14 * n22 * n41 + n14 * n21 * n42 - n11 * n24 * n42 - n12 * n21 * n44 + n11 * n22 * n44;
    let t34 = n14 * n22 * n31 - n12 * n24 * n31 - n14 * n21 * n32 + n11 * n24 * n32 + n12 * n21 * n34 - n11 * n22 * n34;

    let t41 = n23 * n32 * n41 - n22 * n33 * n41 - n23 * n31 * n42 + n21 * n33 * n42 + n22 * n31 * n43 - n21 * n32 * n43;
    let t42 = n12 * n33 * n41 - n13 * n32 * n41 + n13 * n31 * n42 - n11 * n33 * n42 - n12 * n31 * n43 + n11 * n32 * n43;
    let t43 = n13 * n22 * n41 - n12 * n23 * n41 - n13 * n21 * n42 + n11 * n23 * n42 + n12 * n21 * n43 - n11 * n22 * n43;
    let t44 = n12 * n23 * n31 - n13 * n22 * n31 + n13 * n21 * n32 - n11 * n23 * n32 - n12 * n21 * n33 + n11 * n22 * n33;

    return mat4x4<f32>(
        vec4<f32>(t11 * idet, t21 * idet, t31 * idet, t41 * idet),
        vec4<f32>(t12 * idet, t22 * idet, t32 * idet, t42 * idet),
        vec4<f32>(t13 * idet, t23 * idet, t33 * idet, t43 * idet),
        vec4<f32>(t14 * idet, t24 * idet, t34 * idet, t44 * idet)
    );
}
