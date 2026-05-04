pub mod behavior_tree;
pub mod components;
pub mod pathfinding;
pub mod steering;
pub mod system;

pub use behavior_tree::{
    behavior_tree_system, Action, BehaviorTree, BtNode, BtStatus, Condition, Inverter, Selector,
    Sequence,
};
pub use components::{NavAgent, NavAgentState};
pub use pathfinding::NavGrid; // NavGrid::new() ile constructor açık, low-level fns (GridPos, find_path) encapsulate edildi.
pub use steering::{
    alignment, arrive, avoid_obstacles, cohesion, combined_steering, seek, separate,
    SteeringWeights,
};
pub use system::ai_navigation_system;

pub mod prelude {
    pub use super::{
        ai_navigation_system, alignment, arrive, avoid_obstacles, behavior_tree_system, cohesion,
        combined_steering, seek, separate, Action, BehaviorTree, BtNode, BtStatus, Condition,
        Inverter, NavAgent, NavAgentState, NavGrid, Selector, Sequence, SteeringWeights,
    };
}
