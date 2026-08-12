//! The painted-backdrop path: what [`MaterialType::Backdrop`] means, expressed as pure
//! functions so it can be asserted without a GPU.
//!
//! # Why a material type and not a `depth_write` knob on `Material`
//!
//! Drawing a scene's own sky/panorama geometry correctly needs three things at once — drawn
//! before the world, locked to the camera (translation removed, rotation kept), and never
//! writing depth — plus the ordinary pixels of a mesh (its texture and its vertex colour).
//! Exposing those as independent knobs would mean three of them, every one of which is wrong
//! on its own: `depth_write = false` alone gives a panel that still moves with the world and
//! still wins the depth test; a camera lock alone gives a panel welded to the view that
//! occludes everything. Seven of the eight combinations are bugs, and nothing could reject
//! them, because a knob is by construction settable in isolation.
//!
//! A material type is one word at the call site and the three properties travel together:
//!
//! ```no_run
//! use gizmo_renderer::components::Material;
//! use gizmo_math::Vec4;
//! # use std::sync::Arc;
//! // `no_run`: `Material::new` takes an `Arc<wgpu::BindGroup>`, so building one needs a GPU
//! // adapter. The example compiles — which is what pins the builder chain — and is never run.
//! fn make_backdrop(texture_bind_group: Arc<wgpu::BindGroup>, path: String) -> Material {
//!     Material::new(texture_bind_group)
//!         .with_backdrop(Vec4::ONE)        // white tint: the mesh's own pixels, unmodified
//!         .with_texture_source(path)
//! }
//! ```
//!
//! The cost is that it is narrower — a caller who wants a depth-write-off *world* material
//! still has none. That is the trade taken deliberately: the reported need is a backdrop, and
//! `Material` already carries eleven public fields whose legal combinations are undocumented.
//!
//! # Where each property is enforced
//!
//! | property | enforced by | testable without an adapter |
//! |---|---|---|
//! | drawn before the world | the draw-order rank in the batcher (`DrawLayer::Backdrop`) | yes — a comparator |
//! | locked to the camera | `shaders/backdrop.wgsl` `vs_main`, mirrored by [`backdrop_clip_position`] | yes — glam arithmetic |
//! | never writes depth | [`backdrop_state`]`().depth_write == false`, plus the far-plane pin | yes — a pure state fn |
//! | samples texture + vertex colour | `shaders/backdrop.wgsl` `fs_main`, mirrored by [`backdrop_rgba`] | yes — arithmetic + a source check |
//!
//! What is NOT testable here is whether the resulting frame looks right: this crate can
//! neither open a surface nor read a pixel. Nothing below claims it does.

use crate::components::MaterialType;
use gizmo_math::{Mat4, Vec3, Vec4};

/// The NDC depth every backdrop vertex is pinned to, mirroring the constant of the same name
/// in `shaders/backdrop.wgsl`.
///
/// Just short of the `1.0` far plane, because the depth comparison is `LessEqual` against a
/// buffer cleared to `1.0`: at exactly `1.0` the backdrop still draws, but so does anything
/// else pinned there, and the pin stops being a statement about ordering. Just short of it,
/// the backdrop loses to every fragment of real geometry and wins only against the clear.
pub const BACKDROP_NDC_DEPTH: f32 = 0.99999;

/// Whether this material's geometry is drawn locked to the camera — the authored transform
/// positions it *relative to the viewer*, not in the world.
///
/// Callers that cull, sort or pick against a model matrix must put it through
/// [`camera_locked_model`] first, or they will reason about a position the GPU never uses.
#[inline]
pub fn is_camera_locked(material_type: MaterialType) -> bool {
    matches!(material_type, MaterialType::Backdrop)
}

/// Whether this material draws through the backdrop path at all — locked or placed.
///
/// The predicate every caller that means "is this a backdrop" wants, so adding a third variant
/// later is one edit here rather than a grep for `== Backdrop` across three crates.
#[inline]
pub fn is_backdrop(material_type: MaterialType) -> bool {
    matches!(material_type, MaterialType::Backdrop | MaterialType::BackdropPlaced)
}

/// The model matrix to **upload**, which is not always the one the mesh was authored with.
///
/// `shaders/backdrop.wgsl` adds the camera position to every vertex it transforms — that single
/// addition is the camera lock. A [`MaterialType::BackdropPlaced`] wants the rest of the backdrop
/// path and not that, so this hands the shader a matrix with the camera position already taken
/// out: `T(−c) · M`, whose vertices the shader then puts back at `M · v`. The authored place,
/// through the same pipeline, with no second shader to keep in step with the first.
///
/// It is the counterpart of [`camera_locked_model`] and the two must be read together —
/// that one says *where the triangles land* (for culling and LOD), this one says *what to send*.
/// For every material type exactly one of them is the identity.
#[inline]
pub fn instance_model(material_type: MaterialType, model: &Mat4, camera_pos: Vec3) -> Mat4 {
    if matches!(material_type, MaterialType::BackdropPlaced) {
        Mat4::from_translation(-camera_pos) * *model
    } else {
        *model
    }
}

/// The model matrix the vertex shader effectively draws with — the authored one for ordinary
/// geometry, and the authored one shifted to the camera for a camera-locked material.
///
/// This is the CPU's only correct answer to "where is this thing this frame". A camera-locked
/// backdrop authored around the origin is 800 m away from a camera that has driven 800 m, so
/// culling it against the raw model matrix drops the entire backdrop the moment the player
/// leaves the middle of the map — the geometry is on screen and the frustum test says it is
/// not.
#[inline]
pub fn camera_locked_model(material_type: MaterialType, model: &Mat4, camera_pos: Vec3) -> Mat4 {
    if is_camera_locked(material_type) {
        Mat4::from_translation(camera_pos) * *model
    } else {
        *model
    }
}

/// CPU mirror of `vs_main` in `shaders/backdrop.wgsl`: where a local-space backdrop vertex
/// lands in clip space.
///
/// `model` is what was **uploaded** — run an authored matrix through [`instance_model`] first, or
/// this answers for a draw the GPU never makes.
///
/// `view_proj` is `projection · view` as uploaded in
/// [`SceneUniforms`](crate::gpu_types::SceneUniforms), and `camera_pos` the same struct's
/// camera position — the two must come from the same frame, since the whole construction
/// depends on `camera_pos` being the translation `view` removes.
pub fn backdrop_clip_position(
    view_proj: &Mat4,
    camera_pos: Vec3,
    model: &Mat4,
    local: Vec3,
) -> Vec4 {
    let world = model.transform_point3(local) + camera_pos;
    let mut clip = *view_proj * world.extend(1.0);
    clip.z = clip.w * BACKDROP_NDC_DEPTH;
    clip
}

/// CPU mirror of `fs_main` in `shaders/backdrop.wgsl`: `vertex colour × instance albedo ×
/// texture`, componentwise, alpha included.
///
/// Every factor is the mesh's own — this is the half `MaterialType::Skybox` throws away.
pub fn backdrop_rgba(vertex_colour: Vec4, instance_albedo: Vec4, texel: Vec4) -> Vec4 {
    vertex_colour * instance_albedo * texel
}

/// The fixed-function state of the backdrop pipeline.
///
/// A built `wgpu::RenderPipeline` is opaque — nothing can ask it afterwards whether it writes
/// depth — so the choice is made in [`backdrop_state`], a pure function a test can read. That
/// is the only way "does not write depth" is checkable in a repo that cannot see pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BackdropState {
    pub(crate) blend: Option<wgpu::BlendState>,
    pub(crate) depth_write: bool,
    pub(crate) cull: Option<wgpu::Face>,
}

/// The one state a backdrop draw gets.
///
/// - **No depth write.** The defining property. A backdrop sits behind everything; if it wrote
///   depth it would occlude the world it is a backdrop for.
/// - **Alpha blending.** A panorama panel with a cut-out edge (a torn skyline, a distant tree
///   line) carries that edge in its texture alpha, and `blend: None` computes the alpha and
///   then discards it — the bug that had already been found once on the baked-lit path.
/// - **No backface culling.** A backdrop is a single-sided panel or an inside-out dome; which
///   winding faces the camera is an authoring accident, and culling the wrong one makes the
///   panel vanish rather than look wrong, which is much harder to diagnose.
pub(crate) fn backdrop_state() -> BackdropState {
    BackdropState {
        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
        depth_write: false,
        cull: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = include_str!("shaders/backdrop.wgsl");

    /// A right-handed perspective camera at `eye` looking along `forward` — the same
    /// construction `Camera::get_view` + `get_projection` make, so `view_proj` and `eye` are
    /// exactly the pair `SceneUniforms` uploads.
    fn view_proj(eye: Vec3, forward: Vec3) -> Mat4 {
        let view = Mat4::look_at_rh(eye, eye + forward, Vec3::Y);
        let proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, 16.0 / 9.0, 0.1, 2000.0);
        proj * view
    }

    fn ndc(clip: Vec4) -> Vec3 {
        (clip / clip.w).truncate()
    }

    /// What the shader WOULD compute without the camera lock — i.e. what `Unlit` computes.
    /// Present so the tests below can show the property is actually being tested.
    fn unlocked_clip(vp: &Mat4, model: &Mat4, local: Vec3) -> Vec4 {
        *vp * model.transform_point3(local).extend(1.0)
    }

    // ── a placed backdrop stays where it was authored ──────────────────────────────────────

    /// The property `MaterialType::BackdropPlaced` exists for: the vertex lands at the authored
    /// world position, **through the same shader** that adds the camera position to everything.
    ///
    /// This is the whole design in one assertion. If `instance_model` and the shader's addition
    /// ever stop cancelling, a placed backdrop slides with the viewer and the test fails.
    #[test]
    fn a_placed_backdrop_lands_where_it_was_authored() {
        let authored = Mat4::from_translation(Vec3::new(-1200.0, 40.0, 800.0));
        let local = Vec3::new(3.0, -2.0, 11.0);
        let want = authored.transform_point3(local);

        for eye in [Vec3::ZERO, Vec3::new(800.0, 12.0, -450.0), Vec3::new(-3000.0, 0.0, 3000.0)] {
            let upload = instance_model(MaterialType::BackdropPlaced, &authored, eye);
            // `backdrop_clip_position` re-adds the camera position, exactly as `vs_main` does.
            let world = upload.transform_point3(local) + eye;
            assert!(
                (world - want).length() < 1e-3,
                "placed backdrop moved with the camera at {eye:?}: {world:?} != {want:?}"
            );
        }
    }

    /// And the locked one still moves with the camera — the two must not collapse into each
    /// other, which is what a wrong `matches!` in either helper would do.
    #[test]
    fn a_locked_backdrop_still_follows_the_camera_and_a_placed_one_does_not() {
        let authored = Mat4::from_translation(Vec3::new(0.0, 0.0, -50.0));
        let local = Vec3::ZERO;
        let a = Vec3::ZERO;
        let b = Vec3::new(500.0, 0.0, 0.0);

        let locked = |eye: Vec3| {
            instance_model(MaterialType::Backdrop, &authored, eye).transform_point3(local) + eye
        };
        let placed = |eye: Vec3| {
            instance_model(MaterialType::BackdropPlaced, &authored, eye).transform_point3(local)
                + eye
        };
        assert!((locked(b) - locked(a)).length() > 499.0, "the lock stopped locking");
        assert!((placed(b) - placed(a)).length() < 1e-3, "the placed variant is following");

        // And the CPU's "where does it land" answer agrees with each.
        assert_eq!(camera_locked_model(MaterialType::BackdropPlaced, &authored, b), authored);
        assert_ne!(camera_locked_model(MaterialType::Backdrop, &authored, b), authored);
        assert!(is_backdrop(MaterialType::BackdropPlaced));
        assert!(!is_camera_locked(MaterialType::BackdropPlaced));
    }

    // ── (2) locked to the camera ───────────────────────────────────────────────────────────

    // The whole point of the lock: the backdrop is nailed to the viewer. Two cameras with the
    // same orientation 900 m apart must put the same backdrop vertex on the same pixel.
    #[test]
    fn camera_translation_does_not_move_the_backdrop() {
        let fwd = Vec3::new(0.0, -0.1, -1.0).normalize();
        let model = Mat4::from_translation(Vec3::new(0.0, 40.0, -300.0));
        let local = Vec3::new(12.0, 3.0, -1.0);

        let near_origin = view_proj(Vec3::new(0.0, 1.6, 0.0), fwd);
        let far_away = view_proj(Vec3::new(-620.0, 9.0, 700.0), fwd);

        let a = ndc(backdrop_clip_position(&near_origin, Vec3::new(0.0, 1.6, 0.0), &model, local));
        let b = ndc(backdrop_clip_position(&far_away, Vec3::new(-620.0, 9.0, 700.0), &model, local));
        assert!(
            (a - b).length() < 1e-4,
            "the backdrop moved when the camera translated: {a:?} vs {b:?}"
        );

        // …and the same pair WITHOUT the lock lands somewhere else entirely. This is the
        // premise: the property under test is not vacuously true of any transform.
        let ua = ndc(unlocked_clip(&near_origin, &model, local));
        let ub = ndc(unlocked_clip(&far_away, &model, local));
        // Half the NDC cube is most of the screen; the unlocked pair is nowhere near each other.
        assert!(
            (ua - ub).length() > 0.5,
            "premise broken: an unlocked panel is supposed to slide with the camera ({ua:?} vs {ub:?})"
        );
    }

    // Translation removed, ROTATION KEPT — half a lock is a backdrop painted on the lens.
    #[test]
    fn camera_rotation_still_moves_the_backdrop() {
        let eye = Vec3::new(5.0, 2.0, -3.0);
        let model = Mat4::IDENTITY;
        let local = Vec3::new(0.0, 20.0, -200.0);

        let ahead = ndc(backdrop_clip_position(
            &view_proj(eye, Vec3::NEG_Z),
            eye,
            &model,
            local,
        ));
        let turned = ndc(backdrop_clip_position(
            &view_proj(eye, Vec3::new(-0.6, 0.0, -0.8).normalize()),
            eye,
            &model,
            local,
        ));
        assert!(
            (ahead - turned).length() > 0.1,
            "turning the camera left the backdrop where it was — the lock swallowed the \
             rotation too, which paints the sky onto the lens: {ahead:?} vs {turned:?}"
        );
    }

    // The lock is exactly "drop the view's translation": a vertex `d` from the camera-locked
    // origin must project where an ordinary vertex `d` from the camera would.
    #[test]
    fn the_lock_is_precisely_the_view_translation_removed() {
        let eye = Vec3::new(133.0, 8.0, -77.0);
        let fwd = Vec3::new(0.3, -0.2, -1.0).normalize();
        let vp = view_proj(eye, fwd);
        let d = Vec3::new(-30.0, 15.0, -250.0);

        let locked = backdrop_clip_position(&vp, eye, &Mat4::IDENTITY, d);
        // The same offset expressed in world space around the camera, drawn as ordinary
        // geometry: same pixel, and (before the depth pin) the same clip w.
        let plain = unlocked_clip(&vp, &Mat4::IDENTITY, eye + d);
        assert!(
            (ndc(locked).truncate() - ndc(plain).truncate()).length() < 1e-4,
            "camera-locked (x) and world-space (camera + x) must land on the same pixel"
        );
        assert!((locked.w - plain.w).abs() < 1e-3, "the lock perturbed clip w");
    }

    // ── (3) never in front of the world ────────────────────────────────────────────────────

    // The reported symptom, as arithmetic: with `Unlit`, "two washed-out panels sit between
    // the camera and the world". A backdrop panel 5 m from the camera and a wall 50 m from it
    // — the panel must lose the depth test, and without the pin it wins.
    #[test]
    fn a_near_backdrop_panel_loses_the_depth_test_to_a_far_wall() {
        let eye = Vec3::new(0.0, 1.5, 0.0);
        let vp = view_proj(eye, Vec3::NEG_Z);
        let panel_local = Vec3::new(0.0, 0.0, -5.0);
        let wall_world = Vec3::new(0.0, 1.5, -50.0);

        let wall_z = ndc(unlocked_clip(&vp, &Mat4::IDENTITY, wall_world)).z;
        let panel_z = ndc(backdrop_clip_position(&vp, eye, &Mat4::IDENTITY, panel_local)).z;

        // `depth_compare: LessEqual` — the smaller z wins.
        assert!(
            wall_z < panel_z,
            "the wall must beat the backdrop (wall {wall_z}, backdrop {panel_z})"
        );

        // Premise: the same panel drawn as ordinary geometry beats the wall. That is the bug.
        let unpinned_z = ndc(unlocked_clip(&vp, &Mat4::IDENTITY, eye + panel_local)).z;
        assert!(
            unpinned_z < wall_z,
            "premise broken: an unpinned 5 m panel is supposed to occlude a 50 m wall \
             ({unpinned_z} vs {wall_z})"
        );
    }

    // Every backdrop vertex lands at the same depth however far it is authored, so no piece of
    // backdrop can occlude another piece — which is why their order is decided by the draw
    // rank instead. It must also stay strictly inside the clip volume: at z == w the far-plane
    // clip is a coin toss between drivers.
    #[test]
    fn backdrop_depth_is_pinned_and_inside_the_clip_volume() {
        let eye = Vec3::new(-4.0, 12.0, 60.0);
        let vp = view_proj(eye, Vec3::NEG_Z);
        for local in [
            Vec3::new(0.0, 0.0, -0.5),
            Vec3::new(0.0, 0.0, -900.0),
            Vec3::new(400.0, 200.0, -1800.0),
        ] {
            let clip = backdrop_clip_position(&vp, eye, &Mat4::IDENTITY, local);
            let z = clip.z / clip.w;
            assert!(
                (z - BACKDROP_NDC_DEPTH).abs() < 1e-5,
                "backdrop vertex {local:?} landed at depth {z}, not the pinned {BACKDROP_NDC_DEPTH}"
            );
            assert!(z < 1.0, "a pinned depth of {z} is on or past the far plane and may be clipped");
        }
    }

    #[test]
    fn backdrop_pipeline_state_holds_the_no_depth_write_property() {
        let s = backdrop_state();
        assert!(
            !s.depth_write,
            "a backdrop that writes depth occludes the world it is a backdrop FOR — this is \
             the property the whole material type exists for"
        );
        assert_eq!(
            s.blend,
            Some(wgpu::BlendState::ALPHA_BLENDING),
            "with `None` the fragment alpha is computed and then discarded, so a cut-out \
             panorama edge draws as a hard rectangle"
        );
        assert_eq!(s.cull, None, "a backdrop panel must be visible from whichever side faces the camera");
    }

    // ── (4) the mesh's own pixels ──────────────────────────────────────────────────────────

    // `MaterialType::Skybox` reaches the screen with none of the mesh's own data: `sky.wgsl`
    // has zero `textureSample` calls and never reads `input.color`, so 191 textured backdrop
    // meshes came out as one invented gradient. Both inputs must survive to the output here.
    #[test]
    fn every_factor_of_the_mesh_reaches_the_pixel() {
        let white = Vec4::ONE;
        let tex = Vec4::new(0.8, 0.4, 0.2, 1.0);
        let vcol = Vec4::new(0.5, 1.0, 1.0, 1.0);
        let tint = Vec4::new(1.0, 1.0, 0.25, 1.0);

        // The texture reaches the pixel …
        assert_eq!(backdrop_rgba(white, white, tex), tex);
        // … the vertex colour reaches the pixel …
        assert_eq!(backdrop_rgba(vcol, white, white), vcol);
        // … and the material tint does, all three multiplying rather than replacing.
        let all = backdrop_rgba(vcol, tint, tex);
        assert_eq!(all, Vec4::new(0.5 * 0.8, 0.4, 0.25 * 0.2, 1.0));

        // Alpha is a channel, not padding: a cut-out texel stays cut out.
        let cutout = backdrop_rgba(white, white, Vec4::new(1.0, 1.0, 1.0, 0.0));
        assert_eq!(cutout.w, 0.0, "texture alpha must reach the fragment's alpha");
        let faded = backdrop_rgba(Vec4::new(1.0, 1.0, 1.0, 0.5), white, white);
        assert_eq!(faded.w, 0.5, "vertex alpha must reach the fragment's alpha");
    }

    // A vertex colour of black means the author painted it black. `unlit.wgsl` still rewrites
    // a near-zero colour to white; that guess must not be copied here (the same call that was
    // made for `baked_lit.wgsl`).
    #[test]
    fn a_black_vertex_colour_is_taken_at_face_value() {
        let black = Vec4::new(0.0, 0.0, 0.0, 1.0);
        let out = backdrop_rgba(black, Vec4::ONE, Vec4::ONE);
        assert_eq!(out.truncate(), Vec3::ZERO, "a black-painted panel came out lit");
    }

    // ── the shader is text; the mirrors above are Rust. Nothing else notices a drift. ───────

    #[test]
    fn the_shader_still_computes_the_mirrored_expressions() {
        assert!(
            SRC.contains("let world = local + scene.camera_pos.xyz;"),
            "backdrop.wgsl no longer camera-locks the way `backdrop_clip_position` mirrors"
        );
        assert!(
            SRC.contains("clip.z = clip.w * BACKDROP_NDC_DEPTH;"),
            "backdrop.wgsl no longer pins depth to the far plane"
        );
        assert!(
            SRC.contains(&format!("const BACKDROP_NDC_DEPTH: f32 = {BACKDROP_NDC_DEPTH};")),
            "backdrop.wgsl's far-plane constant drifted from `BACKDROP_NDC_DEPTH`"
        );
        assert!(
            SRC.contains("let rgb = in.color.rgb * in.inst_albedo.rgb * tex.rgb;")
                && SRC.contains("let alpha = in.color.a * in.inst_albedo.a * tex.a;"),
            "backdrop.wgsl's fragment no longer matches `backdrop_rgba`"
        );
    }

    // The measured half of the report: `grep -c textureSample sky.wgsl` is 0. Whatever else
    // changes in this shader, it has to sample the mesh's texture and read its vertex colour —
    // that is the entire reason it exists alongside the skybox.
    #[test]
    fn the_shader_samples_the_mesh_texture_and_vertex_colour() {
        assert!(
            SRC.contains("textureSample(t_diffuse, s_diffuse, in.tex_coords)"),
            "a backdrop that does not sample its texture is just the skybox again"
        );
        assert!(
            SRC.contains("@location(1) color: vec4<f32>"),
            "backdrop.wgsl stopped taking the vertex colour (with its alpha) as an input"
        );
        assert!(
            !SRC.contains("length(in.color") && !SRC.contains("length(v_color"),
            "the vertex colour is being second-guessed again — see `baked_lit.wgsl`"
        );
        // The invented gradient is the thing being replaced; it must not creep back in.
        assert!(
            !SRC.contains("scene.sun_color"),
            "a painted backdrop must not tint itself from the sun — that is `sky.wgsl`"
        );
    }

    // ── the CPU's idea of where a backdrop is ──────────────────────────────────────────────

    #[test]
    fn only_backdrops_are_camera_locked() {
        let cam = Vec3::new(800.0, 5.0, -200.0);
        let model = Mat4::from_translation(Vec3::new(0.0, 30.0, 0.0));

        // A backdrop's effective transform follows the camera …
        let locked = camera_locked_model(MaterialType::Backdrop, &model, cam);
        assert!(
            (locked.transform_point3(Vec3::ZERO) - (cam + Vec3::new(0.0, 30.0, 0.0))).length() < 1e-3,
            "a camera-locked model must resolve to the camera plus the authored offset"
        );
        // …and it is exactly what the vertex shader draws, so a cull test on it is honest.
        let vp = view_proj(cam, Vec3::NEG_Z);
        let via_model = ndc(vp * locked.transform_point3(Vec3::new(1.0, 2.0, -60.0)).extend(1.0));
        let via_shader = ndc(backdrop_clip_position(&vp, cam, &model, Vec3::new(1.0, 2.0, -60.0)));
        assert!(
            (via_model.truncate() - via_shader.truncate()).length() < 1e-4,
            "`camera_locked_model` disagrees with the vertex shader about where the backdrop is"
        );

        // …every other material type is left exactly alone (bit-for-bit, not approximately).
        for mt in [
            MaterialType::Pbr,
            MaterialType::Unlit,
            MaterialType::BakedLit,
            MaterialType::Skybox,
            MaterialType::Water,
            MaterialType::Grid,
        ] {
            assert_eq!(
                camera_locked_model(mt, &model, cam).to_cols_array(),
                model.to_cols_array(),
                "{mt:?} must not be camera-locked"
            );
            assert!(!is_camera_locked(mt), "{mt:?} must not be camera-locked");
        }
        assert!(is_camera_locked(MaterialType::Backdrop));
    }
}
