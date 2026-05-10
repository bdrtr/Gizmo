use crate::state::{DebugAssets, StudioState};
use gizmo::editor::EditorState;
use gizmo::physics::components::Transform;
use gizmo::prelude::*;

pub fn handle_scene_operations(
    world: &mut World,
    editor_state: &mut EditorState,
    _state: &mut StudioState,
) {
    // --- REFLECTION (JSON) GÜNCELLEMELERİ ---
    let pending_json: Vec<_> = editor_state.pending_json_updates.drain(..).collect();
    for (entity, set_json, val) in pending_json {
        if let Err(e) = set_json(world, entity, val) {
            editor_state.log_error(&format!("Reflection deserialization hatası: {}", e));
        }
    }

    // --- DİNAMİK COMPONENT EKLEME İŞLEMİ ---
    if let Some((ent_id, comp_name)) = editor_state.add_component_request.take() {
        if let Some(ent) = world.get_entity(ent_id.id()) {
            match comp_name.as_str() {
                "Transform" => world.add_component(ent, Transform::new(Vec3::ZERO)),
                "Velocity" => world.add_component(ent, gizmo::physics::Velocity::new(Vec3::ZERO)),
                "RigidBody" => {
                    world.add_component(ent, gizmo::physics::RigidBody::new(1.0, 0.5, 0.5, true))
                }
                "Collider" => world.add_component(
                    ent,
                    gizmo::physics::Collider::box_collider(gizmo::math::Vec3::new(1.0, 1.0, 1.0)),
                ),
                "Camera" => world.add_component(
                    ent,
                    gizmo::renderer::components::Camera::new(
                        60.0_f32.to_radians(),
                        0.1,
                        1000.0,
                        0.0,
                        0.0,
                        false,
                    ),
                ),
                "PointLight" => world.add_component(
                    ent,
                    gizmo::renderer::components::PointLight::new(Vec3::new(1., 1., 1.), 1.0, 10.0),
                ),
                "Material" => {
                    let white_tex = world
                        .get_resource::<DebugAssets>()
                        .map(|a| a.white_tex.clone());
                    if let Some(tex) = white_tex {
                        world.add_component(ent, gizmo::prelude::Material::new(tex));
                    }
                }
                "Script" => world
                    .add_component(ent, gizmo::scripting::Script::new("scripts/new_script.lua")),
                "ParticleEmitter" => {
                    world.add_component(ent, gizmo::renderer::components::ParticleEmitter::new())
                }
                "AudioSource" => world.add_component(
                    ent,
                    gizmo::prelude::AudioSource {
                        sound_name: "".to_string(),
                        is_3d: true,
                        max_distance: 100.0,
                        volume: 1.0,
                        pitch: 1.0,
                        loop_sound: false,
                        _internal_sink_id: None,
                    },
                ),
                "Terrain" => {
                    world.add_component(
                        ent,
                        gizmo::renderer::components::Terrain::new(
                            "demo/assets/textures/heightmap.png".to_string(),
                            100.0,
                            100.0,
                            20.0,
                        ),
                    );
                    // Request rendering mesh creation
                    editor_state.generate_terrain_requests.push(ent_id);
                }

                _ => editor_state.log_warning(&format!("Bilinmeyen component: {}", comp_name)),
            }
        }
    }

    if editor_state.scene.rebuild_navmesh_request {
        editor_state.scene.rebuild_navmesh_request = false;

        // Tetiklendiğinde gizmo-ai içindeki grid'in needs_rebuild bayrağını true yaparız
        if let Some(mut grid) = world.get_resource_mut::<gizmo::ai::pathfinding::NavGrid>() {
            grid.needs_rebuild = true;
            editor_state.log_info("🤖 NavMesh yeniden oluşturulması talep edildi...");
        } else {
            editor_state.log_warning("NavGrid bulunamadı! AI aktif mi?");
        }
    }

    if !editor_state.despawn_requests.is_empty() {
        let mut history_backup = Vec::new();
        let despawn_reqs: Vec<gizmo::prelude::Entity> =
            editor_state.despawn_requests.drain(..).collect();
        for ent_id in despawn_reqs {
            editor_state.selection.entities.remove(&ent_id);

            // 1. Parent'ın Children listesinden kendini çıkar
            {
                let parent_storage = world.borrow::<gizmo::core::component::Parent>();
                if let Some(p) = parent_storage.get(ent_id.id()) {
                    let parent_id = p.0;
                    drop(parent_storage);
                    {
                        let mut children_storage =
                            world.borrow_mut::<gizmo::core::component::Children>();
                        if let Some(c) = children_storage.get_mut(parent_id) {
                            c.0.retain(|&id| id != ent_id.id());
                        }
                    }
                }
            }

            // 2. Tüm çocuklarını topla
            let mut ids_to_delete = vec![ent_id.id()];
            {
                let children_storage = world.borrow::<gizmo::core::component::Children>();
                let mut i = 0;
                while i < ids_to_delete.len() {
                    let current = ids_to_delete[i];
                    if let Some(c) = children_storage.get(current) {
                        for &child in &c.0 {
                            ids_to_delete.push(child);
                        }
                    }
                    i += 1;
                }
            }

            // 3. Geçmişe kaydet (yalnızca kökü kaydediyoruz şimdilik) ve hepsini despawn et
            if let Some(_ent) = world.get_entity(ent_id.id()) {
                let backup = gizmo::scene::SceneData::serialize_entities(
                    world,
                    vec![ent_id.id()],
                    &gizmo::scene::SceneRegistry::default(),
                );
                if let Some(data) = backup.into_iter().next() {
                    if let Ok(bytes) = bincode::serialize(&data) {
                        history_backup.push(bytes);
                    }
                }

                for &id in &ids_to_delete {
                    if let Some(ent) = world.get_entity(id) {
                        world.despawn(ent);
                    }
                }
                editor_state.log_info(&format!(
                    "Entity {} ve {} çocuğu silindi.",
                    ent_id,
                    ids_to_delete.len() - 1
                ));
            }
        }
        if !history_backup.is_empty() {
            editor_state
                .history
                .push(gizmo::editor::history::EditorAction::EntityDespawned {
                    data: history_backup,
                });
        }
    }

    // --- YENİ ENTITY OLUŞTURMA (Küp / Küre / Boş) ---
    if let Some(kind) = editor_state.spawn_request.take() {
        let pending_assets = world
            .get_resource::<DebugAssets>()
            .map(|a| (a.cube.clone(), a.white_tex.clone()));

        if let Some((cube_mesh, white_tex)) = pending_assets {
            let e = world.spawn();
            world.add_component(e, Transform::new(Vec3::ZERO));
            world.add_component(e, gizmo::renderer::components::MeshRenderer::new());

            match kind.as_str() {
                "Cube" => {
                    world.add_component(e, gizmo::core::component::EntityName("Küp".to_string()));
                    world.add_component(e, cube_mesh);
                    world.add_component(
                        e,
                        gizmo::prelude::Material::new(white_tex).with_pbr(
                            gizmo::math::Vec4::new(0.8, 0.8, 0.8, 1.0),
                            0.5,
                            0.0,
                        ),
                    );
                    world.add_component(
                        e,
                        gizmo::physics::Collider::box_collider(gizmo::math::Vec3::new(
                            1.0, 1.0, 1.0,
                        )),
                    );
                    editor_state.log_info("Yeni küp oluşturuldu.");
                }
                "Sphere" => {
                    world.add_component(e, gizmo::core::component::EntityName("Küre".to_string()));
                    world.add_component(e, cube_mesh);
                    world.add_component(
                        e,
                        gizmo::prelude::Material::new(white_tex).with_pbr(
                            gizmo::math::Vec4::new(0.4, 0.6, 1.0, 1.0),
                            0.2,
                            0.0,
                        ),
                    );
                    world.add_component(e, gizmo::physics::Collider::sphere(1.0));
                    editor_state.log_info("Yeni küre oluşturuldu.");
                }
                _ => {
                    world.add_component(
                        e,
                        gizmo::core::component::EntityName("Boş Entity".to_string()),
                    );
                    editor_state.log_info("Boş entity oluşturuldu.");
                }
            }

            editor_state.select_exclusive(e);
            editor_state
                .history
                .push(gizmo::editor::history::EditorAction::EntitySpawned {
                    entity_ids: vec![e],
                });
        }
    }

    // --- GÖRÜNÜRLÜK AÇMA / KAPATMA ---
    let toggle_requests: Vec<_> = editor_state.toggle_visibility_requests.drain(..).collect();
    for ent_id in toggle_requests {
        if let Some(ent) = world.get_entity(ent_id.id()) {
            let currently_hidden = world
                .borrow::<gizmo::core::component::IsHidden>()
                .contains(ent_id.id());
            if currently_hidden {
                world.remove_component::<gizmo::core::component::IsHidden>(ent);
                editor_state.log_info(&format!("Entity {} görünür yapıldı.", ent_id));
            } else {
                world.add_component(ent, gizmo::core::component::IsHidden);
                editor_state.log_info(&format!("Entity {} gizlendi.", ent_id));
            }
        }
    }

    // --- PARENT DEĞİŞTİRME (Reparent) ---
    if let Some((child_id, new_parent_id)) = editor_state.reparent_request.take() {
        // Eski parent'ı O(1) maliyetle bul
        let old_parent_id = world
            .borrow::<gizmo::core::component::Parent>()
            .get(child_id.id())
            .map(|c| c.0);

        // Eski parent'ın children listesinden çıkar ve yeni parent'a ekle
        {
            let mut children_comp = world.borrow_mut::<gizmo::core::component::Children>();
            if let Some(old_pid) = old_parent_id {
                if let Some(ch) = children_comp.get_mut(old_pid) {
                    ch.0.retain(|&cid| cid != child_id.id());
                }
            }

            // Yeni parent'a ekle
            if let Some(ch) = children_comp.get_mut(new_parent_id.id()) {
                if !ch.0.contains(&child_id.id()) {
                    ch.0.push(child_id.id());
                }
            }
        }

        if let Some(child_ent) = world.get_entity(child_id.id()) {
            world.add_component(
                child_ent,
                gizmo::core::component::Parent(new_parent_id.id()),
            );
            editor_state.log_info(&format!(
                "Entity {} parent {} olarak ayarlandı.",
                child_id, new_parent_id
            ));
        }
    }

    // --- PARENT KALDIR (Root Yap) ---
    if let Some(child_id) = editor_state.unparent_request.take() {
        // Eski parent'ı O(1) maliyetle bul
        let old_parent_id = world
            .borrow::<gizmo::core::component::Parent>()
            .get(child_id.id())
            .map(|c| c.0);

        if let Some(old_pid) = old_parent_id {
            {
                let mut children_comp = world.borrow_mut::<gizmo::core::component::Children>();
                if let Some(ch) = children_comp.get_mut(old_pid) {
                    ch.0.retain(|&cid| cid != child_id.id());
                }
            }
        }

        if let Some(child_ent) = world.get_entity(child_id.id()) {
            world.remove_component::<gizmo::core::component::Parent>(child_ent);
            editor_state.log_info(&format!("Entity {} kök (root) yapıldı.", child_id));
        }
    }
}
