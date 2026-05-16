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

                "Hitbox" => world.add_component(ent, gizmo::physics::components::Hitbox::default()),
                "Hurtbox" => world.add_component(ent, gizmo::physics::components::Hurtbox::default()),
                "BoneAttachment" => world.add_component(ent, gizmo::renderer::components::BoneAttachment::default()),
                "FighterController" => world.add_component(ent, gizmo::physics::components::fighter::FighterController::default()),

                _ => editor_state.log_warning(&format!("Bilinmeyen component: {}", comp_name)),
            }
        }
    }

    if let Some((ent_id, comp_name)) = editor_state.remove_component_request.take() {
        if let Some(ent) = world.get_entity(ent_id.id()) {
            match comp_name.as_str() {
                "Hitbox" => { world.remove_component::<gizmo::physics::components::Hitbox>(ent); }
                "Hurtbox" => { world.remove_component::<gizmo::physics::components::Hurtbox>(ent); }
                "BoneAttachment" => { world.remove_component::<gizmo::renderer::components::BoneAttachment>(ent); }
                _ => editor_state.log_warning(&format!("Component turu silinemiyor: {}", comp_name)),
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
        let mut soft_deleted_entities = Vec::new();
        let despawn_reqs: Vec<gizmo::prelude::Entity> =
            editor_state.despawn_requests.drain(..).collect();
        for ent_id in despawn_reqs {
            editor_state.selection.entities.remove(&ent_id);

            // Korumalı objelerin (Editor Kamera, Grid, Işık) silinmesini engelle
            let mut is_protected = false;
            if let Some(name) = world.borrow::<gizmo::core::component::EntityName>().get(ent_id.id()) {
                if name.0.starts_with("Editor ")
                    || name.0 == "Directional Light"
                    || name.0 == "Highlight Box"
                {
                    is_protected = true;
                }
            }

            if is_protected {
                editor_state.log_warning(&format!("Entity {} korumalı bir objedir ve silinemez.", ent_id.id()));
                continue;
            }

            // 1. Tüm çocuklarını topla (kendisi dahil)
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

            // 2. Etiketleri ekle (Soft Delete)
            for &id in &ids_to_delete {
                if let Some(ent) = world.get_entity(id) {
                    world.add_component(ent, gizmo::core::component::IsDeleted);
                    world.add_component(ent, gizmo::core::component::IsHidden);
                    soft_deleted_entities.push(ent);
                }
            }
            editor_state.log_info(&format!(
                "Entity {} ve {} çocuğu silindi (Soft Delete).",
                ent_id,
                ids_to_delete.len() - 1
            ));
        }

        if !soft_deleted_entities.is_empty() {
            editor_state
                .history
                .push(gizmo::editor::history::EditorAction::EntityDespawned {
                    entity_ids: soft_deleted_entities,
                });
        }
    }

    // --- YENİ ENTITY OLUŞTURMA (Küp / Küre / Boş) ---
    if let Some(kind) = editor_state.spawn_request.take() {
        let pending_assets = world
            .get_resource::<DebugAssets>()
            .map(|a| (a.cube.clone(), a.white_tex.clone()));

        if let Some((cube_mesh, white_tex)) = pending_assets {
            let sphere_mesh = world
                .get_resource::<DebugAssets>()
                .map(|a| a.sphere.clone());
            let e = world.spawn();
            world.add_component(e, Transform::new(Vec3::ZERO));
            world.add_component(e, gizmo::physics::components::GlobalTransform::default());
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
                    world.add_component(e, sphere_mesh.clone().unwrap_or(cube_mesh.clone()));
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

            // === Çocuk Entity olarak bağla (pending_child_parent) ===
            if let Some(parent_entity) = editor_state.pending_child_parent.take() {
                // Parent → Children listesine ekle
                {
                    let mut children_comp = world.borrow_mut::<gizmo::core::component::Children>();
                    if let Some(ch) = children_comp.get_mut(parent_entity.id()) {
                        if !ch.0.contains(&e.id()) {
                            ch.0.push(e.id());
                        }
                    } else {
                        drop(children_comp);
                        world.add_component(
                            parent_entity,
                            gizmo::core::component::Children(vec![e.id()]),
                        );
                    }
                }
                // Child → Parent bileşenini ayarla
                world.add_component(e, gizmo::core::component::Parent(parent_entity.id()));

                // İsmi parent'a göre güncelle
                let parent_name = world
                    .borrow::<gizmo::core::component::EntityName>()
                    .get(parent_entity.id())
                    .map(|n| n.0.clone())
                    .unwrap_or_default();

                editor_state.log_info(&format!(
                    "Entity, '{}' altına çocuk olarak eklendi.",
                    parent_name
                ));
            }

            // === Otomatik bileşen ekleme (pending_child_components) ===
            let pending_components: Vec<String> = editor_state.pending_child_components.drain(..).collect();
            for comp_name in &pending_components {
                match comp_name.as_str() {
                    "Hitbox" => {
                        world.add_component(e, gizmo::physics::components::Hitbox::default());
                        // İsmi güncelle
                        if let Some(ent) = world.get_entity(e.id()) {
                            world.add_component(
                                ent,
                                gizmo::core::component::EntityName("Hitbox".to_string()),
                            );
                        }
                        editor_state.log_info("🥊 Hitbox bileşeni eklendi.");
                    }
                    "Hurtbox" => {
                        world.add_component(e, gizmo::physics::components::Hurtbox::default());
                        if let Some(ent) = world.get_entity(e.id()) {
                            world.add_component(
                                ent,
                                gizmo::core::component::EntityName("Hurtbox".to_string()),
                            );
                        }
                        editor_state.log_info("🛡 Hurtbox bileşeni eklendi.");
                    }
                    _ => {
                        editor_state.add_component_request = Some((e, comp_name.clone()));
                    }
                }
            }
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
            } else {
                // Yeni parent'ın henüz Children component'i yok → oluştur
                drop(children_comp);
                if let Some(parent_ent) = world.get_entity(new_parent_id.id()) {
                    world.add_component(
                        parent_ent,
                        gizmo::core::component::Children(vec![child_id.id()]),
                    );
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
