use crate::render_pipeline;
use crate::state::StudioState;
use gizmo::editor::EditorState;
use gizmo::prelude::*;
use std::collections::HashSet;

/// Identifies the editor's own objects (camera, grid, light, highlight box) and returns the set
/// of protected ids. Used to keep them from being deleted while a scene is cleared or loaded.
pub(crate) fn collect_protected_ids(world: &World, editor_camera: Entity) -> std::collections::HashSet<Entity> {
    let mut protected = HashSet::new();
    protected.insert(editor_camera);

    {
        let names = world.query::<&gizmo::core::component::EntityName>();
        let markers = world.query::<&gizmo::core::component::EditorOnly>();
        for e in world.iter_alive_entities() {
            if gizmo::core::component::is_editor_only(
                markers.get(e).is_some(),
                names.get(e).map(|n| n.0.as_str()),
            ) {
                protected.insert(e);
            }
        }
    }

    // BFS: Korunan objelerin tüm çocuklarını da ekle
    {
        let children = world.query::<&gizmo::core::component::Children>();
        let mut queue: Vec<Entity> = protected.iter().copied().collect();
        let mut i = 0;
        while i < queue.len() {
            let id = queue[i];
            if let Some(c) = children.get(id) {
                for &child in &c.0 {
                    if world.is_alive(child) && protected.insert(child) {
                        queue.push(child);
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
fn despawn_non_protected(world: &mut World, protected: &HashSet<Entity>) {
    let ents = world.iter_alive_entities();
    for e in ents {
        if !protected.contains(&e) {
            world.despawn(e);
        }
    }
}

/// Renders one studio frame: the scene viewport, the game viewport and the editor's own
/// objects, into the render targets the panels display.
pub fn render_studio(
    world: &mut World,
    state: &StudioState,
    encoder: &mut gizmo::wgpu::CommandEncoder,
    view: &gizmo::wgpu::TextureView,
    renderer: &mut gizmo::renderer::Renderer,
    light_time: f32,
) {
    // Shader files changed since the last frame: the update asked, and this is where the renderer
    // is (see `request_shader_rebuild`).
    if crate::systems::simulation::take_shader_rebuild(world) {
        renderer.rebuild_shaders();
        if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
            ed.log_info("🔥 Shader'lar yeniden derlendi");
        }
    }

    // ▶ and ⏹ are the update hook's (`systems::simulation`, `crate::pie`): it swaps the worlds, and
    // the render hook draws whichever is current. What is left here is the game's setup, which needs
    // the renderer.
    let FileRequests {
        save: save_req,
        clear: clear_req,
        load: load_req,
        prefab_save: prefab_save_req,
        prefab_load: prefab_load_req,
        gltf: gltf_req,
        duplicates: duplicate_reqs,
    } = take_file_requests(world, state.session.is_some());

    run_game_setup(world, state, renderer);

    // Yeni istekleri loader'a aktar
    if let Some((path, pos)) = gltf_req {
        request_gltf(world, path, pos);
    }
    spawn_completed_gltfs(world, renderer);

    // One registry for every save, load, snapshot and duplicate below: the engine's built-ins
    // plus whatever the game registered into the world with `register_editor_component`. With
    // the engine's alone, a game's own component was edited in the inspector and then left out
    // of the saved file, the play snapshot (so Stop did not restore it) and a duplicate.
    let scene_registry = gizmo::scene_registry_for(world);

    if let Some(path) = save_req {
        save_scene(world, path, &scene_registry);
    }
    if clear_req {
        clear_scene(world, state.editor_camera);
    }
    if let Some(path) = load_req {
        load_scene(world, state.editor_camera, path, &scene_registry);
    }
    if let Some((ent_id, path)) = prefab_save_req {
        save_prefab(world, ent_id, path, &scene_registry);
    }
    if let Some((path, parent, target_pos)) = prefab_load_req {
        load_prefab(world, path, parent, target_pos, &scene_registry);
    }
    for ent_id in duplicate_reqs {
        duplicate_entity(world, ent_id, &scene_registry);
    }
    generate_requested_terrains(world, renderer);

    // `generate_requested_terrains` answers "an edit asked for a terrain"; the frame's preparation
    // (`prepare_frame`, inside the pipeline) answers "an entity HAS a terrain recipe and no mesh",
    // which is the case a queue of edits cannot see — a loaded scene — and builds cloth meshes, as
    // the game's own frame does.
    render_pipeline::execute_render_pipeline(world, state, encoder, view, renderer, light_time);
}

/// Runs `op` with the asset manager's identity, which scene and prefab files record beside each
/// asset path (`gizmo::asset_identity`), or with none when there is no asset manager. The manager
/// is taken out of the world for the call and put back after: a save or a load reads the whole
/// world and cannot hold a resource borrow across it.
fn with_asset_identity<R>(
    world: &mut World,
    op: impl FnOnce(&mut World, Option<&gizmo::asset_identity::ManagerIdentity>) -> R,
) -> R {
    let manager = world.remove_resource::<gizmo::renderer::asset::AssetManager>();
    let result = op(world, manager.as_ref().map(gizmo::asset_identity::ManagerIdentity).as_ref());
    if let Some(m) = manager {
        world.insert_resource(m);
    }
    result
}

/// The file requests the editor's panels and shortcuts queued this frame, taken out of its state.
#[derive(Default)]
struct FileRequests {
    save: Option<String>,
    clear: bool,
    load: Option<String>,
    prefab_save: Option<(Entity, String)>,
    prefab_load: Option<(String, Option<Entity>, Option<gizmo::math::Vec3>)>,
    gltf: Option<(String, Option<gizmo::math::Vec3>)>,
    duplicates: Vec<Entity>,
}

/// Takes this frame's file requests out of the editor's state. Under ▶ (`playing`) a scene is not
/// saved, opened or cleared: those are the edited world's, and the current world is the play world.
/// Saving there wrote the running game — whatever it had moved, spawned and destroyed — over the
/// scene file, and a scene opened or cleared there was thrown away at ⏹. The requests are refused
/// with a warning, whichever control made them (the shortcut, the menu, a dialog); prefabs and
/// duplicates go on, being made from what is in front of you.
fn take_file_requests(world: &mut World, playing: bool) -> FileRequests {
    let mut requests = FileRequests::default();
    if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
        requests.save = ed.scene.save_request.take();
        requests.load = ed.scene.load_request.take();
        requests.clear = std::mem::take(&mut ed.scene.clear_request);
        requests.prefab_save = ed.prefab_save_request.take();
        requests.prefab_load = ed.prefab_load_request.take();
        requests.gltf = ed.gltf_load_request.take();
        requests.duplicates = std::mem::take(&mut ed.duplicate_requests);
        if playing && (requests.save.is_some() || requests.load.is_some() || requests.clear) {
            requests.save = None;
            requests.load = None;
            requests.clear = false;
            let message = "Play sırasında sahne kaydedilmez, açılmaz ya da temizlenmez — önce ⏹ ile durdurun.";
            ed.log_warning(message);
            ed.status_message = message.to_string();
        }
    }
    requests
}

/// The game's setup on a play world ▶ has just built (`crate::pie::GameSetupPending`), and from
/// then on what it sets on the renderer each frame.
fn run_game_setup(world: &mut World, state: &StudioState, renderer: &mut gizmo::renderer::Renderer) {
    if let Some(game) = state.game.as_ref().filter(|_| state.session.is_some()) {
        if world.remove_resource::<crate::pie::GameSetupPending>().is_some() {
            (game.setup)(world, renderer);
        }
        if let Some(settings) = game.render_settings.as_ref() {
            settings(world, renderer);
        }
    }
}

/// Hands a model the asset browser asked for to the asynchronous loader.
fn request_gltf(world: &mut World, path: String, pos: Option<gizmo::math::Vec3>) {
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

/// Spawns the models the asynchronous loader finished, and reports the ones it could not load.
fn spawn_completed_gltfs(world: &mut World, renderer: &mut gizmo::renderer::Renderer) {
    let mut completed_gltfs = Vec::new();
    let mut completed_errors = Vec::new();
    if let Some(mut asset_server) = world.get_resource_mut::<gizmo::asset_server::AssetServer>() {
        completed_gltfs = std::mem::take(&mut asset_server.completed_gltfs);
        completed_errors = std::mem::take(&mut asset_server.completed_gltf_errors);
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
}

/// Writes `world` to `path` as a scene file, through `registry` and with the asset identities the
/// world's `AssetManager` knows. The one writer of a scene: the editor's save and the export's
/// staging (`systems::build`) both, so the scene a game is exported with is the scene the editor
/// saves.
pub(crate) fn write_scene(world: &mut World, path: &str, registry: &gizmo::scene::registry::SceneRegistry) -> Result<(), String> {
    with_asset_identity(world, |world, identity| match identity {
        Some(identity) => gizmo::scene::SceneData::save_with_identity(world, path, registry, identity),
        None => gizmo::scene::SceneData::save(world, path, registry),
    })
    .map_err(|e| e.to_string())
}

/// Saves the scene to `path`, WITH identity: each asset reference also records the UUID of the
/// file it names, so a later load can find that file if it has since moved. The path is written
/// exactly as before — see `gizmo::asset_identity` for what that does and does not survive.
fn save_scene(world: &mut World, path: String, scene_registry: &gizmo::scene::registry::SceneRegistry) {
    warn_undescribed_meshes(world, "Kaydetme");
    let save = write_scene(world, &path, scene_registry);
    if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
        match save {
            Ok(_) => {
                // The other half of the unsaved-changes guard. `has_unsaved_changes` is raised
                // by the animation panel and this path never lowered it, so one dragged
                // keyframe left the flag true for the rest of the session — and every
                // single-click scene load in the asset browser went through the confirmation
                // route from then on. `gizmo-app`'s editor runtime already clears it here.
                ed.has_unsaved_changes = false;
                ed.log_info("Sahne kaydedildi.");
            }
            Err(e) => ed.log_error(&format!("Sahne kaydedilemedi: {e}")),
        }
    }
}

/// Clears the scene down to the editor's own objects.
fn clear_scene(world: &mut World, editor_camera: Entity) {
    let protected_ids = collect_protected_ids(world, editor_camera);
    despawn_non_protected(world, &protected_ids);
    if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
        ed.clear_selection();
        ed.log_info("Sahne temizlendi. Yeni sahne hazır.");
        ed.scene_path = String::new();
    }
}

/// Loads the scene at `path` in place of the current one, WITH identity: any asset reference whose
/// path has gone stale is repointed at where that asset is now, before the entities are built (the
/// ECS components carry only the path, so after instantiation there is nothing left to repair from).
fn load_scene(world: &mut World, editor_camera: Entity, path: String, scene_registry: &gizmo::scene::registry::SceneRegistry) {
    let protected_ids = collect_protected_ids(world, editor_camera);
    despawn_non_protected(world, &protected_ids);
    let load_result = with_asset_identity(world, |world, identity| match identity {
        Some(identity) => gizmo::scene::SceneData::load_into_with_identity(&path, world, scene_registry, identity),
        None => gizmo::scene::SceneData::load_into(&path, world, scene_registry),
    });
    if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
        ed.clear_selection();
        match load_result {
            Ok(_) => ed.log_info("Sahne yüklendi."),
            Err(e) => ed.log_error(&format!("Sahne yüklenemedi: {}", e)),
        }
    }
}

/// Saves `ent_id` and its children as a prefab at `path`, and says how that went either way. WITH
/// identity, like the scene save: a prefab is the reusable unit, so a model that moves breaks every
/// instance of it in every scene — the case the UUID fallback exists for.
fn save_prefab(world: &mut World, ent_id: Entity, path: String, scene_registry: &gizmo::scene::registry::SceneRegistry) {
    let save = with_asset_identity(world, |world, identity| match identity {
        Some(identity) => gizmo::scene::SceneData::save_prefab_with_identity(world, ent_id, &path, scene_registry, identity),
        None => gizmo::scene::SceneData::save_prefab(world, ent_id, &path, scene_registry),
    });
    if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
        match save {
            Ok(_) => ed.log_info(&format!("Prefab kaydedildi: {}", path)),
            Err(e) => ed.log_error(&format!("Prefab kaydedilemedi ({}): {}", path, e)),
        }
    }
}

/// Instantiates the prefab at `path` under `parent`, at `target_pos` when it was dropped from the
/// asset browser. WITH identity: a prefab whose model has moved since it was saved is repointed
/// before the entities are built — after instantiation the components carry only the path and
/// there is nothing left to repair from.
fn load_prefab(
    world: &mut World,
    path: String,
    parent: Option<Entity>,
    target_pos: Option<gizmo::math::Vec3>,
    scene_registry: &gizmo::scene::registry::SceneRegistry,
) {
    let loaded_root = with_asset_identity(world, |world, identity| match identity {
        Some(identity) => gizmo::scene::SceneData::load_prefab_with_identity(&path, parent, world, scene_registry, identity),
        None => gizmo::scene::SceneData::load_prefab(&path, parent, world, scene_registry),
    });

    // Prefab spawn pozisyonunu (Asset browser'dan drop edilmişse) uygula
    if let (Ok(Some(root_id)), Some(pos)) = (&loaded_root, target_pos) {
        let root_id = *root_id;
        let mut transforms = world.query_mut::<&mut gizmo::physics::components::Transform>();
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

/// Where a duplicate of `ent_id` goes through: the system's temporary directory, named for this
/// process, the entity and the moment. It was `demo/assets/prefabs/`, relative to wherever the
/// studio was started: a project's own assets folder had a file appear and vanish in it on every
/// Ctrl+D, and the directory was created from scratch under any other working directory.
///
/// Named from the entity's NUMBERS, not its `Display` (`Entity(3:0)`): a colon in an NTFS file name
/// is an alternate data stream, so on Windows the save wrote a stream of a file called
/// `…_Entity(3`, the removal deleted only the stream, and the file stayed in the temp folder.
fn duplicate_temp_path(ent_id: Entity, time_ns: u32) -> String {
    std::env::temp_dir()
        .join(format!(
            "gizmo_duplicate_{}_{}v{}_{}.prefab",
            std::process::id(),
            ent_id.id(),
            ent_id.generation(),
            time_ns
        ))
        .to_string_lossy()
        .into_owned()
}

/// `Ctrl+D`: duplicates `ent_id` by a round trip through a temporary prefab, and selects the copy.
fn duplicate_entity(world: &mut World, ent_id: Entity, scene_registry: &gizmo::scene::registry::SceneRegistry) {
    // Çakışmaları(Race condition) engellemek için temp dosyasını entity id ve zaman damgasıyla eşsiz(unique) yapıyoruz
    // Saat UNIX_EPOCH'tan geride olsa bile (nadir) panik yerine 0 kullan.
    let time_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let temp_path = duplicate_temp_path(ent_id, time_ns);

    // Duplicate is a round trip through a temporary prefab: save the entity out, read it back
    // in. Either half can fail, and this used to drop the save's result and announce "Obje
    // çoğaltıldı." before looking at the load's — so a duplicate that produced nothing at all
    // still reported success, and a failed save surfaced (if at all) as a confusing load
    // error about a file the user never asked for.
    let save = gizmo::scene::SceneData::save_prefab(
        world,
        ent_id,
        &temp_path,
        scene_registry,
    );
    let outcome = match save {
        Err(e) => DuplicateOutcome::SaveFailed(e.to_string()),
        Ok(_) => match gizmo::scene::SceneData::load_prefab(
            &temp_path,
            None,
            world,
            scene_registry,
        ) {
            Ok(Some(new_id)) => DuplicateOutcome::Duplicated(new_id.id()),
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

/// Builds the meshes and colliders of the terrains an edit asked for (`generate_terrain_requests`).
fn generate_requested_terrains(world: &mut World, renderer: &mut gizmo::renderer::Renderer) {
    let mut terrain_reqs = Vec::new();
    if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
        terrain_reqs = std::mem::take(&mut ed.generate_terrain_requests);
    }

    if terrain_reqs.is_empty() {
        return;
    }
    let Some(asset_manager) = world.remove_resource::<gizmo::renderer::asset::AssetManager>() else {
        if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
            ed.log_error("Kritik Hata: Terrain üretilemedi. AssetManager bulunamadı!");
        }
        return;
    };
    for ent_id in terrain_reqs {
        generate_terrain(world, renderer, ent_id);
    }
    world.insert_resource(asset_manager);
}

/// One terrain's mesh from its heightmap, with a white material when it has none, its collider and
/// a static body.
fn generate_terrain(world: &mut World, renderer: &mut gizmo::renderer::Renderer, ent_id: Entity) {
    let mut p_width = 100.0;
    let mut p_depth = 100.0;
    let mut p_max_h = 20.0;
    let mut p_path = String::new();

    {
        let terrains = world.query::<&gizmo::renderer::components::Terrain>();
        if let Some(t) = terrains.get(ent_id) {
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
            Ok((mesh, heights, cols, rows)) => {
                if let Some(ent) = world.live(ent_id) {
                    // Material yoksa beyaz default ekle
                    let has_mat = world
                        .query::<&gizmo::prelude::Material>()
                        .contains(ent);
                    if !has_mat {
                        let dummy_rgba = [255, 255, 255, 255];
                        let dummy_bg = renderer.create_texture(&dummy_rgba, 1, 1);
                        world.add_component(
                            ent,
                            gizmo::prelude::Material::new(dummy_bg),
                        );
                    }

                    world.add_component(ent, mesh);
                    world.add_component(
                        ent,
                        gizmo::renderer::components::MeshRenderer::new(),
                    );
                    // The same collider the game's frame builds (`terrain_mesh_system`).
                    world.add_component(
                        ent,
                        gizmo::systems::terrain_collider(heights, cols, rows, p_width, p_depth, p_max_h),
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

/// Says, by name, which entities a save or the Play snapshot will bring back without their geometry:
/// a mesh built straight onto the GPU from points, with neither a `MeshSource` nor the `MeshData`
/// it was made from (`gizmo::renderer::mesh_sync::undescribed_meshes`). Silent loss is what this
/// stops; the fix is the game's, and the message says what it is.
fn warn_undescribed_meshes(world: &mut World, when: &str) {
    let lost = gizmo::renderer::mesh_sync::undescribed_meshes(world);
    if lost.is_empty() {
        return;
    }
    let names: Vec<String> = {
        let names = world.query::<&gizmo::core::EntityName>();
        lost.iter().take(5).map(|e| names.get(*e).map(|n| n.0.clone()).unwrap_or_else(|| format!("#{}", e.id()))).collect()
    };
    let more = lost.len().saturating_sub(names.len());
    let message = format!(
        "{when}: {} nesnenin geometrisi sahneye yazılamıyor ({}{}) — GPU'ya noktalardan kurulmuş bir ağ; MeshData ile yapılırsa kaydedilir.",
        lost.len(),
        names.join(", "),
        if more > 0 { format!(" ve {more} tane daha") } else { String::new() },
    );
    tracing::warn!(count = lost.len(), "{message}");
    if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
        ed.log_warning(&message);
    }
}

#[cfg(test)]
mod file_request_tests {
    //! The requests `render_studio` serves without a GPU — scene save and load, prefab save and
    //! load, duplicate — driven directly, as the editor's panels and shortcuts queue them.
    use super::*;
    use gizmo::core::component::EntityName;
    use gizmo::physics::components::Transform;

    fn named(world: &mut World, name: &str, at: gizmo::math::Vec3) -> Entity {
        let e = world.spawn();
        world.add_component(e, EntityName(name.to_string()));
        world.add_component(e, Transform::new(at));
        e
    }

    /// Every entity's name, sorted, duplicates kept.
    fn names(world: &World) -> Vec<String> {
        let mut out: Vec<String> = world.query::<&EntityName>().iter().map(|n| n.0.clone()).collect();
        out.sort();
        out
    }

    fn temp_file(name: &str) -> String {
        std::env::temp_dir()
            .join(format!("gizmo_studio_test_{}_{name}", std::process::id()))
            .to_string_lossy()
            .into_owned()
    }

    fn editor_world() -> World {
        let mut world = World::new();
        world.insert_resource(EditorState::default());
        world.insert_resource(gizmo::renderer::asset::AssetManager::new());
        world
    }

    /// Under ▶ a scene is not saved, opened or cleared: the current world is the play world, and
    /// saving it wrote the running game over the scene file. Prefabs still are.
    #[test]
    fn a_scene_is_not_saved_opened_or_cleared_in_play() {
        let mut world = editor_world();
        let queue = |world: &mut World| {
            let mut ed = world.get_resource_mut::<EditorState>().unwrap();
            ed.scene.save_request = Some("levels/forest.gizmo".into());
            ed.scene.load_request = Some("levels/cave.gizmo".into());
            ed.scene.clear_request = true;
            ed.prefab_save_request = Some((Entity::new(3, 0), "door.prefab".into()));
        };
        queue(&mut world);
        let playing = take_file_requests(&mut world, true);
        assert_eq!((playing.save, playing.load, playing.clear), (None, None, false));
        assert!(playing.prefab_save.is_some(), "a prefab is made from what is in front of you");
        let ed = world.get_resource::<EditorState>().unwrap();
        assert!(ed.scene.save_request.is_none() && !ed.scene.clear_request, "refused, not kept for after ⏹");
        drop(ed);

        queue(&mut world);
        let editing = take_file_requests(&mut world, false);
        assert_eq!(editing.save.as_deref(), Some("levels/forest.gizmo"));
        assert_eq!(editing.load.as_deref(), Some("levels/cave.gizmo"));
        assert!(editing.clear);
    }

    /// A scene saved and loaded back is the scene, the editor's camera survives the load, and the
    /// asset manager — taken out of the world for each call (`with_asset_identity`) — is back in it
    /// after both.
    #[test]
    fn a_saved_scene_loads_back_and_keeps_the_asset_manager() {
        let mut world = editor_world();
        world.get_resource_mut::<EditorState>().unwrap().has_unsaved_changes = true;
        let camera = world.spawn();
        named(&mut world, "Player", gizmo::math::Vec3::new(1.0, 0.0, 0.0));
        named(&mut world, "Ground", gizmo::math::Vec3::ZERO);
        let registry = gizmo::scene_registry_for(&world);
        let path = temp_file("round_trip.scene");

        save_scene(&mut world, path.clone(), &registry);
        assert!(world.get_resource::<gizmo::renderer::asset::AssetManager>().is_some(), "the save did not put the asset manager back");
        assert!(!world.get_resource::<EditorState>().unwrap().has_unsaved_changes, "a save that worked leaves nothing unsaved");

        named(&mut world, "Stray", gizmo::math::Vec3::ZERO);
        load_scene(&mut world, camera, path.clone(), &registry);
        let _ = std::fs::remove_file(&path);
        assert!(world.get_resource::<gizmo::renderer::asset::AssetManager>().is_some(), "the load did not put the asset manager back");
        assert_eq!(names(&world), ["Ground", "Player"], "the load is the saved scene and nothing else");
        assert!(world.is_alive(camera), "the editor's camera is protected from a load");
    }

    /// A prefab dropped from the asset browser lands at the drop, and the one it was saved from
    /// stays where it was.
    #[test]
    fn a_dropped_prefab_lands_where_it_was_dropped() {
        let mut world = editor_world();
        let at = gizmo::math::Vec3::new(1.0, 2.0, 3.0);
        let lamp = named(&mut world, "Lamp", at);
        let registry = gizmo::scene_registry_for(&world);
        let path = temp_file("lamp.prefab");

        save_prefab(&mut world, lamp, path.clone(), &registry);
        let drop = gizmo::math::Vec3::new(10.0, 0.0, -4.0);
        load_prefab(&mut world, path.clone(), None, Some(drop), &registry);
        let _ = std::fs::remove_file(&path);

        assert!(world.get_resource::<gizmo::renderer::asset::AssetManager>().is_some());
        assert_eq!(names(&world), ["Lamp", "Lamp"]);
        let positions: Vec<(Entity, gizmo::math::Vec3)> =
            world.query::<(Entity, &Transform)>().iter().map(|(e, t)| (e, t.position)).collect();
        assert!(positions.iter().any(|&(e, p)| e == lamp && p == at), "the original moved: {positions:?}");
        assert!(positions.iter().any(|&(e, p)| e != lamp && p == drop), "the instance is not at the drop: {positions:?}");
    }

    /// The duplicate's temporary file name is one every file system takes: nothing Windows reserves.
    /// It held `Entity(3:0)` once, and the colon made it an NTFS stream the removal did not reach.
    #[test]
    fn the_duplicate_temp_file_name_is_valid_on_windows() {
        let path = duplicate_temp_path(Entity::new(3, 7), 123);
        let name = std::path::Path::new(&path).file_name().unwrap().to_string_lossy().into_owned();
        assert!(!name.chars().any(|c| "<>:\"/\\|?*".contains(c)), "a character Windows refuses in {name:?}");
        assert!(name.contains("3v7"), "the entity is not in the name: {name:?}");
    }

    /// Ctrl+D: the copy exists and is selected, and the temporary prefab it went through is gone —
    /// from the system's temporary directory, and never written into a project folder.
    #[test]
    fn a_duplicate_selects_its_copy_and_leaves_no_file() {
        let project_folder = std::path::Path::new("demo/assets/prefabs");
        let folder_was_there = project_folder.exists();
        let mut world = editor_world();
        let original = named(&mut world, "Crate", gizmo::math::Vec3::ZERO);
        let registry = gizmo::scene_registry_for(&world);

        duplicate_entity(&mut world, original, &registry);

        assert_eq!(names(&world), ["Crate", "Crate"]);
        let primary = world.get_resource::<EditorState>().unwrap().selection.primary;
        assert!(primary.is_some_and(|e| e != original), "the copy is not the selection: {primary:?}");
        let prefix = format!("gizmo_duplicate_{}_", std::process::id());
        let left: Vec<_> = std::fs::read_dir(std::env::temp_dir())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|f| f.file_name().to_string_lossy().starts_with(&prefix))
            .collect();
        assert!(left.is_empty(), "the temporary prefab was left behind: {left:?}");
        if !folder_was_there {
            assert!(!project_folder.exists(), "a duplicate created a project folder under the working directory");
        }
    }
}
