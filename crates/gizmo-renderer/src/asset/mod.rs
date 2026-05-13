use crate::components::Mesh;
use crate::renderer::Vertex;
use gizmo_math::Vec3;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;
use wgpu::util::DeviceExt;

pub mod loaders;
pub mod primitives;
pub mod texture;

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
pub fn decode_rgba_image_file(path: &str) -> Result<(Vec<u8>, u32, u32), String> {
    let img = image::open(path)
        .map_err(|e| format!("Cannot read texture ({path}): {e}"))?
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
) -> Result<(Vec<Vertex>, gizmo_math::Aabb), String> {
    let (models, _) = tobj::load_obj(
        file_path,
        &tobj::LoadOptions {
            single_index: true,
            triangulate: true,
            ignore_points: true,
            ignore_lines: true,
        },
    )
    .map_err(|e| format!("OBJ load failed ({file_path}): {e}"))?;

    if models.is_empty() {
        return Err(format!("OBJ file contains no models: {file_path}"));
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
                return Err(format!(
                    "OBJ ({file_path}): position index {idx} out of range \
                     (positions.len={})",
                    m.positions.len()
                ));
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
                    return Err(format!(
                        "OBJ ({file_path}): normal index {idx} out of range \
                         (normals.len={})",
                        m.normals.len()
                    ));
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
                    return Err(format!(
                        "OBJ ({file_path}): texcoord index {idx} out of range \
                         (texcoords.len={})",
                        m.texcoords.len()
                    ));
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

pub struct AssetManager {
    mesh_cache: std::collections::HashMap<String, Mesh>,
    texture_cache: std::collections::HashMap<String, Arc<wgpu::BindGroup>>,
    /// Lazily created magenta octahedron used while async loads are in flight.
    placeholder_mesh: Option<Mesh>,

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
            path_to_uuid: std::collections::HashMap::new(),
            uuid_to_path: std::collections::HashMap::new(),
            embedded_assets: std::collections::HashMap::new(),
        };
        manager.scan_assets_directory(Path::new("assets"));
        manager
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
    pub fn resolve_path_from_meta_source(&self, source: &str) -> Result<String, String> {
        if let Ok(id) = Uuid::parse_str(source) {
            self.get_path(&id)
                .ok_or_else(|| format!("Missing UUID reference: {source}"))
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
