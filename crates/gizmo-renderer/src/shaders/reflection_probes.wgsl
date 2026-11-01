#define_import_path gizmo::reflection_probes

// Reflection probes, shared by the deferred lighting pass and the forward (transparent) pass. The
// bind group is the irradiance volume's (bindings 0–1 are its grid; 2–4 are these), bound at
// `REFLECTION_GROUP`: 3 in the deferred pass, 5 in the forward one.
// Up to four captures of
// the scene in one prefiltered cube array, slot i = probe i. Header = [count, max_mip, background
// slot or −1, 0]; per slot: origin = [position, has box], box_min = [min, blend distance],
// box_max = [max, intensity]. With no probes `probe_environment` returns the sky it is given,
// unchanged — frames are bit-identical to before probes existed.
struct ProbeSlot {
    origin: vec4<f32>,
    box_min: vec4<f32>,
    box_max: vec4<f32>,
};
struct Reflections {
    header: vec4<f32>,
    probes: array<ProbeSlot, 4>,
};
@group(#{REFLECTION_GROUP}) @binding(2) var t_reflection: texture_cube_array<f32>;
@group(#{REFLECTION_GROUP}) @binding(3) var s_reflection: sampler;
@group(#{REFLECTION_GROUP}) @binding(4) var<uniform> reflections: Reflections;

// Box-projected parallax for slot `i`: trace the reflected ray from the surface to the box's far
// wall, and look the cube up toward where it lands as seen from the probe.
fn box_projected(i: u32, r: vec3<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    let p = reflections.probes[i];
    let safe_r = select(r, vec3<f32>(1e-6), abs(r) < vec3<f32>(1e-6));
    let to_hi = (p.box_max.xyz - world_pos) / safe_r;
    let to_lo = (p.box_min.xyz - world_pos) / safe_r;
    let far = max(to_hi, to_lo);
    let dist = min(min(far.x, far.y), far.z);
    return world_pos + r * dist - p.origin.xyz;
}

// The reflected environment along `r` at `world_pos`, for a surface of `roughness`.
//
// Boxed probes are LOCAL: each weighs in only inside its box, fading in over its blend distance
// from the walls, through box-projected parallax. Their weights are normalised when they overlap
// past 1 (a doorway between two rooms is a blend of both). Whatever weight the boxes leave over
// is filled by the BACKGROUND — the first probe without a box, looked up by direction, or the
// procedural `sky` if there is none.
fn probe_environment(r: vec3<f32>, world_pos: vec3<f32>, roughness: f32, sky: vec3<f32>) -> vec3<f32> {
    let count = u32(reflections.header.x);
    if (count == 0u) {
        return sky;
    }
    let lod = roughness * reflections.header.y;
    var local = vec3<f32>(0.0);
    var weight = 0.0;
    for (var i = 0u; i < count; i = i + 1u) {
        let p = reflections.probes[i];
        if (p.origin.w > 0.5) {
            let d = min(world_pos - p.box_min.xyz, p.box_max.xyz - world_pos);
            let inside = min(min(d.x, d.y), d.z);
            if (inside > 0.0) {
                let w = clamp(inside / p.box_min.w, 0.0, 1.0);
                let dir = box_projected(i, r, world_pos);
                local += textureSampleLevel(t_reflection, s_reflection, dir, i, lod).rgb * p.box_max.w * w;
                weight += w;
            }
        }
    }
    if (weight > 1.0) {
        local = local / weight;
        weight = 1.0;
    }
    var background = sky;
    let bg = i32(reflections.header.z);
    if (bg >= 0) {
        background = textureSampleLevel(t_reflection, s_reflection, r, bg, lod).rgb
            * reflections.probes[bg].box_max.w;
    }
    return local + background * (1.0 - weight);
}
