fn main() {
    let string_data = std::fs::read_to_string("crates/gizmo-studio/.play_backup.scene").unwrap();
    match gizmo::ron::from_str::<gizmo::scene::SceneData>(&string_data) {
        Ok(_) => println!("Parse OK"),
        Err(e) => println!("Parse Error: {}", e),
    }
}
