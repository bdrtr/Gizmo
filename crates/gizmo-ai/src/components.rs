use gizmo_math::Vec3;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NavAgent {
    pub target: Option<Vec3>,
    pub path: Vec<Vec3>,
    pub current_path_index: usize, // path.remove(0) yerine indeks takibi — O(1)
    pub max_speed: f32,
    pub max_force: f32,
    pub reach_radius: f32,
    pub path_recalc_timer: f32,
    pub last_target_pos: Option<Vec3>,
}

impl NavAgent {
    pub fn new(max_speed: f32, max_force: f32, reach_radius: f32) -> Self {
        Self {
            target: None,
            path: Vec::new(),
            current_path_index: 0,
            max_speed,
            max_force,
            reach_radius,
            path_recalc_timer: 0.0,
            last_target_pos: None,
        }
    }
}
