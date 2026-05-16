fn main() {
    let play_backup_path = "crates/gizmo-studio/.play_backup.scene";
    if std::path::Path::new(play_backup_path).exists() {
        let string_data = match std::fs::read_to_string(play_backup_path) {
            Ok(content) => content,
            Err(_) => return,
        };
        let scene: Result<gizmo::scene::SceneData, _> = gizmo::ron::from_str(&string_data);
        match scene {
            Ok(_) => println!("OK!"),
            Err(e) => println!("ERROR: {}", e),
        }
    }
}
