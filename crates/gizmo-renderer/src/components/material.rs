use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MaterialType {
    Pbr,
    Unlit,
    Skybox,
    Water,
    Grid,
}

#[derive(Clone)]
pub struct Material {
    pub bind_group: Arc<wgpu::BindGroup>,
    pub albedo: gizmo_math::Vec4,
    pub roughness: f32,
    pub metallic: f32,
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
        if albedo.w < 1.0 { self.is_transparent = true; }
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

    /// Işıklandırmadan etkilenmeyen (Unlit) materyal olarak yapılandırır.
    /// Not: Eğer `albedo.w < 1.0` verilirse `is_transparent` otomatik olarak `true` yapılır.
    pub fn with_unlit(mut self, albedo: gizmo_math::Vec4) -> Self {
        self.albedo = albedo;
        self.material_type = MaterialType::Unlit;
        if albedo.w < 1.0 { self.is_transparent = true; }
        self
    }

    pub fn with_skybox(mut self) -> Self {
        self.material_type = MaterialType::Skybox;
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
        if base_albedo.w < 1.0 { self.is_transparent = true; }
        self
    }

    pub fn with_texture_source(mut self, path: String) -> Self {
        self.texture_source = Some(path);
        self
    }
}
