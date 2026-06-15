//! Property-based tests for the Featherstone ABA forward dynamics (`compute_aba`).
//!
//! Faz 1 — multibody, saf SPATIAL CEBİR (6D); Faz 0'da Fixed/tekil eklemde
//! çocuğun ataletini ebeveyne taşımayan High bug düzeltildi. Mevcut birim
//! testleri tek nokta (q=π/2) sabitliyordu; bu testler ANALİTİK kontratları
//! rastgele parametre uzayında doğrular:
//!   * tek revolute sarkaç — q̈ = −m·g·l·sin(q) / (I_zz + m·l²)  (her açıda)
//!   * tek prismatic eklem — q̈ = gravity·axis  (eğim eksenine yerçekimi izdüşümü)
//!   * rastgele N-link zincir — q̈ daima sonlu (NaN/Inf üretmez)

use gizmo_math::spatial::{SpatialInertia, SpatialMatrix, SpatialVector};
use gizmo_math::{Mat3, Quat, Vec3};
use gizmo_physics_rigid::multibody::aba::compute_aba;
use gizmo_physics_rigid::multibody::{ArticulatedLink, ArticulatedTree, JointType};
use proptest::prelude::*;

/// SpatialInertia rotasyonu Identity (I_zz = 1) olan bir link kur. com = kütle merkezi.
fn link(parent: usize, jt: JointType, t: Vec3, mass: f32, com: Vec3, q: f32) -> ArticulatedLink {
    ArticulatedLink {
        parent_index: parent,
        joint_type: jt,
        transform_to_parent: t,
        rotation_to_parent: Quat::IDENTITY,
        inertia: SpatialInertia::new(mass, Mat3::IDENTITY, com),
        q,
        q_dot: 0.0,
        q_ddot: 0.0,
        joint_force: 0.0,
        v: SpatialVector::ZERO,
        a: SpatialVector::ZERO,
        c: SpatialVector::ZERO,
        i_a: SpatialMatrix::ZERO,
        p_a: SpatialVector::ZERO,
        S: SpatialVector::ZERO,
        u: 0.0,
        d_val: 0.0,
        u_vec: SpatialVector::ZERO,
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// Tek revolute sarkaç (Z ekseni), kütle merkezi pivottan l aşağıda, q̇=0.
    /// Yerçekimi torku = −m·g·l·sin(q); pivot ataleti = I_zz(=1) + m·l².
    /// q̈ = tork / atalet (analitik). Geniş aralıkta sin meaningfully büyük.
    #[test]
    fn single_revolute_pendulum_matches_analytic(
        mass in 0.5f32..5.0,
        l in 0.5f32..3.0,
        g in 1.0f32..20.0,
        q in (std::f32::consts::PI / 6.0)..(5.0 * std::f32::consts::PI / 6.0),
    ) {
        let mut tree = ArticulatedTree::default();
        tree.is_fixed_base = true;
        tree.links.push(link(
            usize::MAX,
            JointType::Revolute(Vec3::Z),
            Vec3::ZERO,
            mass,
            Vec3::new(0.0, -l, 0.0),
            q,
        ));

        compute_aba(&mut tree, Vec3::new(0.0, -g, 0.0));

        let expected = -(mass * g * l * q.sin()) / (1.0 + mass * l * l);
        let got = tree.links[0].q_ddot;
        prop_assert!(got.is_finite(), "q̈ sonlu değil");
        prop_assert!(
            (got - expected).abs() < 0.02 * expected.abs().max(1.0),
            "sarkaç q̈ analitik değerden saptı: beklenen {expected}, gelen {got} (m={mass} l={l} g={g} q={q})"
        );
    }

    /// Tek prismatic eklem (rastgele eksen), q̇=0. Eklem ekseni boyunca ivme,
    /// yerçekiminin o eksene izdüşümüne eşit: q̈ = gravity·axis.
    #[test]
    fn single_prismatic_matches_gravity_projection(
        ax in -1.0f32..1.0, ay in -1.0f32..1.0, az in -1.0f32..1.0,
        gx in -15.0f32..15.0, gy in -15.0f32..15.0, gz in -15.0f32..15.0,
        mass in 0.5f32..5.0,
    ) {
        let axis_raw = Vec3::new(ax, ay, az);
        prop_assume!(axis_raw.length() > 0.3);
        let axis = axis_raw.normalize();
        let gravity = Vec3::new(gx, gy, gz);

        let mut tree = ArticulatedTree::default();
        tree.is_fixed_base = true;
        tree.links.push(link(
            usize::MAX,
            JointType::Prismatic(axis),
            Vec3::ZERO,
            mass,
            Vec3::ZERO,
            0.0,
        ));

        compute_aba(&mut tree, gravity);

        let expected = gravity.dot(axis);
        let got = tree.links[0].q_ddot;
        prop_assert!(got.is_finite(), "q̈ sonlu değil");
        prop_assert!(
            (got - expected).abs() < 1e-2 * expected.abs().max(1.0),
            "prismatic q̈ ≠ yerçekimi izdüşümü: beklenen {expected}, gelen {got}"
        );
    }

    /// Rastgele N-link revolute zincir → tüm eklem ivmeleri sonlu (sağlamlık).
    #[test]
    fn random_revolute_chain_is_finite(
        n in 1usize..6,
        seed in any::<u64>(),
    ) {
        // Deterministik basit LCG ile parametre türet (proptest case'inden bağımsız tekrar).
        let mut s = seed | 1;
        let mut next = || { s = s.wrapping_mul(6364136223846793005).wrapping_add(1); ((s >> 33) as f32 / (1u64 << 31) as f32) - 1.0 };

        let mut tree = ArticulatedTree::default();
        tree.is_fixed_base = true;
        for i in 0..n {
            let parent = if i == 0 { usize::MAX } else { i - 1 };
            let axis = Vec3::new(next(), next(), next());
            let axis = if axis.length() < 0.3 { Vec3::Z } else { axis.normalize() };
            let mass = 0.5 + (next() + 1.0); // ~0.5..2.5
            let q = next() * std::f32::consts::PI;
            tree.links.push(link(
                parent,
                JointType::Revolute(axis),
                Vec3::new(0.0, -1.0, 0.0),
                mass,
                Vec3::new(next() * 0.5, -1.0, next() * 0.5),
                q,
            ));
        }

        compute_aba(&mut tree, Vec3::new(0.0, -9.81, 0.0));

        for (i, lnk) in tree.links.iter().enumerate() {
            prop_assert!(lnk.q_ddot.is_finite(), "link {i} q̈ NaN/Inf");
        }
    }
}
