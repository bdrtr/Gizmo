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
//! # Editing, and the two invariants it has to keep
//!
//! Keyframes can be dragged in time and deleted. Both go through `Track::retime_keyframe` /
//! `remove_keyframe` rather than touching `keyframes` directly, because the list **must stay
//! sorted by time** — sampling binary-searches it, so a key dragged past its neighbour does not
//! error, it silently returns the wrong segment. And every edit ends in
//! `AnimationClip::grow_duration_to_fit`: `duration` is stored rather than derived, and one
//! shorter than the real last keyframe truncates the clip's tail with nothing detecting it.
//!
//! A whole drag is ONE undo entry, taken on release — the same shape the transform gizmo uses.
//! The snapshot is the entire `Arc<[AnimationClip]>`, not a keyframe index: a retime reorders the
//! list by definition, so an index-based undo would move a different key than the one dragged.
//!
//! # What it edits is this ENTITY's clips, not the asset
//!
//! `AnimationPlayer::animations` is a per-spawn `Arc<[AnimationClip]>` — the glTF spawner builds a
//! fresh one per instance, so a hundred copies of a model hold a hundred independent clip sets.
//! Editing here therefore changes one entity and neither its siblings nor the file on disk. That
//! is a real limitation, not a design: writing back to the asset is a separate job.
//!
//! # What the prototype has that this does not
//!
//! **Per-channel rows do not apply**, and this was recorded as "presentational, and addable" until
//! it was measured. A glTF sampler stores one timestamp per `Vec3`: `Track<Vec3>` holds
//! `Keyframe<Vec3>`, so `position.x` and `position.y` share every keyframe there is. Splitting the
//! row into three would draw three *identical* rows of diamonds, each implying a key you could
//! drag on its own — and dragging any of them moves all three, because they are one keyframe. It
//! would be a control that lies about the data underneath it.
//!
//! What the split is actually for is answering "what does this track do", and that *can* be
//! measured: each row says which axes move (`position · x z`) or that none do (`· const`). See
//! [`varying_axes`].
//!
//! **Auto-key is not missing, it does not apply.** The prototype's auto-key records the selected
//! object's transform as a keyframe. This timeline plays a *skeletal* clip: its tracks target
//! glTF joints by node name, and `evaluate_clip` writes them onto a `SkeletonHierarchy`, not onto
//! the entity's `Transform`. Keying an entity transform into one would be a category error. The
//! engine does have a transform-track animation type (`gizmo_animation::clip`), but nothing
//! outside that module uses it — the studio drives the skeletal player. Recorded in
//! `docs/FIXPLAN.md` rather than faked with a toggle that keys the wrong thing.

use crate::editor_state::{EditorState, KeyframeRef, TrackChannel};
use crate::theme::palette::*;
use gizmo_core::World;

/// Height of one track row.
const ROW: f32 = 18.0;
/// Width of the track-name column, matching the prototype's `Tracks` gutter.
const GUTTER: f32 = 150.0;
/// Radius of a keyframe marker, and therefore the lane's inset at each end.
const MARK: f32 = 3.5;
/// Width of a keyframe's interactive rect. Wider than the mark on purpose: a 7 px target is a
/// dexterity test, and a keyframe you cannot reliably grab reads as a broken timeline.
const HIT: f32 = 11.0;

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

    // Rows: every track of the clip, in the prototype's order. Each row remembers which of the
    // three channel lists it came from and its index in it — a row index alone cannot address a
    // track, since the three lists are separate `Vec`s.
    struct Row {
        label: String,
        channel: TrackChannel,
        track: usize,
        times: Vec<f32>,
    }
    let mut rows: Vec<Row> = Vec::new();
    for (i, t) in clip.translations.iter().enumerate() {
        let values: Vec<gizmo_math::Vec3> = t.keyframes.iter().map(|k| k.value).collect();
        rows.push(Row {
            label: format!(
                "{}  position{}",
                track_name(t.target_node_name.as_deref()),
                axis_suffix(varying_axes(&values))
            ),
            channel: TrackChannel::Translation,
            track: i,
            times: t.keyframes.iter().map(|k| k.time).collect(),
        });
    }
    for (i, t) in clip.rotations.iter().enumerate() {
        // A quaternion's x/y/z/w are not axes anyone poses by, so a rotation row says only whether
        // it turns at all — which is the part worth knowing when a clip has fifty bone tracks.
        let moves = t.keyframes.first().is_some_and(|f| {
            t.keyframes
                .iter()
                .any(|k| !k.value.abs_diff_eq(f.value, AXIS_EPS))
        });
        rows.push(Row {
            label: format!(
                "{}  rotation{}",
                track_name(t.target_node_name.as_deref()),
                if moves { "" } else { CONST_SUFFIX }
            ),
            channel: TrackChannel::Rotation,
            track: i,
            times: t.keyframes.iter().map(|k| k.time).collect(),
        });
    }
    for (i, t) in clip.scales.iter().enumerate() {
        let values: Vec<gizmo_math::Vec3> = t.keyframes.iter().map(|k| k.value).collect();
        rows.push(Row {
            label: format!(
                "{}  scale{}",
                track_name(t.target_node_name.as_deref()),
                axis_suffix(varying_axes(&values))
            ),
            channel: TrackChannel::Scale,
            track: i,
            times: t.keyframes.iter().map(|k| k.time).collect(),
        });
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

    // Editing rebuilds the player's `Arc<[AnimationClip]>`, and the rows above were laid out from
    // the copy taken before that. Rather than draw a frame from stale rows, ask for one more.
    let mut pending_redraw = false;

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
        for (k, t) in row.times.iter().enumerate() {
            let here = KeyframeRef { channel: row.channel, track: row.track, keyframe: k };
            let centre = egui::pos2(x_of(*t), row_rect.center().y);
            // A diamond is 7 px across; a hit area that small is a dexterity test, so the
            // interactive rect is deliberately larger than the mark it stands for.
            let hit = egui::Rect::from_center_size(centre, egui::vec2(HIT, ROW));
            let resp = ui.interact(
                hit,
                ui.id().with(("anim_key", row.channel as u8, row.track, k)),
                egui::Sense::click_and_drag(),
            );

            if resp.drag_started() {
                // One undo entry per drag: snapshot before the first mutation.
                state.anim_drag_original = Some(player.animations.clone());
                state.anim_edit.dragging = Some(here);
                state.anim_edit.selected = Some(here);
            }
            if resp.clicked() {
                state.anim_edit.selected = Some(here);
            }

            let is_dragged = state.anim_edit.dragging == Some(here);
            let is_selected = state.anim_edit.selected == Some(here);
            let colour = if is_dragged || resp.hovered() {
                TEXT_BRIGHT
            } else if is_selected {
                ACCENT_LIGHT
            } else {
                ACCENT
            };
            diamond(&painter, centre, if is_selected { MARK + 1.0 } else { MARK }, colour);

            if is_dragged {
                if let Some(pos) = resp.interact_pointer_pos() {
                    let t_new =
                        ((pos.x - lane.left()) / lane.width()).clamp(0.0, 1.0) * duration;
                    // The index follows the key. A retime can push it past its neighbours, and
                    // holding the old index would grab a DIFFERENT keyframe on the next frame of
                    // the same drag — the key you are holding would swap under the cursor.
                    if let Some(landed) = retime(&mut player, row.channel, row.track, k, t_new) {
                        state.anim_edit.dragging = Some(KeyframeRef { keyframe: landed, ..here });
                        state.anim_edit.selected = state.anim_edit.dragging;
                        pending_redraw = true;
                    }
                }
            }
            if resp.drag_stopped() {
                if let Some(before) = state.anim_drag_original.take() {
                    let after = player.animations.clone();
                    state.history.push(crate::history::EditorAction::AnimationClipsChanged {
                        entity,
                        before,
                        after,
                    });
                    state.has_unsaved_changes = true;
                }
                state.anim_edit.dragging = None;
            }

            resp.context_menu(|ui| {
                if ui.button("🗑 Keyframe'i sil").clicked() {
                    let before = player.animations.clone();
                    if remove(&mut player, row.channel, row.track, k) {
                        state.history.push(
                            crate::history::EditorAction::AnimationClipsChanged {
                                entity,
                                before,
                                after: player.animations.clone(),
                            },
                        );
                        state.has_unsaved_changes = true;
                        state.anim_edit.selected = None;
                        pending_redraw = true;
                    }
                    ui.close();
                }
            });
        }
    }

    // Playhead, and scrubbing.
    let head_x = x_of(player.current_time);
    painter.line_segment(
        [egui::pos2(head_x, rect.top()), egui::pos2(head_x, rect.bottom())],
        egui::Stroke::new(1.0_f32, TEXT_BRIGHT),
    );

    // Scrubbing owns the lane's background, but a keyframe's own rect is registered first and
    // wins the pointer — so dragging a key retimes it instead of also dragging the playhead.
    let scrub = ui.interact(lane, ui.id().with("anim_scrub"), egui::Sense::click_and_drag());
    if state.anim_edit.dragging.is_none() && (scrub.dragged() || scrub.clicked()) {
        if let Some(pos) = scrub.interact_pointer_pos() {
            player.current_time =
                ((pos.x - lane.left()) / lane.width()).clamp(0.0, 1.0) * duration;
        }
    }

    // Delete removes the selected keyframe. Gated on the panel actually having the pointer:
    // Delete is also "despawn the selected entity" in the viewport, and a global key that means
    // two things depending on where you last clicked is how you lose an object.
    let delete_pressed = ui.input(|i| i.key_pressed(egui::Key::Delete));
    if delete_pressed && rect.contains(ui.input(|i| i.pointer.hover_pos()).unwrap_or(rect.max)) {
        if let Some(sel) = state.anim_edit.selected {
            let before = player.animations.clone();
            if remove(&mut player, sel.channel, sel.track, sel.keyframe) {
                state.history.push(crate::history::EditorAction::AnimationClipsChanged {
                    entity,
                    before,
                    after: player.animations.clone(),
                });
                state.has_unsaved_changes = true;
                state.anim_edit.selected = None;
                pending_redraw = true;
            }
        }
    }

    if pending_redraw {
        ui.ctx().request_repaint();
    }
}

/// Rebuild the player's clip set with `edit` applied to one track of the active clip, and return
/// whatever `edit` returned.
///
/// `AnimationPlayer::animations` is an `Arc<[AnimationClip]>` documented as immutable — the way to
/// change it is to swap the whole thing. So an edit is: clone the slice, mutate the one clip,
/// grow its duration back into step, put a fresh `Arc` back. That is also exactly what makes the
/// undo snapshot free, since both `Arc`s already exist.
fn edit_active_clip<R>(
    player: &mut gizmo_renderer::components::AnimationPlayer,
    edit: impl FnOnce(&mut gizmo_renderer::AnimationClip) -> R,
) -> Option<R> {
    let index = player.active_animation;
    let mut clips: Vec<gizmo_renderer::AnimationClip> = player.animations.to_vec();
    let clip = clips.get_mut(index)?;
    let out = edit(clip);
    // The one thing no edit may skip: a stored duration shorter than the real last keyframe
    // truncates the clip's tail, and nothing anywhere detects it.
    clip.grow_duration_to_fit();
    player.animations = clips.into();
    Some(out)
}

/// Move one keyframe, returning the index it landed at.
fn retime(
    player: &mut gizmo_renderer::components::AnimationPlayer,
    channel: TrackChannel,
    track: usize,
    keyframe: usize,
    time: f32,
) -> Option<usize> {
    edit_active_clip(player, |clip| match channel {
        TrackChannel::Translation => clip.translations.get_mut(track)?.retime_keyframe(keyframe, time),
        TrackChannel::Rotation => clip.rotations.get_mut(track)?.retime_keyframe(keyframe, time),
        TrackChannel::Scale => clip.scales.get_mut(track)?.retime_keyframe(keyframe, time),
    })
    .flatten()
}

/// Delete one keyframe, reporting whether there was one.
fn remove(
    player: &mut gizmo_renderer::components::AnimationPlayer,
    channel: TrackChannel,
    track: usize,
    keyframe: usize,
) -> bool {
    edit_active_clip(player, |clip| match channel {
        TrackChannel::Translation => clip
            .translations
            .get_mut(track)
            .is_some_and(|t| t.remove_keyframe(keyframe)),
        TrackChannel::Rotation => clip
            .rotations
            .get_mut(track)
            .is_some_and(|t| t.remove_keyframe(keyframe)),
        TrackChannel::Scale => clip
            .scales
            .get_mut(track)
            .is_some_and(|t| t.remove_keyframe(keyframe)),
    })
    .unwrap_or(false)
}

/// How far a value has to move before the row calls the axis animated.
///
/// Exported keyframes carry float noise — a "constant" axis is rarely bit-identical across fifty
/// keys — so an exact comparison would mark every track as moving on every axis and the annotation
/// would say nothing. A micrometre is below anything an animator posed on purpose.
const AXIS_EPS: f32 = 1e-6;

/// What a row with nothing moving is marked with.
const CONST_SUFFIX: &str = " · const";

/// Which axes of a `Vec3` track actually move across its keyframes.
///
/// This is the honest half of the prototype's per-channel rows. It cannot split a track into
/// `position.x` / `position.y` / `position.z` — a glTF sampler stores one timestamp per `Vec3`, so
/// those three share every keyframe and three rows would be three copies of one — but the question
/// the split answers, *what does this track do*, is a measurement.
///
/// An empty track moves on nothing.
fn varying_axes(values: &[gizmo_math::Vec3]) -> [bool; 3] {
    let Some(first) = values.first() else {
        return [false; 3];
    };
    let mut moves = [false; 3];
    for v in values {
        for axis in 0..3 {
            if (v[axis] - first[axis]).abs() > AXIS_EPS {
                moves[axis] = true;
            }
        }
    }
    moves
}

/// The suffix a row label carries for its measured axes — and nothing at all when every axis
/// moves, because a note that is true of every row is noise on all of them.
fn axis_suffix(moves: [bool; 3]) -> String {
    if moves == [true; 3] {
        return String::new();
    }
    if moves == [false; 3] {
        return CONST_SUFFIX.to_string();
    }
    let named: String = ["x", "y", "z"]
        .iter()
        .zip(moves)
        .filter(|(_, m)| *m)
        .map(|(n, _)| *n)
        .collect::<Vec<_>>()
        .join(" ");
    format!(" · {named}")
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

#[cfg(test)]
mod tests {
    use super::{remove, retime};
    use crate::editor_state::TrackChannel;
    use gizmo_renderer::components::AnimationPlayer;
    use gizmo_renderer::{AnimationClip, InterpolationMode, Keyframe, Track};

    fn player(duration: f32, times: &[f32]) -> AnimationPlayer {
        let track = Track {
            target_node: 0,
            target_node_name: Some("Hips".to_string()),
            interpolation: InterpolationMode::Linear,
            keyframes: times
                .iter()
                .map(|&t| Keyframe {
                    time: t,
                    // The value encodes the time it was authored at, so a retime that moved the
                    // timestamps but not the values is visible.
                    value: gizmo_math::Vec3::splat(t),
                    in_tangent: None,
                    out_tangent: None,
                })
                .collect(),
        };
        AnimationPlayer {
            animations: std::sync::Arc::new([AnimationClip {
                name: "test".to_string(),
                duration,
                translations: vec![track],
                rotations: Vec::new(),
                scales: Vec::new(),
            }]),
            ..Default::default()
        }
    }

    fn times(p: &AnimationPlayer) -> Vec<f32> {
        p.animations[0].translations[0]
            .keyframes
            .iter()
            .map(|k| k.time)
            .collect()
    }

    use super::{axis_suffix, varying_axes, AXIS_EPS, CONST_SUFFIX};
    use gizmo_math::Vec3;

    /// The measurement behind the row annotation: which axes actually move.
    #[test]
    fn only_the_axes_that_move_are_reported() {
        let vals = vec![
            Vec3::new(0.0, 1.0, 5.0),
            Vec3::new(2.0, 1.0, 5.0),
            Vec3::new(4.0, 1.0, 9.0),
        ];
        assert_eq!(varying_axes(&vals), [true, false, true]);
        assert_eq!(axis_suffix(varying_axes(&vals)), " · x z");
    }

    /// Exported keyframes carry float noise, so an exact comparison would call every axis animated
    /// and the annotation would say nothing on any row.
    ///
    /// The offsets here are chosen to survive `f32`: `1.0 + 1e-9` rounds straight back to `1.0`,
    /// so a fixture written that way compares two identical numbers and passes with the threshold
    /// deleted. `5e-7` next to `0.0` is exactly representable and genuinely below it.
    #[test]
    fn float_noise_does_not_count_as_movement() {
        let vals = vec![Vec3::new(0.0, 2.0, 3.0), Vec3::new(5e-7, 2.0, 3.0)];
        assert_ne!(vals[0].x, vals[1].x, "the fixture must actually differ in f32");
        assert_eq!(varying_axes(&vals), [false; 3]);
        assert_eq!(axis_suffix(varying_axes(&vals)), CONST_SUFFIX);
    }

    /// Movement is measured against the **first** keyframe, not the previous one, and a slow ramp
    /// is where the two part company: each step here is below the threshold while the whole run is
    /// far above it. Comparing neighbours would call a track that travels a full unit "const".
    #[test]
    fn a_slow_ramp_is_movement_even_though_every_step_is_noise() {
        let vals: Vec<Vec3> = (0..2000).map(|i| Vec3::new(0.0, i as f32 * 5e-7, 0.0)).collect();
        let step = vals[1].y - vals[0].y;
        assert!(step < AXIS_EPS, "each step must be below the threshold, got {step}");
        assert!(vals.last().unwrap().y > 1e-4, "...and the total must be far above it");
        assert_eq!(varying_axes(&vals), [false, true, false]);
    }

    /// Movement is measured against the FIRST keyframe, not against the neighbour: a track that
    /// leaves and comes back has moved, even though consecutive pairs at the ends match.
    #[test]
    fn a_track_that_returns_to_its_start_still_counts_as_moving() {
        let vals = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 3.0, 0.0),
            Vec3::new(0.0, 0.0, 0.0),
        ];
        assert_eq!(varying_axes(&vals), [false, true, false]);
    }

    /// A note that is true of every row is noise on all of them, so the all-axes case says
    /// nothing at all.
    #[test]
    fn a_track_that_moves_on_everything_is_left_unannotated() {
        assert_eq!(axis_suffix([true; 3]), "");
    }

    #[test]
    fn an_empty_track_moves_on_nothing() {
        assert_eq!(varying_axes(&[]), [false; 3]);
        assert_eq!(varying_axes(&[Vec3::ONE]), [false; 3], "a single key is a constant");
    }

    /// The panel's whole editing contract in one test: the key moves, the list stays sorted, the
    /// stored duration grows to cover it, and the player is holding a NEW `Arc` (which is what
    /// makes the undo snapshot free — and what `animations` being documented immutable requires).
    #[test]
    fn an_edit_swaps_the_arc_and_grows_the_duration() {
        let mut p = player(2.0, &[0.0, 1.0, 2.0]);
        let original = p.animations.clone();

        let landed = retime(&mut p, TrackChannel::Translation, 0, 0, 5.0);

        assert_eq!(landed, Some(2), "the key must report where it landed");
        assert_eq!(times(&p), vec![1.0, 2.0, 5.0]);
        assert_eq!(p.animations[0].duration, 5.0, "duration must cover the new last key");
        assert!(
            !std::sync::Arc::ptr_eq(&original, &p.animations),
            "the player must be holding a fresh Arc, not a mutated shared one"
        );
        assert_eq!(
            original[0].duration, 2.0,
            "and the snapshot taken before the edit must be untouched — that is the undo entry"
        );
    }

    /// The drag holds an index across frames, and a retime can push the key past its neighbours.
    /// If the reported index did not follow it, the next frame of the same drag would grab a
    /// different keyframe and the one under the cursor would swap.
    #[test]
    fn the_index_follows_the_key_across_a_reorder() {
        let mut p = player(3.0, &[0.0, 1.0, 2.0]);
        // Drag the first key all the way right, one "frame" at a time.
        let mut at = 0usize;
        for t in [0.5, 1.5, 2.5] {
            at = retime(&mut p, TrackChannel::Translation, 0, at, t).expect("a live key");
        }
        assert_eq!(times(&p), vec![1.0, 2.0, 2.5]);
        assert_eq!(at, 2);
        // ...and it is still the key we started dragging: its value was authored at t = 0.
        assert_eq!(p.animations[0].translations[0].keyframes[at].value.x, 0.0);
    }

    #[test]
    fn deleting_takes_that_key_and_leaves_the_rest_sorted() {
        let mut p = player(2.0, &[0.0, 1.0, 2.0]);
        assert!(remove(&mut p, TrackChannel::Translation, 0, 1));
        assert_eq!(times(&p), vec![0.0, 2.0]);
        assert!(p.animations[0].tracks_are_sorted());
    }

    /// `active_animation` is a public `usize` with no validation — out of range is a supported
    /// state that makes `current_clip` return `None`. An edit there must change nothing rather
    /// than panic on the index.
    #[test]
    fn an_edit_with_no_active_clip_changes_nothing() {
        let mut p = player(2.0, &[0.0, 1.0]);
        p.active_animation = 99;
        assert_eq!(retime(&mut p, TrackChannel::Translation, 0, 0, 5.0), None);
        assert!(!remove(&mut p, TrackChannel::Translation, 0, 0));
        p.active_animation = 0;
        assert_eq!(times(&p), vec![0.0, 1.0], "the clip must be exactly as it was");
    }

    /// A channel the clip has no tracks in, and a track index past the end: both are reachable
    /// from a stale row list after another edit reordered things.
    #[test]
    fn an_edit_on_a_track_that_is_not_there_changes_nothing() {
        let mut p = player(2.0, &[0.0, 1.0]);
        assert_eq!(retime(&mut p, TrackChannel::Rotation, 0, 0, 5.0), None);
        assert_eq!(retime(&mut p, TrackChannel::Translation, 7, 0, 5.0), None);
        assert!(!remove(&mut p, TrackChannel::Scale, 0, 0));
        assert_eq!(times(&p), vec![0.0, 1.0]);
        assert_eq!(p.animations[0].duration, 2.0);
    }
}
