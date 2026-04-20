//! Component Inspector Panel — Sağ panel'de seçili entity'nin bileşenlerini gösterir ve düzenlenebilir

use crate::editor_state::EditorState;
use egui;
use gizmo_audio::AudioSource;
use gizmo_core::{EntityName, World};
use gizmo_math::{Vec3, Vec4};
use gizmo_physics::components::{RigidBody, Transform, Velocity};
use gizmo_physics::shape::Collider;
use gizmo_physics::vehicle::VehicleController;
use gizmo_renderer::components::{Camera, Material, ParticleEmitter, PointLight};

/// Inspector sekmesini çizer
pub fn ui_inspector(ui: &mut egui::Ui, world: &World, state: &mut EditorState) {
    let sel_len = state.selected_entities.len();
    if sel_len == 0 {
        ui.label(egui::RichText::new("Hiçbir obje seçili değil.").color(egui::Color32::GRAY));
        return;
    }

    if sel_len > 1 {
        ui.heading(format!("🔧 Çoklu Oje Seçili ({} adet)", sel_len));
        ui.separator();

        ui.label("Şu anda çoklu seçim modundasınız. Çoklu objelerin özelliklerinin aynı anda değiştirilmesi ilerleyen versiyonlarda desteklenecektir.");

        if ui
            .button(egui::RichText::new("🗑️ Seçili Objeleri Sil").color(egui::Color32::RED))
            .clicked()
        {
            for &entity in state.selected_entities.iter() {
                state.despawn_requests.push(entity);
            }
        }
        return;
    }

    // Tekli seçim durumu
    if let Some(&entity_id) = state.selected_entities.iter().next() {
        ui.heading(format!("🔧 Inspector [{}]", entity_id));

        if ui
            .button(egui::RichText::new("🗑️ Seçili Objeyi Sil").color(egui::Color32::RED))
            .clicked()
        {
            state.despawn_requests.push(entity_id);
        }

        ui.separator();

        egui::ScrollArea::vertical().show(ui, |ui| {
            // === ENTITY NAME ===
            draw_name_section(ui, world, entity_id);

            // === TRANSFORM ===
            draw_transform_section(ui, world, entity_id);

            // === VELOCITY ===
            draw_velocity_section(ui, world, entity_id);

            // === RIGIDBODY ===
            draw_rigidbody_section(ui, world, entity_id);

            // === COLLIDER ===
            draw_collider_section(ui, world, entity_id);

            // === CAMERA ===
            draw_camera_section(ui, world, entity_id);

            // === POINT LIGHT ===
            draw_point_light_section(ui, world, entity_id);

            // === MATERIAL ===
            draw_material_section(ui, world, entity_id);

            // === PARTICLE EMITTER ===
            draw_particle_emitter_section(ui, world, entity_id);

            // === VEHICLE CONTROLLER ===
            draw_vehicle_controller_section(ui, world, entity_id);
            draw_audio_source_section(ui, world, entity_id);
            draw_terrain_section(ui, world, entity_id, state);
            draw_script_section(ui, world, entity_id, state);

            ui.separator();

            // === ADD COMPONENT BUTONU ===
            ui.horizontal(|ui| {
                if ui.button("➕ Bileşen Ekle").clicked() {
                    state.add_component_open = !state.add_component_open;
                }
            });

            if state.add_component_open {
                draw_add_component_menu(ui, world, entity_id, state);
            }
        });
    } else {
        ui.heading("🔧 Inspector");
        ui.separator();
        ui.label("Bir entity seçin.");
        ui.label("");
        ui.label(egui::RichText::new("💡 İpucu: Sol panel'den\nbir entity'ye tıklayın.").weak());
    }
}

fn draw_name_section(ui: &mut egui::Ui, world: &World, entity_id: u32) {
    if let Some(mut names) = world.borrow_mut::<EntityName>().expect("ECS Aliasing Error") {
        if let Some(name) = names.get_mut(entity_id) {
            ui.horizontal(|ui| {
                ui.label("İsim:");
                ui.text_edit_singleline(&mut name.0);
            });
            ui.separator();
        }
    }
}

fn draw_transform_section(ui: &mut egui::Ui, world: &World, entity_id: u32) {
    if let Some(mut transforms) = world.borrow_mut::<Transform>().expect("ECS Aliasing Error") {
        if let Some(t) = transforms.get_mut(entity_id) {
            egui::CollapsingHeader::new("📐 Transform")
                .default_open(true)
                .show(ui, |ui| {
                    ui.label("Pozisyon:");
                    ui.horizontal(|ui| {
                        ui.label("X");
                        ui.add(egui::DragValue::new(&mut t.position.x).speed(0.1));
                        ui.label("Y");
                        ui.add(egui::DragValue::new(&mut t.position.y).speed(0.1));
                        ui.label("Z");
                        ui.add(egui::DragValue::new(&mut t.position.z).speed(0.1));
                    });

                    // Rotasyonu Euler açılarına çevir (daha kullanıcı dostu)
                    ui.label("Rotasyon (Euler°):");
                    let (mut rx, mut ry, mut rz) = quat_to_euler_deg(t.rotation);
                    let old = (rx, ry, rz);
                    ui.horizontal(|ui| {
                        ui.label("X");
                        ui.add(egui::DragValue::new(&mut rx).speed(1.0).suffix("°"));
                        ui.label("Y");
                        ui.add(egui::DragValue::new(&mut ry).speed(1.0).suffix("°"));
                        ui.label("Z");
                        ui.add(egui::DragValue::new(&mut rz).speed(1.0).suffix("°"));
                    });
                    if (rx, ry, rz) != old {
                        t.rotation = euler_deg_to_quat(rx, ry, rz);
                    }

                    ui.label("Ölçek:");
                    ui.horizontal(|ui| {
                        ui.label("X");
                        ui.add(
                            egui::DragValue::new(&mut t.scale.x)
                                .speed(0.05)
                                .clamp_range(0.01..=100.0),
                        );
                        ui.label("Y");
                        ui.add(
                            egui::DragValue::new(&mut t.scale.y)
                                .speed(0.05)
                                .clamp_range(0.01..=100.0),
                        );
                        ui.label("Z");
                        ui.add(
                            egui::DragValue::new(&mut t.scale.z)
                                .speed(0.05)
                                .clamp_range(0.01..=100.0),
                        );
                    });
                });
            ui.separator();
        }
    }
}

fn draw_velocity_section(ui: &mut egui::Ui, world: &World, entity_id: u32) {
    if let Some(mut velocities) = world.borrow_mut::<Velocity>().expect("ECS Aliasing Error") {
        if let Some(v) = velocities.get_mut(entity_id) {
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

fn draw_rigidbody_section(ui: &mut egui::Ui, world: &World, entity_id: u32) {
    if let Some(mut rigidbodies) = world.borrow_mut::<RigidBody>().expect("ECS Aliasing Error") {
        if let Some(rb) = rigidbodies.get_mut(entity_id) {
            egui::CollapsingHeader::new("⚙️ RigidBody")
                .default_open(false)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Kütle:");
                        ui.add(
                            egui::DragValue::new(&mut rb.mass)
                                .speed(0.5)
                                .clamp_range(0.0..=100000.0),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label("Sekme:");
                        ui.add(
                            egui::DragValue::new(&mut rb.restitution)
                                .speed(0.01)
                                .clamp_range(0.0..=1.0),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label("Sürtünme:");
                        ui.add(
                            egui::DragValue::new(&mut rb.friction)
                                .speed(0.01)
                                .clamp_range(0.0..=2.0),
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

fn draw_collider_section(ui: &mut egui::Ui, world: &World, entity_id: u32) {
    if let Some(mut colliders) = world.borrow_mut::<Collider>().expect("ECS Aliasing Error") {
        if let Some(collider) = colliders.get_mut(entity_id) {
            egui::CollapsingHeader::new("🛡️ Collider")
                .default_open(true)
                .show(ui, |ui| {
                    match &mut collider.shape {
                        gizmo_physics::shape::ColliderShape::Aabb(aabb) => {
                            ui.label("Mod: Kutu (AABB)");
                            ui.horizontal(|ui| {
                                ui.label("Extents:");
                                ui.add(egui::DragValue::new(&mut aabb.half_extents.x).speed(0.1).prefix("X: "));
                                ui.add(egui::DragValue::new(&mut aabb.half_extents.y).speed(0.1).prefix("Y: "));
                                ui.add(egui::DragValue::new(&mut aabb.half_extents.z).speed(0.1).prefix("Z: "));
                            });
                        }
                        gizmo_physics::shape::ColliderShape::Sphere(sphere) => {
                            ui.label("Mod: Küre (Sphere)");
                            ui.horizontal(|ui| {
                                ui.label("Yarıçap:");
                                ui.add(egui::DragValue::new(&mut sphere.radius).speed(0.1));
                            });
                        }
                        gizmo_physics::shape::ColliderShape::Capsule(capsule) => {
                            ui.label("Mod: Kapsül");
                            ui.horizontal(|ui| {
                                ui.label("Yarıçap:");
                                ui.add(egui::DragValue::new(&mut capsule.radius).speed(0.1));
                                ui.label("Y. Yükseklik:");
                                ui.add(egui::DragValue::new(&mut capsule.half_height).speed(0.1));
                            });
                        }
                        other => {
                            ui.label(format!("Şekil: {:?}", other));
                        }
                    }
                });
            ui.separator();
        }
    }
}

fn draw_camera_section(ui: &mut egui::Ui, world: &World, entity_id: u32) {
    if let Some(mut cameras) = world.borrow_mut::<Camera>().expect("ECS Aliasing Error") {
        if let Some(cam) = cameras.get_mut(entity_id) {
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
                                    .clamp_range(10.0..=120.0)
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
                                .clamp_range(0.001..=10.0),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label("Far:");
                        ui.add(
                            egui::DragValue::new(&mut cam.far)
                                .speed(10.0)
                                .clamp_range(10.0..=50000.0),
                        );
                    });
                    ui.checkbox(&mut cam.primary, "Ana Kamera");
                });
            ui.separator();
        }
    }
}

fn draw_point_light_section(ui: &mut egui::Ui, world: &World, entity_id: u32) {
    if let Some(mut lights) = world.borrow_mut::<PointLight>().expect("ECS Aliasing Error") {
        if let Some(light) = lights.get_mut(entity_id) {
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
                                .clamp_range(0.0..=100.0),
                        );
                    });
                });
            ui.separator();
        }
    }
}

fn draw_material_section(ui: &mut egui::Ui, world: &World, entity_id: u32) {
    if let Some(mut materials) = world.borrow_mut::<Material>().expect("ECS Aliasing Error") {
        if let Some(mat) = materials.get_mut(entity_id) {
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
                                .clamp_range(0.0..=1.0),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label("Pürüzlülük (Roughness):");
                        ui.add(
                            egui::DragValue::new(&mut mat.roughness)
                                .speed(0.01)
                                .clamp_range(0.0..=1.0),
                        );
                    });

                    let mode = if mat.unlit == 0.0 {
                        "PBR"
                    } else if mat.unlit == 1.0 {
                        "Unlit"
                    } else {
                        "Skybox"
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

fn draw_particle_emitter_section(ui: &mut egui::Ui, world: &World, entity_id: u32) {
    if let Some(mut emitters) = world.borrow_mut::<ParticleEmitter>().expect("ECS Aliasing Error") {
        if let Some(emitter) = emitters.get_mut(entity_id) {
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

fn draw_vehicle_controller_section(ui: &mut egui::Ui, world: &World, entity_id: u32) {
    if let Some(mut vehicles) = world.borrow_mut::<VehicleController>().expect("ECS Aliasing Error") {
        if let Some(vrc) = vehicles.get_mut(entity_id) {
            egui::CollapsingHeader::new("🚗 Vehicle")
                .default_open(false)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Motor Gücü:");
                        ui.add(egui::Slider::new(&mut vrc.engine_force, 0.0..=50000.0));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Fren Gücü:");
                        ui.add(egui::Slider::new(&mut vrc.brake_force, 0.0..=50000.0));
                    });
                });
            ui.separator();
        }
    }
}

fn draw_script_section(ui: &mut egui::Ui, world: &World, entity_id: u32, state: &mut EditorState) {
    if let Some(mut scripts) = world.borrow_mut::<gizmo_scripting::engine::Script>().expect("ECS Aliasing Error") {
        // Drop wrapper ile borrow bitimine izin verelim diye clone alıyoruz ama
        // text_edit bağlamak için referans lazım.
        if let Some(script) = scripts.get_mut(entity_id) {
            egui::CollapsingHeader::new("📜 Script")
                .default_open(true)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Dosya Yolu:");
                        ui.text_edit_singleline(&mut script.file_path);
                    });
                    
                    if ui.button("✏️ Düzenle (Kod Editörü)").clicked() {
                        if std::path::Path::new(&script.file_path).exists() {
                            if let Ok(content) = std::fs::read_to_string(&script.file_path) {
                                state.active_script_content = content;
                            }
                        } else {
                            // Dosya yoksa boş
                            state.active_script_content = "-- Gizmo Script\nfunction on_update(dt)\n\nend".to_string();
                        }
                        state.active_script_path = script.file_path.clone();
                        state.script_editor_open = true;
                    }
                });
            ui.separator();
        }
    }
}

fn draw_add_component_menu(
    ui: &mut egui::Ui,
    _world: &World,
    _entity_id: u32,
    state: &mut EditorState,
) {
    ui.group(|ui| {
        ui.horizontal(|ui| {
            ui.label("Eklenecek Bileşen:");

            let components = [
                "Transform",
                "Velocity",
                "RigidBody",
                "Collider",
                "Camera",
                "PointLight",
                "Material",
                "ParticleEmitter",
                "Script",
                "AudioSource",
                "Terrain",
                "VehicleController",
            ];

            egui::ComboBox::from_id_source("add_comp_combo")
                .selected_text("Seçiniz...")
                .show_ui(ui, |ui| {
                    for comp_name in &components {
                        if ui.selectable_label(false, *comp_name).clicked() {
                            state.add_component_open = false;
                            state.add_component_request = Some((_entity_id, comp_name.to_string()));
                        }
                    }
                });
        });
    });
}

fn draw_audio_source_section(ui: &mut egui::Ui, world: &World, entity_id: u32) {
    if let Some(mut audios) = world.borrow_mut::<AudioSource>().expect("ECS Aliasing Error") {
        if let Some(audio) = audios.get_mut(entity_id) {
            egui::CollapsingHeader::new("🔊 AudioSource")
                .default_open(true)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Ses Dosyası:");
                        ui.label(
                            egui::RichText::new(&audio.sound_name)
                                .color(egui::Color32::from_rgb(100, 200, 255)),
                        );
                    });
                    ui.checkbox(&mut audio.is_3d, "3D Uzamsal Ses");
                    ui.checkbox(&mut audio.loop_sound, "Döngü (Loop)");

                    ui.horizontal(|ui| {
                        ui.label("Ses Şiddeti (Volume):");
                        ui.add(egui::Slider::new(&mut audio.volume, 0.0..=5.0));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Ses İnceliği (Pitch):");
                        ui.add(egui::Slider::new(&mut audio.pitch, 0.1..=3.0));
                    });

                    if audio.is_3d {
                        ui.horizontal(|ui| {
                            ui.label("Maksimum Mesafe:");
                            ui.add(
                                egui::Slider::new(&mut audio.max_distance, 1.0..=1000.0)
                                    .suffix("m"),
                            );
                        });
                    }
                });
            ui.separator();
        }
    }
}

fn draw_terrain_section(ui: &mut egui::Ui, world: &World, entity_id: u32, state: &mut EditorState) {
    if let Some(mut terrains) = world.borrow_mut::<gizmo_renderer::components::Terrain>().expect("ECS Aliasing Error") {
        if let Some(terrain) = terrains.get_mut(entity_id) {
            let mut changed = false;
            egui::CollapsingHeader::new("🏔 Terrain")
                .default_open(true)
                .show(ui, |ui| {
                    ui.label(format!("Heightmap: {}", terrain.heightmap_path));
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

// === YARDIMCI FONKSİYONLAR ===

fn quat_to_euler_deg(q: gizmo_math::Quat) -> (f32, f32, f32) {
    // Quaternion → Euler (XYZ sırası) dönüşümü
    let sinr_cosp = 2.0 * (q.w * q.x + q.y * q.z);
    let cosr_cosp = 1.0 - 2.0 * (q.x * q.x + q.y * q.y);
    let roll = sinr_cosp.atan2(cosr_cosp);

    let sinp = 2.0 * (q.w * q.y - q.z * q.x);
    let pitch = if sinp.abs() >= 1.0 {
        std::f32::consts::FRAC_PI_2.copysign(sinp)
    } else {
        sinp.asin()
    };

    let siny_cosp = 2.0 * (q.w * q.z + q.x * q.y);
    let cosy_cosp = 1.0 - 2.0 * (q.y * q.y + q.z * q.z);
    let yaw = siny_cosp.atan2(cosy_cosp);

    (roll.to_degrees(), pitch.to_degrees(), yaw.to_degrees())
}

fn euler_deg_to_quat(rx: f32, ry: f32, rz: f32) -> gizmo_math::Quat {
    let (rx, ry, rz) = (rx.to_radians(), ry.to_radians(), rz.to_radians());

    let (sx, cx) = (rx * 0.5).sin_cos();
    let (sy, cy) = (ry * 0.5).sin_cos();
    let (sz, cz) = (rz * 0.5).sin_cos();

    gizmo_math::Quat::from_xyzw(
        sx * cy * cz - cx * sy * sz,
        cx * sy * cz + sx * cy * sz,
        cx * cy * sz - sx * sy * cz,
        cx * cy * cz + sx * sy * sz,
    )
}
