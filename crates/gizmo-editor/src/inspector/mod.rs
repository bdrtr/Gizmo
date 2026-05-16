pub mod transform;
pub mod physics;
pub mod light;
pub mod camera;
pub mod material;
pub mod misc;
pub mod environment;
pub mod menu;

use crate::editor_state::EditorState;
use gizmo_core::World;

pub fn ui_inspector(ui: &mut egui::Ui, world: &World, state: &mut EditorState) {
    let sel_len = state.selection.entities.len();
    if sel_len == 0 {
        environment::draw_environment_settings(ui, state);
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
        if sel_len == 1 {
            misc::draw_name_section(ui, world, entity_id, state);
        }

        transform::draw_transform_section(ui, world, entity_id, state);
        physics::draw_velocity_section(ui, world, entity_id, state);
        physics::draw_rigidbody_section(ui, world, entity_id, state);
        physics::draw_collider_section(ui, world, entity_id, state);
        physics::draw_joint_section(ui, world, entity_id, state);

        camera::draw_camera_section(ui, world, entity_id, state);
        light::draw_point_light_section(ui, world, entity_id, state);
        light::draw_directional_light_section(ui, world, entity_id, state);
        material::draw_material_section(ui, world, entity_id, state);

        misc::draw_particle_emitter_section(ui, world, entity_id, state);
        misc::draw_hitbox_section(ui, world, entity_id, state);
        misc::draw_hurtbox_section(ui, world, entity_id, state);
        misc::draw_terrain_section(ui, world, entity_id, state);
        misc::draw_script_section(ui, world, entity_id, state);
        misc::draw_fluid_section(ui, world, entity_id, state);
        misc::draw_ai_section(ui, world, entity_id, state);
        misc::draw_reflection_section(ui, world, entity_id, state);
        misc::draw_animation_player_section(ui, world, entity_id, state);
        misc::draw_bone_attachment_section(ui, world, entity_id, state);
        misc::draw_fighter_controller_section(ui, world, entity_id, state);

        ui.separator();

        if sel_len == 1 {
            ui.horizontal(|ui| {
                if ui.button("➕ Bileşen Ekle").clicked() {
                    state.add_component_open = !state.add_component_open;
                }
            });

            if state.add_component_open {
                menu::draw_add_component_menu(ui, world, entity_id, state);
            }
        }
    });
}
