// ═══════════════════════════════════════════════════════════════════════
//  Physics Debug Line Renderer
//
//  Compute pass: Joint + Box verilerinden debug çizgi vertex'leri üretir.
//  Render pass:  Çizgileri ekrana çizer.
//
//  Renk kodlaması:
//    🟢 Yeşil   → Joint bağlantıları
//    🔵 Mavi    → Aktif kutu wireframe
//    ⚫ Koyu    → Uyuyan kutu wireframe
//    🟡 Sarı    → Hız vektörleri
//    🔴 Kırmızı → Kırılmış joint
// ═══════════════════════════════════════════════════════════════════════

struct DebugVertex {
    position: vec3<f32>,
    color: u32,  // packed RGBA: R | (G<<8) | (B<<16) | (A<<24)
}

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

struct DebugParams {
    num_boxes: u32,
    num_joints: u32,
    show_wireframes: u32,  // bit0=boxes, bit1=joints, bit2=velocity
    _pad: u32,
}

@group(0) @binding(0) var<uniform> debug_params: DebugParams;
@group(0) @binding(1) var<storage, read> boxes: array<BoxItem>;
@group(0) @binding(2) var<storage, read> joints: array<Joint>;
@group(0) @binding(3) var<storage, read_write> debug_lines: array<DebugVertex>;
@group(0) @binding(4) var<storage, read_write> line_count: atomic<u32>;

fn pack_color(r: f32, g: f32, b: f32, a: f32) -> u32 {
    let ri = u32(clamp(r * 255.0, 0.0, 255.0));
    let gi = u32(clamp(g * 255.0, 0.0, 255.0));
    let bi = u32(clamp(b * 255.0, 0.0, 255.0));
    let ai = u32(clamp(a * 255.0, 0.0, 255.0));
    return ri | (gi << 8u) | (bi << 16u) | (ai << 24u);
}

fn rotate_vec(v: vec3<f32>, q: vec4<f32>) -> vec3<f32> {
    let u = q.xyz;
    let s = q.w;
    return 2.0 * dot(u, v) * u + (s * s - dot(u, u)) * v + 2.0 * s * cross(u, v);
}

fn emit_line(start: vec3<f32>, end: vec3<f32>, color: u32) {
    let vi = atomicAdd(&line_count, 2u); // 2 vertices per line
    let max_vertices = 65536u;  // 32768 lines * 2
    if (vi >= max_vertices) { return; }
    
    debug_lines[vi] = DebugVertex(start, color);
    debug_lines[vi + 1u] = DebugVertex(end, color);
}

// ─── Box Wireframe ───
fn emit_box_wireframe(box_item: BoxItem, color: u32) {
    let e = box_item.half_extents;
    let p = box_item.position;
    let q = box_item.rotation;
    
    // 8 köşe
    var corners = array<vec3<f32>, 8>(
        p + rotate_vec(vec3<f32>(-e.x, -e.y, -e.z), q),
        p + rotate_vec(vec3<f32>( e.x, -e.y, -e.z), q),
        p + rotate_vec(vec3<f32>( e.x,  e.y, -e.z), q),
        p + rotate_vec(vec3<f32>(-e.x,  e.y, -e.z), q),
        p + rotate_vec(vec3<f32>(-e.x, -e.y,  e.z), q),
        p + rotate_vec(vec3<f32>( e.x, -e.y,  e.z), q),
        p + rotate_vec(vec3<f32>( e.x,  e.y,  e.z), q),
        p + rotate_vec(vec3<f32>(-e.x,  e.y,  e.z), q)
    );
    
    // 12 kenar
    emit_line(corners[0], corners[1], color);
    emit_line(corners[1], corners[2], color);
    emit_line(corners[2], corners[3], color);
    emit_line(corners[3], corners[0], color);
    
    emit_line(corners[4], corners[5], color);
    emit_line(corners[5], corners[6], color);
    emit_line(corners[6], corners[7], color);
    emit_line(corners[7], corners[4], color);
    
    emit_line(corners[0], corners[4], color);
    emit_line(corners[1], corners[5], color);
    emit_line(corners[2], corners[6], color);
    emit_line(corners[3], corners[7], color);
}

// ═══ Compute: Debug Çizgi Üretici ═══
@compute @workgroup_size(256)
fn generate_debug_lines(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    
    // Her thread bir kutu işler
    if (idx < debug_params.num_boxes) {
        let box_item = boxes[idx];
        
        // Box wireframe
        if ((debug_params.show_wireframes & 1u) != 0u) {
            var color = pack_color(0.0, 0.8, 1.0, 0.6); // Cyan — aktif
            if (box_item.state == 1u) {
                color = pack_color(0.2, 0.2, 0.3, 0.3);  // Koyu — uyuyan
            }
            emit_box_wireframe(box_item, color);
        }
        
        // Velocity vektörü
        if ((debug_params.show_wireframes & 4u) != 0u) {
            let speed = length(box_item.velocity);
            if (speed > 0.1) {
                let vel_end = box_item.position + box_item.velocity * 0.5;
                let color = pack_color(1.0, 0.9, 0.0, 0.8); // Sarı
                emit_line(box_item.position, vel_end, color);
            }
        }
    }
    
    // Joint çizgileri (sadece ilk thread'ler)
    if (idx < debug_params.num_joints && (debug_params.show_wireframes & 2u) != 0u) {
        let joint = joints[idx];
        if ((joint.flags & 1u) == 0u) { return; } // Inactive
        
        let body_a = boxes[joint.body_a];
        let body_b = boxes[joint.body_b];
        
        let world_a = body_a.position + rotate_vec(joint.anchor_a, body_a.rotation);
        let world_b = body_b.position + rotate_vec(joint.anchor_b, body_b.rotation);
        
        // Joint tipi renkleri
        var color = pack_color(0.0, 1.0, 0.3, 0.9); // Yeşil — Ball
        if (joint.joint_type == 1u) {
            color = pack_color(1.0, 0.5, 0.0, 0.9); // Turuncu — Hinge
        } else if (joint.joint_type == 2u) {
            color = pack_color(0.8, 0.0, 1.0, 0.9); // Mor — Fixed
        } else if (joint.joint_type == 3u) {
            color = pack_color(0.0, 0.5, 1.0, 0.9); // Mavi — Spring
        } else if (joint.joint_type == 4u) {
            color = pack_color(1.0, 1.0, 0.0, 0.9); // Sarı — Slider
        }
        
        // Body merkez → anchor çizgisi
        emit_line(body_a.position, world_a, color);
        // Anchor → anchor bağlantı çizgisi
        emit_line(world_a, world_b, color);
        // Anchor → body merkez çizgisi
        emit_line(world_b, body_b.position, color);
        
        // Hinge: eksen yönü gösterimi
        if (joint.joint_type == 1u) {
            let axis_world = rotate_vec(joint.axis, body_a.rotation);
            let axis_color = pack_color(1.0, 0.8, 0.2, 0.7);
            emit_line(world_a - axis_world * 1.5, world_a + axis_world * 1.5, axis_color);
        }
        
        // Spring: zigzag çizgi
        if (joint.joint_type == 3u) {
            let mid = (world_a + world_b) * 0.5;
            let spring_color = pack_color(0.3, 0.7, 1.0, 0.6);
            let dir = normalize(world_b - world_a);
            let perp = normalize(cross(dir, vec3<f32>(0.0, 1.0, 0.0)));
            emit_line(world_a, mid + perp * 0.3, spring_color);
            emit_line(mid + perp * 0.3, mid - perp * 0.3, spring_color);
            emit_line(mid - perp * 0.3, world_b, spring_color);
        }
    }
}

// ═══ Render: Debug Çizgi Çizici ═══
struct Globals {
    view_proj: mat4x4<f32>,
    camera_pos: vec3<f32>,
    _pad: f32,
}

@group(0) @binding(0) var<uniform> globals: Globals;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: u32,
}

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) frag_color: vec4<f32>,
}

@vertex
fn vs_debug(@location(0) position: vec3<f32>, @location(1) color: u32) -> VertexOutput {
    var out: VertexOutput;
    out.clip_pos = globals.view_proj * vec4<f32>(position, 1.0);
    
    // Renk unpack
    let r = f32(color & 0xFFu) / 255.0;
    let g = f32((color >> 8u) & 0xFFu) / 255.0;
    let b = f32((color >> 16u) & 0xFFu) / 255.0;
    let a = f32((color >> 24u) & 0xFFu) / 255.0;
    out.frag_color = vec4<f32>(r, g, b, a);
    
    return out;
}

@fragment
fn fs_debug(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.frag_color;
}
