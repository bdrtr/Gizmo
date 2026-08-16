#![deny(clippy::undocumented_unsafe_blocks)]
//! (`undocumented_unsafe_blocks` is a RATCHET: this crate carries no `unsafe` block without a
//! `// SAFETY:` line stating why it is sound, and the lint keeps it that way. Every crate in the
//! workspace except `gizmo-core` is at zero and denies it; `gizmo-core`'s ECS internals are the
//! measured remainder — see docs/ENGINE.md.)
#![warn(missing_docs)]
//! (`missing_docs` is a RATCHET, not a suggestion. The CI lint gate runs with `-D warnings`,
//! so every public item in this crate must carry a doc comment or the build fails. This crate
//! is Stage A — the dependency-light core that goes to 1.x first — and its documented surface
//! is part of that promise. Do not silence this with `#[allow]`; write the doc.)

//! AI subsystems for the Gizmo game engine.
//!
//! This crate provides the decision-making and navigation building blocks used by
//! ECS-driven agents:
//!
//! - [`behavior_tree`]: composable behavior trees (sequence, selector, inverter,
//!   action and condition nodes) plus a system that ticks them.
//! - [`goap`]: goal-oriented action planning (GOAP).
//! - [`utility_ai`]: utility-based decision making with scoring curves.
//! - [`steering`]: steering and flocking forces (seek, arrive, obstacle
//!   avoidance, separation, cohesion, alignment) for boid-like movement.
//! - [`pathfinding`]: A* grid pathfinding.
//! - [`navmesh`]: navigation-mesh construction and querying.
//! - [`components`] and [`system`]: ECS components and systems that wire
//!   navigation into the engine's world.
//!
//! Most of the crate is pure Rust built on [`gizmo_math`] vectors, keeping the
//! API lightweight. The [`prelude`] module re-exports the most commonly used
//! items for convenient glob imports.

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

/// Glob-import target for everyday use of this crate: `use gizmo_ai::prelude::*;`.
///
/// Re-exports the behavior-tree, GOAP, utility-AI, steering and navigation items that agent
/// code normally touches. It is slightly narrower than the crate root: the navmesh detail types
/// [`crate::navmesh::NavMeshStats`] and [`crate::navmesh::NavPoly`] are omitted here but
/// available at the root.
///
/// Four public items are reachable only through their own modules, from neither this prelude nor
/// the crate root: [`crate::system::ai_navmesh_rebuild_system`], [`crate::steering::clamp_force`],
/// [`crate::pathfinding::GridPos`] and [`crate::components::NavAgentRecalcState`] — the last of
/// which is the type of the public [`crate::components::NavAgent::recalc`] field.
pub mod prelude {
    pub use super::{
        ai_navigation_system, alignment, arrive, avoid_obstacles, behavior_tree_system, cohesion,
        combined_steering, seek, separate, Action, BehaviorTree, BtNode, BtStatus, Condition,
        ContextScorer, GoapAction, GoapGoal, GoapPlanner, GoapState, Inverter, LinearCurve,
        LogisticCurve, NavAgent, NavAgentState, NavGrid, NavMesh, NavMeshConfig, Selector,
        Sequence, SteeringWeights, UtilityAction, UtilityBrain, UtilityConsideration, UtilityCurve,
    };
}
