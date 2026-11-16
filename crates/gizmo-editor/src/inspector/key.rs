//! The ◆ beside an animatable field's row: a click keys that field, at its value now, at the playhead
//! of the entity's own `KeyedAnimationPlayer` — Godot's key icon beside a property. The field's track
//! is made by the first click; the ANIMATION tab shows it from then on.
//!
//! Drawn only on an entity with a player, and only for a name the world's `PropertyRegistry` knows
//! (the engine's fields and the game's own), so an inspector without animation looks as it did. The
//! click is a request (`EditorState::key_property_requests`), applied by [`apply_key_requests`] once
//! the rows have let go of their components: the row holds its component mutably while it draws.

use crate::editor_state::EditorState;
use crate::theme::palette::{ACCENT, TEXT_DIM};
use gizmo_core::entity::Entity;
use gizmo_core::World;
use gizmo_renderer::keyed_animation::{KeyedAnimationPlayer, PropertyRegistry};
use std::sync::Arc;

/// The ◆ for `property` of `entity`, in the row being drawn: lit when the entity's clip already
/// keys it. Draws nothing on an entity without a `KeyedAnimationPlayer`, or for a name the
/// registry does not know.
pub(crate) fn diamond(ui: &mut egui::Ui, world: &World, state: &mut EditorState, entity: Entity, property: &str) {
    if !keyable(world, entity, property) {
        return;
    }
    let Some(player) = world.get::<KeyedAnimationPlayer>(entity) else {
        return;
    };
    let tracked = player
        .clip
        .as_ref()
        .is_some_and(|c| c.tracks.iter().any(|t| t.target_name.is_empty() && t.property.as_deref() == Some(property)));
    let (rect, response) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::click());
    #[cfg(test)]
    tests::DRAWN.with(|d| d.borrow_mut().push((property.to_owned(), rect)));
    let color = if tracked || response.hovered() { ACCENT } else { TEXT_DIM };
    crate::animation_panel::diamond(ui.painter(), rect.center(), 4.5, color);
    let hint = if tracked { "Key at the playhead" } else { "Animate: a track keyed at the playhead" };
    if response.on_hover_text(format!("{hint} — {property}")).clicked() {
        state.key_property_requests.push((entity, property.to_owned()));
    }
}

/// Whether [`diamond`] draws a ◆ for `property` of `entity`: it has a player, and the registry knows
/// the name.
pub(crate) fn keyable(world: &World, entity: Entity, property: &str) -> bool {
    world.get::<KeyedAnimationPlayer>(entity).is_some()
        && world.get_resource::<PropertyRegistry>().is_some_and(|r| r.kind(property).is_some())
}

/// Sizes the next slider to what is left of the row — and starts a new row first when too little
/// is: a slider's own width is fixed (100 px), so one after a label and a ◆ pushed the inspector
/// past its panel, and the `ScrollArea` widened every row under it to match. Call it in a
/// `horizontal_wrapped` row, just before the slider.
pub(crate) fn fit_slider(ui: &mut egui::Ui) {
    // The value box beside the track, and the least track worth dragging.
    const VALUE_BOX: f32 = 68.0;
    const MIN_TRACK: f32 = 60.0;
    // What is left of THIS row: in a wrapping layout `available_width` is the whole row's, and a
    // slider sized to it overshot, and every row under it was laid out wider still.
    if ui.available_size_before_wrap().x - VALUE_BOX < MIN_TRACK {
        ui.end_row();
    }
    ui.spacing_mut().slider_width = (ui.available_size_before_wrap().x - VALUE_BOX).max(MIN_TRACK);
}

/// A section's ◆s: the entity, the component's animatable name, and the editor state a click asks
/// through — shared, because a section's rows are nested closures that each want it for a moment.
#[derive(Clone, Copy)]
pub(crate) struct RowKey<'a> {
    pub(crate) world: &'a World,
    pub(crate) state: &'a std::cell::RefCell<&'a mut EditorState>,
    pub(crate) entity: Entity,
    pub(crate) component: &'static str,
}

impl RowKey<'_> {
    /// The ◆ for `field` of this section's component (`"intensity"` → `"PointLight.intensity"`).
    pub(crate) fn diamond(&self, ui: &mut egui::Ui, field: &str) {
        let name = format!("{}.{field}", self.component);
        diamond(ui, self.world, &mut self.state.borrow_mut(), self.entity, &name);
    }

    /// The same row's ◆ on another component of the entity.
    pub(crate) fn of(self, component: &'static str) -> Self {
        Self { component, ..self }
    }
}

/// Keys every field a ◆ asked for this frame, each at its value now, at its entity's playhead —
/// one undo entry per key.
pub(crate) fn apply_key_requests(world: &World, state: &mut EditorState) {
    for (entity, property) in std::mem::take(&mut state.key_property_requests) {
        let Some(value) = world.get_resource::<PropertyRegistry>().and_then(|r| r.get(world, entity, &property)) else {
            continue;
        };
        // SAFETY: the editor UI runs single-threaded inside the egui draw, and the inspector's rows
        // have released their component borrows by now; no other access to the player is live.
        let mut players = unsafe { world.query_unchecked::<&mut KeyedAnimationPlayer>() };
        let Some(mut player) = players.get_mut(entity) else {
            continue;
        };
        let before = player.clip.clone();
        let Some(after) = crate::keyed_timeline::key_property(before.as_deref(), player.elapsed_time, &property, value) else {
            continue;
        };
        player.clip = Some(Arc::new(after));
        player.target_entities.clear();
        state.history.push(crate::history::EditorAction::TransformClipChanged { entity, before, after: player.clip.clone() });
        state.has_unsaved_changes = true;
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use crate::editor_state::EditorState;
    use gizmo_core::World;
    use gizmo_math::Vec3;
    use gizmo_renderer::components::{Camera, DirectionalLight, LightRole, Material, MaterialMaps, PointLight, PostProcess, RectLight, SpotLight};
    use gizmo_renderer::keyed_animation::{engine_properties, KeyedAnimationPlayer, PropertyRegistry};
    use std::cell::RefCell;
    use std::sync::Arc;

    thread_local! {
        /// Every ◆ drawn on this thread, by property name, and where: what these tests read.
        pub(crate) static DRAWN: RefCell<Vec<(String, egui::Rect)>> = const { RefCell::new(Vec::new()) };
    }

    /// The engine's animatable fields that have no inspector row to put a ◆ on, and why. Short on
    /// purpose: a field a clip can key and the inspector cannot show is a field nobody can key there.
    const NO_ROW: &[(&str, &str)] = &[];

    pub(crate) fn material() -> Material {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::NOOP,
            backend_options: wgpu::BackendOptions {
                noop: wgpu::NoopBackendOptions { enable: true, ..Default::default() },
                ..Default::default()
            },
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("the noop adapter");
        let (device, _queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("a noop device");
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &[] });
        Material::new(Arc::new(device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &layout, entries: &[] })))
    }

    fn registry() -> PropertyRegistry {
        let mut r = PropertyRegistry::default();
        assert!(engine_properties(&mut r).is_empty());
        r
    }

    /// One inspector frame with `events`, and the ◆s it drew, with where.
    fn frame(ctx: &egui::Context, world: &World, state: &mut EditorState, events: Vec<egui::Event>) -> Vec<(String, egui::Rect)> {
        DRAWN.with(|d| d.borrow_mut().clear());
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(420.0, 6000.0))),
            events,
            ..Default::default()
        };
        let output = ctx.run_ui(input, |ui| crate::inspector::ui_inspector(ui, world, state));
        output.drop_without_applying_deltas();
        DRAWN.with(|d| d.borrow().clone())
    }

    /// Two inspector frames (a collapsing header settles over two), and the ◆s the second drew.
    fn drawn(world: &World, state: &mut EditorState) -> Vec<String> {
        let ctx = egui::Context::default();
        frame(&ctx, world, state, Vec::new());
        frame(&ctx, world, state, Vec::new()).into_iter().map(|(n, _)| n).collect()
    }

    /// A click at `at`: pressed on one frame, released on the next, as a hand does it.
    fn click(ctx: &egui::Context, world: &World, state: &mut EditorState, at: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(ctx, world, state, vec![egui::Event::PointerMoved(at), button(true)]);
        frame(ctx, world, state, vec![button(false)]);
    }

    /// **A ◆ keys its field at the playhead.** The point light's intensity ◆, clicked, makes the
    /// field's track with one key at 0 s holding the value it has; at 0.5 s with the intensity
    /// changed, the second click adds a key to that track, not a second track; undo takes it back.
    #[test]
    fn a_click_on_a_key_keys_the_field_at_the_playhead() {
        let mut world = World::new();
        world.insert_resource(registry());
        let e = world.spawn();
        world.add_component(e, KeyedAnimationPlayer::new());
        world.add_component(e, PointLight::new(Vec3::ONE, 2.0, 5.0));
        let mut state = EditorState::default();
        state.selection.entities.insert(e);
        state.selection.primary = Some(e);
        let ctx = egui::Context::default();
        let at = |ctx: &egui::Context, world: &World, state: &mut EditorState| {
            frame(ctx, world, state, Vec::new());
            frame(ctx, world, state, Vec::new())
                .into_iter()
                .find(|(n, _)| n == "PointLight.intensity")
                .map(|(_, r)| r.center())
                .expect("the intensity's ◆")
        };
        let track = |world: &World| {
            let clip = world.get::<KeyedAnimationPlayer>(e).and_then(|p| p.clip.clone());
            clip.map(|c| {
                let tracks: Vec<_> = c.tracks.iter().filter(|t| t.property.as_deref() == Some("PointLight.intensity")).cloned().collect();
                assert_eq!(tracks.len(), 1, "one track for the field");
                (tracks[0].keyframe_timestamps.clone(), tracks[0].sample(0.0), tracks[0].sample(0.5))
            })
        };

        let p = at(&ctx, &world, &mut state);
        click(&ctx, &world, &mut state, p);
        let (times, ..) = track(&world).expect("a clip with the field's track");
        assert_eq!(times, vec![0.0]);

        world.query_mut::<&mut KeyedAnimationPlayer>().get_mut(e).unwrap().elapsed_time = 0.5;
        world.query_mut::<&mut PointLight>().get_mut(e).unwrap().intensity = 6.0;
        let p = at(&ctx, &world, &mut state);
        click(&ctx, &world, &mut state, p);
        let (times, first, second) = track(&world).expect("the track");
        assert_eq!(times, vec![0.0, 0.5], "a second key on the same track");
        use gizmo_renderer::keyed_animation::InterpolatedValue::Scalar;
        assert!(matches!(first, Scalar(v) if v == 2.0) && matches!(second, Scalar(v) if v == 6.0), "{first:?} {second:?}");
        assert_eq!(state.history.undo_stack().len(), 2, "one undo entry per key");

        state.history.undo(&mut world);
        assert_eq!(track(&world).expect("the track").0, vec![0.0], "undo takes the second key back");
    }

    /// An entity with a player and every component the inspector has a section of its own for, a
    /// material with every map named so each map's strength is shown.
    fn everything(world: &mut World) -> gizmo_core::Entity {
        world.insert_resource(registry());
        let e = world.spawn();
        world.add_component(e, KeyedAnimationPlayer::new());
        world.add_component(e, PointLight::new(Vec3::ONE, 1.0, 5.0));
        world.add_component(e, DirectionalLight::new(Vec3::ONE, 1.0, LightRole::Sun));
        world.add_component(e, SpotLight::new(Vec3::ONE, 1.0, 5.0, 0.3, 0.5));
        world.add_component(e, RectLight::new(Vec3::ONE, 1.0, 5.0, 1.0, 1.0));
        world.add_component(e, Camera::new(1.0, 0.1, 100.0, true));
        let mut m = material();
        m.maps = MaterialMaps::default()
            .with_normal("n.png", 1.0)
            .with_emissive("e.png", Vec3::ONE)
            .with_lightmap("l.png", 1.0);
        m.maps.occlusion = Some("o.png".into());
        world.add_component(e, m);
        world.add_component(e, gizmo_renderer::components::Sprite::world(gizmo_renderer::sprite::SpriteImageId::UNRESOLVED, gizmo_math::Vec2::ONE));
        world.add_component(e, gizmo_renderer::components::Text::world("hi", gizmo_renderer::text::FontId::UNRESOLVED, 32.0, 64.0));
        e
    }

    /// **Every animatable field the inspector shows has its ◆.** The engine's names (less the
    /// lanes, which a row keys as the whole vector) on the components with an inspector section of
    /// their own — the lights, the camera, the material, and the camera's `PostProcess` in the
    /// environment panel — each drew one, on an entity with a player. A field registered and given
    /// no row fails here until it gets one or a reason on [`NO_ROW`].
    #[test]
    fn every_animatable_field_shown_in_the_inspector_has_a_key() {
        let r = registry();
        let lane = |n: &str| {
            n.rsplit_once('.').is_some_and(|(whole, l)| ["x", "y", "z", "w"].contains(&l) && r.kind(whole).is_some())
        };
        let expected: Vec<&str> = r.names().filter(|n| !lane(n) && !NO_ROW.iter().any(|(p, _)| n.starts_with(p))).collect();

        let mut world = World::new();
        let e = everything(&mut world);
        let mut state = EditorState::default();
        state.selection.entities.insert(e);
        state.selection.primary = Some(e);
        let mut seen = drawn(&world, &mut state);

        // The look, in the environment panel: nothing selected, the active camera's `PostProcess`.
        let mut world = World::new();
        world.insert_resource(registry());
        let camera = world.spawn();
        world.add_component(camera, Camera::new(1.0, 0.1, 100.0, true));
        world.add_component(camera, PostProcess::default());
        world.add_component(camera, KeyedAnimationPlayer::new());
        seen.extend(drawn(&world, &mut EditorState::default()));

        let missing: Vec<&str> = expected.iter().copied().filter(|n| !seen.iter().any(|s| s == n)).collect();
        assert!(missing.is_empty(), "animatable fields with no ◆ in the inspector: {missing:?}");
        let unknown: Vec<&String> = seen.iter().filter(|s| r.kind(s).is_none()).collect();
        assert!(unknown.is_empty(), "a ◆ for a name nothing registers: {unknown:?}");
    }

    /// **The rows the ◆s and the new material rows widened still fit the inspector** — at the
    /// default layout's 400 px and the two narrower widths the other inspector width tests hold it
    /// to. Measured unclipped: clipping is what hides an overflow on screen.
    #[test]
    fn the_keyed_sections_fit_the_inspector() {
        fn right_edge(shape: &egui::Shape, max: &mut f32) {
            match shape {
                egui::Shape::Vec(v) => v.iter().for_each(|s| right_edge(s, max)),
                other => {
                    let r = other.visual_bounding_rect();
                    if r.is_finite() {
                        *max = max.max(r.max.x);
                    }
                }
            }
        }
        let mut world = World::new();
        let e = everything(&mut world);
        for width in [400.0_f32, 320.0, 260.0] {
            let mut state = EditorState::default();
            state.selection.entities.insert(e);
            state.selection.primary = Some(e);
            let ctx = egui::Context::default();
            let mut ink = 0.0_f32;
            for _ in 0..2 {
                let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    let mut panel = ui.new_child(egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 6000.0))));
                    panel.set_clip_rect(egui::Rect::EVERYTHING);
                    crate::inspector::ui_inspector(&mut panel, &world, &mut state);
                });
                ink = 0.0;
                output.shapes.iter().for_each(|s| right_edge(&s.shape, &mut ink));
                output.drop_without_applying_deltas();
            }
            assert!(ink <= width + 1.0, "the keyed sections painted out to {ink:.1} px in a {width:.0} px inspector");
        }
    }

    /// **A game's own animatable field gets its ◆ too**, in the section the inspector draws for a
    /// component the game registered for editing: `Health.hp`, registered with the property
    /// registry, has one; `lives`, which is not, has none.
    #[test]
    fn a_games_registered_field_has_a_key() {
        #[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
        struct Health {
            hp: f32,
            lives: u32,
        }
        gizmo_core::impl_component!(Health);

        let mut world = World::new();
        let mut properties = registry();
        properties.register::<Health, f32>("Health.hp", |h| h.hp, |h, v| h.hp = v).expect("a free name");
        world.insert_resource(properties);
        let mut components = gizmo_core::ComponentRegistry::new();
        components.register_editable::<Health>("Health").unwrap();
        world.insert_resource(components);
        let e = world.spawn();
        world.add_component(e, Health { hp: 3.0, lives: 2 });
        world.add_component(e, KeyedAnimationPlayer::new());
        let mut state = EditorState::default();
        state.selection.entities.insert(e);
        state.selection.primary = Some(e);
        assert_eq!(drawn(&world, &mut state), vec!["Health.hp".to_string()]);
    }

    /// No player, no ◆: an inspector without animation looks as it did.
    #[test]
    fn an_entity_without_a_player_shows_no_key() {
        let mut world = World::new();
        let e = everything(&mut world);
        world.remove_component::<KeyedAnimationPlayer>(e);
        let mut state = EditorState::default();
        state.selection.entities.insert(e);
        state.selection.primary = Some(e);
        assert!(drawn(&world, &mut state).is_empty());
    }
}
