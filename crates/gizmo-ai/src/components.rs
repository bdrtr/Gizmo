use gizmo_math::Vec3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum NavAgentState {
    Idle,
    Moving,
    Reached,
    Stuck,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NavAgentRecalcState {
    pub timer: f32,
    pub interval: f32,
    pub last_target_pos: Option<Vec3>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NavAgent {
    pub target: Option<Vec3>,
    path: Vec<Vec3>,
    current_path_index: usize, // path.remove(0) yerine indeks takibi — O(1)
    pub state: NavAgentState,
    pub recalc: NavAgentRecalcState,
    pub max_speed: f32,
    pub steering_force: f32,
    pub arrival_radius: f32,
    pub stuck_timer: f32,
    pub last_agent_pos: Option<Vec3>,
}

impl NavAgent {
    pub fn new(max_speed: f32, steering_force: f32, arrival_radius: f32) -> Self {
        Self {
            target: None,
            path: Vec::new(),
            current_path_index: 0,
            state: NavAgentState::Idle,
            recalc: NavAgentRecalcState {
                timer: 0.0,
                interval: 0.5,
                last_target_pos: None,
            },
            max_speed,
            steering_force,
            arrival_radius,
            stuck_timer: 0.0,
            last_agent_pos: None,
        }
    }

    pub fn set_path(&mut self, path: Vec<Vec3>) {
        self.path = path;
        self.current_path_index = 0;
    }

    pub fn clear_path(&mut self) {
        self.path.clear();
        self.current_path_index = 0;
    }

    pub fn current_waypoint(&self) -> Option<&Vec3> {
        self.path.get(self.current_path_index)
    }

    pub fn advance(&mut self) {
        self.current_path_index += 1;
    }

    pub fn is_done(&self) -> bool {
        self.current_path_index >= self.path.len()
    }

    pub fn path_len(&self) -> usize {
        self.path.len()
    }

    pub fn path_index(&self) -> usize {
        self.current_path_index
    }
}

gizmo_core::impl_component!(NavAgent);
