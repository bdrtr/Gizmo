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
    padding: f32,
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
    
    p.position += p.velocity * params.dt;
    
    particles[index] = p;
}
