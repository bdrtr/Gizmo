// Screen-Space Ambient Occlusion.
// Reads world-space normals and positions from the G-buffer, samples a
// hemisphere oriented along the surface normal, and outputs an AO factor.

struct SceneUniforms {
    view_proj:  mat4x4<f32>,
    camera_pos: vec4<f32>,
};

struct SsaoKernel {
    samples: array<vec4<f32>, 16>,
};

@group(0) @binding(0) var<uniform> scene: SceneUniforms;

@group(1) @binding(0) var t_normal:   texture_2d<f32>;
@group(1) @binding(1) var t_position: texture_2d<f32>;
@group(1) @binding(2) var t_noise:    texture_2d<f32>;
@group(1) @binding(3) var s_gbuf:     sampler;
@group(1) @binding(4) var s_noise:    sampler;
@group(1) @binding(5) var<uniform>   kernel: SsaoKernel;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    var pos = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4(pos[vi], 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    let iuv = vec2<i32>(i32(frag_coord.x) * 2, i32(frag_coord.y) * 2);

    let pos_samp = textureLoad(t_position, iuv, 0);
    if (pos_samp.w < 0.5) { return vec4(1.0); } // sky / unlit → fully lit

    let world_pos = pos_samp.xyz;
    let N = normalize(textureLoad(t_normal, iuv, 0).xyz);

    let dims = vec2<f32>(textureDimensions(t_position));

    // Tile the 4×4 noise texture across the screen for random hemisphere rotation
    let noise_uv  = (frag_coord.xy * 2.0) / 4.0;
    let rnd_vec   = normalize(textureSample(t_noise, s_noise, noise_uv).xyz * 2.0 - 1.0);

    // Gram-Schmidt: build TBN aligned with the world-space surface normal
    let T   = normalize(rnd_vec - N * dot(rnd_vec, N));
    let B   = cross(N, T);
    let TBN = mat3x3<f32>(T, B, N);

    let radius = 0.5;   // world-space sampling radius (metres)
    let bias   = 0.015; // prevent self-occlusion

    var occlusion = 0.0;
    for (var i = 0u; i < 16u; i++) {
        // Transform kernel sample from tangent space to world space
        let w_samp = world_pos + TBN * kernel.samples[i].xyz * radius;

        // Project world-space sample to screen
        let clip = scene.view_proj * vec4(w_samp, 1.0);
        if (clip.w <= 0.001) { continue; }
        let ndc = clip.xyz / clip.w;
        let suv = vec2(ndc.x * 0.5 + 0.5, ndc.y * -0.5 + 0.5);
        if (any(suv < vec2(0.0)) || any(suv > vec2(1.0))) { continue; }

        // Look up the actual geometry at that screen position
        let siuv     = vec2<i32>(i32(suv.x * dims.x), i32(suv.y * dims.y));
        let occ_samp = textureLoad(t_position, siuv, 0);
        if (occ_samp.w < 0.5) { continue; }

        // Compare camera distances to detect occlusion
        let occ_dist = length(occ_samp.xyz - scene.camera_pos.xyz);
        let s_dist   = length(w_samp       - scene.camera_pos.xyz);

        // Range falloff: ignore occluders farther than sampling radius
        let range = smoothstep(0.0, 1.0, radius / max(length(world_pos - occ_samp.xyz), 0.001));
        if (occ_dist <= s_dist - bias) {
            occlusion += range;
        }
    }

    let ao = 1.0 - (occlusion / 16.0);
    return vec4(ao, ao, ao, 1.0);
}
