//! What a [`MaterialType`] means to a draw loop, decided once.
//!
//! # Why this exists
//!
//! The engine has two draw loops — `gizmo::systems::render::batching::collect_draw_items` and
//! `gizmo-studio`'s `execute_render_pipeline` — and each decided independently what every material
//! type means. Both did it with a `match … { _ => 0.0 }`, and that wildcard is where capabilities
//! went to die: a variant nobody remembered fell through to deferred PBR silently, in whichever
//! path had not been taught about it.
//!
//! It had already happened twice, in opposite directions, and neither showed up as an error:
//!
//! - **`BakedLit`** was routed by the engine path (instance flag 1.0, its own forward pipeline) and
//!   fell through studio's wildcard to 0.0 — deferred PBR — so a baked-lit level rendered one way
//!   in the game and another in the editor.
//! - **`Grid`** was routed by studio (`is_grid`, its own pipeline) and had no arm at all in the
//!   engine path.
//!
//! `MaterialType` is `#[non_exhaustive]`, so a downstream crate is *obliged* to write a wildcard —
//! which is exactly why the decision cannot live downstream. Inside this crate an exhaustive match
//! is legal, so a ninth variant becomes a compile error **here**, once, instead of two silent
//! misroutes out there.
//!
//! # What it does not do
//!
//! It does not merge the two paths, and it should not: the deferred G-buffer path and the editor's
//! forward path genuinely differ, that difference is the part with no automated coverage, and a
//! change there is gated on human-eye A/B. What is single-sourced here is only the *semantics* —
//! what a material type is — not what either path does about it. Both divergences named above are
//! still divergences after this; the difference is that they are now visible as two call sites
//! reading different fields of one answer, rather than two matches nobody compared.

use crate::components::MaterialType;

/// Everything a draw loop needs to decide from a material's type alone.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct Routing {
    /// The value written into `InstanceRaw`'s shading slot.
    ///
    /// `2.0` skybox, `1.0` "not deferred PBR", `0.0` deferred PBR. Read by the forward shader to
    /// branch, and by anything inspecting the instance buffer generically.
    pub instance_flag: f32,
    /// The type is exactly [`MaterialType::Unlit`].
    ///
    /// Not the same question as [`Self::skips_deferred`], though the two shared a name in the two
    /// loops before this existed — one path called its routing category `unlit` and the other
    /// called its type test `is_unlit`, which is precisely the kind of collision that hides a
    /// divergence.
    pub unlit_material: bool,
    /// Lighting is baked into the vertex colour: forward, but still casts into the cascades.
    pub baked_lit: bool,
    /// The procedural atmospheric gradient, which ignores the mesh.
    pub is_skybox: bool,
    /// The editor's reference grid.
    pub is_grid: bool,
    /// A painted backdrop, camera-locked or placed. Mirrors [`crate::backdrop::is_backdrop`].
    pub is_backdrop: bool,
    /// Drawn forward rather than through the G-buffer.
    ///
    /// Skybox, baked-lit, backdrop and unlit all skip the deferred path. They part company in the
    /// **shadow** pass, where baked-lit casts and the others do not — so this flag is not a licence
    /// to treat them alike beyond pass selection.
    pub skips_deferred: bool,
}

/// Decide what a material type means. Exhaustive on purpose — see the module docs.
#[must_use]
pub fn route(material_type: MaterialType) -> Routing {
    // No `_` arm. Adding a variant to `MaterialType` must fail to compile here, which is the whole
    // point of the module: one compile error beats two silent misroutes.
    let (instance_flag, unlit_material, baked_lit, is_skybox, is_grid, is_backdrop) =
        match material_type {
            MaterialType::Pbr => (0.0, false, false, false, false, false),
            MaterialType::Water => (0.0, false, false, false, false, false),
            MaterialType::Unlit => (1.0, true, false, false, false, false),
            MaterialType::BakedLit => (1.0, false, true, false, false, false),
            MaterialType::Skybox => (2.0, false, false, true, false, false),
            MaterialType::Backdrop | MaterialType::BackdropPlaced => {
                (1.0, false, false, false, false, true)
            }
            // The engine path had no arm for this at all and gave it 0.0 through its wildcard;
            // studio routes it to its own pipeline. 0.0 preserves what the engine path did.
            MaterialType::Grid => (0.0, false, false, false, true, false),
        };

    Routing {
        instance_flag,
        unlit_material,
        baked_lit,
        is_skybox,
        is_grid,
        is_backdrop,
        skips_deferred: is_skybox || baked_lit || is_backdrop || unlit_material,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two divergences that motivated this module, pinned as facts.
    ///
    /// `BakedLit` must not read as deferred PBR: studio's wildcard gave it 0.0 while the engine
    /// path gave it 1.0, so the same level shaded differently in the editor and in the game.
    #[test]
    fn baked_lit_is_not_deferred_pbr() {
        let r = route(MaterialType::BakedLit);
        assert_eq!(r.instance_flag, 1.0, "BakedLit fell through a wildcard to 0.0 in one path");
        assert!(r.baked_lit);
        assert!(r.skips_deferred);
        assert!(!r.unlit_material, "baked-lit casts shadows; unlit does not — not the same flag");
    }

    /// `Grid` was known to studio and invisible to the engine path.
    #[test]
    fn grid_is_named_rather_than_defaulted() {
        let r = route(MaterialType::Grid);
        assert!(r.is_grid);
        assert!(!r.skips_deferred, "the engine path's wildcard put Grid on the deferred side");
    }

    /// Every variant answers, and the answers stay distinguishable.
    #[test]
    fn every_variant_has_an_answer() {
        let all = [
            MaterialType::Pbr,
            MaterialType::Unlit,
            MaterialType::BakedLit,
            MaterialType::Skybox,
            MaterialType::Backdrop,
            MaterialType::BackdropPlaced,
            MaterialType::Water,
            MaterialType::Grid,
        ];
        for t in all {
            let r = route(t);
            assert!(
                (0.0..=2.0).contains(&r.instance_flag),
                "{t:?} produced an instance flag outside the shader's three cases"
            );
            assert_eq!(
                r.is_backdrop,
                crate::backdrop::is_backdrop(t),
                "{t:?}: routing and backdrop::is_backdrop disagree"
            );
        }
    }

    /// The forward group and the shadow-casting group are not the same group.
    #[test]
    fn skipping_deferred_does_not_mean_skipping_shadows() {
        assert!(route(MaterialType::BakedLit).skips_deferred);
        assert!(route(MaterialType::Unlit).skips_deferred);
        assert_ne!(
            route(MaterialType::BakedLit).baked_lit,
            route(MaterialType::Unlit).baked_lit,
            "these two share skips_deferred and part company in the shadow pass"
        );
    }
}
