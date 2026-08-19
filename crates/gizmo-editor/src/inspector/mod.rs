/// The transform rows, and the Euler↔quaternion conversion behind them.
pub mod transform;
/// The rigid-body and collider rows.
pub mod physics;
/// The light rows, one shape of section per light type.
pub mod light;
/// The camera rows.
pub mod camera;
/// The material rows: albedo, metallic, roughness and the texture slots.
pub mod material;
/// Everything else the inspector can edit, one `draw_*_section` per component family.
pub mod misc;
/// The SCRIPT section's property rows. Native-only: `gizmo-scripting` is a
/// `cfg(not(target_arch = "wasm32"))` dependency of this crate.
#[cfg(not(target_arch = "wasm32"))]
pub mod script;
/// The environment rows — sky, ambient and fog.
pub mod environment;
/// The `➕ Add component` menu, and what each entry adds.
pub mod menu;

use crate::editor_state::EditorState;
use gizmo_core::World;

/// Draws the whole inspector panel for the current selection: one section per component the
/// primary entity carries, plus the add-component menu.
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

    // The inspector body sits on SURFACE, not on the panel fill.
    //
    // The prototype inverts the panel/field relationship the rest of the editor uses: its inspector
    // is the LIGHTER `#2d2b2b` with field wells cut into it in the darker `#201e1d`. This theme
    // paints panels CHROME and widgets SURFACE — the other way round — so a field well painted to
    // spec would be invisible here. Reframing just this panel keeps the finished toolbar,
    // hierarchy and viewport work untouched while the fields inside can be drawn as designed.
    egui::Frame::new().fill(crate::theme::palette::SURFACE).show(ui, |ui| {
    egui::ScrollArea::vertical().show(ui, |ui| {
        if sel_len == 1 {
            misc::draw_name_section(ui, world, entity_id, state);
        }

        transform::draw_transform_section(ui, world, entity_id, state);
        misc::draw_mesh_renderer_section(ui, world, entity_id, state);
        physics::draw_velocity_section(ui, world, entity_id, state);
        physics::draw_rigidbody_section(ui, world, entity_id, state);
        physics::draw_collider_section(ui, world, entity_id, state);
        physics::draw_joint_section(ui, world, entity_id, state);

        camera::draw_camera_section(ui, world, entity_id, state);
        light::draw_point_light_section(ui, world, entity_id, state);
        light::draw_directional_light_section(ui, world, entity_id, state);
        light::draw_spot_light_section(ui, world, entity_id, state);
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
            // The prototype's `+ ADD COMPONENT`: full width, accent outline, flush at the bottom of
            // the component list — it is the one action that belongs to the inspector as a whole
            // rather than to any section, and it reads that way only if it spans them all.
            ui.add_space(crate::theme::SPACE_2);
            let label = if state.add_component_open { "− BİLEŞEN EKLE" } else { "+ BİLEŞEN EKLE" };
            let button = egui::Button::new(
                egui::RichText::new(label).size(10.0).color(crate::theme::palette::ACCENT),
            )
            .fill(crate::theme::palette::CHROME)
            .stroke(egui::Stroke::new(1.0_f32, crate::theme::palette::ACCENT))
            .corner_radius(0)
            .min_size(egui::vec2(ui.available_width(), 22.0));
            if ui.add(button).clicked() {
                state.add_component_open = !state.add_component_open;
            }

            if state.add_component_open {
                menu::draw_add_component_menu(ui, world, entity_id, state);
            }
        }
    });
    });
}

#[cfg(test)]
mod inspector_width_tests {
    use super::*;

    /// The Inspector's share of the default layout: `split_right(root, 0.75, ..)` of a 1600 px
    /// window, which `create_default_dock_state` fixes and `main.rs` sizes.
    const DEFAULT_INSPECTOR_WIDTH: f32 = 400.0;

    /// The furthest right anything the frame painted reaches.
    fn painted_right_edge(output: &egui::FullOutput) -> f32 {
        fn scan(shape: &egui::Shape, max: &mut f32) {
            match shape {
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| scan(s, max)),
                other => {
                    let r = other.visual_bounding_rect();
                    if r.is_finite() {
                        *max = max.max(r.max.x);
                    }
                }
            }
        }
        let mut max = 0.0_f32;
        output.shapes.iter().for_each(|s| scan(&s.shape, &mut max));
        max
    }

    /// Draw the inspector into a panel of exactly `width`, and report how far its ink actually got.
    ///
    /// The clip rect is deliberately left wide open. Clipping is what *hides* this defect on a real
    /// screen — the overflowing half is simply not drawn, so the panel looks like a panel with the
    /// ends of its words bitten off. Measuring unclipped is what turns that into a number.
    fn ink_width(width: f32, state: &mut EditorState, world: &World) -> f32 {
        let ctx = egui::Context::default();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let mut panel = ui.new_child(egui::UiBuilder::new().max_rect(
                egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(width, 2000.0)),
            ));
            panel.set_clip_rect(egui::Rect::EVERYTHING);
            ui_inspector(&mut panel, world, state);
        });
        let edge = painted_right_edge(&output);
        output.drop_without_applying_deltas();
        edge
    }

    /// The Inspector must fit the width the default layout gives it.
    ///
    /// It did not. With nothing selected the Inspector shows the environment settings, and every
    /// one of its nine rows was `ui.horizontal(label, Slider)` — a layout that does not shrink, so
    /// the content demanded a flat 422.7 px no matter how narrow the panel was. At the default
    /// 1600 px window the Inspector gets 400 px, so it overflowed by 23 px out of the box; at a
    /// 1280 px window, by about 100. The overflow also widened the enclosing `ScrollArea`, which
    /// is why the closing hint wrapped at a width wider than the panel and lost the end of both
    /// of its lines.
    ///
    /// Measured at three widths, because the failure is that the content has a *fixed* minimum:
    /// a fix that merely moved the constant would still fail the narrow case.
    /// The other half of the panel: the component sections shown for a selected entity.
    ///
    /// `ui_inspector` branches on selection, so the environment settings and the component list
    /// are two disjoint bodies of UI and a measurement of one says nothing about the other. This
    /// selects an entity carrying the components a user actually clicks on — transform, body,
    /// collider, velocity — and holds the same rows to the same panel.
    #[test]
    fn the_component_sections_fit_the_width_too() {
        use gizmo_physics_core::{Collider, Transform};
        use gizmo_physics_rigid::components::{RigidBody, Velocity};

        let mut world = World::new();
        let entity = world.spawn_bundle((
            Transform::new(gizmo_math::Vec3::new(1.5, 2.0, -0.25)),
            RigidBody::new(1.0, true),
            Collider::box_collider(gizmo_math::Vec3::splat(0.5)),
            Velocity::default(),
        ));

        for width in [DEFAULT_INSPECTOR_WIDTH, 320.0, 260.0] {
            let mut state = EditorState::default();
            state.selection.entities.insert(entity);
            state.selection.primary = Some(entity);
            let ink = ink_width(width, &mut state, &world);
            assert!(
                ink <= width + 1.0,
                "the component sections painted out to {ink:.1} px inside a {width:.0} px panel                  ({:.1} px past the edge)",
                ink - width
            );
        }
    }

    #[test]
    fn the_inspector_fits_the_width_the_layout_gives_it() {
        let world = World::new();
        for width in [DEFAULT_INSPECTOR_WIDTH, 320.0, 260.0] {
            let mut state = EditorState::default();
            let ink = ink_width(width, &mut state, &world);
            assert!(
                ink <= width + 1.0,
                "with nothing selected the Inspector painted out to {ink:.1} px inside a \
                 {width:.0} px panel ({:.1} px past the edge). On screen that is not an overflow \
                 you can scroll to — it is clipped, so the words simply end.",
                ink - width
            );
        }
    }
}
