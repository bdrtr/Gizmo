#define_import_path gizmo::surface

// What a surface is at one fragment, as lighting needs it, and what a fragment knows when it asks.
// No bindings here, on purpose: a game's own surface (`custom_material::SurfaceMaterial`) imports
// these types and may give group 1 a meaning of its own, which it could not if importing them also
// declared the engine's material bindings. Those are `gizmo::material`.

struct Surface {
    // Instance colour × texture: what an unlit or debug view shows.
    base_color: vec3<f32>,
    // `base_color` × vertex colour × AO: what light lands on.
    albedo: vec3<f32>,
    alpha: f32,
    // glTF's Mask threshold from the material's params; 0 = no cut-out.
    alpha_cutoff: f32,
    N: vec3<f32>,
    roughness: f32,
    metallic: f32,
    // Added unlit, in linear HDR.
    emissive: vec3<f32>,
};

// What a game's `material_surface` is handed: everything the engine's own material reads at a
// fragment, and the frame's clock and camera, so that it never has to declare group 0 itself — the
// engine's shader already does, and two declarations of one binding are an error.
struct SurfaceInput {
    // The fragment's position in the world.
    world_position: vec3<f32>,
    // The mesh's texture coordinate, as the vertex carried it: no material transform applied.
    // `gizmo::material::standard_surface` applies the material's own.
    uv: vec2<f32>,
    // The vertex colour.
    vertex_color: vec3<f32>,
    // The geometric normal, tangent and bitangent in the world, interpolated: the front face's,
    // whichever face is drawn. Not unit length after interpolation.
    N: vec3<f32>,
    T: vec3<f32>,
    B: vec3<f32>,
    // `Material::albedo`, `roughness`, `metallic` and `emissive`, for this instance.
    albedo: vec4<f32>,
    roughness: f32,
    metallic: f32,
    emissive: vec3<f32>,
    // Whether the front face is the one drawn. The engine turns the normal round for a back face
    // after `material_surface` returns, so a surface describes its front.
    front_facing: bool,
    // `@builtin(position)`: the fragment's pixel centre and depth.
    frag_coord: vec4<f32>,
    // Seconds since the renderer started: the clock `SceneUniforms` carries.
    time: f32,
    // Where the camera is.
    camera_position: vec3<f32>,
};

struct TangentFrame {
    N: vec3<f32>,
    T: vec3<f32>,
    B: vec3<f32>,
};

// The tangent basis at a vertex, in world space. For the VERTEX stage, for the same reason, and the
// fragment stage interpolates it without renormalising: only the normal it finally builds has to be
// unit length, and its branches and three square roots per fragment are what this saves.
fn tangent_frame(world_normal: vec3<f32>, world_tangent: vec3<f32>, handedness: f32) -> TangentFrame {
    var raw_normal = world_normal;
    if (length(raw_normal) < 0.001) { raw_normal = vec3<f32>(0.0, 1.0, 0.0); }
    let N = normalize(raw_normal);

    var raw_tangent = world_tangent;
    // Fall back to a synthesised tangent when the supplied one is missing OR nearly
    // PARALLEL to the normal. Gram-Schmidt of parallel vectors is normalize(0) = NaN/garbage,
    // which produced a per-pixel-varying TBN → normal-map streaks ONLY on the ±X cube faces,
    // whose default vertex tangent [1,0,0] coincides with the face normal. (Front/back/top/
    // bottom faces have a non-parallel tangent, so they were always clean.)
    let tan_len = length(raw_tangent);
    if (tan_len < 0.001 || abs(dot(raw_tangent / max(tan_len, 1e-8), N)) > 0.999) {
        if (abs(N.x) > 0.9) {
            raw_tangent = cross(vec3<f32>(0.0, 1.0, 0.0), N);
        } else {
            raw_tangent = cross(vec3<f32>(1.0, 0.0, 0.0), N);
        }
    }
    // Gram-Schmidt re-orthogonalise the tangent against the normal.
    let T = normalize(raw_tangent - N * dot(N, raw_tangent));
    var frame: TangentFrame;
    frame.N = N;
    frame.T = T;
    // `select`, not `sign`: `sign(0.0)` is 0, which would zero the bitangent of a mesh whose tangent w is 0.
    frame.B = cross(N, T) * select(-1.0, 1.0, handedness >= 0.0);
    return frame;
}

// Anisotropy, clear coat and subsurface, in that order, from the instance's packed slot. Two digits
// per field — see `pack_pbr_params`. Three needed nine digits, which an f32 cannot hold as an exact
// integer, and the low field carried into its neighbour above 2^24.
fn surface_lobes(inst_pbr: vec4<f32>) -> vec3<f32> {
    let packed = inst_pbr.w - floor(inst_pbr.w / 10000.0) * 10000.0;
    return vec3<f32>(
        (packed - floor(packed / 100.0) * 100.0) / 100.0,
        floor(packed / 100.0) / 100.0,
        floor(inst_pbr.w / 10000.0) / 100.0,
    );
}
