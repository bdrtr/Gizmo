use super::key::{fit_slider, RowKey};
use super::material::asset_slot;
use crate::editor_state::EditorState;
use gizmo_core::World;
use gizmo_math::{Vec2, Vec4};
use gizmo_renderer::components::{Sprite, SpriteSpace, Text, TextAlign, TextAnchor, TextSpace};
use gizmo_renderer::sprite::{ImageName, SpriteFilter};

/// The images a sprite slot takes from the asset browser.
const IMAGES: &[&str] = &[".png", ".jpg", ".jpeg", ".webp"];
/// The faces a text's font slot takes.
const FONTS: &[&str] = &[".ttf", ".otf"];

/// Two numbers side by side, `labels` before each.
fn pair(ui: &mut egui::Ui, value: &mut Vec2, labels: [&str; 2], speed: f32) {
    ui.add(egui::DragValue::new(&mut value.x).speed(speed).prefix(labels[0]));
    ui.add(egui::DragValue::new(&mut value.y).speed(speed).prefix(labels[1]));
}

/// A colour with its alpha, straight (not premultiplied): what `Sprite::color` and `Text::color` are.
fn rgba(ui: &mut egui::Ui, value: &mut Vec4) {
    let mut c = value.to_array();
    if ui.color_edit_button_rgba_unmultiplied(&mut c).changed() {
        *value = Vec4::from_array(c);
    }
}

/// The sprite rows: its image (dropped from the asset browser; ✕ goes back to the engine's white)
/// and how it is sampled, its normal map, size, colour, anchor, the part of the image drawn, flips,
/// layer, whether the scene's lights reach it, and where it lives — the world or the window. Each
/// animatable field has its ◆. (A sprite had no section in the studio until 2026-11-16: it could be
/// loaded from a scene and seen, and not changed.)
pub fn draw_sprite_section(ui: &mut egui::Ui, world: &World, entity_id: gizmo_core::entity::Entity, state: &mut EditorState) {
    // SAFETY: editor UI runs single-threaded in the egui draw; no concurrent World access.
    let mut sprites = unsafe { world.query_unchecked::<&mut Sprite>() };
    let Some(mut sprite) = sprites.get_mut(entity_id) else {
        return;
    };
    let state = std::cell::RefCell::new(state);
    let key = RowKey { world, state: &state, entity: entity_id, component: "Sprite" };
    egui::CollapsingHeader::new(crate::theme::section_title("Sprite")).default_open(true).show(ui, |ui| {
        // The image is its name: `sprite_sync` loads what a changed name names on the next frame.
        let mut name = sprite
            .image_name
            .clone()
            .unwrap_or_else(|| ImageName::new(gizmo_renderer::asset::AssetManager::WHITE_TEXTURE, SpriteFilter::Linear));
        let shown = (name.path != gizmo_renderer::asset::AssetManager::WHITE_TEXTURE).then_some(name.path.as_str());
        if let Some(path) = asset_slot(ui, &mut state.borrow_mut(), "Image", shown, IMAGES) {
            name.path = path.unwrap_or_else(|| gizmo_renderer::asset::AssetManager::WHITE_TEXTURE.to_string());
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("Filter:");
            egui::ComboBox::from_id_salt(("sprite_filter", entity_id))
                .selected_text(if name.filter == SpriteFilter::Nearest { "Nearest (pixel art)" } else { "Linear" })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut name.filter, SpriteFilter::Linear, "Linear");
                    ui.selectable_value(&mut name.filter, SpriteFilter::Nearest, "Nearest (pixel art)");
                });
        });
        if let Some(path) = asset_slot(ui, &mut state.borrow_mut(), "Normal map", name.normal_map.as_deref(), IMAGES) {
            name.normal_map = path;
        }
        if sprite.image_name.as_ref() != Some(&name) {
            sprite.image_name = Some(name);
        }

        ui.horizontal_wrapped(|ui| {
            key.diamond(ui, "size");
            ui.label("Size:");
            let mut size = sprite.size;
            pair(ui, &mut size, ["w ", "h "], 0.01);
            if size != sprite.size {
                sprite.size = size.max(Vec2::splat(0.0));
            }
        });
        ui.horizontal_wrapped(|ui| {
            key.diamond(ui, "color");
            ui.label("Color:");
            rgba(ui, &mut sprite.color);
        });
        ui.horizontal_wrapped(|ui| {
            key.diamond(ui, "anchor");
            ui.label("Anchor:");
            pair(ui, &mut sprite.anchor, ["x ", "y "], 0.01);
        });
        ui.horizontal_wrapped(|ui| {
            key.diamond(ui, "uv_rect");
            ui.label("UV rect:");
            for (prefix, v) in ["u0 ", "v0 ", "u1 ", "v1 "].into_iter().zip(sprite.uv_rect.iter_mut()) {
                ui.add(egui::DragValue::new(v).speed(0.005).range(0.0..=1.0).prefix(prefix));
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut sprite.flip_x, "Flip X");
            ui.checkbox(&mut sprite.flip_y, "Flip Y");
            ui.checkbox(&mut sprite.lit, "Lit").on_hover_text("Sahnenin ışıkları bu sprite'ı aydınlatır (yalnız dünyadaki sprite).");
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Layer:");
            ui.add(egui::DragValue::new(&mut sprite.layer));
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Space:");
            let screen = !matches!(sprite.space, SpriteSpace::World);
            egui::ComboBox::from_id_salt(("sprite_space", entity_id))
                .selected_text(if screen { "Screen (pixels)" } else { "World" })
                .show_ui(ui, |ui| {
                    if ui.selectable_label(!screen, "World").clicked() && screen {
                        sprite.space = SpriteSpace::World;
                    }
                    if ui.selectable_label(screen, "Screen (pixels)").clicked() && !screen {
                        sprite.space = SpriteSpace::Screen { position: Vec2::splat(100.0) };
                    }
                });
        });
        if let SpriteSpace::Screen { position } = &mut sprite.space {
            ui.horizontal_wrapped(|ui| {
                ui.label("Position:");
                pair(ui, position, ["x ", "y "], 1.0);
            });
        }
    });
    ui.separator();
}

/// The text rows: what it says, its face (dropped from the asset browser; ✕ goes back to the
/// engine's default), size, colour, anchor, alignment, wrap width, the vertical layout, and where it
/// lives — the window, a point of the window, or the world. Each animatable field has its ◆. Rich
/// text (the `spans`) is counted, not edited here. (A text had no section in the studio until
/// 2026-11-16.)
pub fn draw_text_section(ui: &mut egui::Ui, world: &World, entity_id: gizmo_core::entity::Entity, state: &mut EditorState) {
    // SAFETY: editor UI runs single-threaded in the egui draw; no concurrent World access.
    let mut texts = unsafe { world.query_unchecked::<&mut Text>() };
    let Some(mut text) = texts.get_mut(entity_id) else {
        return;
    };
    let state = std::cell::RefCell::new(state);
    let key = RowKey { world, state: &state, entity: entity_id, component: "Text" };
    egui::CollapsingHeader::new(crate::theme::section_title("Text")).default_open(true).show(ui, |ui| {
        let mut content = text.content.clone();
        if ui.add(egui::TextEdit::multiline(&mut content).desired_rows(2).desired_width(f32::INFINITY)).changed() {
            text.content = content;
        }
        if !text.spans.is_empty() {
            ui.label(egui::RichText::new(format!("+ {} span (zengin metin, kodda düzenlenir)", text.spans.len())).weak().small());
        }
        // The face is its name: `text_sync` finds the face a changed name names on the next frame.
        let shown = text.font_name.as_deref().filter(|n| *n != gizmo_renderer::Renderer::DEFAULT_FONT);
        if let Some(path) = asset_slot(ui, &mut state.borrow_mut(), "Font", shown, FONTS) {
            text.font_name = Some(path.unwrap_or_else(|| gizmo_renderer::Renderer::DEFAULT_FONT.to_string()));
        }
        ui.horizontal_wrapped(|ui| {
            key.diamond(ui, "size_px");
            ui.label("Size (px):");
            fit_slider(ui);
            ui.add(egui::Slider::new(&mut text.size_px, 4.0..=256.0));
        });
        ui.horizontal_wrapped(|ui| {
            key.diamond(ui, "color");
            ui.label("Color:");
            rgba(ui, &mut text.color);
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Anchor:");
            let anchors = [
                (TextAnchor::TopLeft, "Top left"),
                (TextAnchor::TopCenter, "Top centre"),
                (TextAnchor::TopRight, "Top right"),
                (TextAnchor::CenterLeft, "Centre left"),
                (TextAnchor::Center, "Centre"),
                (TextAnchor::CenterRight, "Centre right"),
                (TextAnchor::BottomLeft, "Bottom left"),
                (TextAnchor::BottomCenter, "Bottom centre"),
                (TextAnchor::BottomRight, "Bottom right"),
            ];
            let current = anchors.iter().find(|(a, _)| *a == text.anchor).map_or("—", |(_, n)| n);
            egui::ComboBox::from_id_salt(("text_anchor", entity_id)).selected_text(current).show_ui(ui, |ui| {
                for (anchor, label) in anchors {
                    ui.selectable_value(&mut text.anchor, anchor, label);
                }
            });
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Align:");
            let aligns = [(TextAlign::Start, "Start"), (TextAlign::Left, "Left"), (TextAlign::Center, "Centre"), (TextAlign::Right, "Right")];
            let current = aligns.iter().find(|(a, _)| *a == text.align).map_or("—", |(_, n)| n);
            egui::ComboBox::from_id_salt(("text_align", entity_id)).selected_text(current).show_ui(ui, |ui| {
                for (align, label) in aligns {
                    ui.selectable_value(&mut text.align, align, label);
                }
            });
        });
        ui.horizontal_wrapped(|ui| {
            let mut wraps = text.max_width.is_some();
            if ui.checkbox(&mut wraps, "Wrap at").changed() {
                text.max_width = wraps.then_some(300.0);
            }
            if let Some(width) = text.max_width.as_mut() {
                ui.add(egui::DragValue::new(width).speed(1.0).range(1.0..=f32::MAX).suffix(" px"));
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut text.vertical, "Vertical");
            if text.vertical {
                ui.checkbox(&mut text.upright, "Upright Latin");
            }
        });

        // Where it lives: three kinds of place, each with its own numbers.
        let kinds = ["Screen (pixels)", "Pinned to the window", "World"];
        let kind = match text.space {
            TextSpace::Screen { .. } => 0,
            TextSpace::Pinned { .. } => 1,
            _ => 2,
        };
        ui.horizontal_wrapped(|ui| {
            ui.label("Space:");
            egui::ComboBox::from_id_salt(("text_space", entity_id)).selected_text(kinds[kind]).show_ui(ui, |ui| {
                for (i, label) in kinds.into_iter().enumerate() {
                    if ui.selectable_label(i == kind, label).clicked() && i != kind {
                        text.space = match i {
                            0 => TextSpace::Screen { position: Vec2::splat(100.0) },
                            1 => TextSpace::Pinned { at: Vec2::splat(0.5), offset: Vec2::ZERO },
                            _ => TextSpace::World { px_per_unit: 64.0 },
                        };
                    }
                }
            });
        });
        match &mut text.space {
            TextSpace::Screen { position } => {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Position:");
                    pair(ui, position, ["x ", "y "], 1.0);
                });
            }
            TextSpace::Pinned { at, offset } => {
                ui.horizontal_wrapped(|ui| {
                    ui.label("At (0–1):");
                    pair(ui, at, ["x ", "y "], 0.01);
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label("Offset:");
                    pair(ui, offset, ["x ", "y "], 1.0);
                });
            }
            TextSpace::World { px_per_unit } => {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Pixels per unit:");
                    ui.add(egui::DragValue::new(px_per_unit).speed(0.5).range(1.0..=4096.0));
                });
            }
            _ => {}
        }
    });
    ui.separator();
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_renderer::asset::AssetManager;

    /// One frame of the inspector over `entity`, the pointer at `pointer` (released there with
    /// `release`), and every text it painted, with where.
    fn frame(ctx: &egui::Context, world: &World, state: &mut EditorState, pointer: Option<egui::Pos2>, release: bool) -> Vec<(String, egui::Rect)> {
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(420.0, 4000.0))),
            ..Default::default()
        };
        if let Some(p) = pointer {
            input.events.push(egui::Event::PointerMoved(p));
            if release {
                input.events.push(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
            }
        }
        let output = ctx.run_ui(input, |ui| crate::inspector::ui_inspector(ui, world, state));
        fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
            match shape {
                egui::Shape::Text(t) => out.push((t.galley.text().to_string(), shape.visual_bounding_rect())),
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let mut out = Vec::new();
        output.shapes.iter().for_each(|c| walk(&c.shape, &mut out));
        output.drop_without_applying_deltas();
        out
    }

    /// Drags `asset` from the asset browser and drops it on the slot whose text starts with `slot`.
    fn drop_on(ctx: &egui::Context, world: &World, state: &mut EditorState, slot: &str, asset: &str) {
        let texts = frame(ctx, world, state, None, false);
        let at = texts.iter().find(|(t, _)| t.starts_with(slot)).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no {slot} slot: {texts:?}"));
        state.dragged_asset = Some(asset.into());
        frame(ctx, world, state, Some(at), false);
        frame(ctx, world, state, Some(at), true);
    }

    fn selected(e: gizmo_core::entity::Entity) -> EditorState {
        let mut state = EditorState::default();
        state.selection.entities.insert(e);
        state.selection.primary = Some(e);
        state
    }

    /// **A sprite takes its image from the asset browser.** A new sprite draws the engine's white,
    /// and its Image slot says so by being empty; a PNG dropped on it is the sprite's image by name
    /// (which `sprite_sync` loads), with the filter it had; a font file dropped there is refused.
    #[test]
    fn an_image_dropped_on_a_sprite_is_its_image() {
        let mut world = World::new();
        let e = world.spawn();
        let mut sprite = Sprite::world(gizmo_renderer::sprite::SpriteImageId::UNRESOLVED, Vec2::ONE);
        sprite.image_name = Some(ImageName::new(AssetManager::WHITE_TEXTURE, SpriteFilter::Nearest));
        world.add_component(e, sprite);
        let mut state = selected(e);
        let ctx = egui::Context::default();
        frame(&ctx, &world, &mut state, None, false);

        drop_on(&ctx, &world, &mut state, "Image:", "demo/assets/fonts/face.ttf");
        assert_eq!(world.get::<Sprite>(e).unwrap().image_name.as_ref().unwrap().path, AssetManager::WHITE_TEXTURE, "a font is not an image");

        drop_on(&ctx, &world, &mut state, "Image:", "demo/assets/sprites/hero.png");
        let name = world.get::<Sprite>(e).unwrap().image_name.clone().unwrap();
        assert_eq!((name.path.as_str(), name.filter), ("demo/assets/sprites/hero.png", SpriteFilter::Nearest));
    }

    /// **A text takes its face from the asset browser**: a `.ttf` dropped on the Font slot is the
    /// text's face by name (which `text_sync` finds).
    #[test]
    fn a_font_dropped_on_a_text_is_its_face() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Text::world("hi", gizmo_renderer::text::FontId::UNRESOLVED, 32.0, 64.0));
        let mut state = selected(e);
        let ctx = egui::Context::default();
        frame(&ctx, &world, &mut state, None, false);
        drop_on(&ctx, &world, &mut state, "Font:", "demo/assets/fonts/face.ttf");
        assert_eq!(world.get::<Text>(e).unwrap().font_name.as_deref(), Some("demo/assets/fonts/face.ttf"));
    }
}
