// Volumetric Lighting Shader (God Rays)
// SceneUniforms and LightData come from gizmo::common (composed in by load_shader_composed).
// Only volumetric-specific helpers stay local. The NDC→world unprojection reads the
// CPU-computed scene.inv_view_proj instead of inverting view_proj per fragment.
#import gizmo::common::{SceneUniforms, LightData, rotate_by_quat, spot_cone, spot_cookie_uv, local_shadow_layer, local_shadow_in_air, gizmo_cluster_index}

@group(0) @binding(0) var<uniform> scene: SceneUniforms;
// The clustered light lists (`gizmo_renderer::clustered`), on the global group as the lit passes have them:
// a point of the air takes the lights of its cluster, not every light of the frame.
@group(0) @binding(1) var<storage, read> cluster_table: array<vec2<u32>>;
@group(0) @binding(2) var<storage, read> cluster_light_indices: array<u32>;

@group(1) @binding(0) var t_shadow: texture_depth_2d_array;
@group(1) @binding(1) var s_shadow: sampler_comparison;
// The local lights' shadow layers and the spots' cookies (`gizmo_renderer::{local_shadows, cookies}`):
// the shadow group the lit passes take, so a light's air is shadowed and patterned as its surfaces are.
@group(1) @binding(2) var t_local_shadow: texture_depth_2d_array;
@group(1) @binding(3) var t_cookie: texture_2d_array<f32>;
@group(1) @binding(4) var s_cookie: sampler;

// The frame's depth, where each ray stops: the nearest opaque surface, drawn deferred or forward. The
// G-buffer stood here, which an unlit or baked-lit surface does not write, so a ray marched on through
// an opaque wall drawn forward, and the air behind it was added over the wall.
@group(2) @binding(0) var t_depth: texture_depth_2d;
@group(2) @binding(1) var s_linear: sampler;

// The six numbers that used to be literals in this file. See `VolumetricParams` on the Rust side;
// the defaults there are exactly the values that stood here, so nothing changed by being moved.
struct VolumetricParams {
    phase_g: f32,
    steps: f32,
    max_distance: f32,
    sun_scatter: f32,
    bulb_scatter: f32,
    shadow_bias: f32,
    _pad: vec2<f32>,
}
@group(2) @binding(2) var<uniform> params: VolumetricParams;
// The depth of the nearest surface that blends — glass, a transparent surface — drawn alone, depth
// only, ahead of them (`VolumetricState::veil_depth_view`); 1 where there is none. The air behind it is
// written to `behind` and added ahead of what blends, which shows it as much as it lets through; the
// air in front to `front`, added over everything. Added over everything, all of it, a pane 98 %
// opaque let through the whole of the air behind it.
@group(2) @binding(3) var t_veil_depth: texture_depth_2d;

struct MarchOut {
    @location(0) front: vec4<f32>,
    @location(1) behind: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) screen_uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VertexOutput {
    var pos = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    var uv = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(2.0, 1.0),
        vec2<f32>(0.0, -1.0),
    );
    var out: VertexOutput;
    out.position = vec4<f32>(pos[vi], 0.0, 1.0);
    out.screen_uv = uv[vi];
    return out;
}

fn select_cascade(view_depth: f32) -> u32 {
    if (view_depth < scene.cascade_splits.x) { return 0u; }
    if (view_depth < scene.cascade_splits.y) { return 1u; }
    if (view_depth < scene.cascade_splits.z) { return 2u; }
    return 3u;
}

// Pseudo-random number generator
fn rand(co: vec2<f32>) -> f32 {
    return fract(sin(dot(co.xy ,vec2(12.9898,78.233))) * 43758.5453);
}

// Henyey-Greenstein phase function for forward scattering
// A local light's air over the stretch `x`–`y` of a ray from `origin` along `ray_dir`, the light
// `off_line` (b) from the ray's line and nearest it at `along`. In the angle the stretch spans from the
// light, `φ = atan(u / b)`, the inverse square is flat — `du / (b² + u²) = dφ / b` — and what is left
// (`local_air`: the window, the phase, a spot's cone and cookie, the shadow) turns smoothly; so the
// stretch's air is `(1/b) ∫ local_air dφ`, taken by three-point Gauss–Legendre over the angle — two points
// were 11 % out over a 6 m stretch passing a lamp, where the phase falls from forward scattering in front
// of it (0.61) to back scattering behind (0.02). The same on every frame and every pixel. A single point drawn at random in the angle (equiangular sampling) was
// unbiased, but left each pixel's halo a grain of its own, which TAA could average only while the camera
// stood still: driving, the lamps were dust.
fn stretch_air(light: LightData, j: u32, kind: u32, x: f32, y: f32, along: f32, off_line: f32, origin: vec3<f32>, ray_dir: vec3<f32>) -> vec3<f32> {
    let first = atan((x - along) / off_line);
    let last = atan((y - along) / off_line);
    let middle = 0.5 * (first + last);
    let half_width = 0.5 * (last - first);
    let side = half_width * 0.7745967;
    let centre = local_air(light, j, kind, origin + ray_dir * (along + off_line * tan(middle)), ray_dir);
    let low = local_air(light, j, kind, origin + ray_dir * (along + off_line * tan(middle - side)), ray_dir);
    let high = local_air(light, j, kind, origin + ray_dir * (along + off_line * tan(middle + side)), ray_dir);
    return half_width / off_line * (0.8888889 * centre + 0.5555556 * (low + high));
}

// What of a local light reaches the point `p` of the air and is scattered along `ray_dir` towards the
// camera, short of the inverse square (which `stretch_air` integrates) and the scattering coefficient:
// its colour and intensity, the window at its radius, the phase, on a rectangle's lit side only, inside a
// spot's cone and through its cookie, and past the light's shadow when it casts one.
fn local_air(light: LightData, j: u32, kind: u32, p: vec3<f32>, ray_dir: vec3<f32>) -> vec3<f32> {
    let radius = light.color.a;
    let from_light = p - light.position.xyz;
    let dist = length(from_light);
    if (dist >= radius) {
        return vec3<f32>(0.0);
    }
    // A rectangle light emits from one face: the air behind it stays dark.
    if (kind == 3u && dot(from_light, rotate_by_quat(light.direction, vec3<f32>(0.0, 0.0, -1.0))) <= 0.0) {
        return vec3<f32>(0.0);
    }
    let d_over_r = dist / radius;
    let w = clamp(1.0 - d_over_r * d_over_r * d_over_r * d_over_r, 0.0, 1.0);
    let toward = from_light / max(dist, 1e-6);
    var reach = w * w * phase_function(params.phase_g, dot(ray_dir, -toward));
    var colour = light.color.rgb;
    if (kind == 1u) {
        reach *= spot_cone(light, toward);
        if (reach > 0.0 && light.params.w > 0.0) {
            colour *= textureSampleLevel(t_cookie, s_cookie, spot_cookie_uv(light, p), i32(light.params.w) - 1, 0.0).rgb;
        }
    }
    if (reach <= 0.0) {
        return vec3<f32>(0.0);
    }
    let layer = local_shadow_layer(scene.light_shadows[j / 4u][j % 4u], from_light);
    if (layer >= 0) {
        reach *= local_shadow_in_air(
            t_local_shadow, s_shadow, scene.local_shadow_layers[layer], layer, p, params.shadow_bias * scene.shadow_bias.x,
        );
    }
    return colour * light.position.w * reach;
}

fn phase_function(g: f32, cos_theta: f32) -> f32 {
    let g2 = g * g;
    let num = 1.0 - g2;
    let denom = pow(1.0 + g2 - 2.0 * g * cos_theta, 1.5);
    return (1.0 / (4.0 * 3.14159)) * (num / max(denom, 0.001));
}

@fragment
fn fs_main(in: VertexOutput) -> MarchOut {
    let tex_size = textureDimensions(t_depth);
    let iuv = vec2<i32>(in.screen_uv * vec2<f32>(tex_size));
    let depth = textureLoad(t_depth, iuv, 0);
    let cam_pos = scene.camera_pos.xyz;
    
    // ── Ray Direction Reconstruction ──
    // Unproject the pixel through the inverse view-projection instead of assuming a
    // fixed 1280x720 / 45°-FOV camera. The old hardcoded aspect + fov_factor produced
    // the wrong world ray on any other resolution or FOV, throwing the sky god-rays off
    // in the wrong direction — exactly where they are most visible.
    let ndc = vec2<f32>(in.screen_uv.x * 2.0 - 1.0, 1.0 - in.screen_uv.y * 2.0);
    let inv_vp = scene.inv_view_proj;
    let near_h = inv_vp * vec4<f32>(ndc, 0.0, 1.0);
    let near_world = near_h.xyz / near_h.w;
    let ray_dir_reconstructed = normalize(near_world - cam_pos);

    var ray_dir = ray_dir_reconstructed;
    var max_dist = params.max_distance; // Far clip for skybox god rays

    // The surface the ray stops at, from the depth at the centre of the pixel it reads.
    if (depth < 1.0) {
        let uv = (vec2<f32>(iuv) + 0.5) / vec2<f32>(tex_size);
        let h = inv_vp * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
        let diff = h.xyz / h.w - cam_pos;
        max_dist = min(length(diff), params.max_distance);
        ray_dir = normalize(diff);
    }
    
    var out: MarchOut;
    out.front = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    out.behind = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    if (max_dist < 0.1) {
        return out;
    }

    // How far along the view the nearest surface that blends stands; past the march's end where none.
    let fwd = scene.camera_forward.xyz;
    var veil = 3.0e38;
    let veil_d = textureLoad(t_veil_depth, iuv, 0);
    if (veil_d < 1.0) {
        let uv = (vec2<f32>(iuv) + 0.5) / vec2<f32>(tex_size);
        let h = inv_vp * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, veil_d, 1.0);
        veil = dot(h.xyz / h.w - cam_pos, fwd);
    }
    
    let steps = max(params.steps, 1.0);
    let step_size = max_dist / steps;
    
    // Jitter the first sample to trade banding for noise (smoothed by TAA and bloom). The samples
    // are `jitter + k·step` for k = 0 … steps−1, all short of `max_dist`: they were stepped before
    // each was taken, so the last landed up to a step past the surface the ray ends on — inside it,
    // where the light it shadows is dark.
    let jitter = rand(in.position.xy) * step_size;
    var current_pos = cam_pos + ray_dir * (jitter - step_size);
    
    var front = vec3<f32>(0.0);
    var behind = vec3<f32>(0.0);
    // The veil along the ray (`veil` is a view depth): where a local light's stretch of air is cut in two.
    let veil_along = veil / max(dot(ray_dir, fwd), 1e-4);
    // Where a shadow-casting local light's air ends: the ray's end, short of a surface by the shadow's bias.
    let local_end = select(max_dist, max_dist - params.shadow_bias * scene.shadow_bias.x, depth < 1.0);
    
    // Sun Directional Light Precalculations
    let L_sun = normalize(-scene.sun_direction.xyz);
    let cos_theta = dot(ray_dir, L_sun);
    let sun_phase = phase_function(params.phase_g, cos_theta);
    let sun_intensity = scene.sun_color.rgb * scene.sun_color.w * params.sun_scatter;
    
    for (var i = 0.0; i < steps; i += 1.0) {
        current_pos += ray_dir * step_size;
        var total_scatter = vec3<f32>(0.0);
        
        // ── 1. Sun God Rays (Directional Cascade Shadows) ──
        // Not at all when the sun scatters nothing: a cascade lookup a step for no light (a night whose
        // sun is there for a car's paint, not for its air).
        if (scene.sun_direction.w > 0.5 && params.sun_scatter > 0.0) {
            let view_depth = dot(current_pos - cam_pos, scene.camera_forward.xyz);
            let ci = select_cascade(view_depth);
            
            let m = scene.light_view_proj[ci];
            let light_clip = m * vec4<f32>(current_pos, 1.0);
            let light_ndc = light_clip.xyz / light_clip.w;
            let shadow_uv = vec2<f32>(light_ndc.x * 0.5 + 0.5, light_ndc.y * -0.5 + 0.5);
            
            if (shadow_uv.x >= 0.0 && shadow_uv.x <= 1.0 && shadow_uv.y >= 0.0 && shadow_uv.y <= 1.0 && light_ndc.z <= 1.0) {
                // Metres, not NDC — the fourth and last path that sampled the cascades with a
                // flat constant. See `baked_lit.wgsl` for the reasoning and what widening
                // `CASTER_REACH` did to it: the same 0.0015 went from 0.16 m of world depth to
                // 0.82 m. Ray-marched scattering is more forgiving than a surface shadow, but a
                // bias that changes meaning when an unrelated constant moves is not something to
                // leave in four places and fix in three.
                let sz = length(vec3<f32>(m[0][2], m[1][2], m[2][2]));
                // `ShadowSettings::depth_bias` scales it, as it scales every lookup's (it was left out here).
                let bias = params.shadow_bias * sz * scene.shadow_bias.x;
                let shadow_val = textureSampleCompareLevel(t_shadow, s_shadow, shadow_uv, ci, light_ndc.z - bias);
                
                if (shadow_val > 0.0) {
                    total_scatter += sun_intensity * sun_phase * shadow_val * step_size;
                }
            }
        }
        
        // ── 2. Local lights: what of each reaches this point of the air ──
        //
        // As a surface here would take it: inside the light's radius, on the side a rectangle
        // emits from, inside a spot's cone and through its cookie, and past the light's shadow when
        // it casts one. Without the cone a spot's air glowed all round it, behind it included; without
        // the shadow a spot through a doorway lit the air behind the wall as brightly as in front.
        //
        // Only the lights of this point's cluster: a light reaches a point of the air only inside its
        // radius, and the clusters list every light whose radius touches them, in the order of the frame's
        // array — so the sum is the one the whole array gave, without walking 256 street lamps at every
        // step of every pixel.
        let cluster = gizmo_cluster_index(scene.view_proj, scene.camera_pos.xyz, scene.camera_forward.xyz, scene.cluster_dims, scene.cluster_depth, current_pos);
        let cluster_range = cluster_table[cluster];
        for (var ci = 0u; ci < cluster_range.y; ci += 1u) {
            let j = cluster_light_indices[cluster_range.x + ci];
            let light = scene.lights[j];
            let kind = u32(light.params.y);
            // A directional light other than the sun — a fill, a rim — scatters as the sun does,
            // through the phase function, without the sun's shadow. Its radius is infinite and its
            // position means nothing, so the bulb's sphere below is not its shape.
            if (kind == 2u) {
                let toward = normalize(-light.direction.xyz);
                total_scatter += light.color.rgb * light.position.w * params.sun_scatter
                    * phase_function(params.phase_g, dot(ray_dir, toward)) * step_size;
                continue;
            }
            // This sample's stretch of the ray: the k-th step, `[k·step, (k+1)·step]`, which the jittered
            // sample lies in — so the stretches tile the ray from the camera to where it stops, with no gap
            // and nothing past the surface. Cut at the veil: what is in front of it is the front's.
            //
            // Over a stretch the light's air is integrated in the angle the stretch spans from it
            // (`stretch_air`): the inverse square exactly, the rest by quadrature, the same on every frame —
            // a ray passing a bulb gathers its halo whatever its step. Sampled at the step, one step carried
            // the whole halo and the next none, and a sky ray's 6 m steps missed it outright. For a light
            // that casts a shadow, short of the surface the ray ends on by the shadow's bias: the air there
            // is what its shadow cannot tell from the surface, and a lamp behind a wall lit it as if the wall
            // were not there. It used to be `(1 - d/r)²` at the sample, with no inverse square and no phase:
            // inside its radius a lamp lit the air nearly as brightly a radius off as at the bulb, and a
            // street of them filled the frame with fog (`nfsu2-gizmo`'s 2,233 lamps, 2026-11-20).
            let along = dot(light.position.xyz - cam_pos, ray_dir);
            // The light's distance from the ray's line, with the inverse square's 10 cm core (`light_falloff`
            // divides by `d² + 0.01`): a ray from a camera standing in the light — a reflection probe at a
            // lamp — gathers what the surfaces round it are lit by, not an infinite halo.
            let off_line = sqrt(max(dot(light.position.xyz - cam_pos, light.position.xyz - cam_pos) - along * along, 0.0) + 0.01);
            let start = dot(current_pos - cam_pos, ray_dir) - jitter;
            let casts = local_shadow_layer(scene.light_shadows[j / 4u][j % 4u], vec3<f32>(1.0, 0.0, 0.0)) >= 0;
            let end = min(start + step_size, select(max_dist, local_end, casts));
            let cut = clamp(veil_along, start, max(end, start));
            // The light's sphere misses this stretch (its nearest point is past the radius): nothing to
            // draw. The cluster lists every light whose sphere touches its cell, most of which a stretch of
            // one ray passes by.
            let nearest = clamp(along, start, max(end, start)) - along;
            if (off_line * off_line + nearest * nearest >= light.color.a * light.color.a) {
                continue;
            }
            if (cut > start) {
                front += params.bulb_scatter * stretch_air(light, j, kind, start, cut, along, off_line, cam_pos, ray_dir);
            }
            if (end > cut) {
                behind += params.bulb_scatter * stretch_air(light, j, kind, cut, end, along, off_line, cam_pos, ray_dir);
            }
        }
        if (dot(current_pos - cam_pos, fwd) < veil) {
            front += total_scatter;
        } else {
            behind += total_scatter;
        }
    }
    
    out.front = vec4<f32>(front, 1.0);
    out.behind = vec4<f32>(behind, 1.0);
    return out;
}
