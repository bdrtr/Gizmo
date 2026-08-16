// Deferred Decal Shader
// SceneUniforms shared from gizmo::common (composed by load_shader_composed).
#import gizmo::common::{SceneUniforms}

@group(0) @binding(0) var<uniform> scene: SceneUniforms;

@group(1) @binding(0) var t_position_rel_camera: texture_2d<f32>;

struct DecalUniforms {
    inv_model: mat4x4<f32>,
    model: mat4x4<f32>,
    albedo_color: vec4<f32>,
}
@group(2) @binding(0) var t_albedo: texture_2d<f32>;
@group(2) @binding(1) var s_albedo: sampler;

@group(3) @binding(0) var<uniform> decal: DecalUniforms;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) screen_pos: vec4<f32>,
};

@vertex
fn vs_main(@location(0) pos: vec3<f32>) -> VertexOutput {
    var out: VertexOutput;
    let world_pos = decal.model * vec4<f32>(pos, 1.0);
    out.position = scene.view_proj * world_pos;
    out.screen_pos = out.position;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let tex_coord = vec2<i32>(in.position.xy);
    let world_pos_val = textureLoad(t_position_rel_camera, tex_coord, 0);
    
    // Sky / unwritten G-buffer pixel → nothing to project onto. `< 0.5`, like the other readers
    // of this flag: gbuffer.wgsl writes `(0.5 + 0.49·anisotropy) + floor(100·subsurface)`, so
    // written pixels start at 0.5 and the clear is 0.0. The old form here was an exact `== 0.0`,
    // which is the most fragile of the three spellings this flag had — it happens to work only
    // because the clear value is exactly representable, and it would pass anything in (0, 0.5)
    // straight through. The strict `> 0.5` spelling of the same test is what left SSR and SSGI
    // contributing nothing at all; see their hit tests.
    if (world_pos_val.w < 0.5) {
        discard;
    }
    
    let world_pos = world_pos_val.xyz;
    let local_pos = decal.inv_model * vec4<f32>(world_pos, 1.0);
    
    // Test intersection with the unit cube bounds [-0.5, 0.5]
    if (abs(local_pos.x) > 0.5 || abs(local_pos.y) > 0.5 || abs(local_pos.z) > 0.5) {
        discard;
    }
    
    // Project UV onto the XZ plane (projecting downwards along Y)
    let decal_uv = vec2<f32>(local_pos.x + 0.5, local_pos.z + 0.5);
    
    // Sample texture
    var color = textureSample(t_albedo, s_albedo, decal_uv) * decal.albedo_color;
    
    // Create a circular fade effect (so it looks like a spray/splatter rather than a sharp box)
    let dist = distance(decal_uv, vec2<f32>(0.5, 0.5));
    let alpha_mask = 1.0 - smoothstep(0.3, 0.5, dist);
    
    color.a *= alpha_mask;
    
    if (color.a < 0.01) {
        discard;
    }
    
    return color;
}
