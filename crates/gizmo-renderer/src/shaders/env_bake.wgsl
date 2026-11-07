// Draws the environment into one face of one mip of `env_cube::EnvironmentCube`: the presets, or
// `Renderer::sky`, along each texel's direction at the mip's roughness. The instance index says which
// target: face = index % 6, mip = index / 6.
#import gizmo::common::{scene_environment}

struct EnvInputs {
    zenith: vec4<f32>,
    horizon: vec4<f32>,
    ground: vec4<f32>,
    sun_direction: vec4<f32>,
    // preset, preset blended towards, the sharpest mip's edge in texels, how many mips
    params: vec4<u32>,
    // x = how far the blend is
    blend: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> env: EnvInputs;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) target_index: u32,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32, @builtin(instance_index) target_index: u32) -> VertexOutput {
    // One triangle over the whole target.
    let xy = vec2<f32>(f32((vertex << 1u) & 2u), f32(vertex & 2u));
    var out: VertexOutput;
    out.position = vec4<f32>(xy * 2.0 - 1.0, 0.0, 1.0);
    out.target_index = target_index;
    return out;
}

// A texel's direction, as `environment::cube_direction` gives it and as WebGPU samples a cube: `s`
// rightwards and `t` downwards across the face, each from -1 to 1.
fn cube_direction(face: u32, s: f32, t: f32) -> vec3<f32> {
    switch face {
        case 0u: { return vec3<f32>(1.0, -t, -s); }
        case 1u: { return vec3<f32>(-1.0, -t, s); }
        case 2u: { return vec3<f32>(s, 1.0, t); }
        case 3u: { return vec3<f32>(s, -1.0, -t); }
        case 4u: { return vec3<f32>(s, -t, 1.0); }
        default: { return vec3<f32>(-s, -t, -1.0); }
    }
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let face = in.target_index % 6u;
    let level = in.target_index / 6u;
    let size = f32(env.params.z >> level);
    // The fragment's position is its texel's centre, from the top left.
    let s = in.position.x / size * 2.0 - 1.0;
    let t = in.position.y / size * 2.0 - 1.0;
    let dir = normalize(cube_direction(face, s, t));
    let roughness = f32(level) / f32(max(env.params.w, 2u) - 1u);
    let colour = scene_environment(
        dir, roughness, env.zenith, env.horizon.rgb, env.ground.rgb, env.params.x, env.params.y, env.blend.x,
        normalize(-env.sun_direction.xyz),
    );
    return vec4<f32>(colour, 1.0);
}
