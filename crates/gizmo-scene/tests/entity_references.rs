//! A component's own `Entity` fields survive a scene: saved to a file and loaded, carried into the
//! editor's ▶, restored from a snapshot.
//!
//! The scene format spawns every entity again under a new id and remapped what it knew of —
//! `Parent`, `Children`, joint endpoints — but a component holding an `Entity` (a turret's target,
//! a door's key, a squad's members) was written as a number and read back as the same number:
//! after the trip it named whatever held that id in the new world, or nothing. Measured before the
//! fix: a guard saved targeting a thief pointed, in the loaded world, at the entity that had taken
//! the thief's old id. `gizmo_core::entity::references` now treats every such field as a reference.

use gizmo_core::component::{Component, EntityName};
use gizmo_core::entity::Entity;
use gizmo_core::World;
use gizmo_scene::registry::default_scene_registry;
use gizmo_scene::{SceneData, SceneRegistry, SceneSnapshot};
use serde::{Deserialize, Serialize};

/// A game's component that holds references in every common shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Squad {
    leader: Entity,
    target: Option<Entity>,
    members: Vec<Entity>,
}
impl Component for Squad {}

fn registry() -> SceneRegistry {
    let mut reg = default_scene_registry();
    reg.register_serializable::<Squad>("Squad").expect("registered");
    reg
}

fn named(world: &World, name: &str) -> Entity {
    world.query::<(Entity, &EntityName)>().iter().find(|(_, n)| n.0 == name).map(|(e, _)| e).unwrap_or_else(|| panic!("{name}"))
}

fn squad(world: &World, owner: Entity) -> Squad {
    world.query::<&Squad>().get(owner).cloned().expect("the squad survived")
}

/// A guard, a thief, two members, and an outsider the scene will not hold. The ids are shifted by
/// a few unnamed entities first, so a loaded world gives every entity a different id.
fn source() -> (World, Vec<Entity>, Entity) {
    let mut world = World::new();
    for _ in 0..3 {
        world.spawn();
    }
    let names = ["guard", "thief", "a", "b"];
    let ents: Vec<Entity> = names.iter().map(|n| world.spawn_bundle((EntityName::new(n),))).collect();
    // The editor's own furniture: never saved, as a scene file is written.
    let outsider = world.spawn_bundle((EntityName::new("outsider"), gizmo_core::component::EditorOnly));
    world.add_component(ents[0], Squad { leader: ents[0], target: Some(ents[1]), members: vec![ents[2], ents[3], outsider] });
    (world, ents, outsider)
}

/// Saved to a file and loaded into a world whose ids differ: each reference names the same entity
/// it did, by its new id; one to an entity the file does not hold reads as `INVALID`.
#[test]
fn references_survive_a_scene_file() {
    let (world, ents, _) = source();
    let reg = registry();
    let path = std::env::temp_dir().join(format!("gizmo_entity_refs_{}.scene", std::process::id()));
    let path = path.to_string_lossy().to_string();
    SceneData::save(&world, &path, &reg).expect("saved");
    let mut loaded = World::new();
    loaded.spawn(); // a different id layout from the start
    SceneData::load_into(&path, &mut loaded, &reg).expect("loads");
    let _ = std::fs::remove_file(&path);
    let guard = named(&loaded, "guard");
    let s = squad(&loaded, guard);
    assert_eq!(
        s,
        Squad { leader: guard, target: Some(named(&loaded, "thief")), members: vec![named(&loaded, "a"), named(&loaded, "b"), Entity::INVALID] },
        "each reference names its own entity in the loaded world; the outsider was not saved"
    );
    assert_ne!(guard, ents[0], "the ids did change, which is the point");
}

/// The editor's ▶ builds the play world with `of_entities` → `instantiate`: the same holds.
#[test]
fn references_survive_into_play() {
    let (world, ents, _) = source();
    let reg = registry();
    let mut play = World::new();
    let ids = SceneData::of_entities(&world, ents.clone(), &reg).instantiate(&mut play, &reg);
    let s = squad(&play, ids[&ents[0].id()]);
    assert_eq!(s.leader, ids[&ents[0].id()]);
    assert_eq!(s.target, Some(ids[&ents[1].id()]));
    assert_eq!(s.members, vec![ids[&ents[2].id()], ids[&ents[3].id()], Entity::INVALID]);
}

/// A handle to a dead entity is not revived as whatever took its slot: a stale generation is
/// written as `INVALID`, even when the slot's new occupant is saved.
#[test]
fn a_stale_reference_is_not_revived_by_its_slot() {
    let mut world = World::new();
    let gone = world.spawn_bundle((EntityName::new("gone"),));
    world.despawn(gone);
    let heir = world.spawn_bundle((EntityName::new("heir"),));
    assert_eq!(heir.id(), gone.id(), "the slot was reused");
    let keeper = world.spawn_bundle((EntityName::new("keeper"),));
    world.add_component(keeper, Squad { leader: keeper, target: Some(gone), members: vec![heir] });
    let reg = registry();
    let mut play = World::new();
    let ids = SceneData::of_entities(&world, vec![heir, keeper], &reg).instantiate(&mut play, &reg);
    let s = squad(&play, ids[&keeper.id()]);
    assert_eq!(s.target, Some(Entity::INVALID), "the dead target stays dead");
    assert_eq!(s.members, vec![ids[&heir.id()]], "while the heir itself is referenced as itself");
}

/// A snapshot restore puts an entity Play despawned back on whatever now holds its id (the
/// snapshot claims live entities by id): references to it follow it there, rather than staying
/// on its dead handle, and references to entities that were never captured but still live keep
/// naming them.
#[test]
fn references_survive_a_snapshot_restore() {
    let (mut world, ents, outsider) = source();
    let reg = registry();
    let protected: std::collections::HashSet<Entity> = [outsider].into_iter().collect();
    let snap = SceneSnapshot::capture(&world, &reg, &protected);
    // Play despawns the thief, and something else takes its slot.
    world.despawn(ents[1]);
    let squatter = world.spawn_bundle((EntityName::new("squatter"),));
    snap.restore(&mut world, &reg, &protected);
    let thief = named(&world, "thief");
    assert_eq!(thief, squatter, "the snapshot put the thief back on the entity holding its id");
    assert!(!world.is_alive(ents[1]), "and its old handle is dead");
    let s = squad(&world, ents[0]);
    assert_eq!(s.target, Some(thief), "the guard targets the restored thief, not its dead handle");
    assert_eq!(s.members, vec![ents[2], ents[3], outsider], "the uncaptured outsider is still there and still named");
}

/// A copy within the same world (`SceneData::duplicate`, the editor's `Ctrl+D`): a reference inside
/// the copied subtree names the copy's own entity, one outside it the same entity as the
/// original's. Through a temporary prefab file — what `Ctrl+D` did — the outside one would be
/// lost: a file is for other worlds.
#[test]
fn a_copy_in_the_same_world_keeps_aiming_outside_and_points_inside_at_itself() {
    let mut world = World::new();
    let player = world.spawn_bundle((EntityName::new("player"),));
    let turret = world.spawn_bundle((EntityName::new("turret"),));
    let barrel = world.spawn_bundle((EntityName::new("barrel"),));
    gizmo_core::hierarchy::HierarchyExt::add_child(&mut world, turret, barrel);
    world.add_component(turret, Squad { leader: barrel, target: Some(player), members: vec![turret] });
    let reg = registry();
    let copy = SceneData::duplicate(&mut world, turret, &reg).expect("copied");
    assert_ne!(copy, turret);
    let copied_barrel = world.query::<&gizmo_core::component::Children>().get(copy).and_then(|c| c.0.first().copied()).expect("the barrel came along");
    assert_ne!(copied_barrel, barrel);
    let s = squad(&world, copy);
    assert_eq!(s.target, Some(player), "the copy aims at the same player");
    assert_eq!(s.leader, copied_barrel, "and its own barrel, not the original's");
    assert_eq!(s.members, vec![copy], "and names itself");
    assert_eq!(squad(&world, turret), Squad { leader: barrel, target: Some(player), members: vec![turret] }, "the original is untouched");
}
