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

/// Reads `texture` back as tightly packed RGBA8 rows.
fn texture_bytes(renderer: &Renderer, texture: &wgpu::Texture) -> Vec<u8> {
    let (w, h) = (texture.width(), texture.height());
    let row = (w * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let staging = renderer.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("camera-target-readback"),
        size: u64::from(row * h),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = renderer.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) },
        },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    renderer.queue.submit(Some(encoder.finish()));
    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |v| tx.send(v).unwrap());
    let _ = renderer.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
    rx.recv().unwrap().unwrap();
    let mapped = slice.get_mapped_range().expect("a just-mapped buffer's full range is always valid");
    let bytes = mapped.chunks(row as usize).flat_map(|r| &r[..(w * 4) as usize]).copied().collect();
    drop(mapped);
    staging.unmap();
    bytes
}

/// **A camera that draws into a texture** ([`CameraTarget`](crate::renderer::components::CameraTarget)).
/// Its picture is the scene through it: the same picture, to the pixel, as that camera drawn as the
/// frame's own into a texture of the same size. The frame's own view is untouched by it — the camera
/// with a target is not the frame's, and its uploads, submitted before the frame's, do not leak into
/// it. Its projection takes the texture's shape: in a portrait texture the cube is as wide as it is
/// tall. And all of it holds drawn from a render world's copy, which carries the target.
#[test]
#[ignore = "requires a GPU adapter"]
fn a_camera_with_a_target_draws_into_it_and_leaves_the_frame_alone() {
    use super::RenderWorld;
    use crate::renderer::components::CameraTarget;
    use gizmo_app::RenderStage;

    let _gpu = crate::test_gpu::gpu_lock();
    if !usable_gpu() {
        eprintln!("skipping: no usable GPU adapter");
        return;
    }
    pollster::block_on(async {
        let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
        quiet(&mut renderer);
        let gpu = renderer.gpu_context();
        let mut game = World::new();
        game.insert_resource(clock(1.0 / 60.0));
        let main_camera = cube_scene(&renderer, &mut game);
        prepare_frame(&mut game, &mut renderer);
        let alone = frame_bytes(&mut renderer, |e, v, r| draw_frame(&game, e, v, r));

        // A second camera, in front of the cube on +Z looking back at it.
        let side = crate::bundles::CameraBundle {
            position: Vec3::new(0.0, 0.0, 6.0),
            yaw: -std::f32::consts::FRAC_PI_2,
            primary: false,
            ..Default::default()
        };
        let watcher = game.spawn_bundle(side);
        let square = CameraTarget::new(&gpu, 128, 128);
        game.add_component(watcher, square.clone());
        prepare_frame(&mut game, &mut renderer);
        let with_target = frame_bytes(&mut renderer, |e, v, r| draw_frame(&game, e, v, r));
        let differ = alone.chunks(4).zip(with_target.chunks(4)).filter(|(a, b)| a != b).count();
        assert_eq!(differ, 0, "a camera drawing into a texture changed the frame's own view ({differ} pixels)");
        let through_target = texture_bytes(&renderer, square.texture());
        // The cube's red, lit or in shade: red well over green and blue.
        let red = |p: &[u8]| p[0] > 40 && u32::from(p[0]) > 2 * u32::from(p[1].max(p[2])) + 10;
        assert!(through_target.chunks(4).filter(|p| red(p)).count() > 200, "the target holds no cube");

        // The reference: the same camera as the frame's own, into a texture of the same size.
        let mut reference_world = World::new();
        reference_world.insert_resource(clock(1.0 / 60.0));
        cube_scene(&renderer, &mut reference_world);
        reference_world.query_mut::<&mut Camera>().iter_mut().for_each(|mut c| c.primary = false);
        reference_world.spawn_bundle(crate::bundles::CameraBundle { primary: true, ..side });
        prepare_frame(&mut reference_world, &mut renderer);
        let as_frame = frame_bytes(&mut renderer, |e, v, r| draw_frame(&reference_world, e, v, r));
        let differ = as_frame.chunks(4).zip(through_target.chunks(4)).filter(|(a, b)| a != b).count();
        assert_eq!(differ, 0, "the target's picture is not the camera's own ({differ} pixels differ)");

        // A portrait target: the projection takes its shape, so the cube stays square.
        game.add_component(watcher, CameraTarget::new(&gpu, 64, 128));
        let tall = game.query::<&CameraTarget>().get(watcher).cloned().expect("the target was replaced");
        // Drawn from a render world's copy this time, which must carry the target.
        let mut copy = RenderWorld::new();
        copy.prepare(&mut game, &mut renderer);
        let from_copy = frame_bytes(&mut renderer, |e, v, r| copy.draw(e, v, r));
        let differ = alone.chunks(4).zip(from_copy.chunks(4)).filter(|(a, b)| a != b).count();
        assert_eq!(differ, 0, "drawn from the copy, the frame's own view changed ({differ} pixels)");
        let portrait = texture_bytes(&renderer, tall.texture());
        let (mut min, mut max) = ((u32::MAX, u32::MAX), (0u32, 0u32));
        for (i, p) in portrait.chunks(4).enumerate() {
            if red(p) {
                let (x, y) = (i as u32 % 64, i as u32 / 64);
                min = (min.0.min(x), min.1.min(y));
                max = (max.0.max(x), max.1.max(y));
            }
        }
        assert!(max.0 >= min.0, "the portrait target holds no cube");
        let (w, h) = ((max.0 - min.0 + 1) as f32, (max.1 - min.1 + 1) as f32);
        assert!((w / h - 1.0).abs() < 0.15, "the cube is {w}×{h} in a 64×128 target — the projection did not take its shape");
        assert!(game.query::<&Camera>().get(main_camera).is_some_and(|c| c.primary));

        // Never the frame's camera, even first in the world with no camera marked primary — where
        // the rule falls back to the first camera it finds.
        let mut unmarked = World::new();
        let first = unmarked.spawn_bundle(crate::bundles::CameraBundle { primary: false, ..side });
        unmarked.add_component(first, CameraTarget::new(&gpu, 8, 8));
        let second = unmarked.spawn_bundle(crate::bundles::CameraBundle { primary: false, ..Default::default() });
        // Into a table made after the target camera's, so the first camera a query finds is the one
        // with the target — queries walk tables in the order they were made.
        #[derive(Clone)]
        struct Later;
        gizmo_core::impl_component!(Later);
        unmarked.add_component(second, Later);
        assert_eq!(crate::renderer::components::active_camera(&unmarked), Some(second));
    });
}

/// **Every view of a frame sees the frame's simulation step.** A texture camera standing where the
/// frame's camera stands, at the same shape, draws the frame's picture to the pixel — fast GPU
/// particles included. A texture camera and a mirror's capture are submitted before the frame's own
/// encoder, and the GPU simulations used to be recorded into that encoder, so a texture camera drew
/// every particle where it was one step before the frame's view drew it.
#[test]
#[ignore = "requires a GPU adapter"]
fn a_texture_camera_where_the_frames_camera_stands_draws_the_frames_picture() {
    use crate::renderer::components::CameraTarget;
    use crate::renderer::gpu_particles::GpuParticle;

    let _gpu = crate::test_gpu::gpu_lock();
    if !usable_gpu() {
        eprintln!("skipping: no usable GPU adapter");
        return;
    }
    pollster::block_on(async {
        let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
        quiet(&mut renderer);
        if renderer.gpu_particles.is_none() {
            eprintln!("skipping: this renderer has no GPU particles");
            return;
        }
        let gpu = renderer.gpu_context();
        let mut game = World::new();
        game.insert_resource(clock(1.0 / 60.0));
        cube_scene(&renderer, &mut game);
        // The frame's camera's twin: where it stands, looking where it looks, into a texture of the
        // frame's size.
        let twin = game.spawn_bundle(crate::bundles::CameraBundle {
            position: Vec3::new(-6.0, 0.0, 0.0),
            primary: false,
            ..Default::default()
        });
        let target = CameraTarget::new(&gpu, 128, 128);
        game.add_component(twin, target.clone());
        prepare_frame(&mut game, &mut renderer);
        let without = frame_bytes(&mut renderer, |e, v, r| draw_frame(&game, e, v, r));

        // A sheet of fast white particles crossing the view: one step moves them 2 m.
        let particles: Vec<GpuParticle> = (0..256)
            .map(|i| GpuParticle {
                position: [-2.0, -1.2 + (i / 16) as f32 * 0.15, -2.5 + (i % 16) as f32 * 0.15],
                life: 0.0,
                velocity: [0.0, 0.0, 120.0],
                max_life: 10.0,
                color: [1.0, 1.0, 1.0, 1.0],
                size_start: 0.12,
                size_end: 0.12,
                _padding: [0.0; 2],
            })
            .collect();
        renderer.gpu_particles.as_ref().expect("checked above").spawn_particles(&renderer.queue, &particles);

        prepare_frame(&mut game, &mut renderer);
        let frame = frame_bytes(&mut renderer, |e, v, r| draw_frame(&game, e, v, r));
        let through_twin = texture_bytes(&renderer, target.texture());
        let shown = without.chunks(4).zip(frame.chunks(4)).filter(|(a, b)| a != b).count();
        assert!(shown > 50, "the particles changed only {shown} pixels of the frame");
        let differ = frame.chunks(4).zip(through_twin.chunks(4)).filter(|(a, b)| a != b).count();
        assert_eq!(differ, 0, "the twin drew a different picture from the frame's ({differ} pixels): not the same simulation step");
    });
}

/// A mirror 10 ahead of the origin facing +z, a red cube behind the origin to the left (seen only
/// in the mirror from there), a green one between; all unlit, so a colour counted is that object.
fn mirror_room(renderer: &Renderer, world: &mut World) {
    use crate::renderer::components::{Material, MeshRenderer, PlanarReflection};
    let mut assets = crate::renderer::asset::AssetManager::new();
    let tex = assets.create_white_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout);
    let cube = crate::renderer::asset::AssetManager::create_cube(&renderer.device);
    let mut spawn = |at: Vec3, scale: Vec3, colour: Vec4| {
        world.spawn_bundle((
            Transform::new(at).with_scale(scale),
            GlobalTransform::default(),
            cube.clone(),
            Material::new(tex.clone()).with_unlit(colour),
            MeshRenderer::new(),
        ))
    };
    let mirror = spawn(Vec3::new(0.0, 0.0, -10.0), Vec3::new(9.0, 7.0, 0.05), Vec4::new(0.0, 0.0, 0.0, 1.0));
    spawn(Vec3::new(-6.0, 0.0, 6.0), Vec3::splat(1.5), Vec4::new(1.0, 0.0, 0.0, 1.0));
    spawn(Vec3::new(4.0, 2.0, -6.0), Vec3::splat(0.5), Vec4::new(0.0, 1.0, 0.0, 1.0));
    world.add_component(mirror, PlanarReflection::new().facing(Vec3::Z, 1.0));
}

/// **A mirror a texture camera sees reflects for that camera.** Each view's mirrors are captured
/// for its own camera, just before it: a texture camera standing apart from the frame's camera draws
/// what it draws as the frame's own — the red cube it sees only in the mirror included — to the
/// pixel; and the frame's own view, whose mirrors are captured last, is unchanged by it. The
/// capture used to be the frame's camera's alone, so the texture camera's mirror showed the frame's
/// camera's reflection.
#[test]
#[ignore = "requires a GPU adapter"]
fn a_mirror_a_texture_camera_sees_reflects_for_that_camera() {
    use crate::renderer::components::CameraTarget;

    let _gpu = crate::test_gpu::gpu_lock();
    if !usable_gpu() {
        eprintln!("skipping: no usable GPU adapter");
        return;
    }
    pollster::block_on(async {
        let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
        quiet(&mut renderer);
        let gpu = renderer.gpu_context();
        let toward_mirror = -std::f32::consts::FRAC_PI_2;
        let frame_camera = crate::bundles::CameraBundle { position: Vec3::ZERO, yaw: toward_mirror, primary: true, ..Default::default() };
        let apart = crate::bundles::CameraBundle {
            position: Vec3::new(3.0, 1.0, 3.0),
            yaw: toward_mirror,
            primary: false,
            ..Default::default()
        };

        // The frame alone, and the camera apart as the frame's own camera.
        let mut alone = World::new();
        alone.insert_resource(clock(1.0 / 60.0));
        mirror_room(&renderer, &mut alone);
        alone.spawn_bundle(frame_camera);
        prepare_frame(&mut alone, &mut renderer);
        let frame_alone = frame_bytes(&mut renderer, |e, v, r| draw_frame(&alone, e, v, r));
        let mut reference = World::new();
        reference.insert_resource(clock(1.0 / 60.0));
        mirror_room(&renderer, &mut reference);
        reference.spawn_bundle(crate::bundles::CameraBundle { primary: true, ..apart });
        prepare_frame(&mut reference, &mut renderer);
        let as_frame = frame_bytes(&mut renderer, |e, v, r| draw_frame(&reference, e, v, r));
        let red = |p: &[u8]| p[0] > 120 && p[1] < 60 && p[2] < 60;
        assert!(as_frame.chunks(4).filter(|p| red(p)).count() > 30, "the camera apart does not see the red cube in the mirror");

        // Both cameras in one world, the one apart drawing into a texture.
        let mut game = World::new();
        game.insert_resource(clock(1.0 / 60.0));
        mirror_room(&renderer, &mut game);
        game.spawn_bundle(frame_camera);
        let watcher = game.spawn_bundle(apart);
        let target = CameraTarget::new(&gpu, 128, 128);
        game.add_component(watcher, target.clone());
        prepare_frame(&mut game, &mut renderer);
        let frame = frame_bytes(&mut renderer, |e, v, r| draw_frame(&game, e, v, r));
        let through_target = texture_bytes(&renderer, target.texture());
        let differ = as_frame.chunks(4).zip(through_target.chunks(4)).filter(|(a, b)| a != b).count();
        assert_eq!(differ, 0, "the texture camera's mirror is not its own ({differ} pixels differ)");
        let differ = frame_alone.chunks(4).zip(frame.chunks(4)).filter(|(a, b)| a != b).count();
        assert_eq!(differ, 0, "the frame's own view changed with a texture camera in the room ({differ} pixels)");
        assert_eq!(renderer.planar.captures, 2, "one capture for each camera that sees the mirror");
    });
}

/// **A mirror in a split-screen view reflects for that view's camera.** Both halves see the mirror,
/// and the red cube behind both cameras shows in each half — it can only come through the mirror.
/// Split screen captured no mirror at all: they drew plain in every view.
#[test]
#[ignore = "requires a GPU adapter"]
fn a_mirror_in_a_split_screen_view_reflects_for_its_camera() {
    use crate::renderer::components::Viewport;

    let _gpu = crate::test_gpu::gpu_lock();
    if !usable_gpu() {
        eprintln!("skipping: no usable GPU adapter");
        return;
    }
    pollster::block_on(async {
        let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
        quiet(&mut renderer);
        let toward_mirror = -std::f32::consts::FRAC_PI_2;
        let mut game = World::new();
        game.insert_resource(clock(1.0 / 60.0));
        mirror_room(&renderer, &mut game);
        let left = game.spawn_bundle(crate::bundles::CameraBundle { position: Vec3::ZERO, yaw: toward_mirror, primary: true, ..Default::default() });
        let right = game.spawn_bundle(crate::bundles::CameraBundle {
            position: Vec3::new(3.0, 1.0, 3.0),
            yaw: toward_mirror,
            primary: false,
            ..Default::default()
        });
        for (camera, x) in [(left, 0.0), (right, 0.5)] {
            game.query_mut::<&mut Camera>().get_mut(camera).expect("a camera").viewport = Some(Viewport::new(x, 0.0, 0.5, 1.0));
        }
        prepare_frame(&mut game, &mut renderer);
        let frame = frame_bytes(&mut renderer, |e, v, r| draw_frame(&game, e, v, r));
        let red = |p: &[u8]| p[0] > 120 && p[1] < 60 && p[2] < 60;
        let (mut in_left, mut in_right) = (0, 0);
        for (i, p) in frame.chunks(4).enumerate() {
            if red(p) {
                if i % 128 < 64 { in_left += 1 } else { in_right += 1 }
            }
        }
        assert!(in_left > 10 && in_right > 10, "the red cube through the mirror: left {in_left}, right {in_right}");
        assert_eq!(renderer.planar.captures, 2, "one capture for each view");
    });
}

/// A red-or-green cube above a polished floor, a sun, a camera on −X looking at them: a scene in
/// which screen-space reflections have the cube to find in the floor. Returns the cube.
fn reflecting_floor(renderer: &Renderer, world: &mut World, colour: Vec4) -> Entity {
    use crate::renderer::components::{Material, MeshRenderer};
    let mut assets = crate::renderer::asset::AssetManager::new();
    let tex = assets.create_white_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout);
    let cube_mesh = crate::renderer::asset::AssetManager::create_cube(&renderer.device);
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, -1.2, 0.0)).with_scale(Vec3::new(20.0, 0.2, 20.0)),
        GlobalTransform::default(),
        cube_mesh.clone(),
        Material::new(tex.clone()).with_pbr(Vec4::new(0.8, 0.8, 0.8, 1.0), 0.05, 0.9),
        MeshRenderer::new(),
    ));
    let cube = world.spawn_bundle((
        Transform::new(Vec3::ZERO),
        GlobalTransform::default(),
        cube_mesh,
        Material::new(tex).with_pbr(colour, 0.5, 0.0),
        MeshRenderer::new(),
    ));
    world.spawn_bundle(crate::bundles::DirectionalLightBundle::default());
    world.spawn_bundle(crate::bundles::CameraBundle {
        position: Vec3::new(-6.0, 0.6, 0.0),
        pitch: -0.15,
        primary: true,
        ..Default::default()
    });
    cube
}

/// **A resize leaves screen-space reflections reading the frame the chain draws.** `Renderer::resize`
/// rebinds every effect to its new targets but SSR, which the draw rebuilt only when its size
/// differed from the renderer's — so a resize that ended at SSR's own size before a frame was drawn
/// (a round trip, or a 1280 × 720 window minimised: the zero size went through the web cap and came
/// out 1280 × 720) left it sampling a HDR texture nothing drew any more, and the reflections froze
/// on the last frame before it. Here the cube turns green after a round trip and the frame must be a
/// fresh renderer's green frame to the pixel. And a zero size changes nothing, as documented.
#[test]
#[ignore = "requires a GPU adapter"]
fn a_resize_leaves_reflections_reading_the_frame_the_chain_draws() {
    use crate::renderer::components::Material;

    let _gpu = crate::test_gpu::gpu_lock();
    if !usable_gpu() {
        eprintln!("skipping: no usable GPU adapter");
        return;
    }
    pollster::block_on(async {
        let red = Vec4::new(0.9, 0.1, 0.1, 1.0);
        let green = Vec4::new(0.1, 0.9, 0.1, 1.0);
        let calm = |renderer: &mut Renderer| {
            renderer.taa = None;
            renderer.ssgi = None;
            renderer.volumetric = None;
        };

        let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
        calm(&mut renderer);
        assert!(renderer.ssr.is_some(), "the test needs SSR");
        let mut game = World::new();
        game.insert_resource(clock(1.0 / 60.0));
        let cube = reflecting_floor(&renderer, &mut game, red);
        prepare_frame(&mut game, &mut renderer);
        frame_bytes(&mut renderer, |e, v, r| draw_frame(&game, e, v, r));

        renderer.resize(winit::dpi::PhysicalSize::new(0, 0));
        assert_eq!((renderer.size.width, renderer.size.height), (128, 128), "a zero size resized the renderer");
        renderer.resize(winit::dpi::PhysicalSize::new(96, 64));
        renderer.resize(winit::dpi::PhysicalSize::new(128, 128));
        game.query_mut::<&mut Material>().get_mut(cube).expect("the cube").albedo = green;
        prepare_frame(&mut game, &mut renderer);
        let after = frame_bytes(&mut renderer, |e, v, r| draw_frame(&game, e, v, r));

        let mut fresh = crate::test_gpu::headless_renderer(128, 128).await;
        calm(&mut fresh);
        let mut reference = World::new();
        reference.insert_resource(clock(1.0 / 60.0));
        reflecting_floor(&fresh, &mut reference, green);
        prepare_frame(&mut reference, &mut fresh);
        let want = frame_bytes(&mut fresh, |e, v, r| draw_frame(&reference, e, v, r));
        fresh.ssr.as_mut().expect("SSR").enabled = false;
        let without_ssr = frame_bytes(&mut fresh, |e, v, r| draw_frame(&reference, e, v, r));
        let shown = want.chunks(4).zip(without_ssr.chunks(4)).filter(|(a, b)| a != b).count();
        assert!(shown > 50, "SSR changes only {shown} pixels of the scene — the comparison would not see it");

        let differ = after.chunks(4).zip(want.chunks(4)).filter(|(a, b)| a != b).count();
        assert_eq!(differ, 0, "after the resize the frame is not a fresh renderer's ({differ} pixels differ)");
    });
}

/// **A view's temporal state follows the renderer's effects.** A texture camera standing where the
/// frame's camera stands draws the frame's picture after SSGI is turned on with `enable_ssgi`. Its
/// history was built while SSGI was off and kept none: the texture camera drew with the frame
/// camera's own SSGI — advancing it a step ahead of the frame's view every frame — so the two
/// pictures parted, until a resize. SSGI is checked to change the frame, so the comparison sees it.
#[test]
#[ignore = "requires a GPU adapter"]
fn a_view_gets_its_own_temporal_state_for_an_effect_turned_on_later() {
    use crate::renderer::components::CameraTarget;

    let _gpu = crate::test_gpu::gpu_lock();
    if !usable_gpu() {
        eprintln!("skipping: no usable GPU adapter");
        return;
    }
    pollster::block_on(async {
        let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
        renderer.taa = None;
        renderer.ssr = None;
        renderer.volumetric = None;
        renderer.ssgi = None;
        let gpu = renderer.gpu_context();
        let mut game = World::new();
        game.insert_resource(clock(1.0 / 60.0));
        reflecting_floor(&renderer, &mut game, Vec4::new(0.9, 0.1, 0.1, 1.0));
        let twin = game.spawn_bundle(crate::bundles::CameraBundle {
            position: Vec3::new(-6.0, 0.6, 0.0),
            pitch: -0.15,
            primary: false,
            ..Default::default()
        });
        let target = CameraTarget::new(&gpu, 128, 128);
        game.add_component(twin, target.clone());
        prepare_frame(&mut game, &mut renderer);
        frame_bytes(&mut renderer, |e, v, r| draw_frame(&game, e, v, r));

        renderer.enable_ssgi();
        let mut last = Vec::new();
        for frame in 1..5 {
            prepare_frame(&mut game, &mut renderer);
            last = frame_bytes(&mut renderer, |e, v, r| draw_frame(&game, e, v, r));
            let through_twin = texture_bytes(&renderer, target.texture());
            let differ = last.chunks(4).zip(through_twin.chunks(4)).filter(|(a, b)| a != b).count();
            assert_eq!(differ, 0, "frame {frame}: the twin's picture parted from the frame's ({differ} pixels)");
        }
        renderer.ssgi.as_mut().expect("SSGI").enabled = false;
        prepare_frame(&mut game, &mut renderer);
        let without = frame_bytes(&mut renderer, |e, v, r| draw_frame(&game, e, v, r));
        let shown = last.chunks(4).zip(without.chunks(4)).filter(|(a, b)| a != b).count();
        assert!(shown > 50, "SSGI changes only {shown} pixels — the comparison would not see it");
    });
}

/// **The volumetric smoke steps once a frame, however many views draw it.** A texture camera
/// standing where the frame's camera stands draws the frame's picture, the smoke included, frame
/// after frame. The step was inside the draw: each view — the texture camera, then the frame's own
/// — stepped the grid again, so the smoke ran twice as fast and the two views drew different steps.
#[test]
#[ignore = "requires a GPU adapter"]
fn the_smoke_steps_once_a_frame_whatever_views_draw_it() {
    use crate::renderer::components::CameraTarget;
    use crate::renderer::gpu_smoke::SmokeVolume;

    let _gpu = crate::test_gpu::gpu_lock();
    if !usable_gpu() {
        eprintln!("skipping: no usable GPU adapter");
        return;
    }
    pollster::block_on(async {
        let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
        quiet(&mut renderer);
        let mut smoke = SmokeVolume::new(&renderer.device, &renderer.scene.global_bind_group_layout, wgpu::TextureFormat::Rgba16Float);
        smoke.bounds_min = [-1.8, 0.02, -1.8];
        smoke.bounds_max = [1.8, 4.0, 1.8];
        smoke.source = [0.0, 0.8, 0.0];
        smoke.source_radius = 0.6;
        smoke.inject = 9.0;
        smoke.buoyancy = 1.7;
        smoke.density_scale = 1.6;
        renderer.smoke = Some(smoke);
        let gpu = renderer.gpu_context();
        let mut game = World::new();
        game.insert_resource(clock(1.0 / 60.0));
        game.spawn_bundle(crate::bundles::DirectionalLightBundle::default());
        let eye = crate::bundles::CameraBundle {
            position: Vec3::new(-7.0, 2.0, 0.0),
            primary: true,
            ..Default::default()
        };
        game.spawn_bundle(eye);
        let twin = game.spawn_bundle(crate::bundles::CameraBundle { primary: false, ..eye });
        let target = CameraTarget::new(&gpu, 128, 128);
        game.add_component(twin, target.clone());

        let mut first = None;
        let mut last = Vec::new();
        for frame in 0..6 {
            prepare_frame(&mut game, &mut renderer);
            last = frame_bytes(&mut renderer, |e, v, r| draw_frame(&game, e, v, r));
            let through_twin = texture_bytes(&renderer, target.texture());
            let differ = last.chunks(4).zip(through_twin.chunks(4)).filter(|(a, b)| a != b).count();
            assert_eq!(differ, 0, "frame {frame}: the twin drew another smoke step ({differ} pixels)");
            first.get_or_insert_with(|| last.clone());
        }
        let moved = first.unwrap().chunks(4).zip(last.chunks(4)).filter(|(a, b)| a != b).count();
        assert!(moved > 50, "the smoke changed only {moved} pixels in six frames — the comparison would not see it");
    });
}

/// **A view is fitted to the shape it is drawn at.** A capture's own shape wins — a mirror
/// reflecting a wide split-screen view projects at that view's shape, and its shadow cascades were
/// fitted to the whole window's, so the reflection lost its shadows at the sides — then the view's
/// rectangle, then the whole target.
#[test]
fn a_view_is_fitted_to_its_own_shape() {
    let mirror = super::FixedView {
        position: Vec3::ZERO,
        forward: Vec3::NEG_Z,
        up: Vec3::Y,
        near: 0.1,
        far: 100.0,
        projection: None,
        aspect: 3.5,
    };
    assert_eq!(super::view_aspect(Some(&mirror), Some([0.0, 0.0, 128.0, 64.0]), (128, 128)), 3.5);
    assert_eq!(super::view_aspect(None, Some([0.0, 0.0, 128.0, 64.0]), (128, 128)), 2.0);
    assert_eq!(super::view_aspect(None, None, (256, 64)), 4.0);
    assert_eq!(super::view_aspect(None, None, (256, 0)), 1.0);
}

/// A red cube with the frame's camera on −X, a texture camera on +Z looking at it, and two more
/// cameras on +X and −Z that take the window's two halves in the frames `split` says. Draws the
/// frames with TAA on and returns the texture camera's last picture, with the renderer and world
/// that drew it and the cameras — the texture camera's and the two split ones.
async fn texture_camera_after(split: &[bool]) -> (Vec<u8>, Renderer, World, [Entity; 3]) {
    use crate::renderer::components::{CameraTarget, Viewport};

    let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
    renderer.ssgi = None;
    renderer.ssr = None;
    renderer.volumetric = None;
    assert!(renderer.taa.is_some(), "the comparison is of temporal state: it needs TAA");
    let gpu = renderer.gpu_context();
    let mut game = World::new();
    game.insert_resource(clock(1.0 / 60.0));
    cube_scene(&renderer, &mut game);
    let around = |position: Vec3, yaw: f32| crate::bundles::CameraBundle { position, yaw, primary: false, ..Default::default() };
    let watcher = game.spawn_bundle(around(Vec3::new(0.0, 0.0, 6.0), -std::f32::consts::FRAC_PI_2));
    let target = CameraTarget::new(&gpu, 128, 128);
    game.add_component(watcher, target.clone());
    let left = game.spawn_bundle(around(Vec3::new(6.0, 0.0, 0.0), std::f32::consts::PI));
    let right = game.spawn_bundle(around(Vec3::new(0.0, 0.0, -6.0), std::f32::consts::FRAC_PI_2));
    for &split_now in split {
        for (camera, viewport) in [(left, Viewport::new(0.0, 0.0, 0.5, 1.0)), (right, Viewport::new(0.5, 0.0, 0.5, 1.0))] {
            let mut lens = *game.get::<Camera>(camera).expect("a camera");
            lens.viewport = split_now.then_some(viewport);
            game.add_component(camera, lens);
        }
        prepare_frame(&mut game, &mut renderer);
        frame_bytes(&mut renderer, |e, v, r| draw_frame(&game, e, v, r));
    }
    let picture = texture_bytes(&renderer, target.texture());
    (picture, renderer, game, [watcher, left, right])
}

/// **A view's temporal state is its camera's.** A texture camera draws the same picture on its
/// fourth frame whether the three before it were split-screen frames or not; the renderer holds a
/// state for the cameras it drew besides the frame's own, and lets one go when its camera is gone.
/// The states were numbered slots, the texture cameras' after the split's own views: with the
/// split's two views drawn the texture camera's was the third, back on one view the first — which a
/// split-screen camera had filled — so it blended another camera's picture into its own. And a slot
/// was never let go.
#[test]
#[ignore = "requires a GPU adapter"]
fn a_view_keeps_its_cameras_temporal_state_whatever_views_the_frame_has() {
    let _gpu = crate::test_gpu::gpu_lock();
    if !usable_gpu() {
        eprintln!("skipping: no usable GPU adapter");
        return;
    }
    pollster::block_on(async {
        let (alone, ..) = texture_camera_after(&[false; 4]).await;
        let red = |p: &[u8]| p[0] > 40 && u32::from(p[0]) > 2 * u32::from(p[1].max(p[2])) + 10;
        assert!(alone.chunks(4).filter(|p| red(p)).count() > 200, "the texture camera does not see the cube");

        let (after_split, mut renderer, mut game, [watcher, left, right]) =
            texture_camera_after(&[true, true, true, false]).await;
        let differ = alone.chunks(4).zip(after_split.chunks(4)).filter(|(a, b)| a != b).count();
        assert_eq!(differ, 0, "after split-screen frames the texture camera drew another picture ({differ} pixels): not its own history");

        let mut held = renderer.view_history_cameras();
        held.sort();
        let mut drawn = vec![watcher, left, right];
        drawn.sort();
        assert_eq!(held, drawn, "the renderer holds temporal state for other cameras than the ones it drew besides the frame's own");
        game.despawn(left);
        game.despawn(right);
        prepare_frame(&mut game, &mut renderer);
        frame_bytes(&mut renderer, |e, v, r| draw_frame(&game, e, v, r));
        assert_eq!(renderer.view_history_cameras(), vec![watcher], "a despawned camera's temporal state was kept");
    });
}

/// **Every view draws what the frame's step simulated, whoever chooses how much.** At the editor's
/// full detail, in a scene that never turned the fluid on, the step runs none of it, and a texture
/// camera that sees both where the fluid's particles start and where its first step gathers them
/// draws the picture it draws with no fluid system at all; turned on, the step runs the whole fluid
/// and the camera shows it. The particles: none while nothing is alive, the whole ring at full
/// detail once something is. The editor drew its monitors at full detail with the fluid's switch
/// never read, so a scene without fluid showed the fluid's unstepped particles on every monitor.
/// A pool of 10 000, not the default 100 000: that many have hung this machine's GPU in a test.
#[test]
#[ignore = "requires a GPU adapter"]
fn a_view_draws_what_the_frames_step_simulated() {
    use super::{draw_ahead_of_views, SimulationDetail};
    use crate::renderer::components::CameraTarget;
    use crate::renderer::gpu_particles::GpuParticle;

    let _gpu = crate::test_gpu::gpu_lock();
    if !usable_gpu() {
        eprintln!("skipping: no usable GPU adapter");
        return;
    }
    pollster::block_on(async {
        let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
        quiet(&mut renderer);
        if renderer.gpu_fluid.is_none() || renderer.gpu_particles.is_none() {
            eprintln!("skipping: this renderer has no GPU fluid or particles");
            return;
        }
        let hdr = renderer.post.hdr_texture.format();
        let (width, height) = (renderer.config.width, renderer.config.height);
        renderer.gpu_fluid = Some(crate::renderer::gpu_fluid::GpuFluidSystem::new(
            &renderer.device,
            &renderer.queue,
            10_000,
            &renderer.scene.global_bind_group_layout,
            hdr,
            width,
            height,
        ));
        let gpu = renderer.gpu_context();
        let mut game = World::new();
        game.insert_resource(clock(1.0 / 60.0));
        game.spawn_bundle(crate::bundles::DirectionalLightBundle::default());
        // The particles start as a sheet on the floor of a 10 m tank and the first step gathers them
        // round (0, 8, 0): from here the camera sees both.
        let monitor = game.spawn_bundle(crate::bundles::CameraBundle {
            position: Vec3::new(-12.0, 8.0, 0.0),
            pitch: -0.35,
            primary: false,
            ..Default::default()
        });
        let target = CameraTarget::new(&gpu, 128, 128);
        game.add_component(monitor, target.clone());
        prepare_frame(&mut game, &mut renderer);
        let fluid = |r: &Renderer| r.gpu_fluid.as_ref().map(|f| (f.active_particles, f.num_particles)).expect("a fluid");
        let particles = |r: &Renderer| r.gpu_particles.as_ref().map(|p| (p.active_particles, p.max_particles)).expect("particles");
        let differ = |a: &[u8], b: &[u8]| a.chunks(4).zip(b.chunks(4)).filter(|(a, b)| a != b).count();

        draw_ahead_of_views(&game, &mut renderer, SimulationDetail::Full);
        assert_eq!(fluid(&renderer).0, 0, "the step ran a fluid the scene never turned on");
        assert_eq!(particles(&renderer).0, 0, "the step ran particles with none alive");
        let off = texture_bytes(&renderer, target.texture());
        let pool = renderer.gpu_fluid.take();
        draw_ahead_of_views(&game, &mut renderer, SimulationDetail::Full);
        let no_system = texture_bytes(&renderer, target.texture());
        renderer.gpu_fluid = pool;
        assert_eq!(differ(&off, &no_system), 0, "the monitor drew a fluid the scene never turned on");

        renderer.fluid_enabled = true;
        draw_ahead_of_views(&game, &mut renderer, SimulationDetail::Full);
        let (active, all) = fluid(&renderer);
        assert_eq!(active, all, "full detail ran {active} of the fluid's {all} particles");
        let on = texture_bytes(&renderer, target.texture());
        assert!(differ(&off, &on) > 200, "the monitor does not show the pool ({} pixels changed)", differ(&off, &on));

        let alive = GpuParticle {
            position: [0.0, 2.0, 0.0],
            life: 0.0,
            velocity: [0.0, 0.0, 0.0],
            max_life: 10.0,
            color: [1.0, 1.0, 1.0, 1.0],
            size_start: 0.2,
            size_end: 0.2,
            _padding: [0.0; 2],
        };
        renderer.gpu_particles.as_ref().expect("checked above").spawn_particles(&renderer.queue, &[alive]);
        draw_ahead_of_views(&game, &mut renderer, SimulationDetail::Full);
        let (active, all) = particles(&renderer);
        assert_eq!(active, all, "full detail ran {active} of the particle ring's {all}");
    });
}

/// **The GPU bodies step on the game's clock**, in whole steps of their own fixed size: two frames of
/// 1/60 s and one of 1/30 s are two steps each, a paused clock steps them none, and the rest of a
/// short frame is carried to the next. They stepped once for every frame drawn, whatever its length
/// — twice as fast at 120 Hz as at 60 — and went on falling under ⏸.
///
/// With them, what the solver is told: a plane is uploaded once (it took the next slot of a counter
/// shared by the whole process every frame, and filled the table with copies of itself in some
/// ninety frames); the solver checks the live static colliders, not all hundred slots; and a joint
/// reaches it when it is added, not when the game next happens to call `update_params`.
#[cfg(feature = "physics")]
#[test]
#[ignore = "requires a GPU adapter"]
fn the_gpu_bodies_step_on_the_games_clock() {
    use super::{draw_ahead_of_views, SimulationDetail};
    use crate::renderer::gpu_physics::GpuJoint;
    use gizmo_core::time::Time;

    let _gpu = crate::test_gpu::gpu_lock();
    if !usable_gpu() {
        eprintln!("skipping: no usable GPU adapter");
        return;
    }
    pollster::block_on(async {
        let mut renderer = crate::test_gpu::headless_renderer(64, 64).await;
        quiet(&mut renderer);
        renderer.enable_gpu_physics();
        if renderer.gpu_physics.is_none() {
            eprintln!("skipping: this renderer has no GPU bodies");
            return;
        }
        let mut game = World::new();
        game.insert_resource(Time::new());
        let floor = game.spawn_bundle((
            Transform::new(Vec3::ZERO),
            gizmo_physics_rigid::components::RigidBody::new(0.0, false),
            gizmo_physics_core::Collider::plane(Vec3::Y, 0.0),
        ));
        let frame = |game: &mut World, renderer: &mut Renderer, dt: f32| {
            game.get_resource_mut::<Time>().expect("a clock").update(dt);
            prepare_frame(game, renderer);
            draw_ahead_of_views(game, renderer, SimulationDetail::Full);
        };
        fn physics(r: &Renderer) -> &crate::renderer::gpu_physics::GpuPhysicsSystem {
            r.gpu_physics.as_ref().expect("GPU bodies")
        }
        let initial_colliders = physics(&renderer).params().num_colliders;

        frame(&mut game, &mut renderer, 1.0 / 60.0);
        frame(&mut game, &mut renderer, 1.0 / 60.0);
        assert_eq!(physics(&renderer).clock().step_count(), 2, "two frames of 1/60 s");
        frame(&mut game, &mut renderer, 1.0 / 30.0);
        assert_eq!(physics(&renderer).clock().step_count(), 4, "a frame of 1/30 s is two steps");
        game.get_resource_mut::<Time>().expect("a clock").set_paused(true);
        frame(&mut game, &mut renderer, 1.0 / 60.0);
        assert_eq!(physics(&renderer).clock().step_count(), 4, "a paused clock steps the bodies");
        game.get_resource_mut::<Time>().expect("a clock").set_paused(false);
        frame(&mut game, &mut renderer, 1.0 / 120.0);
        assert_eq!(physics(&renderer).clock().step_count(), 4, "half a step is owed, not run");
        frame(&mut game, &mut renderer, 1.0 / 120.0);
        assert_eq!(physics(&renderer).clock().step_count(), 5, "and runs once the next half arrives");

        assert_eq!(
            physics(&renderer).params().num_colliders,
            initial_colliders + 1,
            "the plane is one static collider however many frames it is in"
        );
        assert!(game.get::<crate::systems::physics::GpuStaticCollider>(floor).is_some(), "the plane is marked uploaded");
        assert!(initial_colliders < 10, "the solver is told the live colliders ({initial_colliders}), not every slot");

        let queue = renderer.queue.clone();
        let joint = renderer.gpu_physics.as_mut().expect("GPU bodies").add_joint(&queue, GpuJoint::ball(0, 1, [0.0; 3], [0.0; 3]));
        assert_eq!(joint, Some(0));
        assert_eq!(physics(&renderer).params().num_joints, 1, "the joint never reached the solver's parameters");
    });
}

/// Every box slot of the renderer's GPU bodies, read back.
#[cfg(feature = "physics")]
fn gpu_boxes(renderer: &Renderer) -> Vec<crate::renderer::gpu_physics::GpuBox> {
    let physics = renderer.gpu_physics.as_ref().expect("GPU bodies");
    let size = physics.boxes_buffer.size();
    let staging = renderer.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("gpu-boxes-readback"),
        size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = renderer.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    encoder.copy_buffer_to_buffer(&physics.boxes_buffer, 0, &staging, 0, size);
    renderer.queue.submit(Some(encoder.finish()));
    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |v| tx.send(v).unwrap());
    let _ = renderer.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
    rx.recv().unwrap().unwrap();
    let boxes = bytemuck::cast_slice(&slice.get_mapped_range().expect("a just-mapped range")).to_vec();
    staging.unmap();
    boxes
}

/// A frame of `dt` seconds for the GPU bodies: the preparation (which uploads and frees them) and
/// everything before the frame's own view (which steps them).
#[cfg(feature = "physics")]
fn gpu_body_frame(game: &mut World, renderer: &mut Renderer, dt: f32) {
    game.get_resource_mut::<gizmo_core::time::Time>().expect("a clock").update(dt);
    prepare_frame(game, renderer);
    super::draw_ahead_of_views(game, renderer, super::SimulationDetail::Full);
}

/// **A GPU body leaves when its entity does, and its slot is nobody else's while it is alive.** The
/// table starts empty. A body despawned is out of the simulation by the next frame — its slot reads
/// free, and stays so however long the bodies step — and the next body in takes that slot, not the
/// one of a body still alive. A plane despawned frees its static-collider slot for the next.
///
/// There was no way out: slots were numbered by counting the linked bodies, so the third body in,
/// after the first had gone, was handed the second's slot, and a despawned body went on falling and
/// being drawn. And every game began with 50 000 falling cubes and three static colliders in it.
#[cfg(feature = "physics")]
#[test]
#[ignore = "requires a GPU adapter"]
fn a_gpu_body_leaves_with_its_entity_and_its_slot_is_nobody_elses() {
    use crate::renderer::gpu_physics::{GpuBox, BOX_FREE};
    use gizmo_physics_core::components::GpuPhysicsLink;
    use gizmo_physics_core::Collider;
    use gizmo_physics_rigid::components::RigidBody;

    let _gpu = crate::test_gpu::gpu_lock();
    if !usable_gpu() {
        eprintln!("skipping: no usable GPU adapter");
        return;
    }
    pollster::block_on(async {
        let mut renderer = crate::test_gpu::headless_renderer(64, 64).await;
        quiet(&mut renderer);
        renderer.enable_gpu_physics();
        if renderer.gpu_physics.is_none() {
            eprintln!("skipping: this renderer has no GPU bodies");
            return;
        }
        fn physics(r: &Renderer) -> &crate::renderer::gpu_physics::GpuPhysicsSystem {
            r.gpu_physics.as_ref().expect("GPU bodies")
        }
        let params = physics(&renderer).params();
        assert_eq!((physics(&renderer).live_boxes(), params.num_boxes, params.num_colliders), (0, 0, 0), "the table starts empty");

        let mut game = World::new();
        game.insert_resource(gizmo_core::time::Time::new());
        let body = |game: &mut World, x: f32| {
            game.spawn_bundle((
                Transform::new(Vec3::new(x, 50.0, 0.0)),
                RigidBody::new(1.0, true),
                Collider::box_collider(Vec3::splat(0.5)),
            ))
        };
        let slot = |game: &World, e: Entity| game.get::<GpuPhysicsLink>(e).map(|l| l.id);
        let first = body(&mut game, 0.0);
        let second = body(&mut game, 10.0);
        gpu_body_frame(&mut game, &mut renderer, 1.0 / 60.0);
        let (first_slot, second_slot) = (slot(&game, first).expect("linked"), slot(&game, second).expect("linked"));
        assert_ne!(first_slot, second_slot);
        assert_eq!(physics(&renderer).live_boxes(), 2);

        game.despawn(first);
        for _ in 0..10 {
            gpu_body_frame(&mut game, &mut renderer, 1.0 / 60.0);
        }
        assert_eq!(physics(&renderer).live_boxes(), 1, "the despawned body is still in the table");
        let freed = gpu_boxes(&renderer)[first_slot as usize];
        assert_eq!(freed.state, BOX_FREE, "the despawned body's slot is still simulated");
        assert_eq!(bytemuck::bytes_of(&freed), bytemuck::bytes_of(&GpuBox::FREE), "a freed slot was moved by the steps after it");

        let third = body(&mut game, -10.0);
        gpu_body_frame(&mut game, &mut renderer, 1.0 / 60.0);
        let third_slot = slot(&game, third).expect("linked");
        assert_ne!(third_slot, second_slot, "the new body was handed a living body's slot");
        assert_eq!(third_slot, first_slot, "a freed slot is given out first");

        let floor = game.spawn_bundle((Transform::new(Vec3::ZERO), RigidBody::new(0.0, false), Collider::plane(Vec3::Y, 0.0)));
        gpu_body_frame(&mut game, &mut renderer, 1.0 / 60.0);
        let plane_slot = game.get::<crate::systems::physics::GpuStaticCollider>(floor).expect("uploaded").0;
        game.despawn(floor);
        gpu_body_frame(&mut game, &mut renderer, 1.0 / 60.0);
        let again = game.spawn_bundle((Transform::new(Vec3::ZERO), RigidBody::new(0.0, false), Collider::plane(Vec3::Y, 0.0)));
        gpu_body_frame(&mut game, &mut renderer, 1.0 / 60.0);
        assert_eq!(game.get::<crate::systems::physics::GpuStaticCollider>(again).map(|c| c.0), Some(plane_slot), "a plane's freed slot is given out again");
        assert_eq!(physics(&renderer).params().num_colliders, 1, "two planes, one after the other, took two slots");

        // A link the table does not back — copied from the living body — is dropped and the entity
        // uploaded again, into a slot of its own.
        let copied = body(&mut game, 20.0);
        game.add_component(copied, GpuPhysicsLink { id: second_slot });
        gpu_body_frame(&mut game, &mut renderer, 1.0 / 60.0);
        let own = slot(&game, copied).expect("linked again");
        assert_ne!(own, second_slot, "an entity carrying another body's link was left mirroring it");
        assert!(physics(&renderer).holds_box(own, copied) && physics(&renderer).holds_box(second_slot, second));

        // A run of bodies goes in past the highest slot in use, as many as fit; and a table emptied
        // from the top gives that room back.
        let queue = renderer.queue.clone();
        let before = physics(&renderer).params().num_boxes;
        let run = renderer.gpu_physics.as_mut().expect("GPU bodies").add_boxes(&queue, &[GpuBox { state: 0, mass: 1.0, ..GpuBox::FREE }; 4]);
        assert_eq!(run, before..before + 4);
        assert_eq!(physics(&renderer).params().num_boxes, before + 4);
        for slot in run.clone() {
            renderer.gpu_physics.as_mut().expect("GPU bodies").remove_box(&queue, slot);
        }
        for _ in 0..3 {
            gpu_body_frame(&mut game, &mut renderer, 1.0 / 60.0);
        }
        assert_eq!(physics(&renderer).params().num_boxes, before, "the freed top of the table was not given back");
        let max = physics(&renderer).max_boxes;
        let all = vec![GpuBox { state: 0, mass: 1.0, ..GpuBox::FREE }; max as usize];
        let fitted = renderer.gpu_physics.as_mut().expect("GPU bodies").add_boxes(&queue, &all);
        assert_eq!(fitted, before..max, "a run that does not all fit puts in what does");
    });
}

/// **A GPU body comes to rest on the ground the game gave it, and the ground stays.** A massless box
/// for a floor and a box dropped onto it: two seconds later the box sits on the floor and the floor
/// has not moved. A massless box — immovable, by `GpuBox::mass` — had gravity added like any other
/// (it fell 6.5 m in those two seconds; `kumas`'s pins, massless, never held its cloth), and the push
/// out of a contact was shared by mass, which made a massless body the lightest of all: the floor
/// was pushed down out from under the box (0.49 m) and the box not at all.
#[cfg(feature = "physics")]
#[test]
#[ignore = "requires a GPU adapter"]
fn a_gpu_body_rests_on_the_ground_the_game_gave_it() {
    use gizmo_physics_core::components::GpuPhysicsLink;
    use gizmo_physics_core::Collider;
    use gizmo_physics_rigid::components::RigidBody;

    let _gpu = crate::test_gpu::gpu_lock();
    if !usable_gpu() {
        eprintln!("skipping: no usable GPU adapter");
        return;
    }
    pollster::block_on(async {
        let mut renderer = crate::test_gpu::headless_renderer(64, 64).await;
        quiet(&mut renderer);
        renderer.enable_gpu_physics();
        if renderer.gpu_physics.is_none() {
            eprintln!("skipping: this renderer has no GPU bodies");
            return;
        }
        let mut game = World::new();
        game.insert_resource(gizmo_core::time::Time::new());
        let floor = game.spawn_bundle((
            Transform::new(Vec3::ZERO),
            RigidBody::new(0.0, false),
            Collider::box_collider(Vec3::new(2.5, 0.1, 2.5)),
        ));
        let dropped = game.spawn_bundle((
            Transform::new(Vec3::new(0.0, 2.0, 0.0)),
            RigidBody::new(1.0, true),
            Collider::box_collider(Vec3::splat(0.3)),
        ));
        for _ in 0..120 {
            gpu_body_frame(&mut game, &mut renderer, 1.0 / 60.0);
        }
        let boxes = gpu_boxes(&renderer);
        let at = |e: Entity| boxes[game.get::<GpuPhysicsLink>(e).expect("linked").id as usize].position;
        let (floor_at, dropped_at) = (at(floor), at(dropped));
        assert!(floor_at.iter().all(|c| c.abs() < 1e-4), "the massless floor moved to {floor_at:?}");
        assert!(
            (dropped_at[1] - 0.4).abs() < 0.1 && dropped_at[0].abs() < 0.5 && dropped_at[2].abs() < 0.5,
            "the box is at {dropped_at:?}, not resting on the floor (its centre would be at y = 0.4)"
        );
    });
}

/// A massless floor and two boxes stacked on it, dropped a hair: returns the lower and the upper.
#[cfg(feature = "physics")]
fn gpu_stack(game: &mut World) -> (Entity, Entity) {
    use gizmo_physics_core::Collider;
    use gizmo_physics_rigid::components::RigidBody;
    game.spawn_bundle((Transform::new(Vec3::ZERO), RigidBody::new(0.0, false), Collider::box_collider(Vec3::new(2.5, 0.1, 2.5))));
    let cube = |game: &mut World, y: f32| {
        game.spawn_bundle((Transform::new(Vec3::new(0.0, y, 0.0)), RigidBody::new(1.0, true), Collider::box_collider(Vec3::splat(0.3))))
    };
    (cube(game, 0.41), cube(game, 1.02))
}

/// **A box resting on a box falls asleep**, and so does the one under it. It never did: the contact
/// between two bodies is resolved in the solve, before `integrate` adds the step's gravity and judges
/// whether the body has stopped, so a resting box always showed the g·dt its support would take away
/// next step; and the solve bounced that back at the restitution besides. Only a body on a static
/// collider ever slept, so a heap of GPU bodies was simulated in full for as long as it lay there.
#[cfg(feature = "physics")]
#[test]
#[ignore = "requires a GPU adapter"]
fn a_box_resting_on_a_box_falls_asleep() {
    use gizmo_physics_core::components::GpuPhysicsLink;

    let _gpu = crate::test_gpu::gpu_lock();
    if !usable_gpu() {
        eprintln!("skipping: no usable GPU adapter");
        return;
    }
    pollster::block_on(async {
        let mut renderer = crate::test_gpu::headless_renderer(64, 64).await;
        quiet(&mut renderer);
        renderer.enable_gpu_physics();
        if renderer.gpu_physics.is_none() {
            eprintln!("skipping: this renderer has no GPU bodies");
            return;
        }
        let mut game = World::new();
        game.insert_resource(gizmo_core::time::Time::new());
        let (lower, upper) = gpu_stack(&mut game);
        for _ in 0..480 {
            gpu_body_frame(&mut game, &mut renderer, 1.0 / 60.0);
        }
        let boxes = gpu_boxes(&renderer);
        let at = |e: Entity| boxes[game.get::<GpuPhysicsLink>(e).expect("linked").id as usize];
        let (lower, upper) = (at(lower), at(upper));
        assert_eq!((lower.state, upper.state), (1, 1), "after eight seconds of rest the stack is not asleep");
        assert!((lower.position[1] - 0.4).abs() < 0.05 && (upper.position[1] - 1.0).abs() < 0.08, "the stack is not standing: {:?} {:?}", lower.position, upper.position);
    });
}

/// **A body taken out wakes the sleeping bodies round it.** A box asleep on a box asleep on the floor:
/// take the lower one out and the upper one falls to the floor. Nothing told the bodies round a freed
/// slot that it had gone, so the upper box stayed asleep in the air.
///
/// It needed three more things to be seen at all, each found on the way. The stack had to sleep (see
/// `a_box_resting_on_a_box_falls_asleep`). The solver read every static collider from the wrong bytes
/// (`StaticCollider`'s padding was a `vec3`, which WGSL aligns to 16), so nothing ever landed on one.
/// And a body resting on a static collider bounced each step's gravity back at the restitution, a few
/// centimetres a second for ever, above the energy a body must fall under to sleep.
#[cfg(feature = "physics")]
#[test]
#[ignore = "requires a GPU adapter"]
fn a_body_taken_out_wakes_the_sleeping_bodies_round_it() {
    use gizmo_physics_core::components::GpuPhysicsLink;

    let _gpu = crate::test_gpu::gpu_lock();
    if !usable_gpu() {
        eprintln!("skipping: no usable GPU adapter");
        return;
    }
    pollster::block_on(async {
        let mut renderer = crate::test_gpu::headless_renderer(64, 64).await;
        quiet(&mut renderer);
        renderer.enable_gpu_physics();
        if renderer.gpu_physics.is_none() {
            eprintln!("skipping: this renderer has no GPU bodies");
            return;
        }
        let mut game = World::new();
        game.insert_resource(gizmo_core::time::Time::new());
        let (lower, upper) = gpu_stack(&mut game);
        for _ in 0..480 {
            gpu_body_frame(&mut game, &mut renderer, 1.0 / 60.0);
        }
        let slot = |game: &World, e: Entity| game.get::<GpuPhysicsLink>(e).expect("linked").id as usize;
        let upper_slot = slot(&game, upper);
        let boxes = gpu_boxes(&renderer);
        assert_eq!((boxes[slot(&game, lower)].state, boxes[upper_slot].state), (1, 1), "the stack did not fall asleep, so this proves nothing");
        let resting = boxes[upper_slot].position[1];

        game.despawn(lower);
        for _ in 0..90 {
            gpu_body_frame(&mut game, &mut renderer, 1.0 / 60.0);
        }
        let fallen = gpu_boxes(&renderer)[upper_slot].position[1];
        assert!(
            fallen < resting - 0.4 && (fallen - 0.4).abs() < 0.1,
            "the box that rested on the one taken out is at y = {fallen} (it rested at {resting}); on the floor it is at 0.4"
        );
    });
}

/// **A body put in is not moved by a readback copied before it.** A readback arrives a frame or more
/// after it is copied; a body put in meanwhile was given that copy's slot — empty, at the origin —
/// and its entity jumped there for a frame. Every frame of a body's first fall keeps it near where
/// it was put.
#[cfg(feature = "physics")]
#[test]
#[ignore = "requires a GPU adapter"]
fn a_body_put_in_is_not_moved_by_a_readback_copied_before_it() {
    use gizmo_physics_core::Collider;
    use gizmo_physics_rigid::components::RigidBody;

    let _gpu = crate::test_gpu::gpu_lock();
    if !usable_gpu() {
        eprintln!("skipping: no usable GPU adapter");
        return;
    }
    pollster::block_on(async {
        let mut renderer = crate::test_gpu::headless_renderer(64, 64).await;
        quiet(&mut renderer);
        renderer.enable_gpu_physics();
        if renderer.gpu_physics.is_none() {
            eprintln!("skipping: this renderer has no GPU bodies");
            return;
        }
        let mut game = World::new();
        game.insert_resource(gizmo_core::time::Time::new());
        let mut falling = Vec::new();
        for round in 0..6 {
            // A readback is in flight from the frames before; a body goes in now.
            gpu_body_frame(&mut game, &mut renderer, 1.0 / 60.0);
            let x = round as f32 * 3.0;
            falling.push((x, game.spawn_bundle((
                Transform::new(Vec3::new(x, 50.0, 0.0)),
                RigidBody::new(1.0, true),
                Collider::box_collider(Vec3::splat(0.5)),
            ))));
            for frame in 0..4 {
                gpu_body_frame(&mut game, &mut renderer, 1.0 / 60.0);
                for &(x, e) in &falling {
                    let at = game.get::<Transform>(e).expect("a transform").position;
                    assert!(
                        at.y > 45.0 && (at.x - x).abs() < 0.5,
                        "round {round}, frame {frame}: a body put in at ({x}, 50) was read back at {at:?}"
                    );
                }
            }
        }
    });
}

/// **A body given a slot another body left starts with none of that body's contacts.** A box rests on
/// a floor for ten seconds and is taken out; a box put in its place, in its slot, and thrown upwards
/// rises. The contact manifold is kept per slot and nothing cleared it, so the new box took over the
/// old one's contact with the floor — and the impulse it had accumulated holding the old box up,
/// which held the new one down.
#[cfg(feature = "physics")]
#[test]
#[ignore = "requires a GPU adapter"]
fn a_body_in_a_left_slot_starts_with_none_of_its_contacts() {
    use gizmo_physics_core::components::GpuPhysicsLink;
    use gizmo_physics_core::Collider;
    use gizmo_physics_rigid::components::{RigidBody, Velocity};

    let _gpu = crate::test_gpu::gpu_lock();
    if !usable_gpu() {
        eprintln!("skipping: no usable GPU adapter");
        return;
    }
    pollster::block_on(async {
        let mut renderer = crate::test_gpu::headless_renderer(64, 64).await;
        quiet(&mut renderer);
        renderer.enable_gpu_physics();
        if renderer.gpu_physics.is_none() {
            eprintln!("skipping: this renderer has no GPU bodies");
            return;
        }
        let mut game = World::new();
        game.insert_resource(gizmo_core::time::Time::new());
        game.spawn_bundle((Transform::new(Vec3::ZERO), RigidBody::new(0.0, false), Collider::box_collider(Vec3::new(2.5, 0.1, 2.5))));
        let first = game.spawn_bundle((Transform::new(Vec3::new(0.0, 0.4, 0.0)), RigidBody::new(1.0, true), Collider::box_collider(Vec3::splat(0.3))));
        for _ in 0..600 {
            gpu_body_frame(&mut game, &mut renderer, 1.0 / 60.0);
        }
        let first_slot = game.get::<GpuPhysicsLink>(first).expect("linked").id;
        game.despawn(first);
        for _ in 0..3 {
            gpu_body_frame(&mut game, &mut renderer, 1.0 / 60.0);
        }
        let thrown = game.spawn_bundle((
            Transform::new(Vec3::new(0.0, 0.4, 0.0)),
            RigidBody::new(1.0, true),
            Collider::box_collider(Vec3::splat(0.3)),
            Velocity { linear: Vec3::new(0.0, 3.0, 0.0), ..Default::default() },
        ));
        for _ in 0..10 {
            gpu_body_frame(&mut game, &mut renderer, 1.0 / 60.0);
        }
        let slot = game.get::<GpuPhysicsLink>(thrown).expect("linked").id;
        assert_eq!(slot, first_slot, "the thrown box was not given the left slot, so this proves nothing");
        let height = gpu_boxes(&renderer)[slot as usize].position[1];
        assert!(height > 0.6, "the box thrown up at 3 m/s from 0.4 m is at {height} m a sixth of a second later");
    });
}

/// **A contact pushes, and never pulls.** A box that has rested on the floor for a second, thrown
/// up, rises; and when a box resting on a box is thrown off it, the lower box stays where it rests.
/// A contact's accumulated impulse was carried from step to step and never applied as a warm start,
/// so it grew by the weight it held every step and the solve's clamp — all that keeps a contact
/// from pulling — never bit: the thrown box was pulled back down (it did not rise at all after a
/// second's rest), and a lower box was pulled 0.3 m up after the one thrown off it.
#[cfg(feature = "physics")]
#[test]
#[ignore = "requires a GPU adapter"]
fn a_contact_pushes_and_never_pulls() {
    use gizmo_physics_core::components::GpuPhysicsLink;
    use gizmo_physics_core::Collider;
    use gizmo_physics_rigid::components::{RigidBody, Velocity};

    let _gpu = crate::test_gpu::gpu_lock();
    if !usable_gpu() {
        eprintln!("skipping: no usable GPU adapter");
        return;
    }
    pollster::block_on(async {
        let mut renderer = crate::test_gpu::headless_renderer(64, 64).await;
        quiet(&mut renderer);
        renderer.enable_gpu_physics();
        if renderer.gpu_physics.is_none() {
            eprintln!("skipping: this renderer has no GPU bodies");
            return;
        }
        let mut game = World::new();
        game.insert_resource(gizmo_core::time::Time::new());
        game.spawn_bundle((Transform::new(Vec3::ZERO), RigidBody::new(0.0, false), Collider::box_collider(Vec3::new(2.5, 0.1, 2.5))));
        let cube = |game: &mut World, y: f32| {
            game.spawn_bundle((Transform::new(Vec3::new(0.0, y, 0.0)), RigidBody::new(1.0, true), Collider::box_collider(Vec3::splat(0.3))))
        };

        // A box rested a second on the floor, then thrown up at 3 m/s, as a game writes it.
        let alone = cube(&mut game, 0.4);
        for _ in 0..60 {
            gpu_body_frame(&mut game, &mut renderer, 1.0 / 60.0);
        }
        let alone_slot = game.get::<GpuPhysicsLink>(alone).expect("linked").id;
        let mut thrown_up = gpu_boxes(&renderer)[alone_slot as usize];
        thrown_up.velocity = [0.0, 3.0, 0.0];
        (thrown_up.state, thrown_up.sleep_counter) = (0, 0);
        let queue = renderer.queue.clone();
        renderer.gpu_physics.as_ref().expect("GPU bodies").update_box(&queue, alone_slot, &thrown_up);
        for _ in 0..10 {
            gpu_body_frame(&mut game, &mut renderer, 1.0 / 60.0);
        }
        let risen = gpu_boxes(&renderer)[alone_slot as usize].position[1];
        assert!(risen > 0.65, "a box thrown up at 3 m/s off the floor it rested on is at {risen} m a sixth of a second later");
        game.despawn(alone);
        for _ in 0..3 {
            gpu_body_frame(&mut game, &mut renderer, 1.0 / 60.0);
        }

        let lower = cube(&mut game, 0.4);
        let top = cube(&mut game, 1.0);
        for _ in 0..600 {
            gpu_body_frame(&mut game, &mut renderer, 1.0 / 60.0);
        }
        let slot = |game: &World, e: Entity| game.get::<GpuPhysicsLink>(e).expect("linked").id as usize;
        let (lower_slot, top_slot) = (slot(&game, lower), slot(&game, top));
        let resting = gpu_boxes(&renderer)[lower_slot].position[1];
        game.despawn(top);
        for _ in 0..3 {
            gpu_body_frame(&mut game, &mut renderer, 1.0 / 60.0);
        }
        let thrown = game.spawn_bundle((
            Transform::new(Vec3::new(0.0, 1.0, 0.0)),
            RigidBody::new(1.0, true),
            Collider::box_collider(Vec3::splat(0.3)),
            Velocity { linear: Vec3::new(0.0, 3.0, 0.0), ..Default::default() },
        ));
        for _ in 0..10 {
            gpu_body_frame(&mut game, &mut renderer, 1.0 / 60.0);
        }
        assert_eq!(slot(&game, thrown), top_slot, "the thrown box was not given the left slot, so this proves nothing");
        let lower_now = gpu_boxes(&renderer)[lower_slot].position[1];
        assert!(
            (lower_now - resting).abs() < 0.05,
            "the lower box moved from {resting} to {lower_now} when the box above it was thrown off"
        );
    });
}
