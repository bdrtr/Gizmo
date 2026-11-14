// ============================================================
// Yelbegen Engine — Post-Processing Shader
// Bloom (Bright Extract + Gaussian Blur) ve ACES Tone Mapping + Sinematikler


struct FullscreenVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) vertex_index: u32) -> FullscreenVertexOutput {
    var out: FullscreenVertexOutput;
    let x = f32(i32(vertex_index) / 2) * 4.0 - 1.0;
    let y = f32(i32(vertex_index) % 2) * 4.0 - 1.0;
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return out;
}

@group(0) @binding(0) var t_source: texture_2d<f32>;
@group(0) @binding(1) var s_source: sampler;


const DOF_TAPS: i32 = 24;
const DOF_SEARCH_TAPS: i32 = 12;

// Depth of field at this distance: x = blur radius in texels, y = its share of the largest. From the
// focus band, or through a lens (`PostProcess::with_lens`). `params.motion.w` is the farthest a
// distance may count as, for a sky that has none of its own.
fn dof_radius_at(distance: f32) -> vec2<f32> {
    if (params.dof_blur_size <= 0.0) {
        return vec2<f32>(0.0);
    }
    let d = select(distance, min(distance, params.motion.w), params.motion.w > 0.0);
    if (params.motion.z > 0.0) {
        let radius = min(params.motion.z * abs(d - params.dof_focus_dist) / max(d, 0.001), params.dof_blur_size);
        return vec2<f32>(radius, radius / params.dof_blur_size);
    }
    let coc = clamp(abs(d - params.dof_focus_dist) / params.dof_focus_range, 0.0, 1.0);
    return vec2<f32>(coc * params.dof_blur_size, coc);
}

// Camera motion blur. A pixel's depth puts it back in clip space; `reproject` says where that
// point was on screen last frame, and the difference is how far the CAMERA moved it. The HDR frame
// is averaged along that path, centred on the pixel (the shutter opens before and closes after the
// instant drawn), over `shutter` of the frame interval. Only camera motion: an object moving in a
// still view has no velocity here. Sky pixels sit at depth 1 and reproject at the far plane, so a
// turning camera smears the sky too, and a translating one barely does — as with a real lens.
fn camera_motion_blur(uv: vec2<f32>, base: vec3<f32>, depth: f32) -> vec3<f32> {
    let shutter = params.motion.x;
    if (shutter <= 0.0) {
        return base;
    }
    var velocity = vec2<f32>(0.0);
    // **Neighbour max** (after McGuire et al. 2012, without the tiles): velocity exists only where a
    // moving object IS, so the background it just uncovered — where its trail belongs — reads zero,
    // and the smear would stay inside the silhouette. Look along both axes as far as a blur can
    // reach and take the fastest motion found: a pixel next to a moving object blurs with it.
    let dims = vec2<f32>(textureDimensions(t_velocity));
    let reach = params.motion.y;
    var neighbour = vec2<f32>(0.0);
    for (var k = -2; k <= 2; k = k + 1) {
        if (k == 0) { continue; }
        let step = f32(k) * 0.5 * reach;
        for (var axis = 0; axis < 2; axis = axis + 1) {
            let offset = select(vec2<f32>(0.0, step), vec2<f32>(step, 0.0), axis == 0);
            let p = vec2<i32>(clamp((uv + offset) * dims, vec2<f32>(0.0), dims - 1.0));
            let v = textureLoad(t_velocity, p, 0).xy;
            if (dot(v, v) > dot(neighbour, neighbour)) {
                neighbour = v;
            }
        }
    }
    if (depth < 1.0) {
        // Geometry: the G-buffer's velocity — object motion as well as the camera's.
        let pixel = vec2<i32>(clamp(uv * dims, vec2<f32>(0.0), dims - 1.0));
        velocity = textureLoad(t_velocity, pixel, 0).xy;
        if (dot(neighbour, neighbour) > dot(velocity, velocity)) {
            velocity = neighbour;
        }
        velocity = velocity * shutter;
    } else if (dot(neighbour, neighbour) > 0.0) {
        // Sky right beside moving geometry: its trail.
        velocity = neighbour * shutter;
    } else {
        // Sky: nothing in the G-buffer, so the camera's own reprojection.
        let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
        let prev_clip = params.reproject * vec4<f32>(ndc, depth, 1.0);
        if (prev_clip.w <= 1e-6) {
            return base;
        }
        let prev_ndc = prev_clip.xy / prev_clip.w;
        velocity = (ndc - prev_ndc) * vec2<f32>(0.5, -0.5) * shutter;
    }
    let len = length(velocity);
    let max_len = params.motion.y;
    if (len > max_len) {
        velocity = velocity * (max_len / len);
    }
    if (length(velocity * vec2<f32>(textureDimensions(t_source))) < 0.5) {
        return base;
    }
    let samples = 12;
    var sum = vec3<f32>(0.0);
    for (var i = 0; i < samples; i = i + 1) {
        let t = (f32(i) + 0.5) / f32(samples) - 0.5;
        sum += textureSampleLevel(t_source, s_source, uv - velocity * t, 0.0).rgb;
    }
    return sum / f32(samples);
}


// `PostProcessParams`, `params` (group 2), the tone curves, the vignette and the dither are in
// `post_common.wgsl`, which `post_process::post_shader_source` puts in front of this file.

// The exposure auto exposure computed this frame (`auto_exposure.wgsl`), or exactly 1.0 while it is
// off — so the product below is bit-identical then. On top of `params.exposure`, the camera's.
@group(3) @binding(0)
var<storage, read> auto_exposure: array<f32>;

// The G-buffer's per-pixel motion since last frame, in screen UV (`crate::motion`): object and
// camera motion together, zero where the G-buffer drew nothing (the sky).
@group(4) @binding(0)
var t_velocity: texture_2d<f32>;


// ============================
// Pass 1: Bright Extract
// ============================
@fragment
fn fs_bright_extract(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(t_source, s_source, in.uv);
    return vec4<f32>(bright_part(color.rgb), 1.0);
}

// The share of a colour that blooms.
fn bright_part(color: vec3<f32>) -> vec3<f32> {
    let luminance = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
    let threshold = params.bloom_threshold;
    let soft_threshold = threshold * (1.0 - params.tonemap.w); // `Renderer::bloom_softness`
    let knee = max(luminance - soft_threshold, 0.0) / (threshold - soft_threshold + 0.0001);
    return color * clamp(knee * knee, 0.0, 1.0);
}

// ============================
// Pass 2: Gaussian Blur
// ============================
struct BlurParams {
    direction: vec2<f32>,
    _padding: vec2<f32>,
};

@group(1) @binding(0) var<uniform> blur_params: BlurParams;

@fragment
fn fs_blur(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    var result = textureSample(t_source, s_source, in.uv) * 0.227027;
    
    let o1 = blur_params.direction * 1.0;
    result += textureSample(t_source, s_source, in.uv + o1) * 0.1945946;
    result += textureSample(t_source, s_source, in.uv - o1) * 0.1945946;
    
    let o2 = blur_params.direction * 2.0;
    result += textureSample(t_source, s_source, in.uv + o2) * 0.1216216;
    result += textureSample(t_source, s_source, in.uv - o2) * 0.1216216;
    
    let o3 = blur_params.direction * 3.0;
    result += textureSample(t_source, s_source, in.uv + o3) * 0.054054;
    result += textureSample(t_source, s_source, in.uv - o3) * 0.054054;
    
    let o4 = blur_params.direction * 4.0;
    result += textureSample(t_source, s_source, in.uv + o4) * 0.016216;
    result += textureSample(t_source, s_source, in.uv - o4) * 0.016216;
    
    return vec4<f32>(result.rgb, 1.0);
}

// ============================
// Pass 3: Composite + Tone Mapping
// ============================
@group(1) @binding(0) var t_bloom: texture_2d<f32>;
@group(1) @binding(2) var t_depth: texture_depth_2d;

@fragment
fn fs_composite(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    // 1. Chromatic Aberration
    let center_dist = distance(in.uv, vec2<f32>(0.5, 0.5));
    let ca_offset = params.chromatic_aberration * center_dist * 0.05;
    
    let r = textureSample(t_source, s_source, in.uv + vec2<f32>(ca_offset, 0.0)).r;
    let g = textureSample(t_source, s_source, in.uv).g;
    let b = textureSample(t_source, s_source, in.uv - vec2<f32>(ca_offset, 0.0)).b;
    let hdr_color_raw = vec3<f32>(r, g, b);

    // 2. Depth of Field (DoF)
    let depth_dims = textureDimensions(t_depth, 0);
    let depth_uv = vec2<i32>(i32(in.uv.x * f32(depth_dims.x)), i32(in.uv.y * f32(depth_dims.y)));
    let depth_val = textureLoad(t_depth, depth_uv, 0);
    // Linearize depth. wgpu/glam perspective_rh writes NDC depth in [0,1] (NOT the
    // OpenGL [-1,1] range), so the [0,1] reconstruction must be used:
    //   view_dist = n*f / (f - d*(f - n))   →  d=0 → n, d=1 → f.
    // (The old (2n)/(f+n-d(f-n)) was the OpenGL [-1,1] formula and was wrong here.)
    let n = params.cam_near;
    let f = params.cam_far;
    let view_dist = (n * f) / (f - depth_val * (f - n));
    let hdr_color = camera_motion_blur(in.uv, hdr_color_raw, depth_val);
    
    let dof = dof_radius_at(view_dist);
    var dof_color = hdr_color;
    if (params.dof_blur_size > 0.0) {
        let texel = 1.0 / vec2<f32>(textureDimensions(t_source));
        let last_depth = vec2<i32>(depth_dims) - vec2<i32>(1);
        let tap_scale = vec2<f32>(depth_dims);

        // Something blurred in front of this pixel spreads over it, however sharp this pixel is, so a
        // sharp pixel has to go looking. How far it gathers is the most any such neighbour reaches.
        var gather = dof.x;
        for (var i = 0; i < DOF_SEARCH_TAPS; i++) {
            let t = (f32(i) + 0.5) / f32(DOF_SEARCH_TAPS);
            let angle = f32(i) * 2.399963;
            let reach = sqrt(t) * params.dof_blur_size;
            let uv = clamp(in.uv + vec2<f32>(cos(angle), sin(angle)) * reach * texel, vec2<f32>(0.0), vec2<f32>(1.0));
            let tap_dist = (n * f) / (f - textureLoad(t_depth, min(vec2<i32>(uv * tap_scale), last_depth), 0) * (f - n));
            if (tap_dist < view_dist) {
                let tap_radius = dof_radius_at(tap_dist).x;
                if (tap_radius >= reach) {
                    gather = max(gather, tap_radius);
                }
            }
        }

        if (gather > 0.01 * params.dof_blur_size) {
            // A disc of taps on the golden angle, so a wide blur does not show as eight copies of the
            // picture. In texels of this frame: these were fractions of a 1920x1080 one, so the blur
            // was narrower the smaller the frame, and wider across than tall on any other shape.
            // The blur's shape (`DofBlur`): a bokeh is the aperture — each tap as heavy as the next, out
            // to a disc or to a polygon of `blades` sides — and a gaussian weighs the near taps most.
            let gaussian = params.dof_shape.x > 0.5;
            let blades = params.dof_shape.y;
            var blurred = vec3<f32>(0.0);
            var kept = 0.0;
            for (var i = 0; i < DOF_TAPS; i++) {
                let t = (f32(i) + 0.5) / f32(DOF_TAPS);
                let angle = f32(i) * 2.399963;
                var reach = sqrt(t) * gather;
                if (!gaussian && blades >= 3.0) {
                    // The polygon's radius toward this tap, its corners on the disc: cos(π/n) over the
                    // cosine of the angle from the middle of the nearest side.
                    let side = 6.2831853 / blades;
                    let a = angle - params.dof_shape.z;
                    let from_side = a - side * floor(a / side) - side * 0.5;
                    reach *= cos(side * 0.5) / cos(from_side);
                }
                let weight = select(1.0, exp(-4.5 * t), gaussian);
                let uv = clamp(in.uv + vec2<f32>(cos(angle), sin(angle)) * reach * texel, vec2<f32>(0.0), vec2<f32>(1.0));
                let tap_dist = (n * f) / (f - textureLoad(t_depth, min(vec2<i32>(uv * tap_scale), last_depth), 0) * (f - n));
                let tap_reaches = dof_radius_at(tap_dist).x >= reach;
                if (tap_dist < view_dist) {
                    // Something sharper in front stays out unless its own blur reaches this far: a
                    // sharp object does not glow onto the blurred background behind it.
                    if (!tap_reaches) {
                        continue;
                    }
                    blurred += textureSampleLevel(t_source, s_source, uv, 0.0).rgb * weight;
                } else if (reach <= dof.x) {
                    blurred += textureSampleLevel(t_source, s_source, uv, 0.0).rgb * weight;
                } else {
                    // Further than this pixel's own blur, and only here for the sake of what is in
                    // front: what is behind stays as sharp as this pixel is.
                    blurred += hdr_color * weight;
                }
                kept += weight;
            }
            if (kept > 0.0) {
                let amount = max(dof.y, gather / params.dof_blur_size);
                dof_color = mix(hdr_color, blurred / kept, min(amount * 4.0, 1.0));
            }
        }
    }

    // 3. Bloom Addition
    let bloom_color = textureSample(t_bloom, s_bloom, in.uv).rgb;
    // Added, the default, a bloom can only brighten. Conserving, the glow comes out of the light that
    // made it: what spreads is taken from where it was, and the total stays what it was.
    var with_bloom = dof_color + bloom_color * params.bloom_intensity;
    if (params.bloom_and_lut.x > 0.5) {
        with_bloom = dof_color + (bloom_color - bright_part(dof_color)) * min(params.bloom_intensity, 1.0);
    }
    let combined = with_bloom * params.exposure * auto_exposure[0];
    
    // 3. Tone mapping — ACES by default, `TonemapCurve` chooses.
    let mapped = display_mapping(
        combined, params.bloom_and_lut.y, params.bloom_and_lut.z, params.bloom_and_lut.w, params.tonemap.x,
        params.tonemap.y,
    );
    
    // Swapchain is configured as an sRGB format (Bgra8UnormSrgb/Rgba8UnormSrgb)
    // Hardware automatically applies gamma correction upon writing.
    var final_color = mapped;

    // ── Su-altı atmosferi: kamera bir su hacmindeyken derinlik-bazlı sis (Beer-Lambert) ──
    // view_dist (lineer sahne derinliği) kullanılır; uzak geometri sis rengine gömülür → mavi-yeşil
    // su hissi. underwater=0 iken tamamen atlanır (etkisiz).
    if (params.underwater > 0.5) {
        let fog_amount = clamp(1.0 - exp(-view_dist * params.fog.a), 0.0, 1.0);
        final_color = mix(final_color, params.fog.rgb, fog_amount);
    }
    
    // 5. Vignette
    final_color *= vignette(in.uv, params.vignette_intensity);
    
    // 6. Film Grain — piksel-koordinatı tabanlı düzgün hash (statik diagonal bant yok).
    let noise = hash12(in.position.xy) - 0.5;
    final_color += final_color * noise * params.film_grain_intensity;

    // 7. Deband dither (`Renderer::deband_dither`): ±half an 8-bit step, added where the steps are
    // — in the display encoding, which the target applies after this — so a dark gradient whose
    // neighbouring pixels would round to the same code no longer shows as flat bands.
    if (params.tonemap.z > 0.5) {
        final_color = deband(final_color, in.position.xy);
    }
    
    return vec4<f32>(final_color, 1.0);
}
