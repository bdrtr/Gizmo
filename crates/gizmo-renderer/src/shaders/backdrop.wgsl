// A PAINTED BACKDROP: the scene's own sky / panorama geometry, drawn from ITS OWN texture and
// vertex colour.
//
// The engine had two materials that each got half of this. `sky.wgsl` gets the DEPTH right —
// it pins NDC z to the far plane so it can never occlude anything — but it contains no
// `textureSample` at all: it throws the mesh's texture and vertex colour away and invents an
// atmospheric gradient out of the sun's colour. `unlit.wgsl` gets the PIXELS right (vertex
// colour × instance albedo × texture) but is ordinary world geometry: it writes depth and it
// is not locked to the camera, so a backdrop panel draws IN FRONT of the world it is meant to
// sit behind. This shader is the union, and the three properties it must hold are:
//
//   1. drawn before the world  — the draw-order rank (`DrawLayer::Backdrop`), on the CPU.
//   2. locked to the camera    — here, in `vs_main`: the camera's translation is cancelled and
//                                its rotation kept.
//   3. never writes depth      — the pipeline (`backdrop::backdrop_state`), plus the far-plane
//                                pin below as a second line of defence.
//
// See `crate::backdrop` for the CPU mirror of the vertex maths and for the tests that pin all
// of the above without a GPU adapter.
#import gizmo::common::{SceneUniforms}

@group(0) @binding(0)
var<uniform> scene: SceneUniforms;

@group(1) @binding(0)
var t_diffuse: texture_2d<f32>;
@group(1) @binding(1)
var s_diffuse: sampler;

// The shadow group (native slot 2) and the skeleton group are part of the shared forward
// pipeline layout but are deliberately NOT declared here: a backdrop receives no shadow and is
// never skinned. A pipeline layout may declare more groups than a shader uses.

struct InstanceRaw {
    model_matrix_0: vec4<f32>,
    model_matrix_1: vec4<f32>,
    model_matrix_2: vec4<f32>,
    model_matrix_3: vec4<f32>,
    albedo_color: vec4<f32>,
    pbr: vec4<f32>,
    // Mirrors `gpu_types::InstanceRaw`. Unread here, but every shader indexing this buffer must
    // declare all eight slots or its element stride is wrong for every instance after the first.
    ambient: vec4<f32>,
    emissive: vec4<f32>,
};

@group(#{INSTANCE_GROUP}) @binding(0)
var<storage, read> instances: array<InstanceRaw>;

// Mirror of `backdrop::BACKDROP_NDC_DEPTH`. Just short of the 1.0 far plane so the backdrop
// still passes the `LessEqual` test against a cleared depth buffer, while losing it to every
// piece of world geometry in front of it. The Rust-side test
// `the_shader_still_computes_the_mirrored_expressions` (in `backdrop.rs`) fails if the two
// drift apart.
const BACKDROP_NDC_DEPTH: f32 = 0.99999;

struct VertexInput {
    @location(0) position: vec3<f32>,
    // RGBA, taken at face value — see `fs_main`.
    @location(1) color: vec4<f32>,
    @location(3) tex_coords: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) tex_coords: vec2<f32>,
    @location(2) inst_albedo: vec4<f32>,
};

@vertex
fn vs_main(@builtin(instance_index) instance_idx: u32, input: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    let inst = instances[instance_idx];
    let model = mat4x4<f32>(
        inst.model_matrix_0,
        inst.model_matrix_1,
        inst.model_matrix_2,
        inst.model_matrix_3,
    );

    // ── (2) CAMERA LOCK: translation removed, rotation kept ────────────────────────────────
    // Adding the camera's world position before the view transform cancels that transform's
    // translation exactly, and nothing else. The view matrix is `V = R · T(−c)`, so
    // `V · (x + c) = R · (x + c − c) = R · x`: the backdrop turns with the camera and never
    // slides past it, however far from the origin the camera travels. Doing it here rather
    // than by moving the entity every frame keeps the authored transform untouched — which is
    // also what lets the CPU predict where a backdrop lands (`backdrop::camera_locked_model`).
    let local = (model * vec4<f32>(input.position, 1.0)).xyz;
    let world = local + scene.camera_pos.xyz;
    var clip = scene.view_proj * vec4<f32>(world, 1.0);

    // ── (3) DEPTH: pinned to the far plane ─────────────────────────────────────────────────
    // The pipeline already disables depth WRITES; this disables winning the depth TEST. Both
    // are needed: the forward pass runs after the deferred G-buffer has filled the depth
    // buffer, so a panel that is physically nearer than a wall would otherwise pass
    // `LessEqual` and paint over it — which is exactly what `Unlit` did.
    clip.z = clip.w * BACKDROP_NDC_DEPTH;

    out.clip_position = clip;
    out.color = input.color;
    out.tex_coords = input.tex_coords;
    out.inst_albedo = inst.albedo_color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // The pixels the mesh actually carries: its texture, its vertex colour, and the material's
    // albedo tint. No lighting, no invented gradient — a backdrop is a painting.
    //
    // The vertex colour is taken as authored. It is not tested for "looks unset and therefore
    // must be white": whether the attribute exists is a property of the vertex layout, not of
    // a pixel value, and this layout always has it (a source with no colours is normalised to
    // opaque white at import — `gpu_types::Vertex::default`). Under such a test a panel the
    // author painted black would come out white.
    let tex = textureSample(t_diffuse, s_diffuse, in.tex_coords);
    let rgb = in.color.rgb * in.inst_albedo.rgb * tex.rgb;
    let alpha = in.color.a * in.inst_albedo.a * tex.a;
    return vec4<f32>(rgb, alpha);
}
