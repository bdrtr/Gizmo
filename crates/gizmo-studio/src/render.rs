use crate::render_pipeline;
use crate::state::StudioState;
use gizmo::editor::EditorState;
use gizmo::prelude::*;
use std::collections::HashSet;

/// Editor'ün iç nesnelerini (Kamera, Grid, Işık, Highlight Box) tanımlayıp
/// korumalı ID kümesi döndürür. Sahne temizleme ve yükleme sırasında
/// bu nesnelerin silinmesini engellemek için kullanılır.
fn collect_protected_ids(world: &World, editor_camera: u32) -> HashSet<u32> {
    let mut protected = HashSet::new();
    protected.insert(editor_camera);

    {
        let names = world.borrow::<gizmo::core::component::EntityName>();
        for e in world.iter_alive_entities() {
            if let Some(name) = names.get(e.id()) {
                if name.0.starts_with("Editor ") || name.0 == "Highlight Box" {
                    protected.insert(e.id());
                }
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

/// Dünya'daki editor-dışı entity'leri temizler (despawn).
/// Korumalı nesneler (kamera, ızgara, ışıklar) dokunulmaz.
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
        println!(">>> render.rs: gltf_load_request yakalandı: {}", path);
        let mut handled = false;
        if let Some(asset_server) = world.get_resource::<gizmo::asset_server::AssetServer>() {
            println!(">>> render.rs: AssetServer bulundu, import isteği gönderiliyor...");
            if asset_server.loader.request_gltf_import(path.clone()) {
                handled = true;
            }
        } else {
            println!(">>> render.rs: HATA - AssetServer bulunamadı!");
        }
        if handled {
            if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
                ed.pending_async_gltfs.insert(path.clone(), pos.unwrap_or(gizmo::math::Vec3::ZERO));
                ed.log_info(&format!("⌛ Asenkron model yüklemesi başlatıldı: {}", path));
            }
        } else {
            println!(">>> render.rs: HATA - İstek AssetServer tarafından reddedildi veya işlenmedi!");
            if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
                ed.log_error(&format!("❌ Model yüklenemedi veya zaten yükleniyor: {}", path));
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
            let mut ed = world.get_resource_mut::<EditorState>().unwrap();
            ed.pending_async_gltfs.remove(&comp.path).unwrap_or(gizmo::math::Vec3::ZERO)
        };

        let path = comp.path.clone();
        let mut cmds = gizmo::spawner::Commands::new(world, renderer);
        let result = cmds.spawn_gltf_async_completed(comp, pos, false).map(|b| b.id());
        drop(cmds);

        if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
            match result {
                Ok(_) => ed.log_info(&format!("✅ Model sahneye eklendi: {}", path)),
                Err(e) => ed.log_error(&e),
            }
        }
    }

    let play_backup_path = format!(
        "{}/.play_backup.scene",
        std::env::var("CARGO_MANIFEST_DIR").unwrap_or(".".to_string())
    );

    if play_start {
        // 1. In-memory snapshot al (hızlı yol — fizik state dahil)
        let protected_ids = collect_protected_ids(world, state.editor_camera);
        let snapshot = gizmo::scene::SceneSnapshot::capture(
            world,
            &gizmo::scene::SceneRegistry::default(),
            &protected_ids,
        );

        // 2. Disk yedeği de al (GPU kaynakları — Mesh, Material — için)
        let _ = gizmo::scene::SceneData::save(
            world,
            &play_backup_path,
            &gizmo::scene::SceneRegistry::default(),
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
        // In-memory snapshot varsa onu kullan, yoksa disk yedeğine düş
        let snapshot_opt = {
            if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
                ed.play_snapshot.take()
            } else {
                None
            }
        };

        if let Some(snapshot) = snapshot_opt {
            // Hızlı yol: In-memory restore (fizik bileşenleri dahil)
            let protected_ids = collect_protected_ids(world, state.editor_camera);

            let result = snapshot.restore(
                world,
                &gizmo::scene::SceneRegistry::default(),
                &protected_ids,
            );

            // GPU kaynaklarını (Mesh/Material) disk yedeğinden yükle
            if std::path::Path::new(&play_backup_path).exists() {
                if let Some(mut asset_manager) =
                    world.remove_resource::<gizmo::renderer::asset::AssetManager>()
                {
                    let dummy_rgba = [255, 255, 255, 255];
                    let dummy_bg = renderer.create_texture(&dummy_rgba, 1, 1);
                    gizmo::scene::SceneData::load_into(
                        &play_backup_path,
                        world,
                        &renderer.device,
                        &renderer.queue,
                        &renderer.scene.texture_bind_group_layout,
                        &mut asset_manager,
                        std::sync::Arc::new(dummy_bg),
                        &gizmo::scene::SceneRegistry::default(),
                    );
                    world.insert_resource(asset_manager);
                }
            }

            if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
                ed.clear_selection();
                ed.log_info(&format!("⏹ Stop: {}", result));
            }
        } else {
            // Fallback: Disk yedeğinden yükle
            load_req = Some(play_backup_path.clone());
            if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
                ed.log_info("⏹ Stop: Disk yedeğinden geri dönüldü (fallback).");
            }
        }
    }

    if let Some(path) = save_req {
        let _ =
            gizmo::scene::SceneData::save(world, &path, &gizmo::scene::SceneRegistry::default());
        if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
            ed.log_info("Sahne kaydedildi.");
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
        if let Some(mut asset_manager) =
            world.remove_resource::<gizmo::renderer::asset::AssetManager>()
        {
            let dummy_rgba = [255, 255, 255, 255];
            let dummy_bg = renderer.create_texture(&dummy_rgba, 1, 1);
            gizmo::scene::SceneData::load_into(
                &path,
                world,
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
                &mut asset_manager,
                std::sync::Arc::new(dummy_bg),
                &gizmo::scene::SceneRegistry::default(),
            );
            world.insert_resource(asset_manager);
            if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
                ed.clear_selection();
                ed.log_info("Sahne yüklendi.");
            }
        } else {
            if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
                ed.log_error("Kritik Hata: Sahne yüklenemedi. AssetManager bulunamadı!");
            }
        }
    }

    if let Some((ent_id, path)) = prefab_save_req {
        let _ = gizmo::scene::SceneData::save_prefab(
            world,
            ent_id.id(),
            &path,
            &gizmo::scene::SceneRegistry::default(),
        );
        if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
            ed.log_info("Prefab kaydedildi.");
        }
    }

    if let Some((path, parent, target_pos)) = prefab_load_req {
        if let Some(mut asset_manager) =
            world.remove_resource::<gizmo::renderer::asset::AssetManager>()
        {
            let dummy_rgba = [255, 255, 255, 255];
            let dummy_bg = renderer.create_texture(&dummy_rgba, 1, 1);
            let loaded_root = gizmo::scene::SceneData::load_prefab(
                &path,
                parent.map(|p| p.id()),
                world,
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
                &mut asset_manager,
                std::sync::Arc::new(dummy_bg),
                &gizmo::scene::SceneRegistry::default(),
            );

            // Prefab spawn pozisyonunu (Asset browser'dan drop edilmişse) uygula
            if let (Some(root_id), Some(pos)) = (loaded_root, target_pos) {
                let mut transforms = world.borrow_mut::<gizmo::physics::components::Transform>();
                {
                    if let Some(t) = transforms.get_mut(root_id) {
                        t.position = pos;
                        t.update_local_matrix();
                    }
                }
            }

            world.insert_resource(asset_manager);
            if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
                ed.log_info("Prefab yüklendi.");
            }
        } else {
            if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
                ed.log_error("Kritik Hata: Prefab yüklenemedi. AssetManager bulunamadı!");
            }
        }
    }

    for ent_id in duplicate_reqs {
        // Çakışmaları(Race condition) engellemek için temp dosyasını entity id ve zaman damgasıyla eşsiz(unique) yapıyoruz
        let time_ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let temp_path = format!(
            "demo/assets/prefabs/temp_duplicate_{}_{}.prefab",
            ent_id, time_ns
        );

        let _ = gizmo::scene::SceneData::save_prefab(
            world,
            ent_id.id(),
            &temp_path,
            &gizmo::scene::SceneRegistry::default(),
        );

        if let Some(mut asset_manager) =
            world.remove_resource::<gizmo::renderer::asset::AssetManager>()
        {
            let dummy_rgba = [255, 255, 255, 255];
            let dummy_bg = renderer.create_texture(&dummy_rgba, 1, 1);
            gizmo::scene::SceneData::load_prefab(
                &temp_path,
                None,
                world,
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
                &mut asset_manager,
                std::sync::Arc::new(dummy_bg),
                &gizmo::scene::SceneRegistry::default(),
            );
            world.insert_resource(asset_manager);
            if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
                ed.log_info("Obje çoğaltıldı.");
            }
        } else {
            if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
                ed.log_error("Kritik Hata: Obje çoğaltılamadı. AssetManager bulunamadı!");
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
