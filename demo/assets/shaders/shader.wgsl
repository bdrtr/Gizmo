struct LightData {
    position:  vec4<f32>,  // xyz=pos, w=intensity
    color:     vec4<f32>,  // rgb=color, a=radius
    direction: vec4<f32>,  // xyz=dir (spot/directional), w=inner_cutoff_cos
    params:    vec4<f32>,  // x=outer_cutoff_cos, y=light_type (0=point,1=spot,2=dir)
};

// Inverse of a 3x3 matrix for correct normal transformation under non-uniform scale
fn inverse_transpose_3x3(m: mat3x3<f32>) -> mat3x3<f32> {
    let cross01 = cross(m[0], m[1]);
    let cross12 = cross(m[1], m[2]);
    let cross20 = cross(m[2], m[0]);
    let inv_det = 1.0 / dot(m[2], cross01);

    return mat3x3<f32>(
        cross12 * inv_det,
        cross20 * inv_det,
        cross01 * inv_det
    );
}

struct SceneUniforms {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    sun_direction: vec4<f32>,
    sun_color: vec4<f32>,
    lights: array<LightData, 10>,
    light_view_proj: array<mat4x4<f32>, 4>,
    cascade_splits: vec4<f32>,
    camera_forward: vec4<f32>,
    cascade_params: vec4<f32>,
    num_lights: u32,
    _pad_scene: vec3<u32>,
};



@group(0) @binding(0)
var<uniform> scene: SceneUniforms;

@group(2) @binding(0)
var t_shadow: texture_depth_2d_array;

@group(2) @binding(1)
var s_shadow: sampler_comparison;

@group(1) @binding(0)
var t_diffuse: texture_2d<f32>;
@group(1) @binding(1)
var s_diffuse: sampler;

struct SkeletonData {
    joints: array<mat4x4<f32>, 64>, // Maksimum 64 kemik destegi
};
@group(3) @binding(0)
var<uniform> skeleton: SkeletonData;

struct InstanceData {
    model_matrix_0: vec4<f32>,
    model_matrix_1: vec4<f32>,
    model_matrix_2: vec4<f32>,
    model_matrix_3: vec4<f32>,
    albedo_color: vec4<f32>,
    pbr: vec4<f32>,
};

@group(4) @binding(0)
var<storage, read> instances: array<InstanceData>;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) tex_coords: vec2<f32>,
    @location(4) joint_indices: vec4<u32>,
    @location(5) joint_weights: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tex_coords: vec2<f32>,
    @location(3) world_position: vec3<f32>,
    @location(4) inst_albedo: vec4<f32>,
    @location(5) inst_pbr: vec4<f32>,
};

@vertex
fn vs_main(@builtin(instance_index) instance_idx: u32, input: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.color = input.color;
    out.tex_coords = input.tex_coords;
    
    let inst = instances[instance_idx];
    let model = mat4x4<f32>(
        inst.model_matrix_0,
        inst.model_matrix_1,
        inst.model_matrix_2,
        inst.model_matrix_3,
    );

    // Skinning Matrix (Skeletal Animation)
    // Eger joint_weights'in tamami 0 ise iskelet yok demektir, kimligi (Identity) koru.
    var skin_mat = mat4x4<f32>(
        vec4<f32>(1.0, 0.0, 0.0, 0.0),
        vec4<f32>(0.0, 1.0, 0.0, 0.0),
        vec4<f32>(0.0, 0.0, 1.0, 0.0),
        vec4<f32>(0.0, 0.0, 0.0, 1.0)
    );
    
    if (input.joint_weights.x + input.joint_weights.y + input.joint_weights.z + input.joint_weights.w > 0.0) {
        skin_mat = 
            input.joint_weights.x * skeleton.joints[input.joint_indices.x] +
            input.joint_weights.y * skeleton.joints[input.joint_indices.y] +
            input.joint_weights.z * skeleton.joints[input.joint_indices.z] +
            input.joint_weights.w * skeleton.joints[input.joint_indices.w];
    }

    // Objenin Vertex'ini dünya evrenine taşı (Model space -> Skin space -> World space)
    let skinned_pos = skin_mat * vec4<f32>(input.position, 1.0);
    let world_pos = model * vec4<f32>(skinned_pos.xyz, 1.0);
    out.world_position = world_pos.xyz;
    
    // Obje veya animasyon döndürüldüğünde ışık da tepki versin (Non-uniform scale desteği ile)
    let skinned_normal = skin_mat * vec4<f32>(input.normal, 0.0);
    let normal_matrix = inverse_transpose_3x3(mat3x3<f32>(model[0].xyz, model[1].xyz, model[2].xyz));
    let world_normal = normal_matrix * skinned_normal.xyz;
    out.normal = world_normal;
    
    out.inst_albedo = inst.albedo_color;
    out.inst_pbr = inst.pbr;

    // Kameraya yansıt
    out.clip_position = scene.view_proj * world_pos;
    
    return out;
}

fn select_cascade(view_depth: f32) -> u32 {
    if (view_depth < scene.cascade_splits.x) { return 0u; }
    if (view_depth < scene.cascade_splits.y) { return 1u; }
    if (view_depth < scene.cascade_splits.z) { return 2u; }
    return 3u;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let tex_color = textureSample(t_diffuse, s_diffuse, in.tex_coords);
    
    // Alpha Cutoff (Alpha Test)
    let final_alpha = in.inst_albedo.a * tex_color.a;
    if (final_alpha < 0.5) {
        discard;
    }

    var raw_normal = in.normal;
    if (length(raw_normal) < 0.001) {
        // Hatalı (sıfır) normalleri olan modeller için NaN hatasını engelle!
        raw_normal = vec3<f32>(0.0, 1.0, 0.0);
    }
    let N = normalize(raw_normal);
    
    // Temel Yüzey Rengi (Albedo Rengi * Texture Rengi)
    let base_color = in.inst_albedo.rgb * tex_color.rgb;
    let metallic = clamp(in.inst_pbr.y, 0.0, 1.0);

    // Eger bu obje 'unlit' (isik yemeyen gokyuzu vs.) ise isiklari es gec ve duz renk bas!
    if (in.inst_pbr.z > 1.5) {
        let view_dir = normalize(in.world_position - scene.camera_pos.xyz);
        let sky_y = view_dir.y;
        
        let sky_color = vec3<f32>(0.08, 0.28, 0.58); // Koyu Mavi
        let horizon_color = vec3<f32>(0.65, 0.75, 0.85); // Ufuk rengi (Puslu Acik Mavi)
        let ground_color = vec3<f32>(0.15, 0.15, 0.18); // Kara toprak

        var final_bg: vec3<f32>;
        if (sky_y > 0.0) {
            final_bg = mix(horizon_color, sky_color, sky_y);
        } else {
            final_bg = mix(horizon_color, ground_color, -sky_y);
        }
        return vec4<f32>(final_bg, 1.0);
    } else if (in.inst_pbr.z > 0.5) {
        return vec4<f32>(base_color, in.inst_albedo.a * tex_color.a);
    }
    
    let min_roughness = max(in.inst_pbr.x, 0.05);
    let shininess = 2.0 / (min_roughness * min_roughness) - 2.0;
    let view_dir = normalize(scene.camera_pos.xyz - in.world_position);
    let f0 = mix(vec3<f32>(0.04), base_color, metallic);
    
    // --- Golden Hour (Gün Batımı) Hemispheric (Yarı-küresel) Ortam Işığı ---
    let sky_ambient = vec3<f32>(0.8, 0.5, 0.4) * 0.7; // Gün batımı gökyüzünden yansıyan kızıl ışık
    let ground_ambient = vec3<f32>(0.15, 0.1, 0.15); // Yerden seken morumsu gölge 
    let hemi_mix = N.y * 0.5 + 0.5;
    let ambient = base_color * mix(ground_ambient, sky_ambient, hemi_mix);
    
    // --- Fake IBL (Image Based Lighting) Yansıması ---
    let R = reflect(-view_dir, N);
    let reflect_mix = clamp(R.y * 0.5 + 0.5, 0.0, 1.0);
    // Yansımaları gökyüzü rengine (gün batımına) uydur
    let fake_env_color = mix(ground_ambient, vec3<f32>(1.0, 0.6, 0.4), reflect_mix);
    
    let fake_ibl_specular = f0 * fake_env_color * ((1.0 - min_roughness) * (1.0 - min_roughness) * 2.0);

    // --- CSM Gölge (PCF, doğrudan güneş açıkken) ---
    var shadow_visibility = 1.0;
    if (scene.sun_direction.w > 0.5) {
        let view_depth = dot(in.world_position - scene.camera_pos.xyz, scene.camera_forward.xyz);
        let ci = select_cascade(view_depth);
        let light_vp = scene.light_view_proj[ci];
        let light_clip = light_vp * vec4<f32>(in.world_position, 1.0);
        let light_ndc = light_clip.xyz / light_clip.w;
        let shadow_uv = vec2<f32>(
            light_ndc.x * 0.5 + 0.5,
            (light_ndc.y * -0.5) + 0.5
        );
        if (shadow_uv.x >= 0.0 && shadow_uv.x <= 1.0 && shadow_uv.y >= 0.0 && shadow_uv.y <= 1.0 && light_ndc.z <= 1.0) {
            let L_dir = normalize(-scene.sun_direction.xyz);
            let slope = 1.0 - max(dot(N, L_dir), 0.0);
            let bias = max(0.005 * slope, 0.001);
            var pcf_visibility = 0.0;
            let texel_size = scene.cascade_params.y;
            for (var x = -1; x <= 1; x++) {
                for (var y = -1; y <= 1; y++) {
                    let offset = vec2<f32>(f32(x), f32(y)) * texel_size;
                    pcf_visibility += textureSampleCompare(
                        t_shadow, s_shadow,
                        shadow_uv + offset,
                        ci,
                        light_ndc.z - bias
                    );
                }
            }
            shadow_visibility = pcf_visibility / 9.0;
        }
    }
    
    var total_diffuse = vec3<f32>(0.0);
    var total_specular = vec3<f32>(0.0);

    // --- 1. Directional Light (Güneş) ---
    if (scene.sun_direction.w > 0.5) { 
        let L = normalize(-scene.sun_direction.xyz);
        let diff = max(dot(N, L), 0.0);
        
        let reflect_dir = reflect(-L, N);
        let spec = pow(max(dot(view_dir, reflect_dir), 0.0), shininess);
        
        let intensity = scene.sun_color.w;
        let sun_color = scene.sun_color.rgb;

        total_diffuse += base_color * (1.0 - metallic) * diff * sun_color * intensity * shadow_visibility;
        total_specular += f0 * spec * (1.0 - min_roughness) * sun_color * intensity * shadow_visibility;
    }

    // --- 2. Dynamic Lights (Point / Spot / Directional) ---
    for (var i = 0u; i < scene.num_lights; i++) {
        let light = scene.lights[i];
        let light_type = u32(light.params.y);
        let intensity = light.position.w;
        
        var L: vec3<f32>;
        var attenuation: f32 = 1.0;

        if (light_type == 2u) {
            // --- Directional Light ---
            L = normalize(-light.direction.xyz);
            attenuation = 1.0;
        } else {
            // --- Point & Spot: distance-based attenuation ---
            let to_light = light.position.xyz - in.world_position;
            let distance = length(to_light);
            let radius = max(light.color.a, 0.001);
            L = normalize(to_light);
            // Smooth inverse-square falloff with radius cutoff
            let d_over_r = distance / radius;
            attenuation = clamp(1.0 - d_over_r * d_over_r * d_over_r * d_over_r, 0.0, 1.0);
            attenuation = (attenuation * attenuation) / (distance * distance + 1.0);

            if (light_type == 1u) {
                // --- Spot cone attenuation ---
                let spot_dir = normalize(light.direction.xyz);
                let cos_angle = dot(-L, spot_dir);
                let inner_cos = light.direction.w;
                let outer_cos = light.params.x;
                let epsilon = max(inner_cos - outer_cos, 0.001);
                let spot_factor = clamp((cos_angle - outer_cos) / epsilon, 0.0, 1.0);
                attenuation *= spot_factor * spot_factor; // Smooth edge
            }
        }

        let diff = max(dot(N, L), 0.0);
        let reflect_dir = reflect(-L, N);
        let spec = pow(max(dot(view_dir, reflect_dir), 0.0), shininess);

        total_diffuse += base_color * (1.0 - metallic) * diff * light.color.rgb * attenuation * intensity;
        total_specular += f0 * spec * (1.0 - min_roughness) * light.color.rgb * attenuation * intensity;
    }
    
    // Parçaları topla
    var v_color = in.color;
    if (length(v_color) < 0.0001) {
        v_color = vec3<f32>(1.0, 1.0, 1.0);
    }
    var final_color = v_color * (ambient + total_diffuse + total_specular + fake_ibl_specular);
    
    // --- ACES Tone Mapping (Filmik Renk Düzenlemesi) ---
    // Patlayan aşırı beyaz/parlak renkleri yumuşatarak sinematik ve gerçekçi bir filtre atar
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    final_color = clamp((final_color * (a * final_color + b)) / (final_color * (c * final_color + d) + e), vec3<f32>(0.0), vec3<f32>(1.0));
    // Srgb gamma düzeltmesi
    final_color = pow(final_color, vec3<f32>(1.0 / 2.2));
    
    return vec4<f32>(final_color, in.inst_albedo.a * tex_color.a);
}
