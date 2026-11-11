
use crate::editor_state::EditorState;
use egui;
use gizmo_core::World;
use gizmo_math::Vec3;
use gizmo_renderer::components::{DirectionalLight, LightRole, PointLight, RectLight, SpotLight};

/// How far toward the light each pixel looks for a contact shadow, in metres.
fn contact_shadow_row(ui: &mut egui::Ui, metres: &mut f32) {
    ui.horizontal(|ui| {
        ui.label("Temas gölgesi:");
        ui.add(egui::DragValue::new(metres).speed(0.01).range(0.0..=5.0).suffix(" m"))
            .on_hover_text("Her pikselin ışığa doğru ekranda baktığı mesafe: gölge haritasının kaçırdığı küçük temasları (bir bozuk para, bir basamağın ilk santimetreleri) yakalar; gölge düşürmeyen bir ışığa da temas gölgesi verir. 0 = kapalı. Yalnız ertelenmiş (deferred) yolda.");
    });
}

/// A source's size, in metres: what softens its shadow and widens its highlight.
fn source_radius_row(ui: &mut egui::Ui, radius: &mut f32) {
    ui.horizontal(|ui| {
        ui.label("Kaynak yarıçapı:");
        ui.add(egui::DragValue::new(radius).speed(0.01).range(0.0..=5.0).suffix(" m"))
            .on_hover_text("Işığın kaynağının büyüklüğü — menzili değil. Büyüdükçe gölgesinin kenarı yumuşar ve parlak yüzeylerdeki yansıması genişler. 0 = nokta kaynak.");
    });
}


/// The point-light rows: colour, intensity, radius, the source's radius and tube length, and whether it
/// casts shadows. (The radius was named here and had no row until 2026-11-11; the source's size, the
/// same day.)
pub fn draw_point_light_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    // SAFETY: editor UI runs single-threaded in the egui draw; no concurrent World access.
    let mut lights = unsafe { world.query_unchecked::<&mut PointLight>() };
    {
        if let Some(mut light) = lights.get_mut(entity_id) {
            egui::CollapsingHeader::new(crate::theme::section_title("PointLight"))
                .default_open(true)
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
                    ui.horizontal(|ui| {
                        ui.label("Menzil:");
                        ui.add(
                            egui::DragValue::new(&mut light.radius)
                                .speed(0.1)
                                .range(0.001..=500.0)
                                .suffix(" m"),
                        );
                    });
                    source_radius_row(ui, &mut light.source_radius);
                    ui.horizontal(|ui| {
                        ui.label("Tüp uzunluğu:");
                        ui.add(egui::DragValue::new(&mut light.source_length).speed(0.01).range(0.0..=50.0).suffix(" m"))
                            .on_hover_text("Floresan tüp gibi uzun bir kaynak: varlığın yerel X ekseni boyunca bu uzunlukta. 0 = küre.");
                    });
                    ui.checkbox(&mut light.casts_shadows, "Cast shadows").on_hover_text("Işık, gölge düşürmesi istenen nesnelerin gölgesini düşürür. Kapalıyken (varsayılan) gölge haritası çizilmez.");
                    contact_shadow_row(ui, &mut light.contact_shadow_length);
                });
            ui.separator();
        }
    }
}


/// The directional-light rows: colour, intensity, its role, the sun's angular radius and its contact
/// shadows; the direction it points is the entity's rotation. (The role, the angular radius and the
/// contact shadows had no rows until 2026-11-11: a soft sun or a contact shadow could be set in code
/// and not in the editor.)
pub fn draw_directional_light_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    // SAFETY: editor UI runs single-threaded in the egui draw; no concurrent World access.
    let mut lights = unsafe { world.query_unchecked::<&mut DirectionalLight>() };
    if let Some(mut light) = lights.get_mut(entity_id) {
        egui::CollapsingHeader::new(crate::theme::section_title("Directional Light (Güneş)"))
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Güneş Rengi:");
                    let mut color = [light.color.x, light.color.y, light.color.z];
                    if ui.color_edit_button_rgb(&mut color).changed() {
                        light.color = Vec3::new(color[0], color[1], color[2]);
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Işık Şiddeti (Intensity):");
                    ui.add(
                        egui::Slider::new(&mut light.intensity, 0.0..=500.0)
                            .text("Lümen")
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("Rol:");
                    egui::ComboBox::from_id_salt(("directional_role", entity_id))
                        .selected_text(if light.role == LightRole::Sun { "Güneş" } else { "Dolgu / kenar" })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut light.role, LightRole::Sun, "Güneş");
                            ui.selectable_value(&mut light.role, LightRole::Generic, "Dolgu / kenar");
                        });
                });
                let mut degrees = light.angular_radius.to_degrees();
                ui.horizontal(|ui| {
                    ui.label("Açısal yarıçap:");
                    if ui
                        .add(egui::Slider::new(&mut degrees, 0.0..=28.6).suffix("°"))
                        .on_hover_text("Güneş diskinin gökyüzündeki büyüklüğü: gölgelerin kenarı, gölgeyi düşürenden uzaklaştıkça bu açıyla yumuşar. Gerçek güneş ~0.27°; 0 = boyutsuz güneş, keskin gölge.")
                        .changed()
                    {
                        light.angular_radius = degrees.to_radians().clamp(0.0, 0.5);
                    }
                });
                contact_shadow_row(ui, &mut light.contact_shadow_length);
                ui.label(egui::RichText::new("Güneşin açısını (gölge yönünü) değiştirmek için üst kısımdaki 'Transform' altından Rotasyon X, Y, Z değerlerini döndürün.").weak().small());
            });
        ui.separator();
    }
}


/// The spot-light rows: colour, intensity, reach, the two cone angles, the source's radius, the cookie
/// and whether it casts shadows.
///
/// The engine has drawn spot lights and the scene file has carried them since the component
/// existed, and this section did not exist — so a spot light arriving from Rust or from a
/// hand-written scene lit the viewport and showed **not one editable row** when selected. It was
/// not reachable through the generic JSON fallback either: that one looks the component up in the
/// `ComponentRegistry`, and `SpotLight` was not in it.
///
/// Angles are edited in DEGREES and stored in radians, because a cone is something an author
/// thinks about in degrees and every other engine's spot light asks for them that way. The inner
/// angle is clamped to the outer one on every edit, which is the same rule `SpotLight::new`
/// applies and for the same reason: an inner cone wider than its outer cone inverts the falloff
/// and lights the outside of the cone.
pub fn draw_spot_light_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    // SAFETY: editor UI runs single-threaded in the egui draw; no concurrent World access.
    let mut lights = unsafe { world.query_unchecked::<&mut SpotLight>() };
    if let Some(mut light) = lights.get_mut(entity_id) {
        egui::CollapsingHeader::new(crate::theme::section_title("Spot Light"))
            .default_open(true)
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
                ui.horizontal(|ui| {
                    ui.label("Menzil:");
                    ui.add(
                        egui::DragValue::new(&mut light.radius)
                            .speed(0.1)
                            .range(0.001..=500.0)
                            .suffix(" m"),
                    );
                });

                let mut inner_deg = light.inner_angle.to_degrees();
                let mut outer_deg = light.outer_angle.to_degrees();
                let mut changed = false;
                ui.horizontal(|ui| {
                    ui.label("İç koni:");
                    changed |= ui
                        .add(egui::Slider::new(&mut inner_deg, 0.0..=89.0).suffix("°"))
                        .changed();
                });
                ui.horizontal(|ui| {
                    ui.label("Dış koni:");
                    changed |= ui
                        .add(egui::Slider::new(&mut outer_deg, 0.0..=89.0).suffix("°"))
                        .changed();
                });
                if changed {
                    // Whichever one moved, the invariant is the same: inner ≤ outer.
                    light.outer_angle = outer_deg.to_radians();
                    light.inner_angle = inner_deg.min(outer_deg).to_radians();
                }
                source_radius_row(ui, &mut light.source_radius);

                // Cookie: a name registered in code, or a path read from disk on first use.
                // Empty is no cookie.
                ui.horizontal(|ui| {
                    ui.label("Cookie:");
                    let mut text = light.cookie.clone().unwrap_or_default();
                    if ui
                        .add(egui::TextEdit::singleline(&mut text).hint_text("yok — görsel yolu"))
                        .changed()
                    {
                        let text = text.trim();
                        light.cookie = (!text.is_empty()).then(|| text.to_string());
                    }
                });
                ui.checkbox(&mut light.casts_shadows, "Cast shadows").on_hover_text("Işık, gölge düşürmesi istenen nesnelerin gölgesini düşürür. Kapalıyken (varsayılan) gölge haritası çizilmez.");
                contact_shadow_row(ui, &mut light.contact_shadow_length);
                ui.label(egui::RichText::new("Koninin baktığı yön varlığın 'Transform' rotasyonundan gelir.").weak().small());
            });
        ui.separator();
    }
}

/// The `RectLight` section: colour, intensity, range, the panel's two extents and whether it casts
/// shadows (from its centre). Which way the
/// panel faces is the entity's rotation (its forward axis, local −Z), as for a spot light.
pub fn draw_rect_light_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    // SAFETY: editor UI runs single-threaded in the egui draw; no concurrent World access.
    let mut lights = unsafe { world.query_unchecked::<&mut RectLight>() };
    if let Some(mut light) = lights.get_mut(entity_id) {
        egui::CollapsingHeader::new(crate::theme::section_title("Rect Light"))
            .default_open(true)
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
                    ui.add(egui::DragValue::new(&mut light.intensity).speed(0.1).range(0.0..=100.0));
                });
                ui.horizontal(|ui| {
                    ui.label("Menzil:");
                    ui.add(
                        egui::DragValue::new(&mut light.radius)
                            .speed(0.1)
                            .range(0.001..=500.0)
                            .suffix(" m"),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("Genişlik:");
                    ui.add(
                        egui::DragValue::new(&mut light.width)
                            .speed(0.01)
                            .range(0.001..=50.0)
                            .suffix(" m"),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("Yükseklik:");
                    ui.add(
                        egui::DragValue::new(&mut light.height)
                            .speed(0.01)
                            .range(0.001..=50.0)
                            .suffix(" m"),
                    );
                });
                ui.checkbox(&mut light.casts_shadows, "Cast shadows").on_hover_text("Işık, gölge düşürmesi istenen nesnelerin gölgesini düşürür. Kapalıyken (varsayılan) gölge haritası çizilmez.");
                contact_shadow_row(ui, &mut light.contact_shadow_length);
                ui.label(egui::RichText::new("Panel, varlığın 'Transform' rotasyonunun ileri ekseni (−Z) yönünde, tek yüzünden ışık verir.").weak().small());
            });
        ui.separator();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inspector::ui_inspector;

    /// Every piece of text one inspector frame painted for `entity`.
    fn painted_text(world: &World, entity: gizmo_core::entity::Entity) -> Vec<String> {
        fn scan(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Vec(v) => v.iter().for_each(|s| scan(s, out)),
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                _ => {}
            }
        }
        let mut state = EditorState::default();
        state.selection.entities.insert(entity);
        state.selection.primary = Some(entity);
        let ctx = egui::Context::default();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| ui_inspector(ui, world, &mut state));
        let mut out = Vec::new();
        output.shapes.iter().for_each(|s| scan(&s.shape, &mut out));
        output.drop_without_applying_deltas();
        out
    }

    /// The fields `value` serialises — what a light is made of, from the type itself, so a field added
    /// later is asked about here without anyone remembering to.
    fn fields(value: &impl serde::Serialize) -> Vec<String> {
        match serde_json::to_value(value).expect("a light serialises") {
            serde_json::Value::Object(map) => map.keys().cloned().collect(),
            other => panic!("a light serialises as an object, not {other}"),
        }
    }

    /// **Every field of every light has a row in the inspector.** The point light's source radius and
    /// tube length, the spot's source radius, and the sun's role, angular radius and contact shadows
    /// were settable in code and nowhere in the editor until 2026-11-11: a soft shadow or a contact
    /// shadow could not be authored there at all. The fields come from each type's own serialisation,
    /// so a field added to a light without a row fails here, by name.
    #[test]
    fn every_field_of_every_light_has_a_row() {
        let label = |field: &str| -> Option<&'static str> {
            Some(match field {
                "color" => "Renk:",
                "intensity" => "Yoğunluk:",
                "radius" => "Menzil:",
                "source_radius" => "Kaynak yarıçapı:",
                "source_length" => "Tüp uzunluğu:",
                "casts_shadows" => "Cast shadows",
                "inner_angle" => "İç koni:",
                "outer_angle" => "Dış koni:",
                "cookie" => "Cookie:",
                "width" => "Genişlik:",
                "height" => "Yükseklik:",
                "role" => "Rol:",
                "angular_radius" => "Açısal yarıçap:",
                "contact_shadow_length" => "Temas gölgesi:",
                _ => return None,
            })
        };
        // The sun's colour and intensity rows say so in its own words.
        let sun_label = |field: &str| match field {
            "color" => Some("Güneş Rengi:"),
            "intensity" => Some("Işık Şiddeti (Intensity):"),
            other => label(other),
        };
        let mut world = World::new();
        let point = world.spawn();
        world.add_component(point, PointLight::new(Vec3::ONE, 1.0, 5.0));
        let spot = world.spawn();
        world.add_component(spot, SpotLight::new(Vec3::ONE, 1.0, 5.0, 0.3, 0.5));
        let rect = world.spawn();
        world.add_component(rect, RectLight::new(Vec3::ONE, 1.0, 5.0, 1.0, 1.0));
        let sun = world.spawn();
        world.add_component(sun, DirectionalLight::new(Vec3::ONE, 1.0, LightRole::Sun));

        let mut missing = Vec::new();
        let cases: [(&str, gizmo_core::entity::Entity, Vec<String>, &dyn Fn(&str) -> Option<&'static str>); 4] = [
            ("PointLight", point, fields(&PointLight::new(Vec3::ONE, 1.0, 5.0)), &label),
            ("SpotLight", spot, fields(&SpotLight::new(Vec3::ONE, 1.0, 5.0, 0.3, 0.5)), &label),
            ("RectLight", rect, fields(&RectLight::new(Vec3::ONE, 1.0, 5.0, 1.0, 1.0)), &label),
            ("DirectionalLight", sun, fields(&DirectionalLight::new(Vec3::ONE, 1.0, LightRole::Sun)), &sun_label),
        ];
        for (name, entity, fields, label) in cases {
            let text = painted_text(&world, entity);
            for field in fields {
                match label(&field) {
                    None => missing.push(format!("{name}::{field}: no label known — give it a row and a label here")),
                    Some(want) if !text.iter().any(|t| t == want) => missing.push(format!("{name}::{field}: no \"{want}\" row")),
                    Some(_) => {}
                }
            }
        }
        assert!(missing.is_empty(), "light fields the inspector does not show: {missing:#?}");
    }
}
