//! The ANIMATION tab for an entity's **keyed transforms** — Godot's animation editor for a node's
//! position, rotation and scale: a door that swings, a platform that rides, a camera that sweeps.
//!
//! The selected entity's [`TransformAnimationPlayer`] is shown as a track per channel, keyframes on
//! a ruler, a playhead. **◆ Key** records the entity's transform as it stands now — position,
//! rotation and scale — at the playhead, into tracks of its own (an empty target name: the
//! player's own entity), making them the first time; so an animation is made by scrubbing, moving
//! the object with the gizmo, and keying, as in Godot. Keys drag in time, delete with a right
//! click or Delete; ▶ plays it in the editor (the engine's frame drives the player in both draw
//! paths, and a paused one follows its playhead, which the ruler scrubs). Every edit is one undo
//! entry, the clip whole (`EditorAction::TransformClipChanged`), for the reason the skeletal
//! timeline's are: a retime reorders the keys.
//!
//! Tracks that target a named entity below the player draw too, under its name.

use crate::animation_panel::{diamond, GUTTER, HIT, MARK, ROW};
use crate::editor_state::{EditorState, KeyframeRef, TrackChannel};
use crate::theme::palette::*;
use gizmo_core::{Entity, World};
use gizmo_physics_core::Transform;
use gizmo_renderer::transform_animation::{AnimationClip, InterpolatedValue, Keyframes, TransformAnimationPlayer, Track};
use std::sync::Arc;

/// Draws the transform timeline for `entity`'s player; `false` when it has none.
pub(crate) fn ui_transform_animation(ui: &mut egui::Ui, world: &World, state: &mut EditorState, entity: Entity) -> bool {
    // SAFETY: the editor UI runs single-threaded inside the egui draw; no concurrent World access.
    let mut players = unsafe { world.query_unchecked::<&mut TransformAnimationPlayer>() };
    let Some(mut player) = players.get_mut(entity) else {
        return false;
    };
    let clip = player.clip.clone();
    // A clip with no keys yet still gets a second of lane to scrub and key along.
    let duration = clip.as_ref().map_or(0.0, |c| c.duration()).max(1.0);

    // ── Header ───────────────────────────────────────────────────────────────────────────────
    let mut key_now = false;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("A N I M A T I O N").size(10.0).color(TEXT_MUTED));
        let name = clip.as_ref().map_or("—", |c| c.name.as_str());
        ui.label(egui::RichText::new(name).size(11.0).color(TEXT_BRIGHT));
        ui.label(egui::RichText::new(format!("{:.2} / {:.2} s", player.elapsed_time, duration)).size(10.0).color(TEXT_DIM));
        if ui.button(if player.playing { "⏸" } else { "▶" }).clicked() {
            if player.playing {
                player.pause();
            } else {
                player.resume();
            }
        }
        if ui.button("◆ Key").on_hover_text("Record this entity's transform at the playhead").clicked() {
            key_now = true;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            crate::theme::toggle(ui, &mut player.looping, "Loop");
        });
    });
    ui.separator();

    if key_now {
        if let Some(transform) = world.query::<&Transform>().get(entity).copied() {
            let before = player.clip.clone();
            let keyed = key_transform(before.as_deref(), player.elapsed_time, &transform);
            player.clip = Some(Arc::new(keyed));
            player.target_entities.clear();
            state.history.push(crate::history::EditorAction::TransformClipChanged { entity, before, after: player.clip.clone() });
            state.has_unsaved_changes = true;
            ui.ctx().request_repaint();
        }
        return true;
    }

    let Some(clip) = clip else {
        crate::animation_panel::empty(ui, "Henüz anahtar yok — nesneyi yerleştir, oynatma kafasını kaydır, ◆ Key.");
        return true;
    };

    // ── Timeline ─────────────────────────────────────────────────────────────────────────────
    let rows = clip.tracks.len().max(1);
    let wanted = ROW * (rows as f32 + 1.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), wanted.min(ui.available_height().max(ROW * 2.0))), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let lane = egui::Rect::from_min_max(egui::pos2(rect.left() + GUTTER + MARK, rect.top()), egui::pos2(rect.right() - MARK, rect.bottom()));
    if lane.width() < 40.0 {
        return true;
    }
    let x_of = |t: f32| lane.left() + (t / duration).clamp(0.0, 1.0) * lane.width();
    let t_of = |x: f32| ((x - lane.left()) / lane.width()).clamp(0.0, 1.0) * duration;

    painter.line_segment([egui::pos2(rect.left(), rect.top() + ROW), egui::pos2(rect.right(), rect.top() + ROW)], egui::Stroke::new(1.0_f32, BORDER));
    painter.text(egui::pos2(rect.left() + 6.0, rect.top() + 3.0), egui::Align2::LEFT_TOP, "Tracks", egui::FontId::proportional(10.0), TEXT_MUTED);
    for s in 0..=(duration.ceil() as i32) {
        let x = x_of(s as f32);
        painter.line_segment([egui::pos2(x, rect.top() + ROW - 4.0), egui::pos2(x, rect.top() + ROW)], egui::Stroke::new(1.0_f32, BORDER_HOT));
        painter.text(egui::pos2(x + 2.0, rect.top() + 2.0), egui::Align2::LEFT_TOP, s.to_string(), egui::FontId::proportional(9.0), TEXT_DIM);
    }

    let mut redraw = false;
    for (i, track) in clip.tracks.iter().enumerate() {
        let y = rect.top() + ROW * (i as f32 + 1.0);
        if y + ROW > rect.bottom() {
            break;
        }
        let row_rect = egui::Rect::from_min_size(egui::pos2(rect.left(), y), egui::vec2(rect.width(), ROW));
        if i % 2 == 1 {
            painter.rect_filled(row_rect, 0.0, CHROME);
        }
        let target = if track.target_name.is_empty() { "this" } else { track.target_name.as_str() };
        painter.text(
            egui::pos2(rect.left() + 6.0, row_rect.center().y),
            egui::Align2::LEFT_CENTER,
            format!("{target}  {}", channel_name(&track.keyframes)),
            egui::FontId::proportional(10.0),
            TEXT_BODY,
        );
        for (k, t) in track.keyframe_timestamps.iter().enumerate() {
            let here = KeyframeRef { channel: TrackChannel::Translation, track: i, keyframe: k };
            let centre = egui::pos2(x_of(*t), row_rect.center().y);
            let hit = egui::Rect::from_center_size(centre, egui::vec2(HIT, ROW));
            let resp = ui.interact(hit, ui.id().with(("transform_key", i, k)), egui::Sense::click_and_drag());
            if resp.drag_started() {
                state.anim_edit.dragging = Some(here);
                state.anim_edit.selected = Some(here);
                state.transform_drag_original = Some(player.clip.clone());
            }
            if resp.clicked() {
                state.anim_edit.selected = Some(here);
            }
            let dragged = state.anim_edit.dragging == Some(here);
            let selected = state.anim_edit.selected == Some(here);
            let colour = if dragged || resp.hovered() { TEXT_BRIGHT } else if selected { ACCENT_LIGHT } else { ACCENT };
            diamond(&painter, centre, if selected { MARK + 1.0 } else { MARK }, colour);
            if dragged {
                if let Some(pos) = resp.interact_pointer_pos() {
                    // The index follows the key: a retime can carry it past its neighbours.
                    if let Some(landed) = edit_clip(&mut player, |c| c.tracks.get_mut(i)?.retime_key(k, t_of(pos.x))).flatten() {
                        state.anim_edit.dragging = Some(KeyframeRef { keyframe: landed, ..here });
                        state.anim_edit.selected = state.anim_edit.dragging;
                        redraw = true;
                    }
                }
            }
            if resp.drag_stopped() {
                if let Some(before) = state.transform_drag_original.take() {
                    state.history.push(crate::history::EditorAction::TransformClipChanged { entity, before, after: player.clip.clone() });
                    state.has_unsaved_changes = true;
                }
                state.anim_edit.dragging = None;
            }
            resp.context_menu(|ui| {
                if ui.button("🗑 Keyframe'i sil").clicked() {
                    if remove_key(&mut player, state, entity, i, k) {
                        redraw = true;
                    }
                    ui.close();
                }
            });
        }
    }

    let head = x_of(player.elapsed_time);
    painter.line_segment([egui::pos2(head, rect.top()), egui::pos2(head, rect.bottom())], egui::Stroke::new(1.0_f32, TEXT_BRIGHT));
    let scrub = ui.interact(lane, ui.id().with("transform_scrub"), egui::Sense::click_and_drag());
    if state.anim_edit.dragging.is_none() && (scrub.dragged() || scrub.clicked()) {
        if let Some(pos) = scrub.interact_pointer_pos() {
            player.elapsed_time = t_of(pos.x);
            redraw = true;
        }
    }
    // Delete removes the selected key, only with the pointer over the timeline (Delete is also the
    // viewport's despawn).
    let over = rect.contains(ui.input(|i| i.pointer.hover_pos()).unwrap_or(rect.max));
    if over && ui.input(|i| i.key_pressed(egui::Key::Delete)) {
        if let Some(sel) = state.anim_edit.selected {
            if remove_key(&mut player, state, entity, sel.track, sel.keyframe) {
                redraw = true;
            }
        }
    }
    if redraw {
        ui.ctx().request_repaint();
    }
    true
}

/// Which channel a track keys.
fn channel_name(keys: &Keyframes) -> &'static str {
    match keys {
        Keyframes::Translation(_) => "position",
        Keyframes::Rotation(_) => "rotation",
        Keyframes::Scale(_) => "scale",
        _ => "?",
    }
}

/// Whether `value` belongs to the channel `keys` holds.
fn same_channel(keys: &Keyframes, value: &InterpolatedValue) -> bool {
    matches!(
        (keys, value),
        (Keyframes::Translation(_), InterpolatedValue::Translation(_))
            | (Keyframes::Rotation(_), InterpolatedValue::Rotation(_))
            | (Keyframes::Scale(_), InterpolatedValue::Scale(_))
    )
}

/// `clip` (a new one when there is none) with `transform` keyed at `time` on the player's own
/// entity: its position, rotation and scale tracks, each made the first time.
pub(crate) fn key_transform(clip: Option<&AnimationClip>, time: f32, transform: &Transform) -> AnimationClip {
    let mut clip = clip.cloned().unwrap_or_else(|| {
        let mut c = AnimationClip::default();
        c.name = "animation".into();
        c
    });
    let values = [
        InterpolatedValue::Translation(transform.position),
        InterpolatedValue::Rotation(transform.rotation),
        InterpolatedValue::Scale(transform.scale),
    ];
    for value in values {
        match clip.tracks.iter_mut().find(|t| t.target_name.is_empty() && same_channel(&t.keyframes, &value)) {
            Some(track) => {
                track.set_key(time, value);
            }
            None => {
                let keys = match value {
                    InterpolatedValue::Translation(v) => Keyframes::Translation(vec![v]),
                    InterpolatedValue::Rotation(q) => Keyframes::Rotation(vec![q]),
                    InterpolatedValue::Scale(s) => Keyframes::Scale(vec![s]),
                    _ => continue,
                };
                if let Ok(track) = Track::new("", vec![time], keys) {
                    clip.tracks.push(track);
                }
            }
        }
    }
    clip
}

/// The player's clip with `edit` applied, put back whole (the clip is shared behind an `Arc`).
fn edit_clip<R>(player: &mut TransformAnimationPlayer, edit: impl FnOnce(&mut AnimationClip) -> R) -> Option<R> {
    let mut clip = (**player.clip.as_ref()?).clone();
    let out = edit(&mut clip);
    player.clip = Some(Arc::new(clip));
    player.target_entities.clear();
    Some(out)
}

/// Removes key `k` of track `track`, recording the undo; whether there was one.
fn remove_key(player: &mut TransformAnimationPlayer, state: &mut EditorState, entity: Entity, track: usize, k: usize) -> bool {
    let before = player.clip.clone();
    let removed = edit_clip(player, |c| c.tracks.get_mut(track).is_some_and(|t| t.remove_key(k))).unwrap_or(false);
    if removed {
        state.history.push(crate::history::EditorAction::TransformClipChanged { entity, before, after: player.clip.clone() });
        state.has_unsaved_changes = true;
        state.anim_edit.selected = None;
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, PointerButton, Pos2};
    use gizmo_math::Vec3;

    fn frame(ctx: &egui::Context, world: &World, state: &mut EditorState, events: Vec<Event>) -> egui::FullOutput {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(900.0, 300.0))),
            events,
            ..Default::default()
        };
        ctx.run_ui(input, |ui| crate::animation_panel::ui_animation(ui, world, state))
    }

    /// Every text the frame painted, with where.
    fn texts(output: &egui::FullOutput) -> Vec<(String, egui::Rect)> {
        fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
            match shape {
                egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.visual_bounding_rect())),
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let mut out = Vec::new();
        for s in &output.shapes {
            walk(&s.shape, &mut out);
        }
        out
    }

    /// Clicks the text `label` the last frame painted.
    fn click(ctx: &egui::Context, world: &World, state: &mut EditorState, label: &str) {
        let shown = frame(ctx, world, state, vec![]);
        let at = texts(&shown).into_iter().find(|(t, _)| t == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no `{label}` drawn"));
        shown.drop_without_applying_deltas();
        frame(ctx, world, state, vec![Event::PointerMoved(at)]).drop_without_applying_deltas();
        let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(ctx, world, state, vec![press(true)]).drop_without_applying_deltas();
        frame(ctx, world, state, vec![press(false)]).drop_without_applying_deltas();
    }

    fn translation_at(world: &World, e: Entity, t: f32) -> Option<Vec3> {
        let p = world.get::<TransformAnimationPlayer>(e)?;
        let clip = p.clip.as_ref()?;
        clip.tracks.iter().find_map(|tr| match tr.sample(t) {
            InterpolatedValue::Translation(v) => Some(v),
            _ => None,
        })
    }

    /// **◆ Key keys the transform at the playhead, in the editor's own frame.** A selected entity with
    /// a transform player: the panel draws it; a click on ◆ Key at 0 s with the entity at the origin,
    /// then — moved to x = 4, the playhead at 1 s — another, makes a position, a rotation and a scale
    /// track of two keys each, the position halfway at 0.5 s, the rows drawn. An undo takes the
    /// second key back, a redo returns it.
    #[test]
    fn key_records_the_transform_at_the_playhead() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::ZERO));
        world.add_component(e, TransformAnimationPlayer::new());
        let mut state = EditorState::new();
        state.selection.entities.insert(e);
        let ctx = egui::Context::default();

        click(&ctx, &world, &mut state, "◆ Key");
        if let Some(mut t) = world.query_mut::<&mut Transform>().get_mut(e) {
            t.position = Vec3::new(4.0, 0.0, 0.0);
        }
        if let Some(mut p) = world.query_mut::<&mut TransformAnimationPlayer>().get_mut(e) {
            p.elapsed_time = 1.0;
        }
        click(&ctx, &world, &mut state, "◆ Key");

        let clip = world.get::<TransformAnimationPlayer>(e).and_then(|p| p.clip.clone()).expect("a clip was made");
        assert_eq!(clip.tracks.len(), 3, "position, rotation, scale");
        assert!(clip.tracks.iter().all(|t| t.keyframe_timestamps == vec![0.0, 1.0] && t.target_name.is_empty()));
        assert_eq!(translation_at(&world, e, 0.5), Some(Vec3::new(2.0, 0.0, 0.0)));
        let drawn = frame(&ctx, &world, &mut state, vec![]);
        let labels: Vec<String> = texts(&drawn).into_iter().map(|(t, _)| t).collect();
        drawn.drop_without_applying_deltas();
        for row in ["this  position", "this  rotation", "this  scale"] {
            assert!(labels.iter().any(|l| l == row), "`{row}` is not drawn: {labels:?}");
        }

        state.history.undo(&mut world);
        let keys = world.get::<TransformAnimationPlayer>(e).and_then(|p| p.clip.as_ref().map(|c| c.tracks[0].keyframe_timestamps.clone()));
        assert_eq!(keys, Some(vec![0.0]), "undone: the first key alone");
        state.history.redo(&mut world);
        assert_eq!(translation_at(&world, e, 1.0), Some(Vec3::new(4.0, 0.0, 0.0)), "redone");
    }

    /// Keying the same moment again replaces the key there rather than adding a second.
    #[test]
    fn keying_the_same_moment_replaces_the_key() {
        let a = key_transform(None, 0.5, &Transform::new(Vec3::X));
        let b = key_transform(Some(&a), 0.5, &Transform::new(Vec3::Y));
        assert!(b.tracks.iter().all(|t| t.keyframe_timestamps.len() == 1));
        assert_eq!(b.tracks[0].sample(0.5), InterpolatedValue::Translation(Vec3::Y));
    }
}
