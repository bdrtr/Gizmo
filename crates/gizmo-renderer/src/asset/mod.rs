use crate::components::Mesh;
use crate::renderer::Vertex;
use gizmo_math::Vec3;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tobj;
use uuid::Uuid;
use wgpu::util::DeviceExt;

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct AssetMeta {
    pub uuid: Uuid,
}

/// Decode an image file to RGBA8 on a background thread (CPU only).
pub fn decode_rgba_image_file(path: &str) -> Result<(Vec<u8>, u32, u32), String> {
    let img = image::open(path)
        .map_err(|e| format!("Doku okunamadi ({path}): {e}"))?
        .to_rgba8();
    let (w, h) = img.dimensions();
    Ok((img.into_raw(), w, h))
}

/// OBJ → vertices + AABB without GPU (for [`crate::async_assets::AsyncAssetLoader`]).
pub fn decode_obj_vertices_for_async(
    file_path: &str,
) -> Result<(Vec<Vertex>, gizmo_math::Aabb), String> {
    let (models, _materials) = tobj::load_obj(
        file_path,
        &tobj::LoadOptions {
            single_index: true,
            triangulate: true,
            ignore_points: true,
            ignore_lines: true,
        },
    )
    .map_err(|e| format!("OBJ ({file_path}): {e}"))?;

    if models.is_empty() {
        return Err(format!("OBJ dosyasinda model yok: {file_path}"));
    }

    let mut aabb = gizmo_math::Aabb::empty();
    let mut vertices = Vec::new();
    let mut has_missing_normals = false;

    for model in &models {
        let m = &model.mesh;
        if m.normals.is_empty() {
            has_missing_normals = true;
        }

        for i in &m.indices {
            let idx = *i as usize;

            if idx * 3 + 2 >= m.positions.len() {
                return Err(format!("OBJ dosyasinda gecersiz pozisyon indeksi: {}", idx));
            }
            let position = [
                m.positions[idx * 3],
                m.positions[idx * 3 + 1],
                m.positions[idx * 3 + 2],
            ];
            aabb.extend(Vec3::new(position[0], position[1], position[2]));

            let normal = if !m.normals.is_empty() {
                if idx * 3 + 2 >= m.normals.len() {
                    return Err(format!("OBJ dosyasinda gecersiz normal indeksi: {}", idx));
                }
                [
                    m.normals[idx * 3],
                    m.normals[idx * 3 + 1],
                    m.normals[idx * 3 + 2],
                ]
            } else {
                [0.0, 1.0, 0.0]
            };

            let tex_coords = if !m.texcoords.is_empty() {
                if idx * 2 + 1 >= m.texcoords.len() {
                    return Err(format!("OBJ dosyasinda gecersiz UV indeksi: {}", idx));
                }
                [m.texcoords[idx * 2], 1.0 - m.texcoords[idx * 2 + 1]]
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
    }

    if has_missing_normals {
        let mut iter = vertices.chunks_exact_mut(3);
        for chunk in iter.by_ref() {
            let p0 = chunk[0].position;
            let p1 = chunk[1].position;
            let p2 = chunk[2].position;
            let v0 = Vec3::new(p0[0], p0[1], p0[2]);
            let v1 = Vec3::new(p1[0], p1[1], p1[2]);
            let v2 = Vec3::new(p2[0], p2[1], p2[2]);
            let norm = (v1 - v0).cross(v2 - v0);
            let final_norm = if norm.length_squared() > 1e-6 {
                norm.normalize()
            } else {
                Vec3::new(0.0, 1.0, 0.0)
            };
            let n_arr = [final_norm.x, final_norm.y, final_norm.z];
            chunk[0].normal = n_arr;
            chunk[1].normal = n_arr;
            chunk[2].normal = n_arr;
        }
        if !iter.into_remainder().is_empty() {
            eprintln!("Uyari: '{}' OBJ dosyasindaki yuzeyler 3gen degil (kalan kopuk vertexler tespit edildi).", file_path);
        }
    }

    Ok((vertices, aabb))
}

pub struct AssetManager {
    mesh_cache: std::collections::HashMap<String, Mesh>,
    pub texture_cache: std::collections::HashMap<String, Arc<wgpu::BindGroup>>,
    /// Reused for [`Self::loading_placeholder_mesh`] while async mesh loads complete.
    placeholder_mesh: Option<Mesh>,

    pub path_to_uuid: std::collections::HashMap<String, Uuid>,
    pub uuid_to_path: std::collections::HashMap<Uuid, String>,
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
        };
        manager.scan_assets_directory(Path::new("assets"));
        manager
    }

    pub fn normalize_path(path: &str) -> String {
        path.replace("\\", "/")
    }

    /// Tries to resolve a UUID from a raw string path
    pub fn get_uuid(&self, path: &str) -> Option<Uuid> {
        // Normalize path
        let normalized = Self::normalize_path(path);
        self.path_to_uuid.get(&normalized).copied()
    }

    /// Tries to resolve a string path from a UUID
    pub fn get_path(&self, uuid: &Uuid) -> Option<String> {
        self.uuid_to_path.get(uuid).cloned()
    }

    /// Tries to parse a UUID string and unwrap the physical file path.
    pub fn resolve_path_from_meta_source(&self, source: &str) -> Result<String, String> {
        if let Ok(id) = Uuid::parse_str(source) {
            self.get_path(&id).ok_or_else(|| format!("Kayip UUID referansi: {}", source))
        } else {
            Ok(Self::normalize_path(source))
        }
    }

    /// Bellekteki bir modeli ID'si ile geri döndürür. (GLTF yüklemelerinde diskte dosya olmadığı için hayati önem taşır)
    pub fn get_cached_mesh(&self, source_id: &str) -> Option<Mesh> {
        self.mesh_cache.get(source_id).cloned()
    }

    pub fn scan_assets_directory(&mut self, dir: &Path) {
        if !dir.exists() || !dir.is_dir() {
            return;
        }

        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    self.scan_assets_directory(&path);
                } else if path.extension().is_some_and(|ext| {
                    let e = ext.to_string_lossy().to_lowercase();
                    matches!(
                        e.as_str(),
                        "obj" | "gltf" | "glb" | "png" | "jpg" | "jpeg" | "hdr" | "wav" | "mp3" | "ogg" | "ttf" | "otf" | "ron"
                    )
                }) {
                    // It's an asset file
                    let meta_path = PathBuf::from(format!("{}.meta", path.display()));
                    let mut needs_save = false;
                    let uuid = if meta_path.exists() {
                        if let Ok(content) = std::fs::read_to_string(&meta_path) {
                            if let Ok(meta) = ron::from_str::<AssetMeta>(&content) {
                                meta.uuid
                            } else {
                                needs_save = true;
                                Uuid::new_v4() // fallback if corrupt
                            }
                        } else {
                            needs_save = true;
                            Uuid::new_v4()
                        }
                    } else {
                        needs_save = true;
                        Uuid::new_v4()
                    };

                    if needs_save {
                        let meta = AssetMeta { uuid };
                        if let Ok(ron_str) =
                            ron::ser::to_string_pretty(&meta, ron::ser::PrettyConfig::default())
                        {
                            let _ = std::fs::write(&meta_path, ron_str);
                        }
                    }

                    let normalized_path = path.to_string_lossy().replace("\\", "/");
                    self.path_to_uuid.insert(normalized_path.clone(), uuid);
                    self.uuid_to_path.insert(uuid, normalized_path);
                }
            }
        }
    }

    /// Küçük renkli placeholder (async OBJ/GLTF beklerken kullanılır).
    pub fn loading_placeholder_mesh(&mut self, device: &wgpu::Device) -> Mesh {
        if let Some(ref m) = self.placeholder_mesh {
            return m.clone();
        }
        let m = Self::create_loading_placeholder(device);
        self.placeholder_mesh = Some(m.clone());
        m
    }

    fn create_loading_placeholder(device: &wgpu::Device) -> Mesh {
        // Octahedron — küçük, her açıdan görünür
        let p = [
            [1.0f32, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0],
        ];
        let col = [0.95, 0.45, 0.95];
        let idx: [[usize; 3]; 8] = [
            [0, 2, 4],
            [2, 1, 4],
            [1, 3, 4],
            [3, 0, 4],
            [2, 0, 5],
            [1, 2, 5],
            [3, 1, 5],
            [0, 3, 5],
        ];
        let mut vertices = Vec::with_capacity(24);
        for tri in idx {
            for &i in &tri {
                let pos = p[i];
                let n = Vec3::new(pos[0], pos[1], pos[2]).normalize();
                vertices.push(Vertex {
                    position: pos,
                    normal: [n.x, n.y, n.z],
                    tex_coords: [0.0, 0.0],
                    color: col,
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

pub mod loaders;
pub mod primitives;
pub mod texture;

pub use loaders::GltfNodeData;
