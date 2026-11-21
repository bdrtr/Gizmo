//! The transform-track player, driven by the engine's frame (`advance_frame_clocks`).

use super::advance_frame_clocks;
use crate::core::World;
use crate::math::Vec3;
use crate::physics::components::Transform;
use crate::renderer::keyed_animation::{AnimationClip, Keyframes, KeyedAnimationPlayer, Track};

/// A door sliding 10 m along x over a second, on its own entity (an empty target name).
fn door() -> (World, gizmo_core::Entity) {
    let mut world = World::new();
    let mut time = gizmo_core::time::Time::new();
    time.update(0.05);
    world.insert_resource(time);
    let track = Track::new("", vec![0.0, 1.0], Keyframes::Translation(vec![Vec3::ZERO, Vec3::new(10.0, 0.0, 0.0)])).expect("valid");
    let mut clip = AnimationClip::default();
    clip.name = "open".into();
    clip.tracks.push(track);
    let mut player = KeyedAnimationPlayer::new().looping(false);
    player.play(std::sync::Arc::new(clip));
    let e = world.spawn();
    world.add_component(e, Transform::new(Vec3::ZERO));
    world.add_component(e, player);
    (world, e)
}

fn x(world: &World, e: gizmo_core::Entity) -> f32 {
    world.get::<Transform>(e).map_or(f32::NAN, |t| t.position.x)
}

/// **The frame plays it, and a paused one follows its playhead.** Four frames of 0.05 s slide the
/// door 2 m; paused and set to 0.75 s, the next frame puts it at 7.5 m and the playhead stays.
/// Nothing drove this player before 2026-11-16 unless a game scheduled it by hand.
#[test]
fn the_frame_plays_a_transform_clip_and_a_paused_one_follows_its_playhead() {
    let (mut world, door) = door();
    for _ in 0..4 {
        advance_frame_clocks(&mut world);
    }
    assert!((x(&world, door) - 2.0).abs() < 1.0e-4, "four frames of 0.05 s: {}", x(&world, door));
    if let Some(mut p) = world.query_mut::<&mut KeyedAnimationPlayer>().get_mut(door) {
        p.pause();
        p.elapsed_time = 0.75;
    }
    advance_frame_clocks(&mut world);
    assert!((x(&world, door) - 7.5).abs() < 1.0e-4, "scrubbed to 0.75 s: {}", x(&world, door));
    assert_eq!(world.get::<KeyedAnimationPlayer>(door).map(|p| p.elapsed_time), Some(0.75), "paused, it stays");
}

/// **A scene keeps the clip and the playhead.** Saved and reopened, the door plays on from where
/// it was.
#[cfg(feature = "scene")]
#[test]
fn a_scene_keeps_a_transform_clip() {
    let (mut world, door) = door();
    for _ in 0..4 {
        advance_frame_clocks(&mut world);
    }
    let registry = crate::full_scene_registry();
    let text = crate::scene::SceneData::of_entities(&world, vec![door], &registry).to_ron_string().expect("saves");
    // Saved under the player's own name, the one the scene registry and the studio register.
    assert!(text.contains("\"KeyedAnimationPlayer\""), "saved as `KeyedAnimationPlayer`: {text}");
    let mut reopened = World::new();
    let mut time = gizmo_core::time::Time::new();
    time.update(0.05);
    reopened.insert_resource(time);
    let ids = crate::scene::SceneData::from_ron_str(&text).expect("reads").instantiate(&mut reopened, &registry);
    let copy = *ids.get(&door.id()).expect("the door came back");
    assert_eq!(reopened.get::<KeyedAnimationPlayer>(copy).map(|p| (p.elapsed_time * 100.0).round()), Some(20.0));
    advance_frame_clocks(&mut reopened);
    assert!((x(&reopened, copy) - 2.5).abs() < 1.0e-4, "it plays on from 0.2 s: {}", x(&reopened, copy));
}

// ── Property tracks: a keyed field of a component, through the engine's registry. ──

/// **A mesh's description follows only what changed.** `describe_meshes` used to compare every
/// mesh's source with its description every frame — a fifth of the draw's CPU time in a 14 600-mesh
/// city. It now reads the meshes written since it last ran, and those it has not described yet.
/// Every edit below is made after the pass, in the tick it ran in: the next frame must see it.
#[test]
fn mesh_descriptions_follow_only_what_changed() {
    use crate::renderer::components::Mesh;
    use crate::renderer::mesh_sync::{describe_meshes, MeshDescribeTick};
    use gizmo_core::component::MeshSource;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::NOOP,
        backend_options: wgpu::BackendOptions {
            noop: wgpu::NoopBackendOptions { enable: true, ..Default::default() },
            ..Default::default()
        },
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("the noop adapter");
    let (device, _queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("a noop device");
    let mesh = |key: &str| Mesh::from_vertices(&device, &[crate::renderer::gpu_types::Vertex::default(); 3], key.to_string());
    let source = |world: &World, e| world.get::<MeshSource>(e).map(|s| s.0.clone());
    let frame = |world: &mut World| {
        let reference = world.tick.wrapping_sub(1);
        world.begin_change_frame(reference);
        describe_meshes(world);
    };
    let mut world = World::new();
    let a = world.spawn();
    world.add_component(a, mesh("sphere_0.5_20_20"));
    frame(&mut world);
    assert_eq!(source(&world, a).as_deref(), Some("sphere_0.5_20_20"));
    frame(&mut world);
    world.add_component(a, mesh("plane_10"));
    frame(&mut world);
    assert_eq!(source(&world, a).as_deref(), Some("plane_10"), "a mesh swapped after the pass");
    let b = world.spawn();
    world.add_component(b, mesh("cone_1_2_12"));
    frame(&mut world);
    assert_eq!(source(&world, b).as_deref(), Some("cone_1_2_12"), "a mesh spawned after the pass");
    assert_eq!(source(&world, a).as_deref(), Some("plane_10"), "an untouched mesh keeps its description");
    // A tick from another world is no reference: its world's meshes are all read.
    let record = world.remove_resource::<MeshDescribeTick>().expect("the pass keeps its tick");
    let mut fresh = World::new();
    let c = fresh.spawn();
    fresh.add_component(c, mesh("plane_4_8"));
    fresh.insert_resource(record);
    fresh.begin_change_frame(fresh.tick);
    describe_meshes(&mut fresh);
    assert_eq!(source(&fresh, c).as_deref(), Some("plane_4_8"), "a tick from another world skipped this one's mesh");
}

/// **A material's bind group is rebuilt only for what changed.** `rebind_changed_materials` used to
/// clone and compare every material's sources every frame — a fifth of the draw's CPU time in a
/// 14 600-material city. It now reads the materials written since it last ran and those it has
/// not seen. A source changed after the pass, in the tick it ran in, is rebuilt next frame; one
/// nobody touched keeps its bind group.
#[test]
#[ignore = "needs a GPU adapter"]
fn material_rebinding_reads_only_what_changed() {
    use crate::renderer::asset::AssetManager;
    use crate::renderer::components::Material;
    use crate::renderer::material_sync::{rebind_changed_materials, MaterialRebindTick};
    use crate::renderer::Renderer;
    let _gpu = crate::test_gpu::gpu_lock();
    if !pollster::block_on(Renderer::headless_adapter_available())
        || pollster::block_on(Renderer::headless_adapter_is_software())
    {
        eprintln!("skipping: no usable GPU adapter");
        return;
    }
    pollster::block_on(async {
        let renderer = crate::test_gpu::headless_renderer(64, 64).await;
        let mut assets = AssetManager::new();
        let layout = renderer.scene.texture_bind_group_layout.clone();
        let own = |assets: &mut AssetManager, k: u8| {
            let rgba: Vec<u8> = (0..16).flat_map(|_| [k, k, k, 255]).collect();
            assets
                .install_decoded_material_texture(&renderer.device, &renderer.queue, &layout, &format!("rebind_{k}"), &rgba, 4, 4)
                .expect("a 4×4 texture installs")
        };
        let frame = |world: &mut World| {
            let reference = world.tick.wrapping_sub(1);
            world.begin_change_frame(reference);
            rebind_changed_materials(world, &renderer);
        };
        let group = |world: &World, e| world.get::<Material>(e).expect("a material").bind_group.clone();
        let mut world = World::new();
        let (first, second) = (own(&mut assets, 40), own(&mut assets, 90));
        let a = world.spawn();
        world.add_component(a, Material::new(first.clone()));
        let b = world.spawn();
        world.add_component(b, Material::new(second.clone()));
        frame(&mut world);
        frame(&mut world);
        assert!(std::sync::Arc::ptr_eq(&group(&world, a), &first), "a material seen first is taken as it is");
        world.query_mut::<&mut Material>().get_mut(a).expect("a").texture_source = Some("no/such/texture.png".into());
        frame(&mut world);
        assert!(!std::sync::Arc::ptr_eq(&group(&world, a), &first), "a source changed after the pass is rebuilt");
        assert!(std::sync::Arc::ptr_eq(&group(&world, b), &second), "an untouched material keeps its bind group");
        // A tick from another world is no reference.
        let record = world.remove_resource::<MaterialRebindTick>().expect("the pass keeps its tick");
        let mut fresh = World::new();
        let c = fresh.spawn();
        let mut named = Material::new(first.clone());
        named.texture_source = Some("no/such/texture.png".into());
        fresh.add_component(c, named);
        fresh.insert_resource(record);
        fresh.begin_change_frame(fresh.tick);
        rebind_changed_materials(&mut fresh, &renderer);
        assert!(fresh.get::<Material>(c).is_some(), "the fresh world's material is read");
    });
}

/// A material over an empty bind group on wgpu's noop backend: no GPU, runs in CI.
pub(crate) fn noop_material() -> crate::renderer::components::Material {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::NOOP,
        backend_options: wgpu::BackendOptions {
            noop: wgpu::NoopBackendOptions { enable: true, ..Default::default() },
            ..Default::default()
        },
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("the noop adapter");
    let (device, _queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("a noop device");
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &[] });
    crate::renderer::components::Material::new(std::sync::Arc::new(
        device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &layout, entries: &[] }),
    ))
}

/// A world on a 0.05 s clock with one entity playing `tracks` (looping off) from its own player.
pub(crate) fn playing(tracks: Vec<Track>) -> (World, gizmo_core::Entity) {
    let mut world = World::new();
    let mut time = gizmo_core::time::Time::new();
    time.update(0.05);
    world.insert_resource(time);
    let mut clip = AnimationClip::default();
    clip.name = "keyed".into();
    clip.tracks = tracks;
    let mut player = KeyedAnimationPlayer::new().looping(false);
    player.play(std::sync::Arc::new(clip));
    let e = world.spawn();
    world.add_component(e, player);
    (world, e)
}

pub(crate) fn scalar(property: &str, from: f32, to: f32) -> Track {
    Track::property("", property, vec![0.0, 1.0], Keyframes::Scalar(vec![from, to])).expect("valid")
}

/// **The frame animates a light and a material** — with no registry made by the game: the frame
/// makes it, with the engine's fields in it. Four frames of 0.05 s are 0.2 s of the clip.
#[test]
fn advance_frame_clocks_animates_point_light_and_material() {
    use crate::renderer::components::{Material, PointLight};
    let (mut world, e) = playing(vec![scalar("PointLight.intensity", 0.0, 10.0), scalar("Material.albedo.w", 1.0, 0.0)]);
    world.add_component(e, PointLight::new(Vec3::ONE, 1.0, 5.0));
    world.add_component(e, noop_material());
    for _ in 0..4 {
        advance_frame_clocks(&mut world);
    }
    let intensity = world.get::<PointLight>(e).unwrap().intensity;
    assert!((intensity - 2.0).abs() < 1.0e-4, "{intensity}");
    let m = world.get::<Material>(e).unwrap();
    assert!((m.albedo.w - 0.8).abs() < 1.0e-4, "{}", m.albedo.w);
    assert_eq!(m.albedo.x, 1.0, "the alpha lane alone");
}

/// **A scrolling surface is a keyed UV offset.** `Material.maps.uv_offset.x` moves on the frame
/// clock and leaves `y` as it was; the changed maps are what `rebind_changed_materials` builds
/// the bind group again from. Not animatable before 2026-11-16: every value took a cache entry
/// for good (`AssetManager::load_material`).
#[test]
fn advance_frame_clocks_scrolls_a_material_uv_offset() {
    use crate::renderer::components::Material;
    let (mut world, e) = playing(vec![scalar("Material.maps.uv_offset.x", 0.0, 1.0)]);
    let mut material = noop_material();
    material.maps.uv_offset = [0.0, 0.25];
    world.add_component(e, material);
    for _ in 0..4 {
        advance_frame_clocks(&mut world);
    }
    let offset = world.get::<Material>(e).unwrap().maps.uv_offset;
    assert!((offset[0] - 0.2).abs() < 1.0e-4, "{offset:?}");
    assert_eq!(offset[1], 0.25, "the x lane alone");
    assert!(!world.get::<Material>(e).unwrap().maps_are_bound(), "the bind group is built again from the new offset");
}

/// **A keyed track wins over a `ColorTween` on the same field** — the driver runs after the
/// colour tweens, deliberately: what an animator keyed is what is drawn.
#[test]
fn keyed_track_wins_over_color_tween_on_sprite_color() {
    use crate::math::{Vec2, Vec4};
    use crate::renderer::components::{ColorTween, Sprite};
    let keyed = Vec4::new(0.2, 0.4, 0.6, 1.0);
    let track = Track::property("", "Sprite.color", vec![0.0, 1.0], Keyframes::Vector4(vec![keyed, keyed])).expect("valid");
    let (mut world, e) = playing(vec![track]);
    world.add_component(e, Sprite::world(crate::renderer::sprite::SpriteImageId::placeholder(0), Vec2::ONE));
    world.add_component(e, ColorTween::to(Vec4::ZERO, 1.0));
    for _ in 0..3 {
        advance_frame_clocks(&mut world);
        assert_eq!(world.get::<Sprite>(e).unwrap().color, keyed);
    }
}

/// **The saved description follows the animated material**: `sync_material_descriptions`, after
/// the clocks, writes the keyed value into the `MaterialDesc`, so a save stores the value at the
/// playhead rather than fighting it.
#[test]
fn material_desc_follows_animated_material() {
    use crate::renderer::components::MaterialDesc;
    use crate::renderer::material_sync::sync_material_descriptions;
    let (mut world, e) = playing(vec![scalar("Material.roughness", 0.0, 1.0)]);
    world.add_component(e, noop_material());
    sync_material_descriptions(&mut world);
    for _ in 0..4 {
        advance_frame_clocks(&mut world);
        sync_material_descriptions(&mut world);
    }
    let desc = world.get::<MaterialDesc>(e).expect("described").roughness;
    assert!((desc - 0.2).abs() < 1.0e-4, "{desc}");
}

/// **The sync reads only what changed, and misses nothing** (`nfsu2-gizmo` MOTOR-NOTLARI #29):
/// with frames ticking, an edit to a material is followed by its description and an edit to a
/// description by its material, a frame or several later; an edit made after the pass in the
/// same tick is seen on the next; and a tick record moved in from another world (the editor's
/// ▶ moves every resource into a fresh world) makes the pass read everything rather than skip
/// what that world never stamped.
#[test]
fn material_sync_follows_every_edit_reading_only_changes() {
    use crate::renderer::components::{Material, MaterialDesc};
    use crate::renderer::material_sync::sync_material_descriptions;
    let mut world = World::new();
    let a = world.spawn();
    world.add_component(a, noop_material());
    let b = world.spawn();
    world.add_component(b, noop_material());
    let frame = |world: &mut World| {
        let reference = world.tick.wrapping_sub(1);
        world.begin_change_frame(reference);
        sync_material_descriptions(world);
    };
    frame(&mut world);
    frame(&mut world);
    assert!(world.get::<MaterialDesc>(a).is_some() && world.get::<MaterialDesc>(b).is_some());

    world.query_mut::<&mut Material>().get_mut(a).unwrap().roughness = 0.11;
    frame(&mut world);
    assert_eq!(world.get::<MaterialDesc>(a).unwrap().roughness, 0.11, "the material's edit");
    frame(&mut world);
    world.query_mut::<&mut MaterialDesc>().get_mut(b).unwrap().roughness = 0.22;
    frame(&mut world);
    assert_eq!(world.get::<Material>(b).unwrap().roughness, 0.22, "the description's edit");

    // After the pass, in the same tick: seen next frame.
    world.query_mut::<&mut Material>().get_mut(a).unwrap().roughness = 0.33;
    frame(&mut world);
    assert_eq!(world.get::<MaterialDesc>(a).unwrap().roughness, 0.33, "an edit after the pass");

    let record = world.remove_resource::<crate::renderer::material_sync::MaterialSyncTick>().unwrap();
    // The fresh world's ticks are behind the record's: everything in it is "older" than a
    // tick from the other world, and only the world's identity tells the two apart.
    let mut fresh = World::new();
    assert!(fresh.tick < record.tick);
    let c = fresh.spawn();
    fresh.add_component(c, noop_material());
    fresh.insert_resource(record);
    fresh.begin_change_frame(fresh.tick);
    sync_material_descriptions(&mut fresh);
    assert!(fresh.get::<MaterialDesc>(c).is_some(), "a tick from another world skipped this one's material");
}

/// **A scene keeps a clip that mixes a transform track and a property track**, and the reopened
/// one plays on bit for bit: the same frame after it, the same position and intensity.
#[cfg(feature = "scene")]
#[test]
fn a_scene_keeps_a_clip_with_a_property_track() {
    use crate::renderer::components::PointLight;
    let slide = Track::new("", vec![0.0, 1.0], Keyframes::Translation(vec![Vec3::ZERO, Vec3::new(10.0, 0.0, 0.0)])).expect("valid");
    let (mut world, e) = playing(vec![slide, scalar("PointLight.intensity", 1.0, 3.0)]);
    world.add_component(e, Transform::new(Vec3::ZERO));
    world.add_component(e, PointLight::new(Vec3::ONE, 1.0, 5.0));
    for _ in 0..3 {
        advance_frame_clocks(&mut world);
    }
    let registry = crate::full_scene_registry();
    let text = crate::scene::SceneData::of_entities(&world, vec![e], &registry).to_ron_string().expect("saves");
    assert!(text.contains("PointLight.intensity"), "the property track is in the file");
    let mut reopened = World::new();
    let mut time = gizmo_core::time::Time::new();
    time.update(0.05);
    reopened.insert_resource(time);
    let ids = crate::scene::SceneData::from_ron_str(&text).expect("reads").instantiate(&mut reopened, &registry);
    let copy = *ids.get(&e.id()).expect("it came back");
    advance_frame_clocks(&mut world);
    advance_frame_clocks(&mut reopened);
    let bits = |w: &World, id| {
        (w.get::<Transform>(id).unwrap().position.x.to_bits(), w.get::<PointLight>(id).unwrap().intensity.to_bits())
    };
    assert_eq!(bits(&reopened, copy), bits(&world, e));
    assert!((world.get::<PointLight>(e).unwrap().intensity - 1.4).abs() < 1.0e-4);
}

/// **A headless frame animates a property track** too: `HeadlessFrames` runs the same clocks, so
/// a game's `--test` sees what its window would.
#[cfg(all(feature = "window", feature = "physics"))]
#[test]
fn a_headless_frame_advances_a_property_track() {
    use crate::renderer::components::PointLight;
    let (mut world, e) = playing(vec![scalar("PointLight.intensity", 0.0, 10.0)]);
    world.remove_resource::<gizmo_core::time::Time>();
    world.add_component(e, PointLight::new(Vec3::ONE, 1.0, 5.0));
    let mut run = crate::simple::HeadlessFrames::new(world);
    for _ in 0..5 {
        run.frame(0.05, |_, _, _, _| {});
    }
    let intensity = run.world.get::<PointLight>(e).unwrap().intensity;
    assert!((intensity - 2.5).abs() < 1.0e-3, "five frames of 0.05 s: {intensity}");
}
