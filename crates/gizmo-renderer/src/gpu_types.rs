use bytemuck::{Pod, Zeroable};

/// The engine's one vertex layout — every mesh, every pipeline, one stride.
///
/// There is deliberately no second, leaner layout for meshes that carry no skin or no tangents:
/// a mesh with two possible strides is a mesh that can be bound to the wrong pipeline, and the
/// bytes saved are dwarfed by the buffers themselves. Absent source attributes are normalised at
/// import instead (see [`Default`]), so a shader may always read all seven.
///
/// [`Vertex::desc`] describes it to wgpu, and the field offsets there come from `offset_of!`
/// rather than a running sum — see the note on that function.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct Vertex {
    /// Position in the mesh's own (model) space.
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
    /// Model-space normal, expected to be unit length.
    pub normal: [f32; 3],
    /// The first UV set. The engine samples every map with these — a second UV set from the
    /// source file is dropped at import.
    pub tex_coords: [f32; 2],
    /// Up to four skin joints influencing this vertex, as indices into the skin's joint palette.
    /// Unskinned meshes carry zeroes here and zero weights, which the vertex shader's
    /// weighted sum turns into the identity.
    pub joint_indices: [u32; 4],
    /// The weights matching [`Vertex::joint_indices`], expected to sum to 1 for a skinned vertex
    /// and to 0 for an unskinned one.
    pub joint_weights: [f32; 4],
    /// Model-space tangent, `w` carrying the bitangent's handedness (±1) as glTF defines it.
    /// Normal mapping needs the sign: mirrored UV islands flip it, and taking it as +1 turns
    /// their normals inside out.
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
    /// This layout as wgpu describes it: the stride, and one attribute per field at
    /// locations 0..=6.
    ///
    /// Offsets are taken from `offset_of!` rather than written out, because a hand-maintained
    /// running sum is one arithmetic slip away from a pipeline that reads normals out of the
    /// tex-coords — see the comment inside.
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

/// One punctual light as the shaders read it: point, spot and directional in a single 64-byte
/// record, discriminated by `params.y`.
///
/// One record type rather than three arrays, because [`SceneUniforms::lights`] is a fixed-length
/// array the shader walks linearly — three kinds in three arrays would mean three loops and three
/// counts. Unused slots are [`LightData::default`], and only the first
/// [`SceneUniforms::num_lights`] are read.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Pod, Zeroable)]
pub struct LightData {
    /// `xyz` = world position, `w` = intensity.
    pub position: [f32; 4],
    /// `rgb` = linear colour, `a` = radius (the distance the falloff reaches zero at).
    pub color: [f32; 4],
    /// `xyz` = direction the light points (spot/directional), `w` = cosine of the inner cone
    /// angle. Cosines rather than angles because the shader compares them against a dot product.
    pub direction: [f32; 4],
    /// `x` = cosine of the outer cone angle, `y` = the light type (0 = point, 1 = spot,
    /// 2 = directional), `zw` unused.
    pub params: [f32; 4],
}

/// Pack the anisotropy/clear-coat/subsurface triple into the one spare `InstanceRaw` slot.
///
/// Private on purpose: [`InstanceRaw::new`] is the only producer, because `gbuffer.wgsl`'s decoder
/// is the only consumer and the two must agree digit for digit. When this was a helper each render
/// path called for itself, they stopped agreeing — see the constructor's docs.
///
/// Two decimal digits per field, not three, and that is a correctness bound rather than taste. An
/// `f32` holds integers exactly only up to 2^24 = 16 777 216 — eight digits, and not even all of
/// those. Three fields of three digits needs nine, and past the limit the step exceeds 1, so the
/// low field rounds *into* its neighbour: with the old layout, anisotropy 1.0 (clamped to 999) and
/// any subsurface ≥ 0.16 rounded 999 up to 1000 and carried, reading back as anisotropy **0.0**
/// with a phantom clear_coat. Which is precisely the overflow the `.min()` clamps were added to
/// stop — f32 rounding simply reintroduced it further up the range, where the endpoint test was
/// not looking.
///
/// Six digits caps the packed value at 999 999, seventeen times under the limit, and the round
/// trip is exact for every combination at the 1 % resolution the subsurface field always had.
fn pack_pbr_params(anisotropy: f32, clear_coat: f32, subsurface: f32) -> f32 {
    (anisotropy * 100.0).floor().min(99.0)
        + 100.0 * (clear_coat * 100.0).floor().min(99.0)
        + 10_000.0 * (subsurface * 100.0).floor().min(99.0)
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

/// Every knob the post-process chain reads, in one uniform block.
///
/// The whole chain shares one block rather than one per effect: the passes run back to back over
/// the same target, and a per-pass block would be four more bind groups to keep in step for no
/// gain. An effect is switched off by zeroing its own scalar, not by skipping the upload.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct PostProcessUniforms {
    /// How much of the bloom blur is added back over the frame. 0 = no bloom.
    pub bloom_intensity: f32,
    /// The luminance a texel must exceed to contribute to bloom.
    pub bloom_threshold: f32,
    /// Linear exposure multiplier applied before tone mapping.
    pub exposure: f32,
    /// Per-channel radial UV offset, in screen fractions. 0 = no aberration.
    pub chromatic_aberration: f32,
    /// How dark the frame's corners get. 0 = no vignette.
    pub vignette_intensity: f32,
    /// Strength of the animated grain. 0 = none.
    pub film_grain_intensity: f32,
    /// Depth of field: the distance, in metres, that is perfectly in focus.
    pub dof_focus_dist: f32,
    /// Depth of field: how far either side of [`Self::dof_focus_dist`] still counts as sharp.
    pub dof_focus_range: f32,
    /// Depth of field: the maximum blur radius, in texels, reached outside that range.
    pub dof_blur_size: f32,
    // Active camera near/far, so DoF depth linearization matches the real projection
    // instead of hardcoded 0.1/1000 (miscalibrated CoC for any other far plane).
    /// The active camera's near plane.
    pub cam_near: f32,
    /// The active camera's far plane.
    pub cam_far: f32,
    // ── Su-altı atmosferi (kamera bir fluid zone içindeyken) ──
    /// 0 = the camera is in air (no effect), 1 = the camera is underwater → depth-based fog is
    /// applied.
    pub underwater: f32,
    /// The underwater fog colour (sea blue-green) plus its density. In WGSL it lines up as a
    /// single `fog: vec4` (rgb + a=density) at offset 48, 16-byte aligned.
    pub fog_r: f32,
    /// Green channel of that fog colour.
    pub fog_g: f32,
    /// Blue channel of that fog colour.
    pub fog_b: f32,
    /// How quickly the underwater fog closes in with distance.
    pub fog_density: f32,
}

/// Uniform block for the shadow pass vertex shader only (one cascade matrix per draw).
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct ShadowVsUniform {
    /// World → clip space for the cascade being rendered.
    pub light_view_proj: [[f32; 4]; 4],
}

/// The per-frame uniform block every shader binds at group 0 — camera, sun, lights, cascades and
/// the environment state.
///
/// Its byte layout is a contract with the WGSL side, and the padding fields below are load-bearing
/// rather than decorative. Two tests hold the two halves of that contract: the size assertion in
/// this file's `tests`, and [`crate::shader_contract`], which parses the shaders and compares
/// naga's computed offsets against `offset_of!` here. A field added anywhere but the tail moves
/// every field after it, in every shader — including the ones that declare only a partial copy of
/// this struct.
///
/// Build it through [`crate::frame_uniforms`] rather than field by field: several fields are
/// derived (the cluster grid, the inverse view-projection) and a second derivation is a second
/// answer.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct SceneUniforms {
    /// The camera's world → clip matrix.
    pub view_proj: [[f32; 4]; 4],
    /// `xyz` = camera world position, `w` unused.
    pub camera_pos: [f32; 4],
    /// `xyz` = direction TO the sun (normalised), `w` = 1 when there is a sun at all — the flag
    /// the shadow and sun-lighting branches test.
    pub sun_direction: [f32; 4],
    /// `rgb` = the sun's linear colour × intensity, `a` unused.
    pub sun_color: [f32; 4],
    /// The punctual lights, of which the first [`Self::num_lights`] are live.
    pub lights: [LightData; crate::frame_uniforms::MAX_LIGHTS],
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
    /// How many entries of [`Self::lights`] are live; the rest are zeroed defaults.
    pub num_lights: u32,
    /// Linear exposure multiplier, for the forward path that tone-maps in place.
    pub exposure: f32,
    /// Explicit padding, to keep the fields below at the offsets the shaders declare.
    pub _pre_align_pad: [u32; 2], // offset 1064-1071
    /// Explicit padding, as above.
    pub _align_pad: [u32; 3], // offset 1072-1083
    /// How far between the two environment presets: 0 = fully [`Self::environment_preset`],
    /// 1 = fully [`Self::environment_preset_2`].
    pub environment_blend_t: f32, // offset 1084-1087
    /// The active sky/environment preset.
    pub environment_preset: u32, // offset 1088-1091
    /// Non-zero when point lights cast shadows this frame.
    pub point_shadows_enabled: u32, // offset 1092-1095
    /// The preset being blended towards. Called `environment_preset_b` on the WGSL side — only
    /// the offset has to agree, not the name.
    pub environment_preset_2: u32, // offset 1096-1099
    /// Debug shading mode: 0 = shade normally, 1 = show world normals, 2 = show albedo.
    ///
    /// The forward and deferred shaders MUST number these identically. They did not once: the
    /// studio's toolbar wrote this uniform every frame while rendering forward, and the forward
    /// shader had no branch for it, so the Normals and Albedo chips produced output byte-identical
    /// to Lit.
    pub shading_mode: u32, // offset 1100-1103
    /// inverse(view_proj), computed once per frame on the CPU so fullscreen passes that
    /// unproject NDC→world (volumetric, particle soft-depth) read it instead of recomputing a
    /// full 4×4 inverse per fragment. Appended at the 16-byte-aligned tail (`464 + 64·MAX_LIGHTS`) so every
    /// existing field offset — and the partial SceneUniforms copies in other shaders — is
    /// unaffected.
    pub inv_view_proj: [[f32; 4]; 4],
    /// Cluster grid dimensions `(x, y, z, 0)` — see [`crate::clustered`].
    ///
    /// The shader needs these to turn a fragment's screen position and view depth into the same
    /// cluster index the CPU assignment used. They are *derived*, not plumbed: the grid is
    /// [`ClusterGrid::default()`](crate::clustered::ClusterGrid::default) on both sides, which is
    /// the one place it may come from — two sources for this number is two different grids.
    pub cluster_dims: [u32; 4],
    /// `[z_scale, z_bias, 0, 0]` for `slice = floor(log(view_depth) * z_scale + z_bias)`, from
    /// [`clustered::depth_params`](crate::clustered::depth_params) over the camera's own near/far.
    pub cluster_depth: [f32; 4],
                           // Total: 560 + 64·MAX_LIGHTS bytes (16 944 at MAX_LIGHTS = 256).
                           // The 64 KiB uniform floor caps this layout at 1015 lights — checked by
                           // `frame_uniforms`'s `the_scene_block_fits_the_uniform_binding_floor_…`.
}

/// One draw's per-instance record: its transform and the material values that do not live in a
/// bind group.
///
/// Instancing is why the split exists — everything here can differ between two instances sharing
/// one material bind group. Build it with [`InstanceRaw::new`], never field by field: the
/// anisotropy/clear-coat/subsurface slot is packed, and assembling it at a call site is how the
/// engine and the studio came to pack it two incompatible ways.
///
/// Every shader that indexes `array<InstanceRaw>` must declare all eight `vec4`s, or `instances[i]`
/// reads the wrong offsets for every `i > 0`.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct InstanceRaw {
    /// Model → world for this instance.
    pub model: [[f32; 4]; 4],
    /// The base colour tint, multiplied into the albedo map. `a` carries alpha.
    pub albedo_color: [f32; 4],
    /// Perceptual roughness, 0 = mirror, 1 = fully diffuse.
    pub roughness: f32,
    /// Metalness, 0 = dielectric, 1 = metal.
    pub metallic: f32,
    /// Which shading route this instance takes: 0 = deferred PBR, 1 = unlit / baked-lit,
    /// 2 = skybox.
    pub unlit: f32,
    /// The anisotropy/clear-coat/subsurface triple, packed into one f32 — the `.w` of the
    /// shader's `pbr` vec4. Called `_padding` until 2026-08-15, which is part of how two render
    /// paths came to pack it two different ways: a slot named "padding" reads as a slot nobody
    /// has to get right. Written only by [`InstanceRaw::new`].
    pub packed_pbr_params: f32,
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
    /// The anisotropy/clear-coat/subsurface triple is packed into one f32 slot **here**, by
    /// [`pack_pbr_params`], rather than by the caller. It was the caller's job until the two
    /// paths were compared: the engine packed two decimal digits per field and studio still
    /// packed three, which is the layout the engine abandoned because it overflows `f32`'s
    /// exact-integer range. `gbuffer.wgsl` decodes the two-digit layout, so studio's instances
    /// would have decoded as a different material entirely — inert only because the editor's
    /// forward pipeline never reaches that shader. A value with one producer and one consumer
    /// should not be assembled at the call site.
    ///
    /// `unlit` is the shading-route flag (0 = deferred PBR, 1 = unlit/baked-lit, 2 = skybox).
    ///
    /// `ambient` and `emissive` are floored at zero here, not only in the `Material` builders:
    /// the fields on `Material` are `pub`, so a builder-side clamp is not an enforcement
    /// point. Light that subtracts is always a mistake, and it would drive the shader's
    /// `lit + ambient` negative. (`f32::max` also drops NaN, which would otherwise poison the
    /// whole fragment.)
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        model: [[f32; 4]; 4],
        albedo_color: [f32; 4],
        roughness: f32,
        metallic: f32,
        unlit: f32,
        anisotropy: f32,
        clear_coat: f32,
        subsurface: f32,
        ambient: [f32; 3],
        emissive: [f32; 3],
    ) -> Self {
        Self {
            model,
            albedo_color,
            roughness,
            metallic,
            unlit,
            packed_pbr_params: pack_pbr_params(anisotropy, clear_coat, subsurface),
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
    /// Packs the glTF factors into the three `vec4` slots the shader reads, in the order
    /// documented on the fields.
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
    /// UV translation, applied after the rotation and scale.
    pub offset: [f32; 2],
    /// UV rotation in radians, about the origin.
    pub rotation: f32,
    /// UV scale, applied first.
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
        // The fixed part is 560 bytes; the light array is the rest. Expressed against
        // MAX_LIGHTS so raising the light ceiling does not require editing three literals in
        // three files (and so that forgetting one is not how the layout drifts).
        assert_eq!(
            std::mem::size_of::<SceneUniforms>(),
            560 + crate::frame_uniforms::MAX_LIGHTS * 64,
            "SceneUniforms total"
        );
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
            // Distinct per field so a swapped pair cannot pass the packed round trip below.
            0.77,
            0.055,
            0.33,
            [0.01, 0.02, 0.03],
            [4.0, 5.0, 6.0],
        );
        assert_eq!(i.model, model);
        assert_eq!(i.albedo_color, [0.1, 0.2, 0.3, 0.4]);
        assert_eq!(i.roughness, 0.5);
        assert_eq!(i.metallic, 0.6);
        assert_eq!(i.unlit, 1.0);
        assert_eq!(
            i.packed_pbr_params,
            pack_pbr_params(0.77, 0.055, 0.33),
            "the constructor packs the triple; no call site may assemble this slot"
        );
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

#[cfg(test)]
mod pbr_pack_tests {
    use super::pack_pbr_params;
    use super::InstanceRaw;

    // Mirror gbuffer.wgsl fs_main's unpack of packed_params (in.inst_pbr.w) exactly.
    fn unpack(w: f32) -> (f32, f32, f32) {
        let subsurface = (w / 10_000.0).floor() / 100.0;
        let rem = w - (w / 10_000.0).floor() * 10_000.0;
        let clear_coat = (rem / 100.0).floor() / 100.0;
        let anisotropy = (rem - (rem / 100.0).floor() * 100.0) / 100.0;
        (anisotropy, clear_coat, subsurface)
    }

    /// The packer is private and the constructor is its only caller, so the round trip that
    /// actually ships is the one through `InstanceRaw::new` — including that the three values
    /// arrive in the right order. A swapped pair here is a material that shades as a different
    /// material, which is what the editor's own copy of this packing was doing.
    #[test]
    fn the_constructor_packs_what_the_shader_decodes() {
        let i = InstanceRaw::new(
            [[0.0; 4]; 4],
            [1.0; 4],
            0.5,
            0.0,
            0.0,
            0.3,
            0.7,
            0.05,
            [0.0; 3],
            [0.0; 3],
        );
        let (aniso, cc, ss) = unpack(i.packed_pbr_params);
        assert!((aniso - 0.3).abs() <= 0.011, "anisotropy {aniso}");
        assert!((cc - 0.7).abs() <= 0.011, "clear_coat {cc}");
        assert!((ss - 0.05).abs() <= 0.011, "subsurface {ss}");
    }

    // Regression: the legal clamped endpoint 1.0 must NOT overflow its 3-digit field into
    // the neighbour. Before the clamp, clear_coat=1.0 packed as floor(1000)*1000
    // which carried into the subsurface field (three-digit layout, since abandoned) → clear_coat read back as 0 and a phantom
    // subsurface≈0.01 appeared. Symmetric for anisotropy=1.0.
    #[test]
    fn endpoint_one_does_not_overflow_into_neighbours() {
        // clear_coat = 1.0, others 0 → clear_coat must survive (~0.999), no phantom subsurface.
        let (aniso, cc, ss) = unpack(pack_pbr_params(0.0, 1.0, 0.0));
        assert!(cc >= 0.99, "clear_coat=1.0 lost (got {cc})");
        assert_eq!(ss, 0.0, "clear_coat=1.0 leaked a phantom subsurface ({ss})");
        assert_eq!(aniso, 0.0, "clear_coat=1.0 leaked into anisotropy ({aniso})");

        // anisotropy = 1.0 → survives, no leak into clear_coat.
        let (aniso, cc, ss) = unpack(pack_pbr_params(1.0, 0.0, 0.0));
        assert!(aniso >= 0.99, "anisotropy=1.0 lost (got {aniso})");
        assert_eq!(cc, 0.0, "anisotropy=1.0 leaked into clear_coat ({cc})");
        assert_eq!(ss, 0.0, "anisotropy=1.0 leaked into subsurface ({ss})");
    }

    // Ordinary mid-range values round-trip within the decimal-packing resolution.
    #[test]
    fn mid_range_values_round_trip() {
        let (aniso, cc, ss) = unpack(pack_pbr_params(0.3, 0.7, 0.05));
        assert!((aniso - 0.3).abs() <= 0.011, "aniso {aniso}");
        assert!((cc - 0.7).abs() <= 0.011, "clear_coat {cc}");
        assert!((ss - 0.05).abs() <= 0.011, "subsurface {ss}");
    }

    /// The endpoint test above only ever tried subsurface 0, and that is where this hid.
    ///
    /// Every field must survive every combination of the other two, which the old three-digit
    /// layout did not: `anisotropy = 1.0` with `subsurface ≥ 0.16` pushed the packed value past
    /// 2^24, where an f32's step exceeds 1, so the clamped 999 rounded to 1000 and carried into
    /// the clear-coat field. Anisotropy read back as **0.0** — full anisotropy rendering as none.
    /// Swept rather than spot-checked, because a spot check is exactly what missed it.
    #[test]
    fn every_combination_round_trips() {
        for i in 0..=100 {
            for j in (0..=100).step_by(5) {
                for k in (0..=100).step_by(5) {
                    let (a, c, s) = (i as f32 / 100.0, j as f32 / 100.0, k as f32 / 100.0);
                    let (da, dc, ds) = unpack(pack_pbr_params(a, c, s));
                    for (got, want, name) in
                        [(da, a, "anisotropy"), (dc, c, "clear_coat"), (ds, s, "subsurface")]
                    {
                        assert!(
                            (got - want).abs() <= 0.011,
                            "{name} {want} came back as {got} (a={a} c={c} s={s}) — a field is \
                             carrying into its neighbour"
                        );
                    }
                }
            }
        }
    }
}
