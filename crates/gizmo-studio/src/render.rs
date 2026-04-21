use crate::render_pipeline;
use crate::state::StudioState;
use gizmo::editor::EditorState;
use gizmo::prelude::*;

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
    let mut highlight_box_id = 0u32;

    if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
        save_req = ed.scene.save_request.take();
        load_req = ed.scene.load_request.take();
        clear_req = ed.scene.clear_request;
        ed.scene.clear_request = false;
        prefab_save_req = ed.prefab_save_request.take();
        prefab_load_req = ed.prefab_load_request.take();
        gltf_req = ed.gltf_load_request.take();
        duplicate_reqs = ed.duplicate_requests.drain(..).collect();
        highlight_box_id = ed.selection.highlight_box.map(|h| h.id()).unwrap_or(0);

        if ed.play_start_request {
            ed.play_start_request = false;
            play_start = true;
        }
        if ed.play_stop_request {
            ed.play_stop_request = false;
            play_stop = true;
        }
    }

    if let Some((path, pos)) = gltf_req {
        let mut cmds = gizmo::spawner::Commands::new(world, renderer);
        let _ = cmds.spawn_gltf(pos.unwrap_or(gizmo::math::Vec3::ZERO), &path, false);
        drop(cmds);
        
        if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
            ed.log_info(&format!("Model sahneye eklendi: {}", path));
        }
    }

    let play_backup_path = format!("{}/.play_backup.scene", std::env::var("CARGO_MANIFEST_DIR").unwrap_or(".".to_string()));

    if play_start {
        let _ = gizmo::scene::SceneData::save(world, &play_backup_path, &gizmo::scene::SceneRegistry::default());
        if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
            ed.log_info("▶ Play: Sahne yedeği alındı ve simülasyon başladı.");
        }
    }

    if play_stop {
        load_req = Some(play_backup_path.clone());
        // Eski fizikten kalan bağlantı (Joint) kalıntılarını temizle
        world.insert_resource(gizmo::physics::JointWorld::new());
        world.insert_resource(gizmo::physics::PhysicsSolverState::new());
        if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
            ed.log_info("Sahne yedeğe geri dönüldü.");
        }
    }

    if let Some(path) = save_req {
        let _ = gizmo::scene::SceneData::save(world, &path, &gizmo::scene::SceneRegistry::default());
        if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
            ed.log_info("Sahne kaydedildi.");
        }
    }

    if clear_req {
        let ents = world.iter_alive_entities();
        let mut protected_ids = std::collections::HashSet::new();
        protected_ids.insert(state.editor_camera);
        protected_ids.insert(highlight_box_id);

        { let names = world.borrow::<gizmo::core::component::EntityName>();
            for e in &ents {
                if let Some(name) = names.get(e.id()) {
                    if name.0.starts_with("Editor ") || name.0 == "Highlight Box" {
                        protected_ids.insert(e.id());
                    }
                }
            }
        }

        { let children = world.borrow::<gizmo::core::component::Children>();
            let mut i = 0;
            let mut pro_list: Vec<u32> = protected_ids.iter().copied().collect();
            while i < pro_list.len() {
                let id = pro_list[i];
                if let Some(c) = children.get(id) {
                    for &child_id in &c.0 {
                        if protected_ids.insert(child_id) {
                            pro_list.push(child_id);
                        }
                    }
                }
                i += 1;
            }
        }

        for e in ents {
            if protected_ids.contains(&e.id()) {
                continue;
            }
            world.despawn(e);
        }
        if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
            ed.clear_selection();
            ed.log_info("Sahne temizlendi. Yeni sahne hazır.");
            ed.scene_path = String::new();
        }
    }

    if let Some(path) = load_req {
        let ents = world.iter_alive_entities();

        let mut protected_ids = std::collections::HashSet::new();
        protected_ids.insert(state.editor_camera);
        protected_ids.insert(highlight_box_id);

        { let names = world.borrow::<gizmo::core::component::EntityName>();
            for e in &ents {
                if let Some(name) = names.get(e.id()) {
                    if name.0.starts_with("Editor ") || name.0 == "Highlight Box" {
                        protected_ids.insert(e.id());
                    }
                }
            }
        }

        { let children = world.borrow::<gizmo::core::component::Children>();
            let mut i = 0;
            let mut pro_list: Vec<u32> = protected_ids.iter().copied().collect();
            while i < pro_list.len() {
                let id = pro_list[i];
                if let Some(c) = children.get(id) {
                    for &child_id in &c.0 {
                        if protected_ids.insert(child_id) {
                            pro_list.push(child_id);
                        }
                    }
                }
                i += 1;
            }
        }

        for e in ents {
            if protected_ids.contains(&e.id()) {
                continue;
            }
            world.despawn(e);
        }
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
        let _ = gizmo::scene::SceneData::save_prefab(world, ent_id.id(), &path, &gizmo::scene::SceneRegistry::default());
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
                let mut transforms = world.borrow_mut::<gizmo::physics::components::Transform>(); {
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
        let time_ns = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos();
        let temp_path = format!("demo/assets/prefabs/temp_duplicate_{}_{}.prefab", ent_id, time_ns);

        let _ = gizmo::scene::SceneData::save_prefab(world, ent_id.id(), &temp_path, &gizmo::scene::SceneRegistry::default());

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

                { let terrains = world.borrow::<gizmo::renderer::components::Terrain>();
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
                        Ok((mesh, heights, w, d)) => {
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
                                    gizmo::physics::Collider {
                                        shape: gizmo::physics::shape::ColliderShape::HeightField {
                                            heights,
                                            segments_x: w,
                                            segments_z: d,
                                            width: p_width,
                                            depth: p_depth,
                                            max_height: p_max_h,
                                        },
                                    },
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
