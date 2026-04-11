use gizmo::prelude::*;
use gizmo::editor::EditorState;
use crate::state::StudioState;
use crate::render_pipeline;

pub fn render_studio(
    world: &mut World,
    state: &StudioState,
    encoder: &mut gizmo::wgpu::CommandEncoder,
    view: &gizmo::wgpu::TextureView,
    renderer: &mut gizmo::renderer::Renderer,
    light_time: f32
) {
    let mut save_req = None;
    let mut load_req = None;
    let mut prefab_save_req = None;
    let mut prefab_load_req = None;
    let mut duplicate_req = None;
    let mut play_start = false;
    let mut play_stop = false;
    let mut highlight_box_id = 0u32;

    if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
        save_req = ed.scene_save_request.take();
        load_req = ed.scene_load_request.take();
        prefab_save_req = ed.prefab_save_request.take();
        prefab_load_req = ed.prefab_load_request.take();
        duplicate_req = ed.duplicate_request.take();
        highlight_box_id = ed.highlight_box;
        
        if ed.play_start_request {
            ed.play_start_request = false;
            play_start = true;
        }
        if ed.play_stop_request {
            ed.play_stop_request = false;
            play_stop = true;
        }
    }
    
    let play_backup_path = "demo/assets/scenes/.gizmo_play_backup";

    if play_start {
        gizmo::scene::SceneData::save(world, play_backup_path);
        if let Some(mut ed) = world.get_resource_mut::<EditorState>() { ed.log_info("▶ Play: Sahne yedeği alındı ve simülasyon başladı."); }
    }
    
    if play_stop {
        load_req = Some(play_backup_path.to_string());
    }

    if let Some(path) = save_req {
        gizmo::scene::SceneData::save(world, &path);
        if let Some(mut ed) = world.get_resource_mut::<EditorState>() { ed.log_info("Sahne kaydedildi."); }
    }

    if let Some(path) = load_req {
        let ents = world.alive_entities();
        for e in ents {
            // Editor kamerası, highlight box ve grid'i koru
            let eid = e.id();
            if eid == state.editor_camera || eid == highlight_box_id { continue ; }
            // Editor Guidelines (grid) — isimle kontrol et
            let is_editor_only = world.borrow::<gizmo::core::component::EntityName>()
                .and_then(|names| names.get(eid).map(|n| n.0.clone()))
                .map(|name| name == "Editor Guidelines" || name == "Highlight Box")
                .unwrap_or(false);
            if is_editor_only { continue; }
            world.despawn(e);
        }
        if let Some(mut asset_manager) = world.remove_resource::<gizmo::renderer::asset::AssetManager>() {
            let dummy_rgba = [255, 255, 255, 255];
            let dummy_bg = renderer.create_texture(&dummy_rgba, 1, 1);
            gizmo::scene::SceneData::load_into(
                &path,
                world,
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
                &mut asset_manager,
                std::sync::Arc::new(dummy_bg)
            );
            world.insert_resource(asset_manager);
            // Highlight box'ı sıfırla (artık geçerli konumda)
            if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
                ed.clear_selection();
                ed.log_info("Sahne yüklendi.");
            }
        }
    }
    
    if let Some((ent_id, path)) = prefab_save_req {
        gizmo::scene::SceneData::save_prefab(world, ent_id, &path);
        if let Some(mut ed) = world.get_resource_mut::<EditorState>() { ed.log_info("Prefab kaydedildi."); }
    }
    
    if let Some((path, parent, target_pos)) = prefab_load_req {
        if let Some(mut asset_manager) = world.remove_resource::<gizmo::renderer::asset::AssetManager>() {
            let dummy_rgba = [255, 255, 255, 255];
            let dummy_bg = renderer.create_texture(&dummy_rgba, 1, 1);
            let loaded_root = gizmo::scene::SceneData::load_prefab(
                &path,
                parent,
                world,
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
                &mut asset_manager,
                std::sync::Arc::new(dummy_bg)
            );
            
            // Prefab spawn pozisyonunu (Asset browser'dan drop edilmişse) uygula
            if let (Some(root_id), Some(pos)) = (loaded_root, target_pos) {
                if let Some(mut transforms) = world.borrow_mut::<gizmo::physics::components::Transform>() {
                    if let Some(t) = transforms.get_mut(root_id) {
                        t.position = pos;
                        t.update_local_matrix();
                    }
                }
            }
            
            world.insert_resource(asset_manager);
            if let Some(mut ed) = world.get_resource_mut::<EditorState>() { ed.log_info("Prefab yüklendi."); }
        }
    }
    
    if let Some(ent_id) = duplicate_req {
         let temp_path = "demo/assets/prefabs/temp_duplicate.prefab";
         gizmo::scene::SceneData::save_prefab(world, ent_id, temp_path);
         
         if let Some(mut asset_manager) = world.remove_resource::<gizmo::renderer::asset::AssetManager>() {
            let dummy_rgba = [255, 255, 255, 255];
            let dummy_bg = renderer.create_texture(&dummy_rgba, 1, 1);
            gizmo::scene::SceneData::load_prefab(
                temp_path,
                None,
                world,
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
                &mut asset_manager,
                std::sync::Arc::new(dummy_bg)
            );
            world.insert_resource(asset_manager);
            if let Some(mut ed) = world.get_resource_mut::<EditorState>() { ed.log_info("Obje çoğaltıldı."); }
        }
    }

    let mut terrain_reqs = Vec::new();
    if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
        terrain_reqs = std::mem::take(&mut ed.generate_terrain_requests);
    }
    
    if !terrain_reqs.is_empty() {
        if let Some(asset_manager) = world.remove_resource::<gizmo::renderer::asset::AssetManager>() {
            for ent_id in terrain_reqs {
                let mut p_width = 100.0;
                let mut p_depth = 100.0;
                let mut p_max_h = 20.0;
                let mut p_path = String::new();
                
                if let Some(terrains) = world.borrow::<gizmo::renderer::components::Terrain>() {
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
                        Ok((mesh, heights, w, d)) => {
                            if let Some(ent) = world.get_entity(ent_id) {
                                // Material yoksa beyaz default ekle
                                let has_mat = world.borrow::<gizmo::prelude::Material>().map(|m| m.contains(ent.id())).unwrap_or(false);
                                if !has_mat {
                                    let dummy_rgba = [255, 255, 255, 255];
                                    let dummy_bg = renderer.create_texture(&dummy_rgba, 1, 1);
                                    world.add_component(ent, gizmo::prelude::Material::new(std::sync::Arc::new(dummy_bg)));
                                }

                                world.add_component(ent, mesh);
                                world.add_component(ent, gizmo::renderer::components::MeshRenderer::new());
                                world.add_component(ent, gizmo::physics::Collider {
                                    shape: gizmo::physics::shape::ColliderShape::HeightField {
                                        heights,
                                        segments_x: w,
                                        segments_z: d,
                                        width: p_width,
                                        depth: p_depth,
                                        max_height: p_max_h,
                                    }
                                });
                                // Yerçekimi etkilemesin
                                world.add_component(ent, gizmo::physics::RigidBody::new_static());
                            }
                        },
                        Err(e) => {
                            if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
                                ed.log_error(&format!("Terrain Error: {}", e));
                            }
                        }
                    }
                }
            }
            world.insert_resource(asset_manager);
        }
    }

    render_pipeline::execute_render_pipeline(world, state, encoder, view, renderer, light_time);
}
