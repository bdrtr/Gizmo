use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────────────────────────────────────
// Kombine Fonksiyon: iki malzemenin değerlerini nasıl birleştireceğimizi belirler
// ─────────────────────────────────────────────────────────────────────────────

/// How the two materials meeting in a contact are merged into one coefficient.
///
/// Each material carries its own mode (one for friction, one for restitution), so every
/// contact has two candidate modes to reconcile. They are resolved by a fixed priority —
/// `Max` beats `Min` beats `GeometricMean` beats `Average` — and the winner is then applied
/// to both materials' values; see [`PhysicsMaterial::combine`]. Priority is a property of
/// the *mode*, not of the values, so a material that asks for `Max` imposes `Max` on every
/// partner it touches.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[non_exhaustive]
pub enum CombineMode {
    /// Arithmetic mean, `(a + b) / 2`. Lowest priority, so it is the pair's mode only when
    /// *both* materials ask for it — in effect the "no opinion" choice.
    Average, // (a + b) / 2
    /// Takes the lower of the two coefficients, so one slippery (or dead) surface is enough
    /// to make the whole contact slippery. Outranked only by `Max`.
    Min,     // a.min(b)
    /// Takes the higher of the two coefficients. Highest priority, so this is the mode for a
    /// material that must keep its character regardless of what it lands on — an
    /// [`ICE`](PhysicsMaterial::ICE) floor does not make [`RUBBER`](PhysicsMaterial::RUBBER)
    /// slide.
    Max,     // a.max(b)
    /// Square root of the product — the usual convention for Coulomb friction coefficients.
    /// For non-negative inputs the result sits between the two operands and never exceeds
    /// their arithmetic mean, and it collapses to zero as soon as either side is zero.
    #[default]
    GeometricMean, // sqrt(a * b)  — geometric mean (sürtünme için ideal)
}

impl CombineMode {
    /// Merge two coefficients under this mode.
    ///
    /// Meant for non-negative inputs (friction, restitution). `GeometricMean` clamps the
    /// product at zero before taking the root, so a single negative operand returns `0.0`
    /// instead of `NaN` — but two negatives multiply to a positive and come back positive.
    /// `Min`/`Max` are `f32::min`/`f32::max`, which return the other operand when one is
    /// `NaN`. Swapping `a` and `b` gives the same result, except that for `Min`/`Max` on
    /// `+0.0` vs `-0.0` either operand may come back.
    pub fn combine(self, a: f32, b: f32) -> f32 {
        match self {
            CombineMode::Average => (a + b) * 0.5,
            CombineMode::Min => a.min(b),
            CombineMode::Max => a.max(b),
            CombineMode::GeometricMean => (a * b).max(0.0).sqrt(), // geometric mean
        }
    }
}

fn resolve_combine_mode(m1: CombineMode, m2: CombineMode) -> CombineMode {
    match (m1, m2) {
        (CombineMode::Max, _) | (_, CombineMode::Max) => CombineMode::Max,
        (CombineMode::Min, _) | (_, CombineMode::Min) => CombineMode::Min,
        (CombineMode::GeometricMean, _) | (_, CombineMode::GeometricMean) => {
            CombineMode::GeometricMean
        }
        _ => CombineMode::Average,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// PhysicsMaterial
// ─────────────────────────────────────────────────────────────────────────────

/// Surface properties of a collider: how much it grips, how much it bounces, how heavy its
/// material is, and the rules by which those merge with whatever it touches.
///
/// Every coefficient is dimensionless and expected to be non-negative. The `with_*` builders
/// clamp, but a struct literal is not validated, so a hand-built material can hold values
/// (negative friction, restitution above 1) that the solver has no sensible answer for.
///
/// A material belongs to a collider — [`Collider::material`](crate::components::Collider) —
/// and reaches the contact solver from there; adding this as a standalone ECS component next
/// to a body does not give that body a surface. Two materials never negotiate at runtime:
/// merging is the pure, stateless [`combine`](Self::combine), which makes the contact
/// coefficients a deterministic function of the pair alone and safe to recompute during
/// replay or rollback.
///
/// [`Default`] is a mid-range surface: friction 0.6/0.5, restitution 0.3, density 1.0,
/// [`GeometricMean`](CombineMode::GeometricMean) friction and [`Max`](CombineMode::Max)
/// restitution.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
// NOT `#[non_exhaustive]`: this is a plain value type that users routinely build
// with `PhysicsMaterial { static_friction: 0.9, ..Default::default() }` to author
// custom materials (the preset consts only cover a fixed set). Keeping it
// exhaustive preserves that ergonomic struct-literal API.
pub struct PhysicsMaterial {
    /// Coulomb coefficient of *static* friction: the largest tangential force a contact can
    /// hold without sliding, as a fraction of the normal force. Dimensionless, non-negative,
    /// and legitimately allowed to exceed 1 (see [`RUBBER`](Self::RUBBER)); 0 means the
    /// contact resists nothing sideways. Physically it should be at least
    /// [`dynamic_friction`](Self::dynamic_friction), but nothing enforces that.
    pub static_friction: f32,
    /// Coulomb coefficient of *kinetic* friction, opposing an already-sliding contact, again
    /// as a fraction of the normal force. Once merged with the partner's, this is the
    /// coefficient the rigid-body pipeline stores as a contact manifold's plain `friction`,
    /// with the static one carried beside it. It shares
    /// [`friction_combine`](Self::friction_combine) with its static twin, so a pair cannot
    /// merge the two under different rules.
    pub dynamic_friction: f32,
    /// Bounciness in `[0, 1]`: 0 is a perfectly inelastic impact, 1 returns the full approach
    /// speed. The `with_*` builders clamp to that range; a struct literal does not, and above
    /// 1 each impact injects energy.
    ///
    /// It shapes the *impact*, not the resting state: the rigid-body pipeline drops
    /// restitution to zero for a contact that was already present on the previous step, so a
    /// bouncy material will not keep a settled stack twitching.
    pub restitution: f32,
    /// Mass per unit volume on a relative scale where water is about 1.0 — the presets are
    /// specific gravities (7.8 for steel-like [`METAL`](Self::METAL), 0.6 for
    /// [`WOOD`](Self::WOOD)), i.e. the g/cm³ figure, not kg/m³. Non-negative; 0 is accepted
    /// and yields zero derived mass.
    ///
    /// It is inert for an ordinary body: nothing multiplies collider volume by it to produce
    /// a mass, and a rigid body is constructed with an explicit mass instead. The one place
    /// the engine turns density into mass is fracture, where a chunk's mass is its volume
    /// times this value — and therefore on the same relative scale, not in kilograms.
    pub density: f32,
    /// Rule for merging both friction coefficients with a partner material's. Only one of the
    /// pair's two modes survives — the higher-priority one, see [`CombineMode`] — and it is
    /// applied to the static and dynamic coefficients alike. Defaults to
    /// [`GeometricMean`](CombineMode::GeometricMean).
    pub friction_combine: CombineMode,
    /// Rule for merging [`restitution`](Self::restitution) with a partner material's,
    /// resolved independently of [`friction_combine`](Self::friction_combine) — a contact can
    /// take its friction from one material's preference and its bounce from the other's.
    /// Defaults to [`Max`](CombineMode::Max), which is why a bouncy material stays bouncy
    /// against a dull one.
    pub restitution_combine: CombineMode,
}

impl Default for PhysicsMaterial {
    fn default() -> Self {
        Self {
            static_friction: 0.6,
            dynamic_friction: 0.5,
            restitution: 0.3,
            density: 1.0,
            friction_combine: CombineMode::GeometricMean,
            restitution_combine: CombineMode::Max,
        }
    }
}

impl PhysicsMaterial {
    /// Shortcut for a material given only bounciness (restitution). Since `restitution_combine`
    /// defaults to `Max`, this material bounces even when the opposing surface is non-bouncy.
    /// E.g.: `Collider::sphere(r).with_material(PhysicsMaterial::bouncy(0.9))`.
    pub fn bouncy(restitution: f32) -> Self {
        Self {
            restitution: restitution.clamp(0.0, 1.0),
            ..Default::default()
        }
    }

    /// Sets bounciness (0=inelastic, 1=fully elastic). Chainable.
    pub fn with_restitution(mut self, restitution: f32) -> Self {
        self.restitution = restitution.clamp(0.0, 1.0);
        self
    }

    /// Sets friction (static = dynamic = `friction`). Chainable.
    pub fn with_friction(mut self, friction: f32) -> Self {
        let f = friction.max(0.0);
        self.static_friction = f;
        self.dynamic_friction = f;
        self
    }

    /// Shortcut for a frictionless material (slippery like ice; restitution stays at its default).
    pub fn frictionless() -> Self {
        Self {
            static_friction: 0.0,
            dynamic_friction: 0.0,
            ..Default::default()
        }
    }

    /// Sets density (for mass/volume computations). Chainable.
    pub fn with_density(mut self, density: f32) -> Self {
        self.density = density.max(0.0);
        self
    }

    /// Combine the contact properties of two materials
    pub fn combine(a: &PhysicsMaterial, b: &PhysicsMaterial) -> CombinedMaterial {
        let f_mode = resolve_combine_mode(a.friction_combine, b.friction_combine);
        let r_mode = resolve_combine_mode(a.restitution_combine, b.restitution_combine);

        let combined = CombinedMaterial {
            static_friction: f_mode.combine(a.static_friction, b.static_friction),
            dynamic_friction: f_mode.combine(a.dynamic_friction, b.dynamic_friction),
            restitution: r_mode.combine(a.restitution, b.restitution),
            density: CombineMode::Average.combine(a.density, b.density),
        };

        // Per-contact-pair operation (hot path → trace only). Shows which combine modes
        // won and the resulting coefficients when debugging unexpected friction/bounce.
        tracing::trace!(
            friction_mode = ?f_mode,
            restitution_mode = ?r_mode,
            static_friction = combined.static_friction,
            dynamic_friction = combined.dynamic_friction,
            restitution = combined.restitution,
            density = combined.density,
            "combined contact material"
        );

        combined
    }

    // ── Hazır Malzemeler ──────────────────────────────────────────────────────

    /// Grippy and lively: friction 1.0/0.9, restitution 0.8, density 1.1.
    ///
    /// Declares [`Max`](CombineMode::Max) on both channels — the top priority — so no partner
    /// can drag a rubber contact below these numbers: rubber on [`ICE`](Self::ICE) still grips
    /// at 0.9 dynamic friction, which is intended, not a bug.
    pub const RUBBER: Self = Self {
        static_friction: 1.0,
        dynamic_friction: 0.9,
        restitution: 0.8,
        density: 1.1,
        friction_combine: CombineMode::Max,
        restitution_combine: CombineMode::Max,
    };

    /// Slippery and dead: friction 0.05/0.03, restitution 0.05, density 0.92 (just under the
    /// water = 1.0 reference).
    ///
    /// Declares [`Min`](CombineMode::Min) on both channels, which outranks everything except
    /// [`Max`](CombineMode::Max): an ice contact never climbs above these numbers, unless the
    /// partner asks for `Max` and overrules the rule entirely.
    pub const ICE: Self = Self {
        static_friction: 0.05,
        dynamic_friction: 0.03,
        restitution: 0.05,
        density: 0.92,
        friction_combine: CombineMode::Min,
        restitution_combine: CombineMode::Min,
    };

    /// Heavy and middling: friction 0.4/0.3, restitution 0.3, density 7.8 — steel on the
    /// water = 1.0 scale, and the densest preset here.
    ///
    /// Its combine rules are the two deferential ones
    /// ([`GeometricMean`](CombineMode::GeometricMean) friction,
    /// [`Average`](CombineMode::Average) restitution), so any partner declaring `Min` or
    /// `Max` decides the contact.
    pub const METAL: Self = Self {
        static_friction: 0.4,
        dynamic_friction: 0.3,
        restitution: 0.3,
        density: 7.8,
        friction_combine: CombineMode::GeometricMean,
        restitution_combine: CombineMode::Average,
    };

    /// Middling and light: friction 0.5/0.4, restitution 0.4, density 0.6 — under the
    /// water = 1.0 reference, and the lightest preset here. Carries exactly the same combine
    /// rules as [`METAL`](Self::METAL), so the two differ only in their numbers.
    pub const WOOD: Self = Self {
        static_friction: 0.5,
        dynamic_friction: 0.4,
        restitution: 0.4,
        density: 0.6,
        friction_combine: CombineMode::GeometricMean,
        restitution_combine: CombineMode::Average,
    };

    /// Rough and deadening: friction 0.8/0.7, restitution 0.1, density 2.4.
    ///
    /// Friction defers ([`GeometricMean`](CombineMode::GeometricMean)) while restitution
    /// insists downward ([`Min`](CombineMode::Min)), so concrete kills most bounces — but
    /// `Min` loses to `Max`, and a [`RUBBER`](Self::RUBBER) ball still comes off it at 0.8.
    pub const CONCRETE: Self = Self {
        static_friction: 0.8,
        dynamic_friction: 0.7,
        restitution: 0.1,
        density: 2.4,
        friction_combine: CombineMode::GeometricMean,
        restitution_combine: CombineMode::Min,
    };

    /// Slick but lively: friction 0.2/0.15, restitution 0.6, density 2.5.
    ///
    /// The only preset that pulls in both directions at once — [`Min`](CombineMode::Min)
    /// friction with [`Max`](CombineMode::Max) restitution — so a glass contact tends to both
    /// slide and bounce, and both of those preferences beat a deferential partner's.
    pub const GLASS: Self = Self {
        static_friction: 0.2,
        dynamic_friction: 0.15,
        restitution: 0.6,
        density: 2.5,
        friction_combine: CombineMode::Min,
        restitution_combine: CombineMode::Max,
    };

    /// Road surface: friction 0.75/0.65, restitution 0.05, density 2.3. A slightly less
    /// grippy [`CONCRETE`](Self::CONCRETE) that swallows impacts harder; the combine rules of
    /// the two are identical, so a scene can swap one for the other and only the numbers move.
    pub const ASPHALT: Self = Self {
        static_friction: 0.75,
        dynamic_friction: 0.65,
        restitution: 0.05,
        density: 2.3,
        friction_combine: CombineMode::GeometricMean,
        restitution_combine: CombineMode::Min,
    };

    /// Loose ground: friction 0.55/0.45, restitution 0.02 (all but total absorption),
    /// density 1.6.
    ///
    /// The only preset asking for [`Average`](CombineMode::Average) friction — the lowest
    /// priority — so its friction rule yields to whatever the other material declares, and
    /// the arithmetic mean is used only when both sides are `Average`.
    pub const SAND: Self = Self {
        static_friction: 0.55,
        dynamic_friction: 0.45,
        restitution: 0.02,
        density: 1.6,
        friction_combine: CombineMode::Average,
        restitution_combine: CombineMode::Min,
    };
}

/// The contact parameters obtained from the combination of two materials
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CombinedMaterial {
    /// Static Coulomb coefficient for this contact pair: the largest tangential force the
    /// contact holds without sliding, as a fraction of the normal force. Dimensionless and
    /// unclamped — above 1 whenever the merged inputs are. See
    /// [`PhysicsMaterial::combine`] for how the pair's value is reached.
    pub static_friction: f32,
    /// Sliding friction for the pair, merged under the same mode as
    /// [`static_friction`](Self::static_friction): the mode is chosen once per contact, so a
    /// pair cannot end up with geometric-mean sliding friction and, say, max static friction.
    pub dynamic_friction: f32,
    /// Bounce for the pair, on the same `[0, 1]` scale as
    /// [`PhysicsMaterial::restitution`] and likewise unclamped here.
    pub restitution: f32,
    /// Plain arithmetic mean of the two densities: no combine mode is consulted for this
    /// field, whatever the materials asked for. A contact has no mass of its own, so this is
    /// informational — the rigid-body contact path reads only the friction and restitution
    /// fields off this struct.
    pub density: f32,
}

#[cfg(feature = "ecs")]
gizmo_core::impl_component!(PhysicsMaterial);

// ─────────────────────────────────────────────────────────────────────────────
// Testler
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ergonomic_material_builders() {
        // `bouncy(e)`: restitution ayarlı, combine=Max (varsayılan) → karşı yüzey mat
        // olsa bile zıplar.
        let m = PhysicsMaterial::bouncy(0.9);
        assert_eq!(m.restitution, 0.9);
        assert_eq!(m.restitution_combine, CombineMode::Max);
        // `with_restitution` / `with_friction` zincirlenebilir + clamp'li.
        let m2 = PhysicsMaterial::default().with_restitution(1.5).with_friction(0.7);
        assert_eq!(m2.restitution, 1.0, "restitution [0,1] aralığına clamp'lanmalı");
        assert_eq!(m2.static_friction, 0.7);
        assert_eq!(m2.dynamic_friction, 0.7);
        assert_eq!(
            PhysicsMaterial::default().with_restitution(-0.5).restitution,
            0.0
        );
        // frictionless: sürtünme sıfır.
        let f = PhysicsMaterial::frictionless();
        assert_eq!(f.static_friction, 0.0);
        assert_eq!(f.dynamic_friction, 0.0);
        // with_density zincirlenebilir.
        assert_eq!(PhysicsMaterial::default().with_density(3.0).density, 3.0);
    }

    #[test]
    fn test_combine_geometric_mean() {
        let a = PhysicsMaterial {
            static_friction: 0.9,
            ..Default::default()
        };
        let b = PhysicsMaterial {
            static_friction: 0.4,
            ..Default::default()
        };
        let c = PhysicsMaterial::combine(&a, &b);
        let expected = (0.9f32 * 0.4).sqrt();
        assert!((c.static_friction - expected).abs() < 1e-5);
    }

    #[test]
    fn test_rubber_ice_low_friction() {
        let r = PhysicsMaterial::RUBBER;
        let i = PhysicsMaterial::ICE;
        // Rubber has CombineMode::Max, Ice has CombineMode::Min.
        // Due to priority Max > Min, the resolved mode is Max.
        // So the dynamic friction is Max(0.9, 0.03) = 0.9
        let c = PhysicsMaterial::combine(&r, &i);
        assert!(
            c.dynamic_friction > 0.5,
            "Rubber's Max mode dominates Ice's Min mode"
        );
    }

    #[test]
    fn test_restitution_max() {
        let a = PhysicsMaterial {
            restitution: 0.9,
            restitution_combine: CombineMode::Max,
            ..Default::default()
        };
        let b = PhysicsMaterial {
            restitution: 0.2,
            restitution_combine: CombineMode::Max,
            ..Default::default()
        };
        let c = PhysicsMaterial::combine(&a, &b);
        assert!((c.restitution - 0.9).abs() < 1e-5);
    }

    #[test]
    fn combine_mode_each_operation() {
        assert!((CombineMode::Average.combine(0.2, 0.8) - 0.5).abs() < 1e-6);
        assert_eq!(CombineMode::Min.combine(0.2, 0.8), 0.2);
        assert_eq!(CombineMode::Max.combine(0.2, 0.8), 0.8);
        assert!((CombineMode::GeometricMean.combine(4.0, 9.0) - 6.0).abs() < 1e-6);
    }

    #[test]
    fn geometric_mean_clamps_negative_product() {
        // A negative operand makes a*b < 0, so a bare sqrt() would be NaN; the .max(0.0)
        // guard must yield 0.0 and stay finite.
        let g = CombineMode::GeometricMean.combine(-1.0, 4.0);
        assert_eq!(g, 0.0);
        assert!(g.is_finite());
    }

    #[test]
    fn resolve_mode_priority_is_max_min_geo_avg() {
        use CombineMode::*;
        // Max dominates every mode.
        for other in [Average, Min, Max, GeometricMean] {
            assert_eq!(resolve_combine_mode(Max, other), Max);
            assert_eq!(resolve_combine_mode(other, Max), Max);
        }
        // Min dominates everything except Max.
        for other in [Average, Min, GeometricMean] {
            assert_eq!(resolve_combine_mode(Min, other), Min);
            assert_eq!(resolve_combine_mode(other, Min), Min);
        }
        // GeometricMean beats only Average.
        assert_eq!(resolve_combine_mode(GeometricMean, Average), GeometricMean);
        assert_eq!(resolve_combine_mode(Average, GeometricMean), GeometricMean);
        // Average only when both are Average.
        assert_eq!(resolve_combine_mode(Average, Average), Average);
    }

    #[test]
    fn resolve_mode_is_symmetric() {
        use CombineMode::*;
        for a in [Average, Min, Max, GeometricMean] {
            for b in [Average, Min, Max, GeometricMean] {
                assert_eq!(
                    resolve_combine_mode(a, b),
                    resolve_combine_mode(b, a),
                    "resolution must not depend on operand order"
                );
            }
        }
    }

    #[test]
    fn combine_density_always_averaged() {
        // Density is averaged regardless of the friction/restitution combine modes.
        let a = PhysicsMaterial {
            density: 2.0,
            friction_combine: CombineMode::Max,
            restitution_combine: CombineMode::Max,
            ..Default::default()
        };
        let b = PhysicsMaterial { density: 8.0, ..a };
        let c = PhysicsMaterial::combine(&a, &b);
        assert!((c.density - 5.0).abs() < 1e-6);
    }

    #[test]
    fn combine_is_symmetric_in_values() {
        let a = PhysicsMaterial::RUBBER;
        let b = PhysicsMaterial::ICE;
        let ab = PhysicsMaterial::combine(&a, &b);
        let ba = PhysicsMaterial::combine(&b, &a);
        assert!((ab.static_friction - ba.static_friction).abs() < 1e-6);
        assert!((ab.dynamic_friction - ba.dynamic_friction).abs() < 1e-6);
        assert!((ab.restitution - ba.restitution).abs() < 1e-6);
        assert!((ab.density - ba.density).abs() < 1e-6);
    }

    #[test]
    fn presets_encode_sensible_extremes() {
        // Spot-check that the preset table isn't accidentally scrambled. These are all
        // `const` presets, so assert at compile time (a bad edit fails the build).
        const { assert!(PhysicsMaterial::ICE.dynamic_friction < PhysicsMaterial::RUBBER.dynamic_friction) };
        const { assert!(PhysicsMaterial::RUBBER.restitution > PhysicsMaterial::CONCRETE.restitution) };
        const { assert!(PhysicsMaterial::METAL.density > PhysicsMaterial::WOOD.density) };
        // Every preset stays inside physically meaningful ranges.
        for m in [
            PhysicsMaterial::RUBBER,
            PhysicsMaterial::ICE,
            PhysicsMaterial::METAL,
            PhysicsMaterial::WOOD,
            PhysicsMaterial::CONCRETE,
            PhysicsMaterial::GLASS,
            PhysicsMaterial::ASPHALT,
            PhysicsMaterial::SAND,
        ] {
            assert!((0.0..=1.0).contains(&m.restitution), "restitution {m:?}");
            assert!(m.static_friction >= 0.0 && m.density > 0.0);
        }
    }
}
