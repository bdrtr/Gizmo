//! **The frame's two halves** (`docs/CAPABILITY_GAPS.md` A8, stage 2): [`prepare_frame`] writes
//! everything the game can see, [`draw_frame`] reads the world and writes only the renderer. The
//! signature makes the draw read-only for ordinary access; these tests hold the parts a signature
//! cannot — that the work moved into the preparation still happens there exactly once a frame, and
//! that the draw neither repeats it nor writes back through interior mutability (a resource's
//! write guard is reachable from `&World`).

use super::{
    advance_frame_clocks, default_render_pass, draw_frame, end_debug_line_frame, prepare_frame,
    step_particle_emitters, ParticleSpawns,
};
use crate::core::World;
use crate::math::{Vec3, Vec4};
use crate::physics::components::{GlobalTransform, Transform};
use crate::renderer::components::{Camera, CameraShake, ParticleEmitter, ReflectionProbe};
use crate::renderer::{GizmoFrame, Gizmos, Renderer};
use gizmo_core::Entity;

fn clock(dt: f32) -> gizmo_core::time::Time {
    let mut time = gizmo_core::time::Time::new();
    time.update(dt);
    time
}

/// A shake wears off once a frame, in the frame's clocks: the same trauma and noise phase as one
/// `advance` per frame — not none (the draw no longer advances it) and not two.
#[test]
fn the_frame_clocks_advance_a_camera_shake_once_a_frame() {
    let dt = 1.0 / 60.0;
    let mut world = World::new();
    world.insert_resource(clock(dt));
    let mut shake = CameraShake::default();
    shake.add(1.0);
    let id = world.spawn_bundle((shake,));
    let mut expected = shake;
    for frame in 1..=10 {
        advance_frame_clocks(&mut world);
        expected.advance(dt);
        let now = *world.query::<&CameraShake>().get(id).unwrap();
        assert_eq!(now, expected, "frame {frame}: the shake did not advance exactly once");
    }
    assert!(expected.trauma() < 1.0, "the test advanced nothing");
}

/// This frame's debug lines reach the draw once: the frame's end moves them into the `GizmoFrame`
/// the draw reads and leaves the game's `Gizmos` empty for the next frame, while the last fixed
/// step's lines are drawn on every frame until the next step replaces them.
#[test]
fn a_frames_debug_lines_are_handed_to_the_draw_once() {
    let mut world = World::new();
    world.insert_resource(Gizmos::default());
    let line = |world: &World, x: f32| {
        world.get_resource_mut::<Gizmos>().unwrap().draw_line(Vec3::new(x, 0.0, 0.0), Vec3::new(x, 1.0, 0.0), [1.0; 4]);
    };
    let drawn = |world: &World| world.get_resource::<GizmoFrame>().map(|f| f.gizmos().vertex_count());

    // A fixed step drew one line, the frame another.
    line(&world, 1.0);
    world.get_resource_mut::<Gizmos>().unwrap().end_fixed_step();
    line(&world, 2.0);
    end_debug_line_frame(&mut world);
    assert_eq!(drawn(&world), Some(4), "the step's line and the frame's");
    assert!(world.get_resource::<Gizmos>().unwrap().lines.is_empty(), "the game's frame lines were not ended");

    // A frame that drew nothing still shows the step's line — once, not the frame's again.
    end_debug_line_frame(&mut world);
    assert_eq!(drawn(&world), Some(2), "last frame's line drawn again, or the step's lost");

    // With no `Gizmos` there is nothing to draw, and last frame's lines are not drawn either.
    world.remove_resource::<Gizmos>();
    end_debug_line_frame(&mut world);
    assert_eq!(drawn(&world), None);
}

/// Emitter spawns come from the frame's preparation and are replaced every frame: a burst is in
/// one frame's spawns, the next frame's hold only what that frame emitted.
#[test]
fn emitter_spawns_are_the_frames_and_replaced_every_frame() {
    let dt = 1.0 / 60.0;
    let mut world = World::new();
    world.insert_resource(clock(dt));
    world.spawn_bundle((Transform::new(Vec3::new(1.0, 2.0, 3.0)), ParticleEmitter::one_shot(250)));
    world.spawn_bundle((ParticleEmitter::with_rate(60.0),));
    let spawns = |world: &World| world.get_resource::<ParticleSpawns>().map(|s| s.particles().len());

    step_particle_emitters(&mut world, dt);
    assert_eq!(spawns(&world), Some(251), "the burst and one from a frame at 60 a second");
    step_particle_emitters(&mut world, dt);
    assert_eq!(spawns(&world), Some(1), "last frame's spawns were kept, or the burst came twice");
    step_particle_emitters(&mut world, 0.0);
    assert_eq!(spawns(&world), Some(0), "a frame that emitted nothing still replaces the spawns");
}

// ---------------------------------------------------------------- GPU: the draw half itself

fn usable_gpu() -> bool {
    pollster::block_on(Renderer::headless_adapter_available())
        && !pollster::block_on(Renderer::headless_adapter_is_software())
}

/// Draws one frame into a 128×128 target with `draw` and reads it back as RGBA8.
fn frame_bytes(
    renderer: &mut Renderer,
    draw: impl FnOnce(&mut wgpu::CommandEncoder, &wgpu::TextureView, &mut Renderer),
) -> Vec<u8> {
    const W: u32 = 128;
    const H: u32 = 128;
    let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("frame-split-target"),
        size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: renderer.config.format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = renderer.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    draw(&mut encoder, &view, renderer);
    let staging = renderer.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("frame-split-readback"),
        size: u64::from(W * H * 4),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(W * 4), rows_per_image: Some(H) },
        },
        wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
    );
    renderer.queue.submit(Some(encoder.finish()));
    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |v| tx.send(v).unwrap());
    let _ = renderer.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
    rx.recv().unwrap().unwrap();
    let bytes = slice.get_mapped_range().expect("a just-mapped buffer's full range is always valid").to_vec();
    staging.unmap();
    bytes
}

/// A lit cube in front of a primary camera on −X looking at it; returns the camera.
fn cube_scene(renderer: &Renderer, world: &mut World) -> Entity {
    let mut assets = crate::renderer::asset::AssetManager::new();
    let tex = assets.create_white_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout);
    world.spawn_bundle((
        Transform::new(Vec3::ZERO),
        GlobalTransform::default(),
        crate::renderer::asset::AssetManager::create_cube(&renderer.device),
        crate::renderer::components::Material::new(tex).with_pbr(Vec4::new(0.9, 0.15, 0.15, 1.0), 0.0, 1.0),
        crate::renderer::components::MeshRenderer::new(),
    ));
    world.spawn_bundle(crate::bundles::DirectionalLightBundle::default());
    world.spawn_bundle(crate::bundles::CameraBundle {
        position: Vec3::new(-6.0, 0.0, 0.0),
        primary: true,
        ..Default::default()
    })
}

fn quiet(renderer: &mut Renderer) {
    renderer.taa = None;
    renderer.ssgi = None;
    renderer.ssr = None;
}

/// **The shake is in the picture and nowhere else**, with the clock on the preparation side: one
/// `prepare_frame` advances the shake once; `draw_frame` run twice on the same world draws the same
/// shaken picture twice and advances nothing; the camera's `Camera`, `Transform` and
/// `GlobalTransform` are bit-identical to what the game left; and the picture is not the unshaken
/// one.
#[test]
#[ignore = "requires a GPU adapter"]
fn a_shake_moves_the_picture_and_never_the_camera() {
    let _gpu = crate::test_gpu::gpu_lock();
    if !usable_gpu() {
        eprintln!("skipping: no usable GPU adapter");
        return;
    }
    pollster::block_on(async {
        let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
        quiet(&mut renderer);
        let dt = 1.0 / 60.0;
        let mut world = World::new();
        world.insert_resource(clock(dt));
        let camera = cube_scene(&renderer, &mut world);

        // The unshaken picture, after a frame that gave every entity its world matrix.
        let still = frame_bytes(&mut renderer, |e, v, r| default_render_pass(&mut world, e, v, r));
        let mut shake = CameraShake::default();
        shake.add(1.0);
        shake.max_offset = 1.5;
        world.add_component(camera, shake);
        let owned = |world: &World| {
            (
                *world.query::<&Camera>().get(camera).unwrap(),
                *world.query::<&Transform>().get(camera).unwrap(),
                *world.query::<&GlobalTransform>().get(camera).unwrap(),
            )
        };
        let mut expected = shake;
        for frame in 1..=3 {
            let before = owned(&world);
            prepare_frame(&mut world, &mut renderer);
            expected.advance(dt);
            assert_eq!(*world.query::<&CameraShake>().get(camera).unwrap(), expected, "frame {frame}: not one advance");
            let prepared = owned(&world);
            let first = frame_bytes(&mut renderer, |e, v, r| draw_frame(&world, e, v, r));
            let second = frame_bytes(&mut renderer, |e, v, r| draw_frame(&world, e, v, r));
            assert_eq!(*world.query::<&CameraShake>().get(camera).unwrap(), expected, "frame {frame}: the draw advanced the shake");
            assert_eq!(owned(&world), prepared, "frame {frame}: the draw moved the camera");
            assert_eq!(prepared, before, "frame {frame}: the shake was written into the camera");
            assert!(first == second, "frame {frame}: two draws of one prepared frame differ");
            let moved = first.chunks(4).zip(still.chunks(4)).filter(|(a, b)| a != b).count();
            assert!(moved > 100, "frame {frame}: the shake did not reach the picture ({moved} pixels differ)");
        }
    });
}

/// **A probe's capture request is honoured once, and the draw never writes the probe.** Captured
/// on its first frame; moved, it keeps the picture it holds (no recapture without a request);
/// `request_capture` recaptures it once, from where it is now; the component the game owns is
/// exactly what the game left after every frame.
#[test]
#[ignore = "requires a GPU adapter"]
fn a_probe_capture_request_is_honoured_once_without_writing_the_probe() {
    let _gpu = crate::test_gpu::gpu_lock();
    if !usable_gpu() {
        eprintln!("skipping: no usable GPU adapter");
        return;
    }
    pollster::block_on(async {
        let mut renderer = crate::test_gpu::headless_renderer(64, 64).await;
        quiet(&mut renderer);
        let mut world = World::new();
        world.insert_resource(clock(1.0 / 60.0));
        cube_scene(&renderer, &mut world);
        let probe = world.spawn_bundle((Transform::new(Vec3::new(0.0, 2.0, 0.0)), GlobalTransform::default(), ReflectionProbe::new()));
        let held = |renderer: &Renderer| {
            let gpu = renderer.reflection_probe.as_ref().expect("a probe array");
            (gpu.captured[0], gpu.captured_at[0])
        };
        let frame = |world: &mut World, renderer: &mut Renderer| {
            let left = world.query::<&ReflectionProbe>().iter().map(|p| (*p, p.capture_request())).next();
            frame_bytes(renderer, |e, v, r| default_render_pass(world, e, v, r));
            let now = world.query::<&ReflectionProbe>().iter().map(|p| (*p, p.capture_request())).next();
            assert_eq!(now, left, "the draw wrote the probe");
        };
        let move_to = |world: &mut World, probe: Entity, y: f32| {
            world.query_mut::<&mut Transform>().get_mut(probe).unwrap().position = Vec3::new(0.0, y, 0.0);
        };
        let asked = |world: &World, probe: Entity| world.query::<&ReflectionProbe>().get(probe).unwrap().capture_request();

        frame(&mut world, &mut renderer);
        let first = asked(&world, probe);
        assert_eq!(held(&renderer), (Some((probe, first)), [0.0, 2.0, 0.0]), "not captured on its first frame");

        move_to(&mut world, probe, 3.0);
        frame(&mut world, &mut renderer);
        assert_eq!(held(&renderer), (Some((probe, first)), [0.0, 2.0, 0.0]), "recaptured without a request");

        world.query_mut::<&mut ReflectionProbe>().get_mut(probe).unwrap().request_capture();
        let second = asked(&world, probe);
        frame(&mut world, &mut renderer);
        assert_eq!(held(&renderer), (Some((probe, second)), [0.0, 3.0, 0.0]), "the request was not honoured");

        move_to(&mut world, probe, 4.0);
        frame(&mut world, &mut renderer);
        assert_eq!(held(&renderer), (Some((probe, second)), [0.0, 3.0, 0.0]), "one request captured twice");

        // **A level restarted in place.** `clear_entities` hands the same handles out again, bit for
        // bit, so the same level rebuilt gives its probe the old probe's exact `Entity` — and a probe
        // nobody asked anything of holds its first request, as the old one did. It is a new probe all
        // the same, and the renderer must capture it rather than keep the old level's cube.
        let restart = |world: &mut World, renderer: &Renderer, y: f32| {
            world.clear_entities();
            cube_scene(renderer, world);
            world.spawn_bundle((Transform::new(Vec3::new(0.0, y, 0.0)), GlobalTransform::default(), ReflectionProbe::new()))
        };
        let level = restart(&mut world, &renderer, 5.0);
        frame(&mut world, &mut renderer);
        assert_eq!(held(&renderer), (Some((level, asked(&world, level))), [0.0, 5.0, 0.0]), "a new level's probe was not captured");
        let again = restart(&mut world, &renderer, 6.0);
        assert_eq!(again, level, "the test needs the restarted level to reuse the probe's handle");
        frame(&mut world, &mut renderer);
        assert_eq!(
            held(&renderer),
            (Some((again, asked(&world, again))), [0.0, 6.0, 0.0]),
            "the restarted level's probe was taken for the old one and never captured"
        );
    });
}

/// **A frame's particles are put out once, however many times it is drawn.** The draw reads
/// `ParticleSpawns` and cannot clear it, so a frame drawn twice — or a copy of the world drawn again
/// without being refreshed — would put the same burst out every time; the particle system keeps the
/// batch it last uploaded and skips it. And `prepare_frame` hands the game's debug lines to the draw.
#[test]
#[ignore = "requires a GPU adapter"]
fn a_frame_drawn_twice_spawns_its_particles_once_and_draws_its_lines() {
    let _gpu = crate::test_gpu::gpu_lock();
    if !usable_gpu() {
        eprintln!("skipping: no usable GPU adapter");
        return;
    }
    pollster::block_on(async {
        let mut renderer = crate::test_gpu::headless_renderer(64, 64).await;
        quiet(&mut renderer);
        assert!(renderer.gpu_particles.is_some(), "the test needs the GPU particle system");
        let mut world = World::new();
        world.insert_resource(clock(1.0 / 60.0));
        world.insert_resource(Gizmos::default());
        cube_scene(&renderer, &mut world);
        world.spawn_bundle((Transform::new(Vec3::ZERO), ParticleEmitter::one_shot(40)));
        world.get_resource_mut::<Gizmos>().unwrap().draw_line(Vec3::ZERO, Vec3::Y, [1.0; 4]);
        let head = |renderer: &Renderer| {
            renderer.gpu_particles.as_ref().unwrap().ring_head.load(std::sync::atomic::Ordering::Relaxed)
        };

        let before = head(&renderer);
        prepare_frame(&mut world, &mut renderer);
        assert_eq!(
            world.get_resource::<GizmoFrame>().map(|f| f.gizmos().vertex_count()),
            Some(2),
            "prepare_frame did not hand the frame's debug lines to the draw"
        );
        assert!(world.get_resource::<Gizmos>().unwrap().lines.is_empty(), "prepare_frame left the game's lines for the next frame");
        frame_bytes(&mut renderer, |e, v, r| draw_frame(&world, e, v, r));
        let once = head(&renderer);
        assert_eq!(once.wrapping_sub(before), 40, "the burst was not put out");
        frame_bytes(&mut renderer, |e, v, r| draw_frame(&world, e, v, r));
        assert_eq!(head(&renderer), once, "drawing the same frame again put the burst out again");

        // The next prepared frame is a new batch: its own spawns go out (none, the burst is spent).
        world.spawn_bundle((Transform::new(Vec3::ZERO), ParticleEmitter::one_shot(5)));
        prepare_frame(&mut world, &mut renderer);
        frame_bytes(&mut renderer, |e, v, r| draw_frame(&world, e, v, r));
        assert_eq!(head(&renderer).wrapping_sub(once), 5, "a new frame's spawns were taken for the old batch");
    });
}

// ------------------------------------------------- GPU: drawing from a copy (A8, stage 3)

/// **A frame drawn from an extracted copy is the frame drawn from the original**, for each thing the
/// draw used to read that a copy could not carry:
///
/// - a cube whose mesh and material are `Handle`s, resolved through `Assets<Mesh>` /
///   `Assets<Material>` — the copy's maps are kept by `Extractor::assets`;
/// - a `VisibilityIndex` that leaves out a cube spawned after it was built — copied as a resource,
///   so the copy culls the cube the original culls;
/// - the camera under water — the fog asks `WaterVolumes`, which the preparation took from the
///   physics world, where the draw used to ask the physics world itself.
///
/// Each one is then taken out of the copy and the picture must change, so an equality reached
/// because the feature drew nothing in either world cannot pass.
#[test]
#[ignore = "requires a GPU adapter"]
fn a_frame_drawn_from_an_extracted_copy_is_the_frame_drawn_from_the_original() {
    use super::visibility_index::VisibilityIndex;
    use super::WaterVolumes;
    use crate::physics::world::{FluidZone, PhysicsWorld, ZoneShape};
    use crate::renderer::components::{DirectionalLight, Material, Mesh, MeshRenderer};
    use gizmo_core::asset::{Assets, Handle};
    use gizmo_core::extract::Extractor;

    let _gpu = crate::test_gpu::gpu_lock();
    if !usable_gpu() {
        eprintln!("skipping: no usable GPU adapter");
        return;
    }
    pollster::block_on(async {
        let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
        quiet(&mut renderer);
        let mut game = World::new();
        game.insert_resource(clock(1.0 / 60.0));
        cube_scene(&renderer, &mut game);

        // A cube drawn through handles.
        let mut textures = crate::renderer::asset::AssetManager::new();
        let tex = textures.create_white_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout);
        let mut meshes = Assets::<Mesh>::new();
        let mut materials = Assets::<Material>::new();
        let mesh: Handle<Mesh> = meshes.add(crate::renderer::asset::AssetManager::create_cube(&renderer.device));
        let material: Handle<Material> =
            materials.add(Material::new(tex.clone()).with_pbr(Vec4::new(0.1, 0.8, 0.2, 1.0), 0.0, 1.0));
        game.insert_resource(meshes);
        game.insert_resource(materials);
        game.spawn_bundle((Transform::new(Vec3::new(0.0, 0.0, 2.2)), GlobalTransform::default(), mesh, material, MeshRenderer::new()));

        // An index built now, then a cube it does not hold: culled, by the index, in the original.
        super::ensure_global_transforms(&mut game);
        let mut index = VisibilityIndex::default();
        index.rebuild_from(&game);
        game.insert_resource(index);
        game.spawn_bundle((
            Transform::new(Vec3::new(0.0, 0.0, -2.2)),
            GlobalTransform::default(),
            crate::renderer::asset::AssetManager::create_cube(&renderer.device),
            Material::new(tex).with_pbr(Vec4::new(0.2, 0.2, 0.9, 1.0), 0.0, 1.0),
            MeshRenderer::new(),
        ));

        // Water around everything, with a fog of its own.
        let mut physics = PhysicsWorld::new();
        physics.fluid_zones.push(FluidZone {
            shape: ZoneShape::Box { min: Vec3::splat(-30.0), max: Vec3::splat(30.0) },
            fog_color: [0.0, 0.35, 0.7],
            fog_density: 0.12,
            ..Default::default()
        });
        game.insert_resource(physics);

        prepare_frame(&mut game, &mut renderer);
        assert!(game.get_resource::<WaterVolumes>().is_some(), "the preparation did not hand the water over");
        let original = frame_bytes(&mut renderer, |e, v, r| draw_frame(&game, e, v, r));

        let extractor = || {
            let mut extractor = Extractor::new();
            extractor
            .component::<Transform>()
            .component::<GlobalTransform>()
            .component::<Camera>()
            .component::<DirectionalLight>()
            .component::<Mesh>()
            .component::<Material>()
            .component::<MeshRenderer>()
            .component::<Handle<Mesh>>()
            .component::<Handle<Material>>()
            .assets::<Mesh>()
            .assets::<Material>()
            .resource::<VisibilityIndex>()
            .resource::<WaterVolumes>()
            .resource::<gizmo_core::time::Time>();
            extractor
        };
        let mut copy = World::new();
        extractor().run(&mut game, &mut copy);
        assert!(copy.get_resource::<PhysicsWorld>().is_none(), "the copy must draw without the physics world");

        let drawn = frame_bytes(&mut renderer, |e, v, r| draw_frame(&copy, e, v, r));
        let differ = |a: &[u8], b: &[u8]| a.chunks(4).zip(b.chunks(4)).filter(|(x, y)| x != y).count();
        assert_eq!(differ(&original, &drawn), 0, "the copy drew a different picture");

        // Each part of the copy is in the picture.
        for (what, take) in [
            ("the handle-drawn cube", Box::new(|w: &mut World| drop(w.remove_resource::<Assets<Mesh>>())) as Box<dyn Fn(&mut World)>),
            ("the visibility index", Box::new(|w: &mut World| drop(w.remove_resource::<VisibilityIndex>()))),
            ("the underwater fog", Box::new(|w: &mut World| drop(w.remove_resource::<WaterVolumes>()))),
        ] {
            // A copy of its own, by an extractor of its own: an extractor copies what changed since
            // its last run, so one that already ran would leave a new world nearly empty.
            let mut without = World::new();
            extractor().run(&mut game, &mut without);
            assert_eq!(differ(&original, &frame_bytes(&mut renderer, |e, v, r| draw_frame(&without, e, v, r))), 0);
            take(&mut without);
            let changed = differ(&original, &frame_bytes(&mut renderer, |e, v, r| draw_frame(&without, e, v, r)));
            assert!(changed > 50, "{what} made no difference to the picture ({changed} pixels)");
        }
    });
}

/// **A `RenderWorld` draws every frame of a changing game as the game world itself would.** Four
/// frames, each changing the game — a cube moved, a cube spawned and another hidden, the spawned
/// one despawned and a UI box recoloured, a point light dimmed — and each frame drawn twice from one
/// preparation: from the game world, and from the `RenderWorld`'s copy brought up to date by
/// `render_extractor`. Equal to the pixel on every frame, and the frames differ from each other, so
/// the copy is following the changes rather than agreeing on a still picture. `frame` then does the
/// whole of it and publishes the draw's cost to the game.
#[test]
#[ignore = "requires a GPU adapter"]
fn a_render_world_draws_every_frame_of_a_changing_game_as_the_game_itself() {
    use super::{publish_render_stats, RenderWorld};
    use crate::renderer::components::{Material, MeshRenderer, PointLight};

    let _gpu = crate::test_gpu::gpu_lock();
    if !usable_gpu() {
        eprintln!("skipping: no usable GPU adapter");
        return;
    }
    pollster::block_on(async {
        let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
        quiet(&mut renderer);
        let mut game = World::new();
        game.insert_resource(clock(1.0 / 60.0));
        cube_scene(&renderer, &mut game);
        let light = game.spawn_bundle(crate::bundles::PointLightBundle {
            position: Vec3::new(-2.0, 2.0, 1.0),
            color: Vec3::new(1.0, 0.6, 0.2),
            intensity: 30.0,
            ..Default::default()
        });
        let mut textures = crate::renderer::asset::AssetManager::new();
        let tex = textures.create_white_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout);
        let cube_mesh = crate::renderer::asset::AssetManager::create_cube(&renderer.device);
        let cube = |y: f32, colour: Vec4| {
            (
                Transform::new(Vec3::new(0.0, y, 2.0)),
                GlobalTransform::default(),
                cube_mesh.clone(),
                Material::new(tex.clone()).with_pbr(colour, 0.0, 1.0),
                MeshRenderer::new(),
            )
        };
        let mover = game.spawn_bundle(cube(1.5, Vec4::new(0.2, 0.9, 0.2, 1.0)));
        let to_hide = game.spawn_bundle(cube(-1.5, Vec4::new(0.9, 0.9, 0.1, 1.0)));
        #[cfg(feature = "ui")]
        let panel = game.spawn_bundle((
            crate::ui::Node { position: gizmo_math::Vec2::new(4.0, 4.0), size: gizmo_math::Vec2::new(30.0, 12.0) },
            crate::ui::BackgroundColor(Vec4::new(0.1, 0.2, 0.9, 1.0)),
        ));

        let mut copy = RenderWorld::new();
        let mut spawned = None;
        let mut pictures = Vec::new();
        for frame in 0..4 {
            match frame {
                1 => game.query_mut::<&mut Transform>().get_mut(mover).unwrap().position.z = -2.0,
                2 => {
                    spawned = Some(game.spawn_bundle(cube(0.0, Vec4::new(0.9, 0.2, 0.9, 1.0))));
                    game.add_component(to_hide, crate::core::component::IsHidden);
                }
                3 => {
                    game.despawn(spawned.take().unwrap());
                    #[cfg(feature = "ui")]
                    {
                        game.query_mut::<&mut crate::ui::BackgroundColor>().get_mut(panel).unwrap().0 =
                            Vec4::new(0.9, 0.9, 0.9, 1.0);
                    }
                    game.query_mut::<&mut PointLight>().get_mut(light).unwrap().intensity = 2.0;
                }
                _ => {}
            }
            prepare_frame(&mut game, &mut renderer);
            let original = frame_bytes(&mut renderer, |e, v, r| draw_frame(&game, e, v, r));
            copy.extract(&mut game);
            let drawn = frame_bytes(&mut renderer, |e, v, r| draw_frame(copy.world(), e, v, r));
            let differ = original.chunks(4).zip(drawn.chunks(4)).filter(|(a, b)| a != b).count();
            assert_eq!(differ, 0, "frame {frame}: the copy drew a different picture");
            publish_render_stats(&mut game, &renderer);
            pictures.push(original);
        }
        for pair in pictures.windows(2) {
            let changed = pair[0].chunks(4).zip(pair[1].chunks(4)).filter(|(a, b)| a != b).count();
            assert!(changed > 20, "a frame's change did not reach the picture ({changed} pixels)");
        }

        game.remove_resource::<crate::renderer::components::RenderStats>();
        frame_bytes(&mut renderer, |e, v, r| copy.frame(&mut game, e, v, r));
        assert!(
            game.get_resource::<crate::renderer::components::RenderStats>().is_some_and(|s| s.draw_calls > 0),
            "`frame` did not publish the draw's cost to the game"
        );

        // As the app's render stage: `prepare` with the game world, `draw` without it. The picture
        // is the original's, and what the draw cost reaches the game at the next `prepare` — the
        // first moment the game world is in reach after the draw, wherever the draw ran.
        use gizmo_app::RenderStage;
        use crate::renderer::components::RenderStats;
        game.query_mut::<&mut Transform>().get_mut(mover).unwrap().position.z = -4.0;
        renderer.render_stats = None;
        game.remove_resource::<RenderStats>();
        copy.prepare(&mut game, &mut renderer);
        assert!(game.get_resource::<RenderStats>().is_none(), "a cost was published before anything was drawn");
        let staged = frame_bytes(&mut renderer, |e, v, r| copy.draw(e, v, r));
        let original = frame_bytes(&mut renderer, |e, v, r| draw_frame(&game, e, v, r));
        let differ = original.chunks(4).zip(staged.chunks(4)).filter(|(a, b)| a != b).count();
        assert_eq!(differ, 0, "the stage drew a different picture from the game's own");
        assert!(
            pictures.last().is_some_and(|last| last != &staged),
            "the stage drew the last frame again, not the moved cube"
        );
        // And the same draw on another thread — where the app runs a stage's draw by default — draws
        // the same picture: the stage and the renderer are all it is given, and both can go there.
        let elsewhere = std::thread::scope(|s| {
            s.spawn(|| frame_bytes(&mut renderer, |e, v, r| copy.draw(e, v, r)))
                .join()
                .expect("the draw panicked on its thread")
        });
        let differ = staged.chunks(4).zip(elsewhere.chunks(4)).filter(|(a, b)| a != b).count();
        assert_eq!(differ, 0, "the stage drew a different picture on a render thread");
        copy.prepare(&mut game, &mut renderer);
        assert!(
            game.get_resource::<RenderStats>().is_some_and(|s| s.draw_calls > 0),
            "the stage's draw cost never reached the game"
        );
    });
}

/// **Geometry rebuilt every frame does not pile up.** A cloth, a rope or a vision cone gets a new
/// `MeshData` each frame; the loader uploads it and the frame is drawn from a copy of the world.
/// The upload the entity no longer names used to be released only if the loader's cache held its
/// last handle — which nothing that had been drawn ever satisfied (the copy held it until its next
/// extract, the batcher's cache for good) — so both the loader's cache and the batcher's kept one
/// upload per frame for the rest of the run. Both are bounded here over 40 frames.
#[test]
#[ignore = "requires a GPU adapter"]
fn geometry_rebuilt_every_frame_and_drawn_from_a_copy_does_not_pile_up() {
    use super::RenderWorld;
    use crate::renderer::components::{Material, MeshData, MeshRenderer};
    use gizmo_app::RenderStage;

    let _gpu = crate::test_gpu::gpu_lock();
    if !usable_gpu() {
        eprintln!("skipping: no usable GPU adapter");
        return;
    }
    pollster::block_on(async {
        let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
        quiet(&mut renderer);
        let mut game = World::new();
        game.insert_resource(clock(1.0 / 60.0));
        cube_scene(&renderer, &mut game);
        let mut assets = crate::renderer::asset::AssetManager::new();
        let tex = assets.create_white_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout);
        let cone = game.spawn_bundle((
            Transform::new(Vec3::new(0.0, 0.0, 2.0)),
            GlobalTransform::default(),
            Material::new(tex).with_pbr(Vec4::new(0.9, 0.3, 0.2, 1.0), 0.5, 0.0),
            MeshRenderer::new(),
        ));
        let mut copy = RenderWorld::new();
        super::batching::clear_render_cache();
        let mut drawn = 0;
        for frame in 0..40u32 {
            // A new allocation every frame, as a rebuilt cloth or cone has.
            game.add_component(cone, MeshData::cylinder(0.5 + frame as f32 * 0.01, 1.0, 12));
            crate::renderer::asset_loading::run_asset_loading_system(
                &mut game,
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
                &mut assets,
            );
            copy.prepare(&mut game, &mut renderer);
            let picture = frame_bytes(&mut renderer, |e, v, r| copy.draw(e, v, r));
            drawn += usize::from(picture.chunks(4).any(|p| p[0] > 120 && p[1] < 100));
        }
        assert!(drawn > 30, "the rebuilt geometry was drawn in only {drawn} of 40 frames");
        assert!(assets.data_mesh_count() <= 1, "the loader keeps {} uploads for one entity", assets.data_mesh_count());
        let batches = super::batching::render_cache_batch_count();
        assert!(batches <= 12, "the batcher keeps {batches} batches for a scene of two meshes");
    });
}

/// **What a game adds to the frame goes on the stage.** `with_settings` runs at the sync point
/// with the game world — a resource only the game has, which the copy does not carry — and what it
/// sets is what this frame is drawn with, the same picture as the renderer set by hand. `with_pass`
/// draws over the finished frame, on the draw's side: it is handed the copy, it runs after the frame
/// (a full clear in it is the whole picture), and the stage with it still draws on another thread.
#[test]
#[ignore = "requires a GPU adapter"]
fn a_stages_settings_reach_its_frame_and_its_pass_draws_over_it() {
    use super::RenderWorld;
    use crate::renderer::gpu_types::TonemapCurve;
    use gizmo_app::RenderStage;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// The game's own setting, which the render extractor does not copy.
    struct Curve(TonemapCurve);

    let _gpu = crate::test_gpu::gpu_lock();
    if !usable_gpu() {
        eprintln!("skipping: no usable GPU adapter");
        return;
    }
    pollster::block_on(async {
        let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
        quiet(&mut renderer);
        let mut game = World::new();
        game.insert_resource(clock(1.0 / 60.0));
        cube_scene(&renderer, &mut game);
        renderer.tonemap_curve = TonemapCurve::Aces;

        let mut stage = RenderWorld::new().with_settings(|game, renderer| {
            if let Some(curve) = game.get_resource::<Curve>() {
                renderer.tonemap_curve = curve.0;
            }
        });
        let mut pictures = Vec::new();
        for curve in [TonemapCurve::None, TonemapCurve::Reinhard] {
            game.insert_resource(Curve(curve));
            stage.prepare(&mut game, &mut renderer);
            assert_eq!(renderer.tonemap_curve, curve, "the setting did not reach the renderer at the sync point");
            assert!(stage.world().get_resource::<Curve>().is_none(), "the copy carries the game's own setting");
            let staged = frame_bytes(&mut renderer, |e, v, r| stage.draw(e, v, r));
            let by_hand = frame_bytes(&mut renderer, |e, v, r| draw_frame(&game, e, v, r));
            let differ = staged.chunks(4).zip(by_hand.chunks(4)).filter(|(a, b)| a != b).count();
            assert_eq!(differ, 0, "{curve:?}: the frame was not drawn with the setting made at its sync point");
            pictures.push(staged);
        }
        let changed = pictures[0].chunks(4).zip(pictures[1].chunks(4)).filter(|(a, b)| a != b).count();
        assert!(changed > 100, "the two curves drew the same picture ({changed} pixels differ)");

        // A pass: it sees the copy (the camera is there), and it runs after the frame — a clear to
        // green over the whole target is the whole picture.
        let cameras_seen = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&cameras_seen);
        let mut stage = RenderWorld::new().with_pass(move |copy, encoder, view, _renderer| {
            seen.store(copy.query::<&Camera>().iter().count(), Ordering::Relaxed);
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("a-game-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::GREEN), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        });
        stage.prepare(&mut game, &mut renderer);
        let picture = std::thread::scope(|s| {
            s.spawn(|| frame_bytes(&mut renderer, |e, v, r| stage.draw(e, v, r)))
                .join()
                .expect("the draw panicked on its thread")
        });
        assert_eq!(cameras_seen.load(Ordering::Relaxed), 1, "the pass was not handed the copy");
        let green = picture.chunks(4).filter(|p| p[0] < 10 && p[1] > 245 && p[2] < 10).count();
        assert_eq!(green, 128 * 128, "the pass did not draw over the finished frame");
    });
}
