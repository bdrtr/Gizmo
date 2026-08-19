use crate::state::{DebugAssets, StudioState};
use gizmo::core::HierarchyExt;
use gizmo::editor::EditorState;
use gizmo::physics::components::Transform;
use gizmo::prelude::*;

/// Carries out the scene operations the UI raised this frame: spawning a primitive, deleting or
/// duplicating a selection, saving and loading.
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
                    world.add_component(ent, gizmo::physics::RigidBody::new(1.0, true))
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
                // `LightRole::Sun` because a directional light a user asks for IS the scene's sun:
                // the alternative role exists for fill lights an author places deliberately, and
                // choosing it here would give a light that lights nothing shadow-wise and looks
                // broken. A scene with two suns is authorable now — it already was, via Ctrl+D —
                // and `shared.rs` picks one; that is a lighting question, not an add-menu one.
                "DirectionalLight" => world.add_component(
                    ent,
                    gizmo::renderer::components::DirectionalLight::new(
                        Vec3::new(1.0, 0.95, 0.9),
                        3.0,
                        gizmo::renderer::components::LightRole::Sun,
                    ),
                ),
                // 25°/35° half-angles: a readable cone rather than a pinprick or a hemisphere.
                // The constructor clamps inner to outer, which is also what the inspector's two
                // sliders enforce — an inner cone wider than its outer one inverts the falloff.
                "SpotLight" => world.add_component(
                    ent,
                    gizmo::renderer::components::SpotLight::new(
                        Vec3::new(1.0, 1.0, 1.0),
                        5.0,
                        20.0,
                        25.0_f32.to_radians(),
                        35.0_f32.to_radians(),
                    ),
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
                    // `AudioSource::new("")` already yields these exact defaults
                    // (is_3d=true, max_distance=100.0, volume=1.0, pitch=1.0,
                    // loop_sound=false, _internal_sink_id=None); switched from an
                    // exhaustive struct literal because AudioSource is now
                    // #[non_exhaustive] and can no longer be built cross-crate.
                    gizmo::prelude::AudioSource::new(""),
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
                // Not `default()`: that is always player 1, so adding two fighters gave two
                // player 1s — and the studio's own fight HUD needs a p1 AND a p2 before it draws
                // anything (`simulation.rs`, `fight_hud.active`). Authoring a versus scene the
                // obvious way therefore produced a HUD that never appeared. The slot is the
                // lowest positive number no fighter in the scene is using.
                "FighterController" => world.add_component(
                    ent,
                    gizmo::physics::components::fighter::FighterController::new(
                        next_free_player_slot(world),
                    ),
                ),

                // The inspector has drawn an "AI NavAgent" section since the component existed,
                // and there was no way to put one on an entity: not in the ➕ menu, not here, and
                // not in the scene format either. `NavAgent::default()` is the human-scale walker
                // (5 m/s, 10 m/s², 0.5 m) — tuning is what the inspector is for.
                "NavAgent" => world.add_component(ent, gizmo::ai::components::NavAgent::default()),

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
                // Four inspector sections draw a 🗑 button; this arm was the one missing, so that
                // button logged "Component turu silinemiyor" at the user and left the component
                // in place. `every_delete_button_in_the_inspector_removes_its_component` below
                // now pins all four together rather than each on its own.
                "FighterController" => { world.remove_component::<gizmo::physics::components::fighter::FighterController>(ent); }
                "NavAgent" => { world.remove_component::<gizmo::ai::components::NavAgent>(ent); }
                _ => editor_state.log_warning(&format!("Component turu silinemiyor: {}", comp_name)),
            }
        }
    }

    if editor_state.scene.rebuild_navmesh_request {
        editor_state.scene.rebuild_navmesh_request = false;

        // Tetiklendiğinde gizmo-ai içindeki grid'in needs_rebuild bayrağını true yaparız.
        //
        // Izgara yoksa ARTIK KURULUYOR: eskiden bu dal yalnız "NavGrid bulunamadı! AI aktif mi?"
        // diye uyarıyordu ve doğru cevap "hayır, hiçbir yerde" idi — motorda bir NavGrid'i
        // kuran tek bir satır yoktu. Sahnede ajan olmadan da kurulur; düğmeye basmak niyetin
        // kendisi, `ensure_nav_grid`'in ajan koşulu ise otomatik yol için.
        let existing = world
            .get_resource_mut::<gizmo::ai::pathfinding::NavGrid>()
            .map(|mut grid| {
                grid.needs_rebuild = true;
            })
            .is_some();
        if existing {
            editor_state.log_info("🤖 NavMesh yeniden oluşturulması talep edildi...");
        } else {
            let grid = match world.get_resource::<gizmo::physics::world::PhysicsWorld>() {
                Some(physics) => gizmo::ai::pathfinding::NavGrid::fitted_to(
                    gizmo::ai::pathfinding::NavGrid::DEFAULT_CELL_SIZE,
                    &physics,
                ),
                None => gizmo::ai::pathfinding::NavGrid::centred_on(
                    gizmo::ai::pathfinding::NavGrid::DEFAULT_CELL_SIZE,
                    gizmo::ai::pathfinding::NavGrid::FALLBACK_CELLS,
                    gizmo::ai::pathfinding::NavGrid::FALLBACK_CELLS,
                    gizmo::math::Vec3::ZERO,
                ),
            };
            editor_state.log_info(&format!(
                "🤖 NavGrid yoktu — statik geometriye göre kuruldu ({}×{} hücre, {} m)",
                grid.width, grid.height, grid.cell_size
            ));
            world.insert_resource(grid);
        }
    }

    if !editor_state.despawn_requests.is_empty() {
        let mut soft_deleted_entities = Vec::new();
        let despawn_reqs: Vec<gizmo::prelude::Entity> =
            editor_state.despawn_requests.drain(..).collect();
        for ent_id in despawn_reqs {
            editor_state.selection.entities.remove(&ent_id);

            // Korumalı objelerin (Editor Kamera, Grid, Işık) silinmesini engelle
            // Krom kararı ortak; "Directional Light" ise krom DEĞİL — sahnenin güneşi, ama
            // varsayılan sahnenin silinmesini istemediğimiz bir parçası. İkisi ayrı gerekçe,
            // o yüzden ayrı satırda duruyorlar.
            let is_protected = {
                let names = world.borrow::<gizmo::core::component::EntityName>();
                let markers = world.borrow::<gizmo::core::component::EditorOnly>();
                let name = names.get(ent_id.id()).map(|n| n.0.clone());
                gizmo::core::component::is_editor_only(
                    markers.get(ent_id.id()).is_some(),
                    name.as_deref(),
                ) || name.as_deref() == Some("Directional Light")
            };

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
        // What the `➕` menu offers and what this understood had drifted a long way apart. The menu
        // has nine entries; this match had two arms and a catch-all, so `Group`, `Plane`,
        // `Cylinder`, `Capsule`, `PointLight`, `Camera` and `ParticleEmitter` all fell through and
        // produced an entity called "Boş Entity" carrying a `MeshRenderer` and no mesh — seven menu
        // items promising seven different things and making the same wrong one.
        //
        // The channel is a `SpawnKind` now rather than a `String`, and this match is **exhaustive**
        // on it: a variant nobody teaches this about does not compile. See the type's own note for
        // why it is deliberately not `#[non_exhaustive]`.
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::ZERO));
        world.add_component(e, gizmo::physics::components::GlobalTransform::default());

        // Meshes come from `DebugAssets`, which needs a GPU. The kinds that do NOT draw — an empty,
        // a group, a light, a camera, an emitter — must not be gated behind it: the whole spawn
        // block used to sit inside `if let Some(assets)`, so with no renderer resource every menu
        // entry did nothing at all, silently.
        let meshes = world.get_resource::<DebugAssets>().map(|a| {
            (
                a.cube.clone(),
                a.sphere.clone(),
                a.plane.clone(),
                a.cylinder.clone(),
                a.capsule.clone(),
                a.white_tex.clone(),
            )
        });

        use crate::state::PrimitiveSize as PS;
        use gizmo::editor::SpawnKind;

        // `Some(())` = built. `None` = a drawing kind asked for before the GPU assets exist.
        let built: Option<()> = match kind {
            SpawnKind::Cube => meshes.clone().map(|(cube, _, _, _, _, tex)| {
                world.add_component(e, cube);
                world.add_component(
                    e,
                    gizmo::prelude::Material::new(tex).with_pbr(
                        gizmo::math::Vec4::new(0.8, 0.8, 0.8, 1.0),
                        0.5,
                        0.0,
                    ),
                );
                world.add_component(
                    e,
                    gizmo::physics::Collider::box_collider(gizmo::math::Vec3::splat(PS::CUBE_HALF)),
                );
            }),
            SpawnKind::Sphere => meshes.clone().map(|(_, sphere, _, _, _, tex)| {
                world.add_component(e, sphere);
                world.add_component(
                    e,
                    gizmo::prelude::Material::new(tex).with_pbr(
                        gizmo::math::Vec4::new(0.4, 0.6, 1.0, 1.0),
                        0.2,
                        0.0,
                    ),
                );
                // Was `sphere(1.0)` against a mesh of radius 0.5: the collision shape was twice
                // the size of the thing you could see, so a dropped sphere came to rest half a
                // radius above the floor. Both numbers are `PS::SPHERE_RADIUS` now.
                world.add_component(e, gizmo::physics::Collider::sphere(PS::SPHERE_RADIUS));
            }),
            SpawnKind::Plane => meshes.clone().map(|(_, _, plane, _, _, tex)| {
                world.add_component(e, plane);
                world.add_component(
                    e,
                    gizmo::prelude::Material::new(tex).with_pbr(
                        gizmo::math::Vec4::new(0.7, 0.7, 0.7, 1.0),
                        0.8,
                        0.0,
                    ),
                );
                // Pushed down by its own half-thickness, so the collider's TOP face is exactly the
                // visible quad instead of hovering above it.
                world.add_component(
                    e,
                    gizmo::physics::Collider::offset_box(
                        PS::plane_collider_offset(),
                        PS::plane_collider_half_extents(),
                    ),
                );
            }),
            SpawnKind::Cylinder => meshes.clone().map(|(_, _, _, cylinder, _, tex)| {
                world.add_component(e, cylinder);
                world.add_component(
                    e,
                    gizmo::prelude::Material::new(tex).with_pbr(
                        gizmo::math::Vec4::new(0.9, 0.7, 0.3, 1.0),
                        0.4,
                        0.0,
                    ),
                );
                // The real cylinder shape, since 2026-08-17. Until then this was the convex hull
                // of the mesh's own ring points — a 24-sided prism standing in for a circle,
                // because a capsule would have rounded the ends off and a box would have squared
                // them. The hull was faithful to the mesh's silhouette and still wrong in two
                // ways the shape fixes: its inertia came from an AABB rather than from
                // `½·m·r²`, and every contact resolved against a facet, so a cylinder on its
                // side settled into whichever flat it happened to land on.
                world.add_component(
                    e,
                    gizmo::physics::Collider::cylinder(
                        PS::CYLINDER_RADIUS,
                        PS::cylinder_collider_half_height(),
                    ),
                );
            }),
            SpawnKind::Capsule => meshes.clone().map(|(_, _, _, _, capsule, tex)| {
                world.add_component(e, capsule);
                world.add_component(
                    e,
                    gizmo::prelude::Material::new(tex).with_pbr(
                        gizmo::math::Vec4::new(0.5, 0.9, 0.6, 1.0),
                        0.3,
                        0.0,
                    ),
                );
                // The mesh takes the cylindrical section's WHOLE length; the collider takes half
                // of it. That conversion is the one place these two disagree by construction.
                world.add_component(
                    e,
                    gizmo::physics::Collider::capsule(
                        PS::CAPSULE_RADIUS,
                        PS::capsule_collider_half_height(),
                    ),
                );
            }),

            // --- The kinds that draw nothing: no mesh, no material, no `MeshRenderer`. ---
            SpawnKind::PointLight => {
                world.add_component(
                    e,
                    gizmo::renderer::components::PointLight::new(
                        gizmo::math::Vec3::new(1.0, 0.95, 0.85),
                        20.0,
                        10.0,
                    ),
                );
                Some(())
            }
            SpawnKind::Camera => {
                world.add_component(
                    e,
                    gizmo::renderer::components::Camera::new(
                        60.0_f32.to_radians(),
                        0.1,
                        500.0,
                        0.0,
                        0.0,
                        // NOT primary: adding a camera to a scene must not hijack the view.
                        false,
                    ),
                );
                Some(())
            }
            SpawnKind::ParticleEmitter => {
                world.add_component(e, gizmo::renderer::components::ParticleEmitter::new());
                Some(())
            }
            // A group is an empty entity with a different name, and that is the honest whole of
            // it — `Parent`/`Children` already carry what "folder" means.
            SpawnKind::Group | SpawnKind::Empty => Some(()),
        };

        if built.is_none() {
            // A drawing kind was asked for before the GPU assets existed. Leave nothing behind: a
            // half-built entity in the scene is worse than none, and keeping it is exactly what
            // the old catch-all did.
            editor_state.log_error(&format!(
                "'{}' oluşturulamadı: mesh varlıkları henüz hazır değil.",
                kind.entity_name()
            ));
            world.despawn_by_id(e.id());
            editor_state.pending_child_parent = None;
            editor_state.pending_group_members.clear();
            editor_state.pending_child_components.clear();
        }

        if built.is_some() {
            let name = kind.entity_name();
            if kind.draws() {
                world.add_component(e, gizmo::renderer::components::MeshRenderer::new());
            }
            world.add_component(e, gizmo::core::component::EntityName(name.to_string()));
            editor_state.log_info(&format!("{name} oluşturuldu."));

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
                    if let Some(mut ch) = children_comp.get_mut(parent_entity.id()) {
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

            // === Seçilenleri bu yeni entity'nin altına al (pending_group_members) ===
            //
            // "📂 Seçilileri Grupla" asked for a `Group` and stopped there — it created a stray
            // empty entity and left the selection exactly where it was. Its own comment said
            // "sonra seçili objeleri ona bağla"; nothing ever did. This is that second half.
            //
            // `add_child` is the core call the drag-reparent already goes through, so grouping
            // gets the same cycle refusal and the same both-sides bookkeeping. It cannot cycle
            // here anyway — the parent was spawned a few lines ago and can be nobody's ancestor —
            // but going through a second implementation is how the first cycle bug happened.
            let members: Vec<_> = editor_state.pending_group_members.drain(..).collect();
            let mut grouped = 0usize;
            for member in members {
                if member.id() == e.id() {
                    continue;
                }
                if let (Some(child), Some(parent)) =
                    (world.get_entity(member.id()), world.get_entity(e.id()))
                {
                    world.add_child(parent, child);
                    grouped += 1;
                }
            }
            if grouped > 0 {
                editor_state.log_info(&format!("{grouped} nesne '{name}' altına alındı."));
                // The group is the thing you now want to move, so it is what stays selected.
                editor_state.select_exclusive(e);
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
    //
    // The link itself is `HierarchyExt::add_child`, not a copy of it. This block used to
    // re-implement the whole thing — the same cycle rejection, the same removal from the old
    // parent's `Children`, the same create-the-list-if-missing — forty lines beside a core method
    // that already did it and has tests for the cycle case. Two implementations of "make B a child
    // of A", one of which has a hang in its history (a `Children` cycle wedges transform
    // propagation, `despawn_recursive` and scene save), is exactly the shape worth removing.
    //
    // What stays here is what the editor adds: telling the user WHY a drag was refused. `add_child`
    // returns silently, so the condition is tested here for the message and then the core call
    // does the work.
    if let Some((child_id, new_parent_id)) = editor_state.reparent_request.take() {
        let would_cycle = new_parent_id.id() == child_id.id()
            || world.is_ancestor(child_id.id(), new_parent_id.id());
        if would_cycle {
            editor_state.log_info(&format!(
                "Reparent reddedildi: {child_id} → {new_parent_id} bir hiyerarşi döngüsü oluştururdu."
            ));
        } else if let (Some(child), Some(parent)) = (
            world.get_entity(child_id.id()),
            world.get_entity(new_parent_id.id()),
        ) {
            world.add_child(parent, child);
            editor_state.log_info(&format!(
                "Entity {} parent {} olarak ayarlandı.",
                child_id, new_parent_id
            ));
        }
    }

    // --- PARENT KALDIR (Root Yap) ---
    //
    // Likewise `HierarchyExt::remove_child`, which drops the `Parent` component and takes the id
    // out of the old parent's `Children` — the two halves that have to happen together or the
    // hierarchy is left describing itself two different ways.
    if let Some(child_id) = editor_state.unparent_request.take() {
        let old_parent_id = world
            .borrow::<gizmo::core::component::Parent>()
            .get(child_id.id())
            .map(|c| c.0);

        if let (Some(old_pid), Some(child)) = (old_parent_id, world.get_entity(child_id.id())) {
            if let Some(old_parent) = world.get_entity(old_pid) {
                world.remove_child(old_parent, child);
                editor_state.log_info(&format!("Entity {} kök (root) yapıldı.", child_id));
            }
        }
    }
}

/// The lowest player slot (`1`, then `2`, …) no `FighterController` in the world is holding.
///
/// Saturates at `u8::MAX` rather than wrapping back onto slot 0 — a scene with 255 fighters is
/// nonsense, and handing out slot 0 twice would be worse than handing out 255 twice.
fn next_free_player_slot(world: &World) -> u8 {
    let taken: Vec<u8> = world
        .borrow::<gizmo::physics::components::fighter::FighterController>()
        .iter()
        .map(|(_, f)| f.player_id)
        .collect();
    let mut slot: u8 = 1;
    while taken.contains(&slot) && slot < u8::MAX {
        slot += 1;
    }
    slot
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo::core::component::{Children, Parent};

    /// The editor's request fields are the only input this system has, so the tests drive it the
    /// way the UI does: set a request, run the system, look at the world.
    ///
    /// What the cycle tests below pin is that the EDITOR'S PATH refuses cycles — not that this
    /// file is what refuses them. Since the reparent became a call to `HierarchyExt::add_child`,
    /// the refusal lives there (and `gizmo-core` tests it directly); the check that remains here
    /// exists to tell the user why the drag was rejected, and that message goes to the global
    /// logger, which is shared between tests and not worth asserting on. Removing the local check
    /// therefore does not fail these tests — verified — and it should not: they are asking whether
    /// the editor can produce a cycle, and the answer stays no either way.
    fn studio_state() -> StudioState {
        StudioState {
            current_fps: 0.0,
            actual_dt: 0.016,
            editor_camera: 0,
            game_camera: 0,
            do_raycast: false,
            play: gizmo::systems::PlayLoop::new(),
            asset_watcher: None,
            gc_timer: 0.0,
            autosave_timer: 0.0,
            visible_entity_count: 0,
            draw_call_count: 0,
            }
    }


    // ── The ➕ menu ──────────────────────────────────────────────────────────────────────────
    //
    // These run with **no** `DebugAssets` in the world, which is the point: the kinds that draw
    // nothing must not be gated behind a GPU. The whole spawn block used to live inside
    // `if let Some(assets) = ...`, so in a world without a renderer every menu entry did nothing,
    // and said nothing about it either.

    use gizmo::editor::SpawnKind;

    fn name_of(world: &World, id: u32) -> Option<String> {
        world
            .borrow::<gizmo::core::component::EntityName>()
            .get(id)
            .map(|n| n.0.clone())
    }

    fn spawn_via_menu(kind: SpawnKind) -> (World, EditorState, Vec<u32>) {
        let mut world = World::new();
        let before: Vec<u32> = world.iter_alive_entities().iter().map(|e| e.id()).collect();
        let mut ed = EditorState::default();
        ed.spawn_request = Some(kind);
        handle_scene_operations(&mut world, &mut ed, &mut studio_state());
        let new: Vec<u32> = world
            .iter_alive_entities()
            .iter()
            .map(|e| e.id())
            .filter(|id| !before.contains(id))
            .collect();
        (world, ed, new)
    }

    /// Each non-drawing kind produces exactly one entity, under its own name.
    ///
    /// Before this, `Group`, `PointLight`, `Camera` and `ParticleEmitter` all hit a catch-all and
    /// came out as "Boş Entity" — four menu entries, one result, no complaint.
    #[test]
    fn each_kind_that_draws_nothing_spawns_itself_and_says_so() {
        for kind in [
            SpawnKind::Empty,
            SpawnKind::Group,
            SpawnKind::PointLight,
            SpawnKind::Camera,
            SpawnKind::ParticleEmitter,
        ] {
            let (world, _ed, new) = spawn_via_menu(kind);
            assert_eq!(new.len(), 1, "{kind:?} must create exactly one entity");
            assert_eq!(
                name_of(&world, new[0]).as_deref(),
                Some(kind.entity_name()),
                "{kind:?} came out under the wrong name"
            );
        }
    }

    /// `MeshRenderer` used to be added to every spawn before the match ran, so every light,
    /// camera and empty in the scene carried a renderer with nothing to render.
    #[test]
    fn nothing_that_draws_nothing_carries_a_renderer() {
        for kind in [
            SpawnKind::Empty,
            SpawnKind::Group,
            SpawnKind::PointLight,
            SpawnKind::Camera,
            SpawnKind::ParticleEmitter,
        ] {
            assert!(!kind.draws(), "{kind:?} is not a drawing kind");
            let (world, _ed, new) = spawn_via_menu(kind);
            assert!(
                world
                    .borrow::<gizmo::renderer::components::MeshRenderer>()
                    .get(new[0])
                    .is_none(),
                "{kind:?} must not be given a MeshRenderer"
            );
        }
    }

    /// A light is a light, a camera is a camera, an emitter is an emitter. All three used to be
    /// an empty entity with a name that said otherwise.
    #[test]
    fn the_light_camera_and_emitter_get_their_own_component() {
        let (world, _ed, new) = spawn_via_menu(SpawnKind::PointLight);
        assert!(world
            .borrow::<gizmo::renderer::components::PointLight>()
            .get(new[0])
            .is_some());

        let (world, _ed, new) = spawn_via_menu(SpawnKind::Camera);
        let cam = world.borrow::<gizmo::renderer::components::Camera>();
        let cam = cam.get(new[0]).expect("a Camera component");
        assert!(
            !cam.primary,
            "a camera dropped into a scene must not take over the view"
        );

        let (world, _ed, new) = spawn_via_menu(SpawnKind::ParticleEmitter);
        assert!(world
            .borrow::<gizmo::renderer::components::ParticleEmitter>()
            .get(new[0])
            .is_some());
    }

    /// Without GPU meshes a drawing kind must leave **nothing** behind. The old code kept the
    /// entity — a nameless thing with a renderer and no mesh, which is a scene member you then
    /// have to find and delete.
    #[test]
    fn a_drawing_kind_leaves_nothing_behind_when_the_meshes_are_missing() {
        for kind in [
            SpawnKind::Cube,
            SpawnKind::Sphere,
            SpawnKind::Plane,
            SpawnKind::Cylinder,
            SpawnKind::Capsule,
        ] {
            assert!(kind.draws(), "{kind:?} is a drawing kind");
            let (_world, _ed, new) = spawn_via_menu(kind);
            assert!(
                new.is_empty(),
                "{kind:?} left {} entities behind with no meshes to build from",
                new.len()
            );
        }
    }

    /// "📂 Seçilileri Grupla" created the group and stopped. Its own comment promised the second
    /// half — *"sonra seçili objeleri ona bağla"* — and nothing ever did it, so the button spawned
    /// a stray empty entity and left the selection exactly where it was.
    #[test]
    fn grouping_puts_the_selection_under_the_new_group() {
        let mut world = World::new();
        let (a, b) = (spawn(&mut world), spawn(&mut world));
        let mut ed = EditorState::default();
        ed.pending_group_members = vec![a, b];
        ed.spawn_request = Some(SpawnKind::Group);

        handle_scene_operations(&mut world, &mut ed, &mut studio_state());

        let group = world
            .iter_alive_entities()
            .into_iter()
            .find(|e| name_of(&world, e.id()).as_deref() == Some("Grup"))
            .expect("the group itself");

        for member in [a, b] {
            assert_eq!(
                world.borrow::<Parent>().get(member.id()).map(|p| p.0),
                Some(group.id()),
                "every selected entity must end up under the group"
            );
        }
        let mut listed = children_of(&world, group.id());
        listed.sort_unstable();
        let mut expected = vec![a.id(), b.id()];
        expected.sort_unstable();
        assert_eq!(listed, expected, "...and the group must list them both");

        // The group is what you now want to drag, so it is what is selected.
        assert!(
            ed.selection.entities.contains(&group),
            "the new group should be the selection"
        );
        assert!(
            ed.pending_group_members.is_empty(),
            "the request must be consumed, or the next spawn eats the same list again"
        );
    }

    /// A spawn that never happens must not leave its pending requests armed — the next one would
    /// pick them up and group the wrong things under the wrong entity.
    #[test]
    fn a_failed_spawn_disarms_its_pending_requests() {
        let mut world = World::new();
        let a = spawn(&mut world);
        let mut ed = EditorState::default();
        ed.pending_group_members = vec![a];
        ed.spawn_request = Some(SpawnKind::Cube); // needs meshes; there are none

        handle_scene_operations(&mut world, &mut ed, &mut studio_state());

        assert!(ed.pending_group_members.is_empty());
        assert!(ed.pending_child_parent.is_none());
        assert!(world.borrow::<Parent>().get(a.id()).is_none());
    }

    fn spawn(world: &mut World) -> gizmo::core::entity::Entity {
        let e = world.spawn();
        world.add_component(e, Transform::default());
        e
    }

    fn children_of(world: &World, parent: u32) -> Vec<u32> {
        world.borrow::<Children>().get(parent).map(|c| c.0.clone()).unwrap_or_default()
    }

    /// A reparent must leave BOTH halves of the hierarchy agreeing: the child points up, the
    /// parent lists it. Half of that is a scene that describes itself two different ways.
    #[test]
    fn a_reparent_sets_both_directions() {
        let mut world = World::new();
        let (parent, child) = (spawn(&mut world), spawn(&mut world));
        let mut ed = EditorState::default();
        ed.reparent_request = Some((child, parent));

        handle_scene_operations(&mut world, &mut ed, &mut studio_state());

        assert_eq!(
            world.borrow::<Parent>().get(child.id()).map(|p| p.0),
            Some(parent.id()),
            "the child must point at its new parent"
        );
        assert_eq!(children_of(&world, parent.id()), vec![child.id()], "…and be listed by it");
    }

    /// Moving a child from one parent to another must take it OUT of the first one's list.
    #[test]
    fn a_reparent_removes_the_child_from_its_previous_parent() {
        let mut world = World::new();
        let (first, second, child) = (spawn(&mut world), spawn(&mut world), spawn(&mut world));
        let mut ed = EditorState::default();

        ed.reparent_request = Some((child, first));
        handle_scene_operations(&mut world, &mut ed, &mut studio_state());
        ed.reparent_request = Some((child, second));
        handle_scene_operations(&mut world, &mut ed, &mut studio_state());

        assert!(
            children_of(&world, first.id()).is_empty(),
            "the old parent still lists a child it no longer has"
        );
        assert_eq!(children_of(&world, second.id()), vec![child.id()]);
        assert_eq!(world.borrow::<Parent>().get(child.id()).map(|p| p.0), Some(second.id()));
    }

    /// Dropping a node onto itself is refused. A `Children` cycle is not a cosmetic problem: it
    /// wedges transform propagation, `despawn_recursive` and scene save.
    #[test]
    fn an_entity_cannot_become_its_own_parent() {
        let mut world = World::new();
        let e = spawn(&mut world);
        let mut ed = EditorState::default();
        ed.reparent_request = Some((e, e));

        handle_scene_operations(&mut world, &mut ed, &mut studio_state());

        assert!(world.borrow::<Parent>().get(e.id()).is_none(), "self-parenting must be refused");
    }

    /// …and neither can it be dropped onto its own descendant, which is the same cycle one level
    /// down and the one a user actually produces by dragging in the hierarchy panel.
    #[test]
    fn an_entity_cannot_become_a_child_of_its_own_descendant() {
        let mut world = World::new();
        let (root, mid, leaf) = (spawn(&mut world), spawn(&mut world), spawn(&mut world));
        let mut ed = EditorState::default();
        ed.reparent_request = Some((mid, root));
        handle_scene_operations(&mut world, &mut ed, &mut studio_state());
        ed.reparent_request = Some((leaf, mid));
        handle_scene_operations(&mut world, &mut ed, &mut studio_state());

        // Now try to make the root a child of the leaf: root → mid → leaf → root.
        ed.reparent_request = Some((root, leaf));
        handle_scene_operations(&mut world, &mut ed, &mut studio_state());

        assert!(
            world.borrow::<Parent>().get(root.id()).is_none(),
            "the root was adopted by its own grandchild — that is a cycle"
        );
        assert_eq!(children_of(&world, leaf.id()), Vec::<u32>::new());
    }

    /// Unparenting drops the `Parent` component and the entry in the old parent's list together.
    #[test]
    fn unparenting_clears_both_directions() {
        let mut world = World::new();
        let (parent, child) = (spawn(&mut world), spawn(&mut world));
        let mut ed = EditorState::default();
        ed.reparent_request = Some((child, parent));
        handle_scene_operations(&mut world, &mut ed, &mut studio_state());

        ed.unparent_request = Some(child);
        handle_scene_operations(&mut world, &mut ed, &mut studio_state());

        assert!(world.borrow::<Parent>().get(child.id()).is_none(), "the child is a root now");
        assert!(
            children_of(&world, parent.id()).is_empty(),
            "the old parent must not keep listing it"
        );
    }

    /// **Every 🗑 button in the inspector must actually remove its component.**
    ///
    /// Four inspector sections draw one (`Hitbox`, `Hurtbox`, `BoneAttachment`,
    /// `FighterController`) and this system is the only thing that answers them. Three arms
    /// existed; the fourth did not, so pressing Delete on a fighter logged
    /// "Component turu silinemiyor: FighterController" at the user and left the component where
    /// it was. Testing the four together is the point — the defect was an asymmetry between a
    /// list of buttons and a list of arms, and a per-component test is exactly the shape that
    /// misses one.
    #[test]
    fn every_delete_button_in_the_inspector_removes_its_component() {
        for name in ["Hitbox", "Hurtbox", "BoneAttachment", "FighterController", "NavAgent"] {
            let mut world = World::new();
            let ent = world.spawn();
            let mut ed = EditorState::default();

            ed.add_component_request = Some((ent, name.to_string()));
            handle_scene_operations(&mut world, &mut ed, &mut studio_state());
            assert!(
                has_component(&world, ent.id(), name),
                "{name} was not added, so the removal below would prove nothing"
            );

            ed.remove_component_request = Some((ent, name.to_string()));
            handle_scene_operations(&mut world, &mut ed, &mut studio_state());
            assert!(
                !has_component(&world, ent.id(), name),
                "the inspector's delete button for {name} left the component in place"
            );
        }
    }

    /// **Every light type the engine draws can be created in the editor.**
    ///
    /// Two of the three could not, and had not since the components existed: the engine collected
    /// spot and directional lights into the light pool, the scene file round-tripped both, and
    /// neither was in the studio's `ComponentRegistry` — which is what the ➕ menu prints — nor in
    /// the arm above. The sun was the worse half: only `setup.rs`'s startup scene ever spawned
    /// one, so a scene that arrived without a sun could never gain one and stayed lit by point
    /// lights for ever, while a spot light that arrived from Rust showed not one editable row.
    ///
    /// Driven through the real request handler rather than compared against a list, because what
    /// broke was the handler having no arm: a menu entry whose "add" does nothing logs
    /// "Bilinmeyen component" at the user and is worse than no entry at all.
    #[test]
    fn every_light_the_engine_draws_can_be_added_in_the_editor() {
        use gizmo::renderer::components::{DirectionalLight, PointLight, SpotLight};

        let mut world = World::new();
        let ent = spawn(&mut world);
        let mut ed = EditorState::default();

        for name in ["PointLight", "DirectionalLight", "SpotLight"] {
            ed.add_component_request = Some((ent, name.to_string()));
            handle_scene_operations(&mut world, &mut ed, &mut studio_state());
        }

        assert!(world.borrow::<PointLight>().get(ent.id()).is_some());
        let suns = world.borrow::<DirectionalLight>();
        let sun = suns.get(ent.id()).expect("a sun can be added");
        assert!(sun.intensity > 0.0, "a sun that lights nothing reads as broken, not as neutral");
        let spots = world.borrow::<SpotLight>();
        let spot = spots.get(ent.id()).expect("a spot light can be added");
        assert!(
            spot.inner_angle <= spot.outer_angle,
            "an inner cone wider than its outer cone inverts the falloff and lights the outside \
             of the cone — the default must not ship that"
        );
        assert!(spot.radius > 0.0 && spot.intensity > 0.0);
    }

    /// Is `name`'s component on `id`? Written as a match rather than generically because the
    /// request the UI raises is a string, and this asks the same question the same way.
    fn has_component(world: &World, id: u32, name: &str) -> bool {
        match name {
            "Hitbox" => world.borrow::<gizmo::physics::components::Hitbox>().get(id).is_some(),
            "Hurtbox" => world.borrow::<gizmo::physics::components::Hurtbox>().get(id).is_some(),
            "BoneAttachment" => world
                .borrow::<gizmo::renderer::components::BoneAttachment>()
                .get(id)
                .is_some(),
            "FighterController" => world
                .borrow::<gizmo::physics::components::fighter::FighterController>()
                .get(id)
                .is_some(),
            "NavAgent" => world.borrow::<gizmo::ai::components::NavAgent>().get(id).is_some(),
            other => panic!("test bilinmeyen bileşen adı kullandı: {other}"),
        }
    }

    /// **Adding two fighters gives player 1 and player 2**, which is what the studio's own fight
    /// HUD requires before it draws anything: it wants a `p1` *and* a `p2` entity, and
    /// `FighterController::default()` is player 1 every time. Authoring a versus scene the
    /// obvious way used to produce two player 1s and a HUD that never appeared.
    #[test]
    fn each_fighter_added_takes_the_next_free_player_slot() {
        let mut world = World::new();
        let mut ed = EditorState::default();

        let mut slots = Vec::new();
        for _ in 0..3 {
            let ent = world.spawn();
            ed.add_component_request = Some((ent, "FighterController".to_string()));
            handle_scene_operations(&mut world, &mut ed, &mut studio_state());
            slots.push(
                world
                    .borrow::<gizmo::physics::components::fighter::FighterController>()
                    .get(ent.id())
                    .expect("the fighter was added")
                    .player_id,
            );
        }

        assert_eq!(slots, vec![1, 2, 3], "each fighter takes the lowest free slot");

        // And a freed slot is reused rather than skipped: delete player 2, add another.
        let p2 = world
            .borrow::<gizmo::physics::components::fighter::FighterController>()
            .iter()
            .find(|(_, f)| f.player_id == 2)
            .map(|(id, _)| id)
            .expect("player 2 is there");
        let p2 = world.get_entity(p2).expect("still alive");
        ed.remove_component_request = Some((p2, "FighterController".to_string()));
        handle_scene_operations(&mut world, &mut ed, &mut studio_state());

        let ent = world.spawn();
        ed.add_component_request = Some((ent, "FighterController".to_string()));
        handle_scene_operations(&mut world, &mut ed, &mut studio_state());
        assert_eq!(
            world
                .borrow::<gizmo::physics::components::fighter::FighterController>()
                .get(ent.id())
                .expect("added")
                .player_id,
            2,
            "the slot the deleted fighter held is free again"
        );
    }
}
