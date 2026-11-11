//! Shader-loading helpers (WGSL module composition through naga_oil, plus the native/web
//! scheme).

use naga_oil::compose::ShaderDefValue;
use std::collections::HashMap;

/// The shared shader library (`#define_import_path gizmo::common`), embedded once and
/// registered with the naga_oil composer so any shader can `#import gizmo::common::{...}`.
const COMMON_WGSL: &str = include_str!("../shaders/common.wgsl");

/// Deferred-specific PBR extensions (`#define_import_path gizmo::pbr_ext`): anisotropic GGX,
/// clear-coat, and the Lazarov env-BRDF LUT. Imports `gizmo::common`, so it is registered
/// AFTER it below.
const PBR_EXT_WGSL: &str = include_str!("../shaders/pbr_ext.wgsl");
/// What a surface is at a fragment, and what a fragment is handed to work it out
/// (`#define_import_path gizmo::surface`): types and functions, no bindings.
const SURFACE_WGSL: &str = include_str!("../shaders/surface.wgsl");
/// The engine's own material (`#define_import_path gizmo::material`): group 1 as
/// `AssetManager::material` fills it, evaluated into a surface. Shared by the G-buffer and the forward
/// shader, and what a game's surface can start from.
const MATERIAL_WGSL: &str = include_str!("../shaders/material.wgsl");

/// The import path a game's surface module is composed under — see
/// [`SurfaceMaterial`](crate::custom_material::SurfaceMaterial). `gbuffer.wgsl` and `shader.wgsl`
/// import `material_surface` from it when `CUSTOM_SURFACE` is defined.
pub(crate) const CUSTOM_SURFACE_MODULE: &str = "gizmo::custom_surface";

/// Irradiance-volume sampling (`#define_import_path gizmo::irradiance`): the WGSL port of
/// `gi::ProbeGrid::sample` and `gi::SHCoeffs::evaluate`. Only `deferred_lighting.wgsl` imports it,
/// and it declares bind group 3, so registering it here is inert for every other shader.
const IRRADIANCE_WGSL: &str = include_str!("../shaders/irradiance.wgsl");

/// Reflection-probe sampling (`#define_import_path gizmo::reflection_probes`): the cube array and
/// `probe_environment`, at bind group `#{REFLECTION_GROUP}`. Imported by the deferred lighting
/// pass (group 3) and, on native, by the forward pass (group 5).
const REFLECTION_PROBES_WGSL: &str = include_str!("../shaders/reflection_probes.wgsl");

/// Shader-defs for the NATIVE render schema: 5 bind groups with the CSM shadow group in the
/// middle. `SHADOWS` keeps the `#ifdef SHADOWS` shadow bindings + PCF block; the group
/// indices place skeleton at 3 and instance at 4 (see build_core_pipelines' native layout).
pub(crate) fn native_render_defs() -> HashMap<String, ShaderDefValue> {
    HashMap::from([
        ("SHADOWS".to_string(), ShaderDefValue::Bool(true)),
        ("SKELETON_GROUP".to_string(), ShaderDefValue::UInt(3)),
        ("INSTANCE_GROUP".to_string(), ShaderDefValue::UInt(4)),
        // The forward pass's sixth group (the native device asks for six): the irradiance
        // volume's group, for its reflection probes. The web has no group left, so no `PROBES`.
        ("PROBES".to_string(), ShaderDefValue::Bool(true)),
        ("REFLECTION_GROUP".to_string(), ShaderDefValue::UInt(5)),
        ("REFLECTION_TEXTURE_BINDING".to_string(), ShaderDefValue::UInt(2)),
        ("REFLECTION_SAMPLER_BINDING".to_string(), ShaderDefValue::UInt(3)),
        ("REFLECTION_UNIFORM_BINDING".to_string(), ShaderDefValue::UInt(4)),
        // The baked irradiance grid, bindings 0–1 of that same group.
        ("IRRADIANCE".to_string(), ShaderDefValue::Bool(true)),
        // Back faces, cut-outs, LOD crossfades and a material's own reflectance, then anisotropy, clear
        // coat and subsurface, in `shader.wgsl`. Always on here; the forward-only path builds its lit
        // pipelines in tiers, so a plain surface does not pay for them. See `SceneState::lit_pipeline`.
        ("DETAILS".to_string(), ShaderDefValue::Bool(true)),
        // A cut-out or a LOD crossfade in `unlit.wgsl`, for the same reason.
        ("KEEPS".to_string(), ShaderDefValue::Bool(true)),
        ("LOBES".to_string(), ShaderDefValue::Bool(true)),
    ])
}

/// Shader-defs for the WEB render schema: WebGPU caps bind groups at 4, so the shadow group
/// is dropped (no `SHADOWS`) and skeleton/instance shift down to 2/3. This REPLACES the old
/// `load_shader_web` text surgery — the divergence is now expressed in the shader source via
/// naga_oil `#ifdef`/`#{...}` (robust) instead of grepping naga's reformatted WGSL output
/// (which was empirically broken: naga rewrites `if (scene.sun_direction.w > 0.5)` to
/// `if (_eNN > 0.5f)`, so the block-strip silently no-oped while the binding-strip still
/// fired → undefined `t_shadow`/`s_shadow`).
///
/// Not cfg-gated to wasm: `core_shaders_compile` composes the web-path shaders under these
/// defs on native too, so the web variant's validity is verified without a browser.
#[allow(dead_code)] // used on wasm (load_shader_composed_web) and by the native validation test
pub(crate) fn web_render_defs() -> HashMap<String, ShaderDefValue> {
    HashMap::from([
        ("SKELETON_GROUP".to_string(), ShaderDefValue::UInt(2)),
        ("INSTANCE_GROUP".to_string(), ShaderDefValue::UInt(3)),
        // The environment from a cube drawn when it changes (`env_cube`), not worked out per pixel.
        ("ENV_CUBE".to_string(), ShaderDefValue::Bool(true)),
        // Reflection probes, for the lit pipelines built with `PROBES` while a scene has some: group 0's
        // bindings 6–8, since this path has no group of its own to give them.
        ("REFLECTION_GROUP".to_string(), ShaderDefValue::UInt(0)),
        ("REFLECTION_TEXTURE_BINDING".to_string(), ShaderDefValue::UInt(6)),
        ("REFLECTION_SAMPLER_BINDING".to_string(), ShaderDefValue::UInt(7)),
        ("REFLECTION_UNIFORM_BINDING".to_string(), ShaderDefValue::UInt(8)),
    ])
}

/// Compose and validate a shader source, returning the `naga::Module` itself.
///
/// This used to be inlined in [`compose_wgsl`], which built the module, validated it, and dropped
/// it on the floor — so the only thing the rest of the crate could see was text, and Rust had no
/// way to ask what layout the shader actually ended up with. `crate::shader_contract` asks exactly
/// that, and this is how. The pipelines go through [`try_compose_wgsl`], so only the tests call it.
#[cfg(test)]
pub(crate) fn compose_module(
    source: &str,
    label: &str,
    shader_defs: HashMap<String, ShaderDefValue>,
) -> Result<(naga::Module, naga::valid::ModuleInfo), String> {
    compose_module_with(source, label, shader_defs, None)
}

/// [`compose_module`], with a game's surface module registered as [`CUSTOM_SURFACE_MODULE`] for the
/// shader to import. `None` is the engine's shaders as they are.
pub(crate) fn compose_module_with(
    source: &str,
    label: &str,
    shader_defs: HashMap<String, ShaderDefValue>,
    custom_surface: Option<&str>,
) -> Result<(naga::Module, naga::valid::ModuleInfo), String> {
    use naga_oil::compose::{
        ComposableModuleDescriptor, Composer, NagaModuleDescriptor, ShaderLanguage,
    };

    let mut composer = Composer::default();
    composer
        .add_composable_module(ComposableModuleDescriptor {
            source: COMMON_WGSL,
            file_path: "gizmo/common.wgsl",
            language: ShaderLanguage::Wgsl,
            ..Default::default()
        })
        .map_err(|e| format!("composing common.wgsl failed: {e}"))?;
    // pbr_ext imports gizmo::common, so it must be registered after common. Only shaders that
    // `#import gizmo::pbr_ext` pull it in; registering it here is otherwise inert.
    composer
        .add_composable_module(ComposableModuleDescriptor {
            source: PBR_EXT_WGSL,
            file_path: "gizmo/pbr_ext.wgsl",
            language: ShaderLanguage::Wgsl,
            ..Default::default()
        })
        .map_err(|e| format!("composing pbr_ext.wgsl failed: {e}"))?;
    composer
        .add_composable_module(ComposableModuleDescriptor {
            source: SURFACE_WGSL,
            file_path: "gizmo/surface.wgsl",
            language: ShaderLanguage::Wgsl,
            ..Default::default()
        })
        .map_err(|e| format!("composing surface.wgsl failed: {e}"))?;
    // After `surface`, which it imports.
    composer
        .add_composable_module(ComposableModuleDescriptor {
            source: MATERIAL_WGSL,
            file_path: "gizmo/material.wgsl",
            language: ShaderLanguage::Wgsl,
            ..Default::default()
        })
        .map_err(|e| format!("composing material.wgsl failed: {e}"))?;
    composer
        .add_composable_module(ComposableModuleDescriptor {
            source: IRRADIANCE_WGSL,
            file_path: "gizmo/irradiance.wgsl",
            language: ShaderLanguage::Wgsl,
            ..Default::default()
        })
        .map_err(|e| format!("composing irradiance.wgsl failed: {e}"))?;
    composer
        .add_composable_module(ComposableModuleDescriptor {
            source: REFLECTION_PROBES_WGSL,
            file_path: "gizmo/reflection_probes.wgsl",
            language: ShaderLanguage::Wgsl,
            ..Default::default()
        })
        .map_err(|e| format!("composing reflection_probes.wgsl failed: {e}"))?;
    // Last: it may import anything above. Named by the engine, so a game's file needs no
    // `#define_import_path` and cannot register itself over one of the engine's modules.
    if let Some(surface) = custom_surface {
        composer
            .add_composable_module(ComposableModuleDescriptor {
                source: surface,
                file_path: label,
                language: ShaderLanguage::Wgsl,
                as_name: Some(CUSTOM_SURFACE_MODULE.to_string()),
                ..Default::default()
            })
            .map_err(|e| format!("composing the surface of '{label}' failed: {e}"))?;
    }

    let module = composer
        .make_naga_module(NagaModuleDescriptor {
            source,
            file_path: label,
            shader_defs,
            ..Default::default()
        })
        .map_err(|e| format!("naga_oil compose of '{label}' failed: {e}"))?;

    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map_err(|e| format!("validating composed '{label}' failed: {e:?}"))?;

    Ok((module, info))
}

/// Compose a shader source (`#import gizmo::common`, `#ifdef`, `#{DEF}`) into flat WGSL text
/// under the given `shader_defs`.
///
/// We resolve with naga_oil then emit WGSL (rather than handing wgpu a `naga::Module`)
/// because wgpu 29 here is built without the `naga-ir` feature. All web/native divergence is
/// expressed via `shader_defs` on the SOURCE, so nothing depends on how naga's WGSL backend
/// reformats output.
///
/// **Memoised for the life of the process**, keyed by everything the output depends on: the
/// source text, the label (naga_oil names the module after it) and the defs. Composition is a pure
/// function of those, and it is nearly all of what building a renderer costs — measured
/// 2026-11-02, 0.99 s of a 1.1 s headless construction, over 18 shaders. A test binary builds a
/// renderer per test, often several, so the GPU tests spent most of their time composing the same
/// eighteen shaders again. A hot-reloaded shader is new source text, so it misses and is composed.
pub(crate) fn compose_wgsl(
    source: &str,
    label: &str,
    shader_defs: HashMap<String, ShaderDefValue>,
) -> String {
    try_compose_wgsl(source, label, shader_defs, None).unwrap_or_else(|e| panic!("{e}"))
}

/// [`compose_wgsl`] with a game's surface module ([`compose_module_with`]), and a failure handed back
/// rather than raised: a game's WGSL failing to compose is the game's error to report, where one of the
/// engine's own is a broken build.
pub(crate) fn try_compose_wgsl(
    source: &str,
    label: &str,
    shader_defs: HashMap<String, ShaderDefValue>,
    custom_surface: Option<&str>,
) -> Result<String, String> {
    use std::hash::{Hash, Hasher};
    use std::sync::{Mutex, OnceLock};
    type Entry = (String, Option<String>, String);
    static COMPOSED: OnceLock<Mutex<HashMap<u64, Entry>>> = OnceLock::new();

    // Defs in a fixed order: a `HashMap`'s iteration order is not one.
    let mut defs: Vec<(&String, &ShaderDefValue)> = shader_defs.iter().collect();
    defs.sort_by(|a, b| a.0.cmp(b.0));
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (source, label, &defs, custom_surface).hash(&mut h);
    let key = h.finish();
    // The full sources are kept beside the output and compared on a hit, so a hash collision
    // composes afresh instead of handing back another shader.
    let cache = COMPOSED.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some((src, surface, out)) = cache.lock().unwrap_or_else(|p| p.into_inner()).get(&key) {
        if src == source && surface.as_deref() == custom_surface {
            return Ok(out.clone());
        }
    }

    let (module, info) = compose_module_with(source, label, shader_defs, custom_surface)?;
    let out = naga::back::wgsl::write_string(&module, &info, naga::back::wgsl::WriterFlags::empty())
        .map_err(|e| format!("emitting WGSL for '{label}' failed: {e}"))?;
    cache
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(key, (source.to_string(), custom_surface.map(str::to_string), out.clone()));
    Ok(out)
}

/// Compiles a shader, preferring a copy on disk over the compiled-in fallback — the mechanism
/// behind shader hot-reload. See [`Renderer::load_shader`](crate::Renderer::load_shader).
pub fn load_shader(
    device: &wgpu::Device,
    file_path: &str,
    fallback_src: &str,
    label: &str,
) -> wgpu::ShaderModule {
    let source = std::fs::read_to_string(file_path).unwrap_or_else(|_| fallback_src.to_string());
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
}

/// Like [`load_shader`], but first resolves `#import gizmo::common`, `#ifdef` and `#{DEF}`
/// via naga_oil under the NATIVE render schema (5 bind groups incl. shadow). Used for every
/// composed shader on native (deferred/SS pipelines + the forward shader/unlit/water/sky/grid).
pub fn load_shader_composed(
    device: &wgpu::Device,
    file_path: &str,
    fallback_src: &str,
    label: &str,
) -> wgpu::ShaderModule {
    load_shader_composed_with(device, file_path, fallback_src, label, &[])
}

/// [`load_shader_composed`] with some native defs overridden — the deferred lighting pass sets
/// `REFLECTION_GROUP` to 3, where its pipeline has the irradiance group.
pub fn load_shader_composed_with(
    device: &wgpu::Device,
    file_path: &str,
    fallback_src: &str,
    label: &str,
    overrides: &[(&str, ShaderDefValue)],
) -> wgpu::ShaderModule {
    let source = std::fs::read_to_string(file_path).unwrap_or_else(|_| fallback_src.to_string());
    let mut defs = native_render_defs();
    for (name, value) in overrides {
        // `#ifdef` asks whether a name is there, so `false` has to take it away.
        if *value == ShaderDefValue::Bool(false) {
            defs.remove(*name);
        } else {
            defs.insert((*name).to_string(), *value);
        }
    }
    let composed = compose_wgsl(&source, label, defs);
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(composed.into()),
    })
}

/// WASM-only replacement for the old text-surgery `load_shader_web`: composes the SAME shader
/// source under the WEB render schema (4 bind groups, no shadow). The `#ifdef SHADOWS` guards
/// strip the shadow bindings + PCF block and `@group(#{SKELETON_GROUP/INSTANCE_GROUP})` place
/// skeleton/instance at 2/3 — all at the source level, so it is immune to naga's WGSL-backend
/// reformatting (the reason the old grep-the-output approach was broken).
#[cfg(gizmo_forward_only)]
pub fn load_shader_composed_web(
    device: &wgpu::Device,
    fallback_src: &str,
    label: &str,
) -> wgpu::ShaderModule {
    load_shader_composed_web_with(device, fallback_src, label, &[])
}

/// [`load_shader_composed_web`] with some defs added: a variant of a shader, as
/// [`load_shader_composed_with`] makes on the desktop.
#[cfg(gizmo_forward_only)]
pub fn load_shader_composed_web_with(
    device: &wgpu::Device,
    fallback_src: &str,
    label: &str,
    overrides: &[(&str, ShaderDefValue)],
) -> wgpu::ShaderModule {
    let mut defs = web_render_defs();
    for (name, value) in overrides {
        // As on the desktop: `#ifdef` asks whether a name is there, so `false` has to take it away.
        if *value == ShaderDefValue::Bool(false) {
            defs.remove(*name);
        } else {
            defs.insert((*name).to_string(), *value);
        }
    }
    let composed = compose_wgsl(fallback_src, label, defs);
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(composed.into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHADER_SRC: &str = include_str!("../shaders/shader.wgsl");

    // These lock in the WASM shader_defs rework WITHOUT a GPU or browser: compose_wgsl is a
    // pure fn (naga_oil → naga validate → WGSL emit), so it runs in any CI. They assert the
    // web/native divergence happens at the SOURCE (`#ifdef SHADOWS`, `@group(#{...})`), which
    // is what makes it robust — unlike the retired load_shader_web that grepped naga's
    // reformatted output (empirically broken: naga rewrites the shadow `if` to `if (_eNN >
    // 0.5f)`, so the block-strip silently no-oped while the binding-strip fired → undefined
    // t_shadow). If someone reintroduces a shadow use outside `#ifdef SHADOWS`, the web
    // variant fails to compose here and this test catches it.

    // `compose_wgsl` runs naga's validator and panics on failure, so composing a shader IS
    // type-checking it — without an adapter, and so without the GPU-test serialisation the
    // existing `core_shaders_compile` needs. Every shader touched by the baked-lit work is
    // checked under BOTH schemas, because the web one drops the shadow group and is where a
    // fade added outside `#ifdef SHADOWS` would come apart.
    /// The composition cache answers only the question it was asked: the same source under other
    /// defs, or under another label, is composed again — a cache keyed on the source alone would
    /// hand the web schema the native shader (shadow group and all) — and a repeat is identical.
    #[test]
    fn the_composition_cache_keys_on_defs_and_label_too() {
        let native = compose_wgsl(SHADER_SRC, "cache_probe", native_render_defs());
        let web = compose_wgsl(SHADER_SRC, "cache_probe", web_render_defs());
        assert_ne!(native, web, "the web schema got the native composition");
        assert_eq!(native, compose_wgsl(SHADER_SRC, "cache_probe", native_render_defs()));
        let mut one_more = native_render_defs();
        one_more.insert("CACHE_PROBE_UNUSED".to_string(), ShaderDefValue::Bool(true));
        // An extra def is an entry of its own; one the shader never reads composes to the same text.
        assert_eq!(native, compose_wgsl(SHADER_SRC, "cache_probe", one_more));
    }

    #[test]
    fn baked_lit_path_shaders_type_check_under_both_schemas() {
        let web_capable: &[(&str, &str)] = &[
            ("baked_lit.wgsl", include_str!("../shaders/baked_lit.wgsl")),
            ("shader.wgsl", include_str!("../shaders/shader.wgsl")),
            ("unlit.wgsl", include_str!("../shaders/unlit.wgsl")),
            ("water.wgsl", include_str!("../shaders/water.wgsl")),
            ("planar_mirror.wgsl", include_str!("../shaders/planar_mirror.wgsl")),
            ("sky.wgsl", include_str!("../shaders/sky.wgsl")),
            // The painted backdrop. Its whole job is a vertex transform and a texture fetch,
            // so type-checking it under both schemas is most of what a GPU would tell us —
            // and it is the only stage of that shader this crate can reach without one.
            ("backdrop.wgsl", include_str!("../shaders/backdrop.wgsl")),
            ("grid.wgsl", include_str!("../shaders/grid.wgsl")),
            // Text composes under both schemas because it binds only groups 0 and 1 — no shadow
            // group to strip, no skeleton or instance group to shift. Checked here anyway: it
            // imports `SceneUniforms` like the rest, so a field renamed in `common.wgsl` breaks it
            // exactly as it breaks them, and this is the only gate that says so without a GPU.
            ("text.wgsl", include_str!("../shaders/text.wgsl")),
        ];
        for (name, src) in web_capable {
            compose_wgsl(src, name, native_render_defs());
            compose_wgsl(src, name, web_render_defs());
        }
        // Water showing a `PlanarReflection` is its own variant of the same source.
        let water = include_str!("../shaders/water.wgsl");
        for mut defs in [native_render_defs(), web_render_defs()] {
            defs.insert("PLANAR".to_string(), ShaderDefValue::Bool(true));
            compose_wgsl(water, "water.wgsl (planar)", defs);
        }
        // Native-only (no web schema): the deferred pass, the shadow pass, the volumetric march
        // (which reads the same shadow group the lit passes do) and the seen mask.
        let native_only: &[(&str, &str)] = &[
            ("deferred_lighting.wgsl", include_str!("../shaders/deferred_lighting.wgsl")),
            ("gbuffer.wgsl", include_str!("../shaders/gbuffer.wgsl")),
            ("shadow.wgsl", include_str!("../shaders/shadow.wgsl")),
            ("volumetric.wgsl", include_str!("../shaders/volumetric.wgsl")),
            ("seen.wgsl", include_str!("../shaders/seen.wgsl")),
        ];
        for (name, src) in native_only {
            compose_wgsl(src, name, native_render_defs());
        }
    }

    /// No shader function takes a big value by value. `gizmo_cluster_index` took the whole
    /// `SceneUniforms` (17 040 bytes, nearly all of it `lights: array<LightData, 256>`), and on an
    /// Adreno 618 phone (driver V@0502, Chrome 156) the lit forward shader came out miscompiled:
    /// speckles and blocks of garbage wherever a value depended on a fragment input, a constant
    /// colour drawn clean. Desktop drivers fold the copy away, so nothing here can see the
    /// miscompile itself — what this can see is the shape that provoked it, in every shader both
    /// schemas compose.
    ///
    /// Entry points are left out: their arguments are stage inputs, not copies.
    #[test]
    fn no_shader_function_takes_a_large_value_by_value() {
        const LIMIT: u32 = 1024;
        let shaders: &[(&str, &str, bool)] = &[
            ("shader.wgsl", include_str!("../shaders/shader.wgsl"), true),
            ("baked_lit.wgsl", include_str!("../shaders/baked_lit.wgsl"), true),
            ("unlit.wgsl", include_str!("../shaders/unlit.wgsl"), true),
            ("water.wgsl", include_str!("../shaders/water.wgsl"), true),
            ("sky.wgsl", include_str!("../shaders/sky.wgsl"), true),
            ("backdrop.wgsl", include_str!("../shaders/backdrop.wgsl"), true),
            ("grid.wgsl", include_str!("../shaders/grid.wgsl"), true),
            ("text.wgsl", include_str!("../shaders/text.wgsl"), true),
            ("deferred_lighting.wgsl", include_str!("../shaders/deferred_lighting.wgsl"), false),
            ("gbuffer.wgsl", include_str!("../shaders/gbuffer.wgsl"), false),
        ];
        let mut offenders = Vec::new();
        for &(name, src, web_too) in shaders {
            let mut schemas = vec![("native", native_render_defs())];
            if web_too {
                schemas.push(("web", web_render_defs()));
            }
            for (schema, defs) in schemas {
                let (module, _) = compose_module(src, name, defs).expect("composes");
                let mut layouter = naga::proc::Layouter::default();
                layouter.update(module.to_ctx()).expect("layout");
                for (_, function) in module.functions.iter() {
                    for argument in &function.arguments {
                        let size = layouter[argument.ty].size;
                        if size > LIMIT {
                            offenders.push(format!(
                                "{name} ({schema}): {}({}: {} bytes)",
                                function.name.as_deref().unwrap_or("?"),
                                argument.name.as_deref().unwrap_or("?"),
                                size
                            ));
                        }
                    }
                }
            }
        }
        assert!(offenders.is_empty(), "functions taking more than {LIMIT} bytes by value: {offenders:#?}");
    }

    #[test]
    fn web_compose_strips_shadows_and_shifts_groups() {
        let web = compose_wgsl(SHADER_SRC, "shader.wgsl", web_render_defs());
        // naga_oil resolved every preprocessor directive (nothing leaks to wgpu).
        assert!(
            !web.contains("#import") && !web.contains("#ifdef") && !web.contains("#{"),
            "web compose left unresolved naga_oil tokens"
        );
        // Shadow bindings + sampling are gone (they'd be undefined ids on the 4-group layout).
        // Match the binding DECLARATION, not the bare substring "t_shadow" — the always-present
        // SceneUniforms field `light_shadows` contains "t_shadow" as a substring.
        assert!(!web.contains("var t_shadow"), "web variant must not declare t_shadow");
        assert!(!web.contains("var t_local_shadow"), "web variant must not declare the local shadow layers");
        assert!(
            !web.contains("textureSampleCompare"),
            "web variant must not sample shadows"
        );
        // With no shadow group, instance shifts from 4 to 3 → group 4 disappears entirely.
        assert!(!web.contains("@group(4)"), "web variant must not use @group(4)");
    }

    #[test]
    fn native_compose_keeps_shadows_and_groups() {
        let native = compose_wgsl(SHADER_SRC, "shader.wgsl", native_render_defs());
        assert!(
            !native.contains("#import") && !native.contains("#ifdef") && !native.contains("#{"),
            "native compose left unresolved naga_oil tokens"
        );
        // Native keeps the CSM shadow bindings + PCF sampling…
        assert!(native.contains("var t_shadow"), "native variant must keep the shadow bindings");
        assert!(
            native.contains("textureSampleCompare"),
            "native variant must keep shadow sampling"
        );
        // …and instance stays at group 4 (shadow occupies group 2).
        assert!(native.contains("@group(4)"), "native variant keeps instance at group 4");
    }

    async fn setup_headless_gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
        crate::test_gpu::headless_device().await
    }

    async fn read_mat_cols(device: &wgpu::Device, queue: &wgpu::Queue, buffer: &wgpu::Buffer) -> [f32; 16] {
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("inv_readback"),
            size: 64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        enc.copy_buffer_to_buffer(buffer, 0, &staging, 0, 64);
        queue.submit(Some(enc.finish()));
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |v| tx.send(v).unwrap());
        let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        rx.recv().unwrap().unwrap();
        let data = slice
            // wgpu 30 made this fallible; the range is the whole buffer we just mapped, so a
            // failure here is a programming error rather than a runtime condition.
            .get_mapped_range()
            .expect("a just-mapped buffer's full range is always valid");
        let mut out = [0.0f32; 16];
        out.copy_from_slice(bytemuck::cast_slice(&data));
        drop(data);
        staging.unmap();
        out
    }

    // Executes the SHIPPED gizmo::common::inverse_mat4 on the GPU (composed against the real
    // common.wgsl, not a copy) and compares it to glam's inverse for a perspective view_proj.
    // A transcription typo (n42 in place of n43 in the t21/t22/t23 cofactors) made inverse_mat4
    // return a wrong inverse for perspective matrices — the ray-reconstruction bug that
    // collapsed volumetric smoke to a thin sliver and silently corrupted every fullscreen pass
    // that unprojects NDC→world (deferred_lighting, particle_render, volumetric). Skips (passes)
    // when no GPU adapter is available. Reintroducing the bad formula fails the assert below.
    #[test]
    fn inverse_mat4_matches_glam_on_gpu() {
        // Guard testin tamamı boyunca: aynı anda birden fazla canlı cihaz sürücüyü
        // düşürüyor (bkz. `crate::test_gpu`).
        let _gpu = crate::test_gpu::gpu_lock();
        use gizmo_math::{Mat4, Vec3};
        use wgpu::util::DeviceExt;

        let composed = compose_wgsl(
            r#"
#import gizmo::common::inverse_mat4
@group(0) @binding(0) var<storage, read> m_in: mat4x4<f32>;
@group(0) @binding(1) var<storage, read_write> m_out: mat4x4<f32>;
@compute @workgroup_size(1)
fn main() { m_out = inverse_mat4(m_in); }
"#,
            "inverse_mat4_test",
            HashMap::new(),
        );

        pollster::block_on(async {
            let Some((device, queue)) = setup_headless_gpu().await else { return };
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("inverse_mat4_test"),
                source: wgpu::ShaderSource::Wgsl(composed.into()),
            });

            // The exact camera from the smoke-sliver repro (perspective → 4th row is not affine).
            let view = Mat4::look_at_rh(Vec3::new(6.0, 3.0, 7.0), Vec3::new(0.0, 2.2, 0.0), Vec3::Y);
            let proj = Mat4::perspective_rh(45f32.to_radians(), 16.0 / 9.0, 0.1, 500.0);
            let vp = proj * view;

            let in_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("m_in"),
                contents: bytemuck::cast_slice(&vp.to_cols_array()),
                usage: wgpu::BufferUsages::STORAGE,
            });
            let out_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("m_out"),
                size: 64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });

            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("inverse_mat4_test"),
                layout: None,
                module: &shader,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: in_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: out_buf.as_entire_binding() },
                ],
            });

            let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
            {
                let mut cpass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: None,
                    timestamp_writes: None,
                });
                cpass.set_pipeline(&pipeline);
                cpass.set_bind_group(0, &bg, &[]);
                cpass.dispatch_workgroups(1, 1, 1);
            }
            queue.submit(Some(enc.finish()));

            let gpu_cols = read_mat_cols(&device, &queue, &out_buf).await;
            let gpu_inv = Mat4::from_cols_array(&gpu_cols);
            let want = vp.inverse();

            // 1) GPU inverse matches glam element-wise (loose f32 tolerance).
            let g = gpu_inv.to_cols_array();
            let w = want.to_cols_array();
            let max_err = g.iter().zip(w.iter()).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
            assert!(max_err < 1e-3, "GPU inverse_mat4 differs from glam by {max_err} (buggy formula?)");

            // 2) The concrete bug scenario: unproject a known NDC point back to world. The box
            //    center (0,2,0) projected to NDC then unprojected must return to (0,2,0). The
            //    buggy formula returned world-y ≈ 94.8 here.
            let clip = vp * gizmo_math::Vec4::new(0.0, 2.0, 0.0, 1.0);
            let ndc = clip / clip.w;
            let unproj_h = gpu_inv * gizmo_math::Vec4::new(ndc.x, ndc.y, ndc.z, 1.0);
            let world = unproj_h.truncate() / unproj_h.w;
            assert!(
                (world - Vec3::new(0.0, 2.0, 0.0)).length() < 1e-2,
                "NDC→world round-trip landed at {world:?}, expected ~(0,2,0) (buggy inverse_mat4)"
            );
        });
    }
}
