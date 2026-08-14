use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    /// Vertex colour, **RGBA**.
    ///
    /// The alpha is a real channel, not padding: a decal or skid-mark layer authored with a
    /// soft edge carries that edge here, and `baked_lit.wgsl` multiplies it into the fragment
    /// alpha (see [`crate::components::MaterialType::BakedLit`]). Shaders that have no use
    /// for it declare `@location(1) color: vec3<f32>` and read only the RGB — a vertex format
    /// may supply more components than the shader consumes.
    ///
    /// The attribute is ALWAYS present, because this is the engine's one vertex layout. A
    /// source file that carries no colours is normalised to opaque white **at import** (see
    /// [`Default`] and the glTF loader), which is the only place that knows the attribute was
    /// absent. Downstream code must therefore take this value at face value: `[0,0,0,1]`
    /// means "authored black", not "missing".
    pub color: [f32; 4],
    pub normal: [f32; 3],
    pub tex_coords: [f32; 2],
    pub joint_indices: [u32; 4],
    pub joint_weights: [f32; 4],
    pub tangent: [f32; 4],
}

impl Default for Vertex {
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            // Opaque white: the neutral element of every multiply the colour feeds, and the
            // value an importer must leave behind when the source has no colour attribute.
            color: [1.0; 4],
            normal: [0.0, 1.0, 0.0],
            tex_coords: [0.0; 2],
            joint_indices: [0; 4],
            joint_weights: [0.0; 4],
            tangent: [1.0, 0.0, 0.0, 1.0],
        }
    }
}

impl Vertex {
    pub fn desc<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                },
                // Vertex colour is RGBA (Float32x4). A shader that only wants the RGB may
                // still declare `vec3<f32>` here — wgpu/WebGPU allow a shader input with
                // FEWER components than the attribute format supplies, so widening this
                // attribute did not require touching the eight shaders that ignore alpha.
                // Offsets come from `offset_of!` rather than a running sum of the preceding
                // field sizes: widening `color` from RGB to RGBA moved five of them by four
                // bytes, and a hand-maintained sum is one arithmetic slip away from a
                // pipeline that reads normals out of the tex-coords.
                wgpu::VertexAttribute {
                    offset: std::mem::offset_of!(Vertex, color) as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: std::mem::offset_of!(Vertex, normal) as wgpu::BufferAddress,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: std::mem::offset_of!(Vertex, tex_coords) as wgpu::BufferAddress,
                    shader_location: 3,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: std::mem::offset_of!(Vertex, joint_indices) as wgpu::BufferAddress,
                    shader_location: 4,
                    format: wgpu::VertexFormat::Uint32x4,
                },
                wgpu::VertexAttribute {
                    offset: std::mem::offset_of!(Vertex, joint_weights) as wgpu::BufferAddress,
                    shader_location: 5,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: std::mem::offset_of!(Vertex, tangent) as wgpu::BufferAddress,
                    shader_location: 6,
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Pod, Zeroable)]
pub struct LightData {
    pub position: [f32; 4],  // xyz=pos, w=intensity
    pub color: [f32; 4],     // rgb=color, a=radius
    pub direction: [f32; 4], // xyz=direction (spot), w=inner_cutoff_cos
    pub params: [f32; 4], // x=outer_cutoff_cos, y=light_type (0=point,1=spot,2=directional), zw=unused
}

impl Default for LightData {
    /// An unlit slot: zero intensity, zero radius, pointing down.
    ///
    /// The array is fixed-length and `SceneUniforms::num_lights` says how much of it is live, so
    /// the tail is never read — but it is uploaded, and a light array padded with garbage is one
    /// shader bug away from being visible.
    fn default() -> Self {
        Self {
            position: [0.0; 4],
            color: [0.0; 4],
            direction: [0.0, -1.0, 0.0, 0.0],
            params: [0.0; 4],
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct PostProcessUniforms {
    pub bloom_intensity: f32,
    pub bloom_threshold: f32,
    pub exposure: f32,
    pub chromatic_aberration: f32,
    pub vignette_intensity: f32,
    pub film_grain_intensity: f32,
    pub dof_focus_dist: f32,
    pub dof_focus_range: f32,
    pub dof_blur_size: f32,
    // Active camera near/far, so DoF depth linearization matches the real projection
    // instead of hardcoded 0.1/1000 (miscalibrated CoC for any other far plane).
    pub cam_near: f32,
    pub cam_far: f32,
    // ── Su-altı atmosferi (kamera bir fluid zone içindeyken) ──
    /// 0 = kamera havada (etki yok), 1 = kamera su altında → derinlik-bazlı sis uygulanır.
    pub underwater: f32,
    /// Su-altı sis rengi (deniz mavisi-yeşili) + yoğunluk. WGSL'de tek `fog: vec4` (rgb+a=density)
    /// olarak hizalanır (offset 48, 16-bayt hizalı).
    pub fog_r: f32,
    pub fog_g: f32,
    pub fog_b: f32,
    pub fog_density: f32,
}

/// Uniform block for the shadow pass vertex shader only (one cascade matrix per draw).
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct ShadowVsUniform {
    pub light_view_proj: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct SceneUniforms {
    pub view_proj: [[f32; 4]; 4],
    pub camera_pos: [f32; 4],
    pub sun_direction: [f32; 4],
    pub sun_color: [f32; 4],
    pub lights: [LightData; 10],
    /// Directional CSM: world → light clip space per cascade (same order as shadow array layers).
    pub light_view_proj: [[[f32; 4]; 4]; 4],
    /// Far distance (along `camera_forward`) for cascades 0..3. `w` is therefore the far edge
    /// of the LAST cascade — the whole range the shadow maps cover, `min(camera far,
    /// csm::SHADOW_DISTANCE)` — which is what the shaders fade the shadow term out over. (It
    /// was documented as "always camera far plane"; it never was, and the shadow fade depends
    /// on it not being.)
    pub cascade_splits: [f32; 4],
    /// xyz = normalized camera forward in world space (for view-depth cascade selection).
    pub camera_forward: [f32; 4],
    /// x = camera z_near, y = 1 / shadow map resolution (PCF texel size),
    /// z = elapsed time in seconds (fluid caustics/wave animation), w unused.
    pub cascade_params: [f32; 4],
    pub num_lights: u32,
    pub exposure: f32,
    pub _pre_align_pad: [u32; 2], // offset 1064-1071
    pub _align_pad: [u32; 3],     // offset 1072-1083
    pub environment_blend_t: f32, // offset 1084-1087
    pub environment_preset: u32,  // offset 1088-1091
    pub point_shadows_enabled: u32, // offset 1092-1095
    pub environment_preset_2: u32, // offset 1096-1099
    pub shading_mode: u32,        // offset 1100-1103
    /// inverse(view_proj), computed once per frame on the CPU so fullscreen passes that
    /// unproject NDC→world (volumetric, particle soft-depth) read it instead of recomputing a
    /// full 4×4 inverse per fragment. Appended at the 16-byte-aligned tail (1104) so every
    /// existing field offset — and the partial SceneUniforms copies in other shaders — is
    /// unaffected. offset 1104-1167.
    pub inv_view_proj: [[f32; 4]; 4],
                           // Total: 1168 bytes
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct InstanceRaw {
    pub model: [[f32; 4]; 4],
    pub albedo_color: [f32; 4],
    pub roughness: f32,
    pub metallic: f32,
    pub unlit: f32,
    pub _padding: f32,
    /// xyz = ambient light reaching this surface regardless of the sun (linear, added to the
    /// baked/vertex term BEFORE the albedo multiply, so it tints with the surface); w unused.
    ///
    /// Zero by default — the shading is then bit-for-bit what it was before this field
    /// existed. Only `baked_lit.wgsl` reads it today; it rides `InstanceRaw` rather than the
    /// per-material uniform because it comes from the `Material` COMPONENT, and two entities
    /// can share one material bind group while carrying different components.
    pub ambient: [f32; 4],
    /// xyz = self-emission (linear), added AFTER the albedo/texture multiply so a black
    /// surface can still glow — the same relationship glTF's `emissiveFactor` has to base
    /// colour. w unused. Zero by default.
    pub emissive: [f32; 4],
}

impl InstanceRaw {
    /// Builds the per-instance record from the values a
    /// [`Material`](crate::components::Material) contributes to a draw.
    ///
    /// This is the single place those values become GPU bytes. It takes the values rather
    /// than the `Material` itself for two reasons: the game (`gizmo`) and studio
    /// (`gizmo-studio`) render paths both build instances and have drifted before, and a
    /// `Material` owns a `wgpu::BindGroup`, so taking one would put this mapping out of reach
    /// of any test that does not open a GPU adapter.
    ///
    /// `packed_params` is the anisotropy/clear-coat/subsurface triple packed into one f32 by
    /// the caller (`gbuffer.wgsl` unpacks it); `unlit` is the shading-route flag
    /// (0 = deferred PBR, 1 = unlit/baked-lit, 2 = skybox).
    ///
    /// `ambient` and `emissive` are floored at zero here, not only in the `Material` builders:
    /// the fields on `Material` are `pub`, so a builder-side clamp is not an enforcement
    /// point. Light that subtracts is always a mistake, and it would drive the shader's
    /// `lit + ambient` negative. (`f32::max` also drops NaN, which would otherwise poison the
    /// whole fragment.)
    pub fn new(
        model: [[f32; 4]; 4],
        albedo_color: [f32; 4],
        roughness: f32,
        metallic: f32,
        unlit: f32,
        packed_params: f32,
        ambient: [f32; 3],
        emissive: [f32; 3],
    ) -> Self {
        Self {
            model,
            albedo_color,
            roughness,
            metallic,
            unlit,
            _padding: packed_params,
            ambient: [ambient[0].max(0.0), ambient[1].max(0.0), ambient[2].max(0.0), 0.0],
            emissive: [emissive[0].max(0.0), emissive[1].max(0.0), emissive[2].max(0.0), 0.0],
        }
    }
}

/// Per-material scalar parameters that accompany the textured-PBR bind group
/// (group 1, binding 6).  These carry the glTF factors that modulate the
/// sampled auxiliary maps so that an absent map falls back to the scalar value:
///
/// * `emissive` = emissiveFactor (× KHR_materials_emissive_strength) — multiplied
///   by the (white-default) emissive map, so absent map + zero factor = no emission.
/// * `normal_scale` = glTF normalTexture.scale — scales the tangent-space XY of the
///   (flat-default) normal map, so absent map = unperturbed geometric normal.
/// * `occlusion_strength` = glTF occlusionTexture.strength — lerps the (white-default)
///   AO map toward 1.0, so absent map = no occlusion.
/// * `uv` = KHR_texture_transform (offset / rotation / scale) applied to the UV
///   before every map is sampled; identity when the extension is absent.
///
/// std140 layout: three 16-byte vec4 slots → 48 bytes total.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct MaterialParams {
    /// xyz = emissive factor (linear), w = normal-map scale.
    pub emissive_and_normal_scale: [f32; 4],
    /// x = occlusion (AO) strength; y = UV rotation (radians); zw = UV offset.
    pub occlusion_uv_rot_offset: [f32; 4],
    /// xy = UV scale; z = alpha cutoff (glTF `AlphaMode::Mask`; 0.0 = no cutout,
    /// the g-buffer hard-`discard`s texels with `alpha < cutoff`); w reserved (0.0).
    pub uv_scale: [f32; 4],
}

impl Default for MaterialParams {
    fn default() -> Self {
        // Neutral material: no emission, unit normal scale, unit AO strength,
        // identity UV transform (zero offset, zero rotation, unit scale).
        Self {
            emissive_and_normal_scale: [0.0, 0.0, 0.0, 1.0],
            occlusion_uv_rot_offset: [1.0, 0.0, 0.0, 0.0],
            uv_scale: [1.0, 1.0, 0.0, 0.0],
        }
    }
}

impl MaterialParams {
    pub fn new(
        emissive: [f32; 3],
        normal_scale: f32,
        occlusion_strength: f32,
        uv: UvTransform,
        alpha_cutoff: f32,
    ) -> Self {
        Self {
            emissive_and_normal_scale: [emissive[0], emissive[1], emissive[2], normal_scale],
            occlusion_uv_rot_offset: [occlusion_strength, uv.rotation, uv.offset[0], uv.offset[1]],
            uv_scale: [uv.scale[0], uv.scale[1], alpha_cutoff, 0.0],
        }
    }
}

/// A UV-coordinate transform from `KHR_texture_transform` (offset, rotation in
/// radians, scale). The renderer applies one transform per material (derived
/// from the base-colour texture) to every map's sampled UV — see
/// `asset::loaders`. Its [`Default`] is the identity (no transform).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct UvTransform {
    pub offset: [f32; 2],
    pub rotation: f32,
    pub scale: [f32; 2],
}

impl Default for UvTransform {
    fn default() -> Self {
        Self { offset: [0.0, 0.0], rotation: 0.0, scale: [1.0, 1.0] }
    }
}

impl UvTransform {
    /// True when this transform leaves UVs unchanged (identity).
    pub fn is_identity(&self) -> bool {
        self.offset == [0.0, 0.0] && self.rotation == 0.0 && self.scale == [1.0, 1.0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // These sizes are the Rust half of the GPU layout: bytemuck::Pod requires no interior or
    // trailing padding, and a size that moves means every shader indexing the buffer reads
    // shifted memory. It is only half — this test never opens a `.wgsl` file, and used to claim
    // it was "a contract with the WGSL side" while checking nothing on that side. The other half
    // is `crate::shader_contract`, which parses the shaders and compares their field offsets to
    // `offset_of!` on these structs. Keep both: this one fails first and reads clearly, that one
    // says which shader disagreed.
    #[test]
    fn gpu_struct_sizes_match_the_shader_layout() {
        // 96 (was 92) since the vertex colour became RGBA.
        assert_eq!(std::mem::size_of::<Vertex>(), 96, "Vertex stride");
        assert_eq!(std::mem::size_of::<LightData>(), 64, "LightData = 4×vec4");
        // 128 (was 96) since `ambient` + `emissive` joined it. EVERY shader that indexes
        // `array<InstanceRaw>` must declare the same 8 vec4s or `instances[i]` reads the
        // wrong offsets for i > 0 — see `every_instance_shader_declares_the_full_struct`.
        assert_eq!(std::mem::size_of::<InstanceRaw>(), 128, "InstanceRaw");
        assert_eq!(std::mem::size_of::<MaterialParams>(), 48, "MaterialParams = 3×vec4");
        // The struct's own comment pins the total at 1168 bytes with inv_view_proj
        // appended at the 16-byte-aligned tail (offset 1104).
        assert_eq!(std::mem::size_of::<SceneUniforms>(), 1168, "SceneUniforms total");
    }

    #[test]
    fn vertex_desc_offsets_are_packed_and_in_bounds() {
        let d = Vertex::desc();
        assert_eq!(d.array_stride as usize, std::mem::size_of::<Vertex>());

        // Offsets strictly increase, shader locations run 0..=6, everything fits.
        let mut prev: Option<u64> = None;
        for (i, a) in d.attributes.iter().enumerate() {
            assert_eq!(a.shader_location, i as u32, "attribute {i} shader_location");
            assert!(a.offset < d.array_stride, "attribute {i} offset past stride");
            if let Some(p) = prev {
                assert!(a.offset > p, "attribute {i} offset must increase");
            }
            prev = Some(a.offset);
        }
        // Last attribute is the Float32x4 tangent (16 bytes) and must end exactly
        // at the stride — no hidden gap that would corrupt the next vertex.
        let last = d.attributes.last().unwrap();
        assert_eq!(last.format, wgpu::VertexFormat::Float32x4);
        assert_eq!(last.offset + 16, d.array_stride);
    }

    // The Material lighting knobs are useless if they land in the wrong slot, and the wrong
    // slot is invisible: the shader would read a neighbouring field and quietly shade with it.
    #[test]
    fn instance_new_puts_the_material_knobs_in_their_own_slots() {
        let model = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [7.0, 8.0, 9.0, 1.0]];
        let i = InstanceRaw::new(
            model,
            [0.1, 0.2, 0.3, 0.4],
            0.5,
            0.6,
            1.0,
            777.0,
            [0.01, 0.02, 0.03],
            [4.0, 5.0, 6.0],
        );
        assert_eq!(i.model, model);
        assert_eq!(i.albedo_color, [0.1, 0.2, 0.3, 0.4]);
        assert_eq!(i.roughness, 0.5);
        assert_eq!(i.metallic, 0.6);
        assert_eq!(i.unlit, 1.0);
        assert_eq!(i._padding, 777.0, "packed pbr params ride the old _padding slot");
        // Distinct values on both sides so a swapped pair cannot pass.
        assert_eq!(i.ambient, [0.01, 0.02, 0.03, 0.0], "ambient must not pick up emissive");
        assert_eq!(i.emissive, [4.0, 5.0, 6.0, 0.0], "emissive must not pick up ambient");
    }

    #[test]
    fn instance_with_zero_knobs_is_byte_identical_to_the_pre_knob_record() {
        // The bit-identical-defaults promise, checked rather than hoped: a material that sets
        // neither knob produces the same first 96 bytes it always did, and the 32 new ones are
        // all zero — which is what makes the shader's `(lit + ambient) * base + emissive`
        // collapse back to `lit * base`.
        let i = InstanceRaw::new(
            [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]],
            [1.0, 1.0, 1.0, 1.0],
            0.5,
            0.0,
            0.0,
            0.0,
            [0.0; 3],
            [0.0; 3],
        );
        let bytes = bytemuck::bytes_of(&i);
        assert_eq!(&bytes[96..], &[0u8; 32], "unset knobs must be all-zero bytes");
        assert_eq!(std::mem::offset_of!(InstanceRaw, ambient), 96);
        assert_eq!(std::mem::offset_of!(InstanceRaw, emissive), 112);
    }

    #[test]
    fn instance_new_floors_negative_and_nan_light() {
        let i = InstanceRaw::new(
            [[0.0; 4]; 4],
            [1.0; 4],
            0.0,
            0.0,
            0.0,
            0.0,
            [-1.0, f32::NAN, 0.25],
            [f32::NEG_INFINITY, 2.0, f32::NAN],
        );
        assert_eq!(i.ambient, [0.0, 0.0, 0.25, 0.0], "negative/NaN ambient must be floored");
        assert_eq!(i.emissive, [0.0, 2.0, 0.0, 0.0], "negative/NaN emissive must be floored");
    }

    #[test]
    fn vertex_colour_attribute_carries_alpha() {
        // Vertex alpha reaches the GPU or it does not exist. A Float32x3 here is exactly the
        // bug that made decal/skid-mark layers draw as opaque geometry: the alpha was in the
        // Rust struct's neighbourhood but never described to the pipeline.
        let d = Vertex::desc();
        let colour = d.attributes.iter().find(|a| a.shader_location == 1).expect("colour attribute");
        assert_eq!(
            colour.format,
            wgpu::VertexFormat::Float32x4,
            "vertex colour must be RGBA — a Float32x3 silently drops the alpha channel"
        );
        // The attribute must cover the whole `color` field and nothing else.
        assert_eq!(colour.offset as usize, std::mem::offset_of!(Vertex, color));
        assert_eq!(colour.offset as usize + 16, std::mem::offset_of!(Vertex, normal));
    }

    #[test]
    fn a_default_vertex_is_opaque_white() {
        // The invariant that lets `baked_lit.wgsl` trust `in.color` instead of guessing:
        // "no colour in the source" is normalised to opaque white at import, so a black
        // vertex colour can only ever mean the author asked for black.
        assert_eq!(Vertex::default().color, [1.0, 1.0, 1.0, 1.0]);
    }

    // The instance buffer's element stride used to be policed here, by a hand-written list of ten
    // shader files and a count of `vec4<f32>` occurrences. It now lives in
    // `crate::shader_contract::every_instance_declaration_matches_the_bytes_rust_uploads`, which
    // takes its subjects from the shader directory and its answer from the offsets naga computes —
    // an eleventh shader is covered the day it is added, which a list cannot do.

    #[test]
    fn material_params_new_packs_fields_into_documented_slots() {
        let uv = UvTransform { offset: [0.1, 0.2], rotation: 0.5, scale: [2.0, 3.0] };
        let p = MaterialParams::new([1.0, 2.0, 3.0], 0.7, 0.4, uv, 0.25);
        // emissive.xyz + normal_scale
        assert_eq!(p.emissive_and_normal_scale, [1.0, 2.0, 3.0, 0.7]);
        // occlusion, uv.rotation, uv.offset.x, uv.offset.y
        assert_eq!(p.occlusion_uv_rot_offset, [0.4, 0.5, 0.1, 0.2]);
        // uv.scale.xy, alpha_cutoff, reserved 0
        assert_eq!(p.uv_scale, [2.0, 3.0, 0.25, 0.0]);
    }

    #[test]
    fn material_params_default_is_neutral() {
        let d = MaterialParams::default();
        assert_eq!(d.emissive_and_normal_scale, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(d.occlusion_uv_rot_offset, [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(d.uv_scale, [1.0, 1.0, 0.0, 0.0]);
        // Default UvTransform folded into MaterialParams::new must reproduce Default.
        let via_new = MaterialParams::new([0.0; 3], 1.0, 1.0, UvTransform::default(), 0.0);
        assert_eq!(via_new.emissive_and_normal_scale, d.emissive_and_normal_scale);
        assert_eq!(via_new.occlusion_uv_rot_offset, d.occlusion_uv_rot_offset);
        assert_eq!(via_new.uv_scale, d.uv_scale);
    }

    #[test]
    fn uv_transform_is_identity_only_for_the_identity() {
        assert!(UvTransform::default().is_identity());
        assert!(!UvTransform { offset: [0.1, 0.0], ..Default::default() }.is_identity());
        assert!(!UvTransform { rotation: 0.001, ..Default::default() }.is_identity());
        assert!(!UvTransform { scale: [1.0, 2.0], ..Default::default() }.is_identity());
        // A non-unit scale of exactly 0 is still "not identity".
        assert!(!UvTransform { scale: [0.0, 0.0], ..Default::default() }.is_identity());
    }
}
