//! The authored "look" — bloom, vignette, aberration, depth of field, film grain — as a component
//! on the camera that sees it.
//!
//! # Why it is a component, and why on the camera
//!
//! These knobs existed in two unrelated places and an author could only reach the wrong one. The
//! editor's "🌍 World & Environment Settings" panel edited `EditorState::post_process`, which lives
//! in `gizmo-editor` and is read by exactly one thing: the editor's own viewport. The engine's
//! frame read a second, unrelated copy — `Renderer`'s own fields — settable only from Rust. So the
//! look a user tuned was gone on reopen (nothing wrote it to a file) and absent from an exported
//! build (the shipped game reads the renderer's neutral defaults), while the panel's own sentence
//! said these were the *scene's* settings.
//!
//! The camera is where the precedent already was: `Camera::exposure` is a component field, it
//! round-trips in a scene, and it is the one post-process value the shipped game has always read.
//! Post-processing is a property of the eye, not of the world — two cameras in one scene can want
//! different looks, and a component per camera says that where a single scene-wide block could not.
//!
//! # The default is "as if this component were absent"
//!
//! Every value below is what the engine's frame produces for a camera that carries no
//! `PostProcess` at all, `dof_blur_size` included: the renderer's `dof_enabled` is false by
//! default and zeroes the blur, so the neutral value here is 0.0 rather than the renderer's 4.0.
//! Adding the component to a camera therefore changes nothing until a slider moves, which is what
//! makes it safe to add from the ➕ menu.
//!
//! Exposure is deliberately **not** here. It is already on [`Camera`], it already
//! round-trips, and duplicating it would create the very split this component exists to end.

/// Per-camera post-processing: what the frame this camera renders is graded with.
///
/// See the module docs for why this lives on the camera and why its default is the no-component
/// behaviour. Exposure is on [`Camera::exposure`](super::Camera), not here.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct PostProcess {
    /// How much of the bloom blur is added back over the frame. 0 = no bloom.
    pub bloom_intensity: f32,
    /// The luminance a texel must exceed to contribute to bloom.
    pub bloom_threshold: f32,
    /// How dark the frame's corners get. 0 = no vignette.
    pub vignette: f32,
    /// Per-channel radial UV offset, in screen fractions. 0 = no aberration.
    pub chromatic_aberration: f32,
    /// Depth of field: the distance, in metres, that is perfectly in focus.
    pub dof_focus_dist: f32,
    /// Depth of field: how far either side of [`Self::dof_focus_dist`] still counts as sharp.
    pub dof_focus_range: f32,
    /// Depth of field: the maximum blur radius outside that range. **0 switches DoF off**, which
    /// is what the renderer's `dof_enabled = false` does and therefore what the default is.
    pub dof_blur_size: f32,
    /// Depth of field through a lens: the aperture as an f-number, so 1.4 is shallow and 16 nearly
    /// all sharp. **0 uses the focus band** ([`dof_focus_range`](Self::dof_focus_range)) instead, and
    /// `dof_blur_size` is then the largest blur a lens may give. Set with
    /// [`with_lens`](Self::with_lens).
    pub dof_f_stops: f32,
    /// The sensor's height in metres, which with the camera's field of view makes the focal length.
    /// Super 35, 18.66 mm, unless set.
    pub dof_sensor_height: f32,
    /// Farther than this counts as this far, in metres. 0 = no limit. A sky has no real distance, and
    /// without a limit it takes the largest blur there is.
    pub dof_max_depth: f32,
    /// Strength of the animated grain. 0 = none.
    pub film_grain: f32,
    /// Camera motion blur: the fraction of the frame the shutter stays open — **0 switches it
    /// off** (the default), 0.5 is a film camera's 180° shutter, 1 smears across the whole frame
    /// interval. Only the camera's own motion blurs; objects moving in a still view do not (no
    /// per-object velocity exists). `#[serde(default)]`.
    pub motion_blur: f32,
}

impl Default for PostProcess {
    fn default() -> Self {
        Self {
            bloom_intensity: 0.8,
            bloom_threshold: 0.85,
            vignette: 0.0,
            chromatic_aberration: 0.0,
            dof_focus_dist: 4.5,
            dof_focus_range: 2.0,
            // Not the renderer's 4.0: `dof_enabled` is false by default and zeroes this, so 0.0 is
            // what a camera without the component actually gets.
            dof_blur_size: 0.0,
            dof_f_stops: 0.0,
            dof_sensor_height: 0.018_66,
            dof_max_depth: 0.0,
            film_grain: 0.012,
            motion_blur: 0.0,
        }
    }
}

impl PostProcess {
    /// The neutral look — identical to [`Default`], named for call sites that mean it.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builder: bloom strength and the luminance it starts at.
    pub fn with_bloom(mut self, intensity: f32, threshold: f32) -> Self {
        self.bloom_intensity = intensity.max(0.0);
        self.bloom_threshold = threshold.max(0.0);
        self
    }

    /// Builder: depth of field. A `blur_size` of 0 switches it off.
    ///
    /// The range is floored just above zero: a focus band of zero width makes every depth
    /// out of focus, which reads as a broken frame rather than as a very shallow one.
    pub fn with_depth_of_field(mut self, focus_dist: f32, focus_range: f32, blur_size: f32) -> Self {
        self.dof_focus_dist = focus_dist.max(0.0);
        self.dof_focus_range = focus_range.max(0.001);
        self.dof_blur_size = blur_size.max(0.0);
        self
    }

    /// Builder: depth of field through a lens. Focused at `focus_dist`, at this f-number, no blur larger
    /// than `max_blur` texels. How fast things leave focus follows the camera's field of view: a long
    /// lens is shallower, and so is a wider aperture, as on a real camera.
    pub fn with_lens(mut self, focus_dist: f32, f_stops: f32, max_blur: f32) -> Self {
        self.dof_focus_dist = focus_dist.max(0.0);
        self.dof_f_stops = f_stops.max(0.0);
        self.dof_blur_size = max_blur.max(0.0);
        self
    }

    /// The blur radius at infinity, in texels, for a frame this tall seen through this field of view: half
    /// the thin lens's circle of confusion, `f² |d − s| / (N d (s − f))`, without the part that depends on
    /// the distance `d`, which the shader has. 0 when this grade is not using a lens.
    #[must_use]
    pub fn dof_lens_factor(&self, fov: f32, frame_height: u32) -> f32 {
        if self.dof_f_stops <= 0.0 || self.dof_blur_size <= 0.0 {
            return 0.0;
        }
        let focal = 0.5 * self.dof_sensor_height / (fov * 0.5).tan();
        let focus = self.dof_focus_dist.max(focal * 1.01);
        0.5 * focal * focal / (self.dof_f_stops * (focus - focal)) / self.dof_sensor_height * frame_height as f32
    }

    /// Builder: the lens artefacts — corner darkening, channel separation, grain.
    pub fn with_grade(mut self, vignette: f32, chromatic_aberration: f32, film_grain: f32) -> Self {
        self.vignette = vignette.max(0.0);
        self.chromatic_aberration = chromatic_aberration.max(0.0);
        self.film_grain = film_grain.max(0.0);
        self
    }

    /// The same values, forced into ranges that cannot produce a broken frame.
    ///
    /// Inherited from the editor's `validate_post_process`, which clamped these while they were
    /// editor state: a slider dragged to an extreme could otherwise produce a frame that is all
    /// white or all NaN, and a hand-written scene file has no slider bounds at all. The ranges are
    /// the ones that were already being enforced, so a scene authored before this moved grades
    /// exactly as it did.
    ///
    /// Applied where a value arrives from outside — a panel edit, a loaded file — rather than
    /// every frame: this is validation, not a per-frame transform.
    pub fn clamped(mut self) -> Self {
        self.bloom_intensity = self.bloom_intensity.clamp(0.0, 5.0);
        self.bloom_threshold = self.bloom_threshold.clamp(0.0, 10.0);
        self.vignette = self.vignette.clamp(0.0, 1.0);
        self.chromatic_aberration = self.chromatic_aberration.clamp(0.0, 0.1);
        self.dof_focus_dist = self.dof_focus_dist.clamp(0.0, 10_000.0);
        self.dof_focus_range = self.dof_focus_range.clamp(0.001, 10_000.0);
        self.dof_blur_size = self.dof_blur_size.clamp(0.0, 32.0);
        self.film_grain = self.film_grain.clamp(0.0, 1.0);
        self
    }

    /// Does this grade do anything at all to the frame?
    ///
    /// Not used to skip the upload — the chain switches an effect off by zeroing its own scalar,
    /// never by not uploading — but it answers "is this camera graded" for tools and tests without
    /// them restating which fields count.
    pub fn is_neutral(&self) -> bool {
        *self == Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The invariant the whole design rests on**: adding this component to a camera must not
    /// change the picture. If the default ever drifts from what the engine does without the
    /// component, adding one from the ➕ menu becomes a surprise edit.
    #[test]
    fn the_default_is_the_no_component_behaviour() {
        let p = PostProcess::default();
        assert_eq!(p.bloom_intensity, 0.8, "the renderer's bloom_intensity");
        assert_eq!(p.bloom_threshold, 0.85, "the renderer's bloom_threshold");
        assert_eq!(p.chromatic_aberration, 0.0);
        assert_eq!(p.film_grain, 0.012, "the renderer's film_grain_intensity");
        assert_eq!(p.vignette, 0.0, "the uniform block's own default");
        assert_eq!(
            p.dof_blur_size, 0.0,
            "`dof_enabled` is false by default and zeroes the blur, so 0.0 — not the renderer's \
             4.0 — is what a camera without this component gets"
        );
        assert!(p.is_neutral());
    }

    /// The builders clamp the values that break the frame rather than trusting the caller: a
    /// negative blur and a zero focus band are both reachable from a slider.
    #[test]
    fn the_builders_refuse_the_values_that_break_a_frame() {
        let p = PostProcess::new().with_depth_of_field(-1.0, 0.0, -5.0);
        assert_eq!(p.dof_focus_dist, 0.0);
        assert!(p.dof_focus_range > 0.0, "a zero-width focus band puts every depth out of focus");
        assert_eq!(p.dof_blur_size, 0.0);

        let g = PostProcess::new().with_grade(-0.5, -0.1, -1.0);
        assert_eq!((g.vignette, g.chromatic_aberration, g.film_grain), (0.0, 0.0, 0.0));
    }

    /// The clamp inherited from the editor's `validate_post_process`, with the ranges it enforced.
    ///
    /// It moved here with the fields: a scene file has no slider bounds, so the rule cannot live
    /// in a panel any more.
    #[test]
    fn out_of_range_values_are_forced_back_into_a_frame_that_renders() {
        let mut p = PostProcess::new();
        p.bloom_intensity = -5.0;
        p.bloom_threshold = 999.0;
        p.vignette = 2.0;
        p.chromatic_aberration = 0.5;
        p.dof_focus_range = 0.0;
        p.film_grain = 7.0;

        let p = p.clamped();
        assert_eq!(p.bloom_intensity, 0.0);
        assert_eq!(p.bloom_threshold, 10.0);
        assert_eq!(p.vignette, 1.0);
        assert_eq!(p.chromatic_aberration, 0.1);
        assert!(p.dof_focus_range > 0.0, "a zero-width focus band blurs every depth");
        assert_eq!(p.film_grain, 1.0);
    }

    /// …and a grade already in range comes back untouched, including the default, which would
    /// otherwise be a clamp that quietly re-grades every ungraded camera.
    #[test]
    fn a_valid_grade_survives_the_clamp_unchanged() {
        assert_eq!(PostProcess::default().clamped(), PostProcess::default());
        let graded = PostProcess::new().with_grade(0.4, 0.01, 0.02);
        assert_eq!(graded.clamped(), graded);
    }

    /// A graded camera is not neutral — the half of `is_neutral` that a wrong `PartialEq` would
    /// otherwise hide.
    #[test]
    fn a_graded_camera_is_not_neutral() {
        assert!(!PostProcess::new().with_grade(0.4, 0.0, 0.012).is_neutral());
        assert!(!PostProcess::new().with_depth_of_field(4.5, 2.0, 3.0).is_neutral());
    }
}

#[cfg(test)]
mod lens_tests {
    use super::PostProcess;

    /// A 50 mm lens on Super 35, f/2, focused at 5 m: a point at 10 m has a circle of confusion 126 µm
    /// across, which on a 1080-line frame of an 18.66 mm sensor is a radius of 3.65 texels. Worked by hand.
    #[test]
    fn the_lens_factor_is_the_thin_lens() {
        let sensor = 0.018_66f32;
        let fov = 2.0 * (0.5 * sensor / 0.05).atan();
        let grade = PostProcess::new().with_lens(5.0, 2.0, 64.0);
        let k = grade.dof_lens_factor(fov, 1080);
        let at_ten = k * (10.0 - 5.0) / 10.0;
        assert!((at_ten - 3.65).abs() < 0.01, "{at_ten}");
        let stopped_down = PostProcess::new().with_lens(5.0, 8.0, 64.0).dof_lens_factor(fov, 1080);
        assert!((stopped_down - k / 4.0).abs() < 1e-3, "two stops are a quarter of the blur");
        let long = PostProcess::new().with_lens(5.0, 2.0, 64.0).dof_lens_factor(fov / 2.0, 1080);
        assert!(long > k * 3.5, "twice the focal length, four times the blur: {k} and {long}");
        assert_eq!(PostProcess::new().dof_lens_factor(fov, 1080), 0.0, "no lens asked for");
    }
}
