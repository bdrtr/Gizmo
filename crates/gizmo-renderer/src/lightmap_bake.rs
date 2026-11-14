//! Lightmap baking: the light that lands on static surfaces, traced on the CPU into the picture a
//! material reads as [`MaterialMaps::lightmap`](crate::components::MaterialMaps::lightmap).
//!
//! ```ignore
//! let mut bake = LightmapBake::new().with_sky(Sky::uniform([0.3, 0.35, 0.4]));
//! let floor = bake.add_mesh(BakeMesh::new(floor_vertices, Mat4::IDENTITY).receiving(128, 128));
//! bake.add_mesh(BakeMesh::new(cube_vertices, cube_transform).with_albedo(Vec3::splat(0.8)));
//! bake.add_mesh(BakeMesh::new(panel_vertices, panel_transform).with_emissive(Vec3::splat(4.0)));
//! bake.add_light(BakeLight::directional(sun_direction, Vec3::ONE, 3.0));
//! let baked = bake.run()?;
//! let intensity = baked.get(floor).unwrap().save_png("floor_lightmap.png")?;
//! material.maps = MaterialMaps::default().with_lightmap("floor_lightmap.png", intensity);
//! ```
//!
//! ## What a texel holds
//!
//! What the shaders put in place of the ambient light: a lightmapped surface draws
//! `albedo × lightmap` where an unlit-by-lights one would draw `albedo × sky`. So a texel is the
//! irradiance at the surface **divided by π** — under a uniform sky of colour `c` and nothing else,
//! exactly `c`, which is what the surface showed before it had a lightmap. It holds:
//!
//! * the sky ([`LightmapBake::with_sky`]), where the surface can see it;
//! * what emissive surfaces send ([`BakeMesh::with_emissive`]) — an area light is a glowing mesh;
//! * every light's light **bounced** off other surfaces, [`BakeSettings::bounces`] times;
//! * the **direct** light of a light that exists only in the bake ([`BakeLight::baked_only`]). A
//!   light also in the scene at run time adds its direct light and shadow there, every frame, so
//!   the bake leaves it out — baking it too would light the surface twice.
//!
//! Direct light is computed as the shaders compute it — the same inverse-square falloff windowed at
//! the light's radius (`light_falloff` in `common.wgsl`), the same spot cone — with a ray toward the
//! light for its shadow. Everything else is path traced: [`BakeSettings::samples`] cosine-weighted
//! rays a texel, stratified, from a generator seeded by the texel, so the bake is the same on every
//! run and on any number of threads.
//!
//! A ray that reaches the back of a surface sees black: the inside of a closed solid holds no
//! light, and letting it through would leak the sky under walls.
//!
//! ## The second UV set
//!
//! A receiver's texels are laid out by [`Vertex::lightmap_uv`]: `(0,0)` is the image's top-left
//! corner, as the GPU samples it. The set must not overlap itself (a texel covered twice takes the
//! first triangle's light) and should leave a texel or two between islands — the bake fills
//! [`BakeSettings::padding`] texels around every island so bilinear filtering does not pull in the
//! black around it. Each receiver has its own image; one atlas over several meshes is laid out by
//! giving their UVs disjoint ranges and baking them as one mesh.

use std::path::Path;

use gizmo_math::{Mat4, Vec2, Vec3};

use crate::frame_uniforms::Sky;
use crate::gpu_types::Vertex;

/// How long and how carefully [`LightmapBake::run`] traces.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct BakeSettings {
    /// Rays a texel. Noise falls as one over its square root: 64 is a preview, 256 the default,
    /// 1024 clean.
    pub samples: u32,
    /// How many times light may bounce off a surface on its way to a texel. `0`: only the sky,
    /// emissive surfaces and baked-only lights' direct light; `2` (the default) is most of what a room
    /// gets.
    pub bounces: u32,
    /// Texels filled around every UV island from its edge, so bilinear filtering does not reach the
    /// empty texels between islands. Default 2.
    pub padding: u32,
    /// Seeds the generator. The same seed and scene give the same lightmap bit for bit.
    pub seed: u64,
}

impl Default for BakeSettings {
    fn default() -> Self {
        Self { samples: 256, bounces: 2, padding: 2, seed: 0 }
    }
}

impl BakeSettings {
    /// These settings with `samples` rays a texel (at least 1).
    #[must_use]
    pub fn with_samples(mut self, samples: u32) -> Self {
        self.samples = samples.max(1);
        self
    }

    /// These settings with `bounces` bounces.
    #[must_use]
    pub fn with_bounces(mut self, bounces: u32) -> Self {
        self.bounces = bounces;
        self
    }

    /// These settings filling `padding` texels around every island.
    #[must_use]
    pub fn with_padding(mut self, padding: u32) -> Self {
        self.padding = padding;
        self
    }

    /// These settings with another seed.
    #[must_use]
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }
}

/// A mesh in the bake: something that blocks light and bounces it, glows if it is emissive, and is
/// given a lightmap of its own if it [`receives`](Self::receiving) one.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct BakeMesh {
    /// A flat, counter-clockwise triangle list in the mesh's own space — the engine's one layout
    /// (`MeshData::vertices`, `Mesh::from_vertices`).
    pub vertices: Vec<Vertex>,
    /// Model to world.
    pub transform: Mat4,
    /// The fraction of light it bounces, linear RGB. Default 0.8 grey.
    pub albedo: Vec3,
    /// The light it sends, linear RGB, in the units `Material::emissive` is drawn in. Default none.
    pub emissive: Vec3,
    /// The size of its lightmap, in texels, or `None` for a mesh that only blocks and bounces.
    pub lightmap_size: Option<(u32, u32)>,
}

impl BakeMesh {
    /// A grey mesh that blocks and bounces light and receives none.
    #[must_use]
    pub fn new(vertices: impl Into<Vec<Vertex>>, transform: Mat4) -> Self {
        Self { vertices: vertices.into(), transform, albedo: Vec3::splat(0.8), emissive: Vec3::ZERO, lightmap_size: None }
    }

    /// A mesh from positions alone, a flat triangle list — something that blocks and bounces light.
    /// Its normals are its faces'; it has no second UV set, so it cannot [`receive`](Self::receiving).
    #[must_use]
    pub fn from_positions(positions: &[Vec3], transform: Mat4) -> Self {
        let mut vertices = Vec::with_capacity(positions.len());
        for tri in positions.as_chunks::<3>().0 {
            let n = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize_or_zero().to_array();
            vertices.extend(tri.iter().map(|p| Vertex { position: p.to_array(), normal: n, ..Default::default() }));
        }
        Self::new(vertices, transform)
    }

    /// A drawn [`Mesh`](crate::components::Mesh), from the triangles it keeps on the CPU — the shape
    /// the scene draws. A mesh with a second UV set keeps its normals and that set too
    /// (`Mesh::cpu_lightmap`), so it can [`receive`](Self::receiving); any other blocks and bounces
    /// light with its faces' normals. `transform` is the entity's world matrix.
    #[must_use]
    pub fn from_mesh(mesh: &crate::components::Mesh, transform: Mat4) -> Self {
        match mesh.lightmap_vertices() {
            Some(vertices) => Self::new(vertices, transform),
            None => Self::from_positions(&mesh.cpu_vertices, transform),
        }
    }

    /// This mesh bouncing `albedo` of the light that reaches it (each channel clamped to 0..=1).
    #[must_use]
    pub fn with_albedo(mut self, albedo: Vec3) -> Self {
        self.albedo = albedo.clamp(Vec3::ZERO, Vec3::ONE);
        self
    }

    /// This mesh glowing `emissive` — an area light, if it is bright.
    #[must_use]
    pub fn with_emissive(mut self, emissive: Vec3) -> Self {
        self.emissive = emissive.max(Vec3::ZERO);
        self
    }

    /// This mesh given a `width × height` lightmap, laid out by its second UV set.
    #[must_use]
    pub fn receiving(mut self, width: u32, height: u32) -> Self {
        self.lightmap_size = Some((width, height));
        self
    }
}

/// Which of a [`BakeLight`]'s shapes it is.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum BakeLightKind {
    /// Parallel rays travelling along `direction` (unit), from infinitely far.
    Directional {
        /// The way the light travels — down for a noon sun.
        direction: Vec3,
    },
    /// Equal in all directions from `position`, reaching zero at `radius`.
    Point {
        /// Where it is.
        position: Vec3,
        /// The distance its falloff reaches zero at (`PointLight::radius`).
        radius: f32,
    },
    /// A cone from `position` along `direction`.
    Spot {
        /// Where it is.
        position: Vec3,
        /// The way the cone points (unit).
        direction: Vec3,
        /// The distance its falloff reaches zero at.
        radius: f32,
        /// Cosine of the fully-lit core's half-angle.
        cos_inner: f32,
        /// Cosine of the half-angle where it reaches zero.
        cos_outer: f32,
    },
}

/// A light in the bake. By default it is also in the scene at run time, so only its bounced light
/// is baked; [`baked_only`](Self::baked_only) bakes its direct light and shadow too.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct BakeLight {
    /// Its shape and place.
    pub kind: BakeLightKind,
    /// Linear RGB.
    pub color: Vec3,
    /// Brightness, as the light component's `intensity`.
    pub intensity: f32,
    /// Whether its direct light goes into the lightmap — true for a light the scene does not have
    /// at run time.
    pub bake_direct: bool,
}

impl BakeLight {
    /// A sun whose light travels along `direction`.
    #[must_use]
    pub fn directional(direction: Vec3, color: Vec3, intensity: f32) -> Self {
        let direction = direction.try_normalize().unwrap_or(Vec3::NEG_Y);
        Self::of(BakeLightKind::Directional { direction }, color, intensity)
    }

    /// A point light at `position` reaching `radius`.
    #[must_use]
    pub fn point(position: Vec3, color: Vec3, intensity: f32, radius: f32) -> Self {
        Self::of(BakeLightKind::Point { position, radius: radius.max(0.001) }, color, intensity)
    }

    /// A spot light at `position` pointing along `direction`, with its core and outer half-angles in
    /// radians.
    #[must_use]
    pub fn spot(position: Vec3, direction: Vec3, color: Vec3, intensity: f32, radius: f32, inner_angle: f32, outer_angle: f32) -> Self {
        let direction = direction.try_normalize().unwrap_or(Vec3::NEG_Y);
        let outer = outer_angle.max(0.0);
        let inner = inner_angle.clamp(0.0, outer);
        Self::of(
            BakeLightKind::Spot { position, direction, radius: radius.max(0.001), cos_inner: inner.cos(), cos_outer: outer.cos() },
            color,
            intensity,
        )
    }

    /// The scene's `DirectionalLight`, travelling along `direction` (its `Transform`'s rotation
    /// applied to −Z).
    #[must_use]
    pub fn from_directional(light: &crate::components::DirectionalLight, direction: Vec3) -> Self {
        Self::directional(direction, light.color, light.intensity)
    }

    /// The scene's `PointLight`, at `position`.
    #[must_use]
    pub fn from_point(light: &crate::components::PointLight, position: Vec3) -> Self {
        Self::point(position, light.color, light.intensity, light.radius)
    }

    /// The scene's `SpotLight`, at `position` pointing along `direction`. Its cookie is not baked.
    #[must_use]
    pub fn from_spot(light: &crate::components::SpotLight, position: Vec3, direction: Vec3) -> Self {
        Self::spot(position, direction, light.color, light.intensity, light.radius, light.inner_angle, light.outer_angle)
    }

    /// This light with its direct light and shadow baked too — for a light the scene does not have
    /// at run time.
    #[must_use]
    pub fn baked_only(mut self) -> Self {
        self.bake_direct = true;
        self
    }

    fn of(kind: BakeLightKind, color: Vec3, intensity: f32) -> Self {
        Self { kind, color: color.max(Vec3::ZERO), intensity: intensity.max(0.0), bake_direct: false }
    }
}

/// Names a mesh added to a [`LightmapBake`], to find its lightmap in [`BakedLightmaps`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BakeMeshId(usize);

/// What can go wrong in a bake.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum LightmapError {
    /// A receiver asked for a lightmap with no texels.
    EmptyLightmap {
        /// The mesh.
        mesh: BakeMeshId,
    },
    /// A receiver asked for a lightmap bigger than [`MAX_LIGHTMAP_SIZE`] on a side.
    TooLarge {
        /// The mesh.
        mesh: BakeMeshId,
    },
    /// No texel of a receiver's lightmap lies on one of its triangles: its second UV set is
    /// missing (all zero) or outside 0..1.
    NothingCovered {
        /// The mesh.
        mesh: BakeMeshId,
    },
    /// The lightmap could not be written.
    Write(String),
}

impl std::fmt::Display for LightmapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyLightmap { mesh } => write!(f, "mesh {} asked for a lightmap with no texels", mesh.0),
            Self::TooLarge { mesh } => write!(f, "mesh {} asked for a lightmap over {MAX_LIGHTMAP_SIZE} texels a side", mesh.0),
            Self::NothingCovered { mesh } => write!(
                f,
                "no texel of mesh {}'s lightmap lies on a triangle: its second UV set (Vertex::lightmap_uv) is missing or outside 0..1",
                mesh.0
            ),
            Self::Write(e) => write!(f, "the lightmap could not be written: {e}"),
        }
    }
}

impl std::error::Error for LightmapError {}

/// The largest lightmap side [`LightmapBake::run`] takes, in texels.
pub const MAX_LIGHTMAP_SIZE: u32 = 8192;

/// A scene to bake: meshes, lights, a sky. See the [module](self).
#[derive(Debug, Clone, Default)]
pub struct LightmapBake {
    meshes: Vec<BakeMesh>,
    lights: Vec<BakeLight>,
    sky: Option<Sky>,
    settings: BakeSettings,
}

impl LightmapBake {
    /// An empty scene under a black sky, with the default [`BakeSettings`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// This bake under `sky` — the one the scene sets with `Renderer::sky`, so a lightmapped surface
    /// and its neighbours agree.
    #[must_use]
    pub fn with_sky(mut self, sky: Sky) -> Self {
        self.sky = Some(sky);
        self
    }

    /// This bake with other settings.
    #[must_use]
    pub fn with_settings(mut self, settings: BakeSettings) -> Self {
        self.settings = settings;
        self
    }

    /// Adds a mesh; the id finds its lightmap after the bake.
    pub fn add_mesh(&mut self, mesh: BakeMesh) -> BakeMeshId {
        self.meshes.push(mesh);
        BakeMeshId(self.meshes.len() - 1)
    }

    /// Adds a light.
    pub fn add_light(&mut self, light: BakeLight) {
        self.lights.push(light);
    }

    /// Traces every receiver's lightmap. Runs on every core on native targets, on the calling
    /// thread on the web; the result does not depend on which.
    pub fn run(&self) -> Result<BakedLightmaps, LightmapError> {
        for (i, mesh) in self.meshes.iter().enumerate() {
            if let Some((w, h)) = mesh.lightmap_size {
                if w == 0 || h == 0 {
                    return Err(LightmapError::EmptyLightmap { mesh: BakeMeshId(i) });
                }
                if w > MAX_LIGHTMAP_SIZE || h > MAX_LIGHTMAP_SIZE {
                    return Err(LightmapError::TooLarge { mesh: BakeMeshId(i) });
                }
            }
        }
        let scene = Scene::build(self);
        let mut maps = Vec::with_capacity(self.meshes.len());
        for (i, mesh) in self.meshes.iter().enumerate() {
            let Some((width, height)) = mesh.lightmap_size else {
                maps.push(None);
                continue;
            };
            let texels = rasterize(&scene, i, width, height);
            if texels.is_empty() {
                return Err(LightmapError::NothingCovered { mesh: BakeMeshId(i) });
            }
            let lit = trace_all(&scene, &self.settings, i as u64, &texels);
            let mut map = Lightmap { width, height, texels: vec![Vec3::ZERO; (width * height) as usize] };
            let mut covered = vec![false; map.texels.len()];
            for (texel, light) in texels.iter().zip(lit) {
                let at = (texel.y * width + texel.x) as usize;
                map.texels[at] = light;
                covered[at] = true;
            }
            dilate(&mut map, &mut covered, self.settings.padding);
            maps.push(Some(map));
        }
        Ok(BakedLightmaps { maps })
    }
}

/// The lightmaps a bake made, one a receiver.
#[derive(Debug, Clone)]
pub struct BakedLightmaps {
    maps: Vec<Option<Lightmap>>,
}

impl BakedLightmaps {
    /// The lightmap of `mesh`, or `None` for a mesh that did not receive one.
    #[must_use]
    pub fn get(&self, mesh: BakeMeshId) -> Option<&Lightmap> {
        self.maps.get(mesh.0)?.as_ref()
    }
}

/// One baked lightmap: linear RGB irradiance over π a texel (see the [module](self)), rows from the
/// top.
#[derive(Debug, Clone, PartialEq)]
pub struct Lightmap {
    width: u32,
    height: u32,
    texels: Vec<Vec3>,
}

impl Lightmap {
    /// Width in texels.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in texels.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The light at texel `(x, y)`, `(0, 0)` the top-left. Panics outside the image.
    #[must_use]
    pub fn texel(&self, x: u32, y: u32) -> Vec3 {
        assert!(x < self.width && y < self.height, "texel ({x}, {y}) outside a {}×{} lightmap", self.width, self.height);
        self.texels[(y * self.width + x) as usize]
    }

    /// Every texel, rows from the top.
    #[must_use]
    pub fn texels(&self) -> &[Vec3] {
        &self.texels
    }

    /// The lightmap as 8-bit RGBA, scaled so its brightest channel is 255, and the scale: the
    /// `intensity` to give `MaterialMaps::with_lightmap` so it draws as baked. A black lightmap's
    /// scale is 1.
    #[must_use]
    pub fn to_rgba8(&self) -> (Vec<u8>, f32) {
        let max = self.texels.iter().fold(0.0f32, |m, t| m.max(t.max_element()));
        let scale = if max > 0.0 && max.is_finite() { max } else { 1.0 };
        let mut bytes = Vec::with_capacity(self.texels.len() * 4);
        for t in &self.texels {
            let c = (*t / scale).clamp(Vec3::ZERO, Vec3::ONE) * 255.0;
            bytes.extend_from_slice(&[c.x.round() as u8, c.y.round() as u8, c.z.round() as u8, 255]);
        }
        (bytes, scale)
    }

    /// Writes [`to_rgba8`](Self::to_rgba8) to `path` as a PNG and returns its scale, the intensity
    /// to give `MaterialMaps::with_lightmap`.
    pub fn save_png(&self, path: impl AsRef<Path>) -> Result<f32, LightmapError> {
        let (bytes, scale) = self.to_rgba8();
        let image = image::RgbaImage::from_raw(self.width, self.height, bytes)
            .ok_or_else(|| LightmapError::Write("texel count does not match the size".into()))?;
        image.save(path.as_ref()).map_err(|e| LightmapError::Write(e.to_string()))?;
        Ok(scale)
    }
}

// ---------------------------------------------------------------------------------------------
// The scene: world-space triangles under a BVH.

#[derive(Clone, Copy)]
struct Triangle {
    v0: Vec3,
    e1: Vec3,
    e2: Vec3,
    /// Unit geometric normal, from the winding (counter-clockwise front).
    normal: Vec3,
    /// World-space vertex normals, for shading.
    n: [Vec3; 3],
    mesh: u32,
}

impl Triangle {
    fn shading_normal(&self, u: f32, v: f32) -> Vec3 {
        let n = self.n[0] * (1.0 - u - v) + self.n[1] * u + self.n[2] * v;
        // A vertex normal on the back of the face is a mesh that disagrees with itself; the
        // face's own is the one that keeps light on the right side.
        match n.try_normalize() {
            Some(n) if n.dot(self.normal) > 0.0 => n,
            _ => self.normal,
        }
    }
}

struct Node {
    min: Vec3,
    max: Vec3,
    /// A leaf: `count > 0`, triangles `first..first + count`. An inner node: children at `first`
    /// and `first + 1`.
    first: u32,
    count: u32,
}

struct Scene<'a> {
    tris: Vec<Triangle>,
    nodes: Vec<Node>,
    meshes: &'a [BakeMesh],
    lights: &'a [BakeLight],
    sky: Option<Sky>,
    /// How far a ray starts off its surface, from the scene's size.
    bias: f32,
    /// Each receiver's triangles, as indices into `tris`, in its vertex order.
    by_mesh: Vec<Vec<u32>>,
    /// Each triangle's lightmap UVs, by the same index.
    uvs: Vec<[Vec2; 3]>,
}

impl<'a> Scene<'a> {
    fn build(bake: &'a LightmapBake) -> Self {
        let mut tris = Vec::new();
        let mut uvs = Vec::new();
        let mut by_mesh = Vec::with_capacity(bake.meshes.len());
        for (m, mesh) in bake.meshes.iter().enumerate() {
            let normal_matrix = mesh.transform.inverse().transpose();
            let mut mine = Vec::new();
            for tri in mesh.vertices.as_chunks::<3>().0 {
                let p = tri.map(|v| mesh.transform.transform_point3(Vec3::from(v.position)));
                let e1 = p[1] - p[0];
                let e2 = p[2] - p[0];
                let Some(normal) = e1.cross(e2).try_normalize() else { continue };
                let n = tri.map(|v| normal_matrix.transform_vector3(Vec3::from(v.normal)).try_normalize().unwrap_or(normal));
                mine.push(tris.len() as u32);
                uvs.push(tri.map(|v| Vec2::from(v.lightmap_uv)));
                tris.push(Triangle { v0: p[0], e1, e2, normal, n, mesh: m as u32 });
            }
            by_mesh.push(mine);
        }
        let (lo, hi) = tris.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), t| {
            let (a, b) = tri_bounds(t);
            (lo.min(a), hi.max(b))
        });
        let extent = if tris.is_empty() { 1.0 } else { (hi - lo).max_element() };
        let bias = (extent * 2e-5).max(1e-4);

        // The BVH sorts `tris`; the per-mesh lists and UVs follow through the permutation.
        let mut order: Vec<u32> = (0..tris.len() as u32).collect();
        let mut nodes = Vec::new();
        if !tris.is_empty() {
            nodes.push(Node { min: Vec3::ZERO, max: Vec3::ZERO, first: 0, count: 0 });
            build_node(&tris, &mut order, 0, tris.len(), &mut nodes, 0);
        }
        let sorted: Vec<Triangle> = order.iter().map(|&i| tris[i as usize]).collect();
        let mut new_index = vec![0u32; order.len()];
        for (new, &old) in order.iter().enumerate() {
            new_index[old as usize] = new as u32;
        }
        for list in &mut by_mesh {
            for i in list.iter_mut() {
                *i = new_index[*i as usize];
            }
        }
        let mut sorted_uvs = vec![[Vec2::ZERO; 3]; uvs.len()];
        for (old, uv) in uvs.into_iter().enumerate() {
            sorted_uvs[new_index[old] as usize] = uv;
        }
        Self { tris: sorted, nodes, meshes: &bake.meshes, lights: &bake.lights, sky: bake.sky, bias, by_mesh, uvs: sorted_uvs }
    }

    /// The nearest surface along the ray, as (triangle, t, u, v).
    fn hit(&self, origin: Vec3, dir: Vec3, t_max: f32) -> Option<(u32, f32, f32, f32)> {
        if self.nodes.is_empty() {
            return None;
        }
        let inv = dir.recip();
        let mut best: Option<(u32, f32, f32, f32)> = None;
        let mut limit = t_max;
        let mut stack = [0u32; 64];
        let mut top = 1;
        while top > 0 {
            top -= 1;
            let node = &self.nodes[stack[top] as usize];
            if slab(origin, inv, node.min, node.max, limit).is_none() {
                continue;
            }
            if node.count > 0 {
                for i in node.first..node.first + node.count {
                    if let Some((t, u, v)) = intersect(&self.tris[i as usize], origin, dir) {
                        if t < limit {
                            limit = t;
                            best = Some((i, t, u, v));
                        }
                    }
                }
            } else {
                // Nearer child last, so it is visited first.
                let (a, b) = (node.first, node.first + 1);
                let da = slab(origin, inv, self.nodes[a as usize].min, self.nodes[a as usize].max, limit);
                let db = slab(origin, inv, self.nodes[b as usize].min, self.nodes[b as usize].max, limit);
                let (near, far, near_hit, far_hit) = match (da, db) {
                    (Some(x), Some(y)) if y < x => (b, a, db, da),
                    _ => (a, b, da, db),
                };
                // A median split halves every level: 64 levels is 2^64 triangles.
                if far_hit.is_some() {
                    stack[top] = far;
                    top += 1;
                }
                if near_hit.is_some() {
                    stack[top] = near;
                    top += 1;
                }
            }
        }
        best
    }

    fn sky(&self, dir: Vec3) -> Vec3 {
        let Some(sky) = self.sky else { return Vec3::ZERO };
        // `sky_gradient` in `common.wgsl`, sharp: what a ray in this direction sees.
        let (zenith, horizon, ground) = (Vec3::from(sky.zenith), Vec3::from(sky.horizon), Vec3::from(sky.ground));
        let y = dir.y.clamp(-1.0, 1.0);
        let c = if y >= 0.0 { horizon.lerp(zenith, y.sqrt()) } else { horizon.lerp(ground, (-y).sqrt()) };
        c * sky.intensity
    }

    /// The direct light of every light (or only the baked-only ones) at a point, in texel units —
    /// irradiance over π.
    fn direct(&self, p: Vec3, n: Vec3, geo: Vec3, only_baked: bool) -> Vec3 {
        let origin = p + geo * self.bias;
        let mut sum = Vec3::ZERO;
        for light in self.lights {
            if only_baked && !light.bake_direct {
                continue;
            }
            let (to_light, distance, atten) = match light.kind {
                BakeLightKind::Directional { direction } => (-direction, f32::INFINITY, 1.0),
                BakeLightKind::Point { position, radius } => {
                    let d = position - p;
                    let dist = d.length();
                    (d / dist.max(1e-6), dist, falloff(dist, radius))
                }
                BakeLightKind::Spot { position, direction, radius, cos_inner, cos_outer } => {
                    let d = position - p;
                    let dist = d.length();
                    let l = d / dist.max(1e-6);
                    let ramp = ((-l).dot(direction) - cos_outer) / (cos_inner - cos_outer).max(0.001);
                    let ramp = ramp.clamp(0.0, 1.0);
                    (l, dist, falloff(dist, radius) * ramp * ramp)
                }
            };
            let cos = n.dot(to_light);
            if cos <= 0.0 || atten <= 0.0 || geo.dot(to_light) <= 0.0 {
                continue;
            }
            let reach = if distance.is_finite() { (distance - self.bias * 2.0).max(0.0) } else { f32::MAX };
            if self.hit(origin, to_light, reach).is_some() {
                continue;
            }
            sum += light.color * (light.intensity * atten * cos * std::f32::consts::FRAC_1_PI);
        }
        sum
    }

    /// The light arriving at `origin` along `dir`, having bounced `depth` times already.
    fn incoming(&self, origin: Vec3, dir: Vec3, depth: u32, bounces: u32, rng: &mut Rng) -> Vec3 {
        let Some((i, t, u, v)) = self.hit(origin, dir, f32::MAX) else { return self.sky(dir) };
        let tri = &self.tris[i as usize];
        if tri.normal.dot(dir) >= 0.0 {
            // The back of a surface: the inside of a solid.
            return Vec3::ZERO;
        }
        let mesh = &self.meshes[tri.mesh as usize];
        let mut light = mesh.emissive;
        if depth < bounces && mesh.albedo != Vec3::ZERO {
            let p = origin + dir * t;
            let n = tri.shading_normal(u, v);
            let mut arriving = self.direct(p, n, tri.normal, false);
            let next = cosine_sample(n, rng.next(), rng.next());
            if next.dot(tri.normal) > 0.0 {
                arriving += self.incoming(p + tri.normal * self.bias, next, depth + 1, bounces, rng);
            }
            light += mesh.albedo * arriving;
        }
        light
    }
}

fn tri_bounds(t: &Triangle) -> (Vec3, Vec3) {
    let (a, b, c) = (t.v0, t.v0 + t.e1, t.v0 + t.e2);
    (a.min(b).min(c), a.max(b).max(c))
}

/// Builds the node at `slot` over `order[first..end]`: a leaf of up to four triangles, or two
/// children side by side at the end of `nodes`, split at the median centroid of the longest axis.
fn build_node(tris: &[Triangle], order: &mut [u32], first: usize, end: usize, nodes: &mut Vec<Node>, slot: usize) {
    let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    let (mut cmin, mut cmax) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for &i in &order[first..end] {
        let (a, b) = tri_bounds(&tris[i as usize]);
        min = min.min(a);
        max = max.max(b);
        let c = (a + b) * 0.5;
        cmin = cmin.min(c);
        cmax = cmax.max(c);
    }
    nodes[slot] = Node { min, max, first: first as u32, count: (end - first) as u32 };
    let extent = cmax - cmin;
    let axis = if extent.x >= extent.y && extent.x >= extent.z { 0 } else if extent.y >= extent.z { 1 } else { 2 };
    if end - first <= 4 || extent[axis] <= 0.0 {
        return;
    }
    let centre = |i: u32| {
        let (a, b) = tri_bounds(&tris[i as usize]);
        (a[axis] + b[axis]) * 0.5
    };
    let mid = first + (end - first) / 2;
    order[first..end].select_nth_unstable_by(mid - first, |&a, &b| centre(a).total_cmp(&centre(b)));
    let left = nodes.len();
    let placeholder = || Node { min: Vec3::ZERO, max: Vec3::ZERO, first: 0, count: 0 };
    nodes.push(placeholder());
    nodes.push(placeholder());
    build_node(tris, order, first, mid, nodes, left);
    build_node(tris, order, mid, end, nodes, left + 1);
    nodes[slot].first = left as u32;
    nodes[slot].count = 0;
}

fn slab(origin: Vec3, inv: Vec3, min: Vec3, max: Vec3, limit: f32) -> Option<f32> {
    let t0 = (min - origin) * inv;
    let t1 = (max - origin) * inv;
    let near = t0.min(t1).max_element().max(0.0);
    let far = t0.max(t1).min_element().min(limit);
    // NaN (a zero direction component on the slab's plane) fails both comparisons: kept.
    if near > far { None } else { Some(near) }
}

/// Möller–Trumbore, both faces: (t, u, v).
fn intersect(tri: &Triangle, origin: Vec3, dir: Vec3) -> Option<(f32, f32, f32)> {
    let h = dir.cross(tri.e2);
    let a = tri.e1.dot(h);
    if a.abs() < 1e-12 {
        return None;
    }
    let f = 1.0 / a;
    let s = origin - tri.v0;
    let u = f * s.dot(h);
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(tri.e1);
    let v = f * dir.dot(q);
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = f * tri.e2.dot(q);
    (t > 0.0).then_some((t, u, v))
}

/// `light_falloff` in `common.wgsl`: the inverse square, windowed to nothing at `radius`.
fn falloff(distance: f32, radius: f32) -> f32 {
    let r = distance / radius.max(0.001);
    let w = (1.0 - r * r * r * r).clamp(0.0, 1.0);
    w * w / (distance * distance + 0.01)
}

fn cosine_sample(n: Vec3, a: f32, b: f32) -> Vec3 {
    let (t, bt) = n.any_orthonormal_pair();
    let r = a.sqrt();
    let phi = b * std::f32::consts::TAU;
    let (x, y) = (r * phi.cos(), r * phi.sin());
    let z = (1.0 - a).max(0.0).sqrt();
    (t * x + bt * y + n * z).normalize_or(n)
}

/// A small, fast generator (SplitMix64), seeded per texel so the bake does not depend on threads.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        let mut r = Self(seed);
        r.step();
        r
    }

    fn step(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    fn next(&mut self) -> f32 {
        (self.step() >> 40) as f32 / (1u64 << 24) as f32
    }
}

// ---------------------------------------------------------------------------------------------
// Texels: where each one is on the surface.

#[derive(Clone, Copy)]
struct Texel {
    x: u32,
    y: u32,
    position: Vec3,
    normal: Vec3,
    geo: Vec3,
}

/// Every texel of mesh `m`'s lightmap that lies on (or within a texel of) one of its triangles,
/// with the surface point it stands for.
fn rasterize(scene: &Scene<'_>, m: usize, width: u32, height: u32) -> Vec<Texel> {
    let size = Vec2::new(width as f32, height as f32);
    let mut owner: Vec<Option<(u32, f32, f32)>> = vec![None; (width * height) as usize];
    let mut near: Vec<Option<(u32, f32, f32, f32)>> = vec![None; (width * height) as usize];
    for &i in &scene.by_mesh[m] {
        let uv = scene.uvs[i as usize].map(|t| t * size);
        let lo = uv[0].min(uv[1]).min(uv[2]) - Vec2::ONE;
        let hi = uv[0].max(uv[1]).max(uv[2]) + Vec2::ONE;
        let (x0, y0) = (lo.x.floor().max(0.0) as u32, lo.y.floor().max(0.0) as u32);
        let (x1, y1) = (hi.x.ceil().min(size.x - 1.0).max(0.0) as u32, hi.y.ceil().min(size.y - 1.0).max(0.0) as u32);
        if lo.x > size.x || lo.y > size.y || hi.x < 0.0 || hi.y < 0.0 {
            continue;
        }
        let d = (uv[1] - uv[0]).perp_dot(uv[2] - uv[0]);
        if d.abs() < 1e-12 {
            continue;
        }
        for y in y0..=y1 {
            for x in x0..=x1 {
                let at = (y * width + x) as usize;
                if owner[at].is_some() {
                    continue;
                }
                let c = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                let u = (c - uv[0]).perp_dot(uv[2] - uv[0]) / d;
                let v = (uv[1] - uv[0]).perp_dot(c - uv[0]) / d;
                if u >= 0.0 && v >= 0.0 && u + v <= 1.0 {
                    owner[at] = Some((i, u, v));
                } else {
                    // Just off the triangle: the nearest point on it, if within a texel — an edge
                    // texel the filter reads half of, sampled on the surface instead of guessed.
                    let (cu, cv) = clamp_barycentric(u, v);
                    let q = uv[0] + (uv[1] - uv[0]) * cu + (uv[2] - uv[0]) * cv;
                    let dist = q.distance(c);
                    if dist <= 0.75 && near[at].is_none_or(|(_, _, _, best)| dist < best) {
                        near[at] = Some((i, cu, cv, dist));
                    }
                }
            }
        }
    }
    let mut texels = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let at = (y * width + x) as usize;
            let found = owner[at].or(near[at].map(|(i, u, v, _)| (i, u, v)));
            if let Some((i, u, v)) = found {
                let tri = &scene.tris[i as usize];
                texels.push(Texel {
                    x,
                    y,
                    position: tri.v0 + tri.e1 * u + tri.e2 * v,
                    normal: tri.shading_normal(u, v),
                    geo: tri.normal,
                });
            }
        }
    }
    texels
}

/// The barycentric point of the triangle nearest `(u, v)` in barycentric space — close enough to
/// the nearest point in the plane for the texel's worth of distance it is used over.
fn clamp_barycentric(u: f32, v: f32) -> (f32, f32) {
    let (mut u, mut v) = (u.max(0.0), v.max(0.0));
    let s = u + v;
    if s > 1.0 {
        u /= s;
        v /= s;
    }
    (u, v)
}

fn trace_all(scene: &Scene<'_>, settings: &BakeSettings, mesh: u64, texels: &[Texel]) -> Vec<Vec3> {
    let shade = |t: &Texel| -> Vec3 {
        let mut rng = Rng::new(
            settings.seed ^ mesh.wrapping_mul(0xA24B_AED4_963E_E407) ^ (u64::from(t.y) << 32 | u64::from(t.x)).wrapping_mul(0x9FB2_1C65_1E98_DF25),
        );
        let origin = t.position + t.geo * scene.bias;
        let n = settings.samples.max(1);
        // Stratified over a k×k grid of the hemisphere's square, the rest at random.
        let k = (n as f32).sqrt() as u32;
        let mut sum = Vec3::ZERO;
        for s in 0..n {
            let (a, b) = if s < k * k {
                ((((s % k) as f32) + rng.next()) / k as f32, (((s / k) as f32) + rng.next()) / k as f32)
            } else {
                (rng.next(), rng.next())
            };
            let dir = cosine_sample(t.normal, a, b);
            if dir.dot(t.geo) <= 0.0 {
                continue;
            }
            sum += scene.incoming(origin, dir, 0, settings.bounces, &mut rng);
        }
        sum / n as f32 + scene.direct(t.position, t.normal, t.geo, true)
    };

    #[cfg(not(target_arch = "wasm32"))]
    {
        let threads = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get).min(texels.len().max(1));
        if threads > 1 {
            let mut out = vec![Vec3::ZERO; texels.len()];
            let chunk = texels.len().div_ceil(threads * 8).max(1);
            let next = std::sync::atomic::AtomicUsize::new(0);
            let slots: Vec<std::sync::Mutex<&mut [Vec3]>> = out.chunks_mut(chunk).map(std::sync::Mutex::new).collect();
            std::thread::scope(|s| {
                for _ in 0..threads {
                    s.spawn(|| loop {
                        let c = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(slot) = slots.get(c) else { break };
                        let mut slot = slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                        let start = c * chunk;
                        for (o, t) in slot.iter_mut().zip(&texels[start..]) {
                            *o = shade(t);
                        }
                    });
                }
            });
            drop(slots);
            return out;
        }
    }
    texels.iter().map(shade).collect()
}

/// Fills `rings` texels around every covered region with the mean of its covered neighbours.
fn dilate(map: &mut Lightmap, covered: &mut [bool], rings: u32) {
    let (w, h) = (map.width as i64, map.height as i64);
    for _ in 0..rings {
        let mut fills = Vec::new();
        for y in 0..h {
            for x in 0..w {
                if covered[(y * w + x) as usize] {
                    continue;
                }
                let (mut sum, mut count) = (Vec3::ZERO, 0);
                for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)] {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx >= 0 && ny >= 0 && nx < w && ny < h && covered[(ny * w + nx) as usize] {
                        sum += map.texels[(ny * w + nx) as usize];
                        count += 1;
                    }
                }
                if count > 0 {
                    fills.push(((y * w + x) as usize, sum / count as f32));
                }
            }
        }
        if fills.is_empty() {
            break;
        }
        for (at, value) in fills {
            map.texels[at] = value;
            covered[at] = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A square in the XZ plane at height `y`, `half` metres each way, facing up (or down), with
    /// both UV sets running 0..1 across it.
    fn square(y: f32, half: f32, up: bool) -> Vec<Vertex> {
        let corner = |u: f32, v: f32| Vertex {
            position: [-half + u * half * 2.0, y, -half + v * half * 2.0],
            normal: if up { [0.0, 1.0, 0.0] } else { [0.0, -1.0, 0.0] },
            tex_coords: [u, v],
            lightmap_uv: [u, v],
            ..Default::default()
        };
        let (a, b, c, d) = (corner(0.0, 0.0), corner(0.0, 1.0), corner(1.0, 1.0), corner(1.0, 0.0));
        if up { vec![a, b, c, a, c, d] } else { vec![a, c, b, a, d, c] }
    }

    /// A closed axis-aligned box from `lo` to `hi`, faces outward.
    fn cuboid(lo: Vec3, hi: Vec3) -> Vec<Vertex> {
        let p = |x: f32, y: f32, z: f32| Vec3::new(if x > 0.0 { hi.x } else { lo.x }, if y > 0.0 { hi.y } else { lo.y }, if z > 0.0 { hi.z } else { lo.z });
        let mut out = Vec::new();
        let mut quad = |a: Vec3, b: Vec3, c: Vec3, d: Vec3| {
            let n = (b - a).cross(c - a).normalize();
            for q in [a, b, c, a, c, d] {
                out.push(Vertex { position: q.to_array(), normal: n.to_array(), ..Default::default() });
            }
        };
        let (s, e) = (-1.0, 1.0);
        quad(p(s, e, s), p(s, e, e), p(e, e, e), p(e, e, s)); // top
        quad(p(s, s, s), p(e, s, s), p(e, s, e), p(s, s, e)); // bottom
        quad(p(s, s, e), p(e, s, e), p(e, e, e), p(s, e, e)); // +z
        quad(p(s, s, s), p(s, e, s), p(e, e, s), p(e, s, s)); // -z
        quad(p(e, s, s), p(e, e, s), p(e, e, e), p(e, s, e)); // +x
        quad(p(s, s, s), p(s, s, e), p(s, e, e), p(s, e, s)); // -x
        out
    }

    fn mean(map: &Lightmap) -> Vec3 {
        map.texels().iter().copied().sum::<Vec3>() / map.texels().len() as f32
    }

    /// Under a uniform sky and nothing else, an open floor's lightmap is the sky's colour: what the
    /// shaders drew there before it had one. The unit the whole bake is in.
    #[test]
    fn an_open_floor_under_a_uniform_sky_bakes_the_sky_colour() {
        let mut bake = LightmapBake::new().with_sky(Sky::uniform([0.4, 0.2, 0.1])).with_settings(BakeSettings::default().with_samples(64));
        let floor = bake.add_mesh(BakeMesh::new(square(0.0, 5.0, true), Mat4::IDENTITY).receiving(8, 8));
        let baked = bake.run().unwrap();
        let m = mean(baked.get(floor).unwrap());
        assert!((m - Vec3::new(0.4, 0.2, 0.1)).abs().max_element() < 1e-4, "{m}");
    }

    /// A box on the floor shades the floor under the sky beside it and blacks out under it; the
    /// open floor far off keeps nearly all of the sky.
    #[test]
    fn a_box_on_the_floor_darkens_the_floor_around_it() {
        let mut bake = LightmapBake::new().with_sky(Sky::uniform([1.0, 1.0, 1.0])).with_settings(BakeSettings::default().with_samples(256).with_bounces(0));
        let floor = bake.add_mesh(BakeMesh::new(square(0.0, 4.0, true), Mat4::IDENTITY).receiving(32, 32));
        bake.add_mesh(BakeMesh::new(cuboid(Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 2.0, 1.0)), Mat4::IDENTITY));
        let baked = bake.run().unwrap();
        let map = baked.get(floor).unwrap();
        let under = map.texel(16, 16).x;
        let beside = map.texel(16, 21).x; // 0.6 m off the box's side
        let corner = map.texel(0, 0).x;
        eprintln!("under {under} beside {beside} corner {corner}");
        assert!(under < 0.01, "the floor under the box saw the sky: {under}");
        assert!(beside > 0.2 && beside < 0.8, "beside the box: {beside}");
        assert!(corner > 0.85, "the open corner lost its sky: {corner}");
        assert!(beside < corner);
    }

    /// A light the scene has at run time adds nothing directly; a baked-only one adds what the
    /// shaders would: `cos × intensity × falloff / π`, and a box between them shadows it.
    #[test]
    fn a_baked_only_light_bakes_its_direct_light_and_shadow() {
        let settings = BakeSettings::default().with_samples(4).with_bounces(0);
        let lit = |baked_only: bool, blocked: bool| {
            let mut bake = LightmapBake::new().with_settings(settings);
            let floor = bake.add_mesh(BakeMesh::new(square(0.0, 1.0, true), Mat4::IDENTITY).receiving(4, 4));
            if blocked {
                bake.add_mesh(BakeMesh::new(cuboid(Vec3::new(-2.0, 1.0, -2.0), Vec3::new(2.0, 1.2, 2.0)), Mat4::IDENTITY));
            }
            let light = BakeLight::point(Vec3::new(0.0, 2.0, 0.0), Vec3::ONE, 10.0, 20.0);
            bake.add_light(if baked_only { light.baked_only() } else { light });
            bake.run().unwrap().get(floor).unwrap().clone()
        };
        let runtime = lit(false, false);
        assert_eq!(mean(&runtime), Vec3::ZERO, "a run-time light's direct light was baked");
        let baked = lit(true, false);
        // Texel (1, 1)'s centre is at (-0.25, 0, -0.25) on the floor.
        let p = Vec3::new(-0.25, 0.0, -0.25);
        let d = Vec3::new(0.0, 2.0, 0.0) - p;
        let expect = 10.0 * falloff(d.length(), 20.0) * (d.y / d.length()) * std::f32::consts::FRAC_1_PI;
        let got = baked.texel(1, 1).x;
        assert!((got - expect).abs() < 1e-4 * expect.max(1.0), "{got} against {expect}");
        assert_eq!(mean(&lit(true, true)), Vec3::ZERO, "the light shone through the box over it");
    }

    /// An emissive panel over the floor lights it with no light at all — an area light — and the
    /// floor's light falls off away from under it.
    #[test]
    fn an_emissive_panel_lights_the_floor() {
        let mut bake = LightmapBake::new().with_settings(BakeSettings::default().with_samples(256).with_bounces(0));
        let floor = bake.add_mesh(BakeMesh::new(square(0.0, 6.0, true), Mat4::IDENTITY).receiving(24, 24));
        bake.add_mesh(BakeMesh::new(square(2.0, 1.0, false), Mat4::IDENTITY).with_emissive(Vec3::splat(4.0)));
        let map = bake.run().unwrap().get(floor).unwrap().clone();
        let centre = map.texel(12, 12).x;
        let edge = map.texel(1, 12).x;
        eprintln!("centre {centre} edge {edge}");
        assert!(centre > 0.4, "{centre}");
        assert!(edge < centre * 0.25, "{edge} against {centre}");
    }

    /// Light bounces: a white wall beside the floor, lit by a sun the floor cannot see, brightens
    /// the floor; with no bounces it does not.
    #[test]
    fn a_lit_wall_bounces_light_onto_the_floor() {
        let bake_with = |bounces: u32| {
            let mut bake = LightmapBake::new().with_settings(BakeSettings::default().with_samples(128).with_bounces(bounces));
            let floor = bake.add_mesh(BakeMesh::new(square(0.0, 2.0, true), Mat4::IDENTITY).receiving(8, 8));
            // A roof keeps the sun off the floor; the wall stands past the roof's edge, in the sun.
            bake.add_mesh(BakeMesh::new(cuboid(Vec3::new(-3.0, 3.0, -3.0), Vec3::new(1.5, 3.2, 3.0)), Mat4::IDENTITY));
            bake.add_mesh(BakeMesh::new(cuboid(Vec3::new(2.5, 0.0, -3.0), Vec3::new(2.7, 6.0, 3.0)), Mat4::IDENTITY).with_albedo(Vec3::ONE));
            // Low from −X: under the roof's edge, onto the wall's west face, not onto the floor.
            bake.add_light(BakeLight::directional(Vec3::new(1.0, -0.6, 0.0), Vec3::ONE, 5.0));
            mean(bake.run().unwrap().get(floor).unwrap())
        };
        let none = bake_with(0);
        let one = bake_with(1);
        eprintln!("no bounce {none} one bounce {one}");
        assert_eq!(none, Vec3::ZERO);
        assert!(one.x > 0.05, "{one}");
    }

    /// A drawn cube, taken by the triangles it keeps on the CPU, blocks the sky as the hand-built box
    /// does.
    #[test]
    fn a_mesh_from_positions_blocks_light() {
        let positions: Vec<Vec3> = cuboid(Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 2.0, 1.0)).iter().map(|v| Vec3::from(v.position)).collect();
        let mut bake = LightmapBake::new().with_sky(Sky::uniform([1.0, 1.0, 1.0])).with_settings(BakeSettings::default().with_samples(16));
        let floor = bake.add_mesh(BakeMesh::new(square(0.0, 4.0, true), Mat4::IDENTITY).receiving(32, 32));
        bake.add_mesh(BakeMesh::from_positions(&positions, Mat4::IDENTITY));
        let map = bake.run().unwrap().get(floor).unwrap().clone();
        assert!(map.texel(16, 16).x < 0.01, "{}", map.texel(16, 16));
        assert!(map.texel(0, 0).x > 0.8, "{}", map.texel(0, 0));
    }

    /// The same scene and seed bake the same lightmap, bit for bit, whatever the threads did.
    #[test]
    fn a_bake_is_deterministic() {
        let run = || {
            let mut bake = LightmapBake::new().with_sky(Sky::gradient([0.2, 0.3, 0.9], [0.8, 0.8, 0.7], [0.1, 0.1, 0.1])).with_settings(BakeSettings::default().with_samples(32));
            let floor = bake.add_mesh(BakeMesh::new(square(0.0, 3.0, true), Mat4::IDENTITY).receiving(32, 32));
            bake.add_mesh(BakeMesh::new(cuboid(Vec3::splat(-0.5), Vec3::splat(0.5)), Mat4::from_translation(Vec3::Y)));
            bake.add_light(BakeLight::directional(Vec3::new(0.3, -1.0, 0.2), Vec3::ONE, 2.0));
            bake.run().unwrap().get(floor).unwrap().clone()
        };
        let (a, b) = (run(), run());
        assert!(a.texels().iter().zip(b.texels()).all(|(x, y)| x.to_array().map(f32::to_bits) == y.to_array().map(f32::to_bits)));
    }

    /// A receiver whose second UV set was never filled is an error that says so, not a black map.
    #[test]
    fn a_receiver_without_lightmap_uvs_is_an_error() {
        let mut flat = square(0.0, 1.0, true);
        for v in &mut flat {
            v.lightmap_uv = [0.0, 0.0];
        }
        let mut bake = LightmapBake::new();
        let floor = bake.add_mesh(BakeMesh::new(flat, Mat4::IDENTITY).receiving(4, 4));
        assert_eq!(bake.run().unwrap_err(), LightmapError::NothingCovered { mesh: floor });
        let mut bake = LightmapBake::new();
        let floor = bake.add_mesh(BakeMesh::new(square(0.0, 1.0, true), Mat4::IDENTITY).receiving(0, 4));
        assert_eq!(bake.run().unwrap_err(), LightmapError::EmptyLightmap { mesh: floor });
    }

    /// Islands are padded: the texels around a triangle that covers part of the map take its light,
    /// those further out stay black.
    #[test]
    fn padding_fills_around_an_island() {
        let mut half = square(0.0, 1.0, true);
        for v in &mut half {
            v.lightmap_uv = [v.lightmap_uv[0] * 0.5, v.lightmap_uv[1]];
        }
        let mut bake = LightmapBake::new().with_sky(Sky::uniform([1.0, 1.0, 1.0])).with_settings(BakeSettings::default().with_samples(4).with_padding(2));
        let floor = bake.add_mesh(BakeMesh::new(half, Mat4::IDENTITY).receiving(16, 4));
        let map = bake.run().unwrap().get(floor).unwrap().clone();
        // The island ends at x = 8: texel 7's centre is on it, texel 8's half a texel off its edge,
        // sampled at the nearest point on it; 9 and 10 are the padding.
        for x in [7, 8] {
            assert!(map.texel(x, 1).x > 0.99, "texel {x}, traced");
        }
        for x in [9, 10] {
            assert!(map.texel(x, 1).x > 0.99, "texel {x}, padded");
        }
        assert_eq!(map.texel(11, 1).x, 0.0, "past the padding");
    }

    /// The 8-bit image's scale is the brightest channel, and scale × byte / 255 is the light back.
    #[test]
    fn the_rgba8_image_carries_its_scale() {
        let map = Lightmap { width: 2, height: 1, texels: vec![Vec3::new(3.0, 1.5, 0.0), Vec3::new(0.75, 0.0, 0.0)] };
        let (bytes, scale) = map.to_rgba8();
        assert_eq!(scale, 3.0);
        assert_eq!(bytes, vec![255, 128, 0, 255, 64, 0, 0, 255]);
        let black = Lightmap { width: 1, height: 1, texels: vec![Vec3::ZERO] };
        assert_eq!(black.to_rgba8(), (vec![0, 0, 0, 255], 1.0));
    }
}
