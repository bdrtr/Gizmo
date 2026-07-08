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
                color: [1.0, 1.0, 1.0],
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
    pub fn new() -> Self {
        let mut manager = Self {
            mesh_cache: std::collections::HashMap::new(),
            texture_cache: std::collections::HashMap::new(),
            placeholder_mesh: None,
            material_defaults: None,
            path_to_uuid: std::collections::HashMap::new(),
            uuid_to_path: std::collections::HashMap::new(),
            embedded_assets: std::collections::HashMap::new(),
        };
        manager.scan_assets_directory(Path::new("assets"));
        manager
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

    /// Recursively scan `dir` for known asset extensions, creating or
    /// reading `.meta` sidecar files to assign stable UUIDs.
    ///
    /// Safe to call multiple times — existing entries are updated, not
    /// duplicated.
    pub fn scan_assets_directory(&mut self, dir: &Path) {
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
                self.scan_assets_directory(&path);
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
            let uuid = self.read_or_create_meta(&path, &meta_path);

            let normalized = Self::normalize_path(&path.to_string_lossy());
            self.path_to_uuid.insert(normalized.clone(), uuid);
            self.uuid_to_path.insert(uuid, normalized);
        }
    }

    /// Read an existing `.meta` file or create a new one, returning the UUID.
    fn read_or_create_meta(&self, asset_path: &Path, meta_path: &Path) -> Uuid {
        if meta_path.exists() {
            match std::fs::read_to_string(meta_path)
                .map_err(|e| e.to_string())
                .and_then(|s| ron::from_str::<AssetMeta>(&s).map_err(|e| e.to_string()))
            {
                Ok(meta) => return meta.uuid,
                Err(e) => {
                    tracing::error!(
                        "[AssetManager] WARN: corrupt .meta for '{}' ({e}). \
                         Regenerating UUID — existing scene references to this \
                         asset will break.",
                        asset_path.display()
                    );
                    // Fall through to generate a fresh UUID.
                }
            }
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

        uuid
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
        const COLOR: [f32; 3] = [0.95, 0.45, 0.95]; // magenta

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

        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Async loading placeholder"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Mesh::new(
            device,
            Arc::new(vbuf),
            &vertices,
            Vec3::ZERO,
            "__async_loading__".to_string(),
        )
    }
}
