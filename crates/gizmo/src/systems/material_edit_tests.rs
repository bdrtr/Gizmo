//! A material edited at run time keeps the edit, whichever of the pair the game edited.
//!
//! `Material` (with its GPU bind group) and `MaterialDesc` (what a scene file holds) describe the
//! same thing, and `sync_material_descriptions` runs every frame to keep them together.

use crate::core::World;
use crate::math::Vec4;
use crate::renderer::asset::AssetManager;
use crate::renderer::components::{Material, MaterialDesc};
use crate::renderer::material_sync::sync_material_descriptions;
use crate::renderer::Renderer;

/// A world holding one red material, synced once (so it has its description), and the entity.
async fn one_red_material(renderer: &Renderer) -> (World, u32) {
    let mut assets = AssetManager::new();
    let tex = assets.create_white_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout);
    let mut world = World::new();
    let e = world.spawn();
    world.add_component(e, Material::new(tex).with_pbr(Vec4::new(0.9, 0.1, 0.1, 1.0), 0.5, 0.0));
    sync_material_descriptions(&mut world);
    (world, e.id())
}

fn gpu_or_skip(name: &str) -> bool {
    if !pollster::block_on(Renderer::headless_adapter_available()) {
        eprintln!("skipping {name}: no GPU adapter available");
        return false;
    }
    if pollster::block_on(Renderer::headless_adapter_is_software()) {
        eprintln!("skipping {name}: software adapter");
        return false;
    }
    true
}

const GREEN: Vec4 = Vec4::new(0.1, 0.8, 0.2, 1.0);

/// The game recolours through `MaterialDesc` — what `kapi` and `kopru` did — and the material
/// it draws with follows.
#[test]
fn an_edited_description_reaches_the_material() {
    let _gpu = crate::test_gpu::gpu_lock();
    if !gpu_or_skip("an_edited_description_reaches_the_material") {
        return;
    }
    pollster::block_on(async {
        let renderer = crate::test_gpu::headless_renderer(64, 64).await;
        let (mut world, e) = one_red_material(&renderer).await;
        world.borrow_mut::<MaterialDesc>().get_mut(e).unwrap().albedo = GREEN;
        for _ in 0..2 {
            sync_material_descriptions(&mut world);
        }
        assert_eq!(world.borrow::<Material>().get(e).unwrap().albedo, GREEN, "the material is drawn red still");
        assert_eq!(world.borrow::<MaterialDesc>().get(e).unwrap().albedo, GREEN, "the edit was written over");
    });
}

/// And through `Material` — what `kale` and `ritim` do — the description follows.
#[test]
fn an_edited_material_reaches_the_description() {
    let _gpu = crate::test_gpu::gpu_lock();
    if !gpu_or_skip("an_edited_material_reaches_the_description") {
        return;
    }
    pollster::block_on(async {
        let renderer = crate::test_gpu::headless_renderer(64, 64).await;
        let (mut world, e) = one_red_material(&renderer).await;
        world.borrow_mut::<Material>().get_mut(e).unwrap().albedo = GREEN;
        for _ in 0..2 {
            sync_material_descriptions(&mut world);
        }
        assert_eq!(world.borrow::<MaterialDesc>().get(e).unwrap().albedo, GREEN);
        assert_eq!(world.borrow::<Material>().get(e).unwrap().albedo, GREEN, "the edit was written over");
    });
}

/// A description that names a different texture cannot be applied without the GPU: the material
/// is dropped so the resolve pass builds it again from the description, rather than drawing on
/// with the old texture.
#[test]
fn a_new_texture_in_the_description_rebuilds_the_material() {
    let _gpu = crate::test_gpu::gpu_lock();
    if !gpu_or_skip("a_new_texture_in_the_description_rebuilds_the_material") {
        return;
    }
    pollster::block_on(async {
        let renderer = crate::test_gpu::headless_renderer(64, 64).await;
        let (mut world, e) = one_red_material(&renderer).await;
        world.borrow_mut::<MaterialDesc>().get_mut(e).unwrap().texture_source = Some("grass.png".into());
        sync_material_descriptions(&mut world);
        assert!(world.borrow::<Material>().get(e).is_none(), "the old bind group is still drawn");
        assert_eq!(
            world.borrow::<MaterialDesc>().get(e).unwrap().texture_source.as_deref(),
            Some("grass.png"),
            "the description is what the rebuild reads"
        );
    });
}
