use std::sync::Arc;

/// Which shading route a [`Material`] takes — which pipeline it is drawn with, and what the
/// fragment shader does with it.
///
/// A type rather than a set of booleans on `Material`, because these are not independent knobs:
/// each one implies a pass, a depth mode and a set of inputs that only make sense together. The
/// two backdrop variants document that reasoning at length.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum MaterialType {
    /// Physically based shading through the deferred G-buffer — the default, and what every
    /// lighting feature (IBL, SSAO, SSGI, clustered lights) applies to.
    Pbr,
    /// Drawn with its albedo exactly as given, with no lighting and no shadow. Interface
    /// elements, debug geometry, anything that should not react to the scene.
    Unlit,
    /// Lighting already baked into the vertex colour, plus the sun's shadow — a static level, a
    /// lightmapped world, anything authored lit. Skips the G-buffer and the point lights: one
    /// forward draw per batch instead of eleven, and it still casts into and receives from the
    /// directional cascades.
    BakedLit,
    /// A generated atmospheric sky: the mesh's geometry is ignored, and the gradient is derived
    /// from the sun's direction and colour. Right when there is no sky asset — see
    /// [`Backdrop`](Self::Backdrop) for when there is.
    Skybox,
    /// A painted backdrop — the scene's OWN sky/panorama geometry, drawn from its own texture
    /// and vertex colour behind everything else.
    ///
    /// Three properties travel together and cannot be set apart: drawn before the world,
    /// locked to the camera (translation removed, rotation kept), and never writing depth. See
    /// [`crate::backdrop`] for where each is enforced and why this is a material type rather
    /// than a set of knobs on [`Material`].
    ///
    /// The difference from [`Skybox`](Self::Skybox) is what reaches the screen: `Skybox`
    /// ignores the mesh entirely and generates an atmospheric gradient from the sun colour,
    /// which is right for "there is no sky asset" and wrong for "here is the sky asset".
    Backdrop,
    /// A painted backdrop that stays **where it was authored**.
    ///
    /// Everything [`Backdrop`](Self::Backdrop) is — drawn before the world, never writing depth,
    /// its own texture and vertex colour — minus the camera lock. It exists because the lock is
    /// right for exactly one authoring convention and wrong for the other, and the two are not
    /// distinguishable from inside the renderer:
    ///
    /// - A backdrop authored as a **shell around the origin** (a sky dome, a cube) is meant to
    ///   follow the viewer. That is [`Backdrop`](Self::Backdrop).
    /// - A backdrop authored as **distant geometry placed in the level** — a panorama ring at the
    ///   edge of the map, silhouette panels on the horizon, a matte painting hung behind a set —
    ///   is meant to stay put and to be approached, parallaxed and passed. Locking it to the
    ///   camera drags a kilometre-wide panel onto the lens. That is this.
    ///
    /// The depth treatment is the same and is what makes both safe: pinned to the far plane, so
    /// whatever is in front wins, however near the panel physically is.
    BackdropPlaced,
    /// An animated water surface: waves, reflection and refraction, and a low fixed roughness.
    Water,
    /// The editor's ground grid — lines that fade with distance and never write depth.
    Grid,
    /// A material the game registered: its own pipeline and shader, drawn forward.
    ///
    /// The id comes from [`MaterialRegistry::register`](crate::custom_material::MaterialRegistry::register).
    /// Forward rather than deferred is a budget fact, not a policy — see
    /// [`crate::custom_material`] for the 28-of-32 bytes that decide it.
    ///
    /// This is a variant rather than a wrapper enum around `MaterialType` because it costs nothing:
    /// `Material::material_type` keeps its type, all 56 existing uses keep compiling, and
    /// [`routing`](crate::routing) stays exhaustive — which is the property that module exists for.
    Custom(crate::custom_material::MaterialId),
}

/// How a surface is shaded: its textures (behind the bind group), its PBR scalars, and which
/// route through the renderer it takes.
///
/// Build one with [`Material::new`] and the `with_*` chain rather than by struct literal — the
/// builders clamp their inputs and set the matching [`MaterialType`], and several of them also
/// flip [`Self::is_transparent`] from the albedo's alpha.
///
/// One `Material` is typically shared by many entities: the bind group is behind an `Arc`, and the
/// per-entity values that can still differ ride [`InstanceRaw`](crate::gpu_types::InstanceRaw)
/// instead.
#[derive(Clone)]
pub struct Material {
    /// The texture bind group — albedo, normal, metallic-roughness, occlusion, emissive and the
    /// per-material parameter buffer.
    pub bind_group: Arc<wgpu::BindGroup>,
    /// Base colour tint (linear), multiplied into the albedo texture. `w` is alpha.
    pub albedo: gizmo_math::Vec4,
    /// Perceptual roughness, 0 = mirror, 1 = fully diffuse.
    pub roughness: f32,
    /// Metalness, 0 = dielectric, 1 = metal. Values in between are physically meaningless and
    /// exist only for blend maps.
    pub metallic: f32,
    /// Anisotropic highlight strength — brushed metal, hair, vinyl. 0 = isotropic.
    pub anisotropy: f32,
    /// A second, smoother specular layer over the base — car paint, lacquer. 0 = none.
    pub clear_coat: f32,
    /// How much light scatters through the surface — skin, wax, leaves. 0 = opaque.
    pub subsurface: f32,
    /// Light that reaches the surface no matter where the sun is (linear RGB).
    ///
    /// Read by [`MaterialType::BakedLit`], which is otherwise a bare multiply chain — vertex
    /// colour × instance albedo × texture — with nothing that can lift content authored dark.
    /// It is added to the baked term BEFORE the albedo/texture multiply, so a lifted surface
    /// still shows its own colour instead of washing toward grey:
    ///
    /// ```text
    /// rgb = (baked · shadow + ambient) · albedo · texture + emissive
    /// ```
    ///
    /// The units are the same linear HDR the rest of the pipeline works in, ahead of exposure
    /// and the ACES curve — and that curve has a steep toe (`aces(x) ≈ 0.214·x` as `x → 0`),
    /// so lifting a near-black scene takes more here than the arithmetic suggests. Defaults to
    /// zero, which reproduces the previous shading bit-for-bit.
    ///
    /// Not read by the PBR path, which gets its ambient from the environment/IBL instead.
    pub ambient: gizmo_math::Vec3,
    /// Light the surface emits itself (linear RGB), added AFTER the albedo/texture multiply so
    /// a black surface can still glow — the same relationship glTF's `emissiveFactor` has to
    /// base colour.
    ///
    /// Read by [`MaterialType::BakedLit`]. Defaults to zero. (The textured-PBR path has its
    /// own emissive, from the glTF material's `emissiveFactor` + emissive map; this knob does
    /// not touch it.)
    pub emissive: gizmo_math::Vec3,
    /// Where the albedo texture came from; `None` for an untextured material. Used for
    /// reloading and for recognising an already-loaded texture.
    pub texture_source: Option<String>,
    /// The shading route — see [`MaterialType`].
    pub material_type: MaterialType,
    /// Whether this draw goes into the transparent bucket: sorted back to front, alpha blended,
    /// and not written into the G-buffer.
    pub is_transparent: bool,
    /// Whether back faces are drawn. Needed for anything modelled as a single sheet — foliage,
    /// cloth, a flat pane.
    pub is_double_sided: bool,
    /// Discard texels whose final alpha falls below this. **0 disables it**, which is every
    /// material that does not ask.
    ///
    /// A cut-out — foliage on a quad, a chain-link fence, a pierced railing — is opaque geometry
    /// with holes in it, not a transparent surface. Blending it works only until something
    /// coplanar has to stay put underneath: the sorted pass cannot hold a decal against the
    /// surface it sits on. Discarding keeps the draw in the opaque pass, where depth is written
    /// and order does not matter, so both survive.
    ///
    /// The G-buffer path has had this since the glTF loader learned `AlphaMode::Mask`; this is
    /// the same threshold, reachable from a [`Material`] and honoured by the baked-lit pipeline
    /// as well.
    pub alpha_cutoff: f32,
}

/// Everything a [`Material`] is, minus the one thing a file cannot hold: the bind group.
///
/// **This is the material a scene saves.** `Material` owns an `Arc<wgpu::BindGroup>` — a handle to
/// live GPU state — so it cannot be serialized, and for as long as that was the end of the story a
/// scene round trip silently dropped every material a user had authored: the albedo they picked,
/// the roughness they dialled in, the texture they chose. The editor said the save worked.
///
/// Everything else on `Material` is data, including [`Material::texture_source`], the path the
/// albedo came from — which is what makes rebuilding the bind group on load possible rather than
/// hypothetical.
///
/// The pair is kept in step by two systems in [`crate::material_sync`]: one writes a description
/// for every live material, the other builds a material for every description that has none.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MaterialDesc {
    /// See [`Material::albedo`].
    pub albedo: gizmo_math::Vec4,
    /// See [`Material::roughness`].
    pub roughness: f32,
    /// See [`Material::metallic`].
    pub metallic: f32,
    /// See [`Material::anisotropy`].
    pub anisotropy: f32,
    /// See [`Material::clear_coat`].
    pub clear_coat: f32,
    /// See [`Material::subsurface`].
    pub subsurface: f32,
    /// See [`Material::ambient`].
    pub ambient: gizmo_math::Vec3,
    /// See [`Material::emissive`].
    pub emissive: gizmo_math::Vec3,
    /// See [`Material::texture_source`] — and note this is the field the resolve step reads to
    /// rebuild the bind group. `None` resolves to the renderer's white 1×1, which is what an
    /// untextured material already draws with.
    pub texture_source: Option<String>,
    /// See [`Material::material_type`].
    pub material_type: MaterialType,
    /// See [`Material::is_transparent`].
    pub is_transparent: bool,
    /// See [`Material::is_double_sided`].
    pub is_double_sided: bool,
    /// See [`Material::alpha_cutoff`].
    pub alpha_cutoff: f32,
}

impl From<&Material> for MaterialDesc {
    /// **Destructured exhaustively on purpose**, with no `..`: a field added to [`Material`] and
    /// forgotten here is a compile error rather than a value that silently stops being saved.
    /// That is the failure this whole pair exists to prevent, so it is worth spending the
    /// compiler on rather than a test — and a test could not do it anyway, since building a
    /// `Material` needs a live GPU bind group.
    fn from(m: &Material) -> Self {
        let Material {
            bind_group: _,
            albedo,
            roughness,
            metallic,
            anisotropy,
            clear_coat,
            subsurface,
            ambient,
            emissive,
            texture_source,
            material_type,
            is_transparent,
            is_double_sided,
            alpha_cutoff,
        } = m;
        Self {
            albedo: *albedo,
            roughness: *roughness,
            metallic: *metallic,
            anisotropy: *anisotropy,
            clear_coat: *clear_coat,
            subsurface: *subsurface,
            ambient: *ambient,
            emissive: *emissive,
            texture_source: texture_source.clone(),
            material_type: *material_type,
            is_transparent: *is_transparent,
            is_double_sided: *is_double_sided,
            alpha_cutoff: *alpha_cutoff,
        }
    }
}

impl MaterialDesc {
    /// The material this description asks for, over a bind group the caller has built (from
    /// [`Self::texture_source`], or the renderer's white texture when it is `None`).
    pub fn into_material(self, bind_group: std::sync::Arc<wgpu::BindGroup>) -> Material {
        Material {
            bind_group,
            albedo: self.albedo,
            roughness: self.roughness,
            metallic: self.metallic,
            anisotropy: self.anisotropy,
            clear_coat: self.clear_coat,
            subsurface: self.subsurface,
            ambient: self.ambient,
            emissive: self.emissive,
            texture_source: self.texture_source,
            material_type: self.material_type,
            is_transparent: self.is_transparent,
            is_double_sided: self.is_double_sided,
            alpha_cutoff: self.alpha_cutoff,
        }
    }
}

impl Material {
    /// A neutral PBR material over the given bind group: white albedo, mid roughness, no metal,
    /// and every extra knob at zero.
    pub fn new(bind_group: Arc<wgpu::BindGroup>) -> Self {
        Self {
            bind_group,
            albedo: gizmo_math::Vec4::new(1.0, 1.0, 1.0, 1.0),
            roughness: 0.5,
            metallic: 0.0,
            anisotropy: 0.0,
            clear_coat: 0.0,
            subsurface: 0.0,
            // Zero: the neutral element of both terms, so a material built today shades
            // exactly as it did before these fields existed.
            ambient: gizmo_math::Vec3::ZERO,
            emissive: gizmo_math::Vec3::ZERO,
            texture_source: None,
            material_type: MaterialType::Pbr,
            is_transparent: false,
            is_double_sided: false,
            alpha_cutoff: 0.0,
        }
    }

    /// Configures this as a PBR material.
    /// Note: if `albedo.w < 1.0` is given, `is_transparent` is set to `true` automatically.
    /// `roughness` and `metallic` are clamped into [0.0, 1.0].
    pub fn with_pbr(mut self, albedo: gizmo_math::Vec4, roughness: f32, metallic: f32) -> Self {
        self.albedo = albedo;
        self.roughness = roughness.clamp(0.0, 1.0);
        self.metallic = metallic.clamp(0.0, 1.0);
        self.material_type = MaterialType::Pbr;
        if albedo.w < 1.0 {
            self.is_transparent = true;
        }
        self
    }

    /// Sets [`Material::anisotropy`], clamped into `[0, 1]`.
    pub fn with_anisotropy(mut self, anisotropy: f32) -> Self {
        self.anisotropy = anisotropy.clamp(0.0, 1.0);
        self
    }

    /// Sets [`Material::clear_coat`], clamped into `[0, 1]`.
    pub fn with_clear_coat(mut self, clear_coat: f32) -> Self {
        self.clear_coat = clear_coat.clamp(0.0, 1.0);
        self
    }

    /// Sets [`Material::subsurface`], clamped into `[0, 1]`.
    pub fn with_subsurface(mut self, subsurface: f32) -> Self {
        self.subsurface = subsurface.clamp(0.0, 1.0);
        self
    }

    /// Sets transparency by hand.
    /// Careful: `with_pbr`, `with_unlit` and `with_water` may change transparency automatically
    /// from the albedo's alpha (w). If you want a definite value, call this last in the builder
    /// chain.
    pub fn with_transparent(mut self, transparent: bool) -> Self {
        self.is_transparent = transparent;
        self
    }

    /// Set the cut-out threshold. See [`Self::alpha_cutoff`]; `0.0` turns it off.
    #[must_use]
    pub fn with_alpha_cutoff(mut self, cutoff: f32) -> Self {
        self.alpha_cutoff = cutoff.clamp(0.0, 1.0);
        self
    }

    /// Sets whether back faces are drawn.
    pub fn with_double_sided(mut self, double_sided: bool) -> Self {
        self.is_double_sided = double_sided;
        self
    }

    /// A material that uses light baked into the vertex colour and still receives the sun's
    /// shadow.
    ///
    /// The only difference from `with_unlit` is the shadow. The light is already in the vertex
    /// colour, so PBR is unnecessary — but the shadows of dynamic objects standing in front of
    /// the world cannot be in the file.
    ///
    /// To lift a dark scene use [`with_ambient`](Self::with_ambient) and
    /// [`with_emissive`](Self::with_emissive); both default to zero.
    pub fn with_baked_lit(mut self, albedo: gizmo_math::Vec4) -> Self {
        self.albedo = albedo;
        self.material_type = MaterialType::BakedLit;
        if albedo.w < 1.0 {
            self.is_transparent = true;
        }
        self
    }

    /// Sets [`Material::ambient`] — the light that reaches the surface with the sun out of the
    /// picture. Negative components are clamped to zero (an ambient that subtracts light is
    /// always a bug, and it would make the shader's `lit + ambient` go negative).
    ///
    /// Only [`MaterialType::BakedLit`] reads it; see the field for the exact expression.
    pub fn with_ambient(mut self, ambient: gizmo_math::Vec3) -> Self {
        self.ambient = ambient.max(gizmo_math::Vec3::ZERO);
        self
    }

    /// Sets [`Material::emissive`] — light the surface emits itself, independent of its albedo
    /// and of the sun's shadow. Negative components are clamped to zero.
    ///
    /// Only [`MaterialType::BakedLit`] reads it; see the field for the exact expression.
    pub fn with_emissive(mut self, emissive: gizmo_math::Vec3) -> Self {
        self.emissive = emissive.max(gizmo_math::Vec3::ZERO);
        self
    }

    /// Configures this as an unlit material (unaffected by lighting).
    /// Note: if `albedo.w < 1.0` is given, `is_transparent` is set to `true` automatically.
    pub fn with_unlit(mut self, albedo: gizmo_math::Vec4) -> Self {
        self.albedo = albedo;
        self.material_type = MaterialType::Unlit;
        if albedo.w < 1.0 {
            self.is_transparent = true;
        }
        self
    }

    /// Configures this as a generated atmospheric sky — see [`MaterialType::Skybox`]. The mesh's
    /// own geometry and texture are ignored; for a painted sky use
    /// [`with_backdrop`](Self::with_backdrop).
    pub fn with_skybox(mut self) -> Self {
        self.material_type = MaterialType::Skybox;
        self
    }

    /// Configures this material as a painted backdrop: the mesh's own texture and vertex
    /// colour, drawn before the world, locked to the camera and writing no depth. See
    /// [`MaterialType::Backdrop`].
    ///
    /// `albedo` is a tint multiplied into every pixel; pass [`Vec4::ONE`](gizmo_math::Vec4::ONE)
    /// to get the artwork unmodified. Unlike the other builders this does NOT flip
    /// `is_transparent` on a sub-1.0 alpha, because it does not need to: the backdrop pipeline
    /// always alpha-blends, and `is_transparent` would additionally move the draw into the
    /// transparent bucket — which is exactly where a backdrop must not be.
    pub fn with_backdrop(mut self, albedo: gizmo_math::Vec4) -> Self {
        self.albedo = albedo;
        self.material_type = MaterialType::Backdrop;
        self
    }

    /// Configures this material as a painted backdrop that stays where it was authored — see
    /// [`MaterialType::BackdropPlaced`].
    ///
    /// Same three-line contract as [`with_backdrop`](Self::with_backdrop) — drawn before the
    /// world, its own pixels, no depth write — except that the geometry keeps its place instead
    /// of following the camera. Reach for it when the backdrop is *in* the level rather than
    /// *around* the viewer.
    pub fn with_backdrop_placed(mut self, albedo: gizmo_math::Vec4) -> Self {
        self.albedo = albedo;
        self.material_type = MaterialType::BackdropPlaced;
        self
    }

    /// Configures this as a water material.
    /// `roughness` is set to 0.05 and `metallic` to 0.0, the defaults for water.
    /// Note: if `base_albedo.w < 1.0` is given, `is_transparent` is set to `true`
    /// automatically.
    pub fn with_water(mut self, base_albedo: gizmo_math::Vec4) -> Self {
        self.albedo = base_albedo;
        self.roughness = 0.05;
        self.metallic = 0.0;
        self.material_type = MaterialType::Water;
        if base_albedo.w < 1.0 {
            self.is_transparent = true;
        }
        self
    }

    /// Records where the albedo texture came from. This does not load anything — the bind group
    /// already holds the texture; this is the path it was loaded from.
    pub fn with_texture_source(mut self, path: String) -> Self {
        self.texture_source = Some(path);
        self
    }
}

#[cfg(test)]
mod baked_lit_shading_tests {
    //! What `BakedLit` computes, checked as arithmetic.
    //!
    //! The effect of these knobs is a picture, and this crate can neither open a surface nor
    //! read a pixel — so nothing here claims the scene looks right. What IS checked is the
    //! expression the fragment shader evaluates, mirrored below: that the knobs default to
    //! inert (the bit-identical promise), and that when set they can lift content the bare
    //! multiply chain has no way to lift. `Material` itself owns a `wgpu::BindGroup` and so
    //! cannot be built without a device; these take the field values directly.

    /// Mirror of the tail of `fs_main` in `shaders/baked_lit.wgsl`:
    ///
    /// ```text
    /// lit    = baked · (1 − sun_share + sun_share · vis)
    /// colour = (lit + ambient) · albedo · texture + emissive
    /// ```
    fn baked_lit_rgb(
        vcol: [f32; 3],
        albedo: [f32; 3],
        tex: [f32; 3],
        vis: f32,
        ambient: [f32; 3],
        emissive: [f32; 3],
    ) -> [f32; 3] {
        const SUN_SHARE: f32 = 0.45;
        let shade = 1.0 - SUN_SHARE + SUN_SHARE * vis;
        std::array::from_fn(|c| (vcol[c] * shade + ambient[c]) * (albedo[c] * tex[c]) + emissive[c])
    }

    /// The expression as it stood before the knobs existed: `baked · shade · albedo · texture`.
    fn baked_lit_rgb_before_knobs(vcol: [f32; 3], albedo: [f32; 3], tex: [f32; 3], vis: f32) -> [f32; 3] {
        const SUN_SHARE: f32 = 0.45;
        let shade = 1.0 - SUN_SHARE + SUN_SHARE * vis;
        std::array::from_fn(|c| vcol[c] * shade * (albedo[c] * tex[c]))
    }

    // The property the brief asks to be stated as checked, not hoped: with both knobs at their
    // defaults, every existing scene shades to the same bits it did before.
    #[test]
    fn zero_knobs_reproduce_the_old_expression_exactly() {
        let cases = [
            ([1.0, 1.0, 1.0], [1.0, 1.0, 1.0], [1.0, 1.0, 1.0], 1.0),
            ([0.5, 0.5, 0.5], [1.0, 0.8, 0.6], [0.25, 0.5, 0.75], 0.0),
            ([0.502, 0.376, 0.251], [0.9, 0.9, 0.9], [0.502, 0.502, 0.502], 0.5),
            ([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [1.0, 1.0, 1.0], 1.0),
            ([1.0, 0.0, 0.25], [0.3, 1.0, 0.7], [1.0, 0.1, 0.9], 0.375),
        ];
        for (vcol, albedo, tex, vis) in cases {
            let now = baked_lit_rgb(vcol, albedo, tex, vis, [0.0; 3], [0.0; 3]);
            let before = baked_lit_rgb_before_knobs(vcol, albedo, tex, vis);
            // Bit equality, not an epsilon: adding a literal 0.0 and multiplying by the same
            // factors in the same order must not perturb a single mantissa bit.
            assert_eq!(
                now.map(f32::to_bits),
                before.map(f32::to_bits),
                "default knobs changed the shading for vcol={vcol:?} albedo={albedo:?} \
                 tex={tex:?} vis={vis}"
            );
        }
    }

    // The reported failure mode: a mid-dark surface (vertex colour 128/255 over a 128/255
    // texture) lands at ~0.25 linear before exposure and the ACES toe, and a darker one
    // (64/255 over 64/255) at ~0.06 — from which nothing downstream can recover it, because
    // `BakedLit` had no term that adds.
    #[test]
    fn ambient_lifts_a_scene_the_multiply_chain_cannot() {
        let dark = 64.0 / 255.0;
        let unlifted = baked_lit_rgb([dark; 3], [1.0; 3], [dark; 3], 1.0, [0.0; 3], [0.0; 3]);
        assert!(unlifted[0] < 0.07, "premise: the bare chain really is this dark ({unlifted:?})");

        let lifted = baked_lit_rgb([dark; 3], [1.0; 3], [dark; 3], 1.0, [0.35; 3], [0.0; 3]);
        assert!(
            lifted[0] > unlifted[0] * 2.0,
            "ambient must be able to more than double a dark surface: {unlifted:?} → {lifted:?}"
        );
        // Ambient is incident light, so it goes through the surface: it must scale with albedo,
        // not flood every material to the same grey.
        let dim_surface = baked_lit_rgb([dark; 3], [0.25; 3], [dark; 3], 1.0, [0.35; 3], [0.0; 3]);
        assert!(
            dim_surface[0] < lifted[0],
            "a dark-albedo surface must stay darker than a white one under the same ambient"
        );
        // …and it must keep the surface's hue rather than washing it toward neutral.
        let red = baked_lit_rgb([dark; 3], [1.0, 0.2, 0.2], [1.0; 3], 1.0, [0.4; 3], [0.0; 3]);
        assert!(red[0] > red[1] * 2.0, "white ambient greyed out a red surface: {red:?}");
    }

    #[test]
    fn emissive_glows_through_a_black_surface_and_through_shadow() {
        // Emissive is the surface emitting, so albedo and texture must not gate it — a lit
        // window in a black wall is the whole use case.
        let black_wall = baked_lit_rgb([0.0; 3], [0.0; 3], [0.0; 3], 1.0, [0.0; 3], [1.5, 1.2, 0.6]);
        assert_eq!(black_wall, [1.5, 1.2, 0.6]);
        // Nor may the sun's shadow dim it: a fully shadowed emitter emits the same.
        let shadowed = baked_lit_rgb([0.5; 3], [1.0; 3], [1.0; 3], 0.0, [0.0; 3], [1.5, 1.2, 0.6]);
        let lit = baked_lit_rgb([0.5; 3], [1.0; 3], [1.0; 3], 1.0, [0.0; 3], [1.5, 1.2, 0.6]);
        for c in 0..3 {
            assert!(
                (shadowed[c] - lit[c] - (0.5 * 0.55 - 0.5)).abs() < 1e-5,
                "shadow must move only the baked term, not the emissive one"
            );
        }
    }

    // The shader is text; this mirror is Rust. Nothing else notices if they drift apart.
    #[test]
    fn the_shader_still_computes_the_mirrored_expression() {
        let src = include_str!("../shaders/baked_lit.wgsl");
        assert!(
            src.contains("let colour = (lit + in.inst_ambient) * base + in.inst_emissive;"),
            "baked_lit.wgsl no longer matches `baked_lit_rgb` — update both or neither"
        );
        assert!(
            src.contains("let lit = baked * (1.0 - sun_share + sun_share * vis);"),
            "baked_lit.wgsl's shadow term no longer matches the mirror"
        );
        // ITEM 5: the vertex colour must be taken as authored. A length test that rewrites
        // near-black to white cannot tell "no attribute" from "painted black".
        assert!(
            !src.contains("length(baked)"),
            "the near-black vertex colour is being second-guessed again"
        );
        // ITEM 6: vertex alpha has to survive to the fragment output or nothing downstream
        // can blend with it.
        assert!(
            src.contains("@location(1) color: vec4<f32>"),
            "baked_lit.wgsl dropped the vertex colour's alpha channel"
        );
        assert!(
            src.contains("in.color.a * in.inst_albedo.a * tex.a"),
            "vertex alpha no longer reaches the fragment's output alpha"
        );
    }
}
