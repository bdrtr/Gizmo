//! Per-joint-kind constraint solvers for `JointSolver`, split out of the former
//! monolithic `joint_types.rs` (one file per joint type). Each submodule adds an
//! `impl JointSolver` block; the `pub(crate) solve_*` methods are called from
//! `solve_joints` in the parent `solver` module.

mod ball_socket;
mod d6;
mod distance;
mod fixed;
mod hinge;
mod slider;
mod spring;
