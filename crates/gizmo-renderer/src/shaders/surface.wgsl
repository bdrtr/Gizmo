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
    // For a surface that lets light through (`Material::transmission`): how much of it the medium
    // behind the surface takes out per metre, a channel at a time — water, murky glass. Zero (the
    // default) is a thin pane: what is behind comes through tinted by `albedo`, as glass always
    // has. Non-zero makes the surface the top of a volume that reaches the opaque world behind it:
    // what shows through is dimmed by `exp(-absorption · depth)` over the depth of that volume
    // (`SurfaceInput::behind_distance`), and `albedo` is the colour the volume scatters where it is
    // too deep to see through — lit, so dark at night. Read only where there is a frame behind.
    absorption: vec3<f32>,
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
    // Where the material's base colour is: 0 its own texture, otherwise one more than its layer in
    // the texture pool's array (`InstanceRaw::texture.x`). `gizmo::material::standard_surface` reads it.
    base_layer: u32,
    // `@builtin(position)`: the fragment's pixel centre and depth.
    frag_coord: vec4<f32>,
    // Seconds since the renderer started: the clock `SceneUniforms` carries.
    time: f32,
    // Where the camera is.
    camera_position: vec3<f32>,
    // How far behind this fragment, along the view ray, the opaque world is, in metres — the depth
    // of water under a lake's surface. Known only to a surface drawn with the frame behind it (one
    // whose material lets light through, `Material::transmission`); `-1.0` everywhere else, and a
    // very large number where there is nothing behind but sky.
    behind_distance: f32,
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

// ── Rain (`Renderer::wetness`) ─────────────────────────────────────────────────────────────────────
// What water does to a surface it lies on: darkens what it soaks into and smooths it, and where it
// stands — a puddle — is a flat mirror over it, rippled by the rain while it falls. The scene's `wetness`
// (`SceneUniforms::wetness`: x = level, y = how much of the flat ground is under water, z = 1 / a
// puddle's size in metres, w = the rings falling rain leaves on water, per square metre per second) is
// handed in by the shader that reads it, as this module declares no bindings.

// What a soaked surface's colour is multiplied by: water fills its pores and it reflects less light
// back out (Lagarde, "Water drop 2b — Dynamic rain and its effects", 2013).
const WET_DARKENING: f32 = 0.6;
// The roughness a soaked surface goes to, if it was rougher: a film of water over it.
const WET_ROUGHNESS: f32 = 0.2;
// A puddle's: still water.
const PUDDLE_ROUGHNESS: f32 = 0.03;
// Water's reflectance at normal incidence: what the water on a surface reflects the sun and the lights
// with (`water_glints` in `baked_lit.wgsl`, the baked branch of `deferred_lighting.wgsl`).
const WATER_F0: f32 = 0.02;

// How wet a fragment is: x = soaked (0 dry … 1), y = under a puddle (0 … 1), z = how much of it soaks
// in (`Material::porosity`). Only what faces up takes rain: `up` is the geometric normal's height, so a
// wall stays near dry and a normal map does not speckle a road; puddles stand only on the flat, and only
// on what is porous — a sealed surface (paint, glass, metal) sheds the rain, keeping the film and nothing
// more. `exposure`: how much of the rain reaches it, 0 under cover (`gizmo::common::rain_exposure`, on a
// path with a rain map; 1 where there is none).
fn rain_on(wetness: vec4<f32>, world_position: vec3<f32>, up: f32, exposure: f32, porosity: f32) -> vec3<f32> {
    let level = clamp(wetness.x, 0.0, 1.0) * clamp(exposure, 0.0, 1.0);
    let pores = clamp(porosity, 0.0, 1.0);
    if (level <= 0.0) {
        return vec3<f32>(0.0, 0.0, pores);
    }
    let soaked = level * smoothstep(0.2, 0.8, up);
    var puddle = 0.0;
    let coverage = clamp(wetness.y, 0.0, 1.0);
    if (coverage > 0.0 && pores > 0.0) {
        // The noise's value at which `coverage` of the ground lies above it (fitted to the noise's
        // distribution: within 0.005, about 1 % of coverage).
        let x = 1.0 - 2.0 * coverage;
        let threshold = 0.5 + 0.2025 * x + 0.1026 * x * x * x;
        let n = puddle_noise(world_position.xz * wetness.z);
        // The shore is a band either side of the threshold, so that half-way into the water is where
        // the share asked for ends.
        puddle = smoothstep(threshold - 0.02, threshold + 0.02, n) * smoothstep(0.9, 0.98, up) * level * pores;
    }
    return vec3<f32>(soaked, puddle, pores);
}

// The surface as rain leaves it (`rain` from `rain_on`, `ripples` from `rain_ripples`).
fn wet_surface(s: Surface, rain: vec3<f32>, ripples: vec3<f32>) -> Surface {
    var out = s;
    let water = max(rain.x, rain.y);
    if (water <= 0.0) {
        return out;
    }
    let darkening = wet_darkening(rain);
    out.albedo = s.albedo * darkening;
    out.base_color = s.base_color * darkening;
    out.roughness = wet_roughness(s.roughness, rain, ripples);
    out.N = water_normal(s.N, rain, ripples);
    return out;
}

// The normal of the water over a surface of normal `n`: a puddle's flat, the rain's rings on it and on a
// soaked surface's film.
fn water_normal(n: vec3<f32>, rain: vec3<f32>, ripples: vec3<f32>) -> vec3<f32> {
    return normalize(mix(n, vec3<f32>(0.0, 1.0, 0.0), rain.y) + vec3<f32>(-ripples.x, 0.0, -ripples.y));
}

// ── The rings falling rain leaves on water ─────────────────────────────────────────────────────────
// A drop landing in standing water throws out a ring of small waves that spreads and dies. Nothing is
// simulated: the ground is a grid of cells, a few grids laid over each other, and each cell takes a drop
// at a time of its own once a period — so that `wetness.w` rings land per square metre per second — at a
// place of its own near its middle. A ring is a packet of waves a wavelength or two wide behind a front
// running out at the waves' speed, dying away linearly over its life (a 2-D wave's height falls with the
// root of its radius, and the water's viscosity takes the rest); it stays inside its cell, so a fragment
// looks at one cell a grid. What the fragment gets is the water's slope there, which tilts the water's normal.
//
// Where a pixel spans more than a wavelength the rings cannot be drawn: their slopes would alias into
// sparkling noise. They are not dropped, either — water that rain is falling on does not reflect like a
// mirror however far off. Their slope's variance over the pixel goes into the roughness instead (Toksvig,
// "Mipmapping Normal Maps", 2005), and a far puddle's reflection blurs as a near one's shimmers.
const RIPPLE_WAVELENGTH: f32 = 0.03;
// How fast a ring runs out, m/s, and how long it lasts, s: 22 cm across at the end.
const RIPPLE_SPEED: f32 = 0.25;
const RIPPLE_LIFE: f32 = 0.45;
// A cell's side, m, and how far from its middle its drop lands, of a cell: a ring never leaves its cell.
const RIPPLE_CELL: f32 = 0.3;
const RIPPLE_JITTER: f32 = 0.1;
// Grids laid over each other. A cell holds one ring at a time, so they hold at most
// `RIPPLE_LAYERS / (RIPPLE_CELL² · RIPPLE_LIFE)` rings a square metre a second, 99; past that the rain
// saturates them rather than cutting a ring off before it dies (a ring vanishing mid-life blinks).
const RIPPLE_LAYERS: u32 = 4u;
// The steepest a fresh ring's waves stand: their slope, height over run.
const RIPPLE_SLOPE: f32 = 0.25;
// How much a soaked porous surface's film ripples, beside a puddle's 1: a millimetre of water carries less.
const FILM_RIPPLES: f32 = 0.3;

// How wide a pixel is on the ground at a fragment whose position runs `dx`, `dy` across neighbouring
// pixels (`dpdx`, `dpdy` of it): its longer side, where a slope seen at a glance foreshortens it.
fn ripple_footprint(dx: vec3<f32>, dy: vec3<f32>) -> f32 {
    return max(length(dx.xz), length(dy.xz));
}

// The rain's rings at `world_position` with the clock at `clock` (`SceneUniforms::cascade_params.z`), for
// a pixel `footprint` metres wide (`ripple_footprint`), on the water `rain` says lies there (`rain_on`'s):
// x, y = the water's slope along x and z, as the pixel shows it; z = the slope's variance it cannot show.
fn rain_ripples(wetness: vec4<f32>, world_position: vec3<f32>, clock: f32, footprint: f32, rain: vec3<f32>) -> vec3<f32> {
    let rate = wetness.w;
    // A puddle's water ripples, and a porous surface's film; a sealed one stands in beads (`rain_beads`).
    let weight = rain.y + FILM_RIPPLES * rain.x * rain.z * (1.0 - rain.y);
    if (rate <= 0.0 || weight <= 0.0) {
        return vec3<f32>(0.0);
    }
    let resolved = 1.0 - smoothstep(0.25 * RIPPLE_WAVELENGTH, RIPPLE_WAVELENGTH, footprint);
    var slope = vec2<f32>(0.0);
    if (resolved > 0.0) {
        // Each grid's cells take one drop a period: the layers together, `rate` a square metre a second — no
        // sooner than the last ring has died.
        let period = max(f32(RIPPLE_LAYERS) / (RIPPLE_CELL * RIPPLE_CELL * rate), RIPPLE_LIFE);
        let k = 6.2831853 / RIPPLE_WAVELENGTH;
        for (var layer = 0u; layer < RIPPLE_LAYERS; layer++) {
            // Each grid shifted by a golden fraction of a cell, so that no two share their edges.
            let p = world_position.xz / RIPPLE_CELL + vec2<f32>(0.381966, 0.618034) * f32(layer);
            let cell = floor(p);
            let h = ripple_hash(vec3<i32>(vec2<i32>(cell), i32(layer)));
            let age = fract(clock / period + h.z) * period;
            if (age >= RIPPLE_LIFE) {
                continue;
            }
            let offset = (p - (cell + 0.5 + (h.xy - 0.5) * 2.0 * RIPPLE_JITTER)) * RIPPLE_CELL;
            let d = length(offset);
            // Behind the front, negative: the packet a wavelength behind it, 0.6 of one wide.
            let x = d - RIPPLE_SPEED * age;
            let packet = (x + RIPPLE_WAVELENGTH) / (0.6 * RIPPLE_WAVELENGTH);
            let fade = 1.0 - age / RIPPLE_LIFE;
            let s = RIPPLE_SLOPE * fade * exp(-packet * packet) * cos(k * x);
            slope += s * offset / max(d, 1e-4);
        }
    }
    // The slope's mean square over the ground, which the rings' shape gives in closed form, at the rings a
    // square metre a second the grids hold: per ring of age t, half the peak's square, faded by (1 − t/L)²,
    // over a band 0.6·λ·√(π/2) wide and 2π·(v·t − λ) long — the packet a wavelength behind the front, so
    // nothing until v·t = λ. With w₀ = 1 − λ/(v·L), ∫ (1 − t/L)² (v·t − λ) dt from λ/v to L is
    // v·L²·(w₀³/3 − w₀⁴/4) − λ·L·w₀³/3. Within 7 % of the field's own, sampled (0.000346 at 60 a second).
    let held = min(rate, f32(RIPPLE_LAYERS) / (RIPPLE_CELL * RIPPLE_CELL * RIPPLE_LIFE));
    let w0 = 1.0 - RIPPLE_WAVELENGTH / (RIPPLE_SPEED * RIPPLE_LIFE);
    let band = RIPPLE_SPEED * RIPPLE_LIFE * RIPPLE_LIFE * (w0 * w0 * w0 / 3.0 - w0 * w0 * w0 * w0 / 4.0)
        - RIPPLE_WAVELENGTH * RIPPLE_LIFE * w0 * w0 * w0 / 3.0;
    let variance = held * 0.5 * RIPPLE_SLOPE * RIPPLE_SLOPE * 6.2831853 * 0.6 * RIPPLE_WAVELENGTH * 1.2533141 * band;
    return vec3<f32>(slope * resolved * weight, variance * weight * weight * (1.0 - resolved));
}

// Three uniform numbers for a cell of a grid (PCG).
fn ripple_hash(i: vec3<i32>) -> vec3<f32> {
    var v = bitcast<vec3<u32>>(i) * 1664525u + vec3<u32>(1013904223u);
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v = v ^ (v >> vec3<u32>(16u));
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    return vec3<f32>(v) * (1.0 / 4294967296.0);
}

// What rain multiplies a surface's colour by: its pores filled, and whatever lies under a puddle.
fn wet_darkening(rain: vec3<f32>) -> f32 {
    return mix(1.0, WET_DARKENING, max(rain.x * rain.z, rain.y));
}

// ── Rain beading on a sealed surface ───────────────────────────────────────────────────────────────
// On paint, glass and metal (`Material::porosity` 0) the rain does not soak in: it stands in beads. They
// are laid out in the surface's OWN space — the model's coordinates (`surface_local`), so that they stay
// where they landed as a car drives — as a few grids of 12 mm cells over each other, each cell holding a
// drop with the likelihood the wetness gives (`bead_density`), at a place and of a size of its own, kept
// inside its cell so a fragment looks at one cell a grid. Where a surface slices a drop's sphere it shows
// a hemisphere of the slice's radius: a bead with the steep sides water has on wax. Its slope is that
// hemisphere's, in closed form; `bead_normal` tilts the surface's normal by it through the positions'
// screen derivatives alone (Mikkelsen, "Bump Mapping Unparametrized Surfaces on the GPU", 2010), so no
// tangent frame is needed and the bead itself is never differentiated. Inside a bead the surface is
// water: as smooth as still water where the pixel is small beside the bead, and as rough as the bead's
// curvature across the pixel makes it where it is not (`Beads::variance`).
//
// On a PBR surface only: a baked-lit one is the still, lit-ahead world — a city — which takes the rain as
// a film, puddles and rings. Carried on the baked path too, the bead code cost every one of a city's
// pixels 0.68 ms of forward GPU time a frame where it beaded none of them (`nfsu2-gizmo`, AMD 780M, 1080p,
// 2026-11-20): code a fragment never runs still holds the registers it would, for every fragment.
//
// A bead is a few millimetres across, so from a chase camera it is a pixel or less. It is drawn down to
// that — fixed on its surface, it is a still detail that TAA's jitter resolves over frames rather than
// noise — and faded past it (`BEAD_FADE`). Not folded into the roughness the way the rain's rings are: a
// beaded surface reflects as a sharp lobe between the beads and a broad one off them, which one widened
// lobe would turn into a matt car.
const BEAD_CELL: f32 = 0.012;
// The radius of a bead's sphere, metres: from the first to the second, more of them small.
const BEAD_RADIUS: vec2<f32> = vec2<f32>(0.0012, 0.0042);
const BEAD_LAYERS: u32 = 3u;
// The steepest a bead's side is drawn, height over run: its rim, which on a hemisphere is upright.
const BEAD_SLOPE_MAX: f32 = 2.5;
const BEAD_ROUGHNESS: f32 = 0.04;
// How wide a pixel is when the beads it covers start to fade, and when they are gone, metres.
const BEAD_FADE: vec2<f32> = vec2<f32>(0.004, 0.012);

struct Beads {
    // The beads' height field's gradient, in the surface's own space: what `bead_normal` tilts by.
    slope: vec3<f32>,
    // How much of the pixel is under a bead, 0 … 1.
    cover: f32,
    // How much a bead's normal turns across the pixel, as a slope variance: `(footprint / R)² / 4` for a
    // sphere of radius R. Water is as smooth as glass, but a bead a pixel or two wide shows the pixel every
    // way its normal faces at once, and drawn as smooth it caught a light in one pixel as a square of glare.
    // Folded into the bead's roughness (`bead_roughness`), as geometric specular anti-aliasing does with a
    // normal's own variance (Kaplanyan et al., "Filtering Distributions of Normals", 2016), here known in
    // closed form.
    variance: f32,
}

// How thickly a surface stands in beads: the scene's wetness (`wetness.x`), on what sheds the rain
// (`1 − porosity`), the most on top, some on the sides, none underneath. `up` is the geometric normal's
// height. Not kept off what is under cover: a car that drives under a bridge keeps the rain on it.
fn bead_density(wetness: vec4<f32>, up: f32, porosity: f32) -> f32 {
    let facing = smoothstep(-0.3, 0.1, up) * (0.35 + 0.65 * smoothstep(0.2, 0.8, up));
    return clamp(wetness.x, 0.0, 1.0) * (1.0 - clamp(porosity, 0.0, 1.0)) * facing;
}

// The beads at `local` (the surface's own coordinates, metres) on a surface beaded `density`
// (`bead_density`), seen through a pixel `footprint` metres wide.
fn rain_beads(local: vec3<f32>, density: f32, footprint: f32) -> Beads {
    var out: Beads;
    out.slope = vec3<f32>(0.0);
    out.cover = 0.0;
    out.variance = 0.0;
    let resolved = 1.0 - smoothstep(BEAD_FADE.x, BEAD_FADE.y, footprint);
    if (density <= 0.0 || resolved <= 0.0) {
        return out;
    }
    for (var layer = 0u; layer < BEAD_LAYERS; layer++) {
        // Each grid shifted by a golden fraction of a cell, so that no two share their edges.
        let p = local / BEAD_CELL + vec3<f32>(0.381966, 0.618034, 0.236068) * f32(layer);
        let cell = floor(p);
        let h = bead_hash(vec4<i32>(vec3<i32>(cell), i32(layer)));
        if (h.w >= density) {
            continue;
        }
        let size = bead_hash(vec4<i32>(vec3<i32>(cell), i32(layer) + 64)).x;
        let radius = mix(BEAD_RADIUS.x, BEAD_RADIUS.y, size * size);
        let r = radius / BEAD_CELL;
        let offset = (p - (cell + r + h.xyz * (1.0 - 2.0 * r))) * BEAD_CELL;
        let d2 = dot(offset, offset);
        if (d2 >= radius * radius) {
            continue;
        }
        // The hemisphere's height is `sqrt(R² − |v|²)`; its gradient `−v / height`, held to the steepest
        // side drawn.
        let height = sqrt(radius * radius - d2);
        out.slope += -offset / max(height, max(sqrt(d2) / BEAD_SLOPE_MAX, 1e-7));
        out.cover = 1.0;
        out.variance = max(out.variance, min(0.25 * (footprint / radius) * (footprint / radius), 1.0));
    }
    out.slope *= resolved;
    out.cover *= resolved;
    return out;
}

// The normal `n` tilted by `beads`, given how the surface's own coordinates and its world position run
// across neighbouring pixels (`dpdx`/`dpdy` of each): Mikkelsen's surface gradient, with the height's
// screen derivatives taken from the beads' own gradient — exact, where differentiating the beads would
// see only their pixels.
fn bead_normal(n: vec3<f32>, beads: Beads, local_dx: vec3<f32>, local_dy: vec3<f32>, world_dx: vec3<f32>, world_dy: vec3<f32>) -> vec3<f32> {
    if (beads.cover <= 0.0) {
        return n;
    }
    let r1 = cross(world_dy, n);
    let r2 = cross(n, world_dx);
    let det = dot(world_dx, r1);
    if (abs(det) < 1e-14) {
        return n;
    }
    let gradient = sign(det) * (dot(beads.slope, local_dx) * r1 + dot(beads.slope, local_dy) * r2);
    return normalize(abs(det) * n - gradient);
}

// A roughness with the beads on it: water's where they stand, widened by how much a bead's normal turns
// across the pixel (`Beads::variance`) — `α² + σ²`, with `α` = roughness².
fn bead_roughness(roughness: f32, beads: Beads) -> f32 {
    let a = BEAD_ROUGHNESS * BEAD_ROUGHNESS;
    return mix(roughness, sqrt(sqrt(a * a + beads.variance)), beads.cover);
}

// The surface with `beads` on it (`rain_beads`), its own coordinates and world position running
// `local_dx`/`local_dy` and `world_dx`/`world_dy` across neighbouring pixels.
fn beaded(s: Surface, beads: Beads, local_dx: vec3<f32>, local_dy: vec3<f32>, world_dx: vec3<f32>, world_dy: vec3<f32>) -> Surface {
    var out = s;
    out.N = bead_normal(s.N, beads, local_dx, local_dy, world_dx, world_dy);
    out.roughness = bead_roughness(s.roughness, beads);
    return out;
}

// Four uniform numbers for a cell of a grid (PCG).
fn bead_hash(i: vec4<i32>) -> vec4<f32> {
    var v = bitcast<vec4<u32>>(i) * 1664525u + vec4<u32>(1013904223u);
    v.x += v.y * v.w;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v.w += v.y * v.z;
    v = v ^ (v >> vec4<u32>(16u));
    v.x += v.y * v.w;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v.w += v.y * v.z;
    return vec4<f32>(v) * (1.0 / 4294967296.0);
}

// A roughness as rain leaves it: no rougher than it was, but for the rain's rings too fine for the pixel
// (`ripples.z`), whose slopes' variance joins the microfacets' — `α² + σ²`, with `α` = roughness².
fn wet_roughness(roughness: f32, rain: vec3<f32>, ripples: vec3<f32>) -> f32 {
    let soaked = min(roughness, mix(roughness, WET_ROUGHNESS, rain.x));
    let still = mix(soaked, min(soaked, PUDDLE_ROUGHNESS), rain.y);
    let a = still * still;
    return sqrt(sqrt(a * a + ripples.z));
}

// Two octaves of value noise over the ground, in [0, 1].
fn puddle_noise(p: vec2<f32>) -> f32 {
    return 0.65 * value_noise(p) + 0.35 * value_noise(p * 2.7 + vec2<f32>(17.3, 5.1));
}

fn value_noise(p: vec2<f32>) -> f32 {
    let i = vec2<i32>(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = lattice(i);
    let b = lattice(i + vec2<i32>(1, 0));
    let c = lattice(i + vec2<i32>(0, 1));
    let d = lattice(i + vec2<i32>(1, 1));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// A uniform number for a lattice point (PCG).
fn lattice(i: vec2<i32>) -> f32 {
    var v = bitcast<vec2<u32>>(i) * 1664525u + vec2<u32>(1013904223u);
    v.x += v.y * 1664525u;
    v.y += v.x * 1664525u;
    v = v ^ (v >> vec2<u32>(16u));
    v.x += v.y * 1664525u;
    v.y += v.x * 1664525u;
    v = v ^ (v >> vec2<u32>(16u));
    return f32(v.x) * (1.0 / 4294967296.0);
}
