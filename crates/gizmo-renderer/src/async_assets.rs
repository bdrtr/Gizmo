//! Background-thread decoding for textures, OBJ, and GLTF **import** (disk + parse).
//! GPU upload and [`AssetManager`](crate::asset::AssetManager) updates must run on the main thread
//! — call [`AsyncAssetLoader::drain_completed`] each frame and then upload via `AssetManager`.

use crate::asset::error::AssetError;
#[cfg(not(target_arch = "wasm32"))]
use crate::asset::{decode_obj_vertices_for_async, decode_rgba_image_file};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;

#[derive(Debug)]
pub struct TextureReloadCompletion {
    pub cache_key: String,
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub entity_ids: Vec<usize>,
}

#[derive(Debug)]
pub struct ObjLoadCompletion {
    pub path: String,
    pub vertices: Vec<crate::gpu_types::Vertex>,
    pub aabb: gizmo_math::Aabb,
    pub handle_ids: Vec<usize>,
}

/// Successful GLTF parse on the worker; GPU upload via [`AssetManager::load_gltf_from_import`](crate::asset::AssetManager::load_gltf_from_import).
#[derive(Debug)]
pub struct GltfImportCompletion {
    pub path: String,
    pub document: gltf::Document,
    pub buffers: Vec<gltf::buffer::Data>,
    pub images: Vec<gltf::image::Data>,
}

#[derive(Debug)]
pub struct GltfImportError {
    pub path: String,
    pub message: String,
}

impl std::fmt::Display for GltfImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GLTF import failed for '{}': {}", self.path, self.message)
    }
}

impl std::error::Error for GltfImportError {}

#[derive(Debug)]
pub struct CompletedAsyncLoads {
    pub textures: Vec<TextureReloadCompletion>,
    pub objs: Vec<ObjLoadCompletion>,
    pub gltfs: Vec<GltfImportCompletion>,
    pub gltf_errors: Vec<GltfImportError>,
}

// WASM: istekler spawn_local fetch yoluna gider — Job kanalı yalnız native
// worker thread'i besler (hedefli allow, native lint gücü korunur).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
enum Job {
    Texture { request_path: String },
    Obj { path: String },
    Gltf { path: String },
}

enum WorkerMsg {
    Texture {
        request_path: String,
        cache_key: String,
        result: Result<(Vec<u8>, u32, u32), AssetError>,
    },
    Obj {
        path: String,
        result: Result<(Vec<crate::gpu_types::Vertex>, gizmo_math::Aabb), AssetError>,
    },
    Gltf {
        path: String,
        // Box'lu: bu varyantın payload'ı (gltf::Document + buffer/image data)
        // diğer varyantlardan çok büyük; Box'lamak enum boyutunu küçük tutar
        // (clippy::large_enum_variant). Gltf mesajları seyrek, indirection bedava.
        result: Box<
            Result<
                (
                    gltf::Document,
                    Vec<gltf::buffer::Data>,
                    Vec<gltf::image::Data>,
                ),
                AssetError,
            >,
        >,
    },
}

struct LoaderShared {
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    job_tx: SyncSender<Job>,
    result_rx: Receiver<WorkerMsg>,
    /// Original request path (as passed to `request_texture_reload`) → entities
    texture_waiters: HashMap<String, Vec<usize>>,
    obj_waiters: HashMap<String, Vec<usize>>,
    texture_inflight: HashSet<String>,
    obj_inflight: HashSet<String>,
    gltf_inflight: HashSet<String>,
}

/// Thread-safe loader; safe to store as an ECS resource (`Send` + `Sync`).
pub struct AsyncAssetLoader {
    shared: Arc<Mutex<LoaderShared>>,
    _worker: Option<thread::JoinHandle<()>>,
    #[allow(dead_code)]
    result_tx: Sender<WorkerMsg>,
}

impl AsyncAssetLoader {
    pub fn new() -> Self {
        let (job_tx, job_rx) = mpsc::sync_channel::<Job>(64);
        let (result_tx, result_rx) = mpsc::channel::<WorkerMsg>();

        #[cfg(not(target_arch = "wasm32"))]
        let worker_job_rx = job_rx;
        #[cfg(not(target_arch = "wasm32"))]
        let worker_result_tx = result_tx.clone();
        #[cfg(not(target_arch = "wasm32"))]
        let _worker = Some(
            thread::Builder::new()
                .name("gizmo-async-assets".into())
                .spawn(move || {
                    for job in worker_job_rx {
                        match job {
                            Job::Texture { request_path } => {
                                let cache_key = std::path::Path::new(&request_path)
                                    .canonicalize()
                                    .map(|p| p.to_string_lossy().into_owned())
                                    .unwrap_or_else(|_| request_path.clone());
                                let result = decode_rgba_image_file(&request_path);
                                let _ = worker_result_tx.send(WorkerMsg::Texture {
                                    request_path,
                                    cache_key,
                                    result,
                                });
                            }
                            Job::Obj { path } => {
                                let result = decode_obj_vertices_for_async(&path);
                                let _ = worker_result_tx.send(WorkerMsg::Obj { path, result });
                            }
                            Job::Gltf { path } => {
                                let result = gltf::import(&path).map_err(|source| {
                                    AssetError::GltfImport {
                                        path: std::path::PathBuf::from(&path),
                                        source,
                                    }
                                });
                                let _ = worker_result_tx
                                    .send(WorkerMsg::Gltf { path, result: Box::new(result) });
                            }
                        }
                    }
                })
                .expect("spawn async asset worker"),
        );

        #[cfg(target_arch = "wasm32")]
        let _worker = {
            // No worker thread on the web: requests go through
            // `wasm_bindgen_futures::spawn_local` fetch paths instead, so the
            // job channel's receive end is intentionally dropped here.
            drop(job_rx);
            None
        };

        Self {
            shared: Arc::new(Mutex::new(LoaderShared {
                job_tx,
                result_rx,
                texture_waiters: HashMap::new(),
                obj_waiters: HashMap::new(),
                texture_inflight: HashSet::new(),
                obj_inflight: HashSet::new(),
                gltf_inflight: HashSet::new(),
            })),
            _worker,
            result_tx,
        }
    }

    /// Queue a texture file decode; when done, [`drain_completed`] yields a row with `entity_ids`.
    /// Duplicate `request_path` while in-flight only adds more waiters (one disk read).
    pub fn request_texture_reload(&self, request_path: String, handle_id: usize) {
        // Poison-recovery: guarded state is plain (queues/sets) and remains consistent
        // even if a thread panicked while holding the lock, so recover instead of panicking.
        let mut g = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        g.texture_waiters
            .entry(request_path.clone())
            .or_default()
            .push(handle_id);
        if g.texture_inflight.insert(request_path.clone()) {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let _ = g.job_tx.send(Job::Texture { request_path });
            }
            #[cfg(target_arch = "wasm32")]
            {
                let result_tx = self.result_tx.clone();
                let path = request_path.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    let result = fetch_and_decode_texture_wasm(&path).await;
                    let cache_key = path.clone();
                    let _ = result_tx.send(WorkerMsg::Texture {
                        request_path: path,
                        cache_key,
                        result,
                    });
                });
            }
        }
    }

    /// Queue an OBJ load (returns `ObjLoadCompletion` eventually).
    pub fn request_obj_load(&self, path: String, handle_id: usize) {
        // Poison-recovery: guarded state is plain (queues/sets) and remains consistent
        // even if a thread panicked while holding the lock, so recover instead of panicking.
        let mut g = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        g.obj_waiters
            .entry(path.clone())
            .or_default()
            .push(handle_id);
        if g.obj_inflight.insert(path.clone()) {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let _ = g.job_tx.send(Job::Obj { path });
            }
            #[cfg(target_arch = "wasm32")]
            {
                let result_tx = self.result_tx.clone();
                let path_clone = path.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    let result = fetch_and_decode_obj_wasm(&path_clone).await;
                    let _ = result_tx.send(WorkerMsg::Obj {
                        path: path_clone,
                        result,
                    });
                });
            }
        }
    }

    /// Run `gltf::import` off the main thread; upload with `AssetManager::load_gltf_from_import`.
    pub fn request_gltf_import(&self, path: String) -> bool {
        tracing::info!(">>> request_gltf_import çağrıldı: {}", path);
        // Poison-recovery: guarded state is plain (queues/sets) and remains consistent
        // even if a thread panicked while holding the lock, so recover instead of panicking.
        let mut g = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        if g.gltf_inflight.contains(&path) {
            tracing::info!(">>> request_gltf_import: Model zaten yükleniyor!");
            return false;
        }
        g.gltf_inflight.insert(path.clone());

        #[cfg(not(target_arch = "wasm32"))]
        {
            let ok = g.job_tx.send(Job::Gltf { path }).is_ok();
            tracing::info!(">>> request_gltf_import: İşlem gönderildi mi? {}", ok);
            ok
        }
        #[cfg(target_arch = "wasm32")]
        {
            let result_tx = self.result_tx.clone();
            let path_clone = path.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let result = fetch_and_parse_gltf_wasm(&path_clone).await;
                let _ = result_tx.send(WorkerMsg::Gltf {
                    path: path_clone,
                    result: Box::new(result),
                });
            });
            true
        }
    }

    /// Non-blocking: collect all finished jobs since the last call.
    pub fn drain_completed(&self) -> CompletedAsyncLoads {
        let mut out = CompletedAsyncLoads {
            textures: Vec::new(),
            objs: Vec::new(),
            gltfs: Vec::new(),
            gltf_errors: Vec::new(),
        };

        // Poison-recovery: guarded state is plain (queues/sets) and remains consistent
        // even if a thread panicked while holding the lock, so recover instead of panicking.
        let mut g = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        while let Ok(msg) = g.result_rx.try_recv() {
            match msg {
                WorkerMsg::Texture {
                    request_path,
                    cache_key,
                    result,
                } => {
                    g.texture_inflight.remove(&request_path);
                    let entity_ids = g.texture_waiters.remove(&request_path).unwrap_or_default();
                    if entity_ids.is_empty() {
                        continue;
                    }
                    match result {
                        Ok((rgba, width, height)) => {
                            out.textures.push(TextureReloadCompletion {
                                cache_key,
                                rgba,
                                width,
                                height,
                                entity_ids,
                            });
                        }
                        Err(e) => {
                            tracing::error!(
                                "[AsyncAssetLoader] Texture decode failed ({request_path}): {e}"
                            );
                        }
                    }
                }
                WorkerMsg::Obj { path, result } => {
                    g.obj_inflight.remove(&path);
                    let handle_ids = g.obj_waiters.remove(&path).unwrap_or_default();
                    if handle_ids.is_empty() {
                        continue;
                    }
                    match result {
                        Ok((vertices, aabb)) => {
                            out.objs.push(ObjLoadCompletion {
                                path,
                                vertices,
                                aabb,
                                handle_ids,
                            });
                        }
                        Err(e) => {
                            tracing::error!("[AsyncAssetLoader] OBJ decode failed ({path}): {e}");
                        }
                    }
                }
                WorkerMsg::Gltf { path, result } => {
                    g.gltf_inflight.remove(&path);
                    match *result {
                        Ok((document, buffers, images)) => {
                            out.gltfs.push(GltfImportCompletion {
                                path,
                                document,
                                buffers,
                                images,
                            });
                        }
                        Err(err) => {
                            out.gltf_errors.push(GltfImportError {
                                path,
                                message: err.to_string(),
                            });
                        }
                    }
                }
            }
        }

        out
    }
}

impl Default for AsyncAssetLoader {
    fn default() -> Self {
        Self::new()
    }
}

// ── WASM Fetch & Parse Helpers ──────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
async fn native_fetch_bytes(url: &str) -> Result<Vec<u8>, AssetError> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let fetch_err = |message: String| AssetError::Fetch {
        url: url.to_string(),
        message,
    };

    let window = web_sys::window().ok_or_else(|| fetch_err("no global window found".into()))?;
    let resp_value = JsFuture::from(window.fetch_with_str(url))
        .await
        .map_err(|e| fetch_err(format!("fetch failed: {e:?}")))?;
    let resp: web_sys::Response = resp_value
        .dyn_into()
        .map_err(|_| fetch_err("failed to cast to Response".into()))?;

    if !resp.ok() {
        return Err(fetch_err(format!("HTTP error status: {}", resp.status())));
    }

    let array_buffer_value = JsFuture::from(
        resp.array_buffer()
            .map_err(|e| fetch_err(format!("failed to get array buffer: {e:?}")))?,
    )
    .await
    .map_err(|e| fetch_err(format!("failed to resolve array buffer: {e:?}")))?;
    let array_buffer = js_sys::ArrayBuffer::from(array_buffer_value);
    let uint8_array = js_sys::Uint8Array::new(&array_buffer);
    let mut bytes = vec![0; uint8_array.length() as usize];
    uint8_array.copy_to(&mut bytes);
    Ok(bytes)
}

#[cfg(target_arch = "wasm32")]
async fn fetch_and_decode_texture_wasm(path: &str) -> Result<(Vec<u8>, u32, u32), AssetError> {
    let bytes = native_fetch_bytes(path).await?;

    let img = image::load_from_memory(&bytes)
        .map_err(|source| AssetError::ImageDecode {
            path: std::path::PathBuf::from(path),
            source,
        })?
        .to_rgba8();
    let (w, h) = img.dimensions();
    Ok((img.into_raw(), w, h))
}

#[cfg(target_arch = "wasm32")]
async fn fetch_and_parse_gltf_wasm(
    path: &str,
) -> Result<
    (
        gltf::Document,
        Vec<gltf::buffer::Data>,
        Vec<gltf::image::Data>,
    ),
    AssetError,
> {
    let bytes = native_fetch_bytes(path).await?;

    gltf::import_slice(&bytes).map_err(|source| AssetError::GltfImport {
        path: std::path::PathBuf::from(path),
        source,
    })
}

#[cfg(target_arch = "wasm32")]
async fn fetch_and_decode_obj_wasm(
    path: &str,
) -> Result<(Vec<crate::gpu_types::Vertex>, gizmo_math::Aabb), AssetError> {
    let bytes = native_fetch_bytes(path).await?;

    let mut reader = std::io::Cursor::new(bytes);
    let (models, _) = tobj::load_obj_buf(
        &mut reader,
        &tobj::LoadOptions {
            single_index: true,
            triangulate: true,
            ignore_points: true,
            ignore_lines: true,
        },
        |_| Err(tobj::LoadError::OpenFileFailed),
    )
    .map_err(|source| AssetError::ObjLoad {
        path: std::path::PathBuf::from(path),
        source,
    })?;

    if models.is_empty() {
        return Err(AssetError::ObjEmpty {
            path: std::path::PathBuf::from(path),
        });
    }

    let mut aabb = gizmo_math::Aabb::empty();
    let mut vertices = Vec::new();

    for model in &models {
        let m = &model.mesh;
        let has_normals = !m.normals.is_empty();
        let has_texcoords = !m.texcoords.is_empty();

        for &raw_idx in &m.indices {
            let idx = raw_idx as usize;
            let pos_base = idx * 3;
            if pos_base + 2 >= m.positions.len() {
                return Err(AssetError::ObjIndexOutOfRange {
                    path: std::path::PathBuf::from(path),
                    kind: crate::asset::ObjIndexKind::Position,
                    index: idx,
                    len: m.positions.len() / 3,
                });
            }
            let position = [
                m.positions[pos_base],
                m.positions[pos_base + 1],
                m.positions[pos_base + 2],
            ];
            aabb.extend(gizmo_math::Vec3::new(position[0], position[1], position[2]));

            let normal = if has_normals {
                let n_base = idx * 3;
                [
                    m.normals[n_base],
                    m.normals[n_base + 1],
                    m.normals[n_base + 2],
                ]
            } else {
                [0.0, 1.0, 0.0]
            };

            let tex_coords = if has_texcoords {
                let uv_base = idx * 2;
                [m.texcoords[uv_base], 1.0 - m.texcoords[uv_base + 1]]
            } else {
                [0.0, 0.0]
            };

            vertices.push(crate::renderer::Vertex {
                position,
                normal,
                tex_coords,
                color: [1.0, 1.0, 1.0],
                joint_indices: [0; 4],
                joint_weights: [0.0; 4],
                ..Default::default()
            });
        }
    }

    Ok((vertices, aabb))
}
