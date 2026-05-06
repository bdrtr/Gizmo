pub mod behavior_tree;
pub mod components;
pub mod navmesh;
pub mod pathfinding;
pub mod steering;
pub mod system;
pub mod goap;
pub mod utility_ai;

pub use behavior_tree::{
    behavior_tree_system, Action, BehaviorTree, BtNode, BtStatus, Condition, Inverter, Selector,
    Sequence,
};
pub use components::{NavAgent, NavAgentState};
pub use pathfinding::NavGrid; // NavGrid::new() ile constructor açık, low-level fns (GridPos, find_path) encapsulate edildi.
pub use navmesh::{NavMesh, NavMeshConfig, NavMeshStats, NavPoly};
pub use goap::{GoapAction, GoapGoal, GoapPlanner, GoapState};
pub use utility_ai::{UtilityBrain, UtilityAction, UtilityConsideration, UtilityCurve, LinearCurve, LogisticCurve, ContextScorer};
pub use steering::{
    alignment, arrive, avoid_obstacles, cohesion, combined_steering, seek, separate,
    SteeringWeights,
};
pub use system::ai_navigation_system;

pub mod prelude {
    pub use super::{
        ai_navigation_system, alignment, arrive, avoid_obstacles, behavior_tree_system, cohesion,
        combined_steering, seek, separate, Action, BehaviorTree, BtNode, BtStatus, Condition,
        Inverter, NavAgent, NavAgentState, NavGrid, NavMesh, NavMeshConfig, Selector, Sequence,
        SteeringWeights, GoapAction, GoapGoal, GoapPlanner, GoapState,
        UtilityBrain, UtilityAction, UtilityConsideration, UtilityCurve, LinearCurve, LogisticCurve, ContextScorer,
    };
}
