use crate::components::Mesh;
use crate::renderer::Vertex;
use gizmo_math::Vec3;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;
use wgpu::util::DeviceExt;

pub mod error;
pub mod loaders;
pub mod primitives;
pub mod procedural;
pub mod texture;

pub use error::{AssetError, ObjIndexKind};
pub use loaders::GltfNodeData;

// ============================================================================
//  Asset metadata
// ============================================================================

/// Persisted alongside every asset file as `<filename>.meta`.
///
/// Stable UUIDs let editor tools and serialised scenes reference assets by
/// identity rather than by path, surviving renames and moves.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct AssetMeta {
    pub uuid: Uuid,
}

// ============================================================================
//  Free decode helpers (CPU-only, safe to call from worker threads)
// ============================================================================

/// Decode an image file to RGBA8 on a background thread (no GPU access).
pub fn decode_rgba_image_file(path: &str) -> Result<(Vec<u8>, u32, u32), AssetError> {
    let img = image::open(path)
        .map_err(|source| AssetError::ImageDecode {
            path: PathBuf::from(path),
            source,
        })?
        .to_rgba8();
    let (w, h) = img.dimensions();
    Ok((img.into_raw(), w, h))
}

/// Decode an OBJ file to a flat vertex buffer + AABB without touching the GPU.
///
/// Intended for use with [`crate::async_assets::AsyncAssetLoader`]: call this
/// on a worker thread, then hand the result to
/// [`AssetManager::install_obj_mesh`] on the main thread.
pub fn decode_obj_vertices_for_async(
    file_path: &str,
) -> Result<(Vec<Vertex>, gizmo_math::Aabb), AssetError> {
    let (models, _) = tobj::load_obj(
        file_path,
        &tobj::LoadOptions {
            single_index: true,
            triangulate: true,
            ignore_points: true,
            ignore_lines: true,
        },
    )
    .map_err(|source| AssetError::ObjLoad {
        path: PathBuf::from(file_path),
        source,
    })?;

    if models.is_empty() {
        return Err(AssetError::ObjEmpty {
            path: PathBuf::from(file_path),
        });
    }

    let mut aabb = gizmo_math::Aabb::empty();
    let mut vertices = Vec::new();

    for model in &models {
        let m = &model.mesh;
        let has_normals = !m.normals.is_empty();
        let has_texcoords = !m.texcoords.is_empty();
        let model_start = vertices.len(); // first vertex of this model

        for &raw_idx in &m.indices {
            let idx = raw_idx as usize;

            // ── Position ─────────────────────────────────────────────────
            let pos_base = idx * 3;
            if pos_base + 2 >= m.positions.len() {
                return Err(AssetError::ObjIndexOutOfRange {
                    path: PathBuf::from(file_path),
                    kind: ObjIndexKind::Position,
                    index: idx,
                    len: m.positions.len(),
                });
            }
            let position = [
                m.positions[pos_base],
                m.positions[pos_base + 1],
                m.positions[pos_base + 2],
            ];
            aabb.extend(Vec3::new(position[0], position[1], position[2]));

            // ── Normal (placeholder when absent; recalculated below) ──────
            let normal = if has_normals {
                let n_base = idx * 3;
                if n_base + 2 >= m.normals.len() {
                    return Err(AssetError::ObjIndexOutOfRange {
                        path: PathBuf::from(file_path),
                        kind: ObjIndexKind::Normal,
                        index: idx,
                        len: m.normals.len(),
                    });
                }
                [
                    m.normals[n_base],
                    m.normals[n_base + 1],
                    m.normals[n_base + 2],
                ]
            } else {
                [0.0, 1.0, 0.0] // temporary; flat normals computed below
            };

            // ── UV ────────────────────────────────────────────────────────
            let tex_coords = if has_texcoords {
                let uv_base = idx * 2;
                if uv_base + 1 >= m.texcoords.len() {
                    return Err(AssetError::ObjIndexOutOfRange {
                        path: PathBuf::from(file_path),
                        kind: ObjIndexKind::TexCoord,
                        index: idx,
                        len: m.texcoords.len(),
                    });
                }
                // OBJ UV origin is bottom-left; flip V to match GPU convention.
                [m.texcoords[uv_base], 1.0 - m.texcoords[uv_base + 1]]
            } else {
                [0.0, 0.0]
            };

            vertices.push(Vertex {
                position,
                normal,
                tex_coords,
                color: [1.0, 1.0, 1.0, 1.0],
                joint_indices: [0; 4],
                joint_weights: [0.0; 4],
                ..Default::default()
            });
        }

        // Compute flat normals per-model, only when the model lacks them.
        // This ensures models WITH normals are never touched.
        if !has_normals {
            let model_verts = &mut vertices[model_start..];
            let remainder = compute_flat_normals_inplace(model_verts);
            if remainder > 0 {
                tracing::error!(
                    "[AssetManager] WARN: '{file_path}' model '{}' has {remainder} \
                     trailing vertices that don't form a complete triangle — \
                     normals for those vertices left as Y-up.",
                    model.name
                );
            }
        }
    }

    Ok((vertices, aabb))
}

/// Compute flat (per-face) normals for a triangle-list vertex buffer in place.
///
/// Returns the number of leftover vertices that could not form a complete
/// triangle (should be 0 for well-formed meshes).
fn compute_flat_normals_inplace(vertices: &mut [Vertex]) -> usize {
    let chunks = vertices.chunks_exact_mut(3);
    let remainder_len = chunks.into_remainder().len(); // borrow ends here

    for tri in vertices.chunks_exact_mut(3) {
        let v0 = Vec3::from(tri[0].position);
        let v1 = Vec3::from(tri[1].position);
        let v2 = Vec3::from(tri[2].position);

        let cross = (v1 - v0).cross(v2 - v0);
        let normal = if cross.length_squared() > 1e-10 {
            cross.normalize()
        } else {
            Vec3::Y // degenerate triangle → default up
        };

        let n = [normal.x, normal.y, normal.z];
        tri[0].normal = n;
        tri[1].normal = n;
        tri[2].normal = n;
    }

    remainder_len
}

// ============================================================================
//  AssetManager
// ============================================================================

/// Shared 1×1 default textures + a neutral `MaterialParams` buffer used to fill
/// the auxiliary slots (normal / metallic-roughness / emissive / AO / params) of
/// a material bind group when the corresponding glTF map is absent.
///
/// The default values are chosen so the textured-PBR shader math reduces to the
/// scalar fallback with no branching:
/// * `flat_normal` = (0.5, 0.5, 1.0) → tangent-space (0,0,1) → unperturbed normal.
/// * `white` = (1,1,1,1) → neutral multiplier for MR / emissive / AO.
///
/// The auxiliary textures are stored as **linear** (`Rgba8Unorm`) — normal / MR /
/// AO data must NOT be gamma-decoded.
pub(crate) struct MaterialDefaults {
    // Keep the GPU textures alive for as long as any bind group references their views.
    _flat_normal_tex: wgpu::Texture,
    _white_tex: wgpu::Texture,
    pub flat_normal_view: wgpu::TextureView,
    pub white_view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub params_buffer: wgpu::Buffer,
}

pub struct AssetManager {
    mesh_cache: std::collections::HashMap<String, Mesh>,
    texture_cache: std::collections::HashMap<String, Arc<wgpu::BindGroup>>,
    /// Lazily created magenta octahedron used while async loads are in flight.
    placeholder_mesh: Option<Mesh>,
    /// Lazily created shared default maps for the textured-PBR material bind group.
    material_defaults: Option<MaterialDefaults>,

    pub path_to_uuid: std::collections::HashMap<String, Uuid>,
    pub uuid_to_path: std::collections::HashMap<Uuid, String>,
    /// Assets whose bytes are baked into the binary (e.g. via `include_bytes!`).
    pub embedded_assets: std::collections::HashMap<String, std::borrow::Cow<'static, [u8]>>,
}

impl Default for AssetManager {
    fn default() -> Self {
        Self::new()
    }
}

impl AssetManager {
    /// An empty manager. **Touches no filesystem.**
    ///
    /// This used to end with `scan_assets_directory(Path::new("assets"))`, which walked a
    /// CWD-relative directory and *wrote* a `.meta` sidecar next to every asset that lacked one.
    /// There are 43 call sites — every demo binary, every render test, the studio, the renderer's
    /// own constructor — so constructing a manager for any reason stamped files into whatever
    /// asset tree happened to sit beside the working directory.
    ///
    /// What that bought: nothing anything reads. The UUID maps it filled are consumed by exactly
    /// one function, [`Self::resolve_path_from_meta_source`], and only on its UUID branch — which
    /// requires a scene, material or prefab to *store* a UUID. None does; `.meta` files are the
    /// only place a UUID appears in this tree. The path branch normalises its argument and never
    /// looks at the maps.
    ///
    /// Registration is now something a caller asks for: [`Self::scan_assets_directory`] to read
    /// identities that already exist, [`Self::import_assets_directory`] to mint the missing ones.
    pub fn new() -> Self {
        Self {
            mesh_cache: std::collections::HashMap::new(),
            texture_cache: std::collections::HashMap::new(),
            placeholder_mesh: None,
            material_defaults: None,
            path_to_uuid: std::collections::HashMap::new(),
            uuid_to_path: std::collections::HashMap::new(),
            embedded_assets: std::collections::HashMap::new(),
        }
    }

    /// Serbest bırakılmış GPU kaynaklarını (mesh/texture) cache'ten siler.
    /// Sadece referans sayısı 1'e düşmüş (yani ECS'te kullanılmayan ve 
    /// sadece AssetManager'ın bildiği) varlıklar silinir.
    pub fn garbage_collect(&mut self) -> usize {
        let mut freed = 0;
        
        let initial_meshes = self.mesh_cache.len();
        self.mesh_cache.retain(|key, mesh| {
            if key.starts_with("primitive/") { return true; }
            std::sync::Arc::strong_count(&mesh.vbuf) > 1
        });
        freed += initial_meshes - self.mesh_cache.len();

        let initial_textures = self.texture_cache.len();
        self.texture_cache.retain(|key, tex| {
            if key.starts_with("primitive/") { return true; }
            std::sync::Arc::strong_count(tex) > 1
        });
        freed += initial_textures - self.texture_cache.len();

        freed
    }

    // ── Textured-PBR material bind groups ─────────────────────────────────

    /// Lazily create (once) the shared default maps + neutral params buffer used
    /// to fill the auxiliary slots of a material bind group when a map is absent.
    pub(crate) fn ensure_material_defaults(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        if self.material_defaults.is_some() {
            return;
        }

        let mk_linear_1x1 = |label: &str, pixel: [u8; 4]| -> (wgpu::Texture, wgpu::TextureView) {
            let size = wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            };
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                // Linear (NOT sRGB) — normal / MR / AO carry data, not colour.
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &pixel,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4),
                    rows_per_image: Some(1),
                },
                size,
            );
            let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
            (tex, view)
        };

        // Flat tangent-space normal (0.5, 0.5, 1.0) and neutral-white multiplier.
        let (flat_normal_tex, flat_normal_view) =
            mk_linear_1x1("__default_flat_normal__", [128, 128, 255, 255]);
        let (white_tex, white_view) = mk_linear_1x1("__default_white_map__", [255, 255, 255, 255]);

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("material_default_sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("__default_material_params__"),
            contents: bytemuck::cast_slice(&[crate::gpu_types::MaterialParams::default()]),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        self.material_defaults = Some(MaterialDefaults {
            _flat_normal_tex: flat_normal_tex,
            _white_tex: white_tex,
            flat_normal_view,
            white_view,
            sampler,
            params_buffer,
        });
    }

    /// Assemble a full 7-entry textured-PBR material bind group from explicit
    /// texture views + a params buffer.  Every material bind group MUST be built
    /// through here (or [`assemble_single_texture_bind_group`](Self::assemble_single_texture_bind_group))
    /// so it stays layout-compatible with `texture_bind_group_layout`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn assemble_material_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        base_view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        normal_view: &wgpu::TextureView,
        mr_view: &wgpu::TextureView,
        emissive_view: &wgpu::TextureView,
        ao_view: &wgpu::TextureView,
        params_buffer: &wgpu::Buffer,
        label: &str,
    ) -> Arc<wgpu::BindGroup> {
        Arc::new(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(base_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(normal_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(mr_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(emissive_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(ao_view),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: params_buffer.as_entire_binding(),
                },
            ],
        }))
    }

    /// Borrow the shared material defaults (only `Some` after
    /// [`ensure_material_defaults`](Self::ensure_material_defaults) has run).
    pub(crate) fn material_defaults(&self) -> Option<&MaterialDefaults> {
        self.material_defaults.as_ref()
    }

    /// Build a material bind group for a single base-colour texture, filling the
    /// normal/MR/emissive/AO/params slots with the shared neutral defaults.
    pub(crate) fn assemble_single_texture_bind_group(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        base_view: &wgpu::TextureView,
        base_sampler: &wgpu::Sampler,
        label: &str,
    ) -> Arc<wgpu::BindGroup> {
        self.ensure_material_defaults(device, queue);
        let d = self
            .material_defaults
            .as_ref()
            .expect("material defaults ensured above");
        Self::assemble_material_bind_group(
            device,
            layout,
            base_view,
            base_sampler,
            &d.flat_normal_view,
            &d.white_view,
            &d.white_view,
            &d.white_view,
            &d.params_buffer,
            label,
        )
    }

    // ── Path / UUID helpers ───────────────────────────────────────────────

    /// Normalise a file-system path to forward-slash form for use as a map key.
    ///
    /// Uses [`Path`] to avoid platform-specific separator assumptions.
    pub fn normalize_path(path: &str) -> String {
        Path::new(path)
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/")
    }

    /// Return the UUID registered for `path`, if any.
    pub fn get_uuid(&self, path: &str) -> Option<Uuid> {
        self.path_to_uuid.get(&Self::normalize_path(path)).copied()
    }

    /// Return the filesystem path registered for `uuid`, if any.
    pub fn get_path(&self, uuid: &Uuid) -> Option<String> {
        self.uuid_to_path.get(uuid).cloned()
    }

    /// Resolve a load source to a filesystem path.
    ///
    /// If `source` parses as a UUID, the registered path is returned.
    /// Otherwise `source` is normalised and returned as-is.
    pub fn resolve_path_from_meta_source(&self, source: &str) -> Result<String, AssetError> {
        if let Ok(id) = Uuid::parse_str(source) {
            self.get_path(&id).ok_or_else(|| AssetError::MissingUuid {
                source: source.to_string(),
            })
        } else {
            Ok(Self::normalize_path(source))
        }
    }

    /// Return a cached mesh by its source ID without triggering a load.
    pub fn get_cached_mesh(&self, source_id: &str) -> Option<Mesh> {
        self.mesh_cache.get(source_id).cloned()
    }

    /// Embed a raw asset byte slice under `path` so it can be loaded without
    /// a filesystem read.
    pub fn embed_asset(&mut self, path: &str, data: impl Into<std::borrow::Cow<'static, [u8]>>) {
        self.embedded_assets
            .insert(Self::normalize_path(path), data.into());
    }

    // ── Asset scanning ────────────────────────────────────────────────────

    /// Recursively register the assets under `dir` that **already** carry a `.meta` sidecar.
    ///
    /// Read-only: an asset with no sidecar is skipped, not stamped. Minting is
    /// [`Self::import_assets_directory`], which is a separate call because it is a separate
    /// decision — writing into someone's asset tree is an action a project-import flow takes
    /// deliberately, not a thing that happens because a directory got walked.
    ///
    /// Safe to call repeatedly; entries are updated, not duplicated.
    pub fn scan_assets_directory(&mut self, dir: &Path) {
        self.walk_assets(dir, false);
    }

    /// Like [`Self::scan_assets_directory`], but **writes**: any asset with no `.meta` sidecar
    /// gets a fresh UUID and a sidecar next to it.
    ///
    /// This is the import action. Note what identity by sidecar can and cannot do: the sidecar
    /// travels with the filename, so a rename orphans it and mints a new UUID for the new name —
    /// this repository's own `assets/` holds 10 orphaned sidecars from deleted `.glb` files
    /// against 6 live assets. Re-adding a file under its old name recovers the old identity;
    /// renaming does not preserve it.
    pub fn import_assets_directory(&mut self, dir: &Path) {
        self.walk_assets(dir, true);
    }

    /// The shared walker. `mint` decides whether a missing sidecar is created or the asset is
    /// skipped — the only difference between scanning and importing.
    fn walk_assets(&mut self, dir: &Path, mint: bool) {
        if !dir.is_dir() {
            return;
        }

        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => {
                tracing::error!(
                    "[AssetManager] Cannot read directory {}: {e}",
                    dir.display()
                );
                return;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();

            if path.is_dir() {
                self.walk_assets(&path, mint);
                continue;
            }

            let is_asset = path
                .extension()
                .map(|ext| {
                    matches!(
                        ext.to_string_lossy().to_lowercase().as_str(),
                        "obj"
                            | "gltf"
                            | "glb"
                            | "png"
                            | "jpg"
                            | "jpeg"
                            | "hdr"
                            | "wav"
                            | "mp3"
                            | "ogg"
                            | "ttf"
                            | "otf"
                            | "ron"
                    )
                })
                .unwrap_or(false);

            if !is_asset {
                continue;
            }

            let meta_path = PathBuf::from(format!("{}.meta", path.display()));
            let Some(uuid) = self.read_meta_or_mint(&path, &meta_path, mint) else {
                continue; // scanning, and this asset has no identity yet
            };

            let normalized = Self::normalize_path(&path.to_string_lossy());
            self.path_to_uuid.insert(normalized.clone(), uuid);
            self.uuid_to_path.insert(uuid, normalized);
        }
    }

    /// Read an existing `.meta` file, and mint one only if `mint` is set.
    ///
    /// Returns `None` when there is no usable identity and we were not asked to create one.
    fn read_meta_or_mint(&self, asset_path: &Path, meta_path: &Path, mint: bool) -> Option<Uuid> {
        if meta_path.exists() {
            match std::fs::read_to_string(meta_path)
                .map_err(|e| e.to_string())
                .and_then(|s| ron::from_str::<AssetMeta>(&s).map_err(|e| e.to_string()))
            {
                Ok(meta) => return Some(meta.uuid),
                Err(e) => {
                    tracing::error!(
                        "[AssetManager] WARN: corrupt .meta for '{}' ({e}). \
                         Regenerating UUID — existing scene references to this \
                         asset will break.",
                        asset_path.display()
                    );
                    // Fall through: an import replaces it, a scan leaves it alone.
                }
            }
        }

        if !mint {
            return None;
        }

        let uuid = Uuid::new_v4();
        let meta = AssetMeta { uuid };

        match ron::ser::to_string_pretty(&meta, ron::ser::PrettyConfig::default()) {
            Ok(ron_str) => {
                if let Err(e) = std::fs::write(meta_path, ron_str) {
                    tracing::error!(
                        "[AssetManager] WARN: could not write .meta for '{}': {e}",
                        asset_path.display()
                    );
                }
            }
            Err(e) => tracing::error!("[AssetManager] WARN: RON serialisation failed: {e}"),
        }

        Some(uuid)
    }

    // ── Placeholder mesh ──────────────────────────────────────────────────

    /// Return (creating if needed) a small magenta octahedron used as a
    /// stand-in while an async asset load is in flight.
    pub fn loading_placeholder_mesh(&mut self, device: &wgpu::Device) -> Mesh {
        if let Some(ref m) = self.placeholder_mesh {
            return m.clone();
        }
        let m = Self::create_loading_placeholder(device);
        self.placeholder_mesh = Some(m.clone());
        m
    }

    fn create_loading_placeholder(device: &wgpu::Device) -> Mesh {
        // Octahedron — recognisable from any angle, low vertex count.
        const POSITIONS: [[f32; 3]; 6] = [
            [1.0, 0.0, 0.0],  // +X
            [-1.0, 0.0, 0.0], // -X
            [0.0, 1.0, 0.0],  // +Y
            [0.0, -1.0, 0.0], // -Y
            [0.0, 0.0, 1.0],  // +Z
            [0.0, 0.0, -1.0], // -Z
        ];
        const TRIANGLES: [[usize; 3]; 8] = [
            [0, 2, 4],
            [2, 1, 4],
            [1, 3, 4],
            [3, 0, 4],
            [2, 0, 5],
            [1, 2, 5],
            [3, 1, 5],
            [0, 3, 5],
        ];
        const COLOR: [f32; 4] = [0.95, 0.45, 0.95, 1.0]; // magenta

        let mut vertices = Vec::with_capacity(TRIANGLES.len() * 3);

        for tri in &TRIANGLES {
            for &i in tri {
                let pos = POSITIONS[i];
                let n = Vec3::new(pos[0], pos[1], pos[2]).normalize();
                vertices.push(Vertex {
                    position: pos,
                    normal: [n.x, n.y, n.z],
                    tex_coords: [0.0, 0.0],
                    color: COLOR,
                    joint_indices: [0; 4],
                    joint_weights: [0.0; 4],
                    ..Default::default()
                });
            }
        }

        Mesh::new_indexed(
            device,
            &vertices,
            Vec3::ZERO,
            "__async_loading__".to_string(),
        )
    }
}

/// Reads an asset's `.meta` sidecar, or `None` when it has none.
///
/// The read-only half of `read_meta_or_mint`, and the distinction is the point: this never mints
/// an identity. The editor's asset detail pane calls it, and a pane that created a UUID because you
/// clicked a file would stamp identities onto assets you merely looked at.
///
/// A free function rather than a method, because a reader needs no `AssetManager` — and because the
/// editor must not gain a `ron` dependency to parse this itself. `gizmo_scene::SceneData::from_ron_str`
/// exists for exactly that reason: parsing belongs to the crate that already owns the parser.
pub fn read_asset_meta(asset_path: &Path) -> Option<AssetMeta> {
    let meta_path = PathBuf::from(format!("{}.meta", asset_path.display()));
    let text = std::fs::read_to_string(&meta_path).ok()?;
    match ron::from_str::<AssetMeta>(&text) {
        Ok(meta) => Some(meta),
        Err(e) => {
            // Reported, and deliberately NOT repaired: `import_assets_directory` answers a corrupt
            // sidecar by minting a fresh UUID, which silently breaks every reference to the old
            // one. Doing that from a mouse click would be worse still.
            tracing::warn!(path = %meta_path.display(), error = %e, "[AssetManager] bozuk .meta sidecar");
            None
        }
    }
}


#[cfg(test)]
mod asset_meta_tests {
    use super::{read_asset_meta, AssetMeta};

    fn temp(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "gizmo_meta_{tag}_{}",
            N.fetch_add(1, Ordering::Relaxed)
        ))
    }

    /// Reads the sidecar beside a file, in the shape the 22 committed ones use.
    #[test]
    fn a_sidecar_next_to_the_asset_is_read() {
        let asset = temp("read");
        std::fs::write(&asset, b"not really an image").unwrap();
        let meta_path = format!("{}.meta", asset.display());
        std::fs::write(
            &meta_path,
            "(\n    uuid: \"ed91a228-ef2a-49ab-a525-1eb3ed0660c0\",\n)",
        )
        .unwrap();

        let meta = read_asset_meta(&asset).expect("sidecar should be read");
        assert_eq!(meta.uuid.to_string(), "ed91a228-ef2a-49ab-a525-1eb3ed0660c0");

        let _ = std::fs::remove_file(&asset);
        let _ = std::fs::remove_file(&meta_path);
    }

    /// No sidecar is `None`, and — the part that matters — reading does not CREATE one.
    ///
    /// `import_assets_directory` mints a UUID when the sidecar is missing. This function must not: the
    /// editor calls it when you click a thumbnail, and minting identity from a mouse click would
    /// stamp UUIDs onto files the user merely looked at.
    #[test]
    fn reading_never_creates_a_sidecar() {
        let asset = temp("nocreate");
        std::fs::write(&asset, b"x").unwrap();
        let meta_path = std::path::PathBuf::from(format!("{}.meta", asset.display()));

        assert!(read_asset_meta(&asset).is_none());
        assert!(
            !meta_path.exists(),
            "the reader wrote a sidecar — it must never mint identity"
        );

        let _ = std::fs::remove_file(&asset);
    }

    /// A corrupt sidecar reads as absent rather than as a fresh identity.
    #[test]
    fn a_corrupt_sidecar_is_not_silently_reminted() {
        let asset = temp("corrupt");
        std::fs::write(&asset, b"x").unwrap();
        let meta_path = format!("{}.meta", asset.display());
        std::fs::write(&meta_path, "this is not ron at all").unwrap();

        assert!(read_asset_meta(&asset).is_none());
        // And the corrupt file is left exactly as it was, for a human to look at.
        assert_eq!(std::fs::read_to_string(&meta_path).unwrap(), "this is not ron at all");

        let _ = std::fs::remove_file(&asset);
        let _ = std::fs::remove_file(&meta_path);
    }

    /// The shape the sidecars on disk actually have round-trips.
    #[test]
    fn the_committed_sidecar_shape_round_trips() {
        let meta = AssetMeta { uuid: uuid::Uuid::new_v4() };
        let text = ron::ser::to_string(&meta).unwrap();
        let back: AssetMeta = ron::from_str(&text).unwrap();
        assert_eq!(back.uuid, meta.uuid);
    }
}

/// Constructing a manager, and scanning with one, must not write to the asset tree.
///
/// # What these pin
///
/// `AssetManager::new()` used to walk a CWD-relative `assets/` and stamp a `.meta` sidecar next to
/// every asset that lacked one — from 43 call sites, including every render test and every demo
/// binary. The first test here fails the moment that scan comes back; the second and third pin the
/// split that replaced it, so "scan" cannot quietly regain the ability to write.
#[cfg(test)]
mod scan_does_not_write_tests {
    use super::AssetManager;
    use std::path::{Path, PathBuf};

    /// A directory holding one asset file with no sidecar.
    ///
    /// Named by `tag` alone — no counter. A shared counter made the directory name depend on which
    /// test the harness scheduled first, so a run could land on a name a *previous* run had left
    /// behind, inherit its minted sidecars, and fail with a count that had nothing to do with the
    /// code. Tags are unique per test, and the tree is wiped before it is built, so each test gets
    /// the same directory in the same state every time.
    fn tree(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gizmo_scan_{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        std::fs::write(dir.join("thing.png"), b"not really a png").unwrap();
        std::fs::write(dir.join("nested").join("deep.obj"), b"o cube").unwrap();
        dir
    }

    fn sidecars(dir: &Path) -> usize {
        let mut n = 0;
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                n += sidecars(&p);
            } else if p.extension().is_some_and(|x| x == "meta") {
                n += 1;
            }
        }
        n
    }

    /// The defect itself: the constructor's body must not reach the filesystem.
    ///
    /// # Why this reads source instead of behaviour
    ///
    /// The obvious runtime test — construct a manager, assert the maps are empty — **does not
    /// bite**, and I verified that by reintroducing the defect: the old code scanned
    /// `Path::new("assets")` relative to the process CWD, which under `cargo test` is
    /// `crates/gizmo-renderer/`, and that directory has no `assets/` subtree. The scan returns at
    /// its first `is_dir()` check, the maps stay empty, and the test passes with the bug present.
    /// Making it bite would need `set_current_dir`, which is process-global and would race every
    /// other test in this binary.
    ///
    /// So the instrument matches the invariant: *the constructor performs no I/O*. The body is
    /// extracted by brace matching rather than scanning the whole file, so nothing in this test
    /// module can satisfy the search — the failure mode of the first guard test I wrote in this
    /// codebase, which searched for a string that appeared in its own assertion.
    #[test]
    fn the_constructor_body_touches_no_filesystem() {
        let src = include_str!("mod.rs");
        let sig = "pub fn new() -> Self {";
        let start = src.find(sig).expect("AssetManager::new not found — was it renamed?");

        // Brace-match from the opening `{` of the signature to its close.
        let open = start + sig.len() - 1;
        let mut depth = 0usize;
        let mut end = open;
        for (i, c) in src[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = open + i;
                        break;
                    }
                }
                _ => {}
            }
        }
        let body = &src[open..=end];
        assert!(body.len() > 20, "brace matching failed; the guard would be vacuous");

        for needle in [
            "scan_assets_directory",
            "import_assets_directory",
            "walk_assets",
            "read_dir",
            "fs::",
            "Path::new",
        ] {
            assert!(
                !body.contains(needle),
                "`AssetManager::new` mentions `{needle}`. The constructor must not touch the \
                 filesystem: it has 43 call sites — every demo binary, every render test, the \
                 studio — and the version that scanned a CWD-relative `assets/` stamped `.meta` \
                 sidecars into whatever asset tree sat next to the working directory. Registration \
                 belongs to `scan_assets_directory` (read-only) or `import_assets_directory` \
                 (mints), called by whoever actually wants it."
            );
        }
    }

    /// A freshly constructed manager knows about nothing.
    ///
    /// Weaker than the guard above and kept for the behaviour it states directly — see that test
    /// for why this one cannot catch the original defect on its own.
    #[test]
    fn a_new_manager_has_registered_nothing() {
        let m = AssetManager::new();
        assert!(m.path_to_uuid.is_empty() && m.uuid_to_path.is_empty());
    }

    /// Scanning registers what already has identity, and creates none.
    #[test]
    fn scanning_reads_existing_identities_and_mints_none() {
        let dir = tree("scan");
        // Give exactly one of the two assets an identity.
        let known = uuid::Uuid::new_v4();
        std::fs::write(
            dir.join("thing.png.meta"),
            ron::ser::to_string(&super::AssetMeta { uuid: known }).unwrap(),
        )
        .unwrap();

        let mut m = AssetManager::new();
        m.scan_assets_directory(&dir);

        assert_eq!(
            m.path_to_uuid.len(),
            1,
            "scan registered {} assets; only the one with a sidecar has an identity to register",
            m.path_to_uuid.len()
        );
        assert_eq!(m.get_uuid(&dir.join("thing.png").to_string_lossy()), Some(known));
        assert_eq!(
            sidecars(&dir),
            1,
            "scanning minted a sidecar for the asset that had none"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Importing is the half that writes — including into subdirectories.
    #[test]
    fn importing_mints_the_missing_sidecars() {
        let dir = tree("import");
        let mut m = AssetManager::new();
        m.import_assets_directory(&dir);

        assert_eq!(m.path_to_uuid.len(), 2, "import missed the nested asset");
        assert_eq!(sidecars(&dir), 2, "import did not write both sidecars");

        // And a second import is idempotent: identities are read back, not reminted.
        let first = m.get_uuid(&dir.join("thing.png").to_string_lossy()).unwrap();
        let mut m2 = AssetManager::new();
        m2.import_assets_directory(&dir);
        assert_eq!(
            m2.get_uuid(&dir.join("thing.png").to_string_lossy()),
            Some(first),
            "re-importing changed an asset's identity, breaking every reference to it"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
