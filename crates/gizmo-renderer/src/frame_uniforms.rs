//! What goes into the per-frame uniform blocks, filled in one place.
//!
//! # Why this exists
//!
//! [`SceneUniforms`] is eighteen fields wide and [`PostProcessUniforms`] sixteen, and until this
//! module every draw loop filled them as an exhaustive struct literal of its own: the engine's
//! `default_render_pass`, `gizmo-studio`'s `execute_render_pipeline`, the renderer's own initial
//! buffers, and three demos with custom render callbacks. Six literals, one layout.
//!
//! An exhaustive literal looks like the safe option — leave a field out and it does not compile —
//! but the check it gives you is *"was every field filled?"*, and every one of them passed that.
//! The question worth asking is *"which field differs, and why?"*, and nothing was asking it:
//!
//! - **`PostProcessUniforms::cam_near`/`cam_far`** exist because DoF linearises depth with them,
//!   and a hardcoded 0.1/1000 miscalibrates the circle of confusion for any other far plane —
//!   that is what their comment in `gpu_types.rs` says. Five of the six sites hardcoded
//!   `0.1`/`2000.0` anyway. Only the engine path passed the real camera, so the editor viewport's
//!   DoF was miscalibrated for exactly the cameras the field was added for.
//! - **`SceneUniforms::cascade_params.x`** is documented in `common.wgsl` as the camera's z-near.
//!   Studio sent `cam_near`; the engine sent a literal `0.1`. They have disagreed for as long as
//!   both paths existed and nothing noticed, because *no shader reads `.x` today* — the drift was
//!   real and its cost was zero, which is the only reason it survived.
//! - **`SceneUniforms::exposure`** is likewise dead (`deferred_lighting.wgsl` says so in a comment:
//!   kept for layout stability, exposure is applied once in the post composite). The engine sent
//!   the camera's, studio sent `1.0`.
//!
//! Two dead fields drifting is not the problem. The problem is that a *live* field would have
//! drifted the same way and been just as invisible, and that the next field added to either block
//! has six literals to reach and no compiler complaint if it reaches five.
//!
//! So the derived work — the inverse view-projection, the packed `cascade_params` slots, the `w`
//! flags, the padding, the shadow-map texel size — happens here once, and the two paths' remaining
//! differences are values passed to one constructor. The ratchet test at the bottom of this file
//! keeps it that way: it scans the workspace rather than naming the files it knows about, so a
//! seventh call site is a test failure the day it is written.
//!
//! # What it does not do
//!
//! It does not merge the two render paths, and it does not decide that the values they pass *ought*
//! to agree. Studio still sends identity cascade matrices when the scene has no shadow-casting
//! light, still leaves point shadows off because its pass list records no cube, and still drives
//! exposure from the editor's slider instead of the camera. Those are choices, and after this they
//! read as choices — arguments at a call site with a reason next to them — instead of as two
//! literals nobody put side by side.

use crate::csm::{CASCADE_COUNT, SHADOW_MAP_RES};
use crate::gpu_types::{LightData, PostProcessUniforms, SceneUniforms};
use gizmo_math::{Mat4, Vec3};

/// Length of the shader's fixed light array, and therefore of [`SceneUniforms::lights`].
///
/// The shaders declare `lights: array<LightData, 10>`; anything that builds the array on the CPU
/// must agree with them, so the number lives here instead of being spelled `10` at each site.
pub const MAX_LIGHTS: usize = 10;

/// The active camera, as *both* uniform blocks need it.
///
/// One struct rather than two because the scene block and the post block disagreeing about which
/// camera they describe is a bug with no symptom until someone turns DoF on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraFrame {
    /// The view-projection actually used to rasterise this frame — **jittered**, when TAA is on.
    /// The unjittered matrix is TAA's business and does not belong in the scene block.
    pub view_proj: Mat4,
    /// World-space camera position.
    pub position: Vec3,
    /// Normalised world-space forward, used for view-depth cascade selection.
    pub forward: Vec3,
    /// Near plane. Feeds `cascade_params.x` and the DoF depth linearisation.
    pub near: f32,
    /// Far plane. Feeds the DoF depth linearisation.
    pub far: f32,
    /// The single exposure knob, applied once over the composited HDR in the post pass.
    pub exposure: f32,
}

impl Default for CameraFrame {
    /// The fallback both render paths already used when the world contains no camera: an identity
    /// view at the origin looking down -Z, 0.1..2000, neutral exposure.
    fn default() -> Self {
        Self {
            view_proj: Mat4::IDENTITY,
            position: Vec3::ZERO,
            forward: Vec3::new(0.0, 0.0, -1.0),
            near: 0.1,
            far: 2000.0,
            exposure: 1.0,
        }
    }
}

/// The directional light the scene is lit and shadowed by.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SunFrame {
    /// Direction the sun points along, normalised.
    pub direction: Vec3,
    /// rgb = colour, w = intensity.
    pub color: [f32; 4],
    /// Whether the scene actually has a sun. Reaches the shaders as `sun_direction.w`, which gates
    /// the whole sun branch *and* the cascade lookup — a scene with no sun that claims one pays for
    /// a shadow sample against cascades built around a placeholder down-vector.
    pub present: bool,
}

impl Default for SunFrame {
    fn default() -> Self {
        Self { direction: Vec3::new(0.0, -1.0, 0.0), color: [1.0, 1.0, 1.0, 0.0], present: false }
    }
}

/// Everything the shadow lookups read out of the scene block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowFrame {
    /// World → light clip space per cascade, in shadow-array layer order.
    pub cascade_view_projs: [Mat4; CASCADE_COUNT],
    /// Far distance along the camera forward for cascades 0..3; `w` is the far edge of the covered
    /// range, which is what the shaders fade the shadow term out over.
    pub cascade_splits: [f32; CASCADE_COUNT],
    /// Index into [`SceneFrame::lights`] of the one point light that owns the shadow cube, or
    /// `None` when nothing casts. Reaches the shader as `cascade_params.w` = index + 1.
    pub point_caster: Option<u32>,
    /// Whether the point-shadow cube was rendered this frame. A path that does not record the cube
    /// pass must leave this `false`: the lookup would otherwise sample whatever the cube held last.
    pub point_shadows_enabled: bool,
}

impl Default for ShadowFrame {
    fn default() -> Self {
        Self {
            cascade_view_projs: [Mat4::IDENTITY; CASCADE_COUNT],
            // The splits the renderer's initial buffer has always carried; overwritten by the
            // cascade helper on the first real frame.
            cascade_splits: [1.0, 10.0, 50.0, 500.0],
            point_caster: None,
            point_shadows_enabled: false,
        }
    }
}

/// Environment/debug state that is a property of the renderer rather than of the scene.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct EnvironmentFrame {
    /// Active environment preset, and the one being blended towards.
    pub preset: u32,
    /// Second preset of the blend pair.
    pub preset_2: u32,
    /// 0 = fully `preset`, 1 = fully `preset_2`.
    pub blend_t: f32,
    /// Debug shading mode (0 = normal shading).
    pub shading_mode: u32,
}

/// One frame's worth of scene state, in the terms the caller has it.
///
/// Build it with `..Default::default()` and fill what you actually know: a caller that has no sun,
/// no lights and no cascades says so by leaving them alone, rather than by hand-zeroing a ten-entry
/// light array — which is what the demos were doing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneFrame {
    pub camera: CameraFrame,
    pub sun: SunFrame,
    /// Point + spot lights, `num_lights` of them live.
    pub lights: [LightData; MAX_LIGHTS],
    pub num_lights: u32,
    pub shadows: ShadowFrame,
    pub environment: EnvironmentFrame,
    /// Seconds since start, driving fluid caustics and the SSGI sample rotation. Left at zero this
    /// is not "no animation" but *frozen* animation — both paths shipped that bug once.
    pub elapsed_time: f32,
}

impl Default for SceneFrame {
    fn default() -> Self {
        Self {
            camera: CameraFrame::default(),
            sun: SunFrame::default(),
            lights: [LightData::default(); MAX_LIGHTS],
            num_lights: 0,
            shadows: ShadowFrame::default(),
            environment: EnvironmentFrame::default(),
            elapsed_time: 0.0,
        }
    }
}

impl SceneUniforms {
    /// Lay one frame out for the GPU.
    ///
    /// Everything derived is derived here: the `w` flags, the inverse view-projection that the
    /// fullscreen unprojecting passes read instead of computing per fragment, the four
    /// `cascade_params` slots, and the padding that keeps the block's offsets where every shader's
    /// partial copy of the struct expects them.
    #[must_use]
    pub fn new(frame: &SceneFrame) -> Self {
        let SceneFrame { camera, sun, lights, num_lights, shadows, environment, elapsed_time } =
            frame;

        Self {
            view_proj: camera.view_proj.to_cols_array_2d(),
            camera_pos: camera.position.extend(1.0).to_array(),
            // w = "sun present". Hardcoding 1.0 here left the deferred shader evaluating the sun
            // branch plus a full cascade lookup in a scene with no sun.
            sun_direction: sun.direction.extend(if sun.present { 1.0 } else { 0.0 }).to_array(),
            sun_color: sun.color,
            lights: *lights,
            light_view_proj: shadows.cascade_view_projs.map(|m| m.to_cols_array_2d()),
            cascade_splits: shadows.cascade_splits,
            camera_forward: camera.forward.extend(0.0).to_array(),
            cascade_params: [
                camera.near,
                1.0 / SHADOW_MAP_RES as f32,
                *elapsed_time,
                shadows.point_caster.map_or(0.0, |i| (i + 1) as f32),
            ],
            num_lights: *num_lights,
            // Read by nothing today — the post composite owns exposure and `deferred_lighting.wgsl`
            // says as much next to the field it no longer reads. Fed from the camera in both paths
            // regardless, so a shader that starts reading it does not find two different answers.
            exposure: camera.exposure,
            _pre_align_pad: [0; 2],
            _align_pad: [0; 3],
            environment_blend_t: environment.blend_t,
            environment_preset: environment.preset,
            point_shadows_enabled: u32::from(shadows.point_shadows_enabled),
            environment_preset_2: environment.preset_2,
            shading_mode: environment.shading_mode,
            inv_view_proj: camera.view_proj.inverse().to_cols_array_2d(),
        }
    }
}

/// The fog a camera inside a fluid volume sees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnderwaterFog {
    /// Linear rgb.
    pub color: [f32; 3],
    pub density: f32,
}

impl Default for PostProcessUniforms {
    /// The renderer's own neutral look — the values its initial buffer has always carried. A caller
    /// that wants a different look overrides the knobs it cares about and inherits the rest, so a
    /// knob added later reaches every site without an edit.
    fn default() -> Self {
        Self {
            bloom_intensity: 0.8,
            // Below 1.0 on purpose: catches highlights, not just clipped pixels.
            bloom_threshold: 0.85,
            // Raised above 1.0 because ACES tone mapping otherwise reads flat.
            exposure: 1.15,
            // Off by default — the ghosting it adds is unpleasant in a fast-moving scene.
            chromatic_aberration: 0.0,
            vignette_intensity: 0.25,
            // Low: the grain hash is static, so a strong grain reads as a fixed pattern.
            film_grain_intensity: 0.012,
            dof_focus_dist: 15.0,
            dof_focus_range: 25.0,
            // Zero blur: thin lines (editor gizmos, debug draws) wash out before geometry does.
            dof_blur_size: 0.0,
            cam_near: 0.1,
            cam_far: 2000.0,
            underwater: 0.0,
            fog_r: 0.0,
            fog_g: 0.0,
            fog_b: 0.0,
            fog_density: 0.0,
        }
    }
}

impl PostProcessUniforms {
    /// Take the depth-linearisation pair from the active camera.
    ///
    /// DoF converts the depth buffer back to view distance with these; a site that hardcodes
    /// `0.1`/`2000.0` while its camera says otherwise focuses at the wrong distance — subtly for a
    /// near far plane, wildly for a far one. Five of the six call sites did.
    ///
    /// Exposure is deliberately *not* taken here even though [`CameraFrame`] carries it: the editor
    /// and the demos drive exposure from their own UI state, and making that an explicit assignment
    /// at those sites is the point. The engine sets it from the same camera, one line away.
    #[must_use]
    pub fn with_camera(mut self, camera: &CameraFrame) -> Self {
        self.cam_near = camera.near;
        self.cam_far = camera.far;
        self
    }

    /// Apply the fog of the fluid volume the camera sits in; `None` = the camera is in air.
    #[must_use]
    pub fn with_underwater(mut self, fog: Option<UnderwaterFog>) -> Self {
        match fog {
            Some(f) => {
                self.underwater = 1.0;
                self.fog_r = f.color[0];
                self.fog_g = f.color[1];
                self.fog_b = f.color[2];
                self.fog_density = f.density;
            }
            None => {
                self.underwater = 0.0;
                self.fog_r = 0.0;
                self.fog_g = 0.0;
                self.fog_b = 0.0;
                self.fog_density = 0.0;
            }
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_camera() -> CameraFrame {
        CameraFrame {
            view_proj: Mat4::perspective_rh(1.0, 1.6, 0.25, 900.0)
                * Mat4::look_at_rh(Vec3::new(3.0, 4.0, 5.0), Vec3::ZERO, Vec3::Y),
            position: Vec3::new(3.0, 4.0, 5.0),
            forward: Vec3::new(0.0, 0.0, -1.0),
            near: 0.25,
            far: 900.0,
            exposure: 1.4,
        }
    }

    /// The four `cascade_params` slots are a packing decision, and every one of them was wrong
    /// somewhere: `.x` was a literal 0.1 in the engine path, `.z` was a literal 0.0 in both (frozen
    /// water), `.w` was 0.0 in studio. Pin what each slot means.
    #[test]
    fn cascade_params_slots_carry_what_common_wgsl_says_they_do() {
        let frame = SceneFrame {
            camera: a_camera(),
            elapsed_time: 12.5,
            shadows: ShadowFrame { point_caster: Some(3), ..Default::default() },
            ..Default::default()
        };
        let u = SceneUniforms::new(&frame);
        assert_eq!(u.cascade_params[0], 0.25, "x = the camera's z-near, not a literal 0.1");
        assert_eq!(u.cascade_params[1], 1.0 / SHADOW_MAP_RES as f32, "y = PCF texel size");
        assert_eq!(u.cascade_params[2], 12.5, "z = elapsed time; 0.0 here freezes the water");
        assert_eq!(u.cascade_params[3], 4.0, "w = caster index + 1, so 0 can mean 'none'");

        let none = SceneUniforms::new(&SceneFrame::default());
        assert_eq!(none.cascade_params[3], 0.0, "no caster must encode as 0, not as light 0");
    }

    /// `sun_direction.w` is a flag, not a component. A scene with no sun that ships `w = 1.0` pays
    /// for the sun branch and a cascade lookup against placeholder matrices.
    #[test]
    fn a_scene_without_a_sun_says_so_in_the_w_component() {
        let dark = SceneUniforms::new(&SceneFrame::default());
        assert_eq!(dark.sun_direction[3], 0.0);

        let lit = SceneUniforms::new(&SceneFrame {
            sun: SunFrame { direction: Vec3::new(0.0, -1.0, 0.0), color: [1.0; 4], present: true },
            ..Default::default()
        });
        assert_eq!(lit.sun_direction[3], 1.0);
    }

    /// The inverse is computed here so the volumetric and particle passes can stop inverting a 4×4
    /// per fragment. It must be the inverse of the matrix actually written — the jittered one.
    #[test]
    fn inv_view_proj_inverts_the_matrix_that_was_written() {
        let u = SceneUniforms::new(&SceneFrame { camera: a_camera(), ..Default::default() });
        let vp = Mat4::from_cols_array_2d(&u.view_proj);
        let inv = Mat4::from_cols_array_2d(&u.inv_view_proj);
        let round_trip = vp * inv;
        for (i, col) in Mat4::IDENTITY.to_cols_array().iter().enumerate() {
            assert!(
                (round_trip.to_cols_array()[i] - col).abs() < 1e-3,
                "view_proj * inv_view_proj is not the identity: {round_trip:?}"
            );
        }
    }

    /// Padding is part of the layout, not slack: every shader's partial copy of the block indexes
    /// the fields after it by absolute offset.
    #[test]
    fn the_padding_is_zeroed_and_the_block_is_the_size_the_shaders_expect() {
        let u = SceneUniforms::new(&SceneFrame { camera: a_camera(), ..Default::default() });
        assert_eq!(u._pre_align_pad, [0; 2]);
        assert_eq!(u._align_pad, [0; 3]);
        assert_eq!(std::mem::size_of::<SceneUniforms>(), 1168);
        assert_eq!(std::mem::offset_of!(SceneUniforms, inv_view_proj), 1104);
        assert_eq!(std::mem::size_of_val(&u.lights) / std::mem::size_of::<LightData>(), MAX_LIGHTS);
    }

    /// A path that renders no point-shadow cube must not enable the lookup — the shader would
    /// sample whatever the cube held on the last frame that did.
    #[test]
    fn point_shadows_are_off_unless_the_caller_rendered_the_cube() {
        let off = SceneUniforms::new(&SceneFrame::default());
        assert_eq!(off.point_shadows_enabled, 0);
        let on = SceneUniforms::new(&SceneFrame {
            shadows: ShadowFrame { point_shadows_enabled: true, ..Default::default() },
            ..Default::default()
        });
        assert_eq!(on.point_shadows_enabled, 1);
    }

    /// The bug this constructor exists for, as an assertion: the post block's depth pair must be
    /// the camera's, because DoF's circle of confusion is computed from it.
    #[test]
    fn post_process_takes_its_depth_range_from_the_camera() {
        let p = PostProcessUniforms::default().with_camera(&a_camera());
        assert_eq!((p.cam_near, p.cam_far), (0.25, 900.0));
        assert_eq!(
            p.exposure,
            PostProcessUniforms::default().exposure,
            "with_camera must not touch exposure — the editor and the demos own that knob"
        );
    }

    #[test]
    fn underwater_fog_is_all_or_nothing() {
        let dry = PostProcessUniforms::default()
            .with_underwater(Some(UnderwaterFog { color: [0.1, 0.3, 0.4], density: 0.05 }))
            .with_underwater(None);
        assert_eq!((dry.underwater, dry.fog_r, dry.fog_density), (0.0, 0.0, 0.0));

        let wet = PostProcessUniforms::default()
            .with_underwater(Some(UnderwaterFog { color: [0.1, 0.3, 0.4], density: 0.05 }));
        assert_eq!(wet.underwater, 1.0);
        assert_eq!([wet.fog_r, wet.fog_g, wet.fog_b], [0.1, 0.3, 0.4]);
        assert_eq!(wet.fog_density, 0.05);
    }

    /// The ratchet.
    ///
    /// Six exhaustive literals is how the two blocks drifted, and adding a seventh is a five-line
    /// copy-paste that no compiler objects to. This walks the workspace's Rust sources and fails on
    /// any hand-filled literal of either block outside this module.
    ///
    /// It **scans** rather than checking a list of known files on purpose: the shader mirror tests
    /// in `gpu_types.rs` are the right idea with the wrong subject list — each names its files by
    /// hand, so a new shader is invisible to the test that exists to police it. A partial literal
    /// (`..Default::default()`) is fine and is the intended escape hatch; what is not fine is a site
    /// that enumerates every field, because that is the one that silently goes stale.
    #[test]
    fn no_hand_filled_uniform_literals_outside_the_constructor() {
        let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .expect("crates/gizmo-renderer sits two levels below the workspace root")
            .to_path_buf();
        if !workspace.join("crates/gizmo-studio").is_dir() {
            // Packaged crate rather than a workspace checkout — the other call sites are not here.
            return;
        }

        let mut sources = Vec::new();
        collect_rs_files(&workspace.join("crates"), &mut sources);
        collect_rs_files(&workspace.join("demo"), &mut sources);
        assert!(sources.len() > 100, "source walk found only {} files", sources.len());

        let this_file = std::path::Path::new(file!()).file_name().unwrap();
        let mut offenders = Vec::new();
        for path in sources {
            if path.file_name() == Some(this_file) {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            let lines: Vec<&str> = text.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") || line.contains("struct ") {
                    continue;
                }
                for block in ["SceneUniforms {", "PostProcessUniforms {"] {
                    if !line.contains(block) {
                        continue;
                    }
                    // Partial literals are the sanctioned form: `..Default::default()` fills
                    // whatever this site does not name, including fields added tomorrow.
                    let partial = lines[i..]
                        .iter()
                        .take_while(|l| !l.trim_start().starts_with('}'))
                        .any(|l| l.trim_start().starts_with(".."));
                    if !partial {
                        offenders.push(format!(
                            "{}:{} — exhaustive `{block}` literal",
                            path.strip_prefix(&workspace).unwrap_or(&path).display(),
                            i + 1
                        ));
                    }
                }
            }
        }

        assert!(
            offenders.is_empty(),
            "hand-filled uniform literals found — build them with `SceneUniforms::new(&SceneFrame \
             {{ .. }})` or `PostProcessUniforms::default()` so a field added later reaches every \
             call site:\n  {}",
            offenders.join("\n  ")
        );
    }

    fn collect_rs_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                collect_rs_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
}
