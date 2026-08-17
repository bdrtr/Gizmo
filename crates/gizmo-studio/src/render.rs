use crate::render_pipeline;
use crate::state::StudioState;
use gizmo::editor::EditorState;
use gizmo::prelude::*;
use std::collections::HashSet;

/// Identifies the editor's own objects (camera, grid, light, highlight box) and returns the set
/// of protected ids. Used to keep them from being deleted while a scene is cleared or loaded.
fn collect_protected_ids(world: &World, editor_camera: u32) -> HashSet<u32> {
    let mut protected = HashSet::new();
    protected.insert(editor_camera);

    {
        let names = world.borrow::<gizmo::core::component::EntityName>();
        let markers = world.borrow::<gizmo::core::component::EditorOnly>();
        for e in world.iter_alive_entities() {
            if gizmo::core::component::is_editor_only(
                markers.get(e.id()).is_some(),
                names.get(e.id()).map(|n| n.0.as_str()),
            ) {
                protected.insert(e.id());
            }
        }
    }

    // BFS: Korunan objelerin tüm çocuklarını da ekle
    {
        let children = world.borrow::<gizmo::core::component::Children>();
        let mut queue: Vec<u32> = protected.iter().copied().collect();
        let mut i = 0;
        while i < queue.len() {
            let id = queue[i];
            if let Some(c) = children.get(id) {
                for &child_id in &c.0 {
                    if protected.insert(child_id) {
                        queue.push(child_id);
                    }
                }
            }
            i += 1;
        }
    }

    protected
}

/// Despawns every non-editor entity in the world.
/// The protected objects (camera, grid, lights) are left alone.
fn despawn_non_protected(world: &mut World, protected: &HashSet<u32>) {
    let ents = world.iter_alive_entities();
    for e in ents {
        if !protected.contains(&e.id()) {
            world.despawn(e);
        }
    }
}

pub fn render_studio(
    world: &mut World,
    state: &StudioState,
    encoder: &mut gizmo::wgpu::CommandEncoder,
    view: &gizmo::wgpu::TextureView,
    renderer: &mut gizmo::renderer::Renderer,
    light_time: f32,
) {
    let mut save_req = None;
    let mut clear_req = false;
    let mut load_req = None;
    let mut prefab_save_req = None;
    let mut prefab_load_req = None;
    let mut gltf_req = None;
    let mut duplicate_reqs = Vec::new();
    let mut play_start = false;
    let mut play_stop = false;

    if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
        save_req = ed.scene.save_request.take();
        load_req = ed.scene.load_request.take();
        clear_req = ed.scene.clear_request;
        ed.scene.clear_request = false;
        prefab_save_req = ed.prefab_save_request.take();
        prefab_load_req = ed.prefab_load_request.take();
        gltf_req = ed.gltf_load_request.take();
        duplicate_reqs = ed.duplicate_requests.drain(..).collect();

        if ed.play_start_request {
            ed.play_start_request = false;
            play_start = true;
        }
        if ed.play_stop_request {
            ed.play_stop_request = false;
            play_stop = true;
        }
    }

    // Yeni istekleri loader'a aktar
    if let Some((path, pos)) = gltf_req {
        tracing::debug!(path, "gltf yükleme isteği alındı");
        // The two ways this fails are told apart. They used to share one message — "Model
        // yüklenemedi veya zaten yükleniyor" — printed for a missing `AssetServer` as well, which
        // sends the reader looking at their file while the loader was never there to ask.
        let outcome = match world.get_resource::<gizmo::asset_server::AssetServer>() {
            None => GltfRequest::NoAssetServer,
            Some(asset_server) => {
                if asset_server.loader.request_gltf_import(path.clone()) {
                    GltfRequest::Started
                } else {
                    GltfRequest::AlreadyLoading
                }
            }
        };
        if outcome == GltfRequest::Started {
            if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
                ed.pending_async_gltfs.insert(path.clone(), pos.unwrap_or(gizmo::math::Vec3::ZERO));
                ed.log_info(&format!("⌛ Asenkron model yüklemesi başlatıldı: {}", path));
            }
        } else {
            if outcome == GltfRequest::NoAssetServer {
                // A real error, and it used to be logged at `info` behind a `>>>` prefix, where it
                // read as scaffolding rather than as the failure it is.
                tracing::error!(path, "AssetServer kaynağı yok — model yüklenemez");
            }
            if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
                ed.log_error(&gltf_request_message(outcome, &path));
            }
        }
    }

    // Tamamlanan GLTF asenkron yüklemeleri işle
    let mut completed_gltfs = Vec::new();
    let mut completed_errors = Vec::new();
    if let Some(mut asset_server) = world.get_resource_mut::<gizmo::asset_server::AssetServer>() {
        completed_gltfs = asset_server.completed_gltfs.drain(..).collect();
        completed_errors = asset_server.completed_gltf_errors.drain(..).collect();
    }

    for err in completed_errors {
        if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
            ed.log_error(&format!("❌ Model yüklenemedi: {} ({})", err.path, err.message));
        }
    }

    for comp in completed_gltfs {
        let pos = {
            if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
                ed.pending_async_gltfs.remove(&comp.path).unwrap_or(gizmo::math::Vec3::ZERO)
            } else {
                gizmo::math::Vec3::ZERO
            }
        };

        let path = comp.path.clone();
        let mut cmds = gizmo::spawner::Commands::new(world, renderer);
        let result = cmds.spawn_gltf_async_completed(comp, pos, false).map(|b| b.id());
        drop(cmds);

        if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
            match result {
                Ok(_) => ed.log_info(&format!("✅ Model sahneye eklendi: {}", path)),
                Err(e) => ed.log_error(&e.to_string()),
            }
        }
    }

    if play_start {
        // 1. In-memory snapshot al (hızlı yol — fizik state dahil, GPU state korunur)
        let protected_ids = collect_protected_ids(world, state.editor_camera);
        let snapshot = gizmo::scene::SceneSnapshot::capture(
            world,
            &gizmo::full_scene_registry(),
            &protected_ids,
        );

        if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
            let entity_count = snapshot.entity_count();
            ed.play_snapshot = Some(snapshot);
            ed.log_info(&format!(
                "▶ Play: {} entity in-memory snapshot alındı, simülasyon başladı.",
                entity_count
            ));
        }
    }

    if play_stop {
        let snapshot_opt = {
            if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
                ed.play_snapshot.take()
            } else {
                None
            }
        };

        if let Some(snapshot) = snapshot_opt {
            let protected_ids = collect_protected_ids(world, state.editor_camera);
            let result = snapshot.restore(
                world,
                &gizmo::full_scene_registry(),
                &protected_ids,
            );
            if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
                ed.log_info(&format!(
                    "⏹ Stop: Sahne geri yüklendi (Silinen: {}, Geri Gelen: {}, Süre: {:?})",
                    result.despawned, result.restored, result.duration
                ));
            }
        } else {
            if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
                ed.log_warning("Geri yüklenecek snapshot bulunamadı!");
            }
        }
    }

    if let Some(path) = save_req {
        // Saved WITH identity: each asset reference also records the UUID of the file it names, so
        // a later load can find that file if it has since moved. The path is written exactly as
        // before — see `gizmo::asset_identity` for what that does and does not survive.
        //
        // The manager is taken out and put back rather than borrowed, because the save reads the
        // whole world and cannot hold a resource borrow across it.
        let manager = world.remove_resource::<gizmo::renderer::asset::AssetManager>();
        let save = match &manager {
            Some(m) => gizmo::scene::SceneData::save_with_identity(
                world,
                &path,
                &gizmo::full_scene_registry(),
                &gizmo::asset_identity::ManagerIdentity(m),
            ),
            None => gizmo::scene::SceneData::save(world, &path, &gizmo::full_scene_registry()),
        };
        if let Some(m) = manager {
            world.insert_resource(m);
        }
        if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
            match save {
                Ok(_) => ed.log_info("Sahne kaydedildi."),
                Err(e) => ed.log_error(&format!("Sahne kaydedilemedi: {e}")),
            }
        }
    }

    if clear_req {
        let protected_ids = collect_protected_ids(world, state.editor_camera);
        despawn_non_protected(world, &protected_ids);
        if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
            ed.clear_selection();
            ed.log_info("Sahne temizlendi. Yeni sahne hazır.");
            ed.scene_path = String::new();
        }
    }

    if let Some(path) = load_req {
        let protected_ids = collect_protected_ids(world, state.editor_camera);
        despawn_non_protected(world, &protected_ids);
        // Loaded WITH identity: any asset reference whose path has gone stale is repointed at
        // where that asset is now, before the entities are built (the ECS components carry only
        // the path, so after instantiation there is nothing left to repair from).
        let manager = world.remove_resource::<gizmo::renderer::asset::AssetManager>();
        let load_result = match &manager {
            Some(m) => gizmo::scene::SceneData::load_into_with_identity(
                &path,
                world,
                &gizmo::full_scene_registry(),
                &gizmo::asset_identity::ManagerIdentity(m),
            ),
            None => {
                gizmo::scene::SceneData::load_into(&path, world, &gizmo::full_scene_registry())
            }
        };
        if let Some(m) = manager {
            world.insert_resource(m);
        }
        if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
            ed.clear_selection();
            match load_result {
                Ok(_) => ed.log_info("Sahne yüklendi."),
                Err(e) => ed.log_error(&format!("Sahne yüklenemedi: {}", e)),
            }
        }
    }

    if let Some((ent_id, path)) = prefab_save_req {
        // Reported both ways, like the scene load above it — this one used to drop the result and
        // say "Prefab kaydedildi." either way.
        let save = gizmo::scene::SceneData::save_prefab(
            world,
            ent_id.id(),
            &path,
            &gizmo::full_scene_registry(),
        );
        if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
            match save {
                Ok(_) => ed.log_info(&format!("Prefab kaydedildi: {}", path)),
                Err(e) => ed.log_error(&format!("Prefab kaydedilemedi ({}): {}", path, e)),
            }
        }
    }

    if let Some((path, parent, target_pos)) = prefab_load_req {
        let loaded_root = gizmo::scene::SceneData::load_prefab(
            &path,
            parent.map(|p| p.id()),
            world,
            &gizmo::full_scene_registry(),
        );

        // Prefab spawn pozisyonunu (Asset browser'dan drop edilmişse) uygula
        if let (Ok(Some(root_id)), Some(pos)) = (&loaded_root, target_pos) {
            let root_id = *root_id;
            let mut transforms = world.borrow_mut::<gizmo::physics::components::Transform>();
            {
                if let Some(mut t) = transforms.get_mut(root_id) {
                    t.position = pos;
                    t.update_local_matrix();
                }
            }
        }

        if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
            match &loaded_root {
                Ok(_) => ed.log_info("Prefab yüklendi."),
                Err(e) => ed.log_error(&format!("Prefab yüklenemedi: {}", e)),
            }
        }
    }

    for ent_id in duplicate_reqs {
        // Çakışmaları(Race condition) engellemek için temp dosyasını entity id ve zaman damgasıyla eşsiz(unique) yapıyoruz
        // Saat UNIX_EPOCH'tan geride olsa bile (nadir) panik yerine 0 kullan.
        let time_ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let temp_path = format!(
            "demo/assets/prefabs/temp_duplicate_{}_{}.prefab",
            ent_id, time_ns
        );

        // Duplicate is a round trip through a temporary prefab: save the entity out, read it back
        // in. Either half can fail, and this used to drop the save's result and announce "Obje
        // çoğaltıldı." before looking at the load's — so a duplicate that produced nothing at all
        // still reported success, and a failed save surfaced (if at all) as a confusing load
        // error about a file the user never asked for.
        let save = gizmo::scene::SceneData::save_prefab(
            world,
            ent_id.id(),
            &temp_path,
            &gizmo::full_scene_registry(),
        );
        let outcome = match save {
            Err(e) => DuplicateOutcome::SaveFailed(e.to_string()),
            Ok(_) => match gizmo::scene::SceneData::load_prefab(
                &temp_path,
                None,
                world,
                &gizmo::full_scene_registry(),
            ) {
                Ok(Some(new_id)) => DuplicateOutcome::Duplicated(new_id),
                Ok(None) => DuplicateOutcome::NothingLoaded,
                Err(e) => DuplicateOutcome::LoadFailed(e.to_string()),
            },
        };
        if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
            match &outcome {
                DuplicateOutcome::Duplicated(new_id) => {
                    ed.log_info("Obje çoğaltıldı.");
                    ed.clear_selection();
                    if let Some(new_ent) = world.get_entity(*new_id) {
                        ed.selection.entities.insert(new_ent);
                        ed.selection.primary = Some(new_ent);
                    }
                }
                other => ed.log_error(&duplicate_failure_message(other)),
            }
        }

        // İşlem biter bitmez arkamızdaki kalıntıyı diskten temizleyelim
        let _ = std::fs::remove_file(&temp_path);
    }

    let mut terrain_reqs = Vec::new();
    if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
        terrain_reqs = std::mem::take(&mut ed.generate_terrain_requests);
    }

    if !terrain_reqs.is_empty() {
        if let Some(asset_manager) = world.remove_resource::<gizmo::renderer::asset::AssetManager>()
        {
            for ent_id in terrain_reqs {
                let mut p_width = 100.0;
                let mut p_depth = 100.0;
                let mut p_max_h = 20.0;
                let mut p_path = String::new();

                {
                    let terrains = world.borrow::<gizmo::renderer::components::Terrain>();
                    if let Some(t) = terrains.get(ent_id.id()) {
                        p_width = t.width;
                        p_depth = t.depth;
                        p_max_h = t.max_height;
                        p_path = t.heightmap_path.clone();
                    }
                }

                if !p_path.is_empty() {
                    match gizmo::renderer::asset::AssetManager::create_terrain(
                        &renderer.device,
                        &p_path,
                        p_width,
                        p_depth,
                        p_max_h,
                    ) {
                        Ok((mesh, _heights, _w, _d)) => {
                            if let Some(ent) = world.get_entity(ent_id.id()) {
                                // Material yoksa beyaz default ekle
                                let has_mat = world
                                    .borrow::<gizmo::prelude::Material>()
                                    .contains(ent.id());
                                if !has_mat {
                                    let dummy_rgba = [255, 255, 255, 255];
                                    let dummy_bg = renderer.create_texture(&dummy_rgba, 1, 1);
                                    world.add_component(
                                        ent,
                                        gizmo::prelude::Material::new(std::sync::Arc::new(
                                            dummy_bg,
                                        )),
                                    );
                                }

                                world.add_component(ent, mesh);
                                world.add_component(
                                    ent,
                                    gizmo::renderer::components::MeshRenderer::new(),
                                );
                                world.add_component(
                                    ent,
                                    gizmo::physics::Collider::box_collider(gizmo::math::Vec3::new(
                                        p_width / 2.0,
                                        p_max_h / 2.0,
                                        p_depth / 2.0,
                                    )),
                                );
                                // Yerçekimi etkilemesin
                                world.add_component(ent, gizmo::physics::RigidBody::new_static());
                            }
                        }
                        Err(e) => {
                            if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
                                ed.log_error(&format!("Terrain Error: {}", e));
                            }
                        }
                    }
                }
            }
            world.insert_resource(asset_manager);
        } else {
            if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
                ed.log_error("Kritik Hata: Terrain üretilemedi. AssetManager bulunamadı!");
            }
        }
    }

    render_pipeline::execute_render_pipeline(world, state, encoder, view, renderer, light_time);
}

/// How a `Ctrl+D` duplicate ended.
///
/// Duplicating is a round trip: the entity is written to a temporary prefab and read straight back.
/// Two calls, and both used to be able to fail without the user hearing about it — the save's
/// result was discarded, and "Obje çoğaltıldı." was logged before the load's result was looked at.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DuplicateOutcome {
    Duplicated(u32),
    SaveFailed(String),
    /// The prefab was written and read, but carried no root entity — nothing appeared.
    NothingLoaded,
    LoadFailed(String),
}

/// The console line for a duplicate that did not produce an object.
///
/// Kept apart from the world-touching code so the four branches can be checked directly, and so
/// the one branch that must NOT reach it — `Duplicated` — is visible as a mistake if it ever does.
pub(crate) fn duplicate_failure_message(outcome: &DuplicateOutcome) -> String {
    match outcome {
        DuplicateOutcome::SaveFailed(e) => {
            format!("Obje çoğaltılamadı — geçici prefab yazılamadı: {e}")
        }
        DuplicateOutcome::LoadFailed(e) => {
            format!("Obje çoğaltılamadı — geçici prefab okunamadı: {e}")
        }
        DuplicateOutcome::NothingLoaded => {
            "Obje çoğaltılamadı — geçici prefab bir kök entity içermiyor.".to_string()
        }
        // Not reachable from the call site, which handles this branch itself. Spelled out rather
        // than `unreachable!()`: a wrong word in the console beats taking the editor down.
        DuplicateOutcome::Duplicated(id) => {
            format!("Obje çoğaltıldı (entity {id}) — bu satır bir hata olarak basıldı.")
        }
    }
}

#[cfg(test)]
mod save_reporting_tests {
    use super::*;

    /// Every way a duplicate can fail gets its own line, and none of them says it worked.
    #[test]
    fn a_duplicate_that_produced_nothing_does_not_report_success() {
        for outcome in [
            DuplicateOutcome::SaveFailed("disk dolu".into()),
            DuplicateOutcome::LoadFailed("bozuk prefab".into()),
            DuplicateOutcome::NothingLoaded,
        ] {
            let line = duplicate_failure_message(&outcome);
            assert!(
                line.contains("çoğaltılamadı"),
                "{outcome:?} produced {line:?}, which does not tell the user it failed"
            );
            assert!(
                !line.contains("Obje çoğaltıldı."),
                "{outcome:?} produced the success line: {line:?}"
            );
        }
    }

    /// The two halves are told apart, because they need different fixes.
    #[test]
    fn the_message_says_which_half_of_the_round_trip_failed() {
        assert!(duplicate_failure_message(&DuplicateOutcome::SaveFailed("x".into()))
            .contains("yazılamadı"));
        assert!(duplicate_failure_message(&DuplicateOutcome::LoadFailed("x".into()))
            .contains("okunamadı"));
        assert!(duplicate_failure_message(&DuplicateOutcome::SaveFailed("disk dolu".into()))
            .contains("disk dolu"), "the underlying error must survive into the message");
    }

    /// No save in the studio may announce success without looking at its result.
    ///
    /// The three sites this guards — auto-save, prefab save, duplicate — all had the same shape:
    /// `let _ = SceneData::save…(..)` followed by an unconditional "kaydedildi" line. The auto-save
    /// one is the reason this test exists rather than a comment: "💾 Auto-Save" ticking past every
    /// interval is exactly what a person relies on to believe their work is on disk.
    ///
    /// Reading the source is the only way to state "the result is not discarded" — a behavioural
    /// test cannot make a real save fail without an unwritable filesystem.
    #[test]
    fn no_save_call_discards_its_result() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        for file in ["render.rs", "systems/gc.rs"] {
            let src = std::fs::read_to_string(root.join(file)).expect("kaynak okunmalı");
            for (n, line) in src.lines().enumerate() {
                let line = line.trim();
                assert!(
                    !(line.starts_with("let _ =") && line.contains("SceneData::save")),
                    "{file}:{} discards a save's result: {line}",
                    n + 1
                );
            }
        }
    }
}

/// What happened to a request to import a glTF asynchronously.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GltfRequest {
    Started,
    /// The same path is already in flight; the loader refused a duplicate.
    AlreadyLoading,
    /// No `AssetServer` resource in the world. Nothing can be loaded at all.
    NoAssetServer,
}

/// The console line for a glTF request that did not start.
pub(crate) fn gltf_request_message(outcome: GltfRequest, path: &str) -> String {
    match outcome {
        GltfRequest::AlreadyLoading => {
            format!("⌛ Model zaten yükleniyor, istek yok sayıldı: {path}")
        }
        GltfRequest::NoAssetServer => {
            format!("❌ Model yüklenemedi: AssetServer kaynağı yok ({path})")
        }
        // Not reachable from the call site, which handles this branch itself.
        GltfRequest::Started => format!("⌛ Asenkron model yüklemesi başlatıldı: {path}"),
    }
}

#[cfg(test)]
mod gltf_request_tests {
    use super::*;

    /// The two ways an import can fail must not share one message.
    ///
    /// They did: a missing `AssetServer` and a duplicate in-flight request both produced
    /// "❌ Model yüklenemedi veya zaten yükleniyor". One of those is the user asking twice and the
    /// other is the loader not existing, and the sentence sends the reader to inspect their file
    /// in both cases.
    #[test]
    fn the_two_failures_are_told_apart() {
        let busy = gltf_request_message(GltfRequest::AlreadyLoading, "a.glb");
        let missing = gltf_request_message(GltfRequest::NoAssetServer, "a.glb");

        assert_ne!(busy, missing, "both failures still produce the same sentence");
        assert!(busy.contains("zaten yükleniyor"), "{busy}");
        assert!(missing.contains("AssetServer"), "{missing}");
        assert!(
            !missing.contains("zaten yükleniyor"),
            "a missing AssetServer is still being blamed on a duplicate request: {missing}"
        );
        for line in [&busy, &missing] {
            assert!(line.contains("a.glb"), "the path is what makes the line actionable: {line}");
        }
    }

    /// No `>>>` scaffolding is left in the workspace, at any level.
    ///
    /// There were eight, across three crates, all at `info`: personal debugging notes shipped as
    /// engine output. Two of them labelled genuine errors — "HATA - AssetServer bulunamadı!" and a
    /// closed worker channel — which meant they could not be filtered as errors and read as noise
    /// beside the rest.
    ///
    /// A source scan, because the point is that these never reach a user, and no runtime assertion
    /// can say "this line was never written".
    #[test]
    fn no_debug_scaffolding_is_left_in_the_logs() {
        let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("crates/");
        let mut found = Vec::new();
        let mut stack = vec![crates.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if path.file_name().is_some_and(|n| n == "target") {
                        continue;
                    }
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let Ok(src) = std::fs::read_to_string(&path) else { continue };
                    for (n, line) in src.lines().enumerate() {
                        // This file's own doc comments talk about the marker; only real log calls
                        // count, which always open the string right after the format macro's `(`.
                        if line.contains("!(\">>>") {
                            found.push(format!("{}:{}", path.display(), n + 1));
                        }
                    }
                }
            }
        }
        assert!(
            found.is_empty(),
            "debug scaffolding is back in the engine's log output: {found:?}"
        );
    }
}
