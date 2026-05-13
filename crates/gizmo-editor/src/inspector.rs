//! Component Inspector Panel — Sağ panel'de seçili entity'nin bileşenlerini gösterir ve düzenlenebilir

use crate::editor_state::EditorState;
use egui;
use gizmo_ai::components::NavAgent;
use gizmo_core::{EntityName, World};
use gizmo_math::{Vec3, Vec4};
use gizmo_physics::components::{FluidSimulation, RigidBody, Transform, Velocity};
use gizmo_physics::shape::Collider;
use gizmo_renderer::components::{Camera, Material, ParticleEmitter, PointLight};

/// Inspector sekmesini çizer
pub fn ui_inspector(ui: &mut egui::Ui, world: &World, state: &mut EditorState) {
    let sel_len = state.selection.entities.len();
    if sel_len == 0 {
        ui.label(egui::RichText::new("Hiçbir obje seçili değil.").color(egui::Color32::GRAY));
        return;
    }

    let primary_entity = state
        .selection
        .primary
        .unwrap_or_else(|| *state.selection.entities.iter().next().unwrap());

    if !world.is_alive(primary_entity) {
        return;
    }

    if sel_len > 1 {
        ui.heading(format!("🔧 Çoklu Obje Seçili ({} adet)", sel_len));
        ui.label(egui::RichText::new("💡 Transform değişiklikleri tüm seçili objelere bağıl (relative) olarak uygulanır.").weak());
        if ui
            .button(egui::RichText::new("🗑️ Seçili Objeleri Sil").color(egui::Color32::RED))
            .clicked()
        {
            for &entity in state.selection.entities.iter() {
                state.despawn_requests.push(entity);
            }
        }
    } else {
        ui.heading(format!("🔧 Inspector [{}]", primary_entity.id()));
        if ui
            .button(egui::RichText::new("🗑️ Seçili Objeyi Sil").color(egui::Color32::RED))
            .clicked()
        {
            state.despawn_requests.push(primary_entity);
        }
    }

    ui.separator();

    let entity_id = primary_entity;

    egui::ScrollArea::vertical().show(ui, |ui| {
        // === ENTITY NAME ===
        if sel_len == 1 {
            draw_name_section(ui, world, entity_id, state);
        }

        // === TRANSFORM ===
        draw_transform_section(ui, world, entity_id, state);

        // === VELOCITY ===
        draw_velocity_section(ui, world, entity_id, state);

        // === RIGIDBODY ===
        draw_rigidbody_section(ui, world, entity_id, state);

        // === COLLIDER ===
        draw_collider_section(ui, world, entity_id, state);

        // === JOINTS (FİZİKSEL BAĞLANTILAR) ===
        draw_joint_section(ui, world, entity_id, state);

        // === CAMERA ===
        draw_camera_section(ui, world, entity_id, state);

        // === POINT LIGHT ===
        draw_point_light_section(ui, world, entity_id, state);

        // === MATERIAL ===
        draw_material_section(ui, world, entity_id, state);

        // === PARTICLE EMITTER ===
        draw_particle_emitter_section(ui, world, entity_id, state);

        draw_terrain_section(ui, world, entity_id, state);
        draw_script_section(ui, world, entity_id, state);
        draw_fluid_section(ui, world, entity_id, state);
        draw_ai_section(ui, world, entity_id, state);
        draw_reflection_section(ui, world, entity_id, state);

        ui.separator();

        // === ADD COMPONENT BUTONU ===
        if sel_len == 1 {
            ui.horizontal(|ui| {
                if ui.button("➕ Bileşen Ekle").clicked() {
                    state.add_component_open = !state.add_component_open;
                }
            });

            if state.add_component_open {
                draw_add_component_menu(ui, world, entity_id, state);
            }
        }
    });
}

fn draw_name_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    let mut names = world.borrow_mut::<EntityName>();
    {
        if let Some(name) = names.get_mut(entity_id.id()) {
            ui.horizontal(|ui| {
                ui.label("İsim:");
                ui.text_edit_singleline(&mut name.0);
            });
            ui.separator();
        }
    }
}

fn draw_transform_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    state: &mut EditorState,
) {
    let mut transforms = world.borrow_mut::<Transform>();
    
    // Eski durumu yedekleyelim (multi-select delta hesaplaması için)
    let old_t = transforms.get(entity_id.id()).copied();

    let mut changed = false;
    if let Some(t) = transforms.get_mut(entity_id.id()) {
        egui::CollapsingHeader::new("📐 Transform")
            .default_open(true)
            .show(ui, |ui| {
                ui.label("Pozisyon:");
                ui.horizontal(|ui| {
                    ui.label("X");
                    if ui.add(egui::DragValue::new(&mut t.position.x).speed(0.1)).changed() { changed = true; }
                    ui.label("Y");
                    if ui.add(egui::DragValue::new(&mut t.position.y).speed(0.1)).changed() { changed = true; }
                    ui.label("Z");
                    if ui.add(egui::DragValue::new(&mut t.position.z).speed(0.1)).changed() { changed = true; }
                });

                ui.label("Rotasyon (Euler°):");
                let (mut rx, mut ry, mut rz) = quat_to_euler_deg(t.rotation);
                let old_euler = (rx, ry, rz);
                ui.horizontal(|ui| {
                    ui.label("X");
                    if ui.add(egui::DragValue::new(&mut rx).speed(1.0).suffix("°")).changed() { changed = true; }
                    ui.label("Y");
                    if ui.add(egui::DragValue::new(&mut ry).speed(1.0).suffix("°")).changed() { changed = true; }
                    ui.label("Z");
                    if ui.add(egui::DragValue::new(&mut rz).speed(1.0).suffix("°")).changed() { changed = true; }
                });
                if (rx, ry, rz) != old_euler {
                    t.rotation = euler_deg_to_quat(rx, ry, rz);
                }

                ui.label("Ölçek:");
                ui.horizontal(|ui| {
                    ui.label("X");
                    if ui.add(egui::DragValue::new(&mut t.scale.x).speed(0.05).range(0.01..=100.0)).changed() { changed = true; }
                    ui.label("Y");
                    if ui.add(egui::DragValue::new(&mut t.scale.y).speed(0.05).range(0.01..=100.0)).changed() { changed = true; }
                    ui.label("Z");
                    if ui.add(egui::DragValue::new(&mut t.scale.z).speed(0.05).range(0.01..=100.0)).changed() { changed = true; }
                });
            });
        
        if changed {
            t.update_local_matrix();
        }
        ui.separator();
    }

    // Çoklu seçim (Multi-Object Editing) Delta Uygulaması
    if changed && state.selection.entities.len() > 1 {
        if let Some(old) = old_t {
            if let Some(new_t) = transforms.get(entity_id.id()).copied() {
                let delta_pos = new_t.position - old.position;
                let delta_rot = new_t.rotation * old.rotation.inverse();
                let delta_scale = gizmo_math::Vec3::new(
                    if old.scale.x != 0.0 { new_t.scale.x / old.scale.x } else { 1.0 },
                    if old.scale.y != 0.0 { new_t.scale.y / old.scale.y } else { 1.0 },
                    if old.scale.z != 0.0 { new_t.scale.z / old.scale.z } else { 1.0 },
                );

                let others: Vec<_> = state.selection.entities.iter().copied().filter(|&e| e != entity_id).collect();
                for e in others {
                    if let Some(other_t) = transforms.get_mut(e.id()) {
                        other_t.position += delta_pos;
                        other_t.rotation = delta_rot * other_t.rotation;
                        other_t.scale *= delta_scale;
                        other_t.update_local_matrix();
                    }
                }
            }
        }
    }
}

fn draw_velocity_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    let mut velocities = world.borrow_mut::<Velocity>();
    {
        if let Some(v) = velocities.get_mut(entity_id.id()) {
            egui::CollapsingHeader::new("💨 Velocity")
                .default_open(false)
                .show(ui, |ui| {
                    ui.label("Doğrusal:");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(&mut v.linear.x)
                                .speed(0.1)
                                .prefix("X: "),
                        );
                        ui.add(
                            egui::DragValue::new(&mut v.linear.y)
                                .speed(0.1)
                                .prefix("Y: "),
                        );
                        ui.add(
                            egui::DragValue::new(&mut v.linear.z)
                                .speed(0.1)
                                .prefix("Z: "),
                        );
                    });
                    ui.label("Açısal:");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(&mut v.angular.x)
                                .speed(0.1)
                                .prefix("X: "),
                        );
                        ui.add(
                            egui::DragValue::new(&mut v.angular.y)
                                .speed(0.1)
                                .prefix("Y: "),
                        );
                        ui.add(
                            egui::DragValue::new(&mut v.angular.z)
                                .speed(0.1)
                                .prefix("Z: "),
                        );
                    });
                });
            ui.separator();
        }
    }
}

fn draw_rigidbody_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    let mut rigidbodies = world.borrow_mut::<RigidBody>();
    {
        if let Some(rb) = rigidbodies.get_mut(entity_id.id()) {
            egui::CollapsingHeader::new("⚙️ RigidBody")
                .default_open(false)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Kütle:");
                        ui.add(
                            egui::DragValue::new(&mut rb.mass)
                                .speed(0.5)
                                .range(0.0..=100000.0),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label("Sekme:");
                        ui.add(
                            egui::DragValue::new(&mut rb.restitution)
                                .speed(0.01)
                                .range(0.0..=1.0),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label("Sürtünme:");
                        ui.add(
                            egui::DragValue::new(&mut rb.friction)
                                .speed(0.01)
                                .range(0.0..=2.0),
                        );
                    });
                    ui.checkbox(&mut rb.use_gravity, "Yerçekimi");
                    ui.checkbox(&mut rb.ccd_enabled, "CCD (Hızlı Obje)");

                    let status = if rb.is_sleeping {
                        "💤 Uyuyor"
                    } else {
                        "⚡ Aktif"
                    };
                    ui.label(format!("Durum: {}", status));

                    if rb.is_sleeping && ui.button("Uyandır").clicked() {
                        rb.wake_up();
                    }
                });
            ui.separator();
        }
    }
}

fn draw_collider_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    let mut colliders = world.borrow_mut::<Collider>();
    {
        if let Some(collider) = colliders.get_mut(entity_id.id()) {
            egui::CollapsingHeader::new("🛡️ Collider")
                .default_open(true)
                .show(ui, |ui| {
                    // === IS TRIGGER ===
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut collider.is_trigger, "Trigger (Tetikleyici)");
                        ui.label(
                            egui::RichText::new("ℹ")
                                .weak()
                                .small(),
                        ).on_hover_text("Trigger açıkken fiziksel çarpışma olmaz,\nsadece giriş/çıkış olayları tetiklenir.\n(Kapı sensörü, checkpoint, alan hasarı vb.)");
                    });

                    if collider.is_trigger {
                        ui.label(
                            egui::RichText::new("⚡ Bu collider bir tetikleyicidir — fiziksel tepki vermez.")
                                .color(egui::Color32::from_rgb(240, 180, 50))
                                .small(),
                        );
                    }

                    ui.add_space(4.0);

                    // === ŞEKİL AYARLARI ===
                    match &mut collider.shape {
                        gizmo_physics::shape::ColliderShape::Box(aabb) => {
                            ui.label("Şekil: Kutu (AABB)");
                            ui.horizontal(|ui| {
                                ui.label("Boyut:");
                                ui.add(
                                    egui::DragValue::new(&mut aabb.half_extents.x)
                                        .speed(0.1)
                                        .prefix("X: "),
                                );
                                ui.add(
                                    egui::DragValue::new(&mut aabb.half_extents.y)
                                        .speed(0.1)
                                        .prefix("Y: "),
                                );
                                ui.add(
                                    egui::DragValue::new(&mut aabb.half_extents.z)
                                        .speed(0.1)
                                        .prefix("Z: "),
                                );
                            });
                        }
                        gizmo_physics::shape::ColliderShape::Sphere(sphere) => {
                            ui.label("Şekil: Küre (Sphere)");
                            ui.horizontal(|ui| {
                                ui.label("Yarıçap:");
                                ui.add(egui::DragValue::new(&mut sphere.radius).speed(0.1));
                            });
                        }
                        gizmo_physics::shape::ColliderShape::Capsule(capsule) => {
                            ui.label("Şekil: Kapsül");
                            ui.horizontal(|ui| {
                                ui.label("Yarıçap:");
                                ui.add(egui::DragValue::new(&mut capsule.radius).speed(0.1));
                                ui.label("Y. Yükseklik:");
                                ui.add(egui::DragValue::new(&mut capsule.half_height).speed(0.1));
                            });
                        }
                        other => {
                            ui.label(
                                egui::RichText::new(format!("Şekil: {:?} (Sadece Okunur)", other))
                                    .color(egui::Color32::GRAY),
                            );
                        }
                    }


                });
            ui.separator();
        }
    }
}

fn draw_camera_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    let mut cameras = world.borrow_mut::<Camera>();
    {
        if let Some(cam) = cameras.get_mut(entity_id.id()) {
            egui::CollapsingHeader::new("📷 Camera")
                .default_open(false)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("FOV:");
                        let mut fov_deg = cam.fov.to_degrees();
                        if ui
                            .add(
                                egui::DragValue::new(&mut fov_deg)
                                    .speed(1.0)
                                    .range(10.0..=120.0)
                                    .suffix("°"),
                            )
                            .changed()
                        {
                            cam.fov = fov_deg.to_radians();
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Near:");
                        ui.add(
                            egui::DragValue::new(&mut cam.near)
                                .speed(0.01)
                                .range(0.001..=10.0),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label("Far:");
                        ui.add(
                            egui::DragValue::new(&mut cam.far)
                                .speed(10.0)
                                .range(10.0..=50000.0),
                        );
                    });
                    ui.checkbox(&mut cam.primary, "Ana Kamera");
                });
            ui.separator();
        }
    }
}

fn draw_point_light_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    let mut lights = world.borrow_mut::<PointLight>();
    {
        if let Some(light) = lights.get_mut(entity_id.id()) {
            egui::CollapsingHeader::new("💡 PointLight")
                .default_open(false)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Renk:");
                        let mut color = [light.color.x, light.color.y, light.color.z];
                        if ui.color_edit_button_rgb(&mut color).changed() {
                            light.color = Vec3::new(color[0], color[1], color[2]);
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Yoğunluk:");
                        ui.add(
                            egui::DragValue::new(&mut light.intensity)
                                .speed(0.1)
                                .range(0.0..=100.0),
                        );
                    });
                });
            ui.separator();
        }
    }
}

fn draw_material_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    let mut materials = world.borrow_mut::<Material>();
    {
        if let Some(mat) = materials.get_mut(entity_id.id()) {
            egui::CollapsingHeader::new("🎨 Material")
                .default_open(false)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Albedo:");
                        let mut color = [mat.albedo.x, mat.albedo.y, mat.albedo.z, mat.albedo.w];
                        if ui.color_edit_button_rgba_unmultiplied(&mut color).changed() {
                            mat.albedo = Vec4::new(color[0], color[1], color[2], color[3]);
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Metalik:");
                        ui.add(
                            egui::DragValue::new(&mut mat.metallic)
                                .speed(0.01)
                                .range(0.0..=1.0),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label("Pürüzlülük (Roughness):");
                        ui.add(
                            egui::DragValue::new(&mut mat.roughness)
                                .speed(0.01)
                                .range(0.0..=1.0),
                        );
                    });

                    let mode = match mat.material_type {
                        gizmo_renderer::components::MaterialType::Pbr => "PBR",
                        gizmo_renderer::components::MaterialType::Unlit => "Unlit",
                        gizmo_renderer::components::MaterialType::Skybox => "Skybox",
                        gizmo_renderer::components::MaterialType::Water => "Water",
                        gizmo_renderer::components::MaterialType::Grid => "Grid",
                    };
                    ui.label(format!("Mod: {}", mode));

                    if let Some(src) = &mat.texture_source {
                        ui.label(format!("Texture: {}", src));
                    }
                });
            ui.separator();
        }
    }
}

fn draw_particle_emitter_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    let mut emitters = world.borrow_mut::<ParticleEmitter>();
    {
        if let Some(emitter) = emitters.get_mut(entity_id.id()) {
            egui::CollapsingHeader::new("✨ Particle Emitter")
                .default_open(false)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Üretim Hızı (Rate):");
                        ui.add(egui::Slider::new(&mut emitter.spawn_rate, 0.0..=5000.0));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Ömür (Lifespan):");
                        ui.add(egui::Slider::new(&mut emitter.lifespan, 0.1..=10.0));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Başlangıç Boyutu:");
                        ui.add(egui::Slider::new(&mut emitter.size_start, 0.1..=10.0));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Saçılma (Rnd):");
                        ui.add(egui::Slider::new(
                            &mut emitter.velocity_randomness,
                            0.0..=20.0,
                        ));
                    });
                });
            ui.separator();
        }
    }
}

fn draw_script_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    state: &mut EditorState,
) {
    let mut pending_text = None;
    let mut file_path = String::new();
    let mut scripts = world.borrow_mut::<gizmo_scripting::Script>();
    {
        if let Some(script) = scripts.get_mut(entity_id.id()) {
            file_path = script.file_path.clone();
            egui::CollapsingHeader::new("📜 Script")
                .default_open(true)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Dosya Yolu:");
                        ui.text_edit_singleline(&mut script.file_path);
                    });

                    if ui.button("✏️ Düzenle").clicked() {
                        match std::fs::read_to_string(&script.file_path) {
                            Ok(content) => pending_text = Some(content),
                            Err(e) => {
                                state.last_error = Some(format!("Script okuma hatası: {}", e));
                            }
                        }
                    }
                });
            ui.separator();
        }
    }

    if let Some(content) = pending_text {
        state.script.active_content = Some(content);
        state.script.active_path = Some(file_path);
        state.script.is_dirty = false;
        state.script.pending_clear_confirm = false;
        state.open_tab(crate::editor_state::EditorTab::ScriptEditor);
    }
}

fn draw_add_component_menu(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    state: &mut EditorState,
) {
    ui.group(|ui| {
        ui.label("Eklenebilecek Bileşenler");
        ui.separator();

        if let Some(registry) = world.get_resource::<gizmo_core::ComponentRegistry>() {
            let names = registry.all_names();
            for comp_name in names {
                // TODO: Entity üzerinde component olup olmadığını gizmo_core registry üzerinden checkle.
                if ui.button(format!("🔹 {}", comp_name)).clicked() {
                    state.add_component_request = Some((entity_id, comp_name.to_string()));
                    state.add_component_open = false;
                }
            }
        } else {
            ui.label(
                egui::RichText::new("ComponentRegistry bulunamadi!").color(egui::Color32::RED),
            );
        }
    });
}

fn draw_terrain_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    state: &mut EditorState,
) {
    let mut terrains = world.borrow_mut::<gizmo_renderer::components::Terrain>();
    {
        if let Some(terrain) = terrains.get_mut(entity_id.id()) {
            let mut changed = false;
            egui::CollapsingHeader::new("🏔 Terrain")
                .default_open(true)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Dosya Yolu:");
                        ui.text_edit_singleline(&mut terrain.heightmap_path);
                    });
                    ui.label(format!("Boyut: {}x{}", terrain.width, terrain.depth));
                    ui.horizontal(|ui| {
                        ui.label("Genişlik (X):");
                        if ui
                            .add(egui::Slider::new(&mut terrain.width, 10.0..=1000.0).suffix("m"))
                            .changed()
                        {
                            changed = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Derinlik (Z):");
                        if ui
                            .add(egui::Slider::new(&mut terrain.depth, 10.0..=1000.0).suffix("m"))
                            .changed()
                        {
                            changed = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Maks. Yükseklik:");
                        if ui
                            .add(
                                egui::Slider::new(&mut terrain.max_height, 1.0..=500.0).suffix("m"),
                            )
                            .changed()
                        {
                            changed = true;
                        }
                    });
                });
            if changed {
                state.generate_terrain_requests.push(entity_id);
            }
            ui.separator();
        }
    }
}

fn draw_fluid_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    let mut fluids = world.borrow_mut::<FluidSimulation>();
    {
        if let Some(fluid) = fluids.get_mut(entity_id.id()) {
            egui::CollapsingHeader::new("🌊 SPH Fluid Simulation (GPU)")
                .default_open(true)
                .show(ui, |ui| {
                    ui.label("GPU SPH Engine Aktif (ECS Üzerinden Yönetilir)");
                    ui.horizontal(|ui| {
                        ui.label("Hedef Yoğunluk:");
                        ui.add(
                            egui::DragValue::new(&mut fluid.target_density)
                                .speed(1.0)
                                .range(100.0..=2000.0),
                        );
                    });

                    ui.horizontal(|ui| {
                        ui.label("Basınç Çarpanı:");
                        ui.add(
                            egui::DragValue::new(&mut fluid.pressure_multiplier)
                                .speed(1.0)
                                .range(1.0..=1000.0),
                        );
                    });

                    ui.horizontal(|ui| {
                        ui.label("Viskozite:");
                        ui.add(
                            egui::DragValue::new(&mut fluid.viscosity)
                                .speed(0.01)
                                .range(0.001..=1.0),
                        );
                    });

                    ui.horizontal(|ui| {
                        ui.label("Parçacık Yarıçapı:");
                        ui.add(
                            egui::DragValue::new(&mut fluid.particle_radius)
                                .speed(0.01)
                                .range(0.01..=1.0),
                        );
                    });
                });
            ui.separator();
        }
    }
}

fn draw_ai_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    let mut agents = world.borrow_mut::<NavAgent>();
    {
        if let Some(agent) = agents.get_mut(entity_id.id()) {
            egui::CollapsingHeader::new("🤖 AI NavAgent")
                .default_open(true)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Maks Hız:");
                        ui.add(
                            egui::DragValue::new(&mut agent.max_speed)
                                .speed(0.1)
                                .range(0.1..=100.0),
                        );
                    });

                    ui.horizontal(|ui| {
                        ui.label("Steering (Dönüş) Gücü:");
                        ui.add(
                            egui::DragValue::new(&mut agent.steering_force)
                                .speed(0.1)
                                .range(0.1..=100.0),
                        );
                    });

                    ui.horizontal(|ui| {
                        ui.label("Varış Yarıçapı:");
                        ui.add(
                            egui::DragValue::new(&mut agent.arrival_radius)
                                .speed(0.1)
                                .range(0.1..=10.0),
                        );
                    });

                    let state_str = match agent.state {
                        gizmo_ai::components::NavAgentState::Idle => "Bekliyor",
                        gizmo_ai::components::NavAgentState::Moving => "Hareket Ediyor",
                        gizmo_ai::components::NavAgentState::Reached => "Ulaştı",
                        gizmo_ai::components::NavAgentState::Stuck => "Sıkıştı",
                    };
                    ui.label(format!("Durum: {}", state_str));

                    if let Some(target) = agent.target {
                        ui.label(format!(
                            "Hedef: {:.1}, {:.1}, {:.1}",
                            target.x, target.y, target.z
                        ));
                    } else {
                        ui.label("Hedef: Yok");
                    }

                    ui.label(format!("Rota Uzunluğu: {}", agent.path_len()));
                });
            ui.separator();
        }
    }
}

// === YARDIMCI FONKSİYONLAR ===

fn draw_json_value(ui: &mut egui::Ui, name: &str, value: &mut serde_json::Value) -> bool {
    let mut changed = false;
    match value {
        serde_json::Value::Number(num) => {
            if let Some(f) = num.as_f64() {
                let mut v = f;
                ui.horizontal(|ui| {
                    ui.label(name);
                    if ui.add(egui::DragValue::new(&mut v).speed(0.1)).changed() {
                        if let Some(n) = serde_json::Number::from_f64(v) {
                            *num = n;
                            changed = true;
                        }
                    }
                });
            } else if let Some(i) = num.as_i64() {
                let mut v = i;
                ui.horizontal(|ui| {
                    ui.label(name);
                    if ui.add(egui::DragValue::new(&mut v)).changed() {
                        *num = serde_json::Number::from(v);
                        changed = true;
                    }
                });
            }
        }
        serde_json::Value::Bool(b) => {
            ui.horizontal(|ui| {
                if ui.checkbox(b, name).changed() {
                    changed = true;
                }
            });
        }
        serde_json::Value::String(s) => {
            ui.horizontal(|ui| {
                ui.label(name);
                if ui.text_edit_singleline(s).changed() {
                    changed = true;
                }
            });
        }
        serde_json::Value::Object(map) => {
            ui.vertical(|ui| {
                ui.label(name);
                ui.indent(name, |ui| {
                    for (k, v) in map.iter_mut() {
                        if draw_json_value(ui, k, v) {
                            changed = true;
                        }
                    }
                });
            });
        }
        serde_json::Value::Array(arr) => {
            ui.vertical(|ui| {
                ui.label(format!("{} (Dizi)", name));
                ui.indent(name, |ui| {
                    for (i, v) in arr.iter_mut().enumerate() {
                        if draw_json_value(ui, &format!("[{}]", i), v) {
                            changed = true;
                        }
                    }
                });
            });
        }
        _ => {
            ui.label(format!("{}: <Desteklenmeyen tip>", name));
        }
    }
    changed
}

fn draw_reflection_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    state: &mut EditorState,
) {
    let skip_names = [
        "EntityName",
        "Transform",
        "Velocity",
        "RigidBody",
        "Collider",
        "Camera",
        "PointLight",
        "Material",
        "ParticleEmitter",
        "Terrain",
        "Script",
        "FluidSimulation",
        "NavAgent",
    ];

    if let Some(registry) = world.get_resource::<gizmo_core::ComponentRegistry>() {
        let types = world.get_entity_component_types(entity_id);

        for tid in types {
            if let Some(reg) = registry.get_registration(tid) {
                if skip_names.contains(&reg.name.as_str()) {
                    continue;
                }

                if let (Some(get_json), Some(set_json)) = (reg.get_json_fn, reg.set_json_fn) {
                    if let Some(ptr) = world.get_component_ptr(entity_id, tid) {
                        if let Ok(mut val) = get_json(ptr) {
                            let mut changed = false;
                            egui::CollapsingHeader::new(format!("🧩 {}", reg.name))
                                .default_open(true)
                                .show(ui, |ui| {
                                    if draw_json_value(ui, &reg.name, &mut val) {
                                        changed = true;
                                    }
                                });
                            ui.separator();
                            if changed {
                                state.pending_json_updates.push((entity_id, set_json, val));
                            }
                        }
                    }
                }
            }
        }
    }
}

fn quat_to_euler_deg(q: gizmo_math::Quat) -> (f32, f32, f32) {
    let (x, y, z) = q.to_euler(gizmo_math::EulerRot::XYZ);
    (x.to_degrees(), y.to_degrees(), z.to_degrees())
}

fn euler_deg_to_quat(rx: f32, ry: f32, rz: f32) -> gizmo_math::Quat {
    gizmo_math::Quat::from_euler(
        gizmo_math::EulerRot::XYZ,
        rx.to_radians(),
        ry.to_radians(),
        rz.to_radians(),
    )
}

fn draw_joint_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    let physics_world_res = world.try_get_resource_mut::<gizmo_physics::world::PhysicsWorld>();
    if let Ok(mut physics_world) = physics_world_res {
        let mut has_joints = false;
        for joint in &physics_world.joints {
            if joint.entity_a == entity_id || joint.entity_b == entity_id {
                has_joints = true;
                break;
            }
        }

        egui::CollapsingHeader::new("🔗 Joints (Fiziksel Bağlantılar)")
            .default_open(has_joints)
            .show(ui, |ui| {
                let mut i = 0;
                while i < physics_world.joints.len() {
                    let mut remove_joint = false;
                    let joint = &mut physics_world.joints[i];

                    if joint.entity_a == entity_id || joint.entity_b == entity_id {
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(format!("Tip: {}", joint.joint_type())).strong());
                                if ui.button(egui::RichText::new("🗑 Sil").color(egui::Color32::RED)).clicked() {
                                    remove_joint = true;
                                }
                            });

                            let other_id = if joint.entity_a == entity_id { joint.entity_b.id() } else { joint.entity_a.id() };
                            ui.label(format!("Bağlı Obje: [{}]", other_id));

                            ui.horizontal(|ui| {
                                ui.label("Kırılma Gücü:");
                                ui.add(egui::DragValue::new(&mut joint.break_force).speed(10.0));
                            });
                            ui.horizontal(|ui| {
                                ui.label("Kırılma Torku:");
                                ui.add(egui::DragValue::new(&mut joint.break_torque).speed(10.0));
                            });
                            ui.checkbox(&mut joint.collision_enabled, "Kendi Aralarında Çarpışma");

                            if joint.is_broken {
                                ui.label(egui::RichText::new("⚠️ BAĞLANTI KOPTU (BROKEN)").color(egui::Color32::RED));
                            }

                            ui.separator();
                            match &mut joint.data {
                                gizmo_physics::joints::JointData::Fixed => {
                                    ui.label("Sabit bağlantı (ayarı yok)");
                                }
                                gizmo_physics::joints::JointData::Spring(spring) => {
                                    ui.horizontal(|ui| { ui.label("Sertlik:"); ui.add(egui::DragValue::new(&mut spring.stiffness).speed(1.0)); });
                                    ui.horizontal(|ui| { ui.label("Sönümleme:"); ui.add(egui::DragValue::new(&mut spring.damping).speed(0.1)); });
                                    ui.horizontal(|ui| { ui.label("Dinlenme Boyu:"); ui.add(egui::DragValue::new(&mut spring.rest_length).speed(0.1)); });
                                }
                                gizmo_physics::joints::JointData::Hinge(hinge) => {
                                    ui.horizontal(|ui| {
                                        ui.label("Eksen:");
                                        ui.add(egui::DragValue::new(&mut hinge.axis.x).speed(0.1));
                                        ui.add(egui::DragValue::new(&mut hinge.axis.y).speed(0.1));
                                        ui.add(egui::DragValue::new(&mut hinge.axis.z).speed(0.1));
                                    });
                                    ui.checkbox(&mut hinge.use_motor, "Motor Kullan");
                                    if hinge.use_motor {
                                        ui.horizontal(|ui| { ui.label("Hedef Hız:"); ui.add(egui::DragValue::new(&mut hinge.motor_target_velocity).speed(0.1)); });
                                        ui.horizontal(|ui| { ui.label("Maks Güç:"); ui.add(egui::DragValue::new(&mut hinge.motor_max_force).speed(1.0)); });
                                    }
                                }
                                gizmo_physics::joints::JointData::Slider(slider) => {
                                    ui.horizontal(|ui| {
                                        ui.label("Eksen:");
                                        ui.add(egui::DragValue::new(&mut slider.axis.x).speed(0.1));
                                        ui.add(egui::DragValue::new(&mut slider.axis.y).speed(0.1));
                                        ui.add(egui::DragValue::new(&mut slider.axis.z).speed(0.1));
                                    });
                                    ui.checkbox(&mut slider.use_limits, "Limit Kullan");
                                    if slider.use_limits {
                                        ui.horizontal(|ui| { ui.label("Min:"); ui.add(egui::DragValue::new(&mut slider.lower_limit).speed(0.1)); });
                                        ui.horizontal(|ui| { ui.label("Max:"); ui.add(egui::DragValue::new(&mut slider.upper_limit).speed(0.1)); });
                                    }
                                }
                                gizmo_physics::joints::JointData::BallSocket(ball) => {
                                    ui.checkbox(&mut ball.use_cone_limit, "Koni Limiti Kullan");
                                    if ball.use_cone_limit {
                                        ui.horizontal(|ui| { ui.label("Açı (Radyan):"); ui.add(egui::DragValue::new(&mut ball.cone_limit_angle).speed(0.01)); });
                                    }
                                }
                            }
                        });
                    }

                    if remove_joint {
                        physics_world.joints.remove(i);
                    } else {
                        i += 1;
                    }
                }

                ui.separator();
                ui.label("➕ Yeni Bağlantı Ekle");
                let target_id_str_id = ui.id().with("j_target");
                let joint_type_idx_id = ui.id().with("j_type");
                
                let mut target_id_str = ui.data_mut(|d| d.get_temp::<String>(target_id_str_id).unwrap_or_default());
                let mut type_idx = ui.data_mut(|d| d.get_temp::<usize>(joint_type_idx_id).unwrap_or(0));
                
                ui.horizontal(|ui| {
                    ui.label("Hedef Obje ID:");
                    ui.text_edit_singleline(&mut target_id_str);
                });
                
                egui::ComboBox::from_label("Bağlantı Tipi")
                    .selected_text(match type_idx {
                        0 => "Fixed (Sabit)",
                        1 => "Hinge (Menteşe)",
                        2 => "BallSocket (Küresel)",
                        3 => "Slider (Kızak)",
                        4 => "Spring (Yay)",
                        _ => "Bilinmeyen",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut type_idx, 0, "Fixed (Sabit)");
                        ui.selectable_value(&mut type_idx, 1, "Hinge (Menteşe)");
                        ui.selectable_value(&mut type_idx, 2, "BallSocket (Küresel)");
                        ui.selectable_value(&mut type_idx, 3, "Slider (Kızak)");
                        ui.selectable_value(&mut type_idx, 4, "Spring (Yay)");
                    });
                    
                ui.data_mut(|d| d.insert_temp(target_id_str_id, target_id_str.clone()));
                ui.data_mut(|d| d.insert_temp(joint_type_idx_id, type_idx));

                if ui.button("Ekle").clicked() {
                    if let Ok(target_id) = target_id_str.parse::<u32>() {
                        let target_entity = gizmo_core::entity::Entity::new(target_id, 0); // Varsayılan generation 0
                        let new_joint = match type_idx {
                            0 => gizmo_physics::joints::Joint::fixed(entity_id, target_entity, gizmo_math::Vec3::ZERO, gizmo_math::Vec3::ZERO),
                            1 => gizmo_physics::joints::Joint::hinge(entity_id, target_entity, gizmo_math::Vec3::ZERO, gizmo_math::Vec3::ZERO, gizmo_math::Vec3::Y),
                            2 => gizmo_physics::joints::Joint::ball_socket(entity_id, target_entity, gizmo_math::Vec3::ZERO, gizmo_math::Vec3::ZERO),
                            3 => gizmo_physics::joints::Joint::slider(entity_id, target_entity, gizmo_math::Vec3::ZERO, gizmo_math::Vec3::ZERO, gizmo_math::Vec3::Y),
                            4 => gizmo_physics::joints::Joint::spring(entity_id, target_entity, gizmo_math::Vec3::ZERO, gizmo_math::Vec3::ZERO, 1.0, 10.0, 1.0),
                            _ => gizmo_physics::joints::Joint::fixed(entity_id, target_entity, gizmo_math::Vec3::ZERO, gizmo_math::Vec3::ZERO),
                        };
                        physics_world.joints.push(new_joint);
                    }
                }
            });
    }
}
