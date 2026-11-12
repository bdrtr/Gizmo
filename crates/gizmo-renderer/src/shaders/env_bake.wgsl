// Draws the environment into `env_cube::EnvironmentCube`: the presets, or `Renderer::sky`, along each
// texel's direction at each mip's roughness — every face of every mip in one pass, as tiles of one atlas,
// which the cube then takes tile by tile (a copy each). Instance `i` is face `i % 6` of mip `i / 6`.
//
// It drew each face of each mip in a pass of its own until 2026-11-12: 24 passes, and on an Adreno 618 a
// sky that changed every frame cost 9.5 ms a frame, most of it the passes rather than the texels.
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

// Where target `index` lies in the atlas, in texels: its corner (x, y) and its edge (z). A row per mip,
// sharpest at the top, the six faces along it; mip `m` starts `2 × (size − size >> m)` down, the sum of
// the rows above it.
fn tile(index: u32) -> vec3<u32> {
    let edge = env.params.z >> (index / 6u);
    return vec3<u32>((index % 6u) * edge, 2u * (env.params.z - edge), edge);
}

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32, @builtin(instance_index) target_index: u32) -> VertexOutput {
    // Two triangles over this target's tile.
    let corner = vec2<f32>(
        select(0.0, 1.0, vertex == 1u || vertex == 4u || vertex == 5u),
        select(0.0, 1.0, vertex == 2u || vertex == 3u || vertex == 5u),
    );
    let t = tile(target_index);
    let atlas = vec2<f32>(f32(6u * env.params.z), f32(2u * (env.params.z - (env.params.z >> env.params.w))));
    let texel = vec2<f32>(t.xy) + corner * f32(t.z);
    var out: VertexOutput;
    out.position = vec4<f32>(texel.x / atlas.x * 2.0 - 1.0, 1.0 - texel.y / atlas.y * 2.0, 0.0, 1.0);
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
    let tile_at = tile(in.target_index);
    let size = f32(tile_at.z);
    // The fragment's position is its texel's centre, from the atlas's top left; from its tile's.
    let local = in.position.xy - vec2<f32>(tile_at.xy);
    let s = local.x / size * 2.0 - 1.0;
    let t = local.y / size * 2.0 - 1.0;
    let dir = normalize(cube_direction(face, s, t));
    let roughness = f32(level) / f32(max(env.params.w, 2u) - 1u);
    let colour = scene_environment(
        dir, roughness, env.zenith, env.horizon.rgb, env.ground.rgb, env.params.x, env.params.y, env.blend.x,
        env.sun_direction,
    );
    return vec4<f32>(colour, 1.0);
}
