use gizmo_math::Vec3;
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use wgpu::util::DeviceExt;

/// Geometry on the GPU, plus the CPU-side facts the renderer needs about it: its bounds, a
/// triangle-list copy for picking, and any simplified LOD buffers.
///
/// Buffers are behind `Arc`s because one loaded mesh is shared by every entity that draws it —
/// cloning a `Mesh` clones the handles, not the vertices.
#[derive(Clone)]
pub struct Mesh {
    /// The vertex buffer, laid out as [`Vertex`](crate::gpu_types::Vertex).
    pub vbuf: Arc<wgpu::Buffer>,
    /// How many vertices `vbuf` holds. For an unindexed mesh this is also the draw count.
    pub vertex_count: u32,
    /// The index buffer, if there is one. `None` means the mesh is drawn as a flat triangle
    /// list (`draw`), `Some` means `draw_indexed`.
    ///
    /// It is optional because of **LOD**, not because of a migration: `lod_vbufs` holds
    /// flattened buffers, so while a LOD level is active the index buffer is NOT valid (its
    /// indices refer to the full-resolution vertex array). Batching drops the index in that
    /// case.
    pub ibuf: Option<Arc<wgpu::Buffer>>,
    /// The number of indices in `ibuf` — this, not `vertex_count`, is how many elements get
    /// drawn. Meaningless (0) while `ibuf` is `None`.
    pub index_count: u32,
    /// `ibuf`'s element width. Whatever format the buffer was WRITTEN in must be what
    /// `set_index_buffer` is given — binding a 16-bit buffer as 32-bit does not crash, it
    /// **draws the wrong triangles**. That is why it is carried rather than derived. Meaningless
    /// while `ibuf` is `None`.
    pub index_format: wgpu::IndexFormat,
    /// The offset used to move the geometry's centre of mass to the origin.
    /// It can be applied to the model matrix at render time.
    /// It does not affect the AABB bounds directly (those are computed from the raw vertex
    /// data).
    pub center_offset: Vec3,
    /// Where this mesh came from — a file path, or a name for generated geometry. Used for
    /// debug labels and for recognising an already-loaded asset.
    pub source: String,
    /// The model-space bounding box, from the raw vertex positions. Frustum culling and the
    /// editor's selection bounds both read it.
    pub bounds: gizmo_math::Aabb,
    /// The CPU-side copy of the geometry, **as a flat triangle list**: every consecutive triple
    /// is one triangle.
    ///
    /// It is NOT guaranteed to match `vbuf` index for index — on an indexed mesh (`ibuf` is
    /// `Some`) the vertex buffer has been deduplicated while this field stays a triangle list.
    /// That is the contract because consumers walk this in groups of three; a caller who wants
    /// the order matching the GPU buffer has to read `ibuf`.
    pub cpu_vertices: Arc<Vec<Vec3>>,
    /// Simplified vertex buffers, coarsest last, generated at load time for meshes above the
    /// decimation threshold. Flattened triangle lists — see [`Self::ibuf`] for why that matters.
    pub lod_vbufs: Vec<Arc<wgpu::Buffer>>,
    /// Vertex counts matching [`Self::lod_vbufs`].
    pub lod_vertex_counts: Vec<u32>,
    /// Whether the triangles close over a volume ([`triangles_close`]), worked out once from
    /// [`Self::cpu_vertices`] when the mesh is made.
    ///
    /// It is what the shadow pass culls by. A closed mesh is drawn into the shadow maps by its back
    /// faces, so its lit front can never shadow itself; a mesh that does not close has no back to
    /// cast from — a plane, a card, a sheet of cloth — and is drawn by both faces. Until 2026-11-11
    /// everything was drawn as if closed, and a plane facing the sun cast no shadow at all.
    pub closed: bool,
}

/// Whether a triangle list (every three positions one triangle) closes over a volume: every edge
/// is met by exactly as many triangles running it the other way. Corners are matched by position,
/// welded to a grid a millionth of the mesh's size, because a mesh's vertices are split wherever
/// a normal or a UV changes — a cube's corners three ways — and an edge is shared by positions, not
/// by indices. Degenerate triangles are left out. A list that is empty, or not a multiple of three,
/// does not close.
///
/// When in doubt it answers `false`: drawing both faces into a shadow map is always a set of
/// casters that blocks the right light, where drawing only the back faces of something open loses
/// the shadow of every face turned to the light.
pub fn triangles_close(positions: &[Vec3]) -> bool {
    if positions.is_empty() || !positions.len().is_multiple_of(3) {
        return false;
    }
    let (low, high) = positions
        .iter()
        .fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    let size = (high - low).max_element();
    if !size.is_finite() || size <= 0.0 {
        return false;
    }
    let cell = size * 1e-6;
    let mut ids: std::collections::HashMap<[i64; 3], u32> = std::collections::HashMap::new();
    let mut id = |p: Vec3| {
        let q = (p / cell).round();
        let key = [q.x as i64, q.y as i64, q.z as i64];
        let next = ids.len() as u32;
        *ids.entry(key).or_insert(next)
    };
    let mut edges: Vec<u64> = Vec::with_capacity(positions.len());
    for [p, q, r] in positions.as_chunks::<3>().0 {
        let [a, b, c] = [id(*p), id(*q), id(*r)];
        if a == b || b == c || c == a {
            continue;
        }
        for (from, to) in [(a, b), (b, c), (c, a)] {
            edges.push((u64::from(from) << 32) | u64::from(to));
        }
    }
    if edges.is_empty() {
        return false;
    }
    edges.sort_unstable();
    // Each directed edge, as many times as it runs that way, against the same count of its reverse.
    let count = |edge: u64| edges.partition_point(|&e| e <= edge) - edges.partition_point(|&e| e < edge);
    let mut i = 0;
    while i < edges.len() {
        let edge = edges[i];
        let run = count(edge);
        let reverse = edge.rotate_left(32);
        if count(reverse) != run {
            return false;
        }
        i += run;
    }
    true
}

/// A mesh draws a simplified LOD1 far away once it has more than this many triangle corners
/// (flat vertices, or index-list entries — the same count): about 6 700 triangles.
pub const LOD_MIN_CORNERS: usize = 20_000;

/// Half the triangles of `(unique_vertices, indices)`, simplified by meshopt within a 10 % error,
/// expanded back to a flat vertex buffer (the form the batcher draws a LOD in). `None` when the
/// simplifier could not remove anything.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn lod1_vertices(
    unique_vertices: &[crate::gpu_types::Vertex],
    indices: &[u32],
) -> Option<Vec<crate::gpu_types::Vertex>> {
    let adapter = meshopt::VertexDataAdapter::new(
        bytemuck::cast_slice(unique_vertices),
        std::mem::size_of::<crate::gpu_types::Vertex>(),
        0,
    )
    .ok()?;
    let lod1_indices =
        meshopt::simplify(indices, &adapter, indices.len() / 2, 0.1, meshopt::SimplifyOptions::empty(), None);
    if lod1_indices.is_empty() || lod1_indices.len() >= indices.len() {
        return None;
    }
    Some(lod1_indices.iter().map(|&i| unique_vertices[i as usize]).collect())
}

/// [`lod1_vertices`], uploaded.
#[cfg(not(target_arch = "wasm32"))]
fn build_lod1(
    device: &wgpu::Device,
    unique_vertices: &[crate::gpu_types::Vertex],
    indices: &[u32],
    source: &str,
) -> Option<(Arc<wgpu::Buffer>, u32)> {
    let lod_flat = lod1_vertices(unique_vertices, indices)?;
    let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(&format!("LOD1 VBuf: {source}")),
        contents: bytemuck::cast_slice(&lod_flat),
        usage: wgpu::BufferUsages::VERTEX,
    });
    Some((Arc::new(buf), lod_flat.len() as u32))
}

impl Mesh {
    /// Creates a new `Mesh` component.
    /// `vertex_count` and `bounds` are computed from the `vertices` array automatically.
    /// For an empty mesh on an error path, use `Mesh::empty()`.
    // WASM: meshopt LOD üretimi native-only cfg'li — `device` ve `mut`'lar orada
    // kullanılmadığından hedefli allow (native lint gücü korunur).
    #[cfg_attr(target_arch = "wasm32", allow(unused_variables, unused_mut))]
    pub fn new(
        device: &wgpu::Device,
        vbuf: Arc<wgpu::Buffer>,
        vertices: &[crate::gpu_types::Vertex],
        center_offset: Vec3,
        source: String,
    ) -> Self {
        Self::from_vertices_with_lod(device, vbuf, vertices, center_offset, source, true)
    }

    /// [`Mesh::new`], with the triangle-list LOD optional — `new_indexed` passes a *unique*
    /// vertex array, which is not a triangle list, and builds its LOD from its own indices.
    #[cfg_attr(target_arch = "wasm32", allow(unused_variables, unused_mut))]
    fn from_vertices_with_lod(
        device: &wgpu::Device,
        vbuf: Arc<wgpu::Buffer>,
        vertices: &[crate::gpu_types::Vertex],
        center_offset: Vec3,
        source: String,
        generate_lod: bool,
    ) -> Self {
        debug_assert!(
            !vertices.is_empty(),
            "Kullanım hatası: Normal kullanımlarda vertices boş olamaz. Boş (fallback) mesh için Mesh::empty() kullanın."
        );
        let vertex_count = vertices.len() as u32;
        debug_assert_eq!(
            vertex_count as usize * std::mem::size_of::<crate::gpu_types::Vertex>(),
            vbuf.size() as usize
        );
        let bounds = gizmo_math::Aabb::from_points(vertices.iter().map(|v| v.position));
        let cpu_vertices: Arc<Vec<Vec3>> = Arc::new(vertices.iter().map(|v| Vec3::from(v.position)).collect());
        // Only a triangle list has edges to match; `new_indexed` hands in its unique vertices with
        // the LOD off and works this out from its own triangle list afterwards.
        let closed = generate_lod && triangles_close(&cpu_vertices);

        let mut lod_vbufs = Vec::new();
        let mut lod_vertex_counts = Vec::new();

        // A flat triangle list: every three vertices are one triangle, so remapping recovers the
        // shared corners and the simplifier sees real topology. `new_indexed` does NOT come
        // through here with LOD on — its vertex array is already unique, not a triangle list.
        #[cfg(not(target_arch = "wasm32"))]
        if generate_lod && vertex_count as usize > LOD_MIN_CORNERS {
            let (unique_count, indices) = meshopt::generate_vertex_remap(vertices, None);
            let mut unique_vertices = vec![crate::gpu_types::Vertex::default(); unique_count];
            for (i, &new_idx) in indices.iter().enumerate() {
                unique_vertices[new_idx as usize] = vertices[i];
            }
            if let Some((buf, count)) = build_lod1(device, &unique_vertices, &indices, &source) {
                lod_vbufs.push(buf);
                lod_vertex_counts.push(count);
            }
        }

        Self {
            vbuf,
            vertex_count,
            ibuf: None,
            index_count: 0,
            index_format: wgpu::IndexFormat::Uint32,
            center_offset,
            source,
            bounds,
            cpu_vertices,
            lod_vbufs,
            lod_vertex_counts,
            closed,
        }
    }

    /// Like [`Mesh::new`], but it **deduplicates** the vertices and builds an index buffer as
    /// well, and creates the vertex buffer itself (the caller does not hand in a ready `vbuf`).
    ///
    /// The engine's deduplication machinery already existed and its output was being thrown
    /// away: above 20000 vertices [`Mesh::new`] calls `meshopt::generate_vertex_remap` to build
    /// LODs, produces an index array, and then expands `unique_vertices[idx]` back into a flat
    /// array ("the Gizmo renderer expects flat"). A cube's 36 vertices become 8, and a glTF
    /// scene's shared corners fall to as few as the number of adjacent triangles — both as
    /// upload bandwidth and as vertex-shader invocations.
    ///
    /// **Callable on every target.** `meshopt` is native-only, so there is no deduplication on
    /// WASM: the mesh stays flat and `ibuf` comes back `None`. That is NOT a burden on the
    /// caller — the draw path already carries both cases (`record_draw`), so the `cfg` branch
    /// happens once here rather than at every call site.
    pub fn new_indexed(
        device: &wgpu::Device,
        vertices: &[crate::gpu_types::Vertex],
        center_offset: Vec3,
        source: String,
    ) -> Self {
        #[cfg(target_arch = "wasm32")]
        {
            use wgpu::util::DeviceExt;
            let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(&format!("VBuf (flat, no meshopt): {source}")),
                contents: bytemuck::cast_slice(vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
            // Tail expression, not a `return`: on wasm the block below is stripped, so this
            // block IS the function body and a `return` here reads as needless — which only the
            // wasm lint can see, and only since CI started linting that target.
            Mesh::new(device, Arc::new(vbuf), vertices, center_offset, source)
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
        let (unique_count, indices) = meshopt::generate_vertex_remap(vertices, None);

        let mut unique_vertices = vec![crate::gpu_types::Vertex::default(); unique_count];
        for (i, &new_idx) in indices.iter().enumerate() {
            unique_vertices[new_idx as usize] = vertices[i];
        }

        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("VBuf (indexed): {source}")),
            contents: bytemuck::cast_slice(&unique_vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        // 16-bit indeksler, sığdığı sürece: indeks tamponunun bandını ve belleğini yarıya
        // indiriyor, ve motorun ürettiği her şey rahatça sığıyor (en büyük primitif torus,
        // 561 tekil vertex). `meshopt` `u32` üretiyor, daraltma burada yapılıyor.
        //
        // Eşik `<= 65536`: `u16`'nın taşıyabildiği en büyük indeks 65535, dolayısıyla o kadar
        // TEKİL VERTEX adreslenebilir. Sınırı `< 65536` yazmak son vertex'i boşuna kaybederdi;
        // `<= 65537` yazmak sessizce sarardı.
        let (index_format, index_bytes): (wgpu::IndexFormat, Vec<u8>) = if unique_count <= 65536 {
            let narrow: Vec<u16> = indices.iter().map(|&i| i as u16).collect();
            (
                wgpu::IndexFormat::Uint16,
                bytemuck::cast_slice(&narrow).to_vec(),
            )
        } else {
            (
                wgpu::IndexFormat::Uint32,
                bytemuck::cast_slice(&indices).to_vec(),
            )
        };
        let ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("IBuf: {source}")),
            // `create_buffer_init` boyutu `COPY_BUFFER_ALIGNMENT`'a kendisi yuvarlıyor, yani
            // tek sayıda `u16` indeks (2 bayt hizası, 4 değil) sorun değil.
            contents: &index_bytes,
            usage: wgpu::BufferUsages::INDEX,
        });

        // `Mesh::new`'e TEKİLLEŞTİRİLMİŞ diziyi veriyoruz — `vertex_count` ile `vbuf.size()`
        // arasındaki debug_assert ancak böyle tutar, ve `bounds` iki durumda da aynı nokta
        // kümesinden çıkıyor.
        // No triangle-list LOD from `Mesh::new`: `unique_vertices` is not a triangle list, and
        // feeding it to that path is how a large indexed mesh (every glTF and OBJ import) used to
        // get a LOD1 of nonsense — each three consecutive unique vertices taken as a triangle.
        let mut mesh = Mesh::from_vertices_with_lod(
            device,
            Arc::new(vbuf),
            &unique_vertices,
            center_offset,
            source,
            false,
        );
        // The LOD from the real topology, under the same gate as the flat path: it counts
        // corners (index-list entries), which is what a flat mesh's vertex count also is.
        if indices.len() > LOD_MIN_CORNERS {
            if let Some((buf, count)) = build_lod1(device, &unique_vertices, &indices, &mesh.source) {
                mesh.lod_vbufs.push(buf);
                mesh.lod_vertex_counts.push(count);
            }
        }
        mesh.index_count = indices.len() as u32;
        mesh.index_format = index_format;
        mesh.ibuf = Some(Arc::new(ibuf));

        // `cpu_vertices` ORİJİNAL üçgen listesine geri alınıyor. Tekilleştirilmiş diziyi
        // orada bırakmak sessiz bir bozulma olurdu: alanın tüketicileri onu üçlü gruplar
        // hâlinde yürüyor (`demo/src/bin/wind_tunnel.rs:535` — `i, i+1, i+2`, adım 3), ve
        // tekilleştirilmiş bir dizide ardışık üçlüler artık üçgen DEĞİL. Bellek açısından
        // bu bir gerileme de değil: bu alan zaten tam listeyi tutuyordu.
        mesh.cpu_vertices = Arc::new(vertices.iter().map(|v| Vec3::from(v.position)).collect());
        mesh.closed = triangles_close(&mesh.cpu_vertices);
        mesh
        }
    }

    /// Creates an empty placeholder Mesh with 0 vertices, so that the engine does not crash
    /// when a file fails to load.
    pub fn empty(vbuf: Arc<wgpu::Buffer>, source: String) -> Self {
        Self {
            vbuf,
            vertex_count: 0,
            ibuf: None,
            index_count: 0,
            index_format: wgpu::IndexFormat::Uint32,
            center_offset: Vec3::ZERO,
            source,
            bounds: gizmo_math::Aabb::empty(),
            cpu_vertices: Arc::new(Vec::new()),
            lod_vbufs: Vec::new(),
            lod_vertex_counts: Vec::new(),
            closed: false,
        }
    }

    /// Builds a mesh from arbitrary (triangle-list) vertex data — it creates the vertex buffer
    /// and hands it to [`Mesh::new`]. A shortcut for procedural geometry (streamline ribbons,
    /// debug drawing), so the caller does not have to deal with wgpu buffer details.
    pub fn from_vertices(
        device: &wgpu::Device,
        vertices: &[crate::gpu_types::Vertex],
        source: impl Into<String>,
    ) -> Self {
        use wgpu::util::DeviceExt;
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ProcMesh VBuf"),
            contents: bytemuck::cast_slice(vertices),
            // COPY_DST → içerik her frame güncellenebilir (bkz `update_vertices`).
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        Mesh::new(device, Arc::new(vbuf), vertices, Vec3::ZERO, source.into())
    }

    /// Updates the CONTENTS of the vertex buffer in place (same vertex count;
    /// position/colour/uv may change). For procedural or animated geometry — it can be written
    /// every frame without rebuilding the mesh. The buffer must have been created with
    /// `COPY_DST` (`from_vertices` does) and the vertex count must NOT change. Note:
    /// `bounds`/`vertex_count` are not updated (the position range is expected to stay the same)
    /// and above 20000 vertices the LOD buffers are not updated.
    pub fn update_vertices(&self, queue: &wgpu::Queue, vertices: &[crate::gpu_types::Vertex]) {
        let bytes: &[u8] = bytemuck::cast_slice(vertices);
        debug_assert!(bytes.len() as u64 <= self.vbuf.size());
        queue.write_buffer(&self.vbuf, 0, bytes);
    }
}

/// The ECS component saying an entity is a Mesh that can be drawn, and how it is drawn.
///
/// The doc above this said "it carries no fields" for as long as it had them, and the stale
/// sentence had a cost: the two fields below are edited in the inspector's "Mesh Renderer"
/// section, and nothing registered the component for save/load, so every artist decision here
/// was lost on the next reopen — silently, because an unregistered component is simply not
/// written. That is why this derives `Serialize`; `full_scene_registry` is where it is claimed.
///
/// `#[serde(default)]` because the defaults are the pre-existing behaviour rather than zero:
/// a scene written before a field existed loads as "no LOD bias, casts shadows", not as a
/// `lod_bias` of 0.0.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct MeshRenderer {
    /// Scales the distance used to pick a level of detail.
    ///
    /// Above 1.0 holds the higher-detail mesh further out, below 1.0 switches down sooner — the
    /// convention every engine with this knob uses, because the number reads as "how much quality
    /// this object is worth". Applied as a division: [`effective_lod_distance`].
    ///
    /// Per-object because that is the only place it means anything: a global multiplier is a
    /// quality setting, and the thing an artist wants is "this hero prop stays sharp, that fence
    /// does not".
    pub lod_bias: f32,
    /// Whether this object casts a shadow, and whether it is drawn at all.
    ///
    /// Per-object rather than per-material, which is where the engine used to decide it: shadow
    /// casting fell out of the material's routing (unlit/skybox/grid were excluded and everything
    /// else cast), so two crates sharing one material could not differ. A light fixture's glowing
    /// panel and the wall behind it often want opposite answers.
    pub shadows: ShadowCasting,
    /// Whether shadows fall on this object — the receiving half that [`shadows`](Self::shadows)
    /// does not cover. `false` keeps it lit wherever its light reaches: a lamp's own shade, a
    /// character that must stay readable, a HUD-like object placed in the world. The sun's
    /// cascades, the point-light cube and contact shadows all skip it. `true` by default.
    pub receives_shadows: bool,
}

/// What an object contributes to the shadow maps and to the picture.
///
/// The third state is not a curiosity: a low-poly stand-in that casts for an expensive mesh, or a
/// blocker that shapes a light without appearing, both need "cast but do not draw". It is the
/// prototype's `On / Off / Only`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ShadowCasting {
    /// Drawn, and casts. The default, and what every object did before this existed.
    #[default]
    On,
    /// Drawn, casts nothing.
    Off,
    /// Casts, but is not drawn.
    Only,
}

impl ShadowCasting {
    /// Does this object go into the shadow maps?
    pub fn casts(self) -> bool {
        matches!(self, Self::On | Self::Only)
    }

    /// Does this object go into the camera's picture?
    pub fn visible(self) -> bool {
        matches!(self, Self::On | Self::Off)
    }
}

impl MeshRenderer {
    /// The default render settings: no LOD bias, casting and receiving shadows.
    pub fn new() -> Self {
        Self { lod_bias: 1.0, shadows: ShadowCasting::On, receives_shadows: true }
    }

    /// Builder form: `MeshRenderer::new().with_lod_bias(2.0)`.
    pub fn with_lod_bias(mut self, bias: f32) -> Self {
        self.lod_bias = bias;
        self
    }

    /// Builder form: `MeshRenderer::new().with_shadows(ShadowCasting::Only)`.
    pub fn with_shadows(mut self, shadows: ShadowCasting) -> Self {
        self.shadows = shadows;
        self
    }

    /// Builder form: `MeshRenderer::new().with_receives_shadows(false)` — see
    /// [`receives_shadows`](Self::receives_shadows).
    pub fn with_receives_shadows(mut self, receives: bool) -> Self {
        self.receives_shadows = receives;
        self
    }
}


/// The distance a LOD decision should be made against, given an object's bias.
///
/// `bias > 1` divides the distance down, so the object behaves as if it were nearer and keeps its
/// higher-detail mesh further out; `bias < 1` does the reverse. One function rather than the
/// division written at each call site, because the two render paths pick LODs by completely
/// different mechanisms (the editor by `LodGroup`, the engine by a mesh's flattened `lod_vbufs`)
/// and the only thing they must agree on is what the number means.
///
/// A non-finite or non-positive bias is ignored rather than obeyed: zero would divide by zero and
/// a negative would invert the whole scale, and neither is a thing a user meant to ask for.
pub fn effective_lod_distance(distance: f32, bias: f32) -> f32 {
    if bias.is_finite() && bias > 0.0 {
        distance / bias
    } else {
        distance
    }
}

impl Default for MeshRenderer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod lod_bias_tests {
    use super::effective_lod_distance;

    /// The direction of the knob, which is the only thing a user can get wrong about it.
    #[test]
    fn a_higher_bias_holds_detail_further_out() {
        // Twice the bias means the object is treated as half as far away, so a LOD boundary at
        // 50 m is not crossed until 100 m.
        assert_eq!(effective_lod_distance(100.0, 2.0), 50.0);
        assert_eq!(effective_lod_distance(100.0, 0.5), 200.0);
        assert_eq!(effective_lod_distance(100.0, 1.0), 100.0, "1.0 is a no-op");
    }

    /// Values that cannot mean anything are ignored rather than obeyed.
    ///
    /// Zero would divide by zero — `inf` distance, everything culled to the coarsest level or
    /// dropped entirely — and a negative would invert the scale so that moving away increased
    /// detail. Both are typos, not requests.
    #[test]
    fn a_bias_that_cannot_mean_anything_is_ignored() {
        assert_eq!(effective_lod_distance(100.0, 0.0), 100.0);
        assert_eq!(effective_lod_distance(100.0, -2.0), 100.0);
        assert_eq!(effective_lod_distance(100.0, f32::NAN), 100.0);
        assert_eq!(effective_lod_distance(100.0, f32::INFINITY), 100.0);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod lod_tests {
    use super::*;
    use crate::gpu_types::Vertex;

    /// A UV sphere of radius 1 as a flat triangle list, its corners shared the way an importer's
    /// are — and remapped as `new_indexed` remaps it.
    fn sphere(rings: usize, segments: usize) -> (Vec<Vertex>, Vec<u32>) {
        let point = |r: usize, s: usize| {
            let theta = std::f32::consts::PI * r as f32 / rings as f32;
            let phi = std::f32::consts::TAU * (s % segments) as f32 / segments as f32;
            Vertex { position: [theta.sin() * phi.cos(), theta.cos(), theta.sin() * phi.sin()], ..Default::default() }
        };
        let mut flat = Vec::new();
        for r in 0..rings {
            for s in 0..segments {
                let (a, b, c, d) = (point(r, s), point(r + 1, s), point(r + 1, s + 1), point(r, s + 1));
                flat.extend([a, b, c, a, c, d]);
            }
        }
        let (unique_count, indices) = meshopt::generate_vertex_remap(&flat, None);
        let mut unique = vec![Vertex::default(); unique_count];
        for (i, &u) in indices.iter().enumerate() {
            unique[u as usize] = flat[i];
        }
        (unique, indices)
    }

    fn area(flat: &[Vertex]) -> f32 {
        flat.as_chunks::<3>()
            .0
            .iter()
            .map(|t| {
                let [a, b, c] = [t[0].position, t[1].position, t[2].position].map(Vec3::from);
                (b - a).cross(c - a).length() * 0.5
            })
            .sum()
    }

    /// The engine's own indexed sphere gets a LOD now: 20 736 corners is past the gate. It used
    /// to get none — the gate counted its 3 575 unique vertices, a number that measures sharing,
    /// not detail — while the same sphere built flat got one.
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn an_indexed_sphere_past_the_gate_gets_a_lod() {
        let _gpu = crate::test_gpu::gpu_lock();
        let Some((device, _queue)) = pollster::block_on(crate::test_gpu::headless_device()) else {
            eprintln!("skipping: no usable GPU adapter");
            return;
        };
        let sphere = crate::asset::AssetManager::create_sphere(&device, 1.0, 48, 72);
        assert!(sphere.ibuf.is_some(), "the primitive is indexed");
        assert!(sphere.index_count as usize > LOD_MIN_CORNERS, "{} corners", sphere.index_count);
        assert_eq!(sphere.lod_vbufs.len(), 1, "an indexed mesh past the gate has a LOD1");
        assert!(
            sphere.lod_vertex_counts[0] <= sphere.index_count / 2 + 3 && sphere.lod_vertex_counts[0].is_multiple_of(3),
            "about half the corners, whole triangles: {} of {}",
            sphere.lod_vertex_counts[0],
            sphere.index_count
        );
        let small = crate::asset::AssetManager::create_sphere(&device, 1.0, 16, 24);
        assert!(small.lod_vbufs.is_empty(), "under the gate, none");
    }

    /// LOD1 from the real topology keeps the surface — its triangles still cover a sphere's area
    /// (4π) — while halving the corners. Taking the unique vertex array as a triangle list, which
    /// is what an indexed mesh's LOD used to be built from, covers almost nothing.
    #[test]
    fn lod1_from_the_indices_keeps_the_surface() {
        let (unique, indices) = sphere(48, 72);
        assert!(indices.len() > LOD_MIN_CORNERS, "a mesh this size gets a LOD");
        let lod = lod1_vertices(&unique, &indices).expect("the simplifier removes something");
        assert!(lod.len() <= indices.len() / 2 + 3, "about half the corners: {} of {}", lod.len(), indices.len());
        let full = std::f32::consts::PI * 4.0;
        let kept = area(&lod);
        assert!((kept - full).abs() < 0.15 * full, "LOD1 covers the sphere: area {kept:.2} against {full:.2}");
        let nonsense = area(&unique[..unique.len() / 3 * 3]);
        assert!(nonsense < 0.2 * full, "the old reading of the unique array covers next to nothing: {nonsense:.2}");
    }
}

#[cfg(test)]
mod closed_tests {
    use super::triangles_close;
    use gizmo_math::Vec3;

    fn positions(vertices: &[crate::gpu_types::Vertex]) -> Vec<Vec3> {
        vertices.iter().map(|v| Vec3::from(v.position)).collect()
    }

    /// A unit cube as a triangle list, each face its own four corners as a mesh with normals has
    /// them, wound outwards; `skip` leaves one face out.
    fn cube(skip: Option<usize>) -> Vec<Vec3> {
        let p = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
        let faces = [
            [p(1., -1., -1.), p(-1., -1., -1.), p(-1., 1., -1.), p(1., 1., -1.)],
            [p(-1., -1., 1.), p(1., -1., 1.), p(1., 1., 1.), p(-1., 1., 1.)],
            [p(-1., -1., -1.), p(1., -1., -1.), p(1., -1., 1.), p(-1., -1., 1.)],
            [p(-1., 1., 1.), p(1., 1., 1.), p(1., 1., -1.), p(-1., 1., -1.)],
            [p(-1., -1., -1.), p(-1., -1., 1.), p(-1., 1., 1.), p(-1., 1., -1.)],
            [p(1., -1., 1.), p(1., -1., -1.), p(1., 1., -1.), p(1., 1., 1.)],
        ];
        faces
            .iter()
            .enumerate()
            .filter(|(i, _)| Some(*i) != skip)
            .flat_map(|(_, [a, b, c, d])| [*a, *b, *c, *a, *c, *d])
            .collect()
    }

    /// The engine's solids close and its flat shapes do not; a cube closes with its corners split
    /// per face, and stops closing with a face gone.
    #[test]
    fn solids_close_and_sheets_do_not() {
        use crate::asset::AssetManager;
        assert!(triangles_close(&cube(None)), "a cube");
        for face in 0..6 {
            assert!(!triangles_close(&cube(Some(face))), "a cube without face {face}");
        }
        assert!(triangles_close(&positions(&AssetManager::sphere_data(1.0, 16, 24))), "a sphere");
        for latitudes in [4, 7, 8] {
            assert!(triangles_close(&positions(&AssetManager::capsule_data(0.5, 1.0, latitudes, 16))), "a capsule of {latitudes}");
        }
        assert!(triangles_close(&positions(&AssetManager::cylinder_data(0.5, 1.0, 12))), "a cylinder");
        assert!(triangles_close(&positions(&AssetManager::cone_data(0.5, 1.0, 12))), "a cone");
        assert!(!triangles_close(&positions(&AssetManager::plane_data(2.0))), "a plane");
        assert!(!triangles_close(&positions(&AssetManager::plane_subdivided_data(2.0, 4))), "a subdivided plane");
        assert!(!triangles_close(&positions(&AssetManager::circle_data(1.0, 12))), "a disc");
    }

    /// A sheet drawn as two layers wound opposite ways closes — and casting it from its back faces
    /// is right, since the layer turned from the light is the one drawn. A sheet with one triangle
    /// turned over does not, nor does anything empty, ragged or degenerate.
    #[test]
    fn two_layers_close_and_broken_lists_do_not() {
        let sheet: Vec<Vec3> = positions(&crate::asset::AssetManager::plane_data(2.0));
        let mut both = sheet.clone();
        both.extend(sheet.chunks(3).flat_map(|t| [t[0], t[2], t[1]]));
        assert!(triangles_close(&both), "a sheet and its reverse");
        let mut flipped = cube(None);
        flipped.swap(1, 2);
        assert!(!triangles_close(&flipped), "a cube with one triangle wound the wrong way");
        assert!(!triangles_close(&[]), "nothing");
        assert!(!triangles_close(&cube(None)[..7]), "not a multiple of three");
        assert!(!triangles_close(&[Vec3::ZERO; 6]), "only degenerate triangles");
    }

    /// Every way a mesh is made answers: the engine's solids close through `new_indexed` (which hands
    /// its unique vertices to `Mesh::new` and works the answer out from its own triangle list) and
    /// through the flat `Mesh::new`, and its sheets do not.
    #[test]
    fn a_mesh_knows_whether_it_closes() {
        use crate::asset::AssetManager;
        let _gpu = crate::test_gpu::gpu_lock();
        let Some((device, _queue)) = pollster::block_on(crate::test_gpu::headless_device()) else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        assert!(AssetManager::create_cube(&device).closed, "a cube");
        assert!(AssetManager::create_sphere(&device, 1.0, 12, 16).closed, "a sphere");
        assert!(!AssetManager::create_plane(&device, 2.0).closed, "a plane");
        let flat = super::Mesh::from_vertices(&device, &AssetManager::sphere_data(1.0, 12, 16), "flat sphere");
        assert!(flat.closed, "a sphere made flat");
        let sheet = super::Mesh::from_vertices(&device, &AssetManager::plane_data(2.0), "flat plane");
        assert!(!sheet.closed, "a plane made flat");
    }

    /// Corners a float's rounding apart are one corner: a mesh whose shared corners came out of two
    /// computations a few ulps apart still closes.
    #[test]
    fn corners_a_rounding_apart_are_one() {
        let jittered: Vec<Vec3> = cube(None)
            .into_iter()
            .enumerate()
            .map(|(i, p)| p + Vec3::splat(if i % 2 == 0 { 1e-7 } else { -1e-7 }))
            .collect();
        assert!(triangles_close(&jittered));
    }
}
