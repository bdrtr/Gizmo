//! The bind group layouts (global, shadow, texture, skeleton, instance).

use crate::descriptors::{buffer_entry, fragment_sampler, fragment_texture};
use wgpu::{
    BindGroupLayoutEntry as Entry, BufferBindingType as BufferType, SamplerBindingType as Sampler, ShaderStages as Stages,
    TextureSampleType as Sample, TextureViewDimension as Dim,
};

pub(crate) struct Layouts {
    pub(crate) global: wgpu::BindGroupLayout,
    pub(super) shadow: wgpu::BindGroupLayout,
    pub(super) shadow_pass: wgpu::BindGroupLayout,
    pub(crate) texture: wgpu::BindGroupLayout,
    pub(super) skeleton: wgpu::BindGroupLayout,
    pub(super) instance: wgpu::BindGroupLayout,
}

/// A float texture that filters: what most of the textures here are.
const FILTERED: Sample = Sample::Float { filterable: true };

pub(crate) fn build_layouts(device: &wgpu::Device) -> Layouts {
    let layout = |label: &str, entries: &[Entry]| {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(label), entries })
    };
    Layouts {
        global: layout("global_bind_group_layout", &global_entries()),
        shadow: layout("shadow_bind_group_layout", &shadow_entries()),
        // The fragment stage too: a cut-out's (`shadow.wgsl`'s `CUT_OUT`) reads the frame's camera and
        // clock for a game's surface.
        shadow_pass: layout("shadow_pass_bind_group_layout", &[buffer_entry(0, Stages::VERTEX_FRAGMENT, BufferType::Uniform)]),
        texture: layout("texture_bind_group_layout", &texture_entries()),
        skeleton: layout("skeleton_bind_group_layout", &[buffer_entry(0, Stages::VERTEX, BufferType::Uniform)]),
        instance: layout("instance_bind_group_layout", &[buffer_entry(0, Stages::VERTEX, BufferType::Storage { read_only: true })]),
    }
}

/// Group 0, which every pipeline binds.
///
/// The clustered-light buffers live here rather than in a group of their own, and that is a hard
/// constraint rather than a preference: the web forward pipeline already binds four groups (global,
/// texture, skeleton, instance) and `max_bind_groups` is 4 on every WebGPU baseline this engine
/// targets, so a fifth group would take clustering away from the browser. A shader that does not
/// light simply does not declare bindings 1 and 2 — wgpu allows a layout to offer more than a shader
/// uses, not less.
fn global_entries() -> Vec<Entry> {
    #[allow(unused_mut)] // The forward-only path adds to it.
    let mut entries = vec![
        buffer_entry(0, Stages::VERTEX | Stages::FRAGMENT | Stages::COMPUTE, BufferType::Uniform),
        // Cluster table: (offset, count) per cluster. FRAGMENT only — lighting is a fragment-stage
        // job, and a storage buffer visible to the vertex stage spends a slot from a per-stage budget
        // that is 8 on the WebGPU baseline.
        buffer_entry(1, Stages::FRAGMENT, BufferType::Storage { read_only: true }),
        // Light indices, referenced by the table.
        buffer_entry(2, Stages::FRAGMENT, BufferType::Storage { read_only: true }),
        // The frame under the glass (`crate::transmission`) and how to read it. On the global group
        // for the reason the clusters are: the web has no group to spare.
        fragment_texture(3, Dim::D2, FILTERED),
        fragment_sampler(4, Sampler::Filtering),
        // The environment, mip by roughness (`crate::env_cube`), which the forward-only path's lit
        // shader reads in place of working it out per pixel. On the global group for the same reason.
        fragment_texture(5, Dim::Cube, FILTERED),
        // The depth of the frame under the glass (`crate::transmission`): how far behind a surface the
        // opaque world is, which a surface over a volume (water) reads for its depth. One sampled
        // texture more: the desktop's deferred lighting layout held 16 with it before the forward-only
        // path's probes and cascades (6–10) left group 0 on the desktop, 14 since; the forward-only lit
        // layout 12 — of WebGPU's 16 a stage.
        fragment_texture(14, Dim::D2, Sample::Depth),
    ];
    // Reflection probes (6–8) and the sun's cascades (9–10), for the forward-only path's lit shaders,
    // which have no irradiance group and no shadow group to read them from. **Not on the desktop**,
    // which reads both from those groups: it used to bind placeholders and the same cascades here, and
    // a layout counts every texture it offers whether a shader reads it or not — two sampled textures
    // in every pipeline over group 0 that no desktop shader touched, which held the forward lit layout
    // at 15 of WebGPU's 16 a stage and the deferred lighting one at 16 (measured 2026-11-17, adding a
    // material map failed validation at 17).
    #[cfg(gizmo_forward_only)]
    entries.extend([
        // The prefiltered cube array, its sampler and the uniform that places them (`reflection_probes.wgsl`).
        fragment_texture(6, Dim::CubeArray, FILTERED),
        fragment_sampler(7, Sampler::Filtering),
        buffer_entry(8, Stages::FRAGMENT, BufferType::Uniform),
        // The sun's cascades and their comparison sampler (`SUN_SHADOWS`).
        fragment_texture(9, Dim::D2Array, Sample::Depth),
        fragment_sampler(10, Sampler::Comparison),
    ]);
    // The local lights' shadow layers and cookies, with the cookies' sampler, for the forward-only path's lit
    // shaders (`LOCAL_LIGHT_MAPS`), which have no shadow group: there the lit pipeline layout holds 11
    // sampled textures and 5 samplers with them, of WebGPU's 16 and 16 a stage. Not on the desktop, which
    // reads them from its shadow group (bindings 2–4): its deferred lighting layout, over group 0 and the
    // G-buffer, would hold two more.
    #[cfg(gizmo_forward_only)]
    entries.extend([
        fragment_texture(11, Dim::D2Array, Sample::Depth),
        fragment_texture(12, Dim::D2Array, FILTERED),
        fragment_sampler(13, Sampler::Filtering),
    ]);
    // The decals the forward-only path's lit shaders project over a surface (`DECALS`,
    // `crate::decal::ForwardDecals`): their textures' array, the decals and the sampler. That path has
    // no G-buffer for a decal to be drawn over, as the desktop's are. One sampled texture more: 15 of
    // WebGPU's 16 a stage in the lit pipeline layout, with the texture group's seven.
    #[cfg(gizmo_forward_only)]
    entries.extend([
        fragment_texture(15, Dim::D2Array, FILTERED),
        buffer_entry(16, Stages::FRAGMENT, BufferType::Storage { read_only: true }),
        fragment_sampler(17, Sampler::Filtering),
    ]);
    entries
}

/// The desktop's shadow group: the sun's cascades, the local lights' layers and their cookies.
fn shadow_entries() -> [Entry; 5] {
    [
        fragment_texture(0, Dim::D2Array, Sample::Depth),
        fragment_sampler(1, Sampler::Comparison),
        // The local lights' shadow layers (`crate::local_shadows`).
        fragment_texture(2, Dim::D2Array, Sample::Depth),
        // Light cookies (`crate::cookies`): beside the shadow maps because a spot's cookie is sampled
        // where its shadow is, and a fixed-size array never needs the group rebuilt.
        fragment_texture(3, Dim::D2Array, FILTERED),
        fragment_sampler(4, Sampler::Filtering),
    ]
}

/// Per-material texture set (bind group 1). The base-colour texture + sampler at bindings 0/1 are the
/// original layout; bindings 2..=5 add the textured-PBR maps (normal, metallic-roughness, emissive,
/// ambient-occlusion) and binding 6 carries the scalar `MaterialParams` (emissive factor, normal
/// scale, AO strength). Forward/decal shaders that only reference bindings 0/1 remain valid — wgpu
/// permits a layout with more entries than a shader uses — but every bind group built against this
/// layout MUST populate all nine entries (binding 7 is the lightmap, binding 8 the texture pool's
/// array) (see `AssetManager::assemble_material_bind_group`).
fn texture_entries() -> [Entry; 9] {
    [
        fragment_texture(0, Dim::D2, FILTERED), // base colour
        fragment_sampler(1, Sampler::Filtering),
        fragment_texture(2, Dim::D2, FILTERED), // normal map
        fragment_texture(3, Dim::D2, FILTERED), // metallic-roughness (ARM/MR)
        fragment_texture(4, Dim::D2, FILTERED), // emissive
        fragment_texture(5, Dim::D2, FILTERED), // ambient occlusion
        buffer_entry(6, Stages::VERTEX_FRAGMENT, BufferType::Uniform),
        // The lightmap: baked light, read at the vertex's second UV set (`Vertex::lightmap_uv`).
        fragment_texture(7, Dim::D2, FILTERED),
        // The texture pool's array (`crate::texture_pool`): a material made from a pool reads its base
        // colour from a layer of it, the instance saying which (`InstanceRaw::texture`). Two white
        // layers in every other material (`MaterialDefaults::pool_placeholder_view`).
        fragment_texture(8, Dim::D2Array, FILTERED),
    ]
}

/// The wireframe pipeline's group 5: one colour.
pub(super) fn wire_color_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("wire_color_bind_group_layout"),
        entries: &[buffer_entry(0, Stages::FRAGMENT, BufferType::Uniform)],
    })
}

pub(super) struct LayoutRefs<'a> {
    pub(super) global: &'a wgpu::BindGroupLayout,
    // WASM pipeline layout'u shadow grubunu kullanmaz (4-grup şeması).
    #[cfg_attr(gizmo_forward_only, allow(dead_code))]
    pub(super) shadow: &'a wgpu::BindGroupLayout,
    pub(super) shadow_pass: &'a wgpu::BindGroupLayout,
    pub(super) texture: &'a wgpu::BindGroupLayout,
    pub(super) skeleton: &'a wgpu::BindGroupLayout,
    pub(super) instance: &'a wgpu::BindGroupLayout,
    /// The irradiance volume's group (with the reflection probes), group 5 of the native forward
    /// pass.
    #[cfg_attr(gizmo_forward_only, allow(dead_code))]
    pub(super) indirect: &'a wgpu::BindGroupLayout,
    /// The wireframe pipeline's group 5: this draw's line colour.
    #[cfg_attr(gizmo_forward_only, allow(dead_code))]
    pub(super) wire_color: &'a wgpu::BindGroupLayout,
}
