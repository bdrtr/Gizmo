
use crate::editor_state::EditorState;
use egui;
use gizmo_core::{EntityName, World};
use gizmo_physics::components::FluidSimulation;
use gizmo_renderer::components::ParticleEmitter;
use gizmo_ai::components::NavAgent;


pub fn draw_animation_player_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    let mut anim_players = world.borrow_mut::<gizmo_renderer::components::AnimationPlayer>();
    if let Some(player) = anim_players.get_mut(entity_id.id()) {
        egui::CollapsingHeader::new("🏃 Animation Player")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Speed:");
                    ui.add(egui::DragValue::new(&mut player.speed).speed(0.1));
                });
                ui.checkbox(&mut player.loop_anim, "Loop Animation");
                
                let num_anims = player.animations.len();
                if num_anims > 0 {
                    ui.label(format!("Animations ({}):", num_anims));
                    
                    let current_anim = player.active_animation;
                    let mut selected_anim = current_anim;
                    
                    let mut anim_name = format!("Anim {}", current_anim);
                    if let Some(clip) = player.animations.get(current_anim) {
                        anim_name = clip.name.clone();
                    }
                    
                    egui::ComboBox::from_id_source(format!("anim_select_{}", entity_id.id()))
                        .selected_text(anim_name)
                        .show_ui(ui, |ui| {
                            for i in 0..num_anims {
                                let name = if let Some(clip) = player.animations.get(i) {
                                    clip.name.clone()
                                } else {
                                    format!("Anim {}", i)
                                };
                                ui.selectable_value(&mut selected_anim, i, name);
                            }
                        });
                        
                    if selected_anim != current_anim {
                        // Yavaş geçiş (Cross-fade blending) başlat
                        player.prev_animation = Some(player.active_animation);
                        player.prev_time = player.current_time;
                        player.active_animation = selected_anim;
                        player.current_time = 0.0;
                        player.blend_time = 0.0;
                        player.blend_duration = 0.25; // Çeyrek saniyede blend
                    }
                    
                    // Timeline progress slider
                    if let Some(clip) = player.animations.get(player.active_animation) {
                        let duration = clip.duration;
                        ui.horizontal(|ui| {
                            let is_playing = player.speed != 0.0;
                            let play_icon = if is_playing { "⏸" } else { "▶" };
                            
                            if ui.button(play_icon).clicked() {
                                if is_playing {
                                    player.speed = 0.0;
                                } else {
                                    player.speed = 1.0;
                                }
                            }
                            if ui.button("⏹").clicked() {
                                player.speed = 0.0;
                                player.current_time = 0.0;
                            }
                            
                            ui.add(egui::Slider::new(&mut player.current_time, 0.0..=duration).show_value(true).text("s"));
                        });
                    }
                } else {
                    ui.label(egui::RichText::new("⚠️ Modelde animasyon bulunamadı").color(egui::Color32::YELLOW));
                }
            });
    }

    let skeletons = world.borrow::<gizmo_renderer::components::Skeleton>();
    if let Some(skel) = skeletons.get(entity_id.id()) {
        egui::CollapsingHeader::new("🦴 Skeleton")
            .default_open(false)
            .show(ui, |ui| {
                ui.label(format!("Joints: {}", skel.hierarchy.joints.len()));
            });
    }
}


pub fn draw_name_section(
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


pub fn draw_particle_emitter_section(
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


pub fn draw_script_section(
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
                        let text = std::fs::read_to_string(&script.file_path).unwrap_or_else(|_| "".to_string());
                        pending_text = Some(text);
                        state.log_info("✏️ Düzenle butonuna basıldı! Dosya okunmaya çalışılıyor...");
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
        state.script.open = true; // Request opening the tab safely
    }
}


pub fn draw_terrain_section(
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


pub fn draw_fluid_section(
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


pub fn draw_ai_section(
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


pub fn draw_reflection_section(
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


pub fn draw_hitbox_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    let mut hitboxes = world.borrow_mut::<gizmo_physics::components::Hitbox>();
    if let Some(hitbox) = hitboxes.get_mut(entity_id.id()) {
        egui::CollapsingHeader::new("🥊 Hitbox")
            .default_open(true)
            .show(ui, |ui| {
                ui.checkbox(&mut hitbox.active, "Aktif (Vurabilir)");
                ui.horizontal(|ui| {
                    ui.label("Damage:");
                    ui.add(egui::DragValue::new(&mut hitbox.damage).speed(1.0));
                });
                
                ui.label("Offset:");
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut hitbox.offset.x).speed(0.1).prefix("X: "));
                    ui.add(egui::DragValue::new(&mut hitbox.offset.y).speed(0.1).prefix("Y: "));
                    ui.add(egui::DragValue::new(&mut hitbox.offset.z).speed(0.1).prefix("Z: "));
                });
                
                ui.label("Half Extents (Boyut):");
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut hitbox.half_extents.x).speed(0.1).prefix("X: "));
                    ui.add(egui::DragValue::new(&mut hitbox.half_extents.y).speed(0.1).prefix("Y: "));
                    ui.add(egui::DragValue::new(&mut hitbox.half_extents.z).speed(0.1).prefix("Z: "));
                });
                
                if ui.button("🗑 Bileşeni Sil").clicked() {
                    _state.remove_component_request = Some((entity_id, "Hitbox".to_string()));
                }
            });
    }
}


pub fn draw_hurtbox_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    let mut hurtboxes = world.borrow_mut::<gizmo_physics::components::Hurtbox>();
    if let Some(hurtbox) = hurtboxes.get_mut(entity_id.id()) {
        egui::CollapsingHeader::new("🛡 Hurtbox")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Damage Multiplier:");
                    ui.add(egui::DragValue::new(&mut hurtbox.damage_multiplier).speed(0.1));
                });
                
                ui.label("Offset:");
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut hurtbox.offset.x).speed(0.1).prefix("X: "));
                    ui.add(egui::DragValue::new(&mut hurtbox.offset.y).speed(0.1).prefix("Y: "));
                    ui.add(egui::DragValue::new(&mut hurtbox.offset.z).speed(0.1).prefix("Z: "));
                });
                
                ui.label("Half Extents (Boyut):");
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut hurtbox.half_extents.x).speed(0.1).prefix("X: "));
                    ui.add(egui::DragValue::new(&mut hurtbox.half_extents.y).speed(0.1).prefix("Y: "));
                    ui.add(egui::DragValue::new(&mut hurtbox.half_extents.z).speed(0.1).prefix("Z: "));
                });
                
                if ui.button("🗑 Bileşeni Sil").clicked() {
                    _state.remove_component_request = Some((entity_id, "Hurtbox".to_string()));
                }
            });
    }
}


pub fn draw_bone_attachment_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    let mut attachments = world.borrow_mut::<gizmo_renderer::components::BoneAttachment>();
    if let Some(attachment) = attachments.get_mut(entity_id.id()) {
        egui::CollapsingHeader::new("🔗 Bone Attachment")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Target Entity ID:");
                    let mut tid = attachment.target_entity.id();
                    if ui.add(egui::DragValue::new(&mut tid)).changed() {
                        attachment.target_entity = gizmo_core::entity::Entity::new(tid, 0);
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Bone Index:");
                    ui.add(egui::DragValue::new(&mut attachment.bone_index));
                });
                
                if ui.button("🗑 Bileşeni Sil").clicked() {
                    _state.remove_component_request = Some((entity_id, "BoneAttachment".to_string()));
                }
            });
    }
}


pub fn draw_json_value(ui: &mut egui::Ui, name: &str, value: &mut serde_json::Value) -> bool {
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



pub fn draw_fighter_controller_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    let mut controllers = world.borrow_mut::<gizmo_physics::components::fighter::FighterController>();
    if let Some(fighter) = controllers.get_mut(entity_id.id()) {
        egui::CollapsingHeader::new("🥋 Fighter Controller")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Player ID:");
                    ui.add(egui::DragValue::new(&mut fighter.player_id).speed(1.0));
                });
                
                ui.horizontal(|ui| {
                    ui.label("Health:");
                    ui.add(egui::DragValue::new(&mut fighter.health).speed(1.0));
                });
                
                ui.checkbox(&mut fighter.is_blocking, "Blocking");
                ui.checkbox(&mut fighter.is_crouching, "Crouching");
                
                ui.horizontal(|ui| {
                    ui.label("Walk Speed:");
                    ui.add(egui::DragValue::new(&mut fighter.walk_speed).speed(0.1));
                });
                ui.horizontal(|ui| {
                    ui.label("Dash Speed:");
                    ui.add(egui::DragValue::new(&mut fighter.dash_speed).speed(0.1));
                });
                
                if ui.button("🗑 Bileşeni Sil").clicked() {
                    _state.remove_component_request = Some((entity_id, "FighterController".to_string()));
                }
            });
    }
}
