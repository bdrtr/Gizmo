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
//! `pole` share it). [`TwoBoneIkResult`] exposes helpers
//! ([`TwoBoneIkResult::upper_dir`], [`TwoBoneIkResult::lower_dir`],
//! [`TwoBoneIkResult::swing_rotations`]) for a caller that poses its own bones.
//!
//! On a skeleton the engine does it: an [`IkChains`] component lists [`TwoBoneIkChain`]s by tip
//! bone, and [`apply_ik_chains`] — run every frame by `gizmo`'s frame preparation — turns each
//! solve into joint rotations on the drawn pose.

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

/// One two-bone limb the engine bends every frame: the bone at its end reaches for [`target`],
/// and the joint between bends toward [`pole`].
///
/// A chain is named by its **tip** — the wrist, the ankle — as a [`SkeletonJoint::name`]; the
/// middle joint is the tip's parent and the root joint the middle one's parent, so a chain is three
/// bones in a row and the two lengths are the skeleton's own (measured on the pose being bent, so a
/// clip that stretches a limb is bent at the length it has). Lives inside [`IkChains`], on the
/// entity that carries the skeleton.
///
/// [`target`]: Self::target
/// [`pole`]: Self::pole
/// [`SkeletonJoint::name`]: crate::skeletal::SkeletonJoint::name
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct TwoBoneIkChain {
    /// The name of the bone that reaches — the end of the limb. The first joint of that name is
    /// used; a name the skeleton does not have, or a bone without two ancestors, bends nothing
    /// (and is said once, see `gizmo`'s IK pass).
    pub tip_bone: String,
    /// Where the tip reaches for, a **world-space point**. Out of reach, the limb straightens
    /// toward it; too close, it folds as far as its two lengths allow.
    pub target: Vec3,
    /// A **world-space point** the middle joint bends toward — in front of a knee, behind an elbow.
    /// A point, not a direction (see [`solve_two_bone_ik`]).
    pub pole: Vec3,
    /// How much of the bend is applied, `0` (the pose as animated) to `1` (the tip on the target);
    /// clamped into that range. Each frame starts again from the animated pose, so a weight below
    /// one is that fraction of the bend, every frame — it does not creep toward the target.
    pub weight: f32,
}

impl Default for TwoBoneIkChain {
    fn default() -> Self {
        Self { tip_bone: String::new(), target: Vec3::ZERO, pole: Vec3::new(0.0, 0.0, 1.0), weight: 1.0 }
    }
}

/// The IK chains of one skeleton, **applied by the engine**: `gizmo`'s frame preparation solves
/// them after the skeletal animation drivers and before bone attachments, on top of the pose this
/// frame's animation produced (the bind pose when nothing animates the skeleton), in list order —
/// a chain sees the bends of the chains before it.
///
/// Put it on the entity that carries the skeleton (a glTF model's root). The animated pose itself
/// is left as it is; only the drawn pose, and what bone attachments follow, are bent.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct IkChains {
    /// The chains, solved in this order.
    pub chains: Vec<TwoBoneIkChain>,
}

impl gizmo_core::component::Component for IkChains {
    fn storage_type() -> gizmo_core::component::StorageType {
        gizmo_core::component::StorageType::Table
    }
}

/// The three joints of a chain whose tip is called `tip_bone`: `(root, mid, tip)`, or `None` when
/// the skeleton has no such bone or it has fewer than two ancestors.
pub fn resolve_chain(hierarchy: &crate::skeletal::SkeletonHierarchy, tip_bone: &str) -> Option<(usize, usize, usize)> {
    let n = hierarchy.joints.len();
    let tip = hierarchy.joints.iter().position(|j| j.name == tip_bone)?;
    let mid = hierarchy.joints[tip].parent_index.filter(|&p| p < n)?;
    let root = hierarchy.joints[mid].parent_index.filter(|&p| p < n)?;
    (root != tip && root != mid).then_some((root, mid, tip))
}

/// What [`apply_ik_chains`] made of a skeleton's chains.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IkOutcome {
    /// The bent pose in model space, one matrix per joint — what skinning and bone attachments read.
    pub global_poses: Vec<gizmo_math::Mat4>,
    /// The chains (by index into the list) that bent nothing because their tip bone was not found.
    pub unresolved: Vec<usize>,
}

/// Bends `local_poses` by `chains` and returns the resulting **model-space** pose.
///
/// `model_from_world` maps the chains' world-space targets and poles into the skeleton's model
/// space — the inverse of the skinned entity's world matrix. `local_poses` is not changed: the bend
/// is computed from it afresh on every call, which is what keeps a weight from compounding frame
/// after frame. Each chain rotates its root and middle joints only (swing, about the joint's own
/// position, so bone lengths and the tip's orientation relative to its parent are kept); what hangs
/// below the tip — fingers, a held sword — follows it.
pub fn apply_ik_chains(
    hierarchy: &crate::skeletal::SkeletonHierarchy,
    local_poses: &[gizmo_math::Mat4],
    model_from_world: gizmo_math::Mat4,
    chains: &[TwoBoneIkChain],
) -> IkOutcome {
    use gizmo_math::Mat4;
    let mut locals = local_poses.to_vec();
    let mut globals = hierarchy.calculate_global_matrices(&locals);
    let mut unresolved = Vec::new();
    // The rotation `q` about the point `pivot`, applied to a model-space matrix.
    let about = |pivot: Vec3, q: Quat| Mat4::from_translation(pivot) * Mat4::from_quat(q) * Mat4::from_translation(-pivot);
    let parent_global = |globals: &[Mat4], joint: usize| -> Mat4 {
        match hierarchy.joints[joint].parent_index.filter(|&p| p < globals.len()) {
            Some(p) => globals[p],
            None => hierarchy.root_transform,
        }
    };
    for (index, chain) in chains.iter().enumerate() {
        let Some((root, mid, tip)) = resolve_chain(hierarchy, &chain.tip_bone) else {
            unresolved.push(index);
            continue;
        };
        let weight = if chain.weight.is_finite() { chain.weight.clamp(0.0, 1.0) } else { 0.0 };
        if weight <= 0.0 {
            continue;
        }
        let target = model_from_world.transform_point3(chain.target);
        let pole = model_from_world.transform_point3(chain.pole);
        let (p_root, p_mid, p_tip) =
            (globals[root].w_axis.truncate(), globals[mid].w_axis.truncate(), globals[tip].w_axis.truncate());
        let solved = solve_two_bone_ik(p_root, (p_mid - p_root).length(), (p_tip - p_mid).length(), target, pole);

        // The upper bone swung from where it points onto where the solve put it.
        let swing = |from: Vec3, to: Vec3| -> Quat {
            match (from.try_normalize(), to.try_normalize()) {
                (Some(a), Some(b)) => Quat::IDENTITY.slerp(Quat::from_rotation_arc(a, b), weight),
                _ => Quat::IDENTITY,
            }
        };
        let root_global = about(p_root, swing(p_mid - p_root, solved.mid - p_root)) * globals[root];
        locals[root] = parent_global(&globals, root).inverse() * root_global;
        // The middle joint where that leaves it, then the lower bone swung onto the solve's.
        let mid_global = root_global * locals[mid];
        let tip_global = mid_global * locals[tip];
        let p_mid = mid_global.w_axis.truncate();
        let mid_global = about(p_mid, swing(tip_global.w_axis.truncate() - p_mid, solved.end - p_mid)) * mid_global;
        locals[mid] = root_global.inverse() * mid_global;
        globals = hierarchy.calculate_global_matrices(&locals);
    }
    IkOutcome { global_poses: globals, unresolved }
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

    // ── Analytic solver degenerate / boundary cases ────────────────────

    #[test]
    fn two_bone_target_at_root_stays_finite_and_preserves_lengths() {
        // dist ~ 0: the root→target axis is undefined and must fall back to the
        // pole direction rather than producing NaNs.
        let root = Vec3::new(2.0, 2.0, 2.0);
        let r = solve_two_bone_ik(root, 1.0, 1.0, root, root + Vec3::Y);
        assert!(r.mid.is_finite() && r.end.is_finite(), "no NaN, got {r:?}");
        assert!((len(r.root, r.mid) - 1.0).abs() < TOL, "upper length preserved");
        assert!((len(r.mid, r.end) - 1.0).abs() < TOL, "lower length preserved");
    }

    #[test]
    fn two_bone_folds_to_min_reach_when_target_too_close() {
        // Asymmetric bones: minimum reach is |2 - 1| = 1. A target only 0.3 away
        // must clamp the effective distance up to 1.0 (limb folded), lengths kept.
        let root = Vec3::ZERO;
        let (upper, lower) = (2.0_f32, 1.0_f32);
        let target = Vec3::new(0.3, 0.0, 0.0);
        let r = solve_two_bone_ik(root, upper, lower, target, Vec3::new(0.0, 1.0, 0.0));
        assert!((len(r.root, r.mid) - upper).abs() < TOL, "upper length preserved");
        assert!((len(r.mid, r.end) - lower).abs() < TOL, "lower length preserved");
        // End sits at the minimum reach along the axis, not on the (too-close) target.
        assert!((len(r.root, r.end) - 1.0).abs() < TOL, "end at min reach, got {:?}", r.end);
    }

    #[test]
    fn two_bone_zero_length_upper_bone_is_finite() {
        // A degenerate zero-length upper bone drives the law-of-cosines denominator
        // to zero; the guard must keep the solve finite and lengths exact.
        let root = Vec3::ZERO;
        let r = solve_two_bone_ik(root, 0.0, 1.0, Vec3::new(0.5, 0.0, 0.0), Vec3::Y);
        assert!(r.mid.is_finite() && r.end.is_finite(), "no NaN, got {r:?}");
        assert!(len(r.root, r.mid) < TOL, "zero-length upper keeps mid at root");
        assert!((len(r.mid, r.end) - 1.0).abs() < TOL, "lower length preserved");
    }

    #[test]
    fn fabrik_reaches_target_at_exact_full_reach() {
        // Target exactly at total reach (not strictly greater) exercises the
        // iterative path (the `> total` shortcut is skipped) and must still converge.
        let mut joints = vec![Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), Vec3::new(2.0, 0.0, 0.0)];
        let lengths = [1.0_f32, 1.0];
        let target = Vec3::new(2.0, 0.0, 0.0); // distance 2 == total reach 2
        let residual = solve_fabrik(&mut joints, &lengths, target, 16, 1e-5);
        assert!(residual < 1e-3, "must reach a just-reachable target, residual {residual}");
        assert!(len(joints[1], Vec3::new(1.0, 0.0, 0.0)) < 1e-3, "chain straight, got {:?}", joints[1]);
        assert!(len(joints[2], target) < 1e-3);
    }

    // ── Chains applied to a skeleton ───────────────────────────────────

    /// root at the origin → mid one up → tip one further up → a finger half a unit above the tip.
    fn arm() -> (crate::skeletal::SkeletonHierarchy, Vec<gizmo_math::Mat4>) {
        use crate::skeletal::{SkeletonHierarchy, SkeletonJoint};
        use gizmo_math::Mat4;
        let joint = |name: &str, parent: Option<usize>, up: f32| SkeletonJoint {
            name: name.into(),
            node_index: 0,
            inverse_bind_matrix: Mat4::IDENTITY,
            parent_index: parent,
            local_bind_transform: Mat4::from_translation(Vec3::Y * up),
            bind_translation: Vec3::Y * up,
            bind_rotation: Quat::IDENTITY,
            bind_scale: Vec3::ONE,
        };
        let hierarchy = SkeletonHierarchy {
            joints: vec![joint("Shoulder", None, 0.0), joint("Elbow", Some(0), 1.0), joint("Hand", Some(1), 1.0), joint("Finger", Some(2), 0.5)],
            root_transform: Mat4::IDENTITY,
        };
        let locals = hierarchy.joints.iter().map(|j| j.local_bind_transform).collect();
        (hierarchy, locals)
    }

    fn at(m: &gizmo_math::Mat4) -> Vec3 {
        m.w_axis.truncate()
    }

    #[test]
    fn a_chain_puts_its_tip_on_the_target_and_keeps_the_rig() {
        let (h, locals) = arm();
        let chain = TwoBoneIkChain { tip_bone: "Hand".into(), target: Vec3::new(1.2, 0.8, 0.0), pole: Vec3::new(0.0, 1.0, 1.0), weight: 1.0 };
        let out = apply_ik_chains(&h, &locals, gizmo_math::Mat4::IDENTITY, std::slice::from_ref(&chain));
        let g = &out.global_poses;
        assert!(out.unresolved.is_empty());
        assert!(len(at(&g[2]), chain.target) < TOL, "the hand reaches the target, it is at {:?}", at(&g[2]));
        assert!(len(at(&g[0]), Vec3::ZERO) < TOL, "the shoulder stays where it is");
        assert!((len(at(&g[0]), at(&g[1])) - 1.0).abs() < TOL, "upper arm length kept");
        assert!((len(at(&g[1]), at(&g[2])) - 1.0).abs() < TOL, "forearm length kept");
        assert!((len(at(&g[2]), at(&g[3])) - 0.5).abs() < TOL, "the finger hangs where it did off the hand");
        assert!(at(&g[1]).z > 0.1, "the elbow bends toward the pole (+Z), it is at {:?}", at(&g[1]));
    }

    #[test]
    fn a_weight_is_that_fraction_of_the_bend_and_zero_is_the_pose_untouched() {
        let (h, locals) = arm();
        let plain = h.calculate_global_matrices(&locals);
        let mut chain = TwoBoneIkChain { tip_bone: "Hand".into(), target: Vec3::new(1.5, 0.5, 0.0), pole: Vec3::new(0.0, 1.0, 1.0), weight: 0.0 };
        let none = apply_ik_chains(&h, &locals, gizmo_math::Mat4::IDENTITY, &[chain.clone()]);
        assert_eq!(none.global_poses, plain, "weight 0 bends nothing");
        chain.weight = 1.0;
        let full = apply_ik_chains(&h, &locals, gizmo_math::Mat4::IDENTITY, &[chain.clone()]);
        chain.weight = 0.5;
        let half = apply_ik_chains(&h, &locals, gizmo_math::Mat4::IDENTITY, &[chain.clone()]);
        let (rest, bent, between) = (at(&plain[2]), at(&full.global_poses[2]), at(&half.global_poses[2]));
        assert!(len(between, rest) > 0.2 && len(between, bent) > 0.2, "half way is neither end: {between:?}");
        // The same input gives the same pose: nothing is carried from one call to the next.
        let again = apply_ik_chains(&h, &locals, gizmo_math::Mat4::IDENTITY, &[chain]);
        assert_eq!(again, half);
    }

    #[test]
    fn targets_are_world_points_mapped_into_the_model() {
        let (h, locals) = arm();
        let world_from_model = gizmo_math::Mat4::from_rotation_translation(Quat::from_rotation_y(0.7), Vec3::new(5.0, 2.0, -1.0));
        let target_model = Vec3::new(1.0, 1.0, 0.0);
        let chain = TwoBoneIkChain {
            tip_bone: "Hand".into(),
            target: world_from_model.transform_point3(target_model),
            pole: world_from_model.transform_point3(Vec3::new(0.0, 1.0, 1.0)),
            weight: 1.0,
        };
        let out = apply_ik_chains(&h, &locals, world_from_model.inverse(), &[chain]);
        assert!(len(at(&out.global_poses[2]), target_model) < TOL, "hand at {:?}", at(&out.global_poses[2]));
    }

    #[test]
    fn a_bone_the_skeleton_has_not_got_bends_nothing_and_is_named() {
        let (h, locals) = arm();
        let plain = h.calculate_global_matrices(&locals);
        let chains = [
            TwoBoneIkChain { tip_bone: "Hnad".into(), target: Vec3::X, ..Default::default() },
            // The elbow's grandparent does not exist: not a chain either.
            TwoBoneIkChain { tip_bone: "Elbow".into(), target: Vec3::X, ..Default::default() },
        ];
        let out = apply_ik_chains(&h, &locals, gizmo_math::Mat4::IDENTITY, &chains);
        assert_eq!(out.unresolved, vec![0, 1]);
        assert_eq!(out.global_poses, plain);
    }

    #[test]
    fn chains_bend_in_order_each_on_the_last() {
        let (h, locals) = arm();
        // The finger's chain (elbow → hand → finger) after the hand's: the hand's bend moves the
        // elbow the finger's chain starts from, and the finger still lands on its own target.
        let hand = TwoBoneIkChain { tip_bone: "Hand".into(), target: Vec3::new(1.2, 0.8, 0.0), pole: Vec3::new(0.0, 1.0, 1.0), weight: 1.0 };
        let elbow = apply_ik_chains(&h, &locals, gizmo_math::Mat4::IDENTITY, std::slice::from_ref(&hand)).global_poses[1];
        let finger_target = at(&elbow) + Vec3::new(0.0, 1.2, 0.0);
        let finger = TwoBoneIkChain { tip_bone: "Finger".into(), target: finger_target, pole: Vec3::new(2.0, 2.0, 0.0), weight: 1.0 };
        let out = apply_ik_chains(&h, &locals, gizmo_math::Mat4::IDENTITY, &[hand, finger]);
        assert!(len(at(&out.global_poses[1]), at(&elbow)) < TOL, "the second chain's root stays where the first put it");
        assert!(len(at(&out.global_poses[3]), finger_target) < TOL, "finger at {:?}", at(&out.global_poses[3]));
    }
}
