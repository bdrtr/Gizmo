//! Background-thread decoding for textures, OBJ, and GLTF **import** (disk + parse).
//! GPU upload and [`AssetManager`](crate::asset::AssetManager) updates must run on the main thread
//! — call [`AsyncAssetLoader::drain_completed`] each frame and then upload via `AssetManager`.

use crate::asset::{decode_obj_vertices_for_async, decode_rgba_image_file};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;

#[derive(Debug)]
pub struct TextureReloadCompletion {
    pub cache_key: String,
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub entity_ids: Vec<u32>,
}

#[derive(Debug)]
pub struct ObjLoadCompletion {
    pub path: String,
    pub vertices: Vec<crate::gpu_types::Vertex>,
    pub aabb: gizmo_math::Aabb,
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

#[derive(Debug)]
pub struct CompletedAsyncLoads {
    pub textures: Vec<TextureReloadCompletion>,
    pub objs: Vec<ObjLoadCompletion>,
    pub gltfs: Vec<GltfImportCompletion>,
    pub gltf_errors: Vec<GltfImportError>,
}

enum Job {
    Texture { request_path: String },
    Obj { path: String },
    Gltf { path: String },
}

enum WorkerMsg {
    Texture {
        request_path: String,
        cache_key: String,
        result: Result<(Vec<u8>, u32, u32), String>,
    },
    Obj {
        path: String,
        result: Result<(Vec<crate::gpu_types::Vertex>, gizmo_math::Aabb), String>,
    },
    Gltf {
        path: String,
        result: Result<(gltf::Document, Vec<gltf::buffer::Data>, Vec<gltf::image::Data>), String>,
    },
}

struct LoaderShared {
    job_tx: SyncSender<Job>,
    result_rx: Receiver<WorkerMsg>,
    /// Original request path (as passed to `request_texture_reload`) → entities
    texture_waiters: HashMap<String, Vec<u32>>,
    texture_inflight: HashSet<String>,
    obj_inflight: HashSet<String>,
    gltf_inflight: HashSet<String>,
}

/// Thread-safe loader; safe to store as an ECS resource (`Send` + `Sync`).
pub struct AsyncAssetLoader {
    shared: Arc<Mutex<LoaderShared>>,
    _worker: thread::JoinHandle<()>,
}

impl AsyncAssetLoader {
    pub fn new() -> Self {
        let (job_tx, job_rx) = mpsc::sync_channel::<Job>(64);
        let (result_tx, result_rx) = mpsc::channel::<WorkerMsg>();

        let worker_job_rx = job_rx;
        let worker_result_tx = result_tx.clone();
        let _worker = thread::Builder::new()
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
                            let result = gltf::import(&path).map_err(|e| e.to_string());
                            let _ = worker_result_tx.send(WorkerMsg::Gltf { path, result });
                        }
                    }
                }
            })
            .expect("spawn async asset worker");

        Self {
            shared: Arc::new(Mutex::new(LoaderShared {
                job_tx,
                result_rx,
                texture_waiters: HashMap::new(),
                texture_inflight: HashSet::new(),
                obj_inflight: HashSet::new(),
                gltf_inflight: HashSet::new(),
            })),
            _worker,
        }
    }

    /// Queue a texture file decode; when done, [`drain_completed`] yields a row with `entity_ids`.
    /// Duplicate `request_path` while in-flight only adds more waiters (one disk read).
    pub fn request_texture_reload(&self, request_path: String, entity_id: u32) {
        let mut g = self.shared.lock().expect("async asset mutex");
        g.texture_waiters
            .entry(request_path.clone())
            .or_default()
            .push(entity_id);
        if g.texture_inflight.insert(request_path.clone()) {
            let _ = g.job_tx.send(Job::Texture { request_path });
        }
    }

    /// Decode OBJ on the worker; complete with [`AssetManager::install_obj_mesh`](crate::asset::AssetManager::install_obj_mesh).
    pub fn request_obj_load(&self, path: String) -> bool {
        let mut g = self.shared.lock().expect("async asset mutex");
        if g.obj_inflight.contains(&path) {
            return false;
        }
        g.obj_inflight.insert(path.clone());
        g.job_tx.send(Job::Obj { path }).is_ok()
    }

    /// Run `gltf::import` off the main thread; upload with `AssetManager::load_gltf_from_import`.
    pub fn request_gltf_import(&self, path: String) -> bool {
        let mut g = self.shared.lock().expect("async asset mutex");
        if g.gltf_inflight.contains(&path) {
            return false;
        }
        g.gltf_inflight.insert(path.clone());
        g.job_tx.send(Job::Gltf { path }).is_ok()
    }

    /// Non-blocking: collect all finished jobs since the last call.
    pub fn drain_completed(&self) -> CompletedAsyncLoads {
        let mut out = CompletedAsyncLoads {
            textures: Vec::new(),
            objs: Vec::new(),
            gltfs: Vec::new(),
            gltf_errors: Vec::new(),
        };

        let mut g = self.shared.lock().expect("async asset mutex");
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
                            eprintln!("[AsyncAssetLoader] Texture decode failed ({request_path}): {e}");
                        }
                    }
                }
                WorkerMsg::Obj { path, result } => {
                    g.obj_inflight.remove(&path);
                    match result {
                        Ok((vertices, aabb)) => {
                            out.objs.push(ObjLoadCompletion {
                                path,
                                vertices,
                                aabb,
                            });
                        }
                        Err(e) => {
                            eprintln!("[AsyncAssetLoader] OBJ decode failed ({path}): {e}");
                        }
                    }
                }
                WorkerMsg::Gltf { path, result } => {
                    g.gltf_inflight.remove(&path);
                    match result {
                        Ok((document, buffers, images)) => {
                            out.gltfs.push(GltfImportCompletion {
                                path,
                                document,
                                buffers,
                                images,
                            });
                        }
                        Err(message) => {
                            out.gltf_errors.push(GltfImportError { path, message });
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
