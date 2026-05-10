pub mod behavior_tree;
pub mod components;
pub mod goap;
pub mod navmesh;
pub mod pathfinding;
pub mod steering;
pub mod system;
pub mod utility_ai;

pub use behavior_tree::{
    behavior_tree_system, Action, BehaviorTree, BtNode, BtStatus, Condition, Inverter, Selector,
    Sequence,
};
pub use components::{NavAgent, NavAgentState};
pub use goap::{GoapAction, GoapGoal, GoapPlanner, GoapState};
pub use navmesh::{NavMesh, NavMeshConfig, NavMeshStats, NavPoly};
pub use pathfinding::NavGrid; // NavGrid::new() ile constructor açık, low-level fns (GridPos, find_path) encapsulate edildi.
pub use steering::{
    alignment, arrive, avoid_obstacles, cohesion, combined_steering, seek, separate,
    SteeringWeights,
};
pub use system::ai_navigation_system;
pub use utility_ai::{
    ContextScorer, LinearCurve, LogisticCurve, UtilityAction, UtilityBrain, UtilityConsideration,
    UtilityCurve,
};

pub mod prelude {
    pub use super::{
        ai_navigation_system, alignment, arrive, avoid_obstacles, behavior_tree_system, cohesion,
        combined_steering, seek, separate, Action, BehaviorTree, BtNode, BtStatus, Condition,
        ContextScorer, GoapAction, GoapGoal, GoapPlanner, GoapState, Inverter, LinearCurve,
        LogisticCurve, NavAgent, NavAgentState, NavGrid, NavMesh, NavMeshConfig, Selector,
        Sequence, SteeringWeights, UtilityAction, UtilityBrain, UtilityConsideration, UtilityCurve,
    };
}
