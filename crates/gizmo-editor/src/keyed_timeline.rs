//! The ANIMATION tab for an entity's **keyed transforms and properties** — Godot's animation editor
//! for a node's position, rotation and scale and its components' fields: a door that swings, a
//! platform that rides, a camera that sweeps, a lamp that pulses, a sprite that fades.
//!
//! The selected entity's [`KeyedAnimationPlayer`] is shown as a track per channel, keyframes on
//! a ruler, a playhead. **◆ Key** records the entity's transform as it stands now — position,
//! rotation and scale — at the playhead, into tracks of its own (an empty target name: the
//! player's own entity), making them the first time, and every property track of this entity at
//! its current value; so an animation is made by scrubbing, moving the object with the gizmo or
//! editing it in the inspector, and keying, as in Godot. **+ Property** adds a track for one of
//! the entity's animatable fields (the world's `PropertyRegistry`, which lists only components the
//! entity has), keyed once at the playhead with the field's current value; a property row's own ◆
//! keys that row alone. Keys drag in time, delete with a right click or Delete; ▶ plays it in the
//! editor (the engine's frame drives the player in both draw paths, and a paused one follows its
//! playhead, which the ruler scrubs). Every edit is one undo entry, the clip whole
//! (`EditorAction::TransformClipChanged`), for the reason the skeletal timeline's are: a retime
//! reorders the keys.
//!
//! **A keyed field follows its track, paused or not**: an inspector edit to it snaps back on the
//! next frame unless it is keyed (◆) — the same as moving a keyed transform with the gizmo.
//!
//! Tracks that target a named entity below the player draw too, under its name. A property row the
//! registry does not know, keyed with the wrong kind of value, or whose target lacks the component
//! is drawn in the warning colour with the reason on hover — never hidden.

use crate::animation_panel::{draw_ruler, key_mark, Timeline, GUTTER, HIT, ROW};

/// What a row's label leaves free at the gutter's right edge: room for the row's own ◆.
const LABEL_RESERVE: f32 = 16.0;
use crate::editor_state::{EditorState, KeyframeRef, TrackChannel};
use crate::theme::palette::*;
use gizmo_core::{Entity, World};
use gizmo_physics_core::Transform;
use gizmo_renderer::keyed_animation::{
    AnimationClip, InterpolatedValue, Keyframes, PropertyKind, PropertyRegistry, KeyedAnimationPlayer, Track,
};
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

/// Draws the transform timeline for `entity`'s player; `false` when it has none.
pub(crate) fn ui_keyed_animation(ui: &mut egui::Ui, world: &World, state: &mut EditorState, entity: Entity) -> bool {
    // SAFETY: the editor UI runs single-threaded inside the egui draw; no concurrent World access.
    let mut players = unsafe { world.query_unchecked::<&mut KeyedAnimationPlayer>() };
    let Some(mut player) = players.get_mut(entity) else {
        return false;
    };
    state.keyframes_of(entity);
    let clip = player.clip.clone();
    // A clip with no keys yet still gets a second of lane to scrub and key along.
    let duration = clip.as_ref().map_or(0.0, |c| c.duration()).max(1.0);

    // The animatable fields: the studio's frame makes the registry, with the engine's in it.
    let registry = world.get_resource::<PropertyRegistry>();
    let registry = registry.as_deref();

    let (key_now, add_property) = header(ui, world, entity, &mut player, clip.as_deref(), duration, registry);
    ui.separator();

    if let Some(name) = add_property {
        add_property_now(ui, world, state, entity, &mut player, registry, &name);
        return true;
    }
    if key_now {
        key_entity_now(ui, world, state, entity, &mut player, clip.as_deref(), registry);
        return true;
    }

    let Some(clip) = clip else {
        crate::animation_panel::empty(ui, "Henüz anahtar yok — nesneyi yerleştir, oynatma kafasını kaydır, ◆ Key.");
        return true;
    };

    // ── Timeline ─────────────────────────────────────────────────────────────────────────────
    let Some((timeline, painter)) = Timeline::lay_out(ui, clip.tracks.len(), duration) else {
        return true;
    };
    draw_ruler(&painter, &timeline);

    let mut redraw = false;
    for (i, track) in clip.tracks.iter().enumerate() {
        let y = timeline.rect.top() + ROW * (i as f32 + 1.0);
        if y + ROW > timeline.rect.bottom() {
            break;
        }
        redraw |= track_row(ui, &painter, &timeline, i, y, track, world, state, entity, &mut player, registry);
    }

    timeline.playhead(&painter, player.elapsed_time);
    if let Some(time) = timeline.scrub(ui, "transform_scrub", state.anim_edit.dragging.is_some()) {
        player.elapsed_time = time;
        redraw = true;
    }
    // Delete removes the selected key, only with the pointer over the timeline (Delete is also the
    // viewport's despawn).
    let rect = timeline.rect;
    let over = rect.contains(ui.input(|i| i.pointer.hover_pos()).unwrap_or(rect.max));
    // Said out loud, for the host's own Delete (`EditorState::timeline_hovered`): it despawned the
    // selected entity on the same press.
    state.timeline_hovered |= over;
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

/// The header: clip name, position, ▶, ◆ Key, + Property and Loop. Returns whether ◆ Key was
/// pressed, and the property + Property asked for.
fn header(
    ui: &mut egui::Ui,
    world: &World,
    entity: Entity,
    player: &mut KeyedAnimationPlayer,
    clip: Option<&AnimationClip>,
    duration: f32,
    registry: Option<&PropertyRegistry>,
) -> (bool, Option<String>) {
    // ── Header ───────────────────────────────────────────────────────────────────────────────
    let mut key_now = false;
    let mut add_property: Option<String> = None;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("A N I M A T I O N").size(10.0).color(TEXT_MUTED));
        let name = clip.map_or("—", |c| c.name.as_str());
        ui.label(egui::RichText::new(name).size(11.0).color(TEXT_BRIGHT));
        ui.label(egui::RichText::new(format!("{:.2} / {:.2} s", player.elapsed_time, duration)).size(10.0).color(TEXT_DIM));
        if ui.button(if player.playing { "⏸" } else { "▶" }).clicked() {
            if player.playing {
                player.pause();
            } else {
                player.resume();
            }
        }
        if ui.button("◆ Key").on_hover_text("Record this entity's transform and keyed properties at the playhead").clicked() {
            key_now = true;
        }
        let choices = registry.map(|r| property_choices(r, world, entity, clip)).unwrap_or_default();
        if !choices.is_empty() {
            egui::ComboBox::from_id_salt(("transform_add_property", entity))
                .selected_text("+ Property")
                .show_ui(ui, |ui| {
                    let mut group = "";
                    for (name, _) in &choices {
                        let (component, field) = name.split_once('.').unwrap_or(("", name.as_str()));
                        if component != group {
                            ui.label(egui::RichText::new(component).size(9.0).color(TEXT_MUTED));
                            group = component;
                        }
                        if ui.selectable_label(false, field).on_hover_text(name.as_str()).clicked() {
                            add_property = Some(name.clone());
                        }
                    }
                })
                .response
                .on_hover_text("Animate one of this entity's fields: a track keyed at the playhead with its value now");
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            crate::theme::toggle(ui, &mut player.looping, "Loop");
        });
    });
    (key_now, add_property)
}

/// + Property: a track for `name`, keyed at the playhead with the field's value now — one undo entry.
fn add_property_now(
    ui: &egui::Ui,
    world: &World,
    state: &mut EditorState,
    entity: Entity,
    player: &mut KeyedAnimationPlayer,
    registry: Option<&PropertyRegistry>,
    name: &str,
) {
    if let Some(value) = registry.and_then(|r| r.get(world, entity, name)) {
        let before = player.clip.clone();
        if let Some(after) = add_property_track(before.as_deref(), player.elapsed_time, name, value) {
            player.clip = Some(Arc::new(after));
            player.target_entities.clear();
            state.history.push(crate::history::EditorAction::TransformClipChanged { entity, before, after: player.clip.clone() });
            state.has_unsaved_changes = true;
            ui.ctx().request_repaint();
        }
    }
}

/// ◆ Key: the entity's transform and every property track of its own, at the playhead — one undo
/// entry.
fn key_entity_now(
    ui: &egui::Ui,
    world: &World,
    state: &mut EditorState,
    entity: Entity,
    player: &mut KeyedAnimationPlayer,
    clip: Option<&AnimationClip>,
    registry: Option<&PropertyRegistry>,
) {
    let transform = world.query::<&Transform>().get(entity).copied();
    // This entity's property tracks, each at the value its field holds now.
    let values: Vec<(String, InterpolatedValue)> = clip
        .iter()
        .flat_map(|c| c.tracks.iter())
        .filter(|t| t.target_name.is_empty())
        .filter_map(|t| t.property.as_deref())
        .filter_map(|name| Some((name.to_owned(), registry?.get(world, entity, name)?)))
        .collect();
    if transform.is_some() || !values.is_empty() {
        let before = player.clip.clone();
        let keyed = key_entity(before.as_deref(), player.elapsed_time, transform.as_ref(), &values);
        player.clip = Some(Arc::new(keyed));
        player.target_entities.clear();
        state.history.push(crate::history::EditorAction::TransformClipChanged { entity, before, after: player.clip.clone() });
        state.has_unsaved_changes = true;
        ui.ctx().request_repaint();
    }
}

/// Track `i` at `y`: its label (in the warning colour with the reason when it cannot play), a
/// property row's own ◆, and its keyframes — selectable, draggable in time, deletable from their
/// menu. Whether an edit asks for a redraw.
#[allow(clippy::too_many_arguments)]
fn track_row(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    timeline: &Timeline,
    i: usize,
    y: f32,
    track: &Track,
    world: &World,
    state: &mut EditorState,
    entity: Entity,
    player: &mut KeyedAnimationPlayer,
    registry: Option<&PropertyRegistry>,
) -> bool {
    let rect = timeline.rect;
    let mut redraw = false;
    let row_rect = timeline.row(painter, i, y);
    let target = if track.target_name.is_empty() { "this" } else { track.target_name.as_str() };
    let problem = property_problem(registry, world, entity, &player.target_entities, track);
    // The label stays in the gutter, left of the row's ◆: a long property name is cut there —
    // the whole of it on hover — rather than drawn under the ◆ and the keys.
    let text = format!("{target}  {}", channel_name(track));
    let gutter = egui::Rect::from_min_max(
        egui::pos2(rect.left(), row_rect.top()),
        egui::pos2(rect.left() + GUTTER - LABEL_RESERVE, row_rect.bottom()),
    );
    let label = painter.with_clip_rect(gutter.intersect(painter.clip_rect())).text(
        egui::pos2(rect.left() + 6.0, row_rect.center().y),
        egui::Align2::LEFT_CENTER,
        &text,
        egui::FontId::proportional(10.0),
        if problem.is_some() { WARNING } else { TEXT_BODY },
    );
    let cut = label.right() > gutter.right();
    if problem.is_some() || cut {
        let hover = match &problem {
            Some(why) => format!("{text}\n{why}"),
            None => text.clone(),
        };
        ui.interact(label.intersect(gutter), ui.id().with(("transform_row_problem", i)), egui::Sense::hover()).on_hover_text(hover);
    }
    // A property row's own ◆: this row alone, at the field's value now.
    if let (Some(name), None) = (track.property.as_deref(), &problem) {
        let owner = if track.target_name.is_empty() { Some(entity) } else { player.target_entities.get(&track.target_name).copied() };
        let at = painter.text(
            egui::pos2(rect.left() + GUTTER - 4.0, row_rect.center().y),
            egui::Align2::RIGHT_CENTER,
            "◆",
            egui::FontId::proportional(10.0),
            ACCENT,
        );
        let key = ui
            .interact(at.expand(2.0), ui.id().with(("transform_row_key", i)), egui::Sense::click())
            .on_hover_text("Key this property at the playhead");
        if key.clicked() {
            if let Some(value) = owner.and_then(|o| registry?.get(world, o, name)) {
                let before = player.clip.clone();
                let time = player.elapsed_time;
                if edit_clip(player, |c| c.tracks.get_mut(i)?.set_key(time, value)).flatten().is_some() {
                    state.history.push(crate::history::EditorAction::TransformClipChanged { entity, before, after: player.clip.clone() });
                    state.has_unsaved_changes = true;
                    redraw = true;
                }
            }
        }
    }
    for (k, t) in track.keyframe_timestamps.iter().enumerate() {
        let here = KeyframeRef { channel: TrackChannel::Translation, track: i, keyframe: k };
        let centre = egui::pos2(timeline.x_of(*t), row_rect.center().y);
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
        key_mark(painter, centre, dragged, resp.hovered(), state.anim_edit.selected == Some(here));
        if dragged {
            if let Some(pos) = resp.interact_pointer_pos() {
                // The index follows the key: a retime can carry it past its neighbours.
                if let Some(landed) = edit_clip(player, |c| c.tracks.get_mut(i)?.retime_key(k, timeline.time_at(pos.x))).flatten() {
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
                if remove_key(player, state, entity, i, k) {
                    redraw = true;
                }
                ui.close();
            }
        });
    }
    redraw
}

/// What a track keys: its channel, or the property it names.
fn channel_name(track: &Track) -> Cow<'_, str> {
    if let Some(p) = &track.property {
        return Cow::Borrowed(p.as_str());
    }
    Cow::Borrowed(match track.keyframes {
        Keyframes::Translation(_) => "position",
        Keyframes::Rotation(_) => "rotation",
        Keyframes::Scale(_) => "scale",
        _ => "?",
    })
}

/// Why a property row cannot play, if it cannot: the registry does not know the name, the keys
/// are of another kind than the property, or the target (when it is resolved) lacks the
/// component. A transform row has no such problem.
fn property_problem(
    registry: Option<&PropertyRegistry>,
    world: &World,
    entity: Entity,
    resolved: &HashMap<String, Entity>,
    track: &Track,
) -> Option<String> {
    let name = track.property.as_deref()?;
    let Some(registry) = registry else {
        return Some("no animatable properties are registered in this world".into());
    };
    let Some(kind) = registry.kind(name) else {
        return Some(format!("`{name}` is not an animatable property"));
    };
    if track.keyframes.kind() != Some(kind) {
        return Some(format!("`{name}` is a {kind}; this track's keys are not"));
    }
    let target = if track.target_name.is_empty() { Some(entity) } else { resolved.get(&track.target_name).copied() };
    match target {
        Some(t) if registry.get(world, t, name).is_none() => {
            let component = name.split('.').next().unwrap_or(name);
            Some(format!("the target has no `{component}`"))
        }
        _ => None,
    }
}

/// The properties `entity` could have a track for: every registered field of a component it has,
/// less those a track of this entity (an empty target name) already keys. In registration order,
/// so a component's fields stay together.
pub(crate) fn property_choices(
    registry: &PropertyRegistry,
    world: &World,
    entity: Entity,
    clip: Option<&AnimationClip>,
) -> Vec<(String, PropertyKind)> {
    let tracked = |name: &str| {
        clip.is_some_and(|c| c.tracks.iter().any(|t| t.target_name.is_empty() && t.property.as_deref() == Some(name)))
    };
    registry.properties_of(world, entity).filter(|(n, _)| !tracked(n)).map(|(n, k)| (n.to_owned(), k)).collect()
}

/// `clip` (a new one when there is none) with a track for `property` of the player's own entity,
/// keyed once at `time` with `value`; `None` when `value` is not a property value.
pub(crate) fn add_property_track(clip: Option<&AnimationClip>, time: f32, property: &str, value: InterpolatedValue) -> Option<AnimationClip> {
    let kind = value.kind()?;
    let mut track = Track::property("", property, Vec::new(), Keyframes::empty(kind)).ok()?;
    track.set_key(time, value)?;
    let mut clip = clip.cloned().unwrap_or_else(new_clip);
    clip.tracks.push(track);
    Some(clip)
}

/// `clip` (a new one when there is none) with `property` of the player's own entity keyed at `time`
/// to `value`: into its track when it has one, into a new track when not. `None` when `value` is not
/// a property value or not of the track's kind.
pub(crate) fn key_property(clip: Option<&AnimationClip>, time: f32, property: &str, value: InterpolatedValue) -> Option<AnimationClip> {
    let tracked = clip.and_then(|c| c.tracks.iter().position(|t| t.target_name.is_empty() && t.property.as_deref() == Some(property)));
    match (clip, tracked) {
        (Some(clip), Some(i)) => {
            let mut clip = clip.clone();
            clip.tracks[i].set_key(time, value)?;
            Some(clip)
        }
        _ => add_property_track(clip, time, property, value),
    }
}

fn new_clip() -> AnimationClip {
    let mut c = AnimationClip::default();
    c.name = "animation".into();
    c
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

/// `clip` (a new one when there is none) keyed at `time` on the player's own entity: its
/// `transform`, when it has one, into position, rotation and scale tracks, each made the first
/// time; and each `(property, value)` into this entity's existing track for that property (a
/// property gets a track only through **+ Property**).
pub(crate) fn key_entity(
    clip: Option<&AnimationClip>,
    time: f32,
    transform: Option<&Transform>,
    properties: &[(String, InterpolatedValue)],
) -> AnimationClip {
    let mut clip = clip.cloned().unwrap_or_else(new_clip);
    for (name, value) in properties {
        if let Some(track) = clip.tracks.iter_mut().find(|t| t.target_name.is_empty() && t.property.as_deref() == Some(name.as_str())) {
            track.set_key(time, *value);
        }
    }
    let Some(transform) = transform else {
        return clip;
    };
    let values = [
        InterpolatedValue::Translation(transform.position),
        InterpolatedValue::Rotation(transform.rotation),
        InterpolatedValue::Scale(transform.scale),
    ];
    for value in values {
        match clip
            .tracks
            .iter_mut()
            .find(|t| t.target_name.is_empty() && t.property.is_none() && same_channel(&t.keyframes, &value))
        {
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
fn edit_clip<R>(player: &mut KeyedAnimationPlayer, edit: impl FnOnce(&mut AnimationClip) -> R) -> Option<R> {
    let mut clip = (**player.clip.as_ref()?).clone();
    let out = edit(&mut clip);
    player.clip = Some(Arc::new(clip));
    player.target_entities.clear();
    Some(out)
}

/// Removes key `k` of track `track`, recording the undo; whether there was one.
fn remove_key(player: &mut KeyedAnimationPlayer, state: &mut EditorState, entity: Entity, track: usize, k: usize) -> bool {
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
        let p = world.get::<KeyedAnimationPlayer>(e)?;
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
        world.add_component(e, KeyedAnimationPlayer::new());
        let mut state = EditorState::new();
        state.selection.entities.insert(e);
        let ctx = egui::Context::default();

        click(&ctx, &world, &mut state, "◆ Key");
        if let Some(mut t) = world.query_mut::<&mut Transform>().get_mut(e) {
            t.position = Vec3::new(4.0, 0.0, 0.0);
        }
        if let Some(mut p) = world.query_mut::<&mut KeyedAnimationPlayer>().get_mut(e) {
            p.elapsed_time = 1.0;
        }
        click(&ctx, &world, &mut state, "◆ Key");

        let clip = world.get::<KeyedAnimationPlayer>(e).and_then(|p| p.clip.clone()).expect("a clip was made");
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
        let keys = world.get::<KeyedAnimationPlayer>(e).and_then(|p| p.clip.as_ref().map(|c| c.tracks[0].keyframe_timestamps.clone()));
        assert_eq!(keys, Some(vec![0.0]), "undone: the first key alone");
        state.history.redo(&mut world);
        assert_eq!(translation_at(&world, e, 1.0), Some(Vec3::new(4.0, 0.0, 0.0)), "redone");
    }

    // ── Property tracks ───────────────────────────────────────────────────────────────────────

    use gizmo_renderer::components::PointLight;

    /// A selected entity at the origin with a point light of `intensity`, a transform player, and
    /// the engine's registry of animatable fields in the world.
    fn lamp(intensity: f32) -> (World, Entity, EditorState) {
        let mut world = World::new();
        let mut registry = PropertyRegistry::default();
        assert!(gizmo_renderer::keyed_animation::engine_properties(&mut registry).is_empty(), "the engine's names");
        world.insert_resource(registry);
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::ZERO));
        world.add_component(e, PointLight::new(Vec3::ONE, intensity, 5.0));
        world.add_component(e, KeyedAnimationPlayer::new());
        let mut state = EditorState::new();
        state.selection.entities.insert(e);
        (world, e, state)
    }

    fn labels(ctx: &egui::Context, world: &World, state: &mut EditorState) -> Vec<String> {
        let shown = frame(ctx, world, state, vec![]);
        let out = texts(&shown).into_iter().map(|(t, _)| t).collect();
        shown.drop_without_applying_deltas();
        out
    }

    fn clip_of(world: &World, e: Entity) -> Option<Arc<AnimationClip>> {
        world.get::<KeyedAnimationPlayer>(e).and_then(|p| p.clip.clone())
    }

    fn set_playhead(world: &mut World, e: Entity, t: f32) {
        if let Some(mut p) = world.query_mut::<&mut KeyedAnimationPlayer>().get_mut(e) {
            p.pause();
            p.elapsed_time = t;
        }
    }

    /// **+ Property offers the fields of the components the entity has**, grouped under the
    /// component's name, and not the ones a track already keys.
    #[test]
    fn property_combo_lists_only_components_entity_has() {
        let (world, e, mut state) = lamp(1.0);
        let ctx = egui::Context::default();
        click(&ctx, &world, &mut state, "+ Property");
        let shown = labels(&ctx, &world, &mut state);
        for wanted in ["PointLight", "intensity", "radius", "color"] {
            assert!(shown.iter().any(|l| l == wanted), "`{wanted}` is not offered: {shown:?}");
        }
        for unwanted in ["Camera", "fov", "Material", "albedo", "Sprite"] {
            assert!(!shown.iter().any(|l| l == unwanted), "`{unwanted}` is offered, and the entity has none: {shown:?}");
        }

        let registry = world.get_resource::<PropertyRegistry>().unwrap();
        let all = property_choices(&registry, &world, e, None);
        assert!(all.iter().all(|(n, _)| n.starts_with("PointLight.")), "{all:?}");
        let tracked = add_property_track(None, 0.0, "PointLight.intensity", InterpolatedValue::Scalar(1.0)).unwrap();
        let rest = property_choices(&registry, &world, e, Some(&tracked));
        assert_eq!(rest.len(), all.len() - 1);
        assert!(!rest.iter().any(|(n, _)| n == "PointLight.intensity"), "already tracked");
    }

    /// **Picking a property makes its track with one key, at the playhead, of the field's value now**
    /// — one undo entry, which takes the clip back to none.
    #[test]
    fn adding_property_creates_one_key_at_playhead_with_current_value_and_one_undo() {
        let (mut world, e, mut state) = lamp(3.0);
        set_playhead(&mut world, e, 0.5);
        let ctx = egui::Context::default();
        click(&ctx, &world, &mut state, "+ Property");
        click(&ctx, &world, &mut state, "intensity");
        let clip = clip_of(&world, e).expect("a clip was made");
        assert_eq!(clip.tracks.len(), 1);
        let t = &clip.tracks[0];
        assert_eq!(t.property.as_deref(), Some("PointLight.intensity"));
        assert!(t.target_name.is_empty());
        assert_eq!(t.keyframe_timestamps, vec![0.5]);
        assert_eq!(t.keyframes.value(0), InterpolatedValue::Scalar(3.0));
        assert_eq!(state.history.undo_stack().len(), 1, "one undo entry");
        let drawn = labels(&ctx, &world, &mut state);
        assert!(drawn.iter().any(|l| l == "this  PointLight.intensity"), "the row is drawn under its name: {drawn:?}");
        state.history.undo(&mut world);
        assert!(clip_of(&world, e).is_none(), "undone: no clip again");
    }

    /// **◆ Key records the transform and every property track of the entity**, each at its value
    /// now; a property row's own ◆ keys that row alone.
    #[test]
    fn key_button_records_transform_and_existing_property_tracks() {
        let (mut world, e, mut state) = lamp(1.0);
        let start = add_property_track(None, 0.0, "PointLight.intensity", InterpolatedValue::Scalar(1.0)).unwrap();
        if let Some(mut p) = world.query_mut::<&mut KeyedAnimationPlayer>().get_mut(e) {
            p.clip = Some(Arc::new(start));
        }
        set_playhead(&mut world, e, 1.0);
        if let Some(mut l) = world.query_mut::<&mut PointLight>().get_mut(e) {
            l.intensity = 6.0;
        }
        let ctx = egui::Context::default();
        click(&ctx, &world, &mut state, "◆ Key");
        let clip = clip_of(&world, e).unwrap();
        let light = clip.tracks.iter().find(|t| t.property.is_some()).unwrap();
        assert_eq!(light.keyframe_timestamps, vec![0.0, 1.0]);
        assert_eq!(light.sample(0.5), InterpolatedValue::Scalar(3.5));
        assert_eq!(clip.tracks.len(), 4, "the property track, and position, rotation, scale made now");
        assert_eq!(state.history.undo_stack().len(), 1, "one undo entry");

        // The row's own ◆, at 2 s with the light at 2: that track gets a key, the others do not.
        set_playhead(&mut world, e, 2.0);
        if let Some(mut l) = world.query_mut::<&mut PointLight>().get_mut(e) {
            l.intensity = 2.0;
        }
        click(&ctx, &world, &mut state, "◆");
        let clip = clip_of(&world, e).unwrap();
        for t in &clip.tracks {
            let want = if t.property.is_some() { 3 } else { 1 };
            assert_eq!(t.keyframe_timestamps.len(), want, "{:?}", t.property);
        }
        assert_eq!(clip.tracks[0].sample(2.0), InterpolatedValue::Scalar(2.0));
        assert_eq!(state.history.undo_stack().len(), 2);

        // The pure function: a property with no track is not given one by ◆ Key.
        let keyed = key_entity(None, 0.0, None, &[("PointLight.intensity".into(), InterpolatedValue::Scalar(1.0))]);
        assert!(keyed.tracks.is_empty());
    }

    /// The colour a frame painted the text `label` in.
    fn colour_of(output: &egui::FullOutput, label: &str) -> Option<egui::Color32> {
        fn walk(shape: &egui::Shape, label: &str, out: &mut Option<egui::Color32>) {
            match shape {
                egui::Shape::Text(t) if t.galley.text() == label => {
                    *out = Some(
                        t.override_text_color
                            .unwrap_or_else(|| t.galley.job.sections.first().map_or(t.fallback_color, |s| s.format.color)),
                    )
                }
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, label, out)),
                _ => {}
            }
        }
        let mut out = None;
        output.shapes.iter().for_each(|s| walk(&s.shape, label, &mut out));
        out
    }

    /// **A property row that cannot play is drawn, in the warning colour** — a name nothing
    /// registered, and a field of a component the entity does not have — and a good row is not.
    #[test]
    fn unresolved_property_row_draws_warning() {
        let (mut world, e, mut state) = lamp(1.0);
        let mut clip = add_property_track(None, 0.0, "PointLight.intensity", InterpolatedValue::Scalar(1.0)).unwrap();
        clip.tracks.push(Track::property("", "Nope.field", vec![0.0], Keyframes::Scalar(vec![1.0])).unwrap());
        clip.tracks.push(Track::property("", "Camera.fov", vec![0.0], Keyframes::Scalar(vec![1.0])).unwrap());
        if let Some(mut p) = world.query_mut::<&mut KeyedAnimationPlayer>().get_mut(e) {
            p.clip = Some(Arc::new(clip));
        }
        let ctx = egui::Context::default();
        let shown = frame(&ctx, &world, &mut state, vec![]);
        assert_eq!(colour_of(&shown, "this  Nope.field"), Some(WARNING));
        assert_eq!(colour_of(&shown, "this  Camera.fov"), Some(WARNING));
        assert_eq!(colour_of(&shown, "this  PointLight.intensity"), Some(TEXT_BODY));
        shown.drop_without_applying_deltas();

        let registry = world.get_resource::<PropertyRegistry>().unwrap();
        let clip = clip_of(&world, e).unwrap();
        let none = HashMap::new();
        let why = |i: usize| property_problem(Some(&registry), &world, e, &none, &clip.tracks[i]);
        assert_eq!(why(0), None);
        assert!(why(1).unwrap().contains("not an animatable property"));
        assert!(why(2).unwrap().contains("no `Camera`"));
    }

    /// A long property name is cut at the gutter, left of its row's ◆ — it is not drawn under
    /// the ◆ and over the key lane, where it was before.
    #[test]
    fn long_property_label_is_clipped_to_the_gutter() {
        let (mut world, e, mut state) = lamp(1.0);
        world.add_component(e, gizmo_renderer::components::DirectionalLight::new(Vec3::ONE, 1.0, gizmo_renderer::components::LightRole::Sun));
        let name = "DirectionalLight.contact_shadow_length";
        let clip = add_property_track(None, 0.0, name, InterpolatedValue::Scalar(0.1)).unwrap();
        if let Some(mut p) = world.query_mut::<&mut KeyedAnimationPlayer>().get_mut(e) {
            p.clip = Some(Arc::new(clip));
        }
        let ctx = egui::Context::default();
        let shown = frame(&ctx, &world, &mut state, vec![]);
        let label = format!("this  {name}");
        let clipped: Vec<(egui::Rect, egui::Rect)> = shown
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(t) if t.galley.text() == label => Some((t.visual_bounding_rect(), c.clip_rect)),
                _ => None,
            })
            .collect();
        assert_eq!(clipped.len(), 1, "the row's label is painted once");
        let (text, clip) = clipped[0];
        let diamond = texts(&shown).into_iter().find(|(t, r)| t == "◆" && (r.center().y - text.center().y).abs() < 2.0);
        let (_, diamond) = diamond.expect("the property row has its own ◆");
        assert!(text.right() > diamond.left(), "the name is long enough to matter: {text:?} vs ◆ {diamond:?}");
        assert!(clip.right() <= diamond.left(), "the label is clipped left of the ◆: clip {clip:?}, ◆ {diamond:?}");
        shown.drop_without_applying_deltas();
    }

    /// Keying the same moment again replaces the key there rather than adding a second.
    #[test]
    fn keying_the_same_moment_replaces_the_key() {
        let a = key_entity(None, 0.5, Some(&Transform::new(Vec3::X)), &[]);
        let b = key_entity(Some(&a), 0.5, Some(&Transform::new(Vec3::Y)), &[]);
        assert!(b.tracks.iter().all(|t| t.keyframe_timestamps.len() == 1));
        assert_eq!(b.tracks[0].sample(0.5), InterpolatedValue::Translation(Vec3::Y));
    }
}
