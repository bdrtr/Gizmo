//! The fighting-game HUD: the two fighters' health bars and names, the round and its timer, and
//! the K.O. — in the game's own UI, so it is drawn wherever the game runs.
//!
//! It was the editor's: its Game panel painted the bars with egui, fed by the studio, so the
//! exported game of a fight scene had none, and "the shipped game behaves like the one you pressed
//! ▶ on" did not hold for it. [`fight_hud_system`] builds the HUD out of UI nodes when a world has
//! two fighters, player 1 and player 2 ([`FighterController::player_id`]), keeps it in step with
//! them each frame, and takes it down when they are gone. [`crate::systems::PlayLoop`] runs it, in
//! the editor's ▶ and in the exported runtime alike; a game with its own loop may call it.
//!
//! The round is a resource, [`FightRound`]: its timer counts down while both fighters stand, and a
//! game that plays more than one round advances it.

use crate::core::component::EntityName;
use crate::core::{Entity, World};
use crate::math::{Vec2, Vec4};
use crate::physics::components::FighterController;
use crate::renderer::components::{Text, TextAlign, TextAnchor};
use crate::ui::{BackgroundColor, NodeBundle, PositionType, ProgressBar, Style, UiRect, Val};
use gizmo_core::hierarchy::HierarchyExt;

/// The round a fight is in, as its HUD shows it.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct FightRound {
    /// Which round, counting from 1.
    pub round: u32,
    /// Seconds left on its clock. [`fight_hud_system`] counts it down while both fighters stand.
    pub seconds_left: f32,
}

impl Default for FightRound {
    /// Round 1, 99 seconds.
    fn default() -> Self {
        Self { round: 1, seconds_left: 99.0 }
    }
}

/// The HUD's own entities, kept so a frame updates them rather than spawning them again.
#[derive(Debug, Clone, Copy)]
struct FightHud {
    root: Entity,
    bars: [Entity; 2],
    names: [Entity; 2],
    round: Entity,
    timer: Entity,
    ko: Entity,
}

/// Where the bars start, how tall they are, and the gap between them in the middle.
const MARGIN: f32 = 16.0;
const BAR_HEIGHT: f32 = 20.0;
const BAR_WIDTH_PERCENT: f32 = 42.0;

/// Text in the engine's default face, which the frame resolves by name — this system has no
/// renderer to load one with.
fn hud_text(content: &str, size_px: f32, color: Vec4, anchor: TextAnchor, align: TextAlign) -> Text {
    let mut text = Text::screen(content, crate::renderer::text::FontId::UNRESOLVED, size_px, Vec2::ZERO)
        .with_anchor(anchor)
        .with_align(align)
        .with_color(color);
    text.font_name = Some("default".to_string());
    text
}

fn node(world: &mut World, parent: Option<Entity>, style: Style, background: Vec4) -> Entity {
    let e = world.spawn_bundle(NodeBundle { style, background_color: BackgroundColor(background), ..Default::default() });
    if let Some(parent) = parent {
        world.add_child(parent, e);
    }
    e
}

fn at(left: Val, top: Val, right: Val, width: Val, height: Val) -> Style {
    Style {
        position_type: PositionType::Absolute,
        inset: UiRect { left, top, right, bottom: Val::Auto },
        width,
        height,
        ..Default::default()
    }
}

fn build(world: &mut World) -> FightHud {
    let clear = Vec4::ZERO;
    let root = node(
        world,
        None,
        Style { width: Val::Percent(100.0), height: Val::Percent(100.0), ..Default::default() },
        clear,
    );
    let track = Vec4::new(0.12, 0.12, 0.12, 0.85);
    let bar_style = |left: bool| {
        if left {
            at(Val::Px(MARGIN), Val::Px(MARGIN), Val::Auto, Val::Percent(BAR_WIDTH_PERCENT), Val::Px(BAR_HEIGHT))
        } else {
            at(Val::Auto, Val::Px(MARGIN), Val::Px(MARGIN), Val::Percent(BAR_WIDTH_PERCENT), Val::Px(BAR_HEIGHT))
        }
    };
    let bars = [true, false].map(|left| {
        let e = node(world, Some(root), bar_style(left), track);
        world.add_component(e, ProgressBar::new(1.0));
        e
    });
    let names = [true, false].map(|left| {
        let style = at(
            if left { Val::Px(MARGIN) } else { Val::Auto },
            Val::Px(MARGIN + BAR_HEIGHT + 4.0),
            if left { Val::Auto } else { Val::Px(MARGIN) },
            Val::Percent(BAR_WIDTH_PERCENT),
            Val::Px(18.0),
        );
        let e = node(world, Some(root), style, clear);
        let (anchor, align, color) = if left {
            (TextAnchor::CenterLeft, TextAlign::Left, Vec4::new(0.4, 0.7, 1.0, 1.0))
        } else {
            (TextAnchor::CenterRight, TextAlign::Right, Vec4::new(1.0, 0.47, 0.47, 1.0))
        };
        world.add_component(e, hud_text("", 14.0, color, anchor, align));
        e
    });
    let centre = |top: f32, height: f32| at(Val::Percent(50.0 - 4.0), Val::Px(top), Val::Auto, Val::Percent(8.0), Val::Px(height));
    let round = node(world, Some(root), centre(MARGIN - 2.0, 22.0), clear);
    world.add_component(round, hud_text("", 18.0, Vec4::new(1.0, 0.86, 0.31, 1.0), TextAnchor::Center, TextAlign::Center));
    let timer = node(world, Some(root), centre(MARGIN + BAR_HEIGHT + 4.0, 28.0), clear);
    world.add_component(timer, hud_text("", 24.0, Vec4::ONE, TextAnchor::Center, TextAlign::Center));
    let ko = node(
        world,
        Some(root),
        at(Val::Percent(20.0), Val::Percent(38.0), Val::Auto, Val::Percent(60.0), Val::Px(120.0)),
        clear,
    );
    world.add_component(ko, hud_text("", 64.0, Vec4::new(1.0, 0.2, 0.2, 1.0), TextAnchor::Center, TextAlign::Center));
    FightHud { root, bars, names, round, timer, ko }
}

/// A fighter's place on the HUD: its health, what the bar is full at, and its name.
struct Corner {
    health: f32,
    max: f32,
    name: String,
}

fn corners(world: &World) -> [Option<Corner>; 2] {
    let names = world.query::<&EntityName>();
    let mut found = [None, None];
    for (e, f) in world.query::<(Entity, &FighterController)>().iter() {
        let slot = match f.player_id {
            1 => 0,
            2 => 1,
            _ => continue,
        };
        let name = names.get(e).map_or_else(|| format!("Player {}", f.player_id), |n| n.0.clone());
        found[slot] = Some(Corner { health: f.health, max: f.max_health, name });
    }
    found
}

fn set_text(world: &mut World, e: Entity, content: &str) {
    if let Some(mut t) = world.query_mut::<&mut Text>().get_mut(e) {
        if t.content != content {
            t.content = content.to_string();
        }
    }
}

/// Keeps the fight HUD in step with the world's two fighters: built when player 1 and player 2 are
/// both there, updated every frame, taken down when either is gone. The round's clock
/// ([`FightRound`]) counts down by `dt` while both stand and stops at a K.O. or at zero.
pub fn fight_hud_system(world: &mut World, dt: f32) {
    let [Some(p1), Some(p2)] = corners(world) else {
        if let Some(hud) = world.remove_resource::<FightHud>() {
            if let Some(root) = world.live(hud.root) {
                world.despawn_recursive(root);
            }
        }
        return;
    };
    let standing = p1.health > 0.0 && p2.health > 0.0;
    let round = {
        let mut round = world.get_resource_mut_or_default::<FightRound>();
        if standing {
            round.seconds_left = (round.seconds_left - dt.max(0.0)).max(0.0);
        }
        *round
    };
    let hud = match world.get_resource::<FightHud>().map(|h| *h) {
        Some(hud) if world.live(hud.root).is_some() => hud,
        _ => {
            let hud = build(world);
            world.insert_resource(hud);
            hud
        }
    };
    let low = |ratio: f32| ratio <= 0.3;
    for (i, c) in [&p1, &p2].into_iter().enumerate() {
        let ratio = if c.max > 0.0 { (c.health / c.max).clamp(0.0, 1.0) } else { 0.0 };
        let fill = match (i, low(ratio)) {
            (0, false) => Vec4::new(0.16, 0.55, 1.0, 1.0),
            (0, true) => Vec4::new(1.0, 0.24, 0.24, 1.0),
            (_, false) => Vec4::new(1.0, 0.24, 0.24, 1.0),
            (_, true) => Vec4::new(1.0, 0.78, 0.16, 1.0),
        };
        if let Some(mut bar) = world.query_mut::<&mut ProgressBar>().get_mut(hud.bars[i]) {
            bar.value = ratio;
            bar.fill = fill;
        }
        set_text(world, hud.names[i], &c.name);
    }
    set_text(world, hud.round, &format!("R{}", round.round));
    set_text(world, hud.timer, &format!("{}", round.seconds_left.ceil() as u32));
    let ko = if standing {
        String::new()
    } else {
        let winner = if p1.health > 0.0 { &p1.name } else if p2.health > 0.0 { &p2.name } else { "" };
        if winner.is_empty() { "K.O.".to_string() } else { format!("K.O. — {winner}") }
    };
    set_text(world, hud.ko, &ko);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::components::FighterController;

    fn fighter(world: &mut World, name: &str, player: u8, health: f32) -> Entity {
        let mut f = FighterController::new(player);
        f.health = health;
        f.max_health = 100.0;
        world.spawn_bundle((EntityName::new(name), f))
    }

    fn text(world: &World, e: Entity) -> String {
        world.query::<&Text>().get(e).map(|t| t.content.clone()).unwrap_or_default()
    }

    /// Two fighters: the bars show their health, the plates their names, the clock counts down
    /// while both stand and stops at a K.O., which names the winner — and with a fighter gone the
    /// HUD goes too.
    #[test]
    fn the_hud_follows_the_two_fighters_and_goes_with_them() {
        let mut world = World::new();
        let ece = fighter(&mut world, "Ece", 1, 80.0);
        let can = fighter(&mut world, "Can", 2, 25.0);
        fight_hud_system(&mut world, 0.5);
        let hud = *world.get_resource::<FightHud>().expect("built");
        let value = |world: &World, e| world.query::<&ProgressBar>().get(e).map(|b| b.value).unwrap();
        assert_eq!((value(&world, hud.bars[0]), value(&world, hud.bars[1])), (0.8, 0.25));
        assert_eq!((text(&world, hud.names[0]), text(&world, hud.names[1])), ("Ece".to_string(), "Can".to_string()));
        assert_eq!((text(&world, hud.round), text(&world, hud.timer)), ("R1".to_string(), "99".to_string()));
        assert_eq!(world.get_resource::<FightRound>().map(|r| r.seconds_left), Some(98.5));
        assert_eq!(text(&world, hud.ko), "");
        assert_eq!(world.query::<&Text>().get(hud.timer).and_then(|t| t.font_name.clone()).as_deref(), Some("default"), "the face the frame resolves");

        world.query_mut::<&mut FighterController>().get_mut(can).unwrap().health = 0.0;
        fight_hud_system(&mut world, 0.5);
        fight_hud_system(&mut world, 0.5);
        assert_eq!(text(&world, hud.ko), "K.O. — Ece");
        assert_eq!(world.get_resource::<FightRound>().map(|r| r.seconds_left), Some(98.5), "the clock stopped at the K.O.");
        assert_eq!(world.get_resource::<FightHud>().map(|h| h.root), Some(hud.root), "updated, not built again");

        world.despawn(ece);
        fight_hud_system(&mut world, 0.5);
        assert!(world.get_resource::<FightHud>().is_none());
        assert!(!world.is_alive(hud.root) && !world.is_alive(hud.bars[0]), "taken down with its nodes");
    }
}
