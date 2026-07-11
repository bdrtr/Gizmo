struct Particle {
    position: vec3<f32>,
    life: f32,
    velocity: vec3<f32>,
    max_life: f32,
    color: vec4<f32>,
    size_start: f32,
    size_end: f32,
    padding: vec2<f32>,
}

struct SimParams {
    dt: f32,
    global_gravity: f32,
    global_drag: f32,
    obstacle_count: f32,
    flow_target: vec4<f32>,          // xyz = nominal akış hızı (relaks hedefi), w = relaks oranı
    misc: vec4<f32>,                 // x = türbülans gücü
    obstacles: array<vec4<f32>, 8>,  // xyz = merkez, w = yarıçap
}

// Diverjanssız (swirl) türbülans alanı — analitik, ucuz, 2 oktav. Düz akışı duman gibi
// dalgalı filamentlere büker. Statik (zamansız) → kararlı, kararlı akış çizgileri.
fn flow_noise(p: vec3<f32>) -> vec3<f32> {
    var v = vec3<f32>(0.0);
    v.x += sin(p.y * 0.7 + 1.3) - sin(p.z * 0.6 + 2.1);
    v.y += sin(p.z * 0.8 + 0.5) - sin(p.x * 0.5 + 4.2);
    v.z += sin(p.x * 0.6 + 3.3) - sin(p.y * 0.9 + 1.7);
    v.x += 0.5 * (sin(p.y * 1.7 + 0.2) - sin(p.z * 1.5 + 3.0));
    v.y += 0.5 * (sin(p.z * 1.9 + 2.4) - sin(p.x * 1.6 + 0.9));
    v.z += 0.5 * (sin(p.x * 1.8 + 1.1) - sin(p.y * 2.0 + 2.7));
    return v * 0.35;
}

// Zamanla evrilen vektör potansiyeli (curl'ün alınacağı alan).
fn curl_potential(p: vec3<f32>, t: f32) -> vec3<f32> {
    return vec3<f32>(
        sin(p.y * 0.9 + t * 0.7) + cos(p.z * 0.7 - t * 0.5),
        sin(p.z * 0.8 - t * 0.6) + cos(p.x * 0.6 + t * 0.4),
        sin(p.x * 0.7 + t * 0.5) + cos(p.y * 0.8 - t * 0.3),
    );
}

// Diverjanssız 3B curl-noise: potansiyelin curl'ü (merkezi farklarla) → hacim korunur
// (parçacıklar sıkışmaz/patlamaz), duman gibi kıvrılır. `t` ile zamanla evrilir.
fn curl3(p: vec3<f32>, t: f32) -> vec3<f32> {
    let e = 0.35;
    let px = curl_potential(p + vec3<f32>(e, 0.0, 0.0), t);
    let mx = curl_potential(p - vec3<f32>(e, 0.0, 0.0), t);
    let py = curl_potential(p + vec3<f32>(0.0, e, 0.0), t);
    let my = curl_potential(p - vec3<f32>(0.0, e, 0.0), t);
    let pz = curl_potential(p + vec3<f32>(0.0, 0.0, e), t);
    let mz = curl_potential(p - vec3<f32>(0.0, 0.0, e), t);
    let curl_x = (py.z - my.z) - (pz.y - mz.y);
    let curl_y = (pz.x - mz.x) - (px.z - mx.z);
    let curl_z = (px.y - mx.y) - (py.x - my.x);
    return vec3<f32>(curl_x, curl_y, curl_z) / (2.0 * e);
}

@group(0) @binding(0) var<uniform> params: SimParams;
@group(0) @binding(1) var<storage, read_write> particles: array<Particle>;

@compute
@workgroup_size(64)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    if (index >= arrayLength(&particles)) {
        return;
    }

    var p = particles[index];
    
    if (p.life >= p.max_life) {
        return; // Dead particle
    }

    p.life += params.dt;
    
    // Physics
    p.velocity.y -= params.global_gravity * params.dt;
    
    let speed = length(p.velocity);
    if (speed > 0.0) {
        var drag_force = p.velocity * params.global_drag * params.dt;
        p.velocity -= drag_force;
    }

    // Curl-noise swirl (duman kıvrılması) — diverjanssız, zamanla evrilir. misc.y=güç, misc.z=zaman.
    let curl_strength = params.misc.y;
    if (curl_strength > 0.0) {
        p.velocity += curl3(p.position, params.misc.z) * curl_strength * params.dt;
    }

    // Engel sapması: parçacık bir engel küresine çarpıp etrafından SAPAR (flow-around).
    // İçeri giren hız bileşeni iptal edilir + dışa doğru sürülür; hız BÜYÜKLÜĞÜ korunur
    // (yalnız YÖN değişir) → çizgi engelin etrafında bükülür, çekirdeğe girmez.
    let oc = i32(params.obstacle_count);
    for (var oi = 0; oi < oc; oi = oi + 1) {
        let o = params.obstacles[oi];
        let c = o.xyz;
        let r = o.w;
        let influence = r * 1.25;
        let d = p.position - c;
        let dist = length(d);
        if (dist < influence && dist > 1e-4) {
            let nrm = d / dist;
            let spd = length(p.velocity);
            // YAPIŞAN akış: içeri (engele doğru) giden hız bileşenini iptal et → hız
            // yüzeye TEĞET olur, parçacık gövde boyunca KAYAR (dışa fırlatma YOK).
            let vin = dot(p.velocity, nrm);
            if (vin < 0.0) {
                p.velocity -= nrm * vin;
            }
            // Yüzeye çok yakınken hafif dışa it (mesh'e girmesin/clipping olmasın) — küçük.
            let push = 1.0 - dist / influence;
            p.velocity += nrm * push * spd * 0.35;
            // Hız büyüklüğünü koru (yalnız yön değişsin).
            let spd2 = length(p.velocity);
            if (spd2 > 1e-4) {
                p.velocity *= spd / spd2;
            }
            // Çekirdeğe (yarıçap içine) girmesin — yüzeye it.
            if (dist < r) {
                p.position = c + nrm * r;
            }
        }
    }

    // Nominal akışa yumuşak relaks: engelden sonra çizgiler tekrar paralelleşir. Hedef,
    // türbülans alanıyla DALGALANDIRILIR → duman gibi kıvrımlı filamentler (çerçeve-bağımsız,
    // genlik doğrudan türbülans gücüyle kontrollü). flow_target.w = relaks oranı.
    let relax = params.flow_target.w;
    if (relax > 0.0) {
        // NOT: 'target' WGSL'de REZERVE kelime → değişken adı olarak KULLANMA (shader derlenmez).
        let flow_goal = params.flow_target.xyz + flow_noise(p.position) * params.misc.x;
        p.velocity += (flow_goal - p.velocity) * relax * params.dt;
    }

    p.position += p.velocity * params.dt;

    particles[index] = p;
}
