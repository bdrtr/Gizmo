struct Sphere {
    position: vec3<f32>,
    radius: f32,
    velocity: vec3<f32>,
    mass: f32,
    color: vec4<f32>,
    padding: vec4<f32>,
}

struct SimParams {
    dt: f32,
    _pad1: vec3<f32>,
    gravity: vec3<f32>,
    damping: f32,
    num_spheres: u32,
    _pad2: vec3<f32>,
}

@group(0) @binding(0) var<uniform> params: SimParams;
@group(0) @binding(1) var<storage, read_write> spheres: array<Sphere>;
@group(0) @binding(2) var<storage, read_write> grid_heads: array<atomic<i32>>; // Size: GRID_SIZE
@group(0) @binding(3) var<storage, read_write> linked_nodes: array<i32>; // Size: num_spheres

const GRID_SIZE: u32 = 262144u; // 2^18
const CELL_SIZE: f32 = 2.0;

fn hash_pos(pos: vec3<f32>) -> u32 {
    let p = vec3<i32>(floor(pos / CELL_SIZE));
    let hash = u32(p.x * 73856093i) ^ u32(p.y * 19349663i) ^ u32(p.z * 83492791i);
    return hash % GRID_SIZE;
}

// Pass 1
@compute @workgroup_size(256)
fn clear_grid(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    if (idx >= GRID_SIZE) { return; }
    atomicStore(&grid_heads[idx], -1);
}

// Pass 2
@compute @workgroup_size(256)
fn build_grid(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    if (idx >= params.num_spheres) { return; }

    let hash = hash_pos(spheres[idx].position);
    let prev = atomicExchange(&grid_heads[hash], i32(idx));
    linked_nodes[idx] = prev;
}

// Pass 3
@compute @workgroup_size(256)
fn solve_collisions(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    if (idx >= params.num_spheres) { return; }

    var me = spheres[idx];
    let grid_p = vec3<i32>(floor(me.position / CELL_SIZE));
    let restitution = 0.8;

    // Check 27 neighbor cells
    for (var x = -1; x <= 1; x++) {
    for (var y = -1; y <= 1; y++) {
    for (var z = -1; z <= 1; z++) {
        let neighbor_p = grid_p + vec3<i32>(x, y, z);
        let h = (u32(neighbor_p.x * 73856093i) ^ u32(neighbor_p.y * 19349663i) ^ u32(neighbor_p.z * 83492791i)) % GRID_SIZE;
        
        var curr_n = atomicLoad(&grid_heads[h]);
        while (curr_n != -1) {
            let neighbor_idx = u32(curr_n);
            if (neighbor_idx > idx) { // Sadece bir kere çarpışsınlar (i < j)
                var other = spheres[neighbor_idx];
                
                let dVec = me.position - other.position;
                let distSq = dot(dVec, dVec);
                let min_dist = me.radius + other.radius;
                
                if (distSq < min_dist * min_dist && distSq > 0.0001) {
                    let dist = sqrt(distSq);
                    let n = dVec / dist;
                    let overlap = min_dist - dist;
                    
                    let total_mass = me.mass + other.mass;
                    let m_ratio_me = other.mass / total_mass;
                    let m_ratio_other = me.mass / total_mass;
                    
                    // Positional correction
                    let corr = n * overlap;
                    me.position += corr * m_ratio_me * 0.5;
                    // Note: Ideally we write to 'other.position' but we avoid scatter writes for atomics,
                    // by relying on iterative convergence, or we DO scatter write. 
                    // However, simultaneous scatter writes cause race conditions.
                    // For perfect safety, each thread only updates `me`, and we just process EVERY neighbor 
                    // and only apply half the correction. (Wait, if we process EVERY neighbor, we don't do `> idx` checks!)
                }
            }
            curr_n = linked_nodes[curr_n];
        }
    }}}
    // Actually, due to race conditions writing to `other`, we should only update `me`!
    // Let's rewrite the solver loop properly:
}

// Pass 3 (Race-condition safe version)
@compute @workgroup_size(256)
fn solve_collisions_safe(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    if (idx >= params.num_spheres) { return; }

    var me = spheres[idx];
    let grid_p = vec3<i32>(floor(me.position / CELL_SIZE));
    let restitution = 0.8;
    var acc_pos_correction = vec3<f32>(0.0);
    var acc_vel_correction = vec3<f32>(0.0);

    // Check 27 neighbor cells
    for (var x = -1; x <= 1; x++) {
    for (var y = -1; y <= 1; y++) {
    for (var z = -1; z <= 1; z++) {
        let neighbor_p = grid_p + vec3<i32>(x, y, z);
        let h = (u32(neighbor_p.x * 73856093i) ^ u32(neighbor_p.y * 19349663i) ^ u32(neighbor_p.z * 83492791i)) % GRID_SIZE;
        
        var curr_n = atomicLoad(&grid_heads[h]);
        while (curr_n != -1) {
            let n_idx = u32(curr_n);
            if (n_idx != idx) {
                let other = spheres[n_idx];
                let dVec = me.position - other.position;
                let distSq = dot(dVec, dVec);
                let min_dist = me.radius + other.radius;
                
                if (distSq < min_dist * min_dist && distSq > 0.0001) {
                    let dist = sqrt(distSq);
                    let n = dVec / dist;
                    let overlap = min_dist - dist;
                    
                    let m_ratio_1 = other.mass / (me.mass + other.mass);
                    acc_pos_correction += n * (overlap * m_ratio_1 * 0.5); // 0.5 for stability
                    
                    // Velocity impulse
                    let rel_vel = me.velocity - other.velocity;
                    let vel_along_normal = dot(rel_vel, n);
                    if (vel_along_normal < 0.0) {
                        let j = -(1.0 + restitution) * vel_along_normal;
                        let j_mod = j / (1.0 / me.mass + 1.0 / other.mass);
                        acc_vel_correction += (j_mod * n) / me.mass;
                    }
                }
            }
            curr_n = linked_nodes[curr_n];
        }
    }}}
    
    // Apply corrections once to avoid race-condition write conflicts (mostly)
    me.position += acc_pos_correction;
    me.velocity += acc_vel_correction;
    spheres[idx] = me;
}

// Pass 4
@compute @workgroup_size(256)
fn integrate(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    if (idx >= params.num_spheres) { return; }

    var sphere = spheres[idx];
    
    sphere.velocity += params.gravity * params.dt;
    sphere.velocity *= params.damping;
    sphere.position += sphere.velocity * params.dt;
    
    // Limits
    let bounds = 50.0;
    if (sphere.position.y - sphere.radius < 0.0) {
        sphere.position.y = sphere.radius;
        sphere.velocity.y *= -0.8;
    }
    if (sphere.position.x > bounds) {
        sphere.position.x = bounds;
        sphere.velocity.x *= -0.8;
    } else if (sphere.position.x < -bounds) {
        sphere.position.x = -bounds;
        sphere.velocity.x *= -0.8;
    }
    if (sphere.position.z > bounds) {
        sphere.position.z = bounds;
        sphere.velocity.z *= -0.8;
    } else if (sphere.position.z < -bounds) {
        sphere.position.z = -bounds;
        sphere.velocity.z *= -0.8;
    }
    
    spheres[idx] = sphere;
}
