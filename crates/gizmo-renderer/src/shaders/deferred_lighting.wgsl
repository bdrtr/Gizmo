// Deferred lighting pass — fullscreen triangle.
// Reads G-buffers, reconstructs surface data, computes PBR + CSM shadows → HDR output.

struct LightData {
    position:  vec4<f32>,
    color:     vec4<f32>,
    direction: vec4<f32>,
    params:    vec4<f32>,
};

struct SceneUniforms {
    view_proj:       mat4x4<f32>,
    camera_pos:      vec4<f32>,
    sun_direction:   vec4<f32>,
    sun_color:       vec4<f32>,
    lights:          array<LightData, 10>,
    light_view_proj: array<mat4x4<f32>, 4>,
    cascade_splits:  vec4<f32>,
    camera_forward:  vec4<f32>,
    cascade_params:  vec4<f32>,
    num_lights: u32,
    _pad: vec3<u32>,
};

@group(0) @binding(0) var<uniform> scene: SceneUniforms;

@group(1) @binding(0) var t_shadow: texture_depth_2d_array;
@group(1) @binding(1) var s_shadow: sampler_comparison;

@group(2) @binding(0) var t_albedo_metallic:  texture_2d<f32>;
@group(2) @binding(1) var t_normal_roughness: texture_2d<f32>;
@group(2) @binding(2) var t_world_position:   texture_2d<f32>;
@group(2) @binding(3) var s_gbuf: sampler;

// Fullscreen triangle — no vertex buffer needed.
@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    var pos = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    return vec4<f32>(pos[vi], 0.0, 1.0);
}

fn select_cascade(view_depth: f32) -> u32 {
    if (view_depth < scene.cascade_splits.x) { return 0u; }
    if (view_depth < scene.cascade_splits.y) { return 1u; }
    if (view_depth < scene.cascade_splits.z) { return 2u; }
    return 3u;
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = frag_coord.xy;
    let iuv = vec2<i32>(i32(uv.x), i32(uv.y));

    let albedo_metallic  = textureLoad(t_albedo_metallic,  iuv, 0);
    let normal_roughness = textureLoad(t_normal_roughness, iuv, 0);
    let pos_sample       = textureLoad(t_world_position,   iuv, 0);

    // Unwritten pixels (skipped geometry, unlit objects) — output black, will be overwritten
    if (pos_sample.w < 0.5) { return vec4<f32>(0.0, 0.0, 0.0, 0.0); }

    let albedo    = albedo_metallic.rgb;
    let metallic  = albedo_metallic.a;
    let N         = normalize(normal_roughness.xyz);
    let roughness = normal_roughness.a;
    let world_pos = pos_sample.xyz;

    let min_roughness = max(roughness, 0.05);
    let shininess     = 2.0 / (min_roughness * min_roughness) - 2.0;
    let view_dir      = normalize(scene.camera_pos.xyz - world_pos);
    let f0            = mix(vec3<f32>(0.04), albedo, metallic);

    // --- Hemispherical ambient ---
    let sky_ambient    = vec3<f32>(0.8, 0.5, 0.4) * 0.7;
    let ground_ambient = vec3<f32>(0.15, 0.1, 0.15);
    let hemi_mix       = N.y * 0.5 + 0.5;
    let ambient        = albedo * mix(ground_ambient, sky_ambient, hemi_mix);

    // --- Fake IBL specular ---
    let R           = reflect(-view_dir, N);
    let reflect_mix = clamp(R.y * 0.5 + 0.5, 0.0, 1.0);
    let fake_env    = mix(ground_ambient, vec3<f32>(1.0, 0.6, 0.4), reflect_mix);
    let fake_ibl    = f0 * fake_env * ((1.0 - min_roughness) * (1.0 - min_roughness) * 2.0);

    // --- CSM Shadow ---
    var shadow_visibility = 1.0;
    if (scene.sun_direction.w > 0.5) {
        let view_depth = dot(world_pos - scene.camera_pos.xyz, scene.camera_forward.xyz);
        let ci         = select_cascade(view_depth);
        let light_clip = scene.light_view_proj[ci] * vec4<f32>(world_pos, 1.0);
        let light_ndc  = light_clip.xyz / light_clip.w;
        let shadow_uv  = vec2<f32>(light_ndc.x * 0.5 + 0.5, light_ndc.y * -0.5 + 0.5);

        if (shadow_uv.x >= 0.0 && shadow_uv.x <= 1.0 &&
            shadow_uv.y >= 0.0 && shadow_uv.y <= 1.0 && light_ndc.z <= 1.0) {
            let slope  = 1.0 - max(dot(N, normalize(-scene.sun_direction.xyz)), 0.0);
            let bias   = max(0.005 * slope, 0.001);
            let texel  = scene.cascade_params.y;
            var pcf    = 0.0;
            for (var x = -1; x <= 1; x++) {
                for (var y = -1; y <= 1; y++) {
                    let off = vec2<f32>(f32(x), f32(y)) * texel;
                    pcf += textureSampleCompare(t_shadow, s_shadow, shadow_uv + off, ci, light_ndc.z - bias);
                }
            }
            shadow_visibility = pcf / 9.0;
        }
    }

    var total_diffuse  = vec3<f32>(0.0);
    var total_specular = vec3<f32>(0.0);

    // --- Directional Sun ---
    if (scene.sun_direction.w > 0.5) {
        let L        = normalize(-scene.sun_direction.xyz);
        let diff     = max(dot(N, L), 0.0);
        let spec     = pow(max(dot(view_dir, reflect(-L, N)), 0.0), shininess);
        let intensity = scene.sun_color.w;
        total_diffuse  += albedo * (1.0 - metallic) * diff * scene.sun_color.rgb * intensity * shadow_visibility;
        total_specular += f0 * spec * (1.0 - min_roughness) * scene.sun_color.rgb * intensity * shadow_visibility;
    }

    // --- Dynamic Lights ---
    for (var i = 0u; i < scene.num_lights; i++) {
        let light      = scene.lights[i];
        let light_type = u32(light.params.y);
        let intensity  = light.position.w;
        var L: vec3<f32>;
        var atten: f32 = 1.0;

        if (light_type == 2u) {
            L = normalize(-light.direction.xyz);
        } else {
            let to_light = light.position.xyz - world_pos;
            let dist     = length(to_light);
            let radius   = max(light.color.a, 0.001);
            L = normalize(to_light);
            let d_over_r = dist / radius;
            atten = clamp(1.0 - d_over_r * d_over_r * d_over_r * d_over_r, 0.0, 1.0);
            atten = (atten * atten) / (dist * dist + 1.0);

            if (light_type == 1u) {
                let spot_dir = normalize(light.direction.xyz);
                let cos_a    = dot(-L, spot_dir);
                let inner    = light.direction.w;
                let outer    = light.params.x;
                let eps      = max(inner - outer, 0.001);
                let sf       = clamp((cos_a - outer) / eps, 0.0, 1.0);
                atten *= sf * sf;
            }
        }

        let diff = max(dot(N, L), 0.0);
        let spec = pow(max(dot(view_dir, reflect(-L, N)), 0.0), shininess);
        total_diffuse  += albedo * (1.0 - metallic) * diff * light.color.rgb * atten * intensity;
        total_specular += f0 * spec * (1.0 - min_roughness) * light.color.rgb * atten * intensity;
    }

    var final_color = ambient + total_diffuse + total_specular + fake_ibl;

    // ACES tone mapping
    let a = 2.51; let b = 0.03; let c = 2.43; let d = 0.59; let e = 0.14;
    final_color = clamp(
        (final_color * (a * final_color + b)) / (final_color * (c * final_color + d) + e),
        vec3<f32>(0.0), vec3<f32>(1.0)
    );

    return vec4<f32>(final_color, 1.0);
}
