use crate::collision::ContactPoint;
use crate::components::{BoxShape, CapsuleShape, ColliderShape, SphereShape};
use gizmo_math::Vec3;

const EPA_TOLERANCE: f32 = 0.001;
const EPA_MAX_ITERATIONS: usize = 32;

/// GJK/EPA simpleksindeki tek bir köşe: Minkowski-fark noktası ve onu üreten
/// her iki şekildeki destek (witness) noktaları. Witness'ler EPA sonunda doğru
/// temas noktasını barycentric olarak geri kurmak için taşınır — aksi halde
/// temas noktası yanlış özelliğe (ör. tekerlek merkezine) düşebiliyordu.
#[derive(Clone, Copy)]
pub(crate) struct SupportPoint {
    /// Minkowski farkı: support_a(d) - support_b(-d)
    v: Vec3,
    /// A şekli üzerindeki destek noktası (witness)
    a: Vec3,
    /// B şekli üzerindeki destek noktası (witness)
    b: Vec3,
}

pub struct Gjk;

impl Gjk {
    /// Test if two shapes are colliding using GJK
    pub fn test_collision(
        shape_a: &ColliderShape,
        pos_a: Vec3,
        rot_a: gizmo_math::Quat,
        shape_b: &ColliderShape,
        pos_b: Vec3,
        rot_b: gizmo_math::Quat,
    ) -> bool {
        let support = |dir: Vec3| {
            let sa = Self::support_point(shape_a, pos_a, rot_a, dir);
            let sb = Self::support_point(shape_b, pos_b, rot_b, -dir);
            SupportPoint { v: sa - sb, a: sa, b: sb }
        };

        Self::gjk_with_simplex(support).is_some()
    }

    /// Get contact information using GJK + EPA
    pub fn get_contact(
        shape_a: &ColliderShape,
        pos_a: Vec3,
        rot_a: gizmo_math::Quat,
        shape_b: &ColliderShape,
        pos_b: Vec3,
        rot_b: gizmo_math::Quat,
    ) -> Option<ContactPoint> {
        let support = |dir: Vec3| {
            let sa = Self::support_point(shape_a, pos_a, rot_a, dir);
            let sb = Self::support_point(shape_b, pos_b, rot_b, -dir);
            SupportPoint { v: sa - sb, a: sa, b: sb }
        };

        if let Some(simplex) = Self::gjk_with_simplex(support) {
            if let Some(contact) = Self::epa(simplex, shape_a, pos_a, rot_a, shape_b, pos_b, rot_b)
            {
                Some(contact)
            } else {
                // EPA failed (likely degenerate simplex), but we KNOW they intersect.
                // Return a basic contact point for triggers and solver fallback.
                tracing::debug!(
                    fallback_penetration = 0.01,
                    "GJK reported an intersection but EPA degenerated; using midpoint fallback contact"
                );
                Some(ContactPoint {
                    point: (pos_a + pos_b) * 0.5,
                    normal: (pos_b - pos_a).try_normalize().unwrap_or(Vec3::Y),
                    penetration: 0.01,
                    ..Default::default()
                })
            }
        } else {
            None
        }
    }

}

// god-file Tier 3 round-2 bölmesi: GJK/EPA metodları alt-modüllerde
// ayrı `impl Gjk` blokları olarak tutulur (hepsi aynı tip üzerinde).
mod epa;
mod simplex;
mod support;

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_math::{Quat, Vec3};

    #[test]
    fn test_sphere_vs_sphere_collision() {
        let shape = ColliderShape::Sphere(SphereShape { radius: 1.0 });

        assert!(Gjk::test_collision(
            &shape,
            Vec3::ZERO,
            Quat::IDENTITY,
            &shape,
            Vec3::new(1.5, 0.0, 0.0),
            Quat::IDENTITY
        ));
        assert!(!Gjk::test_collision(
            &shape,
            Vec3::ZERO,
            Quat::IDENTITY,
            &shape,
            Vec3::new(2.5, 0.0, 0.0),
            Quat::IDENTITY
        ));
    }

    #[test]
    fn test_box_vs_box_collision() {
        let shape = ColliderShape::Box(BoxShape {
            half_extents: Vec3::new(1.0, 1.0, 1.0),
        });

        assert!(Gjk::test_collision(
            &shape,
            Vec3::ZERO,
            Quat::IDENTITY,
            &shape,
            Vec3::new(1.5, 0.0, 0.0),
            Quat::IDENTITY
        ));
        assert!(!Gjk::test_collision(
            &shape,
            Vec3::ZERO,
            Quat::IDENTITY,
            &shape,
            Vec3::new(2.5, 0.0, 0.0),
            Quat::IDENTITY
        ));
    }

    #[test]
    fn test_epa_contact_generation() {
        let shape_a = ColliderShape::Box(BoxShape {
            half_extents: Vec3::new(1.0, 1.0, 1.0),
        });
        let shape_b = ColliderShape::Box(BoxShape {
            half_extents: Vec3::new(1.0, 1.0, 1.0),
        });

        let contact = Gjk::get_contact(
            &shape_a,
            Vec3::ZERO,
            Quat::IDENTITY,
            &shape_b,
            Vec3::new(1.5, 0.0, 0.0),
            Quat::IDENTITY,
        );

        assert!(contact.is_some(), "EPA failed to generate contact");
        let contact = contact.unwrap();

        assert!(
            (contact.penetration - 0.5).abs() < 0.001,
            "Penetration depth is wrong: {}",
            contact.penetration
        );
        assert!(
            (contact.normal.x.abs() - 1.0).abs() < 0.001,
            "Normal is wrong: {:?}",
            contact.normal
        );
    }

    #[test]
    fn test_speculative_contact_approaching() {
        // Two spheres approaching each other — should produce a contact
        let shape = ColliderShape::Sphere(SphereShape { radius: 0.5 });

        let contact = Gjk::speculative_contact(
            &shape,
            Vec3::new(-5.0, 0.0, 0.0),
            Quat::IDENTITY,
            Vec3::new(10.0, 0.0, 0.0),
            1.0,
            &shape,
            Vec3::new(5.0, 0.0, 0.0),
            Quat::IDENTITY,
            Vec3::new(-10.0, 0.0, 0.0),
            1.0,
            1.0,
        );

        assert!(
            contact.is_some(),
            "Speculative contact missed approaching spheres"
        );
    }

    /// The repro that motivated rewriting CA: a swept box with a LATERAL OFFSET.
    ///
    /// CA used to work only for a head-on approach. Two unit boxes with B at x=5 and A
    /// swept along +X reported no hit at all for any offset ≥ 0.05, because `distance()`
    /// over-reports the gap at the exact touching iterate, the step then carried A into
    /// overlap, and the separation certificate rejected the sweep from there.
    ///
    /// The analytic TOI is the same for every offset with |z| < 1.0, because contact is
    /// face-to-face at x = 4.5: (4.5 - 0.5) / 20 = 0.2.
    #[test]
    fn ca_box_sweep_with_lateral_offset_finds_the_toi() {
        let shape = ColliderShape::Box(BoxShape {
            half_extents: Vec3::splat(0.5),
        });
        for z in [0.0f32, 0.05, 0.2, 0.4, 0.6, 0.9] {
            let hit = Gjk::conservative_advancement(
                &shape,
                Vec3::new(0.0, 0.0, z),
                Quat::IDENTITY,
                Vec3::new(20.0, 0.0, 0.0),
                &shape,
                Vec3::new(5.0, 0.0, 0.0),
                Quat::IDENTITY,
                Vec3::ZERO,
                1.0,
            );
            let (toi, normal) = hit.unwrap_or_else(|| {
                panic!("offset z={z} sweeps straight into the box and must report a TOI")
            });
            assert!(
                (toi - 0.2).abs() < 5e-3,
                "z={z}: TOI {toi} should be 0.2 (face contact at x=4.5)"
            );
            assert!(normal.x < -0.99, "z={z}: impact normal should be -x, got {normal:?}");
        }

        // Beyond the combined half-extents there is no z-overlap: a genuine miss.
        let miss = Gjk::conservative_advancement(
            &shape,
            Vec3::new(0.0, 0.0, 1.05),
            Quat::IDENTITY,
            Vec3::new(20.0, 0.0, 0.0),
            &shape,
            Vec3::new(5.0, 0.0, 0.0),
            Quat::IDENTITY,
            Vec3::ZERO,
            1.0,
        );
        assert!(miss.is_none(), "a sweep that passes beside the box must report no hit");
    }

    /// Analytic oracle. Sphere-sphere TOI has a closed form, so this pins the numeric
    /// answer rather than a plausibility band: centres touch at separation r+r = 1, so
    /// with A starting 10 apart in x at 10 u/s and offset z, toi = (10 - sqrt(1 - z²)) / 10.
    #[test]
    fn ca_sphere_sweep_matches_the_closed_form() {
        let shape = ColliderShape::Sphere(SphereShape { radius: 0.5 });
        for z in [0.0f32, 0.3, 0.6, 0.8] {
            let expected = (10.0 - (1.0f32 - z * z).sqrt()) / 10.0;
            let (toi, _) = Gjk::conservative_advancement(
                &shape,
                Vec3::new(-5.0, 0.0, z),
                Quat::IDENTITY,
                Vec3::new(10.0, 0.0, 0.0),
                &shape,
                Vec3::new(5.0, 0.0, 0.0),
                Quat::IDENTITY,
                Vec3::ZERO,
                2.0,
            )
            .unwrap_or_else(|| panic!("z={z} must produce an impact"));
            assert!(
                (toi - expected).abs() < 5e-3,
                "z={z}: TOI {toi} should match the closed form {expected}"
            );
        }
    }

    /// The invariant CA exists to provide, stated directly: the reported TOI is *before*
    /// contact, never past it. A late TOI is how a CCD routine lets a body tunnel.
    #[test]
    fn ca_toi_is_conservative_never_late() {
        let shape = ColliderShape::Box(BoxShape {
            half_extents: Vec3::splat(0.5),
        });
        for z in [0.0f32, 0.3, 0.7] {
            let vel = Vec3::new(20.0, 0.0, 0.0);
            let start = Vec3::new(0.0, 0.0, z);
            let (toi, _) = Gjk::conservative_advancement(
                &shape, start, Quat::IDENTITY, vel, &shape,
                Vec3::new(5.0, 0.0, 0.0), Quat::IDENTITY, Vec3::ZERO, 1.0,
            )
            .expect("impact");

            let just_before = start + vel * (toi - 1e-3).max(0.0);
            assert!(
                !Gjk::test_collision(
                    &shape, just_before, Quat::IDENTITY,
                    &shape, Vec3::new(5.0, 0.0, 0.0), Quat::IDENTITY
                ),
                "z={z}: the shapes must still be apart just before the reported TOI"
            );
        }
    }

    /// A pair already overlapping at t=0 has a TOI of 0 — including when both are
    /// stationary, which the old velocity early-out reported as a miss.
    #[test]
    fn ca_initial_overlap_reports_zero_toi() {
        let shape = ColliderShape::Box(BoxShape {
            half_extents: Vec3::splat(0.5),
        });
        for vel in [Vec3::ZERO, Vec3::new(10.0, 0.0, 0.0)] {
            let (toi, _) = Gjk::conservative_advancement(
                &shape, Vec3::ZERO, Quat::IDENTITY, vel,
                &shape, Vec3::new(0.3, 0.0, 0.0), Quat::IDENTITY, Vec3::ZERO, 1.0,
            )
            .unwrap_or_else(|| panic!("overlapping at t=0 with vel={vel:?} is a hit, not a miss"));
            assert_eq!(toi, 0.0);
        }
    }

    /// A contact landing exactly on the horizon must be reported. The old code incremented
    /// `t` and then rejected `t > max_t`, dropping it.
    #[test]
    fn ca_reports_a_contact_that_lands_exactly_at_max_t() {
        let shape = ColliderShape::Sphere(SphereShape { radius: 0.5 });
        // A at x=-5 closing at 10 u/s touches at t = 0.9 exactly.
        let hit = Gjk::conservative_advancement(
            &shape, Vec3::new(-5.0, 0.0, 0.0), Quat::IDENTITY, Vec3::new(10.0, 0.0, 0.0),
            &shape, Vec3::new(5.0, 0.0, 0.0), Quat::IDENTITY, Vec3::ZERO,
            0.9,
        );
        assert!(hit.is_some(), "a contact exactly at max_t must not be dropped");
    }

    #[test]
    fn test_conservative_advancement_sphere_sphere_toi() {
        // First-ever coverage for the exact-TOI primitive (previously dead + untested).
        // A(r=0.5) at x=-5 moving +10; B(r=0.5) at x=+5 at rest. Surfaces touch when
        // the centres are r+r=1.0 apart, i.e. after A travels 10-1=9 units at 10 u/s
        // ⇒ TOI = 0.9 s. Closed-form so a future decision to WIRE this into the CCD
        // narrowphase rests on a validated primitive, not an assumption.
        let shape = ColliderShape::Sphere(SphereShape { radius: 0.5 });
        let hit = Gjk::conservative_advancement(
            &shape,
            Vec3::new(-5.0, 0.0, 0.0),
            Quat::IDENTITY,
            Vec3::new(10.0, 0.0, 0.0),
            &shape,
            Vec3::new(5.0, 0.0, 0.0),
            Quat::IDENTITY,
            Vec3::ZERO,
            2.0,
        );
        let (toi, normal) = hit.expect("CA must find the sphere-sphere impact within max_t");
        assert!((toi - 0.9).abs() < 0.02, "TOI wrong: {toi} (expected ≈ 0.9)");
        // Was `normal.x.abs() > 0.99`, which the `Vec3::X` fallback `distance()` emits for a
        // near-zero gap satisfies just as well — a vacuous assertion. The true B→A axis is
        // -X, so pin the sign: this is what proves the `reliable_n` carry-back works.
        assert!(normal.x < -0.99, "impact normal must be -x (B→A), got {normal:?}");

        // Separating (moving apart) ⇒ no impact.
        let miss = Gjk::conservative_advancement(
            &shape,
            Vec3::new(-5.0, 0.0, 0.0),
            Quat::IDENTITY,
            Vec3::new(-10.0, 0.0, 0.0),
            &shape,
            Vec3::new(5.0, 0.0, 0.0),
            Quat::IDENTITY,
            Vec3::ZERO,
            2.0,
        );
        assert!(miss.is_none(), "separating spheres must not report an impact");
    }

    #[test]
    fn test_compute_face_normal_follows_winding_not_origin() {
        // Regression (EPA face orientation): the face normal must come from the
        // stored winding order a→b→c (right-hand rule), NOT from a "point away
        // from the origin" heuristic. Here the triangle is wound so the
        // right-hand-rule normal is +Z, yet the whole face sits just BELOW the
        // origin (z < 0) — the situation a shallow/grazing contact creates, with
        // the origin on the OUTER side of the closest face. The old code computed
        // normal·v_a = -0.01 < 0 and flipped the normal to -Z (inward), which is
        // what corrupted shallow contacts (normal pointing the wrong way / wrong
        // faces marked visible during expansion). The winding normal must stay +Z.
        let mk = |x: f32, y: f32, z: f32| SupportPoint {
            v: Vec3::new(x, y, z),
            a: Vec3::ZERO,
            b: Vec3::ZERO,
        };
        let simplex = [mk(0.0, 0.0, -0.01), mk(1.0, 0.0, -0.01), mk(0.0, 1.0, -0.01)];
        let n = Gjk::compute_face_normal(&simplex, 0, 1, 2);
        assert!(
            n.z > 0.9,
            "face normal must follow winding a→b→c (expected ≈ +Z), got {:?}",
            n
        );
        // Reversing the winding must flip the normal (proves it is winding-driven,
        // not origin-driven — both windings sit on the same side of the origin).
        let n_rev = Gjk::compute_face_normal(&simplex, 0, 2, 1);
        assert!(
            n_rev.z < -0.9,
            "reversed winding must flip the normal, got {:?}",
            n_rev
        );
    }

    #[test]
    fn test_epa_shallow_contact_normal_outward() {
        // Behaviour guard for the EPA fix: a very shallow box/box overlap must
        // still yield a positive penetration and a normal along the separating
        // axis (±X), pointing consistently — never inward and never NaN.
        let shape = ColliderShape::Box(BoxShape {
            half_extents: Vec3::new(1.0, 1.0, 1.0),
        });
        // Overlap of only 0.02 along X (origin sits very close to the closest
        // Minkowski face — the regime that tripped the origin heuristic).
        let contact = Gjk::get_contact(
            &shape,
            Vec3::ZERO,
            Quat::IDENTITY,
            &shape,
            Vec3::new(1.98, 0.0, 0.0),
            Quat::IDENTITY,
        )
        .expect("EPA must produce a contact for a shallow overlap");

        assert!(
            contact.penetration > 0.0 && contact.penetration < 0.1,
            "shallow penetration should be small and positive, got {}",
            contact.penetration
        );
        assert!(contact.normal.is_finite(), "normal must be finite");
        assert!(
            (contact.normal.length() - 1.0).abs() < 1e-3,
            "normal must be unit length, got {}",
            contact.normal.length()
        );
        assert!(
            contact.normal.x.abs() > 0.99,
            "separating axis should be X, got normal {:?}",
            contact.normal
        );
    }

    #[test]
    fn test_speculative_contact_separating() {
        // Two spheres moving apart — should NOT produce a contact
        let shape = ColliderShape::Sphere(SphereShape { radius: 0.5 });

        let contact = Gjk::speculative_contact(
            &shape,
            Vec3::new(-5.0, 0.0, 0.0),
            Quat::IDENTITY,
            Vec3::new(-10.0, 0.0, 0.0),
            1.0,
            &shape,
            Vec3::new(5.0, 0.0, 0.0),
            Quat::IDENTITY,
            Vec3::new(10.0, 0.0, 0.0),
            1.0,
            1.0,
        );

        assert!(
            contact.is_none(),
            "Speculative contact incorrectly fired for separating shapes"
        );
    }
}
