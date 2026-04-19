use gizmo_core::World;
use gizmo::editor::EditorState;

pub fn add_dummy_car(world: &mut World) {
    if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
        ed.prefab_load_request = Some(("demo/assets/prefabs/prefab_8.prefab".to_string(), None, gizmo_math::Vec3::ZERO));
        println!("Test: Requested prefab load!");
    }
}
