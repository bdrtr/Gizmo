//! Oyunun modelleri: hepsi ikilinin içine gömülü (`add_embedded_asset`), dosya aramıyor — telefonda
//! da aynı. Hepsi Kay Lousberg'in CC0 paketlerinden (<https://kaylousberg.com>, atıf zorunlu değil):
//!
//! - `games/assets/kaykit/` — *Character Pack: Adventurers*: oyuncu ve köylüler, derili ve
//!   animasyonlu ([`super::karakter`]);
//! - `games/assets/kaykit-medieval/` — *Medieval Hexagon Pack*: ağaçlar, kayalar, köy evleri, kuyu.
//!
//! Her birinin klasöründe `LICENSE.txt` ve dosyalara ne yapıldığı.
//!
//! Bir model, yalnızca `GltfModel` taşıyan bir varlık: motor onu ilk karede dosyadan kuruyor (bir
//! dosya bir kez okunuyor, sonraki her örnek önbellekten). Penceresiz sınamada kurulmuyor.

use super::*;
use gizmo::renderer::gltf_model::GltfModel;

/// Gömülü modeller: oyunun onları çağırdığı yol, ve baytları.
pub(super) const EMBEDDED: [(&str, &[u8]); 12] = [
    (KNIGHT, include_bytes!("../../assets/kaykit/Knight.glb")),
    (VILLAGERS[0], include_bytes!("../../assets/kaykit/Rogue.glb")),
    (VILLAGERS[1], include_bytes!("../../assets/kaykit/Mage.glb")),
    (VILLAGERS[2], include_bytes!("../../assets/kaykit/Barbarian.glb")),
    (TREES[0], include_bytes!("../../assets/kaykit-medieval/tree_single_A.glb")),
    (TREES[1], include_bytes!("../../assets/kaykit-medieval/tree_single_B.glb")),
    (ROCKS[0].0, include_bytes!("../../assets/kaykit-medieval/rock_single_A.glb")),
    (ROCKS[1].0, include_bytes!("../../assets/kaykit-medieval/rock_single_B.glb")),
    (ROCKS[2].0, include_bytes!("../../assets/kaykit-medieval/rock_single_C.glb")),
    (HOUSES[0].0, include_bytes!("../../assets/kaykit-medieval/building_home_A_red.glb")),
    (HOUSES[1].0, include_bytes!("../../assets/kaykit-medieval/building_home_B_blue.glb")),
    (WELL.0, include_bytes!("../../assets/kaykit-medieval/building_well_blue.glb")),
];

/// Oyuncunun modeli.
pub(super) const KNIGHT: &str = "dunya/kaykit/Knight.glb";
/// Köylülerin modelleri.
pub(super) const VILLAGERS: [&str; 3] = ["dunya/kaykit/Rogue.glb", "dunya/kaykit/Mage.glb", "dunya/kaykit/Barbarian.glb"];
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
