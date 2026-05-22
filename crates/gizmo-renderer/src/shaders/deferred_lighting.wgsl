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
    exposure: f32,
    _pad1: vec2<u32>,
    _pad2: vec3<u32>,
    shading_mode: u32,
};

@group(0) @binding(0) var<uniform> scene: SceneUniforms;

@group(1) @binding(0) var t_shadow: texture_depth_2d_array;
@group(1) @binding(1) var s_shadow: sampler_comparison;
@group(1) @binding(2) var t_point_shadow: texture_depth_cube;

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

// Procedural Physical Sky for IBL
fn get_sky_color(dir: vec3<f32>, sun_dir: vec3<f32>, roughness: f32) -> vec3<f32> {
    let zenith = max(dir.y, 0.0);
    // Base sky gradient
    let sky_color = mix(vec3<f32>(0.5, 0.65, 1.0), vec3<f32>(0.05, 0.2, 0.6), zenith);
    let sun_dot = max(dot(dir, sun_dir), 0.0);
    
    // Sun glow widens based on surface roughness to fake pre-filtered environment map
    let sun_power = mix(2048.0, 16.0, roughness);
    let sun_glow = pow(sun_dot, sun_power) * vec3<f32>(15.0, 12.0, 8.0) * mix(1.0, 0.2, roughness);
    
    // Warm horizon glow towards the sun
    let horizon_glow = pow(1.0 - zenith, 4.0) * vec3<f32>(1.0, 0.6, 0.3) * max(dot(dir, sun_dir) * 0.5 + 0.5, 0.0);
    
    // Ground approximation (dark earth colors)
    let ground_color = vec3<f32>(0.05, 0.05, 0.05);
    if (dir.y < 0.0) {
        return mix(ground_color, horizon_glow * 0.2, pow(1.0 + dir.y, 4.0));
    }
    
    return sky_color * 0.8 + sun_glow * 0.05 + horizon_glow;
}

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

    let t21 = n24 * n33 * n41 - n24 * n31 * n42 - n23 * n34 * n41 + n21 * n34 * n42 + n23 * n31 * n44 - n21 * n33 * n44;
    let t22 = n13 * n34 * n41 - n14 * n33 * n41 + n14 * n31 * n42 - n11 * n34 * n42 - n13 * n31 * n44 + n11 * n33 * n44;
    let t23 = n14 * n23 * n41 - n13 * n24 * n41 - n14 * n21 * n42 + n11 * n24 * n42 + n13 * n21 * n44 - n11 * n23 * n44;
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

fn D_GGX(NoH: f32, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let denom = NoH * NoH * (a2 - 1.0) + 1.0;
    return a2 / (3.1415926535 * denom * denom);
}

fn V_SmithJointGGX(NoV: f32, NoL: f32, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let lambdaV = NoL * sqrt(NoV * NoV * (1.0 - a2) + a2);
    let lambdaL = NoV * sqrt(NoL * NoL * (1.0 - a2) + a2);
    return 0.5 / max(lambdaV + lambdaL, 0.0001);
}

fn F_Schlick(VoH: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * pow(clamp(1.0 - VoH, 0.0, 1.0), 5.0);
}

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

    let diffuse = kD * albedo * NoL;
    let specular = D * Vis * F * NoL;

    return (diffuse + specular) * light_color * intensity * atten;
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = frag_coord.xy;
    let iuv = vec2<i32>(i32(uv.x), i32(uv.y));

    let albedo_metallic  = textureLoad(t_albedo_metallic,  iuv, 0);
    let normal_roughness = textureLoad(t_normal_roughness, iuv, 0);
    let pos_sample       = textureLoad(t_world_position,   iuv, 0);

    let size = textureDimensions(t_albedo_metallic);
    let screen_uv = uv / vec2<f32>(size);
    let ndc = vec2<f32>(screen_uv.x * 2.0 - 1.0, 1.0 - screen_uv.y * 2.0);
    let inv_vp = inverse_mat4(scene.view_proj);
    let clip_pos = vec4<f32>(ndc, 0.0, 1.0);
    let world_pos_from_ray = inv_vp * clip_pos;
    let view_dir = normalize(world_pos_from_ray.xyz / world_pos_from_ray.w - scene.camera_pos.xyz);
    let sun_dir = normalize(-scene.sun_direction.xyz);

    // Unwritten pixels (skipped geometry, unlit objects) — render clean dark grey background (Bevy parity)
    if (pos_sample.w < 0.5) { 
        return vec4<f32>(0.05, 0.05, 0.05, 1.0);
    }

    let albedo    = albedo_metallic.rgb;
    let metallic  = albedo_metallic.a;
    let N         = normalize(normal_roughness.xyz);
    let roughness = normal_roughness.a;
    let world_pos = pos_sample.xyz;

    let min_roughness = max(roughness, 0.05);
    let f0            = mix(vec3<f32>(0.04), albedo, metallic);

    // --- Physically Based IBL (Procedural) ---
    let V = normalize(scene.camera_pos.xyz - world_pos);
    let NdV = max(dot(N, V), 0.001);
    
    // 1. Diffuse IBL (Irradiance)
    // We sample the sky at the normal vector with max roughness.
    let ambient_base = vec3<f32>(0.08, 0.08, 0.09);
    let irradiance = get_sky_color(N, sun_dir, 1.0) * 0.15 + ambient_base;
    let ambient = albedo * irradiance * (1.0 - metallic);
    
    // 2. Specular IBL (Pre-filtered Environment Map)
    // We sample the sky at the reflection vector. The reflection vector is pulled towards normal for rough surfaces.
    let R = reflect(-V, N);
    let R_rough = normalize(mix(R, N, roughness)); 
    let specular_env = get_sky_color(R_rough, sun_dir, roughness) * 0.15; // Realistic ambient specular strength
    
    // 3. Environment BRDF (Schlick approximation for IBL)
    let env_brdf = f0 + (max(vec3<f32>(1.0 - roughness), f0) - f0) * pow(1.0 - NdV, 5.0);
    let specular_ibl = vec3<f32>(0.0); // Bypassed for clean matte surface (Bevy parity)

    // --- CSM Shadow ---
    var shadow_visibility = 1.0;
    if (scene.sun_direction.w > 0.5) {
        let view_depth = dot(world_pos - scene.camera_pos.xyz, scene.camera_forward.xyz);
        let ci         = select_cascade(view_depth);
        
        // Normal offset bias - shifts the lookup position along the normal to completely eliminate shadow acne
        let offset_pos = world_pos + N * 0.015;
        let light_clip = scene.light_view_proj[ci] * vec4<f32>(offset_pos, 1.0);
        
        let light_ndc  = light_clip.xyz / light_clip.w;
        let shadow_uv  = vec2<f32>(light_ndc.x * 0.5 + 0.5, light_ndc.y * -0.5 + 0.5);

        if (shadow_uv.x >= 0.0 && shadow_uv.x <= 1.0 &&
            shadow_uv.y >= 0.0 && shadow_uv.y <= 1.0 && light_ndc.z <= 1.0) {
            let slope  = 1.0 - max(dot(N, normalize(-scene.sun_direction.xyz)), 0.0);
            let bias   = max(0.0002 * slope, 0.00003); // Super tight depth bias thanks to normal offset
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

        // Ground plane self-shadowing/cascade boundary seam bypass
        if (N.y > 0.99 && world_pos.y < 0.01) {
            shadow_visibility = 1.0;
        }
    }

    var total_lighting = vec3<f32>(0.0);

    // --- Directional Sun ---
    if (scene.sun_direction.w > 0.5) {
        let L = normalize(-scene.sun_direction.xyz);
        let sun_light = compute_direct_lighting(
            N, V, L, albedo, min_roughness, metallic, f0,
            scene.sun_color.rgb, scene.sun_color.w, shadow_visibility
        );
        total_lighting += sun_light;
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
            } else if (light_type == 0u) {
                // Point Light Shadow
                let dir_from_light = world_pos - light.position.xyz;
                let abs_dir = abs(dir_from_light);
                let z_near = 0.1;
                let z_far  = 100.0;
                let z_val  = max(abs_dir.x, max(abs_dir.y, abs_dir.z));
                let clip_z = (z_far * (z_val - z_near)) / (z_val * (z_far - z_near));
                
                let slope = 1.0 - max(dot(N, normalize(dir_from_light)), 0.0);
                let bias = max(0.0005 * slope, 0.00005);
                let shadow_vis = textureSampleCompare(t_point_shadow, s_shadow, dir_from_light, clip_z - bias);
                atten *= shadow_vis;
            }
        }

        let light_color_contrib = compute_direct_lighting(
            N, V, L, albedo, min_roughness, metallic, f0,
            light.color.rgb, intensity, atten
        );
        total_lighting += light_color_contrib;
    }

    var final_color = ambient + total_lighting + specular_ibl;
    final_color *= scene.exposure;

    // Inline ACES tone mapping for simple scene path (no post-process hooked up)
    let a2 = 2.51;
    let b2 = 0.03;
    let c2 = 2.43;
    let d2 = 0.59;
    let e2 = 0.14;
    final_color = clamp((final_color * (a2 * final_color + b2)) / (final_color * (c2 * final_color + d2) + e2), vec3<f32>(0.0), vec3<f32>(1.0));
    // Gamma correction
    final_color = pow(final_color, vec3<f32>(1.0 / 2.2));

    // Shading Mode overrides
    if (scene.shading_mode == 1u) {
        // Normals
        return vec4<f32>(N * 0.5 + 0.5, 1.0);
    } else if (scene.shading_mode == 2u) {
        // Albedo
        return vec4<f32>(albedo, 1.0);
    } else if (scene.shading_mode == 3u) {
        // Wireframe (Mock based on world pos)
        let grid = fract(world_pos * 4.0);
        let line = min(grid.x, min(grid.y, grid.z));
        let wire = 1.0 - smoothstep(0.0, 0.05, line);
        return vec4<f32>(mix(albedo * 0.2, vec3<f32>(1.0), wire), 1.0);
    }

    return vec4<f32>(final_color, 1.0);
}
