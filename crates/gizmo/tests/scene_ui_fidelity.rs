//! **A UI saved in the middle of use opens as it stood, and is used the same way afterwards.**
//!
//! The UI half of "saved = seen" (decided 2026-11-14; `scene_fidelity` in `gizmo-studio` holds the
//! list). Until then no UI component was registered: a saved menu came back as bare entities with
//! names — no boxes, no colours, no controls — and the editor's ⏹, which restores the world from a
//! scene, emptied every UI a game had built in play. And `Interaction` and `Clicked` were on the
//! "not saved" list as "this frame's pointer state", which was half right: the state is the
//! pointer's, but the component is what makes a node answer the pointer, so a reopened button
//! would never have been clickable again.
//!
//! Each test builds a UI on the real systems (`gizmo_ui::register`), runs it, saves it through
//! `gizmo::full_scene_registry()`, opens it into a fresh world with the same systems and asserts on
//! what the component is for. A pointer gesture — a drag, a press — is not part of a scene: it
//! comes back at rest, and the last test is that it does, and that nothing is let go of, clicked or
//! dropped on the first frame because of it.
#![cfg(all(feature = "ui", feature = "scene"))]

use gizmo::core::component::{Children, EntityName, Parent};
use gizmo::core::input::Input;
use gizmo::core::system::Schedule;
use gizmo::core::window::WindowInfo;
use gizmo::core::world::World;
use gizmo::core::Entity;
use gizmo::math::{Ease, Vec2, Vec4};
use gizmo::scene::scene::SceneData;
use gizmo::ui::text_input::TextInput;
use gizmo::ui::{
    BackgroundColor, Clicked, ClipContent, Drag, DropTarget, Dropdown, FlexDirection, GridTrack, GridTracks, Interaction,
    Node, ProgressBar, Scroll, Slider, Style, Toggle, Tooltip, UiRoot, UiTransform, UiTween, Val, VirtualButton,
    VirtualStick, ZIndex,
};

const DT: f32 = 1.0 / 60.0;

/// A world with the UI's systems, as a game has them, and the resources they read.
fn ui_world() -> (World, Schedule) {
    let mut world = World::new();
    let mut schedule = Schedule::new();
    gizmo::ui::register(&mut world, &mut schedule);
    world.register_component_type::<Parent>();
    world.register_component_type::<Children>();
    world.insert_resource(Input::new());
    world.insert_resource(gizmo::core::time::Time::new());
    *world.get_resource_mut::<WindowInfo>().unwrap() = WindowInfo::new(400.0, 300.0);
    schedule.build();
    (world, schedule)
}

/// One frame: the clock, then the systems, then the input's edge flags cleared for the next.
fn frame(world: &mut World, schedule: &mut Schedule) {
    world.get_resource_mut::<gizmo::core::time::Time>().unwrap().update(DT);
    schedule.run(world, DT);
    world.get_resource_mut::<Input>().unwrap().begin_frame();
}

fn frames(world: &mut World, schedule: &mut Schedule, n: usize) {
    for _ in 0..n {
        frame(world, schedule);
    }
}

fn reopen(world: &World) -> (World, Schedule) {
    let registry = gizmo::full_scene_registry();
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("gizmo_ui_fidelity_{}_{n}.scene", std::process::id()));
    let path = path.to_string_lossy().to_string();
    SceneData::save(world, &path, &registry).expect("save");
    let (mut reopened, schedule) = ui_world();
    let loaded = SceneData::load_into(&path, &mut reopened, &registry);
    let _ = std::fs::remove_file(&path);
    loaded.expect("load");
    (reopened, schedule)
}

fn entity(world: &World, name: &str) -> Entity {
    world
        .query::<(Entity, &EntityName)>()
        .iter()
        .find(|(_, n)| n.0 == name)
        .map(|(e, _)| e)
        .unwrap_or_else(|| panic!("no entity named {name:?}"))
}

fn get<T: Clone + gizmo::core::component::Component>(world: &World, name: &str) -> T {
    world.query::<&T>().get(entity(world, name)).unwrap_or_else(|| panic!("{name:?} has no {}", std::any::type_name::<T>())).clone()
}

fn node(world: &World, name: &str) -> Node {
    get::<Node>(world, name)
}

fn pointer(world: &mut World, at: Vec2, press: bool, release: bool) {
    let mut input = world.get_resource_mut::<Input>().unwrap();
    input.on_mouse_moved(at.x, at.y);
    if press {
        input.on_mouse_button_pressed(0);
    }
    if release {
        input.on_mouse_button_released(0);
    }
}

fn centre(n: &Node) -> Vec2 {
    n.position + n.size * 0.5
}

/// A node named `name` with `style`, under `parent`.
fn spawn(world: &mut World, name: &str, parent: Option<Entity>, style: Style) -> Entity {
    let e = world.spawn();
    world.add_component(e, EntityName::new(name));
    world.add_component(e, style);
    world.add_component(e, Node::default());
    if let Some(p) = parent {
        world.add_component(e, Parent(p));
        let mut kids = world.query::<&Children>().get(p).map(|c| c.0.clone()).unwrap_or_default();
        kids.push(e);
        world.add_component(p, Children(kids));
    }
    e
}

fn row(h: f32) -> Style {
    Style { width: Val::Px(180.0), height: Val::Px(h), flex_shrink: 0.0, ..Default::default() }
}

fn button(world: &mut World, e: Entity) {
    world.add_component(e, Interaction::None);
    world.add_component(e, Clicked(false));
}

/// A settings menu: a grid panel of controls, each in some state a player put it in.
fn build_menu(world: &mut World) {
    let root = spawn(
        world,
        "menu",
        None,
        Style {
            width: Val::Px(400.0),
            height: Val::Px(300.0),
            display: gizmo::ui::Display::Grid,
            grid_template_columns: GridTracks::new(&[GridTrack::Px(190.0), GridTrack::Fr(1.0)]),
            grid_auto_rows: GridTrack::Px(44.0),
            column_gap: Val::Px(6.0),
            row_gap: Val::Px(4.0),
            padding: gizmo::ui::UiRect::all(Val::Px(8.0)),
            ..Default::default()
        },
    );
    world.add_component(root, UiRoot);
    world.add_component(root, BackgroundColor(Vec4::new(0.1, 0.1, 0.15, 0.9)));
    world.add_component(root, ZIndex(3));
    world.add_component(root, ClipContent);

    let volume = spawn(world, "volume", Some(root), row(40.0));
    button(world, volume);
    world.add_component(volume, Slider::new(0.35).with_step(0.1).with_fill(Vec4::new(0.9, 0.5, 0.1, 1.0)));

    let vsync = spawn(world, "vsync", Some(root), row(40.0));
    button(world, vsync);
    world.add_component(vsync, Toggle::new(true));
    world.add_component(vsync, Tooltip::new("Wait for the screen").with_delay(0.2));
    world.add_component(vsync, gizmo::app::locale::LocalizedText::new("settings.vsync").with_arg("hz", 60));

    let quality = spawn(world, "quality", Some(root), row(40.0));
    button(world, quality);
    let mut dropdown = Dropdown::new(["Low", "Medium", "High", "Ultra"], 2);
    dropdown.set_open(true);
    world.add_component(quality, dropdown);

    let name = spawn(world, "name", Some(root), row(40.0));
    button(world, name);
    let mut field = TextInput::new(12);
    field.value = "Şebnem".into();
    field.set_focused(true);
    world.add_component(name, field);

    let loading = spawn(world, "loading", Some(root), row(40.0));
    world.add_component(loading, ProgressBar::new(0.6));

    let list = spawn(world, "list", Some(root), Style { flex_direction: FlexDirection::Column, ..row(40.0) });
    let mut scroll = Scroll::default();
    scroll.offset = Vec2::new(0.0, 25.0);
    world.add_component(list, scroll);
    for i in 0..5 {
        spawn(world, &format!("entry{i}"), Some(list), row(20.0));
    }

    let stick = spawn(world, "stick", Some(root), row(40.0));
    world.add_component(stick, VirtualStick::new(48.0));
    let fire = spawn(world, "fire", Some(root), row(40.0));
    world.add_component(fire, VirtualButton::default());

    let card = spawn(world, "card", Some(root), row(40.0));
    world.add_component(card, UiTransform { translation: Vec2::new(12.0, -4.0), opacity: 0.8 });
    let toast = spawn(world, "toast", Some(root), row(40.0));
    world.add_component(toast, UiTween::translation(Vec2::new(0.0, 60.0), 0.5).and_opacity(0.0).with_ease(Ease::OutCubic).with_delay(0.05));
}

const NAMES: &[&str] = &[
    "menu", "volume", "vsync", "quality", "name", "loading", "list", "entry0", "entry4", "stick", "fire", "card", "toast",
];

/// **A menu opens as it stood: laid out the same, every control holding what it held.** The panel
/// is a grid on layer 3 with a slider, a switch, an open select list, a focused text field, a
/// progress bar, a scrolled list, a stick, a button, a moved card and a toast half way through its
/// tween. After a reopen and one frame each box is where it was, and every value is the one saved.
#[test]
fn a_menu_reopens_laid_out_as_it_was_with_every_control_where_it_was() {
    let (mut world, mut schedule) = ui_world();
    build_menu(&mut world);
    frames(&mut world, &mut schedule, 10);
    let (mut reopened, mut schedule2) = reopen(&world);
    frame(&mut world, &mut schedule);
    frame(&mut reopened, &mut schedule2);

    for name in NAMES {
        assert_eq!(node(&reopened, name), node(&world, name), "{name} is laid out somewhere else after a reopen");
    }
    assert!(node(&world, "entry4").size.y > 0.0, "the menu was never laid out — the comparison would be empty");
    assert_eq!(get::<ZIndex>(&reopened, "menu"), ZIndex(3));
    assert_eq!(get::<BackgroundColor>(&reopened, "menu"), get::<BackgroundColor>(&world, "menu"));
    assert!(reopened.query::<&ClipContent>().get(entity(&reopened, "menu")).is_some(), "the clip was lost");
    assert!(reopened.query::<&UiRoot>().get(entity(&reopened, "menu")).is_some(), "the root mark was lost");
    assert_eq!(get::<Style>(&reopened, "menu"), get::<Style>(&world, "menu"), "the grid's style came back otherwise");

    assert_eq!(get::<Slider>(&reopened, "volume").value, 0.35);
    assert_eq!(get::<Slider>(&reopened, "volume").step, 0.1);
    assert!(get::<Toggle>(&reopened, "vsync").on);
    assert_eq!(get::<Tooltip>(&reopened, "vsync").text, "Wait for the screen");
    assert_eq!(
        get::<gizmo::app::locale::LocalizedText>(&reopened, "vsync"),
        gizmo::app::locale::LocalizedText::new("settings.vsync").with_arg("hz", 60),
        "the label's key in the string table"
    );
    let quality = get::<Dropdown>(&reopened, "quality");
    assert_eq!((quality.selected, quality.is_open(), quality.options.len()), (2, true, 4), "the select list");
    let field = get::<TextInput>(&reopened, "name");
    assert_eq!((field.value.as_str(), field.is_focused(), field.max_chars), ("Şebnem", true, 12), "the text field");
    assert_eq!(get::<ProgressBar>(&reopened, "loading").value, 0.6);
    let (scroll, was) = (get::<Scroll>(&reopened, "list"), get::<Scroll>(&world, "list"));
    assert!(was.offset.y > 0.0, "the list was not scrolled — nothing to compare");
    assert_eq!(scroll.offset, was.offset, "the list is scrolled somewhere else");
    assert_eq!(get::<VirtualStick>(&reopened, "stick").radius, 48.0);
    assert!(reopened.query::<&VirtualButton>().get(entity(&reopened, "fire")).is_some());
    assert_eq!(get::<UiTransform>(&reopened, "card"), get::<UiTransform>(&world, "card"));
}

/// **A tween goes on from where it had got to.** The toast is caught a third of the way through
/// its fade; the original and the reopened copy then run the same frames, and stay together until
/// both have arrived.
#[test]
fn a_ui_tween_goes_on_along_the_same_path_after_a_reopen() {
    let (mut world, mut schedule) = ui_world();
    build_menu(&mut world);
    frames(&mut world, &mut schedule, 13);
    let caught = get::<UiTransform>(&world, "toast");
    assert!(caught.opacity < 0.9 && caught.opacity > 0.05, "not caught in motion: {caught:?}");
    let (mut reopened, mut schedule2) = reopen(&world);
    for i in 0..40 {
        frame(&mut world, &mut schedule);
        frame(&mut reopened, &mut schedule2);
        assert_eq!(get::<UiTransform>(&reopened, "toast"), get::<UiTransform>(&world, "toast"), "frame {i} after the reopen");
    }
    assert!(get::<UiTween>(&reopened, "toast").is_finished());
}

/// **A reopened UI is used as it was: its buttons click, its switches flip, its list picks.**
/// Without `Interaction` a node does not answer the pointer, and without `Clicked` it has no click
/// — both were on the "not saved" list until this test.
#[test]
fn a_reopened_menu_still_answers_the_pointer() {
    let (mut world, mut schedule) = ui_world();
    build_menu(&mut world);
    frames(&mut world, &mut schedule, 3);
    let (mut w, mut s) = reopen(&world);
    frame(&mut w, &mut s);

    // Close the list first (a press off it), so it does not take the pointer.
    let quality = entity(&w, "quality");
    w.query_mut::<&mut Dropdown>().get_mut(quality).unwrap().set_open(false);
    let vsync = centre(&node(&w, "vsync"));
    pointer(&mut w, vsync, true, false);
    frame(&mut w, &mut s);
    pointer(&mut w, vsync, false, true);
    frame(&mut w, &mut s);
    assert!(!get::<Toggle>(&w, "vsync").on, "a click on the reopened switch did not flip it");

    let volume = node(&w, "volume");
    let at = Vec2::new(volume.position.x + volume.size.x * 0.8, volume.position.y + volume.size.y * 0.5);
    pointer(&mut w, at, true, false);
    frame(&mut w, &mut s);
    pointer(&mut w, at, false, true);
    frame(&mut w, &mut s);
    let value = get::<Slider>(&w, "volume").value;
    assert!((value - 0.8).abs() < 0.05, "a press on the reopened slider at 80 % set it to {value}");
}

/// **A gesture under way is not part of a scene.** Saved with a card held in a drag over a target
/// and the switch's button held down, the reopened scene's first frame — the pointer resting over
/// both, no button held — lets go of nothing: no drop on the target, no click on the switch, and
/// the card is not being dragged. Brought back held, the release that follows a reopen would have
/// done all three.
#[test]
fn a_drag_and_a_press_under_way_come_back_at_rest() {
    let (mut world, mut schedule) = ui_world();
    build_menu(&mut world);
    let card = entity(&world, "card");
    button(&mut world, card);
    world.add_component(card, Drag::default());
    let bin = entity(&world, "loading");
    world.add_component(bin, DropTarget::default());
    let quality = entity(&world, "quality");
    world.query_mut::<&mut Dropdown>().get_mut(quality).unwrap().set_open(false);
    frames(&mut world, &mut schedule, 2);
    let from = centre(&node(&world, "card"));
    let over_bin = centre(&node(&world, "loading"));
    pointer(&mut world, from, true, false);
    frame(&mut world, &mut schedule);
    pointer(&mut world, over_bin, false, false);
    frame(&mut world, &mut schedule);
    assert!(get::<Drag>(&world, "card").is_dragging(), "the card is not held — nothing under way to save");
    assert_eq!(get::<DropTarget>(&world, "loading").hovered_by(), Some(card));
    let vsync = centre(&node(&world, "vsync"));
    pointer(&mut world, vsync, false, false);
    // The press on the switch, held: the drag goes on, the switch reads Pressed.
    let switch = entity(&world, "vsync");
    *world.query_mut::<&mut Interaction>().get_mut(switch).unwrap() = Interaction::Pressed;

    let (mut w, mut s) = reopen(&world);
    // The pointer rests over the switch and the bin, nothing held.
    pointer(&mut w, vsync, false, false);
    frame(&mut w, &mut s);
    assert!(!get::<Drag>(&w, "card").is_dragging(), "the reopened card is still being dragged");
    assert!(!get::<Drag>(&w, "card").just_ended(), "the reopened drag was let go of on the first frame");
    assert_eq!(get::<DropTarget>(&w, "loading").dropped(), None, "something was dropped on the bin by the reopen");
    assert!(!get::<Clicked>(&w, "vsync").0, "the reopen clicked the switch");
    assert!(get::<Toggle>(&w, "vsync").on, "the reopen flipped the switch");
}
