use gizmo_core::World;
use std::fs::File;
use std::io::Read;

fn main() {
    let mut file = File::open("demo/assets/perfect_car.scene").unwrap();
    let mut contents = String::new();
    file.read_to_string(&mut contents).unwrap();
    let registry = gizmo_scene::SceneRegistry::default();
    
    // We can just use gizmo_scene::SceneData::load here, wait, we need to create a test bin target in gizmo-scene or something.
    // Or just write a bin target to gizmo-scene.
}
