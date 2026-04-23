use gizmo_core::World;
use gizmo_physics::shape::Collider;
use gizmo_scene::registry::SceneRegistry;
use gizmo_scene::scene::SceneData;

fn main() {
    let mut world = World::new();
    let ent = world.spawn();
    world.add_component(
        ent,
        gizmo_core::component::EntityName("Test Car".to_string()),
    );
    world.add_component(ent, Collider::aabb(gizmo_math::Vec3::new(1.0, 1.0, 1.0)));

    let registry = SceneRegistry::default();

    SceneData::save(&world, "test_backup.scene", &registry).unwrap();

    let entities = world.iter_alive_entities();
    for e in entities {
        world.despawn_by_id(e.id());
    }
}
