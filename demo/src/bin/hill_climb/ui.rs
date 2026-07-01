use gizmo::prelude::*;
use super::DemoState;

pub(super) fn ui_debug_panel(world: &mut World, state: &mut DemoState, ctx: &gizmo::egui::Context) {
    gizmo::egui::Window::new("🛠 Gizmo Debugger")
        .default_pos([10.0, 10.0])
        .show(ctx, |ui| {
            // --- METRICS ---
            ui.heading("Performance");
            if let Ok(time) = world.try_get_resource::<gizmo::core::time::Time>() {
                ui.label(format!("FPS: {:.0}", time.fps()));
                ui.label(format!("Frame Time: {:.2} ms", time.raw_dt() * 1000.0));
            }
            ui.separator();

            // --- PHYSICS ---
            if let Ok(mut phys) =
                world.try_get_resource_mut::<gizmo::physics::world::PhysicsWorld>()
            {
                ui.heading("Physics Engine");
                ui.checkbox(
                    &mut state.show_physics_debug,
                    "Gizmo Debug Draw (Görsel Çarpışma)",
                );
                ui.horizontal(|ui| {
                    ui.checkbox(&mut phys.is_paused, "Pause (P)");
                    if ui.button("Step (O)").clicked() {
                        phys.step_once = true;
                    }
                    if ui.button("Rewind (R)").clicked() {
                        phys.rewind_requested = true;
                    }
                });
                ui.label(format!("Active Rigidbodies: {}", phys.rigid_bodies.len()));
                ui.label(format!(
                    "History Buffer: {} / {}",
                    phys.history.len(),
                    phys.max_history_frames
                ));
            }

            ui.separator();
            ui.label("Use W/A/S/D to drive");

            ui.separator();
            ui.heading("Araba Görünürlüğü");
            let mut show_car = state.show_car;
            if ui.checkbox(&mut show_car, "Arabayı Göster").changed() {
                state.show_car = show_car;
                if show_car {
                    world.add_component(state.car_entity, MeshRenderer::new());
                    for w in &state.wheel_entities {
                        world.add_component(*w, MeshRenderer::new());
                    }
                } else {
                    world.remove_component::<MeshRenderer>(state.car_entity);
                    for w in &state.wheel_entities {
                        world.remove_component::<MeshRenderer>(*w);
                    }
                }
            }

            ui.separator();
            ui.heading("Görsel Kalite / Post-Process");
            ui.add(
                gizmo::egui::Slider::new(&mut state.post_process.bloom_intensity, 0.0..=5.0)
                    .text("Bloom Yoğunluğu"),
            );
            ui.add(
                gizmo::egui::Slider::new(&mut state.post_process.bloom_threshold, 0.0..=2.0)
                    .text("Bloom Eşiği"),
            );
            ui.add(
                gizmo::egui::Slider::new(&mut state.post_process.exposure, 0.1..=5.0)
                    .text("Exposure (Pozlama)"),
            );
            ui.add(
                gizmo::egui::Slider::new(&mut state.post_process.chromatic_aberration, 0.0..=0.05)
                    .text("Kromatik Sapma"),
            );
            ui.add(
                gizmo::egui::Slider::new(&mut state.post_process.vignette_intensity, 0.0..=1.0)
                    .text("Vignette"),
            );
            ui.add(
                gizmo::egui::Slider::new(&mut state.post_process.film_grain_intensity, 0.0..=0.5)
                    .text("Film Greni"),
            );

            ui.label("Depth of Field (Alan Derinliği)");
            ui.add(
                gizmo::egui::Slider::new(&mut state.post_process.dof_focus_dist, 0.0..=100.0)
                    .text("Odak Uzaklığı"),
            );
            ui.add(
                gizmo::egui::Slider::new(&mut state.post_process.dof_focus_range, 0.0..=50.0)
                    .text("Odak Derinliği"),
            );
            ui.add(
                gizmo::egui::Slider::new(&mut state.post_process.dof_blur_size, 0.0..=5.0)
                    .text("Bulanıklık Miktarı"),
            );
        });

    gizmo::egui::Window::new("Araç Dinamikleri (Vehicle Tuning)")
        .default_pos([10.0, 450.0])
        .show(ctx, |ui| {
            let mut vehicles = world.borrow_mut::<gizmo::physics::vehicle::VehicleController>();
            if let Some(mut vehicle) = vehicles.get_mut(state.car_entity.id()) {
                ui.heading("Telemetri (Canlı Veri)");
                ui.label(format!("Hız: {:.1} km/h", vehicle.current_speed_kmh.abs()));
                ui.label(format!("Motor Devri: {:.0} RPM", vehicle.engine_rpm));
                let gear_str = if vehicle.reverse_input {
                    "R".to_string()
                } else if vehicle.current_gear <= 1 {
                    "N".to_string()
                } else {
                    format!("{}", vehicle.current_gear - 1)
                };
                ui.label(format!("Vites: {}", gear_str));

                ui.separator();
                ui.heading("Motor & Şanzıman");
                ui.add(
                    gizmo::egui::Slider::new(
                        &mut vehicle.tuning.max_engine_torque,
                        1000.0..=100000.0,
                    )
                    .text("Max Motor Torku"),
                );
                ui.add(
                    gizmo::egui::Slider::new(&mut vehicle.tuning.max_rpm, 1000.0..=12000.0)
                        .text("Max RPM"),
                );
                ui.checkbox(&mut vehicle.auto_shift, "Otomatik Vites");
                ui.add(
                    gizmo::egui::Slider::new(&mut vehicle.tuning.upshift_rpm, 3000.0..=12000.0)
                        .text("Vites Yükseltme (RPM)"),
                );
                ui.add(
                    gizmo::egui::Slider::new(&mut vehicle.tuning.downshift_rpm, 1000.0..=8000.0)
                        .text("Vites Düşürme (RPM)"),
                );

                ui.heading("Süspansiyon & Tekerlekler");
                // Update all wheels
                let mut stiffness = vehicle.wheels[0].suspension_stiffness;
                let mut damping = vehicle.wheels[0].suspension_damping;
                let mut rest_length = vehicle.wheels[0].suspension_rest_length;
                let current_radius = vehicle.wheels[0].radius;
                let mut current_diameter_inches = current_radius * 2.0 / 0.0254;

                if ui
                    .add(
                        gizmo::egui::Slider::new(&mut current_diameter_inches, 14.0..=40.0)
                            .text("Tekerlek Çapı (İnç)"),
                    )
                    .changed()
                {
                    let new_radius = current_diameter_inches * 0.0254 / 2.0;
                    for w in vehicle.wheels.iter_mut() {
                        w.radius = new_radius;
                    }
                    state.update_wheel_radius = Some(new_radius);
                }
                if ui
                    .add(
                        gizmo::egui::Slider::new(&mut stiffness, 10000.0..=100000.0)
                            .text("Yay Sertliği (Stiffness)"),
                    )
                    .changed()
                {
                    for w in vehicle.wheels.iter_mut() {
                        w.suspension_stiffness = stiffness;
                    }
                }
                if ui
                    .add(
                        gizmo::egui::Slider::new(&mut damping, 1000.0..=20000.0)
                            .text("Amortisör (Damping)"),
                    )
                    .changed()
                {
                    for w in vehicle.wheels.iter_mut() {
                        w.suspension_damping = damping;
                    }
                }
                if ui
                    .add(
                        gizmo::egui::Slider::new(&mut rest_length, 0.2..=2.0)
                            .text("Yerden Yükseklik (Rest Len)"),
                    )
                    .changed()
                {
                    for w in vehicle.wheels.iter_mut() {
                        w.suspension_rest_length = rest_length;
                    }
                }
            }

            ui.separator();
            ui.heading("Şasi & Ağırlık");
            let mut bodies = world.borrow_mut::<gizmo::physics::RigidBody>();
            if let Some(mut rb) = bodies.get_mut(state.car_entity.id()) {
                let mut com_y = rb.center_of_mass.y;
                let mut mass = rb.mass;

                if ui
                    .add(
                        gizmo::egui::Slider::new(&mut com_y, -2.0..=1.0)
                            .text("Ağırlık Merkezi (Y)"),
                    )
                    .changed()
                {
                    rb.center_of_mass.y = com_y;
                }
                if ui
                    .add(
                        gizmo::egui::Slider::new(&mut mass, 500.0..=5000.0)
                            .text("Araç Kütlesi (KG)"),
                    )
                    .changed()
                {
                    rb.mass = mass;
                }
            }
        });
}
