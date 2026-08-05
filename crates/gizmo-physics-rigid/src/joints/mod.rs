//! Joints — constraints tying exactly two rigid bodies together.
//!
//! The module is split into a description half ([`data`]) and a runtime half ([`solver`]),
//! both re-exported here: `joints::Joint` and `joints::data::Joint` name the same item.
//!
//! Joints act at the VELOCITY level. A solver pass writes velocities only and never touches
//! a transform, so a joint correction becomes visible one position-integration step later,
//! and every iteration within a pass sees the same positional error.
//!
//! Frame convention: `local_anchor_a`/`local_anchor_b` are in each body's own local frame
//! and are relative to that body's TRANSFORM ORIGIN, not to its centre of mass. The solver
//! converts them itself when it builds a lever arm, so a body whose centre of mass is offset
//! from its origin still takes its anchors in origin-relative coordinates.

/// Joint description data: the [`data::Joint`] record (body pair, local anchors, break
/// thresholds) plus one payload type per joint kind.
///
/// This is the half a scene file stores — these types are `serde`-serialisable, and the
/// runtime-only fields are `#[serde(skip)]`, so a joint that is reloaded comes back
/// unbroken and with no accumulated impulse.
pub mod data;
/// The runtime half: [`solver::JointSolver`], which turns the descriptions in [`data`] into
/// velocity corrections, one substep at a time.
pub mod solver;

pub use data::*;
pub use solver::*;
