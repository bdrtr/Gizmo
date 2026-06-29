use gizmo_math::Vec3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
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

    /// Stops the agent: clears the destination AND the current path. Clearing only the
    /// path (see [`clear_path`]) leaves `target` set, so the navigation system just
    /// recomputes the path and keeps moving — use this to actually halt the agent.
    pub fn clear_target(&mut self) {
        self.target = None;
        self.clear_path();
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

    pub fn set_target(&mut self, target: Vec3) {
        self.target = Some(target);
        self.recalc.timer = 0.0; // Zorla yeniden hesaplat
    }
}

impl Default for NavAgent {
    fn default() -> Self {
        Self::new(5.0, 10.0, 0.5)
    }
}

gizmo_core::impl_component!(NavAgent);

#[cfg(test)]
mod tests {
    use super::*;

    // REGRESYON (audit round 2): ai.clear_target() hem hedefi hem path'i temizlemeli.
    // Sadece path temizlenirse target durur, ai_navigation_system yeniden hesaplayıp
    // ajanı yürütmeye devam eder.
    #[test]
    fn clear_target_clears_both_target_and_path() {
        let mut a = NavAgent::default();
        a.set_target(Vec3::new(5.0, 0.0, 0.0));
        a.set_path(vec![Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0)]);
        assert!(a.target.is_some() && a.path_len() > 0);

        a.clear_target();
        assert!(a.target.is_none(), "clear_target hedefi temizlemeli");
        assert_eq!(a.path_len(), 0, "clear_target path'i de temizlemeli");
    }

    #[test]
    fn clear_path_keeps_target() {
        // Ayrımı belgeler: clear_path tek başına ajanı durdurmaz (target kalır).
        let mut a = NavAgent::default();
        a.set_target(Vec3::new(5.0, 0.0, 0.0));
        a.set_path(vec![Vec3::ZERO]);
        a.clear_path();
        assert!(a.target.is_some(), "clear_path target'ı temizlememeli");
        assert_eq!(a.path_len(), 0);
    }
}
