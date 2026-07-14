// Deferred lighting pass — fullscreen triangle.
// Reads G-buffers, reconstructs surface data, computes PBR + CSM shadows → HDR output.
//
// SceneUniforms, LightData, compute_direct_lighting and inverse_mat4 are shared from
// common.wgsl; the anisotropic/clear-coat/env-BRDF lobes from pbr_ext.wgsl. Both are composed
// in by load_shader_composed (naga_oil). Only binding-dependent deferred code lives below:
// the fullscreen pass, procedural environment/IBL and PCSS shadows (they read `scene` and the
// shadow textures, so per the common.wgsl convention they stay out of the pure modules).
#import gizmo::common::{SceneUniforms, LightData, compute_direct_lighting}
#import gizmo::pbr_ext::{approximate_env_brdf, compute_direct_lighting_anisotropic, compute_clear_coat}

@group(0) @binding(0) var<uniform> scene: SceneUniforms;

@group(1) @binding(0) var t_shadow: texture_depth_2d_array;
@group(1) @binding(1) var s_shadow: sampler_comparison;
@group(1) @binding(2) var t_point_shadow: texture_depth_cube;

@group(2) @binding(0) var t_albedo_metallic:  texture_2d<f32>;
@group(2) @binding(1) var t_normal_roughness: texture_2d<f32>;
@group(2) @binding(2) var t_world_position:   texture_2d<f32>;
@group(2) @binding(3) var s_gbuf: sampler;
@group(2) @binding(4) var t_world_tangent:    texture_2d<f32>;

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

fn get_single_preset_environment(preset: u32, dir: vec3<f32>, roughness: f32, sun_dot: f32, zenith: f32, horizon_blend: f32) -> vec3<f32> {
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
    
    let sun_dir = normalize(-scene.sun_direction.xyz);
    let horizon_glow = pow(1.0 - max(zenith, 0.0), 4.0) * vec3<f32>(1.0, 0.6, 0.3) * max(dot(dir, sun_dir) * 0.5 + 0.5, 0.0);
    
    let ground_color = vec3<f32>(0.05, 0.05, 0.05);
    
    let sky_val = base_sky * 0.8 + sun_glow * 0.05 + horizon_glow;
    let ground_val = mix(ground_color, horizon_glow * 0.2, pow(1.0 + clamp(zenith, -1.0, 0.0), 4.0)) + sun_glow * 0.1;
    return mix(ground_val, sky_val, horizon_blend);
}

// Procedural HDR Environment Presets with interpolation and smooth blending
fn get_procedural_environment(dir: vec3<f32>, roughness: f32) -> vec3<f32> {
    let zenith = dir.y;
    let sun_dir = normalize(-scene.sun_direction.xyz);
    let sun_dot = max(dot(dir, sun_dir), 0.0);
    
    // Smooth transition factor at the horizon (zenith = 0.0)
    let horizon_blend = smoothstep(0.0, 1.0, clamp(zenith * 10.0 + 0.5, 0.0, 1.0));
    
    let color1 = get_single_preset_environment(scene.environment_preset, dir, roughness, sun_dot, zenith, horizon_blend);
    
    if (scene.environment_blend_t > 0.001) {
        let color2 = get_single_preset_environment(scene.environment_preset_b, dir, roughness, sun_dot, zenith, horizon_blend);
        return mix(color1, color2, scene.environment_blend_t);
    }
    
    return color1;
}

// The anisotropic GGX, clear-coat and env-BRDF lobes now live in gizmo::pbr_ext (imported at
// the top). inverse_mat4 / compute_direct_lighting come from gizmo::common. Only the
// binding-dependent PCSS shadow filter + procedural environment remain deferred-local.

fn search_blockers(
    shadow_uv: vec2<f32>, receiver_depth: f32, ci: u32, texel: f32
) -> vec2<f32> {
    var num_blockers = 0.0;
    var sum_depth = 0.0;

    let search_radius = 2; 
    let step = texel * 1.5;

    for (var x = -search_radius; x <= search_radius; x++) {
        for (var y = -search_radius; y <= search_radius; y++) {
            let offset = vec2<f32>(f32(x), f32(y)) * step;
            let sample_depth = textureSampleLevel(t_shadow, s_gbuf, shadow_uv + offset, ci, 0i);
            
            if (sample_depth < receiver_depth) {
                num_blockers += 1.0;
                sum_depth += sample_depth;
            }
        }
    }

    return vec2<f32>(num_blockers, sum_depth);
}

fn filter_pcss(
    shadow_uv: vec2<f32>, receiver_depth: f32, ci: u32, bias: f32, texel: f32
) -> f32 {
    let blockers = search_blockers(shadow_uv, receiver_depth, ci, texel);
    let num_blockers = blockers.x;
    
    if (num_blockers < 0.5) {
        return 1.0;
    }

    let avg_blocker_depth = blockers.y / num_blockers;

    // (receiver - blocker) / blocker * light_size
    // The sun is nearly a point source at infinity → small angular size → crisp
    // shadows that only soften with distance from the caster. 0.015 modelled a
    // large area light (a soft, unrealistic blob); 0.004 keeps contact-hardening
    // but a sun-like edge.
    let light_size = 0.004;
    let penumbra = (receiver_depth - avg_blocker_depth) / max(avg_blocker_depth, 0.0001) * light_size;

    // Crisp sun shadow: keep the PCF radius near one texel — just enough to
    // anti-alias the shadow-map edge (no blocky stair-stepping), not so much that
    // it turns soft/mushy. The higher SHADOW_MAP_RES (3072) makes one texel small
    // on screen, so a ~1-texel filter reads as a sharp, straight edge.
    let filter_radius = clamp(penumbra, texel * 0.6, texel * 1.2);

    var shadow_sum = 0.0;
    let grid_size = 2;
    let step = filter_radius / 2.0;

    for (var x = -grid_size; x <= grid_size; x++) {
        for (var y = -grid_size; y <= grid_size; y++) {
            let offset = vec2<f32>(f32(x), f32(y)) * step;
            shadow_sum += textureSampleCompare(t_shadow, s_shadow, shadow_uv + offset, ci, receiver_depth - bias);
        }
    }

    return shadow_sum / 25.0;
}

fn compute_height_fog(world_pos: vec3<f32>, camera_pos: vec3<f32>) -> vec4<f32> {
    let view_vec = world_pos - camera_pos;
    let dist = length(view_vec);
    let view_dir = view_vec / max(dist, 0.0001);

    var fog_color = vec3<f32>(0.5, 0.6, 0.7); // default Daylight
    var fog_density = 0.015;
    var fog_height_falloff = 0.05;
    var fog_base_height = -5.0; // base height of fog plane

    if (scene.environment_preset == 0u) {
        // Sunset Gold
        fog_color = vec3<f32>(0.85, 0.38, 0.15);
        fog_density = 0.025;
        fog_height_falloff = 0.08;
    } else if (scene.environment_preset == 1u) {
        // Studio Neutral
        fog_color = vec3<f32>(0.2, 0.22, 0.25);
        fog_density = 0.008;
        fog_height_falloff = 0.04;
    } else if (scene.environment_preset == 2u) {
        // Midnight Neon
        fog_color = vec3<f32>(0.12, 0.02, 0.25);
        fog_density = 0.035;
        fog_height_falloff = 0.12;
    }

    // Blend fog color if we are interpolating presets!
    if (scene.environment_blend_t > 0.001) {
        var fog_color_2 = vec3<f32>(0.5, 0.6, 0.7);
        var fog_density_2 = 0.015;
        var fog_height_falloff_2 = 0.05;

        if (scene.environment_preset_b == 0u) {
            fog_color_2 = vec3<f32>(0.85, 0.38, 0.15);
            fog_density_2 = 0.025;
            fog_height_falloff_2 = 0.08;
        } else if (scene.environment_preset_b == 1u) {
            fog_color_2 = vec3<f32>(0.2, 0.22, 0.25);
            fog_density_2 = 0.008;
            fog_height_falloff_2 = 0.04;
        } else if (scene.environment_preset_b == 2u) {
            fog_color_2 = vec3<f32>(0.12, 0.02, 0.25);
            fog_density_2 = 0.035;
            fog_height_falloff_2 = 0.12;
        }
        fog_color = mix(fog_color, fog_color_2, scene.environment_blend_t);
        fog_density = mix(fog_density, fog_density_2, scene.environment_blend_t);
        fog_height_falloff = mix(fog_height_falloff, fog_height_falloff_2, scene.environment_blend_t);
    }

    // Volumetric analytical scattering (height-decay fog integration)
    let cam_y = camera_pos.y - fog_base_height;
    let dir_y = view_dir.y;

    var fog_amount = 0.0;
    if (abs(dir_y) < 0.0001) {
        fog_amount = fog_density * exp(-fog_height_falloff * cam_y) * dist;
    } else {
        let falloff_dir_y = fog_height_falloff * dir_y;
        fog_amount = (fog_density * exp(-fog_height_falloff * cam_y) * (1.0 - exp(-falloff_dir_y * dist))) / falloff_dir_y;
    }

    let fog_factor = 1.0 - clamp(exp(-fog_amount), 0.0, 1.0);
    return vec4<f32>(fog_color, fog_factor);
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = frag_coord.xy;
    let iuv = vec2<i32>(i32(uv.x), i32(uv.y));

    let albedo_metallic  = textureLoad(t_albedo_metallic,  iuv, 0);
    let normal_roughness = textureLoad(t_normal_roughness, iuv, 0);
    let pos_sample       = textureLoad(t_world_position,   iuv, 0);
    let tangent_sample   = textureLoad(t_world_tangent,    iuv, 0);

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

    var raw_tangent = tangent_sample.xyz;
    if (length(raw_tangent) < 0.001) {
        if (abs(N.x) > 0.9) {
            raw_tangent = cross(vec3<f32>(0.0, 1.0, 0.0), N);
        } else {
            raw_tangent = cross(vec3<f32>(1.0, 0.0, 0.0), N);
        }
    }
    let T = normalize(raw_tangent);
    let bitangent_sign = select(-1.0, 1.0, tangent_sample.w >= 0.0); // handedness only; never 0 (a null tangent.w would zero the bitangent)
    let clear_coat = clamp((abs(tangent_sample.w) - 0.01) / 0.99, 0.0, 1.0);

    let B = normalize(cross(N, T) * bitangent_sign);

    let w_val = pos_sample.w;
    let subsurface = floor(w_val) / 100.0;
    let anisotropy = clamp((w_val - floor(w_val) - 0.5) / 0.49, 0.0, 1.0);

    // --- Physically Based IBL (Procedural HDR Environment Maps) ---
    let V = normalize(scene.camera_pos.xyz - world_pos);
    let NdV = max(dot(N, V), 0.001);
    
    // 1. Diffuse IBL (Irradiance)
    let ambient_base = vec3<f32>(0.02, 0.02, 0.025);
    let irradiance = get_procedural_environment(N, 1.0) + ambient_base;
    var ambient = albedo * irradiance * (1.0 - metallic);
    if (subsurface > 0.0) {
        let sss_color = vec3<f32>(0.96, 0.28, 0.15);
        ambient += ambient * subsurface * sss_color * 0.45;
    }
    
    // 2. Specular IBL (Pre-filtered Environment Map with Anisotropic Stretch)
    var R = reflect(-V, N);
    if (anisotropy > 0.0) {
        let anisotropy_stretch = anisotropy * (1.0 - roughness);
        let anisotropic_direction = cross(cross(N, T), N);
        R = normalize(mix(R, anisotropic_direction, anisotropy_stretch));
    }
    let R_rough = normalize(mix(R, N, roughness)); 
    let specular_env = get_procedural_environment(R_rough, roughness);
    
    // 3. Environment BRDF (Lazarov Analytical Split-Sum LUT approximation)
    let env_brdf_lut = approximate_env_brdf(NdV, roughness);
    var specular_ibl = specular_env * (f0 * env_brdf_lut.x + env_brdf_lut.y);

    // --- 4. Clear Coat Specular & Attenuation IBL ---
    if (clear_coat > 0.0) {
        let F_env = 0.04 + (1.0 - 0.04) * pow(1.0 - NdV, 5.0);
        let coat_atten_env = 1.0 - clear_coat * F_env;

        ambient = ambient * coat_atten_env;
        specular_ibl = specular_ibl * coat_atten_env;

        let R_coat = normalize(mix(reflect(-V, N), N, 0.08));
        let specular_env_coat = get_procedural_environment(R_coat, 0.08);
        let env_brdf_lut_coat = approximate_env_brdf(NdV, 0.08);
        let specular_ibl_coat = specular_env_coat * (0.04 * env_brdf_lut_coat.x + env_brdf_lut_coat.y);

        specular_ibl += specular_ibl_coat * clear_coat;
    }

    // --- CSM Shadow ---
    var shadow_visibility = 1.0;
    if (scene.sun_direction.w > 0.5) {
        let view_depth = dot(world_pos - scene.camera_pos.xyz, scene.camera_forward.xyz);
        let ci         = select_cascade(view_depth);
        
        // Normal-offset shadows: cascade'in DÜNYA texel boyutuna ORANTILI offset ile
        // örnek noktasını yüzeyden ayır. Sabit N*0.0018 yalnızca en yakın cascade'e
        // uyuyordu; uzak cascade'lerde (texel çok daha büyük) offset texel'in ~1/4'ü
        // kalıp yüzeyi temizleyemiyor → diagonal self-shadow acne. Ortho X ölçeği
        // sx = |M'nin lineer satır-0'ı| (V ortonormal), world_texel = 2·uv_texel/sx;
        // ~2 texel offset her cascade'de acne'yi keser, peter-pan minimum.
        // Asıl acne çözümü gölge-pass'te FRONT-FACE CULLING (arka yüzler haritada) — aydınlık
        // ön yüz kendi derinliğiyle kıyaslanmaz. Burada yalnız silüet/temas için ufak, cascade
        // texel'ine orantılı normal-offset kalır (grazing 1/NoL patch'i artık gereksiz).
        let m = scene.light_view_proj[ci];
        let sx = length(vec3<f32>(m[0][0], m[1][0], m[2][0]));
        let world_texel = 2.0 * scene.cascade_params.y / max(sx, 1e-6);
        let offset_pos = world_pos + N * world_texel * 2.0;
        let light_clip = m * vec4<f32>(offset_pos, 1.0);

        let light_ndc  = light_clip.xyz / light_clip.w;
        let shadow_uv  = vec2<f32>(light_ndc.x * 0.5 + 0.5, light_ndc.y * -0.5 + 0.5);

        if (shadow_uv.x >= 0.0 && shadow_uv.x <= 1.0 &&
            shadow_uv.y >= 0.0 && shadow_uv.y <= 1.0 && light_ndc.z <= 1.0) {
            let slope  = 1.0 - max(dot(N, normalize(-scene.sun_direction.xyz)), 0.0);
            // Normal-offset (yukarıdaki world_texel·2) örneği yüzeyden ittiği için depth
            // bias küçük kalır. Eski düz-zemin tabanı `if (N.y>0.99){bias=max(bias,0.005)}`
            // 50x aşırı düzeltmeydi ve gölgeyi kaynağın tabanından peter-pan'ledi.
            let bias   = max(0.0004 * slope, 0.00004);
            let texel  = scene.cascade_params.y;
            shadow_visibility = filter_pcss(shadow_uv, light_ndc.z, ci, bias, texel);
        }
    }

    var total_lighting = vec3<f32>(0.0);

    // --- Directional Sun ---
    if (scene.sun_direction.w > 0.5) {
        let L = normalize(-scene.sun_direction.xyz);
        var sun_light = vec3<f32>(0.0);
        if (anisotropy > 0.0) {
            sun_light = compute_direct_lighting_anisotropic(
                N, V, L, T, B, albedo, min_roughness, metallic, anisotropy, f0,
                scene.sun_color.rgb, scene.sun_color.w, shadow_visibility
            );
        } else {
            sun_light = compute_direct_lighting(
                N, V, L, albedo, min_roughness, metallic, f0,
                scene.sun_color.rgb, scene.sun_color.w, shadow_visibility
            );
        }

        if (clear_coat > 0.0) {
            let H = normalize(V + L);
            let VoH = max(dot(V, H), 0.0);
            let F_c = 0.04 + (1.0 - 0.04) * pow(1.0 - VoH, 5.0);
            let coat_atten = 1.0 - clear_coat * F_c;

            let coat_spec = compute_clear_coat(N, V, L, scene.sun_color.rgb, scene.sun_color.w, shadow_visibility);
            sun_light = sun_light * coat_atten + coat_spec * clear_coat;
        }

        if (subsurface > 0.0) {
            let sss_wrap = 0.35;
            let sss_ndl = max((dot(N, L) + sss_wrap) / (1.0 + sss_wrap), 0.0);
            let sss_power = 8.0;
            let sss_scale = 0.65;
            let sss_trans = pow(max(dot(-V, L), 0.0), sss_power) * sss_scale * (1.0 - metallic);
            
            let sss_color = vec3<f32>(0.96, 0.28, 0.15);
            let sss_contrib = (sss_ndl * 0.12 + sss_trans) * subsurface * sss_color;
            
            sun_light += sss_contrib * scene.sun_color.rgb * scene.sun_color.w * shadow_visibility;
        }

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
            atten = (atten * atten) / (dist * dist + 0.01);

            if (light_type == 1u) {
                let spot_dir = normalize(light.direction.xyz);
                let cos_a    = dot(-L, spot_dir);
                let inner    = light.direction.w;
                let outer    = light.params.x;
                let eps      = max(inner - outer, 0.001);
                let sf       = clamp((cos_a - outer) / eps, 0.0, 1.0);
                atten *= sf * sf;
            } else if (light_type == 0u) {
                // Point Light Shadow (optional). There is a single point-shadow cube;
                // it belongs to ONE designated caster whose cubemap was rendered this
                // frame. cascade_params.w carries (caster_index + 1), with 0 meaning
                // "no point shadow this frame". The old code applied this one cube to
                // EVERY point light using each light's own position, so every light but
                // the caster got a shadow centred on the wrong place.
                let sp_idx = u32(scene.cascade_params.w);
                if (scene.point_shadows_enabled > 0u && sp_idx > 0u && i == sp_idx - 1u) {
                    let dir_from_light = world_pos - light.position.xyz;
                    let abs_dir = abs(dir_from_light);
                    let z_near = 0.1;
                    // Far plane tracks the light's radius (the same value the CPU builds
                    // the cube projection with) instead of a hardcoded 100 that clipped
                    // large lights and wasted depth precision on small ones.
                    let z_far  = max(light.color.a, 1.0);
                    let z_val  = max(abs_dir.x, max(abs_dir.y, abs_dir.z));
                    let clip_z = (z_far * (z_val - z_near)) / (z_val * (z_far - z_near));

                    // Slope-scaled bias only. The old flat-ground floor
                    // `if (N.y > 0.99) { bias = max(bias, 0.01); }` was the same 200x
                    // over-correction the CSM path already dropped — it peter-panned the
                    // shadow off a flat receiver's contact point.
                    let slope = 1.0 - max(dot(N, normalize(dir_from_light)), 0.0);
                    let bias = max(0.0005 * slope, 0.00005);
                    let shadow_vis = textureSampleCompare(t_point_shadow, s_shadow, dir_from_light, clip_z - bias);
                    atten *= shadow_vis;
                }
            }
        }

        var light_color_contrib = vec3<f32>(0.0);
        if (anisotropy > 0.0) {
            light_color_contrib = compute_direct_lighting_anisotropic(
                N, V, L, T, B, albedo, min_roughness, metallic, anisotropy, f0,
                light.color.rgb, intensity, atten
            );
        } else {
            light_color_contrib = compute_direct_lighting(
                N, V, L, albedo, min_roughness, metallic, f0,
                light.color.rgb, intensity, atten
            );
        }

        if (clear_coat > 0.0) {
            let H = normalize(V + L);
            let VoH = max(dot(V, H), 0.0);
            let F_c = 0.04 + (1.0 - 0.04) * pow(1.0 - VoH, 5.0);
            let coat_atten = 1.0 - clear_coat * F_c;

            let coat_spec = compute_clear_coat(N, V, L, light.color.rgb, intensity, atten);
            light_color_contrib = light_color_contrib * coat_atten + coat_spec * clear_coat;
        }

        total_lighting += light_color_contrib;
    }

    // Exposure is NOT applied here anymore — it is a single post-process knob applied over
    // the whole composited HDR (deferred + sky + unlit), so it can't compound or skip the
    // sky/unlit forward objects. scene.exposure is left in the uniform for layout stability.
    var final_color = ambient + total_lighting + specular_ibl;

    // Apply volumetric analytical height fog
    let fog = compute_height_fog(world_pos, scene.camera_pos.xyz);
    final_color = mix(final_color, fog.rgb, fog.a);

    // Shading Mode overrides
    if (scene.shading_mode == 1u) {
        // Normals
        return vec4<f32>(N * 0.5 + 0.5, 1.0);
    } else if (scene.shading_mode == 2u) {
        // Albedo
        return vec4<f32>(albedo, 1.0);
    } else if (scene.shading_mode == 3u) {
        // Roughness/Metallic
        return vec4<f32>(roughness, metallic, 0.0, 1.0);
    } else if (scene.shading_mode == 4u) {
        // Shadows debug
        return vec4<f32>(vec3<f32>(shadow_visibility), 1.0);
    } else if (scene.shading_mode == 5u) {
        // Tangents View
        return vec4<f32>(T * 0.5 + 0.5, 1.0);
    } else if (scene.shading_mode == 6u) {
        // Clear Coat View
        return vec4<f32>(vec3<f32>(clear_coat), 1.0);
    }

    return vec4<f32>(final_color, 1.0);
}
