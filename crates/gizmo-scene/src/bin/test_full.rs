use gizmo_core::World;
use gizmo_physics::components::RigidBody;
use gizmo_physics::shape::Collider;
use gizmo_scene::registry::SceneRegistry;
use gizmo_scene::scene::SceneData;

fn main() {
    let mut world = World::new();
    let ent = world.spawn();
    
    world.add_component(ent, RigidBody::new(1.0, 0.5, 0.5, true));
    world.add_component(ent, Collider::aabb(gizmo_math::Vec3::new(1.0, 1.0, 1.0)));
    
    let registry = SceneRegistry::default();
    let ent_ids = vec![ent.id()];
    let entities_data = SceneData::serialize_entities(&world, ent_ids, &registry);
    
    let scene = SceneData { entities: entities_data };
    let sz = ron::ser::to_string_pretty(&scene, ron::ser::PrettyConfig::default()).unwrap();
    println!("{}", sz);
}
