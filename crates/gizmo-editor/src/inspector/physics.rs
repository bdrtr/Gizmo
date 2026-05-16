
use crate::editor_state::EditorState;
use egui;
use gizmo_core::World;
use gizmo_physics::components::{RigidBody, Velocity};
use gizmo_physics::shape::Collider;


pub fn draw_velocity_section(
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


pub fn draw_rigidbody_section(
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


pub fn draw_collider_section(
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


pub fn draw_joint_section(
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


