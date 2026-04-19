use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterialData {
    pub albedo: (f32, f32, f32, f32),
    pub roughness: f32,
    pub metallic: f32,
    pub unlit: f32,
    pub texture_source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityData {
    pub material_source: Option<MaterialData>,
}

fn main() {
    let s = r#"
        (
            material_source: Some((
                albedo: (0.9, 0.1, 0.1, 1.0),
                roughness: 0.5,
                metallic: 0.2,
                unlit: 0.0,
                texture_source: None
            ))
        )
    "#;
    let data: Result<EntityData, _> = ron::from_str(s);
    println!("{:?}", data);
}
