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
    _pad2: vec2<u32>,
}

struct StaticCollider {
    shape_type: u32,
    _pad1: vec3<u32>,
    data1: vec4<f32>,
    data2: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: SimParams;
@group(0) @binding(1) var<storage, read_write> boxes: array<BoxItem>;
@group(0) @binding(2) var<storage, read_write> grid_heads: array<atomic<i32>>; // Size: GRID_SIZE
@group(0) @binding(3) var<storage, read_write> linked_nodes: array<i32>; // Size: num_boxes
@group(0) @binding(4) var<storage, read> colliders: array<StaticCollider>;
@group(0) @binding(5) var<storage, read_write> awake_flags: array<atomic<u32>>;

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



// Pass 3 (Race-condition safe version)
@compute @workgroup_size(256)
fn solve_collisions_safe(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    if (idx >= params.num_boxes) { return; }

    var me = boxes[idx];
    if (me.state == 1u) { return; } // Uyuyan objeler çarpışma testi başlatmaz

    let grid_p = vec3<i32>(floor(me.position / CELL_SIZE));
    let restitution = 0.4;
    let friction = 0.5;
    
    var acc_pos_correction = vec3<f32>(0.0);
    var acc_vel_correction = vec3<f32>(0.0);
    var acc_ang_vel_correction = vec3<f32>(0.0);
    var num_contacts = 0.0;
    
    // OBB Axes for me
    var axesA = array<vec3<f32>, 3>(
        rotate_vector(vec3<f32>(1.0, 0.0, 0.0), me.rotation),
        rotate_vector(vec3<f32>(0.0, 1.0, 0.0), me.rotation),
        rotate_vector(vec3<f32>(0.0, 0.0, 1.0), me.rotation)
    );
    
    // inertia approx (1/6 * m * size^2)
    let sA = me.half_extents * 2.0;
    let invInertiaA = vec3<f32>(
        12.0 / (me.mass * (sA.y * sA.y + sA.z * sA.z)),
        12.0 / (me.mass * (sA.x * sA.x + sA.z * sA.z)),
        12.0 / (me.mass * (sA.x * sA.x + sA.y * sA.y))
    );

    let v_dt = me.velocity * params.dt;
    let rad_x = max(1i, i32(ceil((abs(v_dt.x) + me.half_extents.x) / CELL_SIZE)));
    let rad_y = max(1i, i32(ceil((abs(v_dt.y) + me.half_extents.y) / CELL_SIZE)));
    let rad_z = max(1i, i32(ceil((abs(v_dt.z) + me.half_extents.z) / CELL_SIZE)));
    
    let cx = min(2i, rad_x);
    let cy = min(2i, rad_y);
    let cz = min(2i, rad_z);
    
    // Check neighbor cells
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
                
                // Broadphase (Swept AABB check)
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
                
                // SAT (15 axes)
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
                    var toi = 0.0;
                    if (!is_intersecting) {
                        active_normal = swept_hit_normal;
                        toi = global_t_first;
                        min_overlap = 0.0;
                    }
                    
                    let me_pos_toi = me.position + me.velocity * params.dt * toi;
                    let other_pos_toi = other.position + other.velocity * params.dt * toi;
                    let distVec = other_pos_toi - me_pos_toi;
                    
                    if (dot(active_normal, distVec) < 0.0) {
                        active_normal = -active_normal;
                    }
                    
                    if (is_intersecting && min_overlap > 0.0001) {
                        let total_mass = me.mass + other.mass;
                        let m_ratio_me = other.mass / total_mass;
                        acc_pos_correction += active_normal * (-min_overlap * m_ratio_me * 0.5);
                    }
                    
                    num_contacts += 1.0;
                    
                    let contactPoint = me_pos_toi + active_normal * (length(distVec) * 0.5);
                    let r1 = contactPoint - me.position; 
                    let r2 = contactPoint - other.position;
                    
                    let sB = other.half_extents * 2.0;
                    let invInertiaB = vec3<f32>(
                        12.0 / (other.mass * (sB.y * sB.y + sB.z * sB.z)),
                        12.0 / (other.mass * (sB.x * sB.x + sB.z * sB.z)),
                        12.0 / (other.mass * (sB.x * sB.x + sB.y * sB.y))
                    );

                    let v1 = me.velocity + cross(me.angular_velocity, r1);
                    let v2 = other.velocity + cross(other.angular_velocity, r2);
                    let rel_vel = v1 - v2;
                    let n_b2a = -active_normal;
                    
                    let vel_along_normal = dot(rel_vel, n_b2a);
                    if (vel_along_normal < 0.0) {
                            
                            let invMassA = 1.0 / me.mass;
                            let invMassB = 1.0 / other.mass;
                            
                            let crossA = cross(r1, n_b2a);
                            let crossB = cross(r2, n_b2a);
                            
                            let ptA = crossA * invInertiaA;
                            let ptB = crossB * invInertiaB;
                            
                            let denom = invMassA + invMassB + dot(crossA, ptA) + dot(crossB, ptB);
                            
                            let j = -(1.0 + restitution) * vel_along_normal / denom;
                            let impulse = j * n_b2a;
                            
                            // Friction
                            var tangent = rel_vel - n_b2a * vel_along_normal;
                            let tang_len = length(tangent);
                            if (tang_len > 0.001) {
                                tangent = tangent / tang_len;
                                let crossA_t = cross(r1, tangent);
                                let crossB_t = cross(r2, tangent);
                                let ptA_t = crossA_t * invInertiaA;
                                let ptB_t = crossB_t * invInertiaB;
                                let denom_t = invMassA + invMassB + dot(crossA_t, ptA_t) + dot(crossB_t, ptB_t);
                                let jt = -dot(rel_vel, tangent) / denom_t;
                                
                                var friction_impulse = tangent;
                                if (abs(jt) < j * friction) {
                                    friction_impulse *= jt;
                                } else {
                                    friction_impulse *= -j * friction * sign(dot(rel_vel, tangent));
                                }
                                
                                acc_vel_correction += (impulse + friction_impulse) * invMassA;
                                acc_ang_vel_correction += invInertiaA * cross(r1, impulse + friction_impulse);
                            } else {
                                acc_vel_correction += impulse * invMassA;
                                acc_ang_vel_correction += invInertiaA * cross(r1, impulse);
                            }
                        }
                    }
                }
            curr_n = linked_nodes[curr_n];
        }
    }}}
    
    // Apply corrections using Jacobi averaging to avoid race condition explosion
    if (num_contacts > 0.0) {
        let relaxation = 0.8; // Jacobi relaxation parameter
        me.position += (acc_pos_correction / num_contacts) * relaxation;
        me.velocity += (acc_vel_correction / num_contacts) * relaxation;
        me.angular_velocity += (acc_ang_vel_correction / num_contacts) * relaxation;
    }
    boxes[idx] = me;
}

// Pass 4
@compute @workgroup_size(256)
fn integrate(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    if (idx >= params.num_boxes) { return; }

    var box_struct = boxes[idx];
    
    // Wake Up Tetikleyicisi
    if (atomicExchange(&awake_flags[idx], 0u) == 1u) {
        box_struct.state = 0u;
        box_struct.sleep_counter = 0u;
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
    let invInertia = vec3<f32>(
        12.0 / (box_struct.mass * (sA.y * sA.y + sA.z * sA.z)),
        12.0 / (box_struct.mass * (sA.x * sA.x + sA.z * sA.z)),
        12.0 / (box_struct.mass * (sA.x * sA.x + sA.y * sA.y))
    );
    let invMass = 1.0 / box_struct.mass;

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
                    let ptA = crossA * invInertia;
                    let denom = invMass + dot(crossA, ptA);
                    
                    let j = -(1.0 + restitution) * vel_along_normal / denom;
                    let impulse = j * n;
                    
                    var tangent = v1 - n * vel_along_normal;
                    let tang_len = length(tangent);
                    if (tang_len > 0.001) {
                        tangent = tangent / tang_len;
                        let crossA_t = cross(r1, tangent);
                        let ptA_t = crossA_t * invInertia;
                        let denom_t = invMass + dot(crossA_t, ptA_t);
                        let jt = -dot(v1, tangent) / denom_t;
                        
                        var friction_impulse = tangent;
                        if (abs(jt) < j * friction) {
                            friction_impulse *= jt;
                        } else {
                            friction_impulse *= -j * friction * sign(dot(v1, tangent));
                        }
                        
                        box_struct.velocity += (impulse + friction_impulse) * invMass;
                        box_struct.angular_velocity += invInertia * cross(r1, impulse + friction_impulse);
                    } else {
                        box_struct.velocity += impulse * invMass;
                        box_struct.angular_velocity += invInertia * cross(r1, impulse);
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
                    let ptA = crossA * invInertia;
                    let denom = invMass + dot(crossA, ptA);
                    
                    let j = -(1.0 + restitution) * vel_along_normal / denom;
                    let impulse = j * normal;
                    
                    var tangent = v1 - normal * vel_along_normal;
                    let tang_len = length(tangent);
                    if (tang_len > 0.001) {
                        tangent = tangent / tang_len;
                        let crossA_t = cross(r1, tangent);
                        let ptA_t = crossA_t * invInertia;
                        let denom_t = invMass + dot(crossA_t, ptA_t);
                        let jt = -dot(v1, tangent) / denom_t;
                        
                        var friction_impulse = tangent;
                        if (abs(jt) < j * friction) {
                            friction_impulse *= jt;
                        } else {
                            friction_impulse *= -j * friction * sign(dot(v1, tangent));
                        }
                        
                        box_struct.velocity += (impulse + friction_impulse) * invMass;
                        box_struct.angular_velocity += invInertia * cross(r1, impulse + friction_impulse);
                    } else {
                        box_struct.velocity += impulse * invMass;
                        box_struct.angular_velocity += invInertia * cross(r1, impulse);
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
    // Uykuya Geçiş Kontrolü
    let SLEEP_THRESHOLD: f32 = 0.05;
    if (length(box_struct.velocity) + length(box_struct.angular_velocity) < SLEEP_THRESHOLD) {
        box_struct.sleep_counter++;
        if (box_struct.sleep_counter > 15u) {
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
