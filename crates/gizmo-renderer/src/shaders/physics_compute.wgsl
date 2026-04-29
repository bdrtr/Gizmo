struct BoxItem {
    position: vec3<f32>,
    mass: f32,
    velocity: vec3<f32>,
    state: u32,
    rotation: vec4<f32>,
    angular_velocity: vec3<f32>,
    sleep_counter: u32,
    color: vec4<f32>,
    half_extents: vec3<f32>,
    _pad: u32,
}

struct SimParams {
    dt: f32,
    _pad1: vec3<f32>,
    gravity: vec3<f32>,
    damping: f32,
    num_boxes: u32,
    num_colliders: u32,
    num_joints: u32,
    _pad2: u32,
}

// ═══ Joint / Constraint Tipleri ═══
// 0=Ball, 1=Hinge, 2=Fixed, 3=Spring, 4=Slider
struct Joint {
    body_a: u32,
    body_b: u32,
    joint_type: u32,
    flags: u32,
    anchor_a: vec3<f32>,
    compliance: f32,
    anchor_b: vec3<f32>,
    damping_coeff: f32,
    axis: vec3<f32>,
    max_force: f32,
}

struct StaticCollider {
    shape_type: u32,
    _pad1: vec3<u32>,
    data1: vec4<f32>,
    data2: vec4<f32>,
}

struct BoxContacts {
    count: u32,
    _pad: vec3<u32>,
    neighbors: array<u32, 8>,
    normals: array<vec4<f32>, 8>,
    accum_impulse: array<vec4<f32>, 8>,
    is_active: array<u32, 8>,
}

@group(0) @binding(0) var<uniform> params: SimParams;
@group(0) @binding(1) var<storage, read_write> boxes: array<BoxItem>;
@group(0) @binding(2) var<storage, read_write> grid_heads: array<atomic<i32>>; // Size: GRID_SIZE
@group(0) @binding(3) var<storage, read_write> linked_nodes: array<i32>; // Size: num_boxes
@group(0) @binding(4) var<storage, read> colliders: array<StaticCollider>;
@group(0) @binding(5) var<storage, read_write> awake_flags: array<atomic<u32>>;
@group(0) @binding(6) var<storage, read_write> joints: array<Joint>;
@group(0) @binding(7) var<storage, read_write> box_contacts: array<BoxContacts>;

const GRID_SIZE: u32 = 262144u; // 2^18
const CELL_SIZE: f32 = 2.0;

fn quat_mul(q1: vec4<f32>, q2: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(
        q1.w * q2.x + q1.x * q2.w + q1.y * q2.z - q1.z * q2.y,
        q1.w * q2.y - q1.x * q2.z + q1.y * q2.w + q1.z * q2.x,
        q1.w * q2.z + q1.x * q2.y - q1.y * q2.x + q1.z * q2.w,
        q1.w * q2.w - q1.x * q2.x - q1.y * q2.y - q1.z * q2.z
    );
}

fn rotate_vector(v: vec3<f32>, q: vec4<f32>) -> vec3<f32> {
    let u = q.xyz;
    let s = q.w;
    return 2.0 * dot(u, v) * u
         + (s * s - dot(u, u)) * v
         + 2.0 * s * cross(u, v);
}

fn quat_conjugate(q: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(-q.xyz, q.w);
}

/// World-space inverse inertia uygulaması: I_world_inv * v = R * (I_local_inv * (R^-1 * v))
fn apply_inv_inertia(v: vec3<f32>, inv_inertia_local: vec3<f32>, rotation: vec4<f32>) -> vec3<f32> {
    let local_v = rotate_vector(v, quat_conjugate(rotation));
    let scaled = local_v * inv_inertia_local;
    return rotate_vector(scaled, rotation);
}

fn sat_overlap(
    ax: vec3<f32>,
    posA: vec3<f32>, axesA: array<vec3<f32>, 3>, extA: vec3<f32>,
    posB: vec3<f32>, axesB: array<vec3<f32>, 3>, extB: vec3<f32>,
    overlap: ptr<function, f32>
) -> bool {
    if (length(ax) < 0.0001) {
        return true; // zero axis, skip
    }
    let n = normalize(ax);
    
    // Project A
    let projA = extA.x * abs(dot(n, axesA[0])) +
                extA.y * abs(dot(n, axesA[1])) +
                extA.z * abs(dot(n, axesA[2]));
                
    // Project B
    let projB = extB.x * abs(dot(n, axesB[0])) +
                extB.y * abs(dot(n, axesB[1])) +
                extB.z * abs(dot(n, axesB[2]));
                
    let dist = abs(dot(posB - posA, n));
    let o = (projA + projB) - dist;
    
    if (o < 0.0) {
        return false;
    }
    
    *overlap = o;
    return true;
}
fn swept_sat_overlap(
    ax: vec3<f32>,
    posA: vec3<f32>, axesA: array<vec3<f32>, 3>, extA: vec3<f32>,
    posB: vec3<f32>, axesB: array<vec3<f32>, 3>, extB: vec3<f32>,
    rel_vel_dt: vec3<f32>,
    t_first: ptr<function, f32>,
    t_last: ptr<function, f32>,
    hit_normal: ptr<function, vec3<f32>>
) -> bool {
    if (length(ax) < 0.0001) { return true; }
    let n = normalize(ax);
    
    let projA = extA.x * abs(dot(n, axesA[0])) + extA.y * abs(dot(n, axesA[1])) + extA.z * abs(dot(n, axesA[2]));
    let projB = extB.x * abs(dot(n, axesB[0])) + extB.y * abs(dot(n, axesB[1])) + extB.z * abs(dot(n, axesB[2]));
    
    let R = projA + projB;
    let D0 = dot(posB - posA, n);
    let V = dot(rel_vel_dt, n);
    
    var t0 = 0.0;
    var t1 = 0.0;
    
    if (abs(V) < 0.00001) {
        if (abs(D0) > R) { return false; }
        t0 = 0.0;
        t1 = 1.0;
    } else {
        t0 = (-R - D0) / V;
        t1 = ( R - D0) / V;
        if (t0 > t1) {
            let temp = t0; t0 = t1; t1 = temp;
        }
    }
    
    if (t0 > *t_first) { 
        *t_first = t0; 
        *hit_normal = n;
    }
    if (t1 < *t_last)  { 
        *t_last = t1; 
    }
    
    return (*t_first <= *t_last) && (*t_first <= 1.0) && (*t_last >= 0.0);
}


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
    if (idx >= params.num_boxes) { return; }

    let hash = hash_pos(boxes[idx].position);
    let prev = atomicExchange(&grid_heads[hash], i32(idx));
    linked_nodes[idx] = prev;
}



// Pass 3: Narrowphase (Run Once)
@compute @workgroup_size(256)
fn narrowphase(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    if (idx >= params.num_boxes) { return; }

    var me = boxes[idx];
    if (me.state == 1u) { return; }

    let old_count = min(box_contacts[idx].count, 8u);
    for (var i = 0u; i < old_count; i++) {
        box_contacts[idx].is_active[i] = 0u;
    }

    let grid_p = vec3<i32>(floor(me.position / CELL_SIZE));
    
    var axesA = array<vec3<f32>, 3>(
        rotate_vector(vec3<f32>(1.0, 0.0, 0.0), me.rotation),
        rotate_vector(vec3<f32>(0.0, 1.0, 0.0), me.rotation),
        rotate_vector(vec3<f32>(0.0, 0.0, 1.0), me.rotation)
    );
    let v_dt = me.velocity * params.dt;
    let rad_x = max(1i, i32(ceil((abs(v_dt.x) + me.half_extents.x) / CELL_SIZE)));
    let rad_y = max(1i, i32(ceil((abs(v_dt.y) + me.half_extents.y) / CELL_SIZE)));
    let rad_z = max(1i, i32(ceil((abs(v_dt.z) + me.half_extents.z) / CELL_SIZE)));
    
    let cx = min(2i, rad_x);
    let cy = min(2i, rad_y);
    let cz = min(2i, rad_z);
    
    for (var x = -cx; x <= cx; x++) {
    for (var y = -cy; y <= cy; y++) {
    for (var z = -cz; z <= cz; z++) {
        let neighbor_p = grid_p + vec3<i32>(x, y, z);
        let h = (u32(neighbor_p.x * 73856093i) ^ u32(neighbor_p.y * 19349663i) ^ u32(neighbor_p.z * 83492791i)) % GRID_SIZE;
        
        var curr_n = atomicLoad(&grid_heads[h]);
        while (curr_n != -1) {
            let n_idx = u32(curr_n);
            if (n_idx != idx) {
                let other = boxes[n_idx];
                
                // Broadphase
                let rA = length(me.half_extents);
                let rB = length(other.half_extents);
                let minA = min(me.position, me.position + v_dt) - vec3<f32>(rA);
                let maxA = max(me.position, me.position + v_dt) + vec3<f32>(rA);
                let future_other = other.position + other.velocity * params.dt;
                let minB = min(other.position, future_other) - vec3<f32>(rB);
                let maxB = max(other.position, future_other) + vec3<f32>(rB);
                
                if (minA.x > maxB.x || maxA.x < minB.x ||
                    minA.y > maxB.y || maxA.y < minB.y ||
                    minA.z > maxB.z || maxA.z < minB.z) {
                    curr_n = linked_nodes[curr_n];
                    continue;
                }
                
                var axesB = array<vec3<f32>, 3>(
                    rotate_vector(vec3<f32>(1.0, 0.0, 0.0), other.rotation),
                    rotate_vector(vec3<f32>(0.0, 1.0, 0.0), other.rotation),
                    rotate_vector(vec3<f32>(0.0, 0.0, 1.0), other.rotation)
                );
                
                var axes_to_test = array<vec3<f32>, 15>();
                axes_to_test[0] = axesA[0]; axes_to_test[1] = axesA[1]; axes_to_test[2] = axesA[2];
                axes_to_test[3] = axesB[0]; axes_to_test[4] = axesB[1]; axes_to_test[5] = axesB[2];
                axes_to_test[6] = cross(axesA[0], axesB[0]); axes_to_test[7] = cross(axesA[0], axesB[1]); axes_to_test[8] = cross(axesA[0], axesB[2]);
                axes_to_test[9] = cross(axesA[1], axesB[0]); axes_to_test[10] = cross(axesA[1], axesB[1]); axes_to_test[11] = cross(axesA[1], axesB[2]);
                axes_to_test[12] = cross(axesA[2], axesB[0]); axes_to_test[13] = cross(axesA[2], axesB[1]); axes_to_test[14] = cross(axesA[2], axesB[2]);
                
                var min_overlap = 10000.0;
                var hit_normal = vec3<f32>(0.0, 1.0, 0.0);
                var is_intersecting = true;
                
                var is_swept_intersecting = true;
                var global_t_first = 0.0;
                var global_t_last  = 1.0;
                var swept_hit_normal = vec3<f32>(0.0, 1.0, 0.0);
                let rel_vel_dt = (other.velocity - me.velocity) * params.dt;
                
                for(var i = 0u; i < 15u; i++) {
                    var o = 0.0;
                    if (!sat_overlap(axes_to_test[i], me.position, axesA, me.half_extents, other.position, axesB, other.half_extents, &o)) {
                        is_intersecting = false;
                    } else if (o < min_overlap && length(axes_to_test[i]) > 0.0001) {
                        min_overlap = o;
                        hit_normal = normalize(axes_to_test[i]);
                    }
                    if (!swept_sat_overlap(axes_to_test[i], me.position, axesA, me.half_extents, other.position, axesB, other.half_extents, rel_vel_dt, &global_t_first, &global_t_last, &swept_hit_normal)) {
                        is_swept_intersecting = false;
                    }
                }
                
                if (is_intersecting || (is_swept_intersecting && global_t_first <= 1.0 && global_t_first >= 0.0)) {
                    if (other.state == 1u) {
                        atomicStore(&awake_flags[n_idx], 1u);
                    }
                    var active_normal = hit_normal;
                    var penetration = min_overlap;
                    if (!is_intersecting) {
                        active_normal = swept_hit_normal;
                        penetration = 0.001;
                    }
                    let toi = select(0.0, global_t_first, !is_intersecting);
                    let me_pos_toi = me.position + me.velocity * params.dt * toi;
                    let other_pos_toi = other.position + other.velocity * params.dt * toi;
                    let distVec = other_pos_toi - me_pos_toi;
                    
                    if (dot(active_normal, distVec) < 0.0) {
                        active_normal = -active_normal;
                    }

                    // Persistent Manifold Update
                    var found = false;
                    for (var c = 0u; c < old_count; c++) {
                        if (box_contacts[idx].neighbors[c] == n_idx) {
                            box_contacts[idx].is_active[c] = 1u;
                            box_contacts[idx].normals[c] = vec4<f32>(active_normal, penetration);
                            found = true;
                            break;
                        }
                    }

                    if (!found) {
                        var slot = 8u;
                        for (var c = 0u; c < 8u; c++) {
                            if (c >= old_count || box_contacts[idx].is_active[c] == 0u) {
                                slot = c;
                                break;
                            }
                        }
                        if (slot < 8u) {
                            box_contacts[idx].neighbors[slot] = n_idx;
                            box_contacts[idx].normals[slot] = vec4<f32>(active_normal, penetration);
                            box_contacts[idx].accum_impulse[slot] = vec4<f32>(0.0);
                            box_contacts[idx].is_active[slot] = 1u;
                            if (slot >= old_count) {
                                box_contacts[idx].count = slot + 1u;
                            }
                        }
                    }
                }
            }
            curr_n = linked_nodes[curr_n];
        }
    }}}
}

// Pass 3.5: Solver Loop (Iterative Impulse with Warm Starting)
@compute @workgroup_size(256)
fn solve_collisions_safe(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    if (idx >= params.num_boxes) { return; }

    var me = boxes[idx];
    if (me.state == 1u) { return; }

    let num_c = min(box_contacts[idx].count, 8u);
    if (num_c == 0u) { return; }

    let restitution = 0.4;
    let friction = 0.5;
    
    var acc_vel_correction = vec3<f32>(0.0);
    var acc_ang_vel_correction = vec3<f32>(0.0);
    var acc_pos_correction = vec3<f32>(0.0);
    
    let BAUMGARTE_BETA: f32 = 0.2;
    let SLOP: f32 = 0.005;
    let SI_RELAXATION: f32 = 0.65;
    
    let sA = me.half_extents * 2.0;
    var invInertiaA = vec3<f32>(0.0);
    if (me.mass > 0.0) {
        invInertiaA = vec3<f32>(
            12.0 / (me.mass * (sA.y * sA.y + sA.z * sA.z)),
            12.0 / (me.mass * (sA.x * sA.x + sA.z * sA.z)),
            12.0 / (me.mass * (sA.x * sA.x + sA.y * sA.y))
        );
    }
    let rotA = me.rotation;

    for (var i = 0u; i < num_c; i++) {
        if (box_contacts[idx].is_active[i] == 0u) { continue; }

        let n_idx = box_contacts[idx].neighbors[i];
        let nrm_pen = box_contacts[idx].normals[i];
        let active_normal = nrm_pen.xyz;
        let penetration = nrm_pen.w;

        let other = boxes[n_idx];

        let bias_vel = (BAUMGARTE_BETA / params.dt) * max(penetration - SLOP, 0.0);
        if (penetration > 0.05) {
            let total_mass = me.mass + other.mass;
            let m_ratio_me = other.mass / total_mass;
            acc_pos_correction += active_normal * (-penetration * m_ratio_me * 0.3);
        }

        let contactPoint = me.position + active_normal * (length(other.position - me.position) * 0.5);
        let r1 = contactPoint - me.position; 
        let r2 = contactPoint - other.position;
        
        let sB = other.half_extents * 2.0;
        var invInertiaB = vec3<f32>(0.0);
        if (other.mass > 0.0) {
            invInertiaB = vec3<f32>(
                12.0 / (other.mass * (sB.y * sB.y + sB.z * sB.z)),
                12.0 / (other.mass * (sB.x * sB.x + sB.z * sB.z)),
                12.0 / (other.mass * (sB.x * sB.x + sB.y * sB.y))
            );
        }
        let rotB = other.rotation;

        let v1 = me.velocity + cross(me.angular_velocity, r1);
        let v2 = other.velocity + cross(other.angular_velocity, r2);
        let rel_vel = v1 - v2;
        let n_b2a = -active_normal;
        
        let vel_along_normal = dot(rel_vel, n_b2a);
        
        // Persistent Impulse (Warm Start)
        var old_accum = box_contacts[idx].accum_impulse[i];
        
        let invMassA = select(1.0 / me.mass, 0.0, me.mass <= 0.00001);
        let invMassB = select(1.0 / other.mass, 0.0, other.mass <= 0.00001);
        let crossA = cross(r1, n_b2a);
        let crossB = cross(r2, n_b2a);
        let ptA = apply_inv_inertia(crossA, invInertiaA, rotA);
        let ptB = apply_inv_inertia(crossB, invInertiaB, rotB);
        let K = invMassA + invMassB + dot(crossA, ptA) + dot(crossB, ptB);
        
        // Calculate new normal impulse
        let j_normal = -(1.0 + restitution) * vel_along_normal - bias_vel;
        let unprojected_new_accum = old_accum.x + (j_normal / K);
        let new_accum_n = max(unprojected_new_accum, 0.0); // Clamp to positive
        let applied_j_n = new_accum_n - old_accum.x; // Delta impulse
        
        old_accum.x = new_accum_n; // Save back
        let impulse = applied_j_n * n_b2a;
        
        // Friction with Warm Start
        var tangent = rel_vel - n_b2a * vel_along_normal;
        let tang_len = length(tangent);
        if (tang_len > 0.001) {
            tangent = tangent / tang_len;
            let crossA_t = cross(r1, tangent);
            let crossB_t = cross(r2, tangent);
            let ptA_t = apply_inv_inertia(crossA_t, invInertiaA, rotA);
            let ptB_t = apply_inv_inertia(crossB_t, invInertiaB, rotB);
            let K_t = invMassA + invMassB + dot(crossA_t, ptA_t) + dot(crossB_t, ptB_t);
            
            let jt = -dot(rel_vel, tangent);
            let unprojected_jt_accum = old_accum.yzw + (tangent * (jt / K_t));
            let max_fric = friction * new_accum_n;
            
            // Clamp tangent vector
            let current_fric_len = length(unprojected_jt_accum);
            var new_accum_t = unprojected_jt_accum;
            if (current_fric_len > max_fric) {
                new_accum_t = unprojected_jt_accum * (max_fric / current_fric_len);
            }
            
            let applied_j_t = new_accum_t - old_accum.yzw;
            old_accum.y = new_accum_t.x;
            old_accum.z = new_accum_t.y;
            old_accum.w = new_accum_t.z;
            
            acc_vel_correction += (impulse + applied_j_t) * invMassA;
            acc_ang_vel_correction += apply_inv_inertia(cross(r1, impulse + applied_j_t), invInertiaA, rotA);
        } else {
            acc_vel_correction += impulse * invMassA;
            acc_ang_vel_correction += apply_inv_inertia(cross(r1, impulse), invInertiaA, rotA);
        }
        
        // Save the accumulated impulse for next iteration/frame
        box_contacts[idx].accum_impulse[i] = old_accum;
    }
    
    me.velocity += acc_vel_correction * SI_RELAXATION;
    me.angular_velocity += acc_ang_vel_correction * SI_RELAXATION;
    
    var active_count = 0.0;
    for (var i = 0u; i < num_c; i++) {
        if (box_contacts[idx].is_active[i] == 1u) {
            active_count += 1.0;
        }
    }
    
    if (active_count > 0.0 && length(acc_pos_correction) > 0.0001) {
        me.position += (acc_pos_correction / active_count);
    }
    boxes[idx] = me;
}

// Pass 4
@compute @workgroup_size(256)
fn integrate(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    if (idx >= params.num_boxes) { return; }

    var box_struct = boxes[idx];
    
    // Wake Up Tetikleyicisi — cascade wake-up dahil
    if (atomicExchange(&awake_flags[idx], 0u) == 1u) {
        box_struct.state = 0u;
        box_struct.sleep_counter = 0u;
        
        // Cascade: bu nesne uyandığında, aynı hücredeki uyuyan komşularını da uyandır
        let wake_hash = hash_pos(box_struct.position);
        var wake_curr = atomicLoad(&grid_heads[wake_hash]);
        while (wake_curr != -1) {
            let wake_idx = u32(wake_curr);
            if (wake_idx != idx && boxes[wake_idx].state == 1u) {
                let dist_sq = dot(
                    box_struct.position - boxes[wake_idx].position,
                    box_struct.position - boxes[wake_idx].position
                );
                // Sadece yakın komşuları uyandır (2 × CELL_SIZE mesafe)
                if (dist_sq < CELL_SIZE * CELL_SIZE * 4.0) {
                    atomicStore(&awake_flags[wake_idx], 1u);
                }
            }
            wake_curr = linked_nodes[wake_curr];
        }
    }
    
    if (box_struct.state == 1u) {
        boxes[idx] = box_struct;
        return; // Uyuyorsan fizik uygulanmaz!
    }
    
    box_struct.velocity += params.gravity * params.dt;
    box_struct.velocity *= params.damping;
    box_struct.position += box_struct.velocity * params.dt;
    
    // Angular integration
    box_struct.angular_velocity *= params.damping;
    let w = vec4<f32>(box_struct.angular_velocity.x, box_struct.angular_velocity.y, box_struct.angular_velocity.z, 0.0);
    let dq = quat_mul(w, box_struct.rotation);
    box_struct.rotation.x += 0.5 * dq.x * params.dt;
    box_struct.rotation.y += 0.5 * dq.y * params.dt;
    box_struct.rotation.z += 0.5 * dq.z * params.dt;
    box_struct.rotation.w += 0.5 * dq.w * params.dt;
    box_struct.rotation = normalize(box_struct.rotation);
    
    let restitution = 0.4;
    let friction = 0.5;
    
    let sA = box_struct.half_extents * 2.0;
    var invInertia = vec3<f32>(0.0);
    if (box_struct.mass > 0.00001) {
        invInertia = vec3<f32>(
            12.0 / (box_struct.mass * (sA.y * sA.y + sA.z * sA.z)),
            12.0 / (box_struct.mass * (sA.x * sA.x + sA.z * sA.z)),
            12.0 / (box_struct.mass * (sA.x * sA.x + sA.y * sA.y))
        );
    }
    let invMass = select(1.0 / box_struct.mass, 0.0, box_struct.mass <= 0.00001);
    let rot = box_struct.rotation;

    // Check against all static colliders
    for (var i = 0u; i < params.num_colliders; i++) {
        let col = colliders[i];
        
        let axes0 = rotate_vector(vec3<f32>(1.0, 0.0, 0.0), box_struct.rotation);
        let axes1 = rotate_vector(vec3<f32>(0.0, 1.0, 0.0), box_struct.rotation);
        let axes2 = rotate_vector(vec3<f32>(0.0, 0.0, 1.0), box_struct.rotation);
            
        if (col.shape_type == 0u) {
            // AABB vs Box
            let min_b = col.data1.xyz;
            let max_b = col.data2.xyz;
            let static_pos = (min_b + max_b) * 0.5;
            let static_ext = (max_b - min_b) * 0.5;
            
            let ext_ws = vec3<f32>(
                box_struct.half_extents.x * abs(axes0.x) + box_struct.half_extents.y * abs(axes1.x) + box_struct.half_extents.z * abs(axes2.x),
                box_struct.half_extents.x * abs(axes0.y) + box_struct.half_extents.y * abs(axes1.y) + box_struct.half_extents.z * abs(axes2.y),
                box_struct.half_extents.x * abs(axes0.z) + box_struct.half_extents.y * abs(axes1.z) + box_struct.half_extents.z * abs(axes2.z)
            );
            
            let minkowski_min = min_b - ext_ws;
            let minkowski_max = max_b + ext_ws;
            let old_pos = box_struct.position - box_struct.velocity * params.dt;
            let ray_dir = box_struct.velocity * params.dt;
            
            var t_min = 0.0;
            var t_max = 1.0;
            var swept_n = vec3<f32>(0.0, 1.0, 0.0);
            var is_hit = true;
            
            for(var a = 0; a < 3; a++) {
                if (abs(ray_dir[a]) < 0.0001) {
                    if (old_pos[a] < minkowski_min[a] || old_pos[a] > minkowski_max[a]) { is_hit = false; break; }
                } else {
                    let ood = 1.0 / ray_dir[a];
                    var t1 = (minkowski_min[a] - old_pos[a]) * ood;
                    var t2 = (minkowski_max[a] - old_pos[a]) * ood;
                    var n1 = vec3<f32>(0.0); n1[a] = -1.0;
                    var n2 = vec3<f32>(0.0); n2[a] = 1.0;
                    if (t1 > t2) {
                        let tmp = t1; t1 = t2; t2 = tmp;
                        let t_n = n1; n1 = n2; n2 = t_n;
                    }
                    if (t1 > t_min) { t_min = t1; swept_n = n1; }
                    if (t2 < t_max) { t_max = t2; }
                    if (t_min > t_max) { is_hit = false; break; }
                }
            }
            
            let dVec = box_struct.position - static_pos;
            let overlap = (ext_ws + static_ext) - abs(dVec);
            
            var apply_static = (overlap.x > 0.0 && overlap.y > 0.0 && overlap.z > 0.0);
            var apply_swept = (is_hit && t_min <= 1.0 && t_min >= 0.0 && !apply_static);
            
            if (apply_static || apply_swept) {
                var n = vec3<f32>(0.0);
                var r1_dist = 0.0;
                
                if (apply_static) {
                    n = vec3<f32>(sign(dVec.x), 0.0, 0.0);
                    var min_overlap = overlap.x;
                    r1_dist = ext_ws.x;
                    
                    if (overlap.y < min_overlap) {
                        min_overlap = overlap.y;
                        n = vec3<f32>(0.0, sign(dVec.y), 0.0);
                        r1_dist = ext_ws.y;
                    }
                    if (overlap.z < min_overlap) {
                        min_overlap = overlap.z;
                        n = vec3<f32>(0.0, 0.0, sign(dVec.z));
                        r1_dist = ext_ws.z;
                    }
                    box_struct.position += n * min_overlap;
                } else {
                    n = swept_n;
                    box_struct.position = old_pos + ray_dir * t_min;
                    r1_dist = ext_ws.x * abs(n.x) + ext_ws.y * abs(n.y) + ext_ws.z * abs(n.z);
                }
                
                let r1 = -n * r1_dist;
                
                let v1 = box_struct.velocity + cross(box_struct.angular_velocity, r1);
                let vel_along_normal = dot(v1, n);
                
                if (vel_along_normal < 0.0) {
                    let crossA = cross(r1, n);
                    let ptA = apply_inv_inertia(crossA, invInertia, rot);
                    let denom = invMass + dot(crossA, ptA);
                    
                    let j = -(1.0 + restitution) * vel_along_normal / denom;
                    let impulse = j * n;
                    
                    var tangent = v1 - n * vel_along_normal;
                    let tang_len = length(tangent);
                    if (tang_len > 0.001) {
                        tangent = tangent / tang_len;
                        let crossA_t = cross(r1, tangent);
                        let ptA_t = apply_inv_inertia(crossA_t, invInertia, rot);
                        let denom_t = invMass + dot(crossA_t, ptA_t);
                        let jt = -dot(v1, tangent) / denom_t;
                        
                        var friction_impulse = tangent;
                        if (abs(jt) < j * friction) {
                            friction_impulse *= jt;
                        } else {
                            friction_impulse *= -j * friction * sign(dot(v1, tangent));
                        }
                        
                        box_struct.velocity += (impulse + friction_impulse) * invMass;
                        box_struct.angular_velocity += apply_inv_inertia(cross(r1, impulse + friction_impulse), invInertia, rot);
                    } else {
                        box_struct.velocity += impulse * invMass;
                        box_struct.angular_velocity += apply_inv_inertia(cross(r1, impulse), invInertia, rot);
                    }
                }
            }
        } else if (col.shape_type == 1u) {
            // Plane
            let normal = col.data1.xyz; // points outward
            let d = col.data2.x;
            
            let dist = dot(box_struct.position, normal) + d;
            let proj_r = box_struct.half_extents.x * abs(dot(axes0, normal)) + 
                         box_struct.half_extents.y * abs(dot(axes1, normal)) + 
                         box_struct.half_extents.z * abs(dot(axes2, normal));
            
            if (dist < proj_r) {
                let overlap = proj_r - dist;
                box_struct.position += normal * overlap;
                
                let r1 = -normal * proj_r;
                let v1 = box_struct.velocity + cross(box_struct.angular_velocity, r1);
                let vel_along_normal = dot(v1, normal);
                
                if (vel_along_normal < 0.0) {
                    let crossA = cross(r1, normal);
                    let ptA = apply_inv_inertia(crossA, invInertia, rot);
                    let denom = invMass + dot(crossA, ptA);
                    
                    let j = -(1.0 + restitution) * vel_along_normal / denom;
                    let impulse = j * normal;
                    
                    var tangent = v1 - normal * vel_along_normal;
                    let tang_len = length(tangent);
                    if (tang_len > 0.001) {
                        tangent = tangent / tang_len;
                        let crossA_t = cross(r1, tangent);
                        let ptA_t = apply_inv_inertia(crossA_t, invInertia, rot);
                        let denom_t = invMass + dot(crossA_t, ptA_t);
                        let jt = -dot(v1, tangent) / denom_t;
                        
                        var friction_impulse = tangent;
                        if (abs(jt) < j * friction) {
                            friction_impulse *= jt;
                        } else {
                            friction_impulse *= -j * friction * sign(dot(v1, tangent));
                        }
                        
                        box_struct.velocity += (impulse + friction_impulse) * invMass;
                        box_struct.angular_velocity += apply_inv_inertia(cross(r1, impulse + friction_impulse), invInertia, rot);
                    } else {
                        box_struct.velocity += impulse * invMass;
                        box_struct.angular_velocity += apply_inv_inertia(cross(r1, impulse), invInertia, rot);
                    }
                }
            }
        }
    }
    
    // Bounds limit (World Limits x, z)
    let bounds = 150.0;
    if (box_struct.position.x > bounds) {
        box_struct.position.x = bounds;
        box_struct.velocity.x *= -0.8;
    } else if (box_struct.position.x < -bounds) {
        box_struct.position.x = -bounds;
        box_struct.velocity.x *= -0.8;
    }
    if (box_struct.position.z > bounds) {
        box_struct.position.z = bounds;
        box_struct.velocity.z *= -0.8;
    } else if (box_struct.position.z < -bounds) {
        box_struct.position.z = -bounds;
        box_struct.velocity.z *= -0.8;
    }
    // ═══ Gelişmiş Uyku Sistemi (Energy-Based Island Sleeping) ═══
    //
    // Kinetik enerji tabanlı eşik: E = 0.5 * m * v² + 0.5 * I * ω²
    // Basit hız eşiği yerine enerji eşiği kullanılır → kütle farklılıklarına duyarsız
    //
    let linear_energy = 0.5 * box_struct.mass * dot(box_struct.velocity, box_struct.velocity);
    let angular_speed_sq = dot(box_struct.angular_velocity, box_struct.angular_velocity);
    // Basitleştirilmiş rotasyonel enerji (ortalama inertia yaklaşımı)
    let avg_inertia = box_struct.mass * (sA.x * sA.x + sA.y * sA.y + sA.z * sA.z) / 12.0;
    let angular_energy = 0.5 * avg_inertia * angular_speed_sq;
    let total_energy = linear_energy + angular_energy;

    let SLEEP_ENERGY_THRESHOLD: f32 = 0.0005;  // Çok düşük enerji eşiği
    let SLEEP_FRAMES: u32 = 120u;              // 2 saniye @ 60fps
    let SETTLING_FRAMES: u32 = 60u;            // Uyumadan önce 1s yavaşlatma

    if (total_energy < SLEEP_ENERGY_THRESHOLD) {
        box_struct.sleep_counter += 1u;

        // Settling fazı: uykuya geçmeden önce hızları kademeli azalt (jitter önleme)
        if (box_struct.sleep_counter > SETTLING_FRAMES) {
            let settle_factor = 0.9;  // Her frame %10 sönümleme
            box_struct.velocity *= settle_factor;
            box_struct.angular_velocity *= settle_factor;
        }

        // Tam uyku
        if (box_struct.sleep_counter > SLEEP_FRAMES) {
            box_struct.state = 1u;
            box_struct.velocity = vec3<f32>(0.0);
            box_struct.angular_velocity = vec3<f32>(0.0);
        }
    } else {
        box_struct.sleep_counter = 0u;
        box_struct.state = 0u;
    }

    boxes[idx] = box_struct;
}

// ═══════════════════════════════════════════════════════════════════════
//  Pass 5: Joint / Constraint Solver
//
//  Body-centric: her thread bir gövde işler, tüm joint'leri tarar.
//  Race condition yok çünkü her thread sadece kendi gövdesini yazar.
//  Baumgarte bias ile pozisyon düzeltme, XPBD compliance ile yumuşak
//  constraint desteği.
// ═══════════════════════════════════════════════════════════════════════
const JOINT_BETA: f32 = 0.3;  // Joint pozisyon düzeltme oranı (daha agresif)

fn compute_inv_inertia_diag(body: BoxItem) -> vec3<f32> {
    if (body.mass <= 0.00001) { return vec3<f32>(0.0); }
    let s = body.half_extents * 2.0;
    return vec3<f32>(
        12.0 / (body.mass * (s.y * s.y + s.z * s.z)),
        12.0 / (body.mass * (s.x * s.x + s.z * s.z)),
        12.0 / (body.mass * (s.x * s.x + s.y * s.y))
    );
}

// Tek eksen boyunca pozisyonel constraint çözücü
fn solve_positional_axis(
    n: vec3<f32>,
    err: f32,
    r_a: vec3<f32>,
    r_b: vec3<f32>,
    body_a: BoxItem,
    body_b: BoxItem,
    inv_inertia_a: vec3<f32>,
    inv_inertia_b: vec3<f32>,
    compliance: f32,
    damping_c: f32,
    is_body_a: bool,
) -> vec4<f32> { // xyz = velocity correction, w = angular correction magnitude
    let inv_mass_a = select(1.0 / body_a.mass, 0.0, body_a.mass <= 0.00001);
    let inv_mass_b = select(1.0 / body_b.mass, 0.0, body_b.mass <= 0.00001);
    
    let cross_a = cross(r_a, n);
    let cross_b = cross(r_b, n);
    let pt_a = apply_inv_inertia(cross_a, inv_inertia_a, body_a.rotation);
    let pt_b = apply_inv_inertia(cross_b, inv_inertia_b, body_b.rotation);
    
    let K = inv_mass_a + inv_mass_b + dot(cross_a, pt_a) + dot(cross_b, pt_b);
    
    // XPBD compliance: α̃ = α / (dt²)
    let alpha_tilde = compliance / (params.dt * params.dt);
    let effective_mass = 1.0 / (K + alpha_tilde);
    
    // Baumgarte bias + damping
    let bias = (JOINT_BETA / params.dt) * err;
    
    let v_a = body_a.velocity + cross(body_a.angular_velocity, r_a);
    let v_b = body_b.velocity + cross(body_b.angular_velocity, r_b);
    let v_rel = dot(v_a - v_b, n);
    
    let lambda = -(v_rel + bias + damping_c * v_rel) * effective_mass;
    
    let impulse = n * lambda;
    
    if (is_body_a) {
        return vec4<f32>(
            impulse * inv_mass_a,
            dot(apply_inv_inertia(cross(r_a, impulse), inv_inertia_a, body_a.rotation), n)
        );
    } else {
        return vec4<f32>(
            -impulse * inv_mass_b,
            dot(-apply_inv_inertia(cross(r_b, impulse), inv_inertia_b, body_b.rotation), n)
        );
    }
}

@compute @workgroup_size(256)
fn solve_joints(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let body_idx = global_id.x;
    if (body_idx >= params.num_boxes) { return; }
    if (params.num_joints == 0u) { return; }
    
    var body = boxes[body_idx];
    if (body.state == 1u) { return; } // Uyuyan gövde
    
    var acc_vel = vec3<f32>(0.0);
    var acc_ang = vec3<f32>(0.0);
    var joint_count = 0.0;
    
    let inv_inertia_self = compute_inv_inertia_diag(body);
    
    for (var j = 0u; j < params.num_joints; j++) {
        let joint = joints[j];
        if ((joint.flags & 1u) == 0u) { continue; } // Inactive
        
        let is_a = (joint.body_a == body_idx);
        let is_b = (joint.body_b == body_idx);
        if (!is_a && !is_b) { continue; }
        
        // Diğer gövdeyi al
        var other_idx = joint.body_b;
        if (is_b) { other_idx = joint.body_a; }
        
        let other = boxes[other_idx];
        let inv_inertia_other = compute_inv_inertia_diag(other);
        
        // Uyuyan diğer gövdeyi uyandır
        if (other.state == 1u) {
            atomicStore(&awake_flags[other_idx], 1u);
        }
        
        // Dünya-uzayı bağlantı noktaları
        let r_a = rotate_vector(joint.anchor_a, boxes[joint.body_a].rotation);
        let r_b = rotate_vector(joint.anchor_b, boxes[joint.body_b].rotation);
        let w_a = boxes[joint.body_a].position + r_a;
        let w_b = boxes[joint.body_b].position + r_b;
        
        // Pozisyon hatası
        let delta = w_a - w_b;
        let err = length(delta);
        
        // ─── Joint Tiplerine Göre Çözüm ───
        
        if (joint.joint_type == 0u || joint.joint_type == 2u || joint.joint_type == 3u) {
            // BALL (0), FIXED (2), SPRING (3) — Pozisyonel constraint
            if (err > 0.0001) {
                let n = delta / err;
                let bias = (JOINT_BETA / params.dt) * err;
                
                let inv_mass_self = select(1.0 / body.mass, 0.0, body.mass <= 0.00001);
                let inv_mass_other = select(1.0 / other.mass, 0.0, other.mass <= 0.00001);
                
                let r_self = select(r_b, r_a, is_a);
                let r_other = select(r_a, r_b, is_a);
                let inv_i_self = select(inv_inertia_other, inv_inertia_self, is_a);
                let inv_i_other = select(inv_inertia_self, inv_inertia_other, is_a);
                let rot_self = select(other.rotation, body.rotation, is_a);
                let rot_other = select(body.rotation, other.rotation, is_a);
                
                let cross_s = cross(r_self, n);
                let cross_o = cross(r_other, n);
                let pt_s = apply_inv_inertia(cross_s, inv_i_self, rot_self);
                let pt_o = apply_inv_inertia(cross_o, inv_i_other, rot_other);
                
                let K = inv_mass_self + inv_mass_other + dot(cross_s, pt_s) + dot(cross_o, pt_o);
                
                // XPBD compliance
                let alpha_tilde = joint.compliance / (params.dt * params.dt);
                
                let v_self = body.velocity + cross(body.angular_velocity, r_self);
                let v_other = other.velocity + cross(other.angular_velocity, r_other);
                let v_rel_sign = select(-1.0, 1.0, is_a);
                let v_rel = dot(v_self - v_other, n) * v_rel_sign;
                
                let lambda = -(v_rel + bias * v_rel_sign + joint.damping_coeff * v_rel) / (K + alpha_tilde);
                let impulse_sign = select(-1.0, 1.0, is_a);
                let impulse = n * lambda * impulse_sign;
                
                acc_vel += impulse * inv_mass_self;
                acc_ang += apply_inv_inertia(cross(r_self, impulse), inv_i_self, rot_self);
                joint_count += 1.0;
            }
            
            // FIXED joint: Açısal constraint ekle (tüm rotasyon kısıtlı)
            if (joint.joint_type == 2u) {
                // Gövdeler arası göreceli dönme hatası
                let q_a = boxes[joint.body_a].rotation;
                let q_b = boxes[joint.body_b].rotation;
                let q_err = quat_mul(q_a, quat_conjugate(q_b));
                // Hata quaternion'dan açısal hata vektörü: 2 * vec(q_err) (küçük açılar için)
                let ang_err = q_err.xyz * 2.0 * select(-1.0, 1.0, q_err.w >= 0.0);
                
                let ang_bias = (JOINT_BETA / params.dt) * ang_err;
                let omega_rel = body.angular_velocity - other.angular_velocity;
                let omega_sign = select(-1.0, 1.0, is_a);
                
                let avg_inv_i = (inv_inertia_self + inv_inertia_other) * 0.5;
                let correction = -(omega_rel * omega_sign + ang_bias * omega_sign) * avg_inv_i * 0.3;
                acc_ang += correction;
            }
        }
        
        if (joint.joint_type == 1u) {
            // HINGE — Ball constraint + 2-axis angular lock
            // Pozisyonel kısım (Ball gibi)
            if (err > 0.0001) {
                let n = delta / err;
                let bias = (JOINT_BETA / params.dt) * err;
                let inv_mass_self = select(1.0 / body.mass, 0.0, body.mass <= 0.00001);
                let inv_mass_other = select(1.0 / other.mass, 0.0, other.mass <= 0.00001);
                let r_self = select(r_b, r_a, is_a);
                let K = inv_mass_self + inv_mass_other;
                let v_rel_sign = select(-1.0, 1.0, is_a);
                let lambda = -(bias * v_rel_sign) / K;
                let impulse = n * lambda * v_rel_sign;
                acc_vel += impulse * inv_mass_self;
                joint_count += 1.0;
            }
            
            // Açısal kısım: Hinge ekseni dışındaki dönmeyi kısıtla
            let hinge_axis_world = rotate_vector(joint.axis, boxes[joint.body_a].rotation);
            let omega_rel = body.angular_velocity - other.angular_velocity;
            let omega_sign = select(-1.0, 1.0, is_a);
            
            // Hinge eksenine dik bileşenleri kısıtla
            let omega_along = dot(omega_rel, hinge_axis_world) * hinge_axis_world;
            let omega_perp = omega_rel - omega_along;
            
            acc_ang -= omega_perp * 0.5 * omega_sign;
        }
        
        if (joint.joint_type == 4u) {
            // SLIDER — Eksen boyunca serbest, diğer yönler kısıtlı
            let slide_axis_world = rotate_vector(joint.axis, boxes[joint.body_a].rotation);
            
            // Eksen dışı pozisyon bileşenini kısıtla
            let along = dot(delta, slide_axis_world) * slide_axis_world;
            let perp = delta - along;
            let perp_err = length(perp);
            
            if (perp_err > 0.0001) {
                let n = perp / perp_err;
                let bias = (JOINT_BETA / params.dt) * perp_err;
                let inv_mass_self = 1.0 / body.mass;
                let K = inv_mass_self + 1.0 / other.mass;
                let sign = select(-1.0, 1.0, is_a);
                let lambda = -(bias * sign) / K;
                acc_vel += n * lambda * sign * inv_mass_self;
                joint_count += 1.0;
            }
            
            // Tüm dönmeyi kısıtla
            let omega_rel = body.angular_velocity - other.angular_velocity;
            let omega_sign = select(-1.0, 1.0, is_a);
            acc_ang -= omega_rel * 0.4 * omega_sign;
        }
        
        // Kırılma kontrolü
        if ((joint.flags & 2u) != 0u && joint.max_force > 0.0) {
            let force_mag = length(acc_vel) * body.mass / params.dt;
            if (force_mag > joint.max_force) {
                // Joint'i deaktive et
                joints[j].flags = 0u;
                acc_vel = vec3<f32>(0.0);
                acc_ang = vec3<f32>(0.0);
            }
        }
    }
    
    // Düzeltmeleri uygula
    if (joint_count > 0.0) {
        body.velocity += acc_vel * 0.7;  // Relaxation
        body.angular_velocity += acc_ang * 0.7;
        boxes[body_idx] = body;
    }
}
