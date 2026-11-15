//! Oyunun modelleri ve dokuları: hepsi ikilinin içine gömülü (`add_embedded_asset`), dosya aramıyor — telefonda
//! da aynı. Hepsi Kay Lousberg'in CC0 paketlerinden (<https://kaylousberg.com>, atıf zorunlu değil):
//!
//! - `games/assets/kaykit/` — *Character Pack: Adventurers*: oyuncu ve köylüler, derili ve
//!   animasyonlu ([`super::karakter`]);
//! - `games/assets/kaykit-medieval/` — *Medieval Hexagon Pack*: ağaçlar, kayalar, köy evleri, kuyu;
//! - `games/assets/kaykit-city/` — *City Builder Bits*: arabalar.
//!
//! Kayık Kenney'nin CC0 *Watercraft Kit*'inden (<https://kenney.nl>): `games/assets/kenney/`.
//!
//! Hayvanlar Quaternius'un CC0 paketlerinden (<https://quaternius.com>): `games/assets/quaternius/`.
//!
//! Arazinin dokuları ambientCG'den (<https://ambientcg.com>, CC0): `games/assets/ambientcg/`.
//!
//! Her birinin klasöründe `LICENSE.txt` ve dosyalara ne yapıldığı.
//!
//! Bir model, yalnızca `GltfModel` taşıyan bir varlık: motor onu ilk karede dosyadan kuruyor (bir
//! dosya bir kez okunuyor, sonraki her örnek önbellekten). Penceresiz sınamada kurulmuyor.

use super::*;
use gizmo::renderer::gltf_model::GltfModel;

/// Gömülü modeller: oyunun onları çağırdığı yol, ve baytları.
pub(super) const EMBEDDED: [(&str, &[u8]); 23] = [
    (KNIGHT.0, include_bytes!("../../assets/kaykit/Knight.glb")),
    (VILLAGERS[0].0, include_bytes!("../../assets/kaykit/Rogue.glb")),
    (VILLAGERS[1].0, include_bytes!("../../assets/kaykit/Mage.glb")),
    (VILLAGERS[2].0, include_bytes!("../../assets/kaykit/Barbarian.glb")),
    (SHEEP.0, include_bytes!("../../assets/quaternius/Sheep.glb")),
    (DEER.0, include_bytes!("../../assets/quaternius/Deer.glb")),
    (CARS[0], include_bytes!("../../assets/kaykit-city/car_sedan.glb")),
    (CARS[1], include_bytes!("../../assets/kaykit-city/car_hatchback.glb")),
    (CARS[2], include_bytes!("../../assets/kaykit-city/car_taxi.glb")),
    (BOAT, include_bytes!("../../assets/kenney/boat-speed-a.glb")),
    (TREES[0], include_bytes!("../../assets/kaykit-medieval/tree_single_A.glb")),
    (TREES[1], include_bytes!("../../assets/kaykit-medieval/tree_single_B.glb")),
    (ROCKS[0].0, include_bytes!("../../assets/kaykit-medieval/rock_single_A.glb")),
    (ROCKS[1].0, include_bytes!("../../assets/kaykit-medieval/rock_single_B.glb")),
    (ROCKS[2].0, include_bytes!("../../assets/kaykit-medieval/rock_single_C.glb")),
    (HOUSES[0].0, include_bytes!("../../assets/kaykit-medieval/building_home_A_red.glb")),
    (HOUSES[1].0, include_bytes!("../../assets/kaykit-medieval/building_home_B_blue.glb")),
    (WELL.0, include_bytes!("../../assets/kaykit-medieval/building_well_blue.glb")),
    (GROUND[0], include_bytes!("../../assets/ambientcg/grass.jpg")),
    (GROUND[1], include_bytes!("../../assets/ambientcg/dirt.jpg")),
    (GROUND[2], include_bytes!("../../assets/ambientcg/rock.jpg")),
    (GROUND[3], include_bytes!("../../assets/ambientcg/snow.jpg")),
    (GROUND[4], include_bytes!("../../assets/ambientcg/sand.jpg")),
];

/// Arazinin dokuları: çim, toprak, kaya, kar, kum.
pub(super) const GROUND: [&str; 5] =
    ["dunya/zemin/grass.jpg", "dunya/zemin/dirt.jpg", "dunya/zemin/rock.jpg", "dunya/zemin/snow.jpg", "dunya/zemin/sand.jpg"];

/// Arazinin gölgelendiricisi (`arazi.wgsl`'in başı neyin nerede olduğunu anlatıyor).
pub(super) const GROUND_WGSL: &str = include_str!("arazi.wgsl");

/// Göllerin gölgelendiricisi (`su.wgsl`).
pub(super) const WATER_WGSL: &str = include_str!("su.wgsl");

/// Göllerin materyali: ışık geçiren (motor altını kırarak gösteriyor, derinliğe göre karartıyor),
/// suyun kırma indisiyle, `surface`'ta kayıtlı su yüzeyiyle.
pub(super) struct WaterLook(pub(super) MaterialDesc);

pub(super) fn water_look(surface: gizmo::renderer::custom_material::SurfaceId) -> WaterLook {
    let mut look = MaterialDesc::new().with_pbr(Vec4::ONE, 0.04, 0.0).with_transmission(1.0).with_ior(1.33).with_thickness(0.4);
    look.material_type = gizmo::renderer::components::MaterialType::CustomSurface(surface);
    WaterLook(look)
}

/// Arazinin materyali, motorun özel yüzeyi kaydedilmişse: hücreler bununla çiziliyor.
pub(super) struct GroundLook(pub(super) MaterialDesc);

/// `surface`'ta kayıtlı arazi gölgelendiricisi için materyal: yuvalara dokular, gölgelendiricinin
/// beklediği sırayla.
pub(super) fn ground_look(surface: gizmo::renderer::custom_material::SurfaceId) -> GroundLook {
    let maps = gizmo::renderer::components::MaterialMaps::default()
        .with_normal(GROUND[1], 1.0)
        .with_metallic_roughness(GROUND[3])
        .with_emissive(GROUND[2], Vec3::ONE)
        .with_occlusion(GROUND[4], 1.0);
    let mut look = MaterialDesc::new().with_pbr(Vec4::ONE, 0.95, 0.0).with_texture_source(GROUND[0].to_string()).with_maps(maps);
    look.material_type = gizmo::renderer::components::MaterialType::CustomSurface(surface);
    GroundLook(look)
}

/// Oyuncunun modeli, ve kendi birimindeki boyu (ayaktan başın tepesine): KayKit'in iri başlı
/// karakterleri.
pub(super) const KNIGHT: (&str, f32) = ("dunya/kaykit/Knight.glb", 2.31);
/// Köylülerin modelleri.
pub(super) const VILLAGERS: [(&str, f32); 3] =
    [("dunya/kaykit/Rogue.glb", 2.31), ("dunya/kaykit/Mage.glb", 2.31), ("dunya/kaykit/Barbarian.glb", 2.31)];
/// Hayvanlar, ve kendi birimlerindeki boyları (yürürken, ayaktan başın tepesine; ölçülen).
pub(super) const SHEEP: (&str, f32) = ("dunya/quaternius/Sheep.glb", 4.2);
pub(super) const DEER: (&str, f32) = ("dunya/quaternius/Deer.glb", 4.26);
/// Arabalar: aynı tekerlek düzeni (iz 0,352, dingil arası 0,5, yarıçap 0,072, kendi biriminde).
pub(super) const CARS: [&str; 3] = ["dunya/city/car_sedan.glb", "dunya/city/car_hatchback.glb", "dunya/city/car_taxi.glb"];
/// Kayık: 1,78 × 3,38 birimlik sürat teknesi.
pub(super) const BOAT: &str = "dunya/kenney/boat-speed-a.glb";
/// Arabaların büyütmesi: 0,94 birimlik sedan 4,3 m.
pub(super) const CAR_SCALE: f32 = 4.6;

/// Bir araba modelinin tekerlek düğümlerini aracın tekerleklerine bağlar, model kurulduğunda:
/// adlarının sonu (`_wheel_front_left` …) motorun `four_wheeled` sırası (ön sol, ön sağ, arka sol,
/// arka sağ). Model 180° dönük; onun solu aracın solu, önü aracın önü.
pub(super) fn bind_wheels(world: &mut World) {
    use gizmo::physics::WheelVisual;
    const ORDER: [&str; 4] = ["_wheel_front_left", "_wheel_front_right", "_wheel_rear_left", "_wheel_rear_right"];
    let found: Vec<(Entity, usize)> = world
        .query::<(Entity, &gizmo::core::EntityName)>()
        .iter()
        .filter_map(|(e, n)| ORDER.iter().position(|suffix| n.0.ends_with(suffix)).map(|i| (e, i)))
        .filter(|(e, _)| world.get::<WheelVisual>(*e).is_none())
        .collect();
    for (e, i) in found {
        world.add_component(e, WheelVisual::new(i));
    }
}

/// Ağaçlar: kendi birimlerinde 1,1 boyunda (dibi 0,1 toprağın altında).
pub(super) const TREES: [&str; 2] = ["dunya/medieval/tree_single_A.glb", "dunya/medieval/tree_single_B.glb"];
pub(super) const TREE_HEIGHT: f32 = 1.1;
/// Kayalar ve kendi birimlerindeki yarı genişlikleri, boyları: (x, y, z).
pub(super) const ROCKS: [(&str, Vec3); 3] = [
    ("dunya/medieval/rock_single_A.glb", Vec3::new(0.15, 0.07, 0.15)),
    ("dunya/medieval/rock_single_B.glb", Vec3::new(0.14, 0.13, 0.13)),
    ("dunya/medieval/rock_single_C.glb", Vec3::new(0.18, 0.19, 0.17)),
];
/// Köy evleri ve kuyu, kendi birimlerindeki yarı genişlikleri ve boyları: (x, y, z), dibi 0'da.
pub(super) const HOUSES: [(&str, Vec3); 2] = [
    ("dunya/medieval/building_home_A_red.glb", Vec3::new(0.4, 0.93, 0.47)),
    ("dunya/medieval/building_home_B_blue.glb", Vec3::new(0.44, 1.28, 0.55)),
];
pub(super) const WELL: (&str, Vec3) = ("dunya/medieval/building_well_blue.glb", Vec3::new(0.34, 0.83, 0.38));

/// `parent`'ın çocuğu olarak bir model: `at`'ta, `turn` dönük, `scale` ölçekli.
pub(super) fn place(world: &mut World, parent: Entity, model: &str, at: Vec3, turn: Quat, scale: Vec3) -> Entity {
    let e = world.spawn_bundle((Transform::new(at).with_rotation(turn).with_scale(scale), GlobalTransform::default(), GltfModel::new(model)));
    world.add_child(parent, e);
    e
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo::renderer::Renderer;

    /// The terrain's and the lakes' shaders compose into the engine's: the windowed game panics at start-up when it
    /// does not, and nothing headless would notice. Also the model and texture table: every path is
    /// one the game asks for, every file decodes.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn the_ground_shader_composes() {
        pollster::block_on(async {
            if !Renderer::headless_adapter_available().await {
                eprintln!("skipping: no GPU adapter");
                return;
            }
            let mut renderer = Renderer::new_headless(64, 64, None).await;
            for (label, wgsl) in [("dunya_arazi", GROUND_WGSL), ("dunya_su", WATER_WGSL)] {
                if let Err(e) = renderer.add_surface_material(label, wgsl) {
                    panic!("{label}: {e}");
                }
            }
        });
    }

    /// Every embedded texture decodes, and is a square power of two (mipmaps, tiling).
    #[test]
    fn the_ground_textures_decode() {
        for path in GROUND {
            let bytes = EMBEDDED.iter().find(|(p, _)| *p == path).map(|(_, b)| *b).unwrap_or_else(|| panic!("{path} is not embedded"));
            let image = image_size(bytes).unwrap_or_else(|| panic!("{path} is not a JPEG"));
            assert!(image.0 == image.1 && image.0.is_power_of_two(), "{path} is {image:?}");
        }
    }

    /// A JPEG's size from its SOF marker — the crate has no image decoder of its own.
    fn image_size(b: &[u8]) -> Option<(u32, u32)> {
        let mut i = 2;
        while i + 9 < b.len() {
            if b[i] != 0xFF {
                return None;
            }
            let marker = b[i + 1];
            let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
            if (0xC0..=0xC2).contains(&marker) {
                let h = u16::from_be_bytes([b[i + 5], b[i + 6]]) as u32;
                let w = u16::from_be_bytes([b[i + 7], b[i + 8]]) as u32;
                return Some((w, h));
            }
            i += 2 + len;
        }
        None
    }
}
