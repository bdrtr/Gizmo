use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[non_exhaustive]
pub enum MaterialType {
    Pbr,
    Unlit,
    /// Lighting already baked into the vertex colour, plus the sun's shadow — a static level, a
    /// lightmapped world, anything authored lit. Skips the G-buffer and the point lights: one
    /// forward draw per batch instead of eleven, and it still casts into and receives from the
    /// directional cascades.
    BakedLit,
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
    Water,
    Grid,
}

#[derive(Clone)]
pub struct Material {
    pub bind_group: Arc<wgpu::BindGroup>,
    pub albedo: gizmo_math::Vec4,
    pub roughness: f32,
    pub metallic: f32,
    pub anisotropy: f32,
    pub clear_coat: f32,
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
    pub texture_source: Option<String>,
    pub material_type: MaterialType,
    pub is_transparent: bool,
    pub is_double_sided: bool,
}

impl Material {
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
        }
    }

    /// PBR materyali olarak yapılandırır.
    /// Not: Eğer `albedo.w < 1.0` verilirse `is_transparent` otomatik olarak `true` yapılır.
    /// `roughness` ve `metallic` değerleri [0.0, 1.0] aralığına sınırlandırılır.
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

    pub fn with_anisotropy(mut self, anisotropy: f32) -> Self {
        self.anisotropy = anisotropy.clamp(0.0, 1.0);
        self
    }

    pub fn with_clear_coat(mut self, clear_coat: f32) -> Self {
        self.clear_coat = clear_coat.clamp(0.0, 1.0);
        self
    }

    pub fn with_subsurface(mut self, subsurface: f32) -> Self {
        self.subsurface = subsurface.clamp(0.0, 1.0);
        self
    }

    /// Saydamlığı manuel olarak belirler.
    /// Uyarı: `with_pbr`, `with_unlit` veya `with_water` metodları albedo'nun alpha değerine (w) bakarak
    /// saydamlığı otomatik değiştirebilir. Kesin bir saydamlık istiyorsanız, bu metodu builder zincirinin en sonunda çağırın.
    pub fn with_transparent(mut self, transparent: bool) -> Self {
        self.is_transparent = transparent;
        self
    }

    pub fn with_double_sided(mut self, double_sided: bool) -> Self {
        self.is_double_sided = double_sided;
        self
    }

    /// Vertex renginde pişmiş ışığı kullanan, güneşin gölgesini alan materyal.
    ///
    /// `with_unlit`'ten farkı tek şey: gölge. Işık zaten vertex renginde olduğu için PBR'a gerek
    /// yok, ama dünyanın önünde duran dinamik nesnelerin gölgesi dosyada olamaz.
    ///
    /// Karanlık sahneleri açmak için [`with_ambient`](Self::with_ambient) ve
    /// [`with_emissive`](Self::with_emissive); ikisi de varsayılan olarak sıfırdır.
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
    ///
    /// Yüzeye güneşten bağımsız ulaşan ışık. Karanlık pişmiş sahneleri açmanın yolu.
    pub fn with_ambient(mut self, ambient: gizmo_math::Vec3) -> Self {
        self.ambient = ambient.max(gizmo_math::Vec3::ZERO);
        self
    }

    /// Sets [`Material::emissive`] — light the surface emits itself, independent of its albedo
    /// and of the sun's shadow. Negative components are clamped to zero.
    ///
    /// Only [`MaterialType::BakedLit`] reads it; see the field for the exact expression.
    ///
    /// Yüzeyin kendi yaydığı ışık; albedo'dan ve gölgeden bağımsızdır.
    pub fn with_emissive(mut self, emissive: gizmo_math::Vec3) -> Self {
        self.emissive = emissive.max(gizmo_math::Vec3::ZERO);
        self
    }

    /// Işıklandırmadan etkilenmeyen (Unlit) materyal olarak yapılandırır.
    /// Not: Eğer `albedo.w < 1.0` verilirse `is_transparent` otomatik olarak `true` yapılır.
    pub fn with_unlit(mut self, albedo: gizmo_math::Vec4) -> Self {
        self.albedo = albedo;
        self.material_type = MaterialType::Unlit;
        if albedo.w < 1.0 {
            self.is_transparent = true;
        }
        self
    }

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
    ///
    /// Boyalı arka plan (gökyüzü/panorama): mesh'in kendi dokusu ve vertex rengi, dünyadan
    /// önce, kameraya kilitli, derinlik yazmadan çizilir.
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
    ///
    /// Kameraya kilitlenmeyen boyalı arka plan: dünyada durduğu yerde çizilir.
    pub fn with_backdrop_placed(mut self, albedo: gizmo_math::Vec4) -> Self {
        self.albedo = albedo;
        self.material_type = MaterialType::BackdropPlaced;
        self
    }

    /// Su materyali olarak yapılandırır.
    /// `roughness` 0.05, `metallic` 0.0 olarak varsayılan su değerlerine ayarlanır.
    /// Not: Eğer `base_albedo.w < 1.0` verilirse `is_transparent` otomatik olarak `true` yapılır.
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
