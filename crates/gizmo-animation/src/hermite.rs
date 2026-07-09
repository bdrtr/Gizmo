//! Cubic-Hermite spline evaluation — the single keyframe-sampling primitive
//! shared by **both** of this crate's animation subsystems:
//!
//! * the name-based transform-track player ([`crate::clip`] / [`crate::player`]
//!   / [`crate::system`]), which animates entity `Transform`s, and
//! * the GPU-skinning skeletal sampler ([`crate::skeletal::sample`]), which
//!   produces per-bone TRS for the renderer.
//!
//! Those two systems are intentionally separate (see [`crate::skeletal`]), but
//! they must interpolate keyframes *identically*. Keeping one implementation
//! here is what stops them from silently diverging — the skeletal path used to
//! carry its own copy and fell behind on scale/cubic support.
//!
//! Follows the glTF Appendix C convention: `m0` / `m1` are the in/out tangents
//! **already scaled by the segment duration** (`tangent_per_second * dt`). Each
//! caller applies that scaling before calling in, because the two subsystems
//! store their segment duration differently.

use gizmo_math::{Quat, Vec3, Vec4};

/// Scalar cubic-Hermite basis applied to a [`Vec4`] (also reused for [`Vec3`]
/// via a zero `w`). `m0` / `m1` are the already-scaled in/out tangents.
pub(crate) fn hermite_vec4(p0: Vec4, m0: Vec4, p1: Vec4, m1: Vec4, t: f32) -> Vec4 {
    let t2 = t * t;
    let t3 = t2 * t;
    let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
    let h10 = t3 - 2.0 * t2 + t;
    let h01 = -2.0 * t3 + 3.0 * t2;
    let h11 = t3 - t2;
    p0 * h00 + m0 * h10 + p1 * h01 + m1 * h11
}

/// Cubic-Hermite interpolation for a [`Vec3`]. `m0` / `m1` are already-scaled
/// in/out tangents.
pub(crate) fn hermite_vec3(p0: Vec3, m0: Vec3, p1: Vec3, m1: Vec3, t: f32) -> Vec3 {
    let out = hermite_vec4(
        Vec4::new(p0.x, p0.y, p0.z, 0.0),
        Vec4::new(m0.x, m0.y, m0.z, 0.0),
        Vec4::new(p1.x, p1.y, p1.z, 0.0),
        Vec4::new(m1.x, m1.y, m1.z, 0.0),
        t,
    );
    Vec3::new(out.x, out.y, out.z)
}

/// Component-wise cubic-Hermite interpolation for a quaternion, following the
/// glTF convention (interpolate the four components independently, then
/// re-normalize). `m0` / `m1` are already-scaled in/out tangents.
pub(crate) fn hermite_quat(p0: Quat, m0: Quat, p1: Quat, m1: Quat, t: f32) -> Quat {
    let v0 = Vec4::new(p0.x, p0.y, p0.z, p0.w);
    let t0 = Vec4::new(m0.x, m0.y, m0.z, m0.w);
    let v1 = Vec4::new(p1.x, p1.y, p1.z, p1.w);
    let t1 = Vec4::new(m1.x, m1.y, m1.z, m1.w);
    let out = hermite_vec4(v0, t0, v1, t1, t);
    Quat::from_xyzw(out.x, out.y, out.z, out.w).normalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOL: f32 = 1e-5;

    #[test]
    fn hermite_vec3_flat_tangents_is_smooth_ease() {
        // Zero tangents → smoothstep from p0 to p1. From 0→10 at t=0.25 the
        // Hermite basis gives h01(0.25)*10 = 1.5625 (the value both subsystems'
        // cubic tests pin against).
        let v = hermite_vec3(Vec3::ZERO, Vec3::ZERO, Vec3::splat(10.0), Vec3::ZERO, 0.25);
        assert!((v.x - 1.5625).abs() < TOL, "got {}", v.x);
        assert!((v.y - 1.5625).abs() < TOL && (v.z - 1.5625).abs() < TOL);
    }

    #[test]
    fn hermite_vec3_endpoints_are_exact() {
        let (p0, p1) = (Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0));
        let m = Vec3::new(7.0, 8.0, 9.0); // tangents must not perturb the endpoints
        assert!((hermite_vec3(p0, m, p1, m, 0.0) - p0).length() < TOL);
        assert!((hermite_vec3(p0, m, p1, m, 1.0) - p1).length() < TOL);
    }

    #[test]
    fn hermite_vec3_matches_hand_computed_tangent() {
        // p0=1, m0=4 (already scaled), p1=2, m1=0, t=0.5:
        // h00=0.5, h10=0.125, h01=0.5, h11=-0.125 → 0.5*1 + 0.125*4 + 0.5*2 = 2.0
        let v = hermite_vec3(
            Vec3::splat(1.0),
            Vec3::splat(4.0),
            Vec3::splat(2.0),
            Vec3::ZERO,
            0.5,
        );
        assert!((v.x - 2.0).abs() < TOL, "got {}", v.x);
    }

    #[test]
    fn hermite_quat_result_is_normalized() {
        let a = Quat::from_rotation_y(0.3);
        let b = Quat::from_rotation_y(1.2);
        let q = hermite_quat(a, Quat::from_xyzw(0.1, 0.0, 0.0, 0.0), b, Quat::default(), 0.5);
        assert!((q.length() - 1.0).abs() < 1e-4, "hermite quat must be unit, got {}", q.length());
    }
}
