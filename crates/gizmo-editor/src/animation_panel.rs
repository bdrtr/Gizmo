//! The ANIMATION timeline: tracks on the left, keyframes on a ruler on the right.
//!
//! # Why this is a panel and not a feature
//!
//! Everything drawn here already existed. The skeletal `AnimationClip` carries `translations`,
//! `rotations` and `scales` as tracks of timestamped keyframes, plus a `duration`, and
//! `AnimationPlayer` holds `current_time` / `active_animation` / `loop_anim`. Its doc comment even
//! anticipates this panel: *"the editor's timeline slider scrubs exactly this field"*. The editor's
//! entire animation UI was one "Loop" checkbox in the inspector, so a clip you could load, play and
//! blend was one you could not *see*.
//!
//! # Which of the two AnimationPlayers this reads
//!
//! There are two, unrelated, with the same name — `gizmo_animation::skeletal::AnimationPlayer` and
//! `gizmo_animation::player::AnimationPlayer` (the crate's own doc calls this out). This panel
//! reads the **skeletal** one, because that is what the studio actually drives:
//! `animation_update_system` logs "updating skeletal players", and it is the only one
//! `gizmo-renderer` re-exports, so it is the only one the editor can see without a new dependency.
//!
//! Nothing here writes keyframes. Scrubbing moves `elapsed_time`, which the animation system
//! already treats as the source of truth, and that is the only mutation — an editor that could
//! author keyframes needs an undo story, and this one is a viewer.
//!
//! # What the prototype has that this does not
//!
//! Per-channel rows (`position.x` separately from `position.y`), the auto-key toggle, and dragging
//! a keyframe. The channel split is presentational and could be added; the other two are authoring,
//! which is the line above. Recorded in `docs/FIXPLAN.md` rather than faked with dead controls.

use crate::editor_state::EditorState;
use crate::theme::palette::*;
use gizmo_core::World;

/// Height of one track row.
const ROW: f32 = 18.0;
/// Width of the track-name column, matching the prototype's `Tracks` gutter.
const GUTTER: f32 = 150.0;
/// Radius of a keyframe marker, and therefore the lane's inset at each end.
const MARK: f32 = 3.5;

/// Draws the animation timeline for the selected entity's player.
pub fn ui_animation(ui: &mut egui::Ui, world: &World, state: &mut EditorState) {
    let Some(entity) = state.selection.entities.iter().next().copied() else {
        empty(ui, "Bir varlık seç — animasyon oynatıcısı olan bir varlık.");
        return;
    };

    // SAFETY: the editor UI runs single-threaded inside the egui draw; no concurrent World access.
    let mut players =
        unsafe { world.borrow_mut_unchecked::<gizmo_renderer::components::AnimationPlayer>() };
    let Some(mut player) = players.get_mut(entity.id()) else {
        empty(ui, "Seçili varlıkta AnimationPlayer yok.");
        return;
    };
    let Some(clip) = player.current_clip().cloned() else {
        empty(ui, "Oynatıcıda etkin klip yok (active_animation aralık dışında olabilir).");
        return;
    };

    // Stored, not derived — see the field's doc. A zero duration means an empty clip, and dividing
    // by it would put every keyframe on top of the playhead.
    let duration = clip.duration.max(f32::EPSILON);

    // ── Header: clip name, position, transport ───────────────────────────────────────────────
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("A N I M A T I O N").size(10.0).color(TEXT_MUTED));
        ui.label(egui::RichText::new(&clip.name).size(11.0).color(TEXT_BRIGHT));
        ui.label(
            egui::RichText::new(format!("{:.2} / {:.2} s", player.current_time, duration))
                .size(10.0)
                .color(TEXT_DIM),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            crate::theme::toggle(ui, &mut player.loop_anim, "Loop");
        });
    });
    ui.separator();

    // Rows: every track of the clip, in the prototype's order.
    struct Row {
        label: String,
        times: Vec<f32>,
    }
    let mut rows: Vec<Row> = Vec::new();
    for t in &clip.translations {
        rows.push(Row { label: format!("{}  position", track_name(t.target_node_name.as_deref())), times: t.keyframes.iter().map(|k| k.time).collect() });
    }
    for t in &clip.rotations {
        rows.push(Row { label: format!("{}  rotation", track_name(t.target_node_name.as_deref())), times: t.keyframes.iter().map(|k| k.time).collect() });
    }
    for t in &clip.scales {
        rows.push(Row { label: format!("{}  scale", track_name(t.target_node_name.as_deref())), times: t.keyframes.iter().map(|k| k.time).collect() });
    }

    // ── Timeline ─────────────────────────────────────────────────────────────────────────────
    let visible_rows = rows.len().max(1);
    let wanted = ROW * (visible_rows as f32 + 1.0); // +1 for the ruler
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), wanted.min(ui.available_height().max(ROW * 2.0))),
        egui::Sense::hover(),
    );
    let painter = ui.painter_at(rect);

    // Inset by the marker radius at both ends: a keyframe at t=0 or t=duration otherwise lands
    // exactly on the lane edge and gets clipped in half by the panel — which reads as a rendering
    // glitch rather than as the last key of the clip.
    let lane = egui::Rect::from_min_max(
        egui::pos2(rect.left() + GUTTER + MARK, rect.top()),
        egui::pos2(rect.right() - MARK, rect.bottom()),
    );
    if lane.width() < 40.0 {
        return; // panel too narrow for a timeline to mean anything
    }
    let x_of = |t: f32| lane.left() + (t / duration).clamp(0.0, 1.0) * lane.width();

    // Ruler: a tick per second, labelled.
    painter.line_segment(
        [
            egui::pos2(rect.left(), rect.top() + ROW),
            egui::pos2(rect.right(), rect.top() + ROW),
        ],
        egui::Stroke::new(1.0_f32, BORDER),
    );
    painter.text(
        egui::pos2(rect.left() + 6.0, rect.top() + 3.0),
        egui::Align2::LEFT_TOP,
        "Tracks",
        egui::FontId::proportional(10.0),
        TEXT_MUTED,
    );
    for s in 0..=(duration.ceil() as i32) {
        let x = x_of(s as f32);
        painter.line_segment(
            [egui::pos2(x, rect.top() + ROW - 4.0), egui::pos2(x, rect.top() + ROW)],
            egui::Stroke::new(1.0_f32, BORDER_HOT),
        );
        painter.text(
            egui::pos2(x + 2.0, rect.top() + 2.0),
            egui::Align2::LEFT_TOP,
            s.to_string(),
            egui::FontId::proportional(9.0),
            TEXT_DIM,
        );
    }

    for (i, row) in rows.iter().enumerate() {
        let y = rect.top() + ROW * (i as f32 + 1.0);
        if y + ROW > rect.bottom() {
            break; // out of panel; the scroll story belongs to a later pass
        }
        let row_rect =
            egui::Rect::from_min_size(egui::pos2(rect.left(), y), egui::vec2(rect.width(), ROW));
        if i % 2 == 1 {
            painter.rect_filled(row_rect, 0.0, CHROME);
        }
        painter.text(
            egui::pos2(rect.left() + 6.0, row_rect.center().y),
            egui::Align2::LEFT_CENTER,
            &row.label,
            egui::FontId::proportional(10.0),
            TEXT_BODY,
        );
        for t in &row.times {
            diamond(&painter, egui::pos2(x_of(*t), row_rect.center().y), MARK, ACCENT);
        }
    }

    // Playhead, and scrubbing.
    let head_x = x_of(player.current_time);
    painter.line_segment(
        [egui::pos2(head_x, rect.top()), egui::pos2(head_x, rect.bottom())],
        egui::Stroke::new(1.0_f32, TEXT_BRIGHT),
    );

    let scrub = ui.interact(lane, ui.id().with("anim_scrub"), egui::Sense::click_and_drag());
    if scrub.dragged() || scrub.clicked() {
        if let Some(pos) = scrub.interact_pointer_pos() {
            player.current_time =
                ((pos.x - lane.left()) / lane.width()).clamp(0.0, 1.0) * duration;
        }
    }
}

/// A track's target, or a placeholder when the clip did not name one.
///
/// `target_node_name` is optional: glTF channels can target a node index with no name, and
/// `evaluate_clip` falls back to the index. A row labelled with an empty string would look like a
/// rendering bug rather than like missing data.
fn track_name(name: Option<&str>) -> &str {
    match name {
        Some(n) if !n.is_empty() => n,
        _ => "(node)",
    }
}

/// A keyframe marker: the prototype draws these as diamonds, and a diamond reads as a point in
/// time in a way a square does not.
fn diamond(painter: &egui::Painter, c: egui::Pos2, r: f32, color: egui::Color32) {
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(c.x, c.y - r),
            egui::pos2(c.x + r, c.y),
            egui::pos2(c.x, c.y + r),
            egui::pos2(c.x - r, c.y),
        ],
        color,
        egui::Stroke::NONE,
    ));
}

/// The panel's empty states, which are most of what it shows in a scene with no animation — so
/// they say which of the three things is missing rather than going blank.
fn empty(ui: &mut egui::Ui, why: &str) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("A N I M A T I O N").size(10.0).color(TEXT_MUTED));
        ui.label(egui::RichText::new(why).size(10.0).color(TEXT_DIM));
    });
}
