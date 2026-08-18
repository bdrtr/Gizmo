
use crate::editor_state::EditorState;
use egui;
use gizmo_core::{EntityName, World};
use gizmo_physics_core::components::FluidSimulation;
use gizmo_renderer::components::ParticleEmitter;
use gizmo_ai::components::NavAgent;


/// The inspector's animation-player rows: the clips on the entity and which one is playing.
pub fn draw_animation_player_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    // SAFETY: editor UI runs single-threaded in the egui draw; no concurrent World access.
    let mut anim_players = unsafe { world.borrow_mut_unchecked::<gizmo_renderer::components::AnimationPlayer>() };
    if let Some(mut player) = anim_players.get_mut(entity_id.id()) {
        egui::CollapsingHeader::new(crate::theme::section_title("Animation Player"))
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
                    
                    egui::ComboBox::from_id_salt(format!("anim_select_{}", entity_id.id()))
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
        egui::CollapsingHeader::new(crate::theme::section_title("Skeleton"))
            .default_open(true)
            .show(ui, |ui| {
                ui.label(format!("Joints: {}", skel.hierarchy.joints.len()));
            });
    }
}


/// The name row at the top of the inspector, and the rename it commits.
pub fn draw_name_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    // SAFETY: editor UI runs single-threaded in the egui draw; no concurrent World access.
    let mut names = unsafe { world.borrow_mut_unchecked::<EntityName>() };
    {
        if let Some(mut name) = names.get_mut(entity_id.id()) {
            ui.horizontal(|ui| {
                ui.label("İsim:");
                ui.text_edit_singleline(&mut name.0);
            });
            ui.separator();
        }
    }
}


/// The particle-emitter rows: rate, lifetime, velocity and the emitter's shape.
pub fn draw_particle_emitter_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    // SAFETY: editor UI runs single-threaded in the egui draw; no concurrent World access.
    let mut emitters = unsafe { world.borrow_mut_unchecked::<ParticleEmitter>() };
    {
        if let Some(mut emitter) = emitters.get_mut(entity_id.id()) {
            egui::CollapsingHeader::new(crate::theme::section_title("Particle Emitter"))
                .default_open(true)
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


#[cfg(not(target_arch = "wasm32"))]
/// The script rows: which file is attached, and the properties it declares.
pub fn draw_script_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    state: &mut EditorState,
) {
    let mut pending_text = None;
    let mut file_path = String::new();
    // SAFETY: editor UI runs single-threaded in the egui draw; no concurrent World access.
    let mut scripts = unsafe { world.borrow_mut_unchecked::<gizmo_scripting::Script>() };
    {
        if let Some(mut script) = scripts.get_mut(entity_id.id()) {
            file_path = script.file_path.clone();
            egui::CollapsingHeader::new(crate::theme::section_title("Script"))
                .default_open(true)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Dosya Yolu:");
                        ui.text_edit_singleline(&mut script.file_path);
                    });

                    if ui.button("Düzenle").clicked() {
                        let text = std::fs::read_to_string(&script.file_path).unwrap_or_else(|_| "".to_string());
                        pending_text = Some(text);
                        state.log_info("Düzenle butonuna basıldı! Dosya okunmaya çalışılıyor...");
                    }

                    crate::inspector::script::draw_script_properties(
                        ui, world, &mut script, state,
                    );
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

/// The script rows on wasm, where there is no filesystem to browse: a stub that draws nothing.
///
/// It needs its own doc comment because `missing_docs` is per-target in exactly the way the wasm
/// clippy job exists to catch — the native lint never compiles this arm, so the crate read as
/// fully documented while this one function was not. See the `wasm` job in `.github/workflows/ci.yml`.
#[cfg(target_arch = "wasm32")]
pub fn draw_script_section(
    _ui: &mut egui::Ui,
    _world: &World,
    _entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {}


/// The terrain rows, including the generate request the studio acts on.
pub fn draw_terrain_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    state: &mut EditorState,
) {
    // SAFETY: editor UI runs single-threaded in the egui draw; no concurrent World access.
    let mut terrains = unsafe { world.borrow_mut_unchecked::<gizmo_renderer::components::Terrain>() };
    {
        if let Some(mut terrain) = terrains.get_mut(entity_id.id()) {
            let mut changed = false;
            egui::CollapsingHeader::new(crate::theme::section_title("Terrain"))
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


/// The fluid rows: which phase the entity is, and how it couples to the simulation.
pub fn draw_fluid_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    // SAFETY: editor UI runs single-threaded in the egui draw; no concurrent World access.
    let mut fluids = unsafe { world.borrow_mut_unchecked::<FluidSimulation>() };
    {
        if let Some(mut fluid) = fluids.get_mut(entity_id.id()) {
            egui::CollapsingHeader::new(crate::theme::section_title("SPH Fluid Simulation (GPU)"))
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


/// The AI rows: the navigation agent and its current target.
pub fn draw_ai_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    // SAFETY: editor UI runs single-threaded in the egui draw; no concurrent World access.
    let mut agents = unsafe { world.borrow_mut_unchecked::<NavAgent>() };
    {
        if let Some(mut agent) = agents.get_mut(entity_id.id()) {
            egui::CollapsingHeader::new(crate::theme::section_title("AI NavAgent"))
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
                        _ => "Bilinmiyor",
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


/// The reflection-probe rows.
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
        let types = world.entity_component_types(entity_id);

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


/// The fighting-game hitbox rows — the volume that deals damage.
pub fn draw_hitbox_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    // SAFETY: editor UI runs single-threaded in the egui draw; no concurrent World access.
    let mut hitboxes = unsafe { world.borrow_mut_unchecked::<gizmo_physics_core::components::Hitbox>() };
    if let Some(mut hitbox) = hitboxes.get_mut(entity_id.id()) {
        egui::CollapsingHeader::new(crate::theme::section_title("Hitbox"))
            .default_open(true)
            .show(ui, |ui| {
                ui.checkbox(&mut hitbox.active, "Aktif (Vurabilir)");
                ui.horizontal(|ui| {
                    ui.label("Damage:");
                    ui.add(egui::DragValue::new(&mut hitbox.damage).speed(1.0));
                });

                // Which move owns this box. Empty = every move, which is right for a fighter with
                // one hitbox and wrong the moment there are two — a jab's fist and a kick's foot
                // would otherwise both go live on either move.
                ui.horizontal(|ui| {
                    ui.label("Hareket (boş = hepsi):");
                    let mut name = hitbox.move_name.clone().unwrap_or_default();
                    if ui.text_edit_singleline(&mut name).changed() {
                        hitbox.move_name = if name.trim().is_empty() {
                            None
                        } else {
                            Some(name)
                        };
                    }
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


/// The fighting-game hurtbox rows — the volume that takes it.
pub fn draw_hurtbox_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    // SAFETY: editor UI runs single-threaded in the egui draw; no concurrent World access.
    let mut hurtboxes = unsafe { world.borrow_mut_unchecked::<gizmo_physics_core::components::Hurtbox>() };
    if let Some(mut hurtbox) = hurtboxes.get_mut(entity_id.id()) {
        egui::CollapsingHeader::new(crate::theme::section_title("Hurtbox"))
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


/// The bone-attachment rows: which skeleton bone the entity rides.
pub fn draw_bone_attachment_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    // SAFETY: editor UI runs single-threaded in the egui draw; no concurrent World access.
    let mut attachments = unsafe { world.borrow_mut_unchecked::<gizmo_renderer::components::BoneAttachment>() };
    if let Some(mut attachment) = attachments.get_mut(entity_id.id()) {
        egui::CollapsingHeader::new(crate::theme::section_title("Bone Attachment"))
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


/// Draws an editable row for one `serde_json::Value`, recursing into objects and arrays.
///
/// This is how the inspector edits a component whose type it does not know: the component is
/// round-tripped through JSON, edited here, and written back by the function pointer stored
/// beside it in `pending_json_updates`. Returns whether anything changed.
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



/// The fighter-controller rows: health, stance, and — read-only, because the engine's fight clock
/// owns them — the move in flight and the frame counters that freeze it.
///
/// The live half was missing entirely: the section drew six authoring fields and not one of
/// `active_move`, `current_move_frame`, `hitstop_frames` or `hitstun_frames`, so
/// `fighter_frame_system` could count a whole move out and the inspector showed nothing moving.
/// It did not draw `max_health` either — the denominator of the health bar the studio's own fight
/// HUD paints from the field right above it.
pub fn draw_fighter_controller_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    // SAFETY: editor UI runs single-threaded in the egui draw; no concurrent World access.
    let mut controllers = unsafe { world.borrow_mut_unchecked::<gizmo_physics_core::components::FighterController>() };
    if let Some(mut fighter) = controllers.get_mut(entity_id.id()) {
        egui::CollapsingHeader::new(crate::theme::section_title("Fighter Controller"))
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Player ID:");
                    ui.add(egui::DragValue::new(&mut fighter.player_id).speed(1.0));
                });
                
                ui.horizontal(|ui| {
                    ui.label("Health:");
                    ui.add(egui::DragValue::new(&mut fighter.health).speed(1.0));
                    ui.label("/");
                    ui.add(egui::DragValue::new(&mut fighter.max_health).speed(1.0));
                });
                
                ui.checkbox(&mut fighter.is_blocking, "Blocking");
                ui.checkbox(&mut fighter.is_crouching, "Crouching");

                // ── The live half: what the fight clock is doing to this fighter right now ──
                //
                // Read-only on purpose. `fighter_frame_system` writes these every fixed step, so
                // a drag here would be overwritten before the next frame is drawn — a control
                // that fights the engine reads as a broken control.
                ui.separator();
                match &fighter.active_move {
                    Some(m) => {
                        let fd = &m.frame_data;
                        let total = fd.total_frames();
                        let frame = fighter.current_move_frame;
                        let phase = if frame < fd.startup {
                            "startup"
                        } else if frame < fd.startup + fd.active {
                            "AKTİF"
                        } else {
                            "recovery"
                        };
                        ui.label(format!(
                            "Hareket: {}  —  kare {}/{}  ({})",
                            if m.name.is_empty() { "(isimsiz)" } else { &m.name },
                            frame,
                            total,
                            phase
                        ));
                        ui.label(format!(
                            "  {} startup / {} aktif / {} recovery · {:.0} hasar",
                            fd.startup, fd.active, fd.recovery, fd.damage
                        ));
                    }
                    None => {
                        ui.label("Hareket: yok (nötr)");
                    }
                }
                ui.label(format!(
                    "Hitstop: {} · Hitstun: {}{}",
                    fighter.hitstop_frames,
                    fighter.hitstun_frames,
                    if fighter.is_locked() { "  — KİLİTLİ" } else { "" }
                ));
                ui.separator();
                
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

/// The prototype's MESH RENDERER section — for now, the one control on it that the engine can
/// answer: whether this object casts a shadow, and whether it is drawn.
///
/// The prototype also shows the mesh and material asset names and an LOD bias. Those need asset
/// identities and a bias field that do not exist yet (`docs/ENGINE.md` §3), and a section
/// that displayed them as blanks would be worse than one that shows what it has.
pub fn draw_mesh_renderer_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    use gizmo_renderer::components::{MeshRenderer, ShadowCasting};

    // SAFETY: the editor UI runs single-threaded inside the egui draw; no concurrent World access.
    let mut renderers = unsafe { world.borrow_mut_unchecked::<MeshRenderer>() };
    let Some(mut renderer) = renderers.get_mut(entity_id.id()) else {
        return;
    };

    egui::CollapsingHeader::new(crate::theme::section_title("Mesh Renderer"))
        .default_open(true)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("LOD bias:");
                ui.add(
                    egui::DragValue::new(&mut renderer.lod_bias)
                        .speed(0.05)
                        .range(0.1..=8.0),
                )
                .on_hover_text("1'in üstü daha uzakta bile yüksek detayı korur");
            });
            ui.horizontal(|ui| {
                ui.label("Cast shadow:");
                crate::theme::segmented(
                    ui,
                    &mut renderer.shadows,
                    &[
                        (ShadowCasting::On, "On"),
                        (ShadowCasting::Off, "Off"),
                        (ShadowCasting::Only, "Only"),
                    ],
                );
            });
        });
    ui.separator();
}


#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_physics_core::components::fighter::{CombatMove, FighterController, FrameData};

    /// Every string the frame painted, flattened out of the nested `Shape::Vec`s.
    fn painted_text(output: &egui::FullOutput) -> Vec<String> {
        fn scan(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| scan(s, out)),
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                _ => {}
            }
        }
        let mut out = Vec::new();
        output.shapes.iter().for_each(|s| scan(&s.shape, &mut out));
        out
    }

    /// **The inspector shows what the fight clock is doing.**
    ///
    /// It showed none of it: six authoring fields and not one of `active_move`,
    /// `current_move_frame`, `hitstop_frames` or `hitstun_frames`. So `fighter_frame_system` could
    /// count a jab from startup through recovery and the panel a user was staring at never moved —
    /// the clock was correct and unobservable, which for an editor is most of the way to absent.
    ///
    /// Driven headlessly (`Context::run_ui`), so it needs no window and no GPU and asserts the
    /// text the frame actually painted rather than that the code was called.
    #[test]
    fn the_fighter_section_paints_the_move_in_flight_and_its_counters() {
        let mut world = World::new();
        let entity = world.spawn();
        // `FrameData`/`CombatMove` are `#[non_exhaustive]`, so from outside their crate they are
        // built by `default()` and assigned into — the same shape the scripting crate's
        // `SetFighterMove` handler uses.
        let mut frame_data = FrameData::default();
        frame_data.startup = 5;
        frame_data.active = 3;
        frame_data.recovery = 2;
        let mut combat_move = CombatMove::default();
        combat_move.name = "Jab".to_string();
        combat_move.frame_data = frame_data;

        let mut fighter = FighterController::new(2);
        fighter.active_move = Some(combat_move);
        // Frame 6 of 10: inside the 5..8 active window, and frozen for two more frames.
        fighter.current_move_frame = 6;
        fighter.apply_hitstop(2);
        world.add_component(entity, fighter);

        let mut state = EditorState::default();
        let ctx = egui::Context::default();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            draw_fighter_controller_section(ui, &world, entity, &mut state);
        });
        let painted = painted_text(&output).join("\n");
        output.drop_without_applying_deltas();

        assert!(
            painted.contains("Jab") && painted.contains("kare 6/10"),
            "the move in flight and how far through it the fighter is must both be on screen:\n{painted}"
        );
        assert!(
            painted.contains("AKTİF"),
            "frame 6 of a 5/3/2 move is inside its hitting window, and that is the one state a \
             fighting-game author is looking for:\n{painted}"
        );
        assert!(
            painted.contains("Hitstop: 2") && painted.contains("KİLİTLİ"),
            "the freeze counter and the lock it implies must be readable:\n{painted}"
        );
    }

    /// A neutral fighter says so rather than showing a stale move.
    #[test]
    fn the_fighter_section_says_when_there_is_no_move() {
        let mut world = World::new();
        let entity = world.spawn();
        world.add_component(entity, FighterController::new(1));

        let mut state = EditorState::default();
        let ctx = egui::Context::default();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            draw_fighter_controller_section(ui, &world, entity, &mut state);
        });
        let painted = painted_text(&output).join("\n");
        output.drop_without_applying_deltas();

        assert!(
            painted.contains("nötr"),
            "a fighter with no active move must say so:\n{painted}"
        );
        assert!(
            !painted.contains("KİLİTLİ"),
            "and must not claim to be locked:\n{painted}"
        );
    }
}
