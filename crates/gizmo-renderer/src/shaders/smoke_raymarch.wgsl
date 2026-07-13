// GERÇEK VOLUMETRİK DUMAN (T6-lite): tam-ekran raymarch. Sınırlı bir kutu içinde animasyonlu
// 3B fBm yoğunluğunu ışın boyunca march eder; Beer-Lambert geçirgenlik + güneş saçılımı + sahne
// derinliğine göre occlusion; HDR'ye premultiplied-over kompozit. (Grid/advect/pressure YOK —
// gerçek Eulerian sim bir sonraki adım.)
#import gizmo::common::{SceneUniforms, inverse_mat4}

@group(0) @binding(0) var<uniform> scene: SceneUniforms;
@group(1) @binding(0) var scene_depth: texture_depth_2d;

struct SmokeParams {
    bounds_min: vec4<f32>, // xyz = kutu min, w = zaman
    bounds_max: vec4<f32>, // xyz = kutu max, w = absorption
    p0: vec4<f32>,         // x=density_scale, y=(boş), z=steps, w=dt
    color: vec4<f32>,      // rgb = duman rengi, w = ambient
    grid: vec4<f32>,       // x=N (grid çözünürlüğü), z=source_radius, w=inject
    source: vec4<f32>,     // xyz = kaynak, w = dissipation
    sim: vec4<f32>,        // x=buoyancy, y=curl_strength, z=curl_scale
};
@group(2) @binding(0) var<uniform> smoke: SmokeParams;

// Group 3: simüle edilen 3B yoğunluk grid'i (advect compute doldurur) — trilinear örneklenir.
@group(3) @binding(0) var<storage, read> density_grid: array<f32>;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VOut {
    var out: VOut;
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    out.uv = vec2<f32>(x, y);
    out.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    return out;
}

// Simüle edilen 3B yoğunluk grid'ini dünya konumunda trilinear örnekler (sınır dışı → 0).
fn density_at(p: vec3<f32>, t: f32) -> f32 {
    let n = i32(smoke.grid.x);
    let bmin = smoke.bounds_min.xyz;
    let bmax = smoke.bounds_max.xyz;
    let g = (p - bmin) / (bmax - bmin) * f32(n) - 0.5;
    let gi = floor(g);
    let f = g - gi;
    var acc = 0.0;
    for (var dz = 0; dz < 2; dz = dz + 1) {
        for (var dy = 0; dy < 2; dy = dy + 1) {
            for (var dx = 0; dx < 2; dx = dx + 1) {
                let ci = i32(gi.x) + dx;
                let cj = i32(gi.y) + dy;
                let ck = i32(gi.z) + dz;
                if (ci < 0 || cj < 0 || ck < 0 || ci >= n || cj >= n || ck >= n) {
                    continue;
                }
                let wx = mix(1.0 - f.x, f.x, f32(dx));
                let wy = mix(1.0 - f.y, f.y, f32(dy));
                let wz = mix(1.0 - f.z, f.z, f32(dz));
                acc += density_grid[u32((ck * n + cj) * n + ci)] * wx * wy * wz;
            }
        }
    }
    return acc * smoke.p0.x;
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    let ndc = vec2<f32>(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0);
    let inv = inverse_mat4(scene.view_proj);
    let near_h = inv * vec4<f32>(ndc, 0.0, 1.0);
    let far_h = inv * vec4<f32>(ndc, 1.0, 1.0);
    let ro = scene.camera_pos.xyz;
    let rd = normalize(far_h.xyz / far_h.w - near_h.xyz / near_h.w);

    // Işın–AABB (slab)
    let bmin = smoke.bounds_min.xyz;
    let bmax = smoke.bounds_max.xyz;
    let inv_rd = 1.0 / rd;
    let t0 = (bmin - ro) * inv_rd;
    let t1 = (bmax - ro) * inv_rd;
    let tsm = min(t0, t1);
    let tbg = max(t0, t1);
    let tmin = max(max(tsm.x, tsm.y), tsm.z);
    let tmax = min(min(tbg.x, tbg.y), tbg.z);
    if (tmax <= max(tmin, 0.0)) {
        discard;
    }

    // Sahne derinliği → occlusion: ışın boyunca sahne yüzeyine olan mesafe.
    let sz = textureLoad(scene_depth, vec2<i32>(in.pos.xy), 0);
    let sw_h = inv * vec4<f32>(ndc, sz, 1.0);
    let scene_dist = length(sw_h.xyz / sw_h.w - ro);

    let start = max(tmin, 0.0);
    // Arka plan (derinlik=far=1.0) → scene_dist çok büyük → kutu tam march edilir.
    // Geometri (kutu) → scene_dist küçük → duman yüzeyin arkasında kesilir (occlusion).
    let end = min(tmax, scene_dist);
    if (end <= start) {
        discard;
    }

    let steps = i32(smoke.p0.z);
    let dstep = (end - start) / f32(steps);
    let t = smoke.bounds_min.w;
    let absorption = smoke.bounds_max.w;
    let sun_dir = normalize(scene.sun_direction.xyz);
    let sun_col = scene.sun_color.rgb * scene.sun_color.w;

    var transm = 1.0;
    var accum = vec3<f32>(0.0);
    for (var i = 0; i < steps; i = i + 1) {
        let tt = start + (f32(i) + 0.5) * dstep;
        let p = ro + rd * tt;
        let d = density_at(p, t);
        if (d > 0.002) {
            let a = d * absorption * dstep;
            // Basit ışıklandırma: ambient + ileri saçılım (güneşe bakış). Self-shadow yok (perf).
            let fwd = max(dot(rd, sun_dir), 0.0);
            let light = smoke.color.rgb * (smoke.color.w + fwd * 0.6) * (vec3<f32>(0.25) + sun_col * 0.5);
            accum += transm * (1.0 - exp(-a)) * light;
            transm *= exp(-a);
        }
        if (transm < 0.01) {
            break;
        }
    }
    return vec4<f32>(accum, 1.0 - transm); // premultiplied over
}
