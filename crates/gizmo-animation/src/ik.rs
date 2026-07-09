//! Inverse-kinematics solvers for skeletal animation.
//!
//! Two families are provided:
//!
//! * [`solve_two_bone_ik`] — an *analytic* two-bone solver (the classic
//!   upper/lower limb: shoulder→elbow→wrist or hip→knee→ankle). It computes the
//!   joint placement in closed form via the law of cosines, so it is exact and
//!   allocation-free. A pole/hint vector controls the plane the middle joint
//!   bends in (elbow/knee direction).
//! * [`solve_fabrik`] — an iterative FABRIK solver for arbitrary N-bone chains.
//!
//! All solvers operate on positions expressed in a single common space (world
//! or model space — the solver does not care, as long as `root`, `target` and
//! `pole` share it). Converting the resulting bone directions into per-joint
//! local rotations is the caller's responsibility; [`TwoBoneIkResult`] exposes
//! helpers ([`TwoBoneIkResult::upper_dir`], [`TwoBoneIkResult::lower_dir`],
//! [`TwoBoneIkResult::swing_rotations`]) to make that easy.

use gizmo_math::{Quat, Vec3};

const EPS: f32 = 1e-6;

/// Result of a two-bone IK solve, in the same space as the inputs.
///
/// Bone lengths are *always* preserved exactly (`|mid - root| == upper_len` and
/// `|end - mid| == lower_len`), including for out-of-reach targets — in that
/// case the limb is fully extended toward the target and `end` sits at maximum
/// reach rather than on the (unreachable) target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TwoBoneIkResult {
    /// World/model position of the root joint (unchanged from the input).
    pub root: Vec3,
    /// Position of the middle joint (elbow / knee) after solving.
    pub mid: Vec3,
    /// Position of the end effector (wrist / ankle) after solving. Equals the
    /// target when it is reachable.
    pub end: Vec3,
}

impl TwoBoneIkResult {
    /// Normalized direction of the upper bone (`root` → `mid`).
    pub fn upper_dir(&self) -> Vec3 {
        (self.mid - self.root).normalize_or_zero()
    }

    /// Normalized direction of the lower bone (`mid` → `end`).
    pub fn lower_dir(&self) -> Vec3 {
        (self.end - self.mid).normalize_or_zero()
    }

    /// Swing-only world/model-space rotations that carry each bone from its rest
    /// (bind) direction onto the solved direction.
    ///
    /// This is the shortest-arc rotation per bone; twist about the bone axis is
    /// not recovered (that information is not present in a direction-only IK
    /// solve). Feed the bones' rest directions in the *same* space as the solve.
    pub fn swing_rotations(&self, upper_rest_dir: Vec3, lower_rest_dir: Vec3) -> (Quat, Quat) {
        let upper = Quat::from_rotation_arc(upper_rest_dir.normalize_or_zero(), self.upper_dir());
        let lower = Quat::from_rotation_arc(lower_rest_dir.normalize_or_zero(), self.lower_dir());
        (upper, lower)
    }
}

/// Analytic two-bone IK.
///
/// Given the fixed `root` joint, the two bone lengths and a `target`, computes
/// where the middle joint (elbow/knee) and end effector should sit. The `pole`
/// (a.k.a. hint) is a point that, together with the root→target axis, defines
/// the plane the middle joint bends into: the elbow is pushed toward the side of
/// the axis that `pole` lies on.
///
/// The solution preserves both bone lengths exactly. When the target is farther
/// than `upper_len + lower_len` the limb is straightened toward it (`end` at max
/// reach); when it is closer than `|upper_len - lower_len|` the limb folds to
/// that minimum reach.
pub fn solve_two_bone_ik(
    root: Vec3,
    upper_len: f32,
    lower_len: f32,
    target: Vec3,
    pole: Vec3,
) -> TwoBoneIkResult {
    let to_target = target - root;
    let dist = to_target.length();

    // Axis from root toward the target. Fall back to the pole direction, then to
    // +X, if the target coincides with the root (otherwise the axis is undefined).
    let axis_dir = if dist > EPS {
        to_target / dist
    } else {
        let p = pole - root;
        if p.length_squared() > EPS * EPS {
            p.normalize()
        } else {
            Vec3::X
        }
    };

    // Clamp the effective distance to the reachable annulus so the triangle is
    // always valid; this is what guarantees bone-length preservation.
    let min_reach = (upper_len - lower_len).abs();
    let max_reach = upper_len + lower_len;
    let eff_dist = dist.clamp(min_reach, max_reach);

    // Law of cosines: interior angle at the root between the axis and the upper
    // bone. Guard the degenerate denominator (zero-length bone or eff_dist == 0).
    let denom = 2.0 * upper_len * eff_dist;
    let cos_root = if denom > EPS {
        ((upper_len * upper_len + eff_dist * eff_dist - lower_len * lower_len) / denom)
            .clamp(-1.0, 1.0)
    } else {
        1.0
    };
    let sin_root = (1.0 - cos_root * cos_root).max(0.0).sqrt();

    // Bend direction: component of the pole hint perpendicular to the axis.
    let pole_dir = pole - root;
    let proj = pole_dir - axis_dir * pole_dir.dot(axis_dir);
    let bend = if proj.length_squared() > EPS * EPS {
        proj.normalize()
    } else {
        any_perpendicular(axis_dir)
    };

    let mid = root + axis_dir * (upper_len * cos_root) + bend * (upper_len * sin_root);
    let end = root + axis_dir * eff_dist;

    TwoBoneIkResult { root, mid, end }
}

/// Returns some unit vector perpendicular to `v` (used when the pole hint is
/// colinear with the limb axis and therefore gives no bend information).
fn any_perpendicular(v: Vec3) -> Vec3 {
    let c = v.cross(Vec3::X);
    if c.length_squared() > EPS * EPS {
        c.normalize()
    } else {
        v.cross(Vec3::Y).normalize_or_zero()
    }
}

/// Iterative FABRIK (Forward And Backward Reaching Inverse Kinematics) for an
/// N-bone chain.
///
/// `joints` holds the chain positions (`joints[0]` is the fixed root and is kept
/// in place); `bone_lengths[i]` is the rest length between `joints[i]` and
/// `joints[i + 1]`, so `bone_lengths.len()` must equal `joints.len() - 1`. The
/// positions are updated in place. Iteration stops early once the end effector
/// is within `tolerance` of `target`.
///
/// Returns the residual distance from the end effector to the target after
/// solving (`0.0` when reached; the overshoot distance when the target is out of
/// reach — in that case the chain is straightened toward the target). Returns
/// [`f32::INFINITY`] for malformed input (fewer than two joints, or a
/// length/joint count mismatch).
pub fn solve_fabrik(
    joints: &mut [Vec3],
    bone_lengths: &[f32],
    target: Vec3,
    iterations: usize,
    tolerance: f32,
) -> f32 {
    let n = joints.len();
    if n < 2 || bone_lengths.len() != n - 1 {
        return f32::INFINITY;
    }

    let root = joints[0];
    let total: f32 = bone_lengths.iter().sum();

    // Unreachable: lay the chain out straight toward the target.
    if (target - root).length() > total {
        let dir = (target - root).normalize_or_zero();
        for i in 0..n - 1 {
            joints[i + 1] = joints[i] + dir * bone_lengths[i];
        }
        return (joints[n - 1] - target).length();
    }

    for _ in 0..iterations {
        // Backward reaching: pin the end effector to the target, walk to the root.
        joints[n - 1] = target;
        for i in (0..n - 1).rev() {
            let dir = (joints[i] - joints[i + 1]).normalize_or_zero();
            joints[i] = joints[i + 1] + dir * bone_lengths[i];
        }
        // Forward reaching: pin the root back, walk to the end effector.
        joints[0] = root;
        for i in 0..n - 1 {
            let dir = (joints[i + 1] - joints[i]).normalize_or_zero();
            joints[i + 1] = joints[i] + dir * bone_lengths[i];
        }
        if (joints[n - 1] - target).length() < tolerance {
            break;
        }
    }

    (joints[n - 1] - target).length()
}

/// A two-bone IK chain as a component: stores the solve parameters so callers
/// can drive the analytic solver from their own system.
///
/// This is a plain data holder — it does not itself mutate transforms (applying
/// the result requires world-space joint positions from the transform
/// hierarchy). Read `root` from the resolved skeleton, call [`Self::solve`], then
/// write the joint rotations back yourself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TwoBoneIkChain {
    /// Length of the upper bone (root → mid).
    pub upper_len: f32,
    /// Length of the lower bone (mid → end).
    pub lower_len: f32,
    /// Desired end-effector position (in the solve space).
    pub target: Vec3,
    /// Pole/hint controlling the bend plane of the middle joint.
    pub pole: Vec3,
    /// Blend weight in `[0, 1]` for callers that want to fade IK in/out.
    pub weight: f32,
}

impl Default for TwoBoneIkChain {
    fn default() -> Self {
        Self {
            upper_len: 1.0,
            lower_len: 1.0,
            target: Vec3::ZERO,
            pole: Vec3::Y,
            weight: 1.0,
        }
    }
}

impl TwoBoneIkChain {
    /// Solve this chain for the given (fixed) root joint position.
    pub fn solve(&self, root: Vec3) -> TwoBoneIkResult {
        solve_two_bone_ik(root, self.upper_len, self.lower_len, self.target, self.pole)
    }
}

impl gizmo_core::component::Component for TwoBoneIkChain {
    fn storage_type() -> gizmo_core::component::StorageType {
        gizmo_core::component::StorageType::Table
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOL: f32 = 1e-4;

    fn len(a: Vec3, b: Vec3) -> f32 {
        (a - b).length()
    }

    #[test]
    fn two_bone_reaches_target_and_preserves_lengths() {
        let root = Vec3::ZERO;
        let (upper, lower) = (1.0_f32, 1.0_f32);
        // Reachable: distance 1.0 < 2.0 total reach.
        let target = Vec3::new(1.0, 0.0, 0.0);
        let pole = Vec3::new(0.5, 1.0, 0.0);

        let r = solve_two_bone_ik(root, upper, lower, target, pole);

        assert!(len(r.end, target) < TOL, "end effector must land on target, got {:?}", r.end);
        assert!((len(r.root, r.mid) - upper).abs() < TOL, "upper bone length preserved");
        assert!((len(r.mid, r.end) - lower).abs() < TOL, "lower bone length preserved");
        // With a real bend the elbow must leave the root→target axis.
        assert!(r.mid.y > 0.1, "elbow should bend toward the pole side (+Y), got {:?}", r.mid);
    }

    #[test]
    fn two_bone_unreachable_fully_extends() {
        let root = Vec3::ZERO;
        let (upper, lower) = (1.0_f32, 1.0_f32);
        let target = Vec3::new(5.0, 0.0, 0.0); // distance 5 >> reach 2
        let pole = Vec3::new(0.0, 1.0, 0.0);

        let r = solve_two_bone_ik(root, upper, lower, target, pole);

        // Bone lengths still preserved.
        assert!((len(r.root, r.mid) - upper).abs() < TOL);
        assert!((len(r.mid, r.end) - lower).abs() < TOL);
        // Fully extended toward the target: end at max reach along +X, straight.
        assert!(len(r.end, Vec3::new(2.0, 0.0, 0.0)) < TOL, "end at max reach, got {:?}", r.end);
        assert!(len(r.mid, Vec3::new(1.0, 0.0, 0.0)) < TOL, "mid straight, got {:?}", r.mid);
        // End points from root straight at the (unreachable) target.
        let to_target = (target - root).normalize();
        assert!(r.upper_dir().dot(to_target) > 1.0 - TOL, "limb aims at target");
    }

    #[test]
    fn two_bone_pole_controls_elbow_plane() {
        let root = Vec3::ZERO;
        let (upper, lower) = (1.0_f32, 1.0_f32);
        let target = Vec3::new(1.0, 0.0, 0.0);

        let up = solve_two_bone_ik(root, upper, lower, target, Vec3::new(0.5, 1.0, 0.0));
        let down = solve_two_bone_ik(root, upper, lower, target, Vec3::new(0.5, -1.0, 0.0));
        let front = solve_two_bone_ik(root, upper, lower, target, Vec3::new(0.5, 0.0, 1.0));

        // Both reach the target regardless of pole.
        assert!(len(up.end, target) < TOL);
        assert!(len(down.end, target) < TOL);
        assert!(len(front.end, target) < TOL);

        // Elbow follows the pole side.
        assert!(up.mid.y > 0.1, "pole +Y -> elbow +Y, got {:?}", up.mid);
        assert!(down.mid.y < -0.1, "pole -Y -> elbow -Y, got {:?}", down.mid);
        assert!(front.mid.z > 0.1, "pole +Z -> elbow +Z, got {:?}", front.mid);
    }

    #[test]
    fn two_bone_asymmetric_lengths() {
        let root = Vec3::new(1.0, 2.0, 3.0);
        let (upper, lower) = (2.0_f32, 1.0_f32);
        let target = root + Vec3::new(2.0, 0.5, 0.0);
        let pole = root + Vec3::new(1.0, 3.0, 0.0);

        let r = solve_two_bone_ik(root, upper, lower, target, pole);
        assert!(len(r.end, target) < TOL, "reachable asymmetric target hit");
        assert!((len(r.root, r.mid) - upper).abs() < TOL);
        assert!((len(r.mid, r.end) - lower).abs() < TOL);
    }

    #[test]
    fn swing_rotations_align_bones() {
        let root = Vec3::ZERO;
        let target = Vec3::new(1.0, 0.0, 0.0);
        let r = solve_two_bone_ik(root, 1.0, 1.0, target, Vec3::new(0.5, 1.0, 0.0));
        // Rest pose: both bones point along +X.
        let (q_up, q_low) = r.swing_rotations(Vec3::X, Vec3::X);
        let rotated_up = q_up * Vec3::X;
        let rotated_low = q_low * Vec3::X;
        assert!(len(rotated_up, r.upper_dir()) < TOL, "upper swing aligns rest->solved");
        assert!(len(rotated_low, r.lower_dir()) < TOL, "lower swing aligns rest->solved");
    }

    #[test]
    fn fabrik_reaches_reachable_target() {
        // 4 joints (3 bones of length 1) along +X.
        let mut joints = vec![
            Vec3::ZERO,
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(3.0, 0.0, 0.0),
        ];
        let lengths = [1.0_f32, 1.0, 1.0];
        let target = Vec3::new(1.0, 1.5, 0.0); // within reach (3.0)

        let residual = solve_fabrik(&mut joints, &lengths, target, 32, 1e-5);

        assert!(residual < 1e-3, "end effector should reach target, residual {residual}");
        assert!(len(joints[3], target) < 1e-3);
        assert!(len(joints[0], Vec3::ZERO) < TOL, "root stays fixed");
        // Bone lengths preserved by construction.
        for i in 0..3 {
            assert!((len(joints[i], joints[i + 1]) - 1.0).abs() < 1e-3, "bone {i} length preserved");
        }
    }

    #[test]
    fn fabrik_unreachable_straightens() {
        let mut joints = vec![Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), Vec3::new(2.0, 0.0, 0.0)];
        let lengths = [1.0_f32, 1.0];
        let target = Vec3::new(0.0, 10.0, 0.0); // distance 10 >> reach 2

        let residual = solve_fabrik(&mut joints, &lengths, target, 16, 1e-5);

        // Cannot reach; residual is the overshoot.
        assert!((residual - 8.0).abs() < TOL, "residual = 10 - 2 = 8, got {residual}");
        // Chain straight toward +Y.
        assert!(len(joints[1], Vec3::new(0.0, 1.0, 0.0)) < TOL);
        assert!(len(joints[2], Vec3::new(0.0, 2.0, 0.0)) < TOL);
    }

    #[test]
    fn fabrik_rejects_malformed_input() {
        let mut one = vec![Vec3::ZERO];
        assert_eq!(solve_fabrik(&mut one, &[], Vec3::X, 4, 1e-4), f32::INFINITY);
        let mut two = vec![Vec3::ZERO, Vec3::X];
        // Wrong number of bone lengths.
        assert_eq!(solve_fabrik(&mut two, &[1.0, 1.0], Vec3::X, 4, 1e-4), f32::INFINITY);
    }
}
