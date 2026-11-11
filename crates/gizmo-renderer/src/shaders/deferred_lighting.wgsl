// Deferred lighting pass — fullscreen triangle.
// Reads G-buffers, reconstructs surface data, computes PBR + CSM shadows → HDR output.
//
// SceneUniforms, LightData, compute_direct_lighting and inverse_mat4 are shared from
// common.wgsl; the anisotropic/clear-coat/env-BRDF lobes from pbr_ext.wgsl. Both are composed
// in by load_shader_composed (naga_oil). Only binding-dependent deferred code lives below: the
// fullscreen pass, the procedural environment/IBL and contact shadows. The shadow lookups — the
// sun's and the local lights' — are common.wgsl's, handed the textures, so the forward shader takes
// the same ones.
#import gizmo::common::{SceneUniforms, LightData, dielectric_f0, light_falloff, compute_direct_lighting, compute_sphere_lighting, compute_tube_lighting, compute_rect_lighting, rotate_by_quat, spot_cone, spot_cookie_uv, gizmo_cluster_index, scene_environment, height_fog, local_shadow_layer, local_shadow_visibility, light_source_radius, receiver_axes, sun_shadow_visibility, shadow_distance_fade}
#import gizmo::pbr_ext::{approximate_env_brdf, compute_direct_lighting_anisotropic, subsurface_ambient, anisotropic_reflection, coat_env_attenuation, coat_env_specular, layer_clear_coat, subsurface_light}

// Irradiance volumes: group 3. See `irradiance.wgsl` and `gizmo_renderer::irradiance`.
#import gizmo::irradiance::{gi_sample}
#import gizmo::reflection_probes::{probe_environment, environment_irradiance}

@group(0) @binding(0) var<uniform> scene: SceneUniforms;


// Clustered light lists (gizmo-renderer/src/clustered.rs). On the GLOBAL group because the web
// forward pipeline has no fifth bind group to spare — see `pipeline::layouts`.
@group(0) @binding(1)
var<storage, read> cluster_table: array<vec2<u32>>;
@group(0) @binding(2)
var<storage, read> cluster_light_indices: array<u32>;
@group(1) @binding(0) var t_shadow: texture_depth_2d_array;
@group(1) @binding(1) var s_shadow: sampler_comparison;
// The local lights' shadows (`gizmo_renderer::local_shadows`), a layer or six a light.
@group(1) @binding(2) var t_local_shadow: texture_depth_2d_array;
// Light cookies (`gizmo_renderer::cookies`): a spot's params.w = layer + 1.
@group(1) @binding(3) var t_cookie: texture_2d_array<f32>;
@group(1) @binding(4) var s_cookie: sampler;

@group(2) @binding(0) var t_albedo_metallic:  texture_2d<f32>;
@group(2) @binding(1) var t_normal_roughness: texture_2d<f32>;
@group(2) @binding(2) var t_position_rel_camera:   texture_2d<f32>;
@group(2) @binding(3) var s_gbuf: sampler;
@group(2) @binding(4) var t_world_tangent:    texture_2d<f32>;
@group(2) @binding(5) var t_emissive_gbuf:    texture_2d<f32>;
// RT4: the motion vector in `xy`, and in `zw` the specular tint's hue (see `gbuffer.wgsl`).
@group(2) @binding(6) var t_velocity_gbuf:    texture_2d<f32>;

// Reflection probes: `gizmo::reflection_probes` (group 3 here, after the irradiance volume's two
// bindings — `REFLECTION_GROUP` is set to 3 for this shader).

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

// What lights and is reflected by a surface: `scene_environment` along `dir`, as in `shader.wgsl`.
fn get_procedural_environment(dir: vec3<f32>, roughness: f32) -> vec3<f32> {
    return scene_environment(
        dir, roughness, scene.sky_zenith, scene.sky_horizon.rgb, scene.sky_ground.rgb, scene.environment_preset,
        scene.environment_preset_b, scene.environment_blend_t, normalize(-scene.sun_direction.xyz),
    );
}

// The anisotropic GGX, clear-coat and env-BRDF lobes now live in gizmo::pbr_ext (imported at
// the top). inverse_mat4 / compute_direct_lighting come from gizmo::common. Only the
// binding-dependent procedural environment and contact shadows remain deferred-local.

// **Contact shadows**: a short screen-space march from the surface toward the sun, `length` metres
// (`cluster_depth.z`; 0 = off) in 16 steps. Each step is projected into the frame and compared with
// the G-buffer surface there: if that surface is nearer the camera than the step — by a little,
// not by more than a `thickness` that would mean an unrelated object far in front — something
// small sits between this pixel and the sun. That is what the shadow map misses: anything shorter
// than its bias or finer than its texels. Steps leaving the frame, or landing on sky, see nothing.
fn contact_shadow(world_pos: vec3<f32>, n: vec3<f32>, l: vec3<f32>) -> f32 {
    let length = scene.cluster_depth.z;
    if (length <= 0.0) {
        return 1.0;
    }
    let dims = vec2<f32>(textureDimensions(t_position_rel_camera));
    let fwd = scene.camera_forward.xyz;
    let thickness = max(length * 0.5, 0.05);
    let start = world_pos + n * 0.01;
    for (var i = 1; i <= 16; i = i + 1) {
        let p = start + l * (length * f32(i) / 16.0);
        let clip = scene.view_proj * vec4<f32>(p, 1.0);
        if (clip.w <= 0.0) {
            return 1.0;
        }
        let ndc = clip.xy / clip.w;
        if (abs(ndc.x) > 1.0 || abs(ndc.y) > 1.0) {
            return 1.0;
        }
        let pixel = vec2<i32>(vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5) * dims);
        let stored = textureLoad(t_position_rel_camera, pixel, 0);
        if (stored.w < 0.5) {
            continue; // sky
        }
        let ray_depth = dot(p - scene.camera_pos.xyz, fwd);
        let surface_depth = dot(stored.xyz, fwd);
        let gap = ray_depth - surface_depth;
        if (gap > 0.005 && gap < thickness) {
            return 0.0;
        }
    }
    return 1.0;
}

fn compute_height_fog(world_pos: vec3<f32>, camera_pos: vec3<f32>) -> vec4<f32> {
    return height_fog(world_pos, camera_pos, scene.fog, scene.fog_shape);
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = frag_coord.xy;
    let iuv = vec2<i32>(i32(uv.x), i32(uv.y));

    let albedo_metallic  = textureLoad(t_albedo_metallic,  iuv, 0);
    let normal_roughness = textureLoad(t_normal_roughness, iuv, 0);
    let pos_sample       = textureLoad(t_position_rel_camera,   iuv, 0);
    let tangent_sample   = textureLoad(t_world_tangent,    iuv, 0);
    // How the surface runs across neighbouring pixels: its geometry's plane, which a local light's
    // soft shadow filters along (`receiver_axes`). Taken here, before any pixel returns, where every
    // pixel of the quad is still running.
    let pos_dx = dpdx(pos_sample.xyz);
    let pos_dy = dpdy(pos_sample.xyz);

    // Unwritten pixels (skipped geometry, unlit objects): the background — the camera's, or the
    // one default every path draws (`frame_uniforms::DEFAULT_BACKGROUND`) — rather than whatever the
    // G-buffer happened to still hold from the previous frame.
    if (pos_sample.w < 0.5) {
        return vec4<f32>(scene.clear_color, 1.0);
    }

    let albedo    = albedo_metallic.rgb;
    let metallic  = albedo_metallic.a;
    let N         = normalize(normal_roughness.xyz);
    let roughness = normal_roughness.a;
    // G-buffer position is camera-relative (see gbuffer.wgsl); put it back in world space.
    let world_pos = pos_sample.xyz + scene.camera_pos.xyz;

    let min_roughness = max(roughness, 0.05);
    let dielectric    = textureLoad(t_emissive_gbuf, iuv, 0).w;
    // `Material::specular_tint`: the hue from RT4, green put back from a luminance of 1; RT5's F0
    // carries the luminance.
    let hue_rb        = vec2<f32>(1.0) - textureLoad(t_velocity_gbuf, iuv, 0).zw;
    let tint          = vec3<f32>(hue_rb.x, max((1.0 - 0.2126 * hue_rb.x - 0.0722 * hue_rb.y) / 0.7152, 0.0), hue_rb.y);
    let f0            = mix(min(vec3<f32>(dielectric_f0(dielectric)) * tint, vec3<f32>(1.0)), albedo, metallic);

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
    // See `gbuffer.wgsl`: 1 + 32 x coat steps + roughness steps.
    let coat_packed = max(round(abs(tangent_sample.w)) - 1.0, 0.0);
    let clear_coat = floor(coat_packed / 32.0) / 31.0;
    let coat_roughness = 0.08 + 0.92 * (coat_packed - 32.0 * floor(coat_packed / 32.0)) / 31.0;

    let B = normalize(cross(N, T) * bitangent_sign);

    let w_val = pos_sample.w;
    let subsurface = floor(w_val) / 100.0;
    let anisotropy = clamp((w_val - floor(w_val) - 0.5) / 0.49, 0.0, 1.0);

    // --- Physically Based IBL (Procedural HDR Environment Maps) ---
    let V = normalize(scene.camera_pos.xyz - world_pos);
    let NdV = max(dot(N, V), 0.001);
    
    // 1. Diffuse IBL (Irradiance)
    let ambient_base = vec3<f32>(0.02, 0.02, 0.025);
    let irradiance = environment_irradiance(N, get_procedural_environment(N, 1.0)) + ambient_base;
    var ambient = albedo * irradiance * (1.0 - metallic);
    if (subsurface > 0.0) {
        ambient = subsurface_ambient(ambient, subsurface);
    }
    
    // 2. Specular IBL (Pre-filtered Environment Map with Anisotropic Stretch)
    var R = reflect(-V, N);
    if (anisotropy > 0.0) {
        R = anisotropic_reflection(R, N, T, anisotropy, roughness);
    }
    let R_rough = normalize(mix(R, N, roughness)); 
    // Probes are prefiltered by roughness already, so they take the mirror direction and a mip;
    // the procedural sky takes the bent ray, as it always has.
    let specular_env = probe_environment(R, world_pos, roughness, get_procedural_environment(R_rough, roughness));
    
    // 3. Environment BRDF (Lazarov Analytical Split-Sum LUT approximation)
    let env_brdf_lut = approximate_env_brdf(NdV, roughness);
    var specular_ibl = specular_env * (f0 * env_brdf_lut.x + env_brdf_lut.y);

    // --- 4. Clear Coat Specular & Attenuation IBL ---
    if (clear_coat > 0.0) {
        let coat_atten_env = coat_env_attenuation(NdV, clear_coat);

        ambient = ambient * coat_atten_env;
        specular_ibl = specular_ibl * coat_atten_env;

        let R_coat = normalize(mix(reflect(-V, N), N, coat_roughness));
        // The coat is a second, nearly smooth layer: it reflects the probes too, at its own roughness.
        let specular_env_coat = probe_environment(reflect(-V, N), world_pos, coat_roughness, get_procedural_environment(R_coat, coat_roughness));
        specular_ibl += coat_env_specular(specular_env_coat, NdV, coat_roughness) * clear_coat;
    }

    // How the surface runs across neighbouring pixels — its geometry's plane, which the soft shadows
    // filter along (`receiver_axes`).
    let surface_axes = receiver_axes(pos_dx, pos_dy, N);

    // --- The sun's shadow: `sun_shadow_visibility` in `common.wgsl`, as the forward shader takes it.
    var shadow_visibility = 1.0;
    let view_depth = dot(world_pos - scene.camera_pos.xyz, scene.camera_forward.xyz);
    // cascade_splits.w = far edge of the last cascade = the whole covered range.
    let shadow_fade = shadow_distance_fade(view_depth, scene.cascade_splits.w);
    if (scene.sun_direction.w > 0.5 && shadow_fade > 0.0) {
        let ci = select_cascade(view_depth);
        // Walked back to "lit" over the last stretch of the covered range, so the end of the shadow
        // maps is a gradient rather than a line.
        shadow_visibility = mix(
            1.0,
            sun_shadow_visibility(
                t_shadow, s_shadow, scene.light_view_proj[ci], i32(ci), scene.cascade_params.y, world_pos, N,
                surface_axes, normalize(-scene.sun_direction.xyz), scene.shadow_bias.xy, scene.camera_forward.w,
            ),
            shadow_fade,
        );
    }
    if (scene.sun_direction.w > 0.5 && shadow_visibility > 0.0) {
        shadow_visibility = min(shadow_visibility, contact_shadow(world_pos, N, normalize(-scene.sun_direction.xyz)));
    }
    // `MeshRenderer::receives_shadows == false` (RT5's alpha is negative): no shadow falls here —
    // not the cascades, not contact shadows, and below, not the point-light cube.
    let receives_shadows = dielectric >= 0.0;
    if (!receives_shadows) {
        shadow_visibility = 1.0;
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
            sun_light = layer_clear_coat(sun_light, N, V, L, scene.sun_color.rgb, scene.sun_color.w, shadow_visibility, clear_coat, coat_roughness);
        }

        if (subsurface > 0.0) {
            sun_light += subsurface_light(N, V, L, metallic, subsurface) * scene.sun_color.rgb * scene.sun_color.w * shadow_visibility;
        }

        total_lighting += sun_light;
    }

    // --- Dynamic Lights (clustered) ---
    //
    // Only the lights assigned to THIS fragment's cluster, instead of every light in the frame.
    // `i` still holds the index into `scene.lights`, so everything below — including the
    // point-shadow caster comparison — reads exactly as it did when this looped over all of them.
    let cluster = gizmo_cluster_index(scene.view_proj, scene.camera_pos.xyz, scene.camera_forward.xyz, scene.cluster_dims, scene.cluster_depth, world_pos);
    let cluster_range = cluster_table[cluster];
    for (var ci = 0u; ci < cluster_range.y; ci++) {
        let i = cluster_light_indices[cluster_range.x + ci];
        let light      = scene.lights[i];
        let light_type = u32(light.params.y);
        let intensity  = light.position.w;
        var L: vec3<f32>;
        var atten: f32 = 1.0;
        // The range falloff alone, without the inverse square — what a rectangle light takes.
        var window: f32 = 1.0;
        // A spot's cookie colour (white = none).
        var cookie = vec3<f32>(1.0);

        if (light_type == 2u) {
            L = normalize(-light.direction.xyz);
        } else {
            let to_light = light.position.xyz - world_pos;
            L = normalize(to_light);
            let falloff = light_falloff(length(to_light), light.color.a);
            atten = falloff.x;
            window = falloff.y;

            if (light_type == 3u) {
                // A rectangle lights one side: nothing behind it, for the anisotropic and
                // clear-coat terms below as much as for its own.
                let emit = rotate_by_quat(light.direction, vec3<f32>(0.0, 0.0, -1.0));
                if (dot(-L, emit) <= 0.0) {
                    atten = 0.0;
                    window = 0.0;
                }
            } else if (light_type == 1u) {
                atten *= spot_cone(light, -L);
                if (light.params.w > 0.0) {
                    cookie = textureSampleLevel(
                        t_cookie, s_cookie, spot_cookie_uv(light, world_pos), i32(light.params.w) - 1, 0.0
                    ).rgb;
                }
            }
            // The light's shadow, if it casts one (`gizmo_renderer::local_shadows`) and lights this
            // point at all. On `window` as well as `atten`: a rectangle light is shaded from the
            // window alone.
            let shadow_layer = local_shadow_layer(scene.light_shadows[i / 4u][i % 4u], world_pos - light.position.xyz);
            if (receives_shadows && shadow_layer >= 0 && max(atten, window) > 0.0) {
                let lit = local_shadow_visibility(
                    t_local_shadow, s_shadow, scene.local_shadow_layers[shadow_layer], shadow_layer,
                    world_pos, N, surface_axes, light.position.xyz - world_pos, scene.shadow_bias.xy, light_source_radius(light),
                );
                atten *= lit;
                window *= lit;
            }
        }

        let light_rgb = light.color.rgb * cookie;
        var light_color_contrib = vec3<f32>(0.0);
        if (light_type == 3u) {
            // A rectangle light.
            light_color_contrib = compute_rect_lighting(
                N, V, light.position.xyz - world_pos, light.direction, light.params.z, light.params.w,
                albedo, min_roughness, metallic, f0, light_rgb, intensity, window
            );
        } else if (anisotropy > 0.0) {
            light_color_contrib = compute_direct_lighting_anisotropic(
                N, V, L, T, B, albedo, min_roughness, metallic, anisotropy, f0,
                light_rgb, intensity, atten
            );
        } else if (light_type == 0u && light.params.w > 0.0) {
            // A tube light.
            light_color_contrib = compute_tube_lighting(
                N, V, light.position.xyz - world_pos, light.direction.xyz, light.params.w, light.params.z,
                albedo, min_roughness, metallic, f0, light_rgb, intensity, atten
            );
        } else if (light_type != 2u && light.params.z > 0.0) {
            // A sphere-sourced point or spot light.
            light_color_contrib = compute_sphere_lighting(
                N, V, light.position.xyz - world_pos, light.params.z, albedo, min_roughness, metallic, f0,
                light_rgb, intensity, atten
            );
        } else {
            light_color_contrib = compute_direct_lighting(
                N, V, L, albedo, min_roughness, metallic, f0,
                light_rgb, intensity, atten
            );
        }

        if (clear_coat > 0.0) {
            light_color_contrib = layer_clear_coat(light_color_contrib, N, V, L, light_rgb, intensity, atten, clear_coat, coat_roughness);
        }

        total_lighting += light_color_contrib;
    }

    // Exposure is NOT applied here anymore — it is a single post-process knob applied over
    // the whole composited HDR (deferred + sky + unlit), so it can't compound or skip the
    // sky/unlit forward objects. scene.exposure is left in the uniform for layout stability.
    // Indirect light from the probe grid, if one is uploaded. `gi_sample` returns black when
    // `probe_count == 0`, which is what a renderer with no baked GI binds — so this line costs a
    // branch and nothing else until a scene actually bakes a grid.
    //
    // Added to `ambient` rather than replacing it: the constant ambient term is what a scene
    // without probes relies on, and taking it away would darken every existing scene the moment
    // this shipped. A scene that bakes GI is expected to lower its own ambient.
    let indirect = gi_sample(world_pos, N) * albedo;

    // Emitted light: not lit, not clamped, fogged like the surface it leaves.
    let emitted = textureLoad(t_emissive_gbuf, iuv, 0).rgb;
    var final_color = ambient + indirect + total_lighting + specular_ibl + emitted;

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
