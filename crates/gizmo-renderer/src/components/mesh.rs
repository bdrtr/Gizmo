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
        let cpu_vertices = Arc::new(vertices.iter().map(|v| Vec3::from(v.position)).collect());

        let mut lod_vbufs = Vec::new();
        let mut lod_vertex_counts = Vec::new();

        // 1. Un-indexed vertex array üzerinden index array oluştur (meshopt için gereklidir)
        #[cfg(not(target_arch = "wasm32"))]
        if vertex_count > 20000 {
            let (unique_count, indices) = meshopt::generate_vertex_remap(vertices, None);

            let mut unique_vertices = vec![crate::gpu_types::Vertex::default(); unique_count];
            for (i, &new_idx) in indices.iter().enumerate() {
                unique_vertices[new_idx as usize] = vertices[i];
            }

            let adapter = meshopt::VertexDataAdapter::new(
                bytemuck::cast_slice(&unique_vertices),
                std::mem::size_of::<crate::gpu_types::Vertex>(),
                0,
            )
            .unwrap();

            let target_count = (indices.len() as f32 * 0.5) as usize; // %50 decimation
            let lod1_indices = meshopt::simplify(
                &indices,
                &adapter,
                target_count,
                0.1, // %10 error tolerance
                meshopt::SimplifyOptions::empty(),
                None,
            );

            // Eğer başarıyla decimation yapıldıysa ve gerçekten vertex sayısı düştüyse GPU'ya at
            if !lod1_indices.is_empty() && lod1_indices.len() < indices.len() {
                // Flat vertex array'e geri döndür (Gizmo renderer flat bekliyor)
                let mut lod_flat = Vec::with_capacity(lod1_indices.len());
                for &idx in &lod1_indices {
                    lod_flat.push(unique_vertices[idx as usize]);
                }

                let lod_vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(&format!("LOD1 VBuf: {}", source)),
                    contents: bytemuck::cast_slice(&lod_flat),
                    usage: wgpu::BufferUsages::VERTEX,
                });

                lod_vbufs.push(Arc::new(lod_vbuf));
                lod_vertex_counts.push(lod_flat.len() as u32);
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
        let mut mesh = Mesh::new(
            device,
            Arc::new(vbuf),
            &unique_vertices,
            center_offset,
            source,
        );
        mesh.index_count = indices.len() as u32;
        mesh.index_format = index_format;
        mesh.ibuf = Some(Arc::new(ibuf));

        // `cpu_vertices` ORİJİNAL üçgen listesine geri alınıyor. Tekilleştirilmiş diziyi
        // orada bırakmak sessiz bir bozulma olurdu: alanın tüketicileri onu üçlü gruplar
        // hâlinde yürüyor (`demo/src/bin/wind_tunnel.rs:535` — `i, i+1, i+2`, adım 3), ve
        // tekilleştirilmiş bir dizide ardışık üçlüler artık üçgen DEĞİL. Bellek açısından
        // bu bir gerileme de değil: bu alan zaten tam listeyi tutuyordu.
        mesh.cpu_vertices = Arc::new(vertices.iter().map(|v| Vec3::from(v.position)).collect());
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

/// The ECS marker component saying an entity is a Mesh that can be drawn.
/// It carries no fields; it only brings the entity into the render system.
#[derive(Clone)]
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
}

/// What an object contributes to the shadow maps and to the picture.
///
/// The third state is not a curiosity: a low-poly stand-in that casts for an expensive mesh, or a
/// blocker that shapes a light without appearing, both need "cast but do not draw". It is the
/// prototype's `On / Off / Only`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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
        Self { lod_bias: 1.0, shadows: ShadowCasting::On }
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
