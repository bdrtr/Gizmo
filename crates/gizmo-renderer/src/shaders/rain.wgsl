// Rain: falling streaks around the camera and the splashes where they land (`gizmo_renderer::rain`).
//
// Nothing is simulated. A drop is its instance index: a point of a box the size of `rain.box_size`, moved
// down by its velocity with the clock and wrapped into the box around the camera — so it stands still in
// the world but for its fall, and the box goes where the camera goes. It is drawn as a streak a few
// millimetres wide along where it went relative to the camera while a shutter was open, lit by the frame's lights (its cluster's) and scattered towards the
// camera, forward the most, as drops do: a street's rain shows in its lamps and headlights and hardly
// elsewhere. Where the rain map (`gizmo_renderer::rain_map`) has something above it, it is under cover
// and not drawn. A splash is a cell of a grid round the camera: a crown of water thrown up and falling
// back, again and again, standing on the surface the rain map says the rain lands on there and turned
// to the camera — a ring lying on the ground was edge-on to a camera a car's height up, under a pixel tall
// at 7 m.

#import gizmo::common::{SceneUniforms, LightData, light_falloff, rotate_by_quat, spot_cone, spot_cookie_uv, gizmo_cluster_index, height_fog}

@group(0) @binding(0) var<uniform> scene: SceneUniforms;
@group(0) @binding(1) var<storage, read> cluster_table: array<vec2<u32>>;
@group(0) @binding(2) var<storage, read> cluster_light_indices: array<u32>;

struct RainParams {
    // xyz: the box around the camera, metres; w: how many drops are in it.
    box_size: vec4<f32>,
    // xyz: the drops' velocity, m/s (down, and the wind); w: how long the shutter is open, seconds.
    fall: vec4<f32>,
    // x: a drop's width, m; y: how bright the rain is; z: splash cells a side; w: a splash's size, m.
    look: vec4<f32>,
    // x: the rain map's depth, m; y: a splash cell, m; z: a splash's life, s; w: what the air lends a drop.
    map: vec4<f32>,
    // xyz: the camera's velocity, m/s.
    camera_velocity: vec4<f32>,
}

@group(1) @binding(0) var t_depth: texture_depth_2d;
@group(1) @binding(1) var t_rain: texture_depth_2d;
@group(1) @binding(2) var<uniform> rain: RainParams;
@group(1) @binding(3) var t_cookie: texture_2d_array<f32>;
@group(1) @binding(4) var s_cookie: sampler;

struct Varyings {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    // A drop: (0, along the streak 0–1, -1, how much of the quad's width it covers). A splash: (across it,
    // −1 – 1, up it, 0 – 1, how far through its life 0 – 1, how much of its quad it covers).
    @location(1) shape: vec4<f32>,
}

fn hash(n: u32) -> u32 {
    var x = n * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return (x >> 22u) ^ x;
}

fn hash3(n: u32) -> vec3<f32> {
    let a = hash(n);
    let b = hash(a);
    let c = hash(b);
    return vec3<f32>(f32(a), f32(b), f32(c)) / 4294967295.0;
}

// The height the rain lands at, above `xz`, from the rain map — or nothing (`.y` 0) off the map, where
// nothing lies below, and when the frame has no map.
fn landing(xz: vec2<f32>) -> vec2<f32> {
    if (scene.rain_params.x < 0.5) {
        return vec2<f32>(0.0, 0.0);
    }
    let clip = scene.rain_view_proj * vec4<f32>(xz.x, scene.camera_pos.y, xz.y, 1.0);
    let uv = vec2<f32>(clip.x * 0.5 + 0.5, 0.5 - clip.y * 0.5);
    if (any(uv <= vec2<f32>(0.0)) || any(uv >= vec2<f32>(1.0))) {
        return vec2<f32>(0.0, 0.0);
    }
    let dims = vec2<f32>(textureDimensions(t_rain));
    let d = textureLoad(t_rain, vec2<i32>(uv * dims), 0);
    if (d >= 1.0) {
        return vec2<f32>(0.0, 0.0);
    }
    // The map looks down from half its depth above the camera, orthographic: its depth is height.
    return vec2<f32>(scene.camera_pos.y + 0.5 * rain.map.x - d * rain.map.x, 1.0);
}

// A point off-screen: what a vertex of a drop or splash that is not drawn goes to.
const NOWHERE = vec4<f32>(2.0, 2.0, 2.0, 1.0);

@vertex
fn vs_drop(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> Varyings {
    var out: Varyings;
    out.clip = NOWHERE;
    out.shape = vec4<f32>(0.0);
    out.world_pos = vec3<f32>(0.0);
    let box = rain.box_size.xyz;
    let r = hash3(ii);
    let cam = scene.camera_pos.xyz;
    // Still in the world but for its fall, wrapped into the box round the camera.
    let p0 = r * box + rain.fall.xyz * scene.cascade_params.z;
    let rel = p0 - (cam - 0.5 * box);
    let pos = cam - 0.5 * box + (rel - box * floor(rel / box));
    // Under cover: below what the rain lands on here.
    let land = landing(pos.xz);
    if (land.y > 0.5 && pos.y < land.x) {
        return out;
    }
    // The streak: from the drop back along where it went relative to the camera while the shutter was open
    // — its fall, less the camera's own motion.
    let tail = pos - (rain.fall.xyz - rain.camera_velocity.xyz) * rain.fall.w;
    let a = scene.view_proj * vec4<f32>(pos, 1.0);
    let b = scene.view_proj * vec4<f32>(tail, 1.0);
    let still = scene.view_proj * vec4<f32>(pos - rain.fall.xyz * rain.fall.w, 1.0);
    if (a.w < 0.05 || b.w < 0.05 || still.w < 0.05) {
        return out;
    }
    let size = vec2<f32>(textureDimensions(t_depth));
    let sa = a.xy / a.w * 0.5 * size;
    let sb = b.xy / b.w * 0.5 * size;
    // A drop's light over the shutter is spread along where it went: a streak the camera's motion draws
    // out is as much dimmer than the drop's fall alone as it is longer. And a drop within a couple of metres
    // of the lens is out of focus, its light spread wide and faint: drawn sharp, the nearest drops were
    // single lines across a third of the screen.
    let still_px = max(length(still.xy / still.w * 0.5 * size - sa), 1.0);
    let spread = min(still_px / max(length(sb - sa), 1.0), 1.0);
    let focus = smoothstep(0.5, 2.0, length(pos - cam));
    var along = sb - sa;
    if (length(along) < 1e-3) {
        along = vec2<f32>(0.0, -1.0);
    }
    let across = normalize(vec2<f32>(-along.y, along.x));
    // The drop's width in pixels at its distance, and the quad at least 1.5 px wide: what it covers of it
    // dims the streak instead of the streak flickering in and out between pixels.
    var side_dir = cross(normalize(pos - cam), normalize(rain.fall.xyz));
    if (length(side_dir) < 1e-3) {
        side_dir = cross(normalize(pos - cam), vec3<f32>(1.0, 0.0, 0.0));
    }
    let edge = scene.view_proj * vec4<f32>(pos + normalize(side_dir) * rain.look.x, 1.0);
    let width_px = length(edge.xy / edge.w * 0.5 * size - sa);
    let quad_px = max(width_px, 1.5);
    let corner = array<vec2<f32>, 6>(
        vec2<f32>(0.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
    )[vi];
    let end = select(a, b, corner.x > 0.5);
    let offset = across * corner.y * 0.5 * quad_px / (0.5 * size) * end.w;
    out.clip = vec4<f32>(end.xy + offset, end.z, end.w);
    out.world_pos = mix(pos, tail, corner.x);
    out.shape = vec4<f32>(0.0, corner.x, -1.0, min(width_px / quad_px, 1.0) * spread * focus);
    return out;
}

@vertex
fn vs_splash(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> Varyings {
    var out: Varyings;
    out.clip = NOWHERE;
    out.shape = vec4<f32>(0.0);
    out.world_pos = vec3<f32>(0.0);
    let side = u32(rain.look.z);
    let cell = rain.map.y;
    // The grid stands on the world's own cells, so a cell keeps its splashes as the camera moves.
    let first = floor(scene.camera_pos.xz / cell) - f32(side / 2u);
    let ij = vec2<f32>(f32(ii % side), f32(ii / side));
    let world_cell = first + ij;
    let key = hash(u32(i32(world_cell.x) + 65536) * 73856093u ^ u32(i32(world_cell.y) + 65536) * 19349663u);
    let jitter = hash3(key);
    let xz = (world_cell + jitter.xy) * cell;
    let land = landing(xz);
    if (land.y < 0.5) {
        return out;
    }
    // Over and over, at a time of its own.
    let life = max(rain.map.z, 0.01);
    let phase = fract(scene.cascade_params.z / life + jitter.z);
    let foot = vec3<f32>(xz.x, land.x, xz.y);
    // Standing up, turned to the camera about the vertical; at least two pixels tall, dimmer by as much as
    // it had to grow, so that a far splash does not flicker in and out between pixels.
    let cam = scene.camera_pos.xyz;
    var facing = vec3<f32>(cam.x - foot.x, 0.0, cam.z - foot.z);
    if (length(facing) < 1e-3) {
        facing = vec3<f32>(0.0, 0.0, 1.0);
    }
    let right = normalize(cross(vec3<f32>(0.0, 1.0, 0.0), facing));
    let size = rain.look.w;
    let base = scene.view_proj * vec4<f32>(foot, 1.0);
    let top = scene.view_proj * vec4<f32>(foot + vec3<f32>(0.0, size, 0.0), 1.0);
    if (base.w < 0.05 || top.w < 0.05) {
        return out;
    }
    let pixels = vec2<f32>(textureDimensions(t_depth));
    let tall_px = length((top.xy / top.w - base.xy / base.w) * 0.5 * pixels);
    let grow = max(2.0 / max(tall_px, 1e-3), 1.0);
    let corner = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    )[vi];
    let at = foot + (right * corner.x + vec3<f32>(0.0, corner.y, 0.0)) * size * grow;
    out.clip = scene.view_proj * vec4<f32>(at, 1.0);
    out.world_pos = at;
    out.shape = vec4<f32>(corner.x, corner.y, phase, 1.0 / (grow * grow));
    return out;
}

// A splash's crown at `(across, up)` of its quad, `t` through its life: water thrown up and out in an arch
// that widens as it rises and falls back, broken into drops along it, gone by the end.
fn crown(across: f32, up: f32, t: f32) -> f32 {
    let width = 0.3 + 0.7 * sqrt(t);
    let height = 4.0 * t * (1.0 - t);
    let x = across / width;
    if (abs(x) >= 1.0) {
        return 0.0;
    }
    let arch = height * (1.0 - x * x);
    let band = 1.0 - smoothstep(0.0, 0.15, abs(up - arch));
    let drops = 0.5 + 0.5 * cos(x * 3.5 * 3.14159);
    return band * drops * (1.0 - t) * (1.0 - t);
}

// Henyey-Greenstein, normalised so that light scattered evenly is 1: drops throw it forward.
fn drop_phase(cos_theta: f32) -> f32 {
    let g = 0.6;
    let g2 = g * g;
    return (1.0 - g2) / pow(max(1.0 + g2 - 2.0 * g * cos_theta, 1e-3), 1.5);
}

// The light the frame's lights put on a drop at `p`, as the camera sees it scattered.
fn drop_light(p: vec3<f32>) -> vec3<f32> {
    let to_camera = normalize(scene.camera_pos.xyz - p);
    let cluster = gizmo_cluster_index(scene.view_proj, scene.camera_pos.xyz, scene.camera_forward.xyz, scene.cluster_dims, scene.cluster_depth, p);
    let range = cluster_table[cluster];
    var total = vec3<f32>(0.0);
    for (var ci = 0u; ci < range.y; ci++) {
        let light = scene.lights[cluster_light_indices[range.x + ci]];
        let kind = u32(light.params.y);
        if (kind == 2u) {
            continue;
        }
        let from_light = p - light.position.xyz;
        // A rectangle light lights one side of itself.
        if (kind == 3u && dot(from_light, rotate_by_quat(light.direction, vec3<f32>(0.0, 0.0, -1.0))) <= 0.0) {
            continue;
        }
        let dist = length(from_light);
        var reach = light_falloff(dist, light.color.a).x;
        let travel = from_light / max(dist, 1e-6);
        var colour = light.color.rgb;
        if (kind == 1u) {
            // In a spot's cone and through its cookie: a headlight's low beam lights the rain below its
            // cut-off, not the rain above it.
            reach *= spot_cone(light, travel);
            if (reach > 0.0 && light.params.w > 0.0) {
                colour *= textureSampleLevel(t_cookie, s_cookie, spot_cookie_uv(light, p), i32(light.params.w) - 1, 0.0).rgb;
            }
        }
        if (reach > 0.0) {
            total += colour * light.position.w * reach * drop_phase(dot(travel, to_camera));
        }
    }
    return total;
}

@fragment
fn fs_rain(in: Varyings) -> @location(0) vec4<f32> {
    // Behind what the frame drew here: hidden. The depth is compared as distance from the camera.
    let scene_z = textureLoad(t_depth, vec2<i32>(in.clip.xy), 0);
    let dims = vec2<f32>(textureDimensions(t_depth));
    let uv = in.clip.xy / dims;
    let h = scene.inv_view_proj * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, scene_z, 1.0);
    let surface = length(h.xyz / h.w - scene.camera_pos.xyz);
    let here = length(in.world_pos - scene.camera_pos.xyz);
    let is_splash = in.shape.z >= 0.0;
    // A splash stands on the surface: its foot is level with what is behind it.
    if (here > surface + select(0.0, 0.05, is_splash)) {
        discard;
    }
    var amount = in.shape.w;
    if (is_splash) {
        amount *= crown(in.shape.x, in.shape.y, in.shape.z);
    } else {
        // A streak fades in at its head and out at its tail.
        amount *= sin(3.14159 * in.shape.y);
    }
    if (amount <= 0.0) {
        discard;
    }
    let lit = drop_light(in.world_pos) + vec3<f32>(rain.map.w);
    let fog = height_fog(in.world_pos, scene.camera_pos.xyz, scene.fog, scene.fog_shape);
    return vec4<f32>(lit * rain.look.y * amount * (1.0 - fog.a), 0.0);
}
