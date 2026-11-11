//! Does the WGSL see the layout Rust uploads? Asked of every shader that has an opinion.
//!
//! # Why this exists
//!
//! `common.wgsl` opens by declaring itself "the SINGLE source of truth for the scene uniform
//! layout, replacing the hand-copied duplicates that used to live in 20+ shaders and silently
//! drift apart". That is true of the shaders which `#import` it — and **seven shaders still
//! declare their own `SceneUniforms`**, because whether a shader shares the definition depends on
//! whether its Rust call site reached for `load_shader_composed` rather than `load_shader`.
//! Nothing in the shader says which it is. The seven are legitimate: each is a *prefix* of the
//! real block, truncated after the last field it reads, which is how a shader avoids declaring 1168
//! bytes to read a view-projection. What is not legitimate is that nothing checked the prefix.
//!
//! The tests that existed were the right shape with the wrong subject list. `gpu_types.rs`'s
//! `gpu_struct_sizes_match_the_shader_layout` calls itself "a contract with the WGSL side" and
//! never opens a `.wgsl` file — it pins the Rust sizes and hopes. `every_instance_shader_declares_
//! the_full_struct` does read the shaders, but from a hand-written list of ten, and it counts
//! `vec4<f32>` occurrences rather than checking where the fields land. A shader added tomorrow is
//! invisible to both.
//!
//! So these tests take their subjects from the **directory**, and their answer from **naga**: each
//! shader that declares one of the mirrored structs is parsed, and the byte offset naga computes
//! for every named field is compared against `offset_of!` on the Rust struct that fills it. That
//! also closes, at the level that mattered, the gap recorded as "`compose_wgsl` builds a
//! `naga::Module`, validates it and throws it away, so Rust cannot compare its own structs with
//! the layout the shader sees" — it can now; it just parses the source itself.
//!
//! # What it does not check
//!
//! Types, only offsets and order — `gbuffer.wgsl` deliberately declares the light array as
//! `array<vec4<f32>, 40>` (it never reads a light, and 40 vec4s is the same 640 bytes as 10
//! `LightData`), and an offset check accepts that while still catching a field that moved. Padding
//! fields (`_`-prefixed) are ignored by name for the same reason: their names are local inventions,
//! and their effect is entirely visible in where the *named* fields land.

use crate::gpu_types::{InstanceRaw, LightData, SceneUniforms};
use std::mem::offset_of;
use std::path::{Path, PathBuf};

/// Every `.wgsl` in the shader directory, taken from the directory rather than from a list.
fn all_shaders() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/shaders");
    let mut out: Vec<(String, String)> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p: &PathBuf| p.extension().is_some_and(|x| x == "wgsl"))
        .map(|p| {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            (name, std::fs::read_to_string(&p).expect("shader is readable"))
        })
        .collect();
    out.sort();
    assert!(out.len() > 40, "shader walk found only {} files", out.len());
    out
}

/// Parse one shader for its type declarations.
///
/// Most of these shaders are plain WGSL once the line-oriented naga_oil directives are dropped, and
/// naga's front end takes them as they are. The composed ones substitute bind-group indices inline
/// (`@group(#{INSTANCE_GROUP})`) and only naga_oil can resolve that, so they go through the real
/// composition path — which is the same module the pipeline would hand to wgpu.
fn struct_members(shader: &str, src: &str, name_prefix: &str) -> Vec<(String, Vec<(String, u32)>, u32)> {
    // Cheap pre-filter: parsing 50 shaders to look at 17 is wasted work, and the filter is still
    // "what the file says" rather than a list of file names.
    if !src.contains(&format!("struct {name_prefix}")) {
        return Vec::new();
    }

    let stripped: String = src
        .lines()
        .map(|l| if l.trim_start().starts_with('#') { "" } else { l })
        .collect::<Vec<_>>()
        .join("\n");
    let module = match naga::front::wgsl::parse_str(&stripped) {
        Ok(m) => m,
        Err(plain) => crate::pipeline::shaders::compose_module(
            src,
            shader,
            crate::pipeline::shaders::native_render_defs(),
        )
        .unwrap_or_else(|composed| {
            panic!(
                "{shader} declares `{name_prefix}` but could not be read either way, so nothing \
                 checks its layout.\n  as plain WGSL: {plain}\n  composed: {composed}"
            )
        })
        .0,
    };

    let mut found = Vec::new();
    for (_, ty) in module.types.iter() {
        let Some(ty_name) = ty.name.as_deref() else { continue };
        if !ty_name.starts_with(name_prefix) {
            continue;
        }
        if let naga::TypeInner::Struct { members, span } = &ty.inner {
            let fields = members
                .iter()
                .filter_map(|m| m.name.clone().map(|n| (n, m.offset)))
                .collect();
            found.push((ty_name.to_string(), fields, *span));
        }
    }
    found
}

/// What Rust uploads, by the name the WGSL uses for it.
///
/// Derived from `offset_of!` rather than written down: a field that moves in `gpu_types.rs` moves
/// here too, which is the only way this stays a check on the shaders instead of a second thing to
/// keep in sync.
fn scene_layout() -> Vec<(&'static str, u32)> {
    vec![
        ("view_proj", offset_of!(SceneUniforms, view_proj) as u32),
        ("camera_pos", offset_of!(SceneUniforms, camera_pos) as u32),
        ("sun_direction", offset_of!(SceneUniforms, sun_direction) as u32),
        ("sun_color", offset_of!(SceneUniforms, sun_color) as u32),
        ("lights", offset_of!(SceneUniforms, lights) as u32),
        ("light_view_proj", offset_of!(SceneUniforms, light_view_proj) as u32),
        ("cascade_splits", offset_of!(SceneUniforms, cascade_splits) as u32),
        ("camera_forward", offset_of!(SceneUniforms, camera_forward) as u32),
        ("cascade_params", offset_of!(SceneUniforms, cascade_params) as u32),
        ("num_lights", offset_of!(SceneUniforms, num_lights) as u32),
        ("exposure", offset_of!(SceneUniforms, exposure) as u32),
        ("wireframe_color", offset_of!(SceneUniforms, wireframe_color) as u32),
        ("clear_color", offset_of!(SceneUniforms, clear_color) as u32),
        ("environment_blend_t", offset_of!(SceneUniforms, environment_blend_t) as u32),
        ("environment_preset", offset_of!(SceneUniforms, environment_preset) as u32),
        ("_pad_shadow_flag", offset_of!(SceneUniforms, _pad_shadow_flag) as u32),
        // naga_oil reserves the `_<number>` suffix for naga's name mangling, so the WGSL field
        // cannot be called `environment_preset_2`. Only the byte offset has to agree.
        ("environment_preset_b", offset_of!(SceneUniforms, environment_preset_2) as u32),
        ("shading_mode", offset_of!(SceneUniforms, shading_mode) as u32),
        ("inv_view_proj", offset_of!(SceneUniforms, inv_view_proj) as u32),
        ("cluster_dims", offset_of!(SceneUniforms, cluster_dims) as u32),
        ("cluster_depth", offset_of!(SceneUniforms, cluster_depth) as u32),
        ("fog", offset_of!(SceneUniforms, fog) as u32),
        ("fog_shape", offset_of!(SceneUniforms, fog_shape) as u32),
        ("shadow_bias", offset_of!(SceneUniforms, shadow_bias) as u32),
        ("sky_zenith", offset_of!(SceneUniforms, sky_zenith) as u32),
        ("sky_horizon", offset_of!(SceneUniforms, sky_horizon) as u32),
        ("sky_ground", offset_of!(SceneUniforms, sky_ground) as u32),
        ("local_shadow_layers", offset_of!(SceneUniforms, local_shadow_layers) as u32),
        ("light_shadows", offset_of!(SceneUniforms, light_shadows) as u32),
    ]
}

/// The instance buffer's element layout, in the names the shaders give it. The model matrix is
/// four separate vec4 attributes in WGSL and one `[[f32; 4]; 4]` in Rust, so its columns are
/// derived from the Rust field's offset rather than written down.
fn instance_layout() -> Vec<(&'static str, u32)> {
    let model = offset_of!(InstanceRaw, model) as u32;
    vec![
        ("model_matrix_0", model),
        ("model_matrix_1", model + 16),
        ("model_matrix_2", model + 32),
        ("model_matrix_3", model + 48),
        ("albedo_color", offset_of!(InstanceRaw, albedo_color) as u32),
        // One packed vec4 on the shader side: x=roughness, y=metallic, z=unlit, w=padding.
        ("pbr", offset_of!(InstanceRaw, roughness) as u32),
        ("ambient", offset_of!(InstanceRaw, ambient) as u32),
        ("emissive", offset_of!(InstanceRaw, emissive) as u32),
        ("optics", offset_of!(InstanceRaw, optics) as u32),
    ]
}

/// Compare one shader's declaration against the Rust truth, and describe every disagreement.
///
/// A declaration may stop early (that is what a partial copy is) but may not reorder, rename or
/// re-place a field it does declare, and may not run past the end of the Rust struct.
fn check(
    shader: &str,
    struct_name: &str,
    fields: &[(String, u32)],
    span: u32,
    truth: &[(&str, u32)],
    rust_size: usize,
    problems: &mut Vec<String>,
) {
    if span as usize > rust_size {
        problems.push(format!(
            "{shader}: `{struct_name}` is {span} bytes, the Rust struct is {rust_size} — the \
             shader reads past the end of the buffer"
        ));
    }
    let mut last_truth_index = None;
    for (name, offset) in fields {
        // Padding names are local inventions; where the named fields land already proves what the
        // padding did.
        if name.starts_with('_') {
            continue;
        }
        let Some(index) = truth.iter().position(|(t, _)| t == name) else {
            problems.push(format!(
                "{shader}: `{struct_name}.{name}` is not a field of the Rust struct — a rename on \
                 one side only, or a field the CPU never uploads"
            ));
            continue;
        };
        if truth[index].1 != *offset {
            problems.push(format!(
                "{shader}: `{struct_name}.{name}` sits at byte {offset}, Rust puts it at {} — this \
                 shader reads a different field than the one it names",
                truth[index].1
            ));
        }
        if let Some(prev) = last_truth_index {
            if index <= prev {
                problems.push(format!(
                    "{shader}: `{struct_name}.{name}` appears out of order against the Rust struct"
                ));
            }
        }
        last_truth_index = Some(index);
    }
}

/// The scene block, as declared by every shader that declares it.
///
/// This is the drift the module docs describe: seven prefix copies plus the source of truth, none
/// of them previously compared to the bytes Rust actually uploads.
#[test]
fn every_scene_uniform_declaration_matches_the_bytes_rust_uploads() {
    let truth = scene_layout();
    let mut problems = Vec::new();
    let mut checked = Vec::new();

    for (shader, src) in all_shaders() {
        for (struct_name, fields, span) in struct_members(&shader, &src, "SceneUniforms") {
            checked.push(shader.clone());
            check(
                &shader,
                &struct_name,
                &fields,
                span,
                &truth,
                std::mem::size_of::<SceneUniforms>(),
                &mut problems,
            );
        }
    }

    assert!(
        checked.contains(&"common.wgsl".to_string()),
        "the source of truth was not among the shaders checked — the walk found {checked:?}"
    );
    assert!(
        checked.len() >= 6,
        "only {} shaders declare a scene block; the copies were expected too: {checked:?}",
        checked.len()
    );
    assert!(problems.is_empty(), "scene uniform layout disagreements:\n  {}", problems.join("\n  "));
}

/// `common.wgsl` says it is the single source of truth. It only is if it is *complete* — a prefix
/// copy that stops early is a copy, but the source of truth stopping early means every importing
/// shader is missing the tail.
#[test]
fn the_shared_scene_declaration_is_the_whole_block() {
    let src = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/shaders/common.wgsl"),
    )
    .expect("common.wgsl");
    let declarations = struct_members("common.wgsl", &src, "SceneUniforms");
    let (_, fields, span) = declarations.first().expect("common.wgsl declares SceneUniforms");

    assert_eq!(
        *span as usize,
        std::mem::size_of::<SceneUniforms>(),
        "common.wgsl's block is {span} bytes and Rust's is {} — the shared declaration is not the \
         whole block, so every importing shader is short",
        std::mem::size_of::<SceneUniforms>()
    );
    for (name, want) in scene_layout() {
        let got = fields.iter().find(|(n, _)| n == name);
        assert_eq!(
            got.map(|(_, o)| *o),
            Some(want),
            "common.wgsl is missing `{name}` (or has it at the wrong offset); Rust puts it at {want}"
        );
    }
}

/// The instance buffer's element stride comes from the Rust struct, and every shader that indexes
/// `instances[i]` re-declares the element with its own idea of that stride. Miss one field in one
/// shader and every instance after the first reads shifted memory — silently, in that one pipeline.
///
/// Replaces a hand-listed count of `vec4<f32>` occurrences with the offsets naga computes, over
/// whatever shaders are in the directory today.
#[test]
fn every_instance_declaration_matches_the_bytes_rust_uploads() {
    let truth = instance_layout();
    let mut problems = Vec::new();
    let mut checked = Vec::new();

    for (shader, src) in all_shaders() {
        // The two names the same struct goes by in the shaders.
        for prefix in ["InstanceRaw", "InstanceData"] {
            for (struct_name, fields, span) in struct_members(&shader, &src, prefix) {
                checked.push(shader.clone());
                let named = fields.iter().filter(|(n, _)| !n.starts_with('_')).count();
                if named != truth.len() {
                    problems.push(format!(
                        "{shader}: `{struct_name}` declares {named} instance fields, Rust has {} \
                         — `instances[i]` reads the wrong offsets for every i > 0",
                        truth.len()
                    ));
                }
                check(
                    &shader,
                    &struct_name,
                    &fields,
                    span,
                    &truth,
                    std::mem::size_of::<InstanceRaw>(),
                    &mut problems,
                );
            }
        }
    }

    assert!(
        checked.len() >= 10,
        "only {} shaders declare the instance element; the walk should have found the whole \
         instanced set: {checked:?}",
        checked.len()
    );
    assert!(problems.is_empty(), "instance layout disagreements:\n  {}", problems.join("\n  "));
}

/// The light array's element, same rule. It is the field the prefix copies most often carry
/// without reading, and a wrong element size there moves everything after it.
#[test]
fn every_light_declaration_matches_the_bytes_rust_uploads() {
    let truth = vec![
        ("position", offset_of!(LightData, position) as u32),
        ("color", offset_of!(LightData, color) as u32),
        ("direction", offset_of!(LightData, direction) as u32),
        ("params", offset_of!(LightData, params) as u32),
    ];
    let mut problems = Vec::new();
    let mut checked = 0;

    for (shader, src) in all_shaders() {
        for (struct_name, fields, span) in struct_members(&shader, &src, "LightData") {
            checked += 1;
            if span as usize != std::mem::size_of::<LightData>() {
                problems.push(format!(
                    "{shader}: `{struct_name}` is {span} bytes against Rust's {} — the light array \
                     stride is wrong, which moves every field after `lights`",
                    std::mem::size_of::<LightData>()
                ));
            }
            check(
                &shader,
                &struct_name,
                &fields,
                span,
                &truth,
                std::mem::size_of::<LightData>(),
                &mut problems,
            );
        }
    }

    assert!(checked >= 2, "only {checked} shaders declare LightData");
    assert!(problems.is_empty(), "light layout disagreements:\n  {}", problems.join("\n  "));
}

/// **A shadow pass's uniform**, as `shadow.wgsl` declares it: the light's matrix, and since
/// 2026-11-11 the frame's camera and clock, which a cut-out's stage hands a game's surface. A field
/// read from the wrong offset would cut a dissolving surface's shadow at the wrong time with nothing
/// to say so.
#[test]
fn every_shadow_pass_declaration_matches_the_bytes_rust_uploads() {
    use crate::gpu_types::ShadowVsUniform;
    let truth = vec![
        ("light_view_proj", offset_of!(ShadowVsUniform, light_view_proj) as u32),
        ("camera_position", offset_of!(ShadowVsUniform, camera_position) as u32),
        ("params", offset_of!(ShadowVsUniform, params) as u32),
    ];
    let size = std::mem::size_of::<ShadowVsUniform>();
    let mut problems = Vec::new();
    let mut checked = Vec::new();
    for (shader, src) in all_shaders() {
        for (struct_name, fields, span) in struct_members(&shader, &src, "ShadowVsUniform") {
            check(&shader, &struct_name, &fields, span, &truth, size, &mut problems);
            checked.push(shader.clone());
        }
    }
    assert!(checked.iter().any(|s| s.contains("shadow.wgsl")), "`shadow.wgsl` no longer declares `ShadowVsUniform`: {checked:?}");
    assert!(problems.is_empty(), "shadow pass layout disagreements:\n  {}", problems.join("\n  "));
}

/// **The GPU bodies' structs**, as the solver, the culling pass and the debug lines declare them —
/// by exact name, since `fem_compute.wgsl` declares a `GpuCollider` of its own for another system.
///
/// None of them was checked, and one was wrong from the day it was written: the solver's
/// `StaticCollider` padded its type with a `vec3<u32>`, which WGSL aligns to 16 bytes, so the shader
/// saw a 64-byte collider with its shape at byte 32 where Rust uploads 48 bytes with the shape at 16.
/// Every static collider read the next one's bytes: a plane's normal came out (0, 0, 0), and bodies
/// fell through the ground they were given.
#[test]
fn every_gpu_body_declaration_matches_the_bytes_rust_uploads() {
    use crate::gpu_physics::{GpuBox, GpuBoxContacts, GpuCollider, GpuJoint, PhysicsSimParams};
    use std::mem::size_of;

    let subjects: Vec<(&str, Vec<(&str, u32)>, usize)> = vec![
        (
            "StaticCollider",
            vec![
                ("shape_type", offset_of!(GpuCollider, shape_type) as u32),
                ("data1", offset_of!(GpuCollider, data1) as u32),
                ("data2", offset_of!(GpuCollider, data2) as u32),
            ],
            size_of::<GpuCollider>(),
        ),
        (
            "BoxItem",
            vec![
                ("position", offset_of!(GpuBox, position) as u32),
                ("mass", offset_of!(GpuBox, mass) as u32),
                ("velocity", offset_of!(GpuBox, velocity) as u32),
                ("state", offset_of!(GpuBox, state) as u32),
                ("rotation", offset_of!(GpuBox, rotation) as u32),
                ("angular_velocity", offset_of!(GpuBox, angular_velocity) as u32),
                ("sleep_counter", offset_of!(GpuBox, sleep_counter) as u32),
                ("color", offset_of!(GpuBox, color) as u32),
                ("half_extents", offset_of!(GpuBox, half_extents) as u32),
            ],
            size_of::<GpuBox>(),
        ),
        (
            "BoxContacts",
            vec![
                ("count", offset_of!(GpuBoxContacts, count) as u32),
                ("neighbors", offset_of!(GpuBoxContacts, neighbors) as u32),
                ("normals", offset_of!(GpuBoxContacts, normals) as u32),
                ("accum_impulse", offset_of!(GpuBoxContacts, accum_impulse) as u32),
                ("is_active", offset_of!(GpuBoxContacts, is_active) as u32),
                ("approach", offset_of!(GpuBoxContacts, approach) as u32),
            ],
            size_of::<GpuBoxContacts>(),
        ),
        (
            "Joint",
            vec![
                ("body_a", offset_of!(GpuJoint, body_a) as u32),
                ("body_b", offset_of!(GpuJoint, body_b) as u32),
                ("joint_type", offset_of!(GpuJoint, joint_type) as u32),
                ("flags", offset_of!(GpuJoint, flags) as u32),
                ("anchor_a", offset_of!(GpuJoint, anchor_a) as u32),
                ("compliance", offset_of!(GpuJoint, compliance) as u32),
                ("anchor_b", offset_of!(GpuJoint, anchor_b) as u32),
                ("damping_coeff", offset_of!(GpuJoint, damping) as u32),
                ("axis", offset_of!(GpuJoint, axis) as u32),
                ("max_force", offset_of!(GpuJoint, max_force) as u32),
            ],
            size_of::<GpuJoint>(),
        ),
        (
            "SimParams",
            vec![
                ("dt", offset_of!(PhysicsSimParams, dt) as u32),
                ("gravity", offset_of!(PhysicsSimParams, gravity) as u32),
                ("damping", offset_of!(PhysicsSimParams, damping) as u32),
                ("num_boxes", offset_of!(PhysicsSimParams, num_boxes) as u32),
                ("num_colliders", offset_of!(PhysicsSimParams, num_colliders) as u32),
                ("num_joints", offset_of!(PhysicsSimParams, num_joints) as u32),
            ],
            size_of::<PhysicsSimParams>(),
        ),
    ];

    let mut problems = Vec::new();
    let mut checked = Vec::new();
    for (shader, src) in all_shaders() {
        // Only the GPU bodies' shaders: other systems have their own `SimParams` and `Joint`.
        if !shader.starts_with("physics_") {
            continue;
        }
        for (name, truth, rust_size) in &subjects {
            for (struct_name, fields, span) in struct_members(&shader, &src, name) {
                if struct_name != *name {
                    continue;
                }
                check(&shader, &struct_name, &fields, span, truth, *rust_size, &mut problems);
                // An array element's stride is its span rounded up to its alignment; a span that
                // differs at all is a stride that differs once the element is in an array.
                if span as usize != *rust_size {
                    problems.push(format!(
                        "{shader}: `{struct_name}` is {span} bytes, the Rust struct is {rust_size} — \
                         in an array every element after the first is read from the wrong place"
                    ));
                }
                checked.push(format!("{shader}:{struct_name}"));
            }
        }
    }
    for name in ["StaticCollider", "BoxItem", "BoxContacts", "Joint", "SimParams"] {
        assert!(checked.iter().any(|c| c.ends_with(&format!(":{name}"))), "no shader declares `{name}` any more: {checked:?}");
    }
    assert!(problems.is_empty(), "GPU body layout disagreements:\n  {}", problems.join("\n  "));
}
