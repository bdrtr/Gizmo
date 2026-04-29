// GPU frustum culling for ECS mesh instances.
// Reads world-space bounding spheres, tests against the view-projection frustum,
// and writes instance_count (0 = culled, 1 = visible) to the indirect draw buffer.

struct SceneUniforms {
    view_proj: mat4x4<f32>,
    // remaining fields present in the buffer but unused here
};

struct MeshBounds {
    world_center: vec3<f32>,
    radius:       f32,
};

struct DrawArgs {
    vertex_count:   u32,
    instance_count: u32,
    first_vertex:   u32,
    first_instance: u32,
};

struct CullParams {
    num_instances: u32,
    _pad:          vec3<u32>,
};

@group(0) @binding(0) var<uniform>             scene:     SceneUniforms;
@group(1) @binding(0) var<storage, read>       bounds:    array<MeshBounds>;
@group(1) @binding(1) var<storage, read_write> draw_args: array<DrawArgs>;
@group(1) @binding(2) var<uniform>             params:    CullParams;

fn test_plane(plane: vec4<f32>, center: vec3<f32>, radius: f32) -> bool {
    let n_len = length(plane.xyz);
    if (n_len < 0.0001) { return true; }
    return (dot(plane.xyz, center) + plane.w) >= -radius * n_len;
}

fn frustum_visible(center: vec3<f32>, radius: f32) -> bool {
    let m = scene.view_proj;
    // Extract matrix rows from column-major storage (m[col][row])
    let row0 = vec4<f32>(m[0][0], m[1][0], m[2][0], m[3][0]);
    let row1 = vec4<f32>(m[0][1], m[1][1], m[2][1], m[3][1]);
    let row2 = vec4<f32>(m[0][2], m[1][2], m[2][2], m[3][2]);
    let row3 = vec4<f32>(m[0][3], m[1][3], m[2][3], m[3][3]);
    // Gribb-Hartmann plane extraction (depth range 0..1 for wgpu/Vulkan)
    if (!test_plane(row3 + row0, center, radius)) { return false; } // left
    if (!test_plane(row3 - row0, center, radius)) { return false; } // right
    if (!test_plane(row3 + row1, center, radius)) { return false; } // bottom
    if (!test_plane(row3 - row1, center, radius)) { return false; } // top
    if (!test_plane(row2,         center, radius)) { return false; } // near
    if (!test_plane(row3 - row2, center, radius)) { return false; } // far
    return true;
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= params.num_instances) { return; }
    let b = bounds[i];
    draw_args[i].instance_count = select(0u, 1u, frustum_visible(b.world_center, b.radius));
}
