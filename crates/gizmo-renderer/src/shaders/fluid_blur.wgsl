@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var dst_tex: texture_storage_2d<rgba16float, write>;

struct BlurParams {
    direction: vec2<i32>,
    filter_radius: i32,
    blur_scale: f32,
}
@group(0) @binding(2) var<uniform> params: BlurParams;

@compute @workgroup_size(16, 16)
fn blur_main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let coords = vec2<i32>(global_id.xy);
    let dims = vec2<i32>(textureDimensions(src_tex));
    if (coords.x >= dims.x || coords.y >= dims.y) { return; }

    let center_val = textureLoad(src_tex, coords, 0);
    let center_depth = center_val.w;
    if (center_depth >= 1.0) { // Far plane / no fluid
        textureStore(dst_tex, coords, vec4<f32>(0.0, 0.0, 0.0, 1.0));
        return;
    }

    var sum = vec4<f32>(0.0);
    var wsum = 0.0;
    // blur_scale helps tune the edge preservation based on projection matrix
    let blur_depth_falloff = params.blur_scale; 

    for (var i = -params.filter_radius; i <= params.filter_radius; i = i + 1) {
        let sample_coords = coords + params.direction * i;
        
        let clamped = vec2<i32>(
            clamp(sample_coords.x, 0, dims.x - 1),
            clamp(sample_coords.y, 0, dims.y - 1)
        );

        let sample_val = textureLoad(src_tex, clamped, 0);
        let sample_depth = sample_val.w;
        if (sample_depth >= 1.0) { continue; } // Don't bleed background into fluid
        
        // Spatial weight (Gaussian)
        let r = f32(i);
        let spatial_w = exp(-r * r / 10.0);

        // Range weight (Bilateral - preserves edges)
        let diff = (sample_depth - center_depth) * blur_depth_falloff;
        let range_w = exp(-diff * diff);

        let weight = spatial_w * range_w;
        sum += sample_val * weight;
        wsum += weight;
    }

    if (wsum > 0.0) {
        var final_val = sum / wsum;
        // Re-normalize the blurred normal vector
        let length_xyz = length(final_val.xyz);
        if (length_xyz > 0.001) {
            final_val = vec4<f32>(final_val.xyz / length_xyz, final_val.w);
        }
        textureStore(dst_tex, coords, final_val);
    } else {
        textureStore(dst_tex, coords, center_val);
    }
}
