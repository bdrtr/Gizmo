// VOLUMETRİK DUMAN SİM (T6-full) — 3B yoğunluk grid'ini semi-Lagrangian advekte eder + kaynaktan
// enjekte + dissipation. Hız alanı PROSEDÜREL: buoyancy (yukarı) + diverjanssız curl-noise
// (zamanla evrilir). Tam basınç-çözücüsü YOK (yükselen/kıvrılan/dağılan duman için yeterli).
// src (okuma) → dst (yazma) ping-pong.

struct SmokeParams {
    bounds_min: vec4<f32>, // xyz = min, w = zaman
    bounds_max: vec4<f32>, // xyz = max, w = absorption (sim'de kullanılmaz)
    p0: vec4<f32>,         // x=density_scale, y=(boş), z=steps, w=dt
    color: vec4<f32>,      // (sim'de kullanılmaz)
    grid: vec4<f32>,       // x=N, y=(boş), z=source_radius, w=inject_amount
    source: vec4<f32>,     // xyz = kaynak, w = dissipation (frame başına çarpan)
    sim: vec4<f32>,        // x=buoyancy, y=curl_strength, z=curl_scale, w=(boş)
};

@group(0) @binding(0) var<uniform> P: SmokeParams;
@group(0) @binding(1) var<storage, read> src: array<f32>;
@group(0) @binding(2) var<storage, read_write> dst: array<f32>;

fn curl_potential(p: vec3<f32>, t: f32) -> vec3<f32> {
    return vec3<f32>(
        sin(p.y * 0.9 + t * 0.7) + cos(p.z * 0.7 - t * 0.5),
        sin(p.z * 0.8 - t * 0.6) + cos(p.x * 0.6 + t * 0.4),
        sin(p.x * 0.7 + t * 0.5) + cos(p.y * 0.8 - t * 0.3),
    );
}
fn curl3(p: vec3<f32>, t: f32) -> vec3<f32> {
    let e = 0.35;
    let px = curl_potential(p + vec3<f32>(e, 0.0, 0.0), t);
    let mx = curl_potential(p - vec3<f32>(e, 0.0, 0.0), t);
    let py = curl_potential(p + vec3<f32>(0.0, e, 0.0), t);
    let my = curl_potential(p - vec3<f32>(0.0, e, 0.0), t);
    let pz = curl_potential(p + vec3<f32>(0.0, 0.0, e), t);
    let mz = curl_potential(p - vec3<f32>(0.0, 0.0, e), t);
    let cx = (py.z - my.z) - (pz.y - mz.y);
    let cy = (pz.x - mz.x) - (px.z - mx.z);
    let cz = (px.y - mx.y) - (py.x - my.x);
    return vec3<f32>(cx, cy, cz) / (2.0 * e);
}

fn cell_index(i: i32, j: i32, k: i32, n: i32) -> u32 {
    return u32((k * n + j) * n + i);
}

// Dünya konumunda grid yoğunluğunu trilinear örnekler (sınır dışı → 0).
fn sample_grid(wpos: vec3<f32>, n: i32) -> f32 {
    let bmin = P.bounds_min.xyz;
    let bmax = P.bounds_max.xyz;
    let g = (wpos - bmin) / (bmax - bmin) * f32(n) - 0.5;
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
                acc += src[cell_index(ci, cj, ck, n)] * wx * wy * wz;
            }
        }
    }
    return acc;
}

@compute
@workgroup_size(4, 4, 4)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = i32(P.grid.x);
    let i = i32(gid.x);
    let j = i32(gid.y);
    let k = i32(gid.z);
    if (i >= n || j >= n || k >= n) {
        return;
    }
    let bmin = P.bounds_min.xyz;
    let bmax = P.bounds_max.xyz;
    let cs = (bmax - bmin) / f32(n);
    let world = bmin + (vec3<f32>(f32(i), f32(j), f32(k)) + 0.5) * cs;
    let dt = P.p0.w;
    let t = P.bounds_min.w;

    // Prosedürel hız: buoyancy (yukarı) + diverjanssız curl (kıvrılma).
    var vel = vec3<f32>(0.0, P.sim.x, 0.0);
    vel += curl3(world * P.sim.z, t) * P.sim.y;

    // Semi-Lagrangian: geriye izle, eski yoğunluğu örnekle, dissipation uygula.
    let back = world - vel * dt;
    var d = sample_grid(back, n) * P.source.w;

    // Kaynaktan enjeksiyon (yumuşak küre).
    let sdist = length(world - P.source.xyz);
    if (sdist < P.grid.z) {
        d += P.grid.w * dt * (1.0 - sdist / P.grid.z);
    }

    dst[cell_index(i, j, k, n)] = clamp(d, 0.0, 6.0);
}
