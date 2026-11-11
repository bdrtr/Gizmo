// Volumetric Lighting Shader (God Rays)
// SceneUniforms and LightData come from gizmo::common (composed in by load_shader_composed).
// Only volumetric-specific helpers stay local. The NDC→world unprojection reads the
// CPU-computed scene.inv_view_proj instead of inverting view_proj per fragment.
#import gizmo::common::{SceneUniforms, LightData, rotate_by_quat, spot_cone, spot_cookie_uv, local_shadow_layer, local_shadow_in_air}

@group(0) @binding(0) var<uniform> scene: SceneUniforms;

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
    
    // Sun Directional Light Precalculations
    let L_sun = normalize(-scene.sun_direction.xyz);
    let cos_theta = dot(ray_dir, L_sun);
    let sun_phase = phase_function(params.phase_g, cos_theta);
    let sun_intensity = scene.sun_color.rgb * scene.sun_color.w * params.sun_scatter;
    
    for (var i = 0.0; i < steps; i += 1.0) {
        current_pos += ray_dir * step_size;
        var total_scatter = vec3<f32>(0.0);
        
        // ── 1. Sun God Rays (Directional Cascade Shadows) ──
        if (scene.sun_direction.w > 0.5) {
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
        for (var j = 0u; j < scene.num_lights; j += 1u) {
            let light = scene.lights[j];
            let kind = u32(light.params.y);
            let radius = light.color.a;
            let from_light = current_pos - light.position.xyz;
            let dist = length(from_light);
            if (dist >= radius) {
                continue;
            }
            // A rectangle light emits from one face: the air behind it stays dark.
            if (kind == 3u && dot(from_light, rotate_by_quat(light.direction, vec3<f32>(0.0, 0.0, -1.0))) <= 0.0) {
                continue;
            }
            let near = clamp(1.0 - dist / radius, 0.0, 1.0);
            var reach = near * near;
            var colour = light.color.rgb;
            if (kind == 1u) {
                reach *= spot_cone(light, from_light / max(dist, 1e-6));
                if (reach > 0.0 && light.params.w > 0.0) {
                    colour *= textureSampleLevel(
                        t_cookie, s_cookie, spot_cookie_uv(light, current_pos), i32(light.params.w) - 1, 0.0
                    ).rgb;
                }
            }
            if (reach <= 0.0) {
                continue;
            }
            let layer = local_shadow_layer(scene.light_shadows[j / 4u][j % 4u], from_light);
            if (layer >= 0) {
                reach *= local_shadow_in_air(
                    t_local_shadow, s_shadow, scene.local_shadow_layers[layer], layer, current_pos,
                    params.shadow_bias * scene.shadow_bias.x,
                );
            }
            total_scatter += colour * light.position.w * params.bulb_scatter * reach * step_size;
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
