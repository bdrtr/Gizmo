use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MaterialType {
    Pbr,
    Unlit,
    Water,
    Grid,
}

#[derive(Clone)]
pub struct Material {
    pub bind_group: Arc<wgpu::BindGroup>,
    pub albedo: gizmo_math::Vec4,
    pub roughness: f32,
    pub metallic: f32,
    pub unlit: f32,
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
            unlit: 0.0,
            texture_source: None,
            material_type: MaterialType::Pbr,
            is_transparent: false,
            is_double_sided: false,
        }
    }

    pub fn with_pbr(mut self, albedo: gizmo_math::Vec4, roughness: f32, metallic: f32) -> Self {
        self.albedo = albedo;
        self.roughness = roughness;
        self.metallic = metallic;
        self.unlit = 0.0;
        self.material_type = MaterialType::Pbr;
        self
    }

    pub fn with_transparent(mut self, transparent: bool) -> Self {
        self.is_transparent = transparent;
        self
    }

    pub fn with_double_sided(mut self, double_sided: bool) -> Self {
        self.is_double_sided = double_sided;
        self
    }

    pub fn with_unlit(mut self, albedo: gizmo_math::Vec4) -> Self {
        self.albedo = albedo;
        self.unlit = 1.0;
        self.material_type = MaterialType::Unlit;
        self
    }

    pub fn with_skybox(mut self) -> Self {
        self.unlit = 2.0;
        self.material_type = MaterialType::Unlit;
        self
    }

    pub fn with_water(mut self, base_albedo: gizmo_math::Vec4) -> Self {
        self.albedo = base_albedo;
        self.material_type = MaterialType::Water;
        self
    }

    pub fn with_texture_source(mut self, path: String) -> Self {
        self.texture_source = Some(path);
        self
    }
}
