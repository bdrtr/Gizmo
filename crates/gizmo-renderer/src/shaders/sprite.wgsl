// One quad per sprite, from an instance buffer. No vertex buffer: the CPU has already put the four
// corners in clip space (with the UNJITTERED view-projection, so a sprite drawn after TAA does not
// shimmer by the jitter), and this shader only picks one.
//
// The corners are in image order — top-left, top-right, bottom-left, bottom-right — so the corner
// index alone says where in the image a vertex samples; flipping is already folded into `uv`.
//
// A LIT world sprite (`Sprite::lit`) also carries its plane in world space — a corner and the two
// edges from it — and `fs_lit`, the world pipeline's entry, lights it with the scene's own lights:
// the view's `SceneUniforms` (the same ranked array, sun and camera the 3D passes read), the same
// falloff and cone (`gizmo::common`), the Lambert 1/PI a matte surface gets, over an ambient that is
// white unless the game darkens it (`SpriteAmbient`). An unlit sprite, and every screen sprite, is
// drawn in its own colours, by either entry. Group 1 is read by `fs_lit` alone, so the overlay's
// pipelines, which draw screen sprites with `fs_main`, do not carry it.

#import gizmo::common::{SceneUniforms, light_falloff, spot_cone}

@group(1) @binding(0)
var<uniform> scene: SceneUniforms;
@group(1) @binding(1)
var<uniform> sprite_ambient: vec4<f32>;

@group(0) @binding(0)
var image: texture_2d<f32>;
@group(0) @binding(1)
var image_sampler: sampler;

struct SpriteInstance {
    @location(0) top_left: vec4<f32>,
    @location(1) top_right: vec4<f32>,
    @location(2) bottom_left: vec4<f32>,
    @location(3) bottom_right: vec4<f32>,
    // (u0, v0, u1, v1): u0/v0 at the top-left corner, u1/v1 at the bottom-right.
    @location(4) uv: vec4<f32>,
    @location(5) color: vec4<f32>,
    // The top-left corner in world space, `w` = 1 for a lit sprite; the edges to the top-right and
    // bottom-left corners. Zero for a screen sprite.
    @location(6) world_origin: vec4<f32>,
    @location(7) world_u: vec4<f32>,
    @location(8) world_v: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) world: vec3<f32>,
    @location(3) normal: vec3<f32>,
    @location(4) lit: f32,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, inst: SpriteInstance) -> VertexOutput {
    // Two triangles over corners 0 1 2 / 2 1 3. Culling is off, so the winding is documentation;
    // a mirrored sprite (negative scale) is still a sprite.
    var order = array<u32, 6>(0u, 1u, 2u, 2u, 1u, 3u);
    let k = order[vi];
    var corners = array<vec4<f32>, 4>(inst.top_left, inst.top_right, inst.bottom_left, inst.bottom_right);
    let c = vec2<f32>(f32(k & 1u), f32(k >> 1u));

    var out: VertexOutput;
    out.clip = corners[k];
    out.uv = mix(inst.uv.xy, inst.uv.zw, c);
    out.color = inst.color;
    out.world = inst.world_origin.xyz + inst.world_u.xyz * c.x + inst.world_v.xyz * c.y;
    out.normal = cross(inst.world_u.xyz, inst.world_v.xyz);
    out.lit = inst.world_origin.w;
    return out;
}

const PI: f32 = 3.14159265;

// The world pipeline's entry: an unlit sprite as `fs_main` draws it, a lit one lit — see the top.
@fragment
fn fs_lit(in: VertexOutput) -> @location(0) vec4<f32> {
    let base = textureSample(image, image_sampler, in.uv) * in.color;
    if (in.lit < 0.5) {
        return base;
    }
    // The face toward the camera: a sprite is seen from either side.
    var n = normalize(in.normal);
    if (dot(n, scene.camera_pos.xyz - in.world) < 0.0) {
        n = -n;
    }
    var light = sprite_ambient.rgb;
    if (scene.sun_direction.w > 0.5) {
        let l = normalize(-scene.sun_direction.xyz);
        light += scene.sun_color.rgb * scene.sun_color.w * max(dot(n, l), 0.0) / PI;
    }
    let count = min(scene.num_lights, 256u);
    for (var i = 0u; i < count; i++) {
        let ld = scene.lights[i];
        let kind = u32(ld.params.y + 0.5);
        var l = vec3<f32>(0.0);
        var atten = 1.0;
        if (kind == 2u) {
            l = normalize(-ld.direction.xyz);
        } else if (kind <= 1u) {
            let to_light = ld.position.xyz - in.world;
            let d = length(to_light);
            l = to_light / max(d, 1.0e-4);
            atten = light_falloff(d, ld.color.a).x;
            if (kind == 1u) {
                atten *= spot_cone(ld, -l);
            }
        } else {
            // A rectangle light's area integral is not here yet; it lights 3D surfaces only.
            continue;
        }
        light += ld.color.rgb * ld.position.w * atten * max(dot(n, l), 0.0) / PI;
    }
    return vec4<f32>(base.rgb * light, base.a);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(image, image_sampler, in.uv) * in.color;
}

// The overlay's entry for a target that is NOT sRGB: the post chain's output there is already
// encoded, so a linear colour written raw would come out too dark. An sRGB target encodes in
// hardware and uses the plain entry.
fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

@fragment
fn fs_main_encoded(in: VertexOutput) -> @location(0) vec4<f32> {
    let c = textureSample(image, image_sampler, in.uv) * in.color;
    return vec4<f32>(linear_to_srgb(c.rgb), c.a);
}
