//! Shader yükleme yardımcıları (naga_oil ile WGSL modül kompozisyonu + native/web şeması).

use naga_oil::compose::ShaderDefValue;
use std::collections::HashMap;

/// The shared shader library (`#define_import_path gizmo::common`), embedded once and
/// registered with the naga_oil composer so any shader can `#import gizmo::common::{...}`.
const COMMON_WGSL: &str = include_str!("../shaders/common.wgsl");

/// Deferred-specific PBR extensions (`#define_import_path gizmo::pbr_ext`): anisotropic GGX,
/// clear-coat, and the Lazarov env-BRDF LUT. Imports `gizmo::common`, so it is registered
/// AFTER it below.
const PBR_EXT_WGSL: &str = include_str!("../shaders/pbr_ext.wgsl");

/// Shader-defs for the NATIVE render schema: 5 bind groups with the CSM shadow group in the
/// middle. `SHADOWS` keeps the `#ifdef SHADOWS` shadow bindings + PCF block; the group
/// indices place skeleton at 3 and instance at 4 (see build_core_pipelines' native layout).
pub(crate) fn native_render_defs() -> HashMap<String, ShaderDefValue> {
    HashMap::from([
        ("SHADOWS".to_string(), ShaderDefValue::Bool(true)),
        ("SKELETON_GROUP".to_string(), ShaderDefValue::UInt(3)),
        ("INSTANCE_GROUP".to_string(), ShaderDefValue::UInt(4)),
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
    ])
}

/// Compose a shader source (`#import gizmo::common`, `#ifdef`, `#{DEF}`) into flat WGSL text
/// under the given `shader_defs`.
///
/// We resolve with naga_oil then emit WGSL (rather than handing wgpu a `naga::Module`)
/// because wgpu 29 here is built without the `naga-ir` feature. All web/native divergence is
/// expressed via `shader_defs` on the SOURCE, so nothing depends on how naga's WGSL backend
/// reformats output.
pub(crate) fn compose_wgsl(
    source: &str,
    label: &str,
    shader_defs: HashMap<String, ShaderDefValue>,
) -> String {
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
        .unwrap_or_else(|e| panic!("composing common.wgsl failed: {e}"));
    // pbr_ext imports gizmo::common, so it must be registered after common. Only shaders that
    // `#import gizmo::pbr_ext` pull it in; registering it here is otherwise inert.
    composer
        .add_composable_module(ComposableModuleDescriptor {
            source: PBR_EXT_WGSL,
            file_path: "gizmo/pbr_ext.wgsl",
            language: ShaderLanguage::Wgsl,
            ..Default::default()
        })
        .unwrap_or_else(|e| panic!("composing pbr_ext.wgsl failed: {e}"));

    let module = composer
        .make_naga_module(NagaModuleDescriptor {
            source,
            file_path: label,
            shader_defs,
            ..Default::default()
        })
        .unwrap_or_else(|e| panic!("naga_oil compose of '{label}' failed: {e}"));

    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap_or_else(|e| panic!("validating composed '{label}' failed: {e:?}"));

    naga::back::wgsl::write_string(&module, &info, naga::back::wgsl::WriterFlags::empty())
        .unwrap_or_else(|e| panic!("emitting WGSL for '{label}' failed: {e}"))
}

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
    let source = std::fs::read_to_string(file_path).unwrap_or_else(|_| fallback_src.to_string());
    let composed = compose_wgsl(&source, label, native_render_defs());
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
#[cfg(target_arch = "wasm32")]
pub fn load_shader_composed_web(
    device: &wgpu::Device,
    fallback_src: &str,
    label: &str,
) -> wgpu::ShaderModule {
    let composed = compose_wgsl(fallback_src, label, web_render_defs());
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
        // SceneUniforms field `point_shadows_enabled` contains "t_shadow" as a substring.
        assert!(!web.contains("var t_shadow"), "web variant must not declare t_shadow");
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
}
