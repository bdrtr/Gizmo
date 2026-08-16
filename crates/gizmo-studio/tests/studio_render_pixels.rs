//! The editor's render path, judged by the pixels it produces.
//!
//! # Why this file exists
//!
//! The engine's deferred path has seventeen golden tests that read the framebuffer back and assert
//! on it (`gizmo::systems::render::golden_render_tests`). The editor's forward path — six hundred
//! lines, the one a person is looking at all day — had **none**. Its sibling
//! `render_parity.rs` covers the shared *setup* precisely because setup is a pure function; pass
//! recording is not, so it stayed unobserved.
//!
//! What that cost: the editor viewport shipped for months rendering one gamma step too dark (the
//! egui sample-view bug, see `gizmo_app::editor_runtime`), and before that spent a month crashing
//! on startup. Neither is subtle in a screenshot. Both were invisible to a test suite that never
//! looked at a pixel.
//!
//! # What was supposedly in the way
//!
//! It was recorded that these tests need `StudioState` to be made headless-constructible first.
//! That turned out not to be true — the struct is scalars and one `Option`, with no GPU handle in
//! it — and the pipeline reaches for exactly three world resources, all optional. The entry point
//! took a headless renderer on the first attempt. The blocker was never measured, only assumed;
//! this is the third such note found in this codebase and the third that did not survive contact.
//!
//! # Scope
//!
//! Coarse assertions on purpose. A golden image would pin the editor's look, and the editor's look
//! is meant to change; these pin the properties whose loss means *broken*, which is the class of
//! bug that actually shipped: nothing drawn, everything one colour, the lit thing indistinguishable
//! from the void behind it.

use gizmo::core::World;
use gizmo::math::{Vec3, Vec4};
use gizmo::prelude::*;
use gizmo::renderer::asset::AssetManager;
use gizmo::renderer::components::MeshRenderer;
use gizmo::renderer::Renderer;
use gizmo_studio::render_pipeline::execute_render_pipeline;
use gizmo_studio::StudioState;

/// Serialises the GPU work in this binary.
///
/// Not belt-and-braces: `gizmo_renderer::test_gpu` records the measurement — four live wgpu
/// devices bring this driver down with a SIGSEGV (no panic, no test name, the whole binary dies),
/// two are fine. Cargo runs a binary's tests in parallel, and each test here builds a device, so
/// the guard is held for the whole body rather than just around device creation: the same doc
/// records that narrowing it to creation was tried and did not work. `gizmo`'s own lock is
/// `#[cfg(test)]`-private to that crate, so this binary needs its own.
static GPU_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn gpu_lock() -> std::sync::MutexGuard<'static, ()> {
    // A test that panicked while holding it poisoned it; that says nothing about the GPU.
    GPU_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

const W: u32 = 128;
const H: u32 = 128;
const BPP: u32 = 4;

/// A frame rendered by the editor's pipeline, as RGBA8 rows.
struct Frame {
    pixels: Vec<u8>,
    /// True when the surface format put blue first, so the accessors can hide it.
    bgra: bool,
}

impl Frame {
    fn at(&self, x: u32, y: u32) -> (u8, u8, u8) {
        let i = ((y * W + x) * BPP) as usize;
        let (a, b, c) = (self.pixels[i], self.pixels[i + 1], self.pixels[i + 2]);
        if self.bgra { (c, b, a) } else { (a, b, c) }
    }

    fn luma(&self, x: u32, y: u32) -> f32 {
        let (r, g, b) = self.at(x, y);
        0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b)
    }

    /// How many distinct colours the frame contains — one means the pipeline drew nothing but a
    /// clear, which every "the editor is black" report has looked like.
    fn distinct_colours(&self) -> usize {
        let mut seen = std::collections::HashSet::new();
        for px in self.pixels.chunks_exact(BPP as usize) {
            seen.insert([px[0], px[1], px[2]]);
        }
        seen.len()
    }
}

/// Builds a minimal editor scene and runs the real pipeline over it.
///
/// `game_camera` is deliberately an id no entity has: the editor culls against the game camera, and
/// with none in the scene that falls back to the drawn camera (see `viewpoint::culling_frustum`).
/// It also suppresses the game-frustum wire box, which would otherwise draw lines across the frame
/// this test measures.
fn render_scene(albedo: Vec4) -> Option<Frame> {
    if !pollster::block_on(Renderer::headless_adapter_available()) {
        eprintln!("skipping: no GPU adapter (the editor pipeline needs a real one)");
        return None;
    }
    if pollster::block_on(Renderer::headless_adapter_is_software()) {
        eprintln!("skipping: software adapter — it compiles these pipelines and cannot finish them");
        return None;
    }

    pollster::block_on(async {
        let mut renderer = Renderer::new_headless(W, H, None).await;
        let mut asset_manager = AssetManager::new();
        let mut world = World::new();

        let tex = asset_manager.create_white_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
        );

        // A cube at the origin, spanning -1..1, turned so that three faces are visible.
        //
        // The rotation is load-bearing for `the_cube_is_shaded_rather_than_filled_with_one_colour`:
        // an axis-aligned cube seen head-on from -X shows exactly one face, one normal, and
        // therefore one shade — measured at 153.6..153.6, which is correct output and reads
        // exactly like broken lighting. The first version of that test failed on this and the
        // renderer was not at fault.
        let cube = world.spawn();
        world.add_component(
            cube,
            Transform::new(Vec3::ZERO).with_rotation(gizmo::math::Quat::from_axis_angle(
                Vec3::new(1.0, 1.0, 0.0).normalize(),
                0.7,
            )),
        );
        // Deliberately NO `GlobalTransform`: the pipeline must backfill it, exactly as the
        // engine's golden tests require of the game path. Without the backfill this cube is not
        // drawn at all — measured: centre 44.0 against a background of 34.0, i.e. pure background.
        world.add_component(cube, AssetManager::create_cube(&renderer.device));
        world.add_component(cube, Material::new(tex).with_pbr(albedo, 0.6, 0.0));
        world.add_component(cube, MeshRenderer::new());

        // The editor camera, on -X looking toward +X (yaw 0 → front = +X).
        let cam = world.spawn();
        world.add_component(cam, Transform::new(Vec3::new(-6.0, 0.0, 0.0)));
        world.add_component(cam, GlobalTransform::default());
        world.add_component(
            cam,
            gizmo::renderer::components::Camera::new(
                std::f32::consts::FRAC_PI_4,
                0.1,
                1000.0,
                0.0,
                0.0,
                true,
            ),
        );

        // A sun, so the cube is lit rather than ambient-only.
        world.spawn_bundle(gizmo::prelude::DirectionalLightBundle::default());

        let state = StudioState {
            current_fps: 60.0,
            actual_dt: 1.0 / 60.0,
            editor_camera: cam.id(),
            game_camera: 4242,
            do_raycast: false,
            play: gizmo::systems::PlayLoop::new(),
            asset_watcher: None,
            gc_timer: 0.0,
            autosave_timer: 0.0,
            visible_entity_count: 0,
            draw_call_count: 0,
        };

        let format = renderer.config.format;
        let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("studio-pixel-target"),
            size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = renderer
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

        execute_render_pipeline(&mut world, &state, &mut encoder, &view, &mut renderer, 0.0);

        // W * BPP = 512, already 256-aligned.
        let staging = renderer.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("studio-pixel-readback"),
            size: u64::from(W * H * BPP),
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
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(W * BPP),
                    rows_per_image: Some(H),
                },
            },
            wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        );
        renderer.queue.submit(std::iter::once(encoder.finish()));

        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = renderer
            .device
            .poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        rx.recv().expect("readback channel").expect("readback");
        let pixels = slice.get_mapped_range()
        // wgpu 30 made this fallible; the range is the whole buffer the test just mapped.
        .expect("a just-mapped buffer's full range is always valid").to_vec();
        staging.unmap();

        Some(Frame { pixels, bgra: matches!(
            format.remove_srgb_suffix(),
            wgpu::TextureFormat::Bgra8Unorm
        ) })
    })
}

/// The floor: the editor draws something, and the something is a lit cube against a background.
///
/// This is the assertion the month-long startup crash and the gamma bug would both have tripped —
/// the first by never producing a frame, the second by flattening the difference this measures.
#[test]
fn the_editor_draws_a_lit_cube_against_its_background() {
    let _gpu = gpu_lock();
    let Some(frame) = render_scene(Vec4::new(0.85, 0.85, 0.85, 1.0)) else {
        return;
    };

    let centre = frame.luma(W / 2, H / 2);
    let corner = frame.luma(3, 3);

    assert!(
        frame.distinct_colours() > 4,
        "the frame has {} distinct colours — the pipeline recorded a clear and little else",
        frame.distinct_colours()
    );
    assert!(
        centre > corner + 12.0,
        "the cube at the centre ({centre:.1}) is not brighter than the background ({corner:.1}); \
         a bright cube that does not stand out means it was not drawn, or was not lit"
    );
    assert!(
        centre < 254.0,
        "the centre is clipped at {centre:.1} — the frame is blown out, not shaded"
    );
}

/// The cube's own shading varies across its faces.
///
/// A single flat value over the whole silhouette is what "the sun reaches the uniform but nothing
/// is lit" looks like, and it is indistinguishable from correct output at one sample point — which
/// is why the ambient-only viewport went unnoticed for so long.
#[test]
fn the_cube_is_shaded_rather_than_filled_with_one_colour() {
    let _gpu = gpu_lock();
    let Some(frame) = render_scene(Vec4::new(0.85, 0.85, 0.85, 1.0)) else {
        return;
    };

    // Samples inside the silhouette: the cube spans -1..1 six units from a 45° camera, so it
    // covers roughly the middle third of a 128 px frame.
    let mut lo = f32::MAX;
    let mut hi = f32::MIN;
    for y in (H / 2 - 12)..(H / 2 + 12) {
        for x in (W / 2 - 12)..(W / 2 + 12) {
            let l = frame.luma(x, y);
            lo = lo.min(l);
            hi = hi.max(l);
        }
    }
    assert!(
        hi - lo > 3.0,
        "the cube is a single flat value ({lo:.1}..{hi:.1}) — lighting is not reaching the surface"
    );
}

/// Two materials that differ only in albedo must not render the same.
///
/// The engine path has this test (`two_different_dark_materials_do_not_render_identically`); the
/// editor path is the one where instance packing was last changed, and it had nothing.
#[test]
fn albedo_reaches_the_editors_pixels() {
    let _gpu = gpu_lock();
    let Some(bright) = render_scene(Vec4::new(0.9, 0.9, 0.9, 1.0)) else {
        return;
    };
    let Some(dark) = render_scene(Vec4::new(0.1, 0.1, 0.1, 1.0)) else {
        return;
    };

    let b = bright.luma(W / 2, H / 2);
    let d = dark.luma(W / 2, H / 2);
    assert!(
        b > d + 20.0,
        "a 0.9 albedo cube ({b:.1}) is not meaningfully brighter than a 0.1 one ({d:.1}) — the \
         editor's instance data is not carrying the material through"
    );
}

/// The editor casts shadows onto the world, not just into a shadow map.
///
/// `record_studio_shadow_passes` renders four cascades every frame; nothing checked that a single
/// pixel of the result reaches the screen. It nearly went down as broken during the viewport
/// investigation: forcing `shadow_visibility = 1.0` in the forward shader changed the sampled
/// pixel by zero, which reads like a dead shadow path. It was not — the sample was on a lit face.
/// The scene below puts the question where it can be answered.
///
/// Measured on this path: shadowed ground luma 172.7 against 223.8 lit, and the shadow is warm
/// (195,168,153) because what survives it is the hemisphere ambient, which is warm by design. The
/// threshold sits at 0.90 — far above "no shadow" (1.0) and far below what was measured.
#[test]
fn the_editor_casts_a_shadow_onto_the_ground() {
    let _gpu = gpu_lock();
    if !pollster::block_on(Renderer::headless_adapter_available()) {
        eprintln!("skipping: no GPU adapter");
        return;
    }
    if pollster::block_on(Renderer::headless_adapter_is_software()) {
        eprintln!("skipping: software adapter");
        return;
    }

    const S: u32 = 256;
    let frame = pollster::block_on(async {
        let mut renderer = Renderer::new_headless(S, S, None).await;
        let mut am = AssetManager::new();
        let mut world = World::new();
        let tex = am.create_white_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
        );

        // A wide, rough, near-white floor two units down: one normal everywhere, so the only thing
        // that can make part of it darker than the rest is a shadow.
        let ground = world.spawn();
        world.add_component(ground, Transform::new(Vec3::new(0.0, -2.0, 0.0)));
        world.add_component(ground, AssetManager::create_plane(&renderer.device, 40.0));
        world.add_component(
            ground,
            Material::new(tex.clone()).with_pbr(Vec4::new(0.8, 0.8, 0.8, 1.0), 0.9, 0.0),
        );
        world.add_component(ground, MeshRenderer::new());

        let cube = world.spawn();
        world.add_component(cube, Transform::new(Vec3::ZERO));
        world.add_component(cube, AssetManager::create_cube(&renderer.device));
        world.add_component(
            cube,
            Material::new(tex).with_pbr(Vec4::new(0.9, 0.25, 0.25, 1.0), 0.6, 0.0),
        );
        world.add_component(cube, MeshRenderer::new());

        let cam = world.spawn();
        world.add_component(cam, Transform::new(Vec3::new(0.0, 10.0, 10.0)));
        world.add_component(
            cam,
            gizmo::renderer::components::Camera::new(
                std::f32::consts::FRAC_PI_4,
                0.1,
                1000.0,
                -std::f32::consts::FRAC_PI_2,
                -0.6,
                true,
            ),
        );

        // Tilted so the shadow lands beside the cube rather than under it, where the cube's own
        // pixels would be measured instead.
        world.spawn_bundle(gizmo::prelude::DirectionalLightBundle {
            rotation: gizmo::math::Quat::from_rotation_z(-0.9)
                * gizmo::math::Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
            ..Default::default()
        });

        let state = StudioState {
            current_fps: 60.0,
            actual_dt: 1.0 / 60.0,
            editor_camera: cam.id(),
            game_camera: 4242,
            do_raycast: false,
            play: gizmo::systems::PlayLoop::new(),
            asset_watcher: None,
            gc_timer: 0.0,
            autosave_timer: 0.0,
            visible_entity_count: 0,
            draw_call_count: 0,
        };

        let format = renderer.config.format;
        let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("studio-shadow-target"),
            size: wgpu::Extent3d { width: S, height: S, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut enc = renderer
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        execute_render_pipeline(&mut world, &state, &mut enc, &view, &mut renderer, 0.0);

        // S * 4 = 1024, already 256-aligned.
        let staging = renderer.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("studio-shadow-readback"),
            size: u64::from(S * S * BPP),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(S * BPP),
                    rows_per_image: Some(S),
                },
            },
            wgpu::Extent3d { width: S, height: S, depth_or_array_layers: 1 },
        );
        renderer.queue.submit(std::iter::once(enc.finish()));

        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = renderer
            .device
            .poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        rx.recv().expect("readback channel").expect("readback");
        let pixels = slice.get_mapped_range()
        // wgpu 30 made this fallible; the range is the whole buffer the test just mapped.
        .expect("a just-mapped buffer's full range is always valid").to_vec();
        staging.unmap();
        (pixels, matches!(format.remove_srgb_suffix(), wgpu::TextureFormat::Bgra8Unorm))
    });

    let (pixels, bgra) = frame;
    let luma = |x: u32, y: u32| {
        let i = ((y * S + x) * BPP) as usize;
        let (a, b, c) = (pixels[i], pixels[i + 1], pixels[i + 2]);
        let (r, g, bl) = if bgra { (c, b, a) } else { (a, b, c) };
        0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(bl)
    };

    let shadow = luma(75, 215);
    let lit = luma(200, 215);
    assert!(
        lit > 150.0,
        "the control patch is not lit ground ({lit:.1}) — the scene framing moved and this test is          measuring something else"
    );
    assert!(
        shadow < lit * 0.90,
        "no shadow on the ground: the shadowed patch ({shadow:.1}) is not meaningfully darker than          the lit one ({lit:.1}). The editor records four cascades every frame; this is the only          check that any of it reaches a pixel"
    );
}

/// Render one studio frame with the Game panel either visible or hidden, and read both targets.
///
/// `game_view_visible` is a real input now: the studio only renders the separate game-camera pass
/// for a panel that is actually on screen, and this is what lets both halves of that be tested —
/// the picture when it is shown, and the absence of the work when it is not.
fn render_scene_and_game_targets(game_view_visible: bool) -> (Vec<u8>, Vec<u8>) {
    render_with_shading(game_view_visible, 0)
}

/// Render one studio frame at a given toolbar shading mode, and read both targets.
fn render_with_shading(game_view_visible: bool, shading_mode: u32) -> (Vec<u8>, Vec<u8>) {
    render_with_editor(game_view_visible, |ed| ed.shading_mode = shading_mode)
}

/// Render one studio frame with the editor state tweaked, and read both targets.
fn render_with_editor(
    game_view_visible: bool,
    tweak: impl FnOnce(&mut gizmo::editor::EditorState),
) -> (Vec<u8>, Vec<u8>) {
    pollster::block_on(async {
        let mut renderer = Renderer::new_headless(W, H, None).await;
        let mut am = AssetManager::new();
        let mut world = World::new();
        let tex = am.create_white_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
        );

        // Both cameras must be able to see the cube.
        //
        // The obvious setup — point them in opposite directions — proves nothing here, and finding
        // out why is worth the comment: the editor culls against the **game** camera even in edit
        // mode (deliberately, so you can watch what the game would drop). A cube behind the game
        // camera is therefore culled out of the batch list altogether and missing from *both*
        // pictures, which makes them identical for a reason that has nothing to do with the defect
        // under test. So: same direction, very different distance.
        let cube = world.spawn();
        world.add_component(cube, Transform::new(Vec3::new(0.0, 0.0, 0.0)));
        world.add_component(cube, AssetManager::create_cube(&renderer.device));
        world.add_component(
            cube,
            Material::new(tex).with_pbr(Vec4::new(0.9, 0.2, 0.2, 1.0), 0.6, 0.0),
        );
        world.add_component(cube, MeshRenderer::new());

        // Editor camera: looking AT the cube from -X.
        let editor_cam = world.spawn();
        world.add_component(editor_cam, Transform::new(Vec3::new(-6.0, 0.0, 0.0)));
        world.add_component(
            editor_cam,
            gizmo::renderer::components::Camera::new(
                std::f32::consts::FRAC_PI_4, 0.1, 1000.0, 0.0, 0.0, true,
            ),
        );

        // Game camera: same direction, four times further back, so the cube it sees is a
        // fraction of the size — a difference no shared render can produce.
        let game_cam = world.spawn();
        world.add_component(game_cam, Transform::new(Vec3::new(-25.0, 0.0, 0.0)));
        world.add_component(
            game_cam,
            gizmo::renderer::components::Camera::new(
                std::f32::consts::FRAC_PI_4, 0.1, 1000.0, 0.0, 0.0, false,
            ),
        );

        world.spawn_bundle(gizmo::prelude::DirectionalLightBundle::default());

        let make_target = |label: &str, device: &wgpu::Device, format: wgpu::TextureFormat| {
            let t = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let v = t.create_view(&wgpu::TextureViewDescriptor::default());
            (t, v)
        };
        let format = renderer.config.format;
        let (scene_tex, scene_view) = make_target("scene-rtt", &renderer.device, format);
        let (game_tex, game_view) = make_target("game-rtt", &renderer.device, format);

        world.insert_resource(gizmo::renderer::components::EditorRenderTarget(
            gizmo::renderer::components::RenderTarget {
                view: std::sync::Arc::new(scene_view),
                width: W,
                height: H,
            },
        ));
        world.insert_resource(gizmo::renderer::components::GameRenderTarget(
            gizmo::renderer::components::RenderTarget {
                view: std::sync::Arc::new(game_view),
                width: W,
                height: H,
            },
        ));

        // The panel's own answer, which the pipeline reads through `EditorState`. Without one in
        // the world the studio takes the hidden path and never renders the game camera at all.
        let mut ed = gizmo::editor::EditorState::default();
        ed.game_view_visible = game_view_visible;
        tweak(&mut ed);
        world.insert_resource(ed);

        let state = StudioState {
            current_fps: 60.0,
            actual_dt: 1.0 / 60.0,
            editor_camera: editor_cam.id(),
            game_camera: game_cam.id(),
            do_raycast: false,
            play: gizmo::systems::PlayLoop::new(),
            asset_watcher: None,
            gc_timer: 0.0,
            autosave_timer: 0.0,
            visible_entity_count: 0,
            draw_call_count: 0,
        };

        // A throwaway swapchain-ish view: with an EditorRenderTarget present the pipeline clears
        // this and draws into the target instead.
        let (_scratch_tex, scratch_view) = make_target("scratch", &renderer.device, format);
        let mut enc = renderer
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        execute_render_pipeline(&mut world, &state, &mut enc, &scratch_view, &mut renderer, 0.0);

        let read = |tex: &wgpu::Texture, enc: &mut wgpu::CommandEncoder, device: &wgpu::Device| {
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: u64::from(W * H * BPP),
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            enc.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buf,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(W * BPP),
                        rows_per_image: Some(H),
                    },
                },
                wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
            );
            buf
        };
        let scene_buf = read(&scene_tex, &mut enc, &renderer.device);
        let game_buf = read(&game_tex, &mut enc, &renderer.device);
        renderer.queue.submit(std::iter::once(enc.finish()));

        let fetch = |buf: &wgpu::Buffer, device: &wgpu::Device| {
            let slice = buf.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |r| {
                let _ = tx.send(r);
            });
            let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
            rx.recv().expect("channel").expect("map");
            let v = slice.get_mapped_range()
        // wgpu 30 made this fallible; the range is the whole buffer the test just mapped.
        .expect("a just-mapped buffer's full range is always valid").to_vec();
            buf.unmap();
            v
        };
        (fetch(&scene_buf, &renderer.device), fetch(&game_buf, &renderer.device))
    })
}

/// How many bytes of the two targets differ. Counted rather than compared with `assert_ne!`,
/// because a failure there prints two 64 KB byte arrays into the log and buries its own message.
fn target_difference(scene_px: &[u8], game_px: &[u8]) -> usize {
    scene_px.iter().zip(game_px.iter()).filter(|(a, b)| a != b).count()
}

/// The Game panel shows the game camera, not a copy of the scene view.
///
/// # The defect this guards
///
/// Studio used to make one scene render per frame and post-process it into both targets, so the
/// Game tab was a byte-identical copy of the Scene tab — editor camera, gizmos, grid and all —
/// precisely while you were editing, which is when a game preview is worth having. Measured before
/// the fix: 0 of 65536 bytes differed.
///
/// # The trap in fixing it, which this test also pins
///
/// The obvious fix — write the game camera's uniforms mid-frame and record a second pass into the
/// same encoder — does not work, and fails in a way that looks like nothing happened at all:
/// `Queue::write_buffer` is ordered against **submissions**, not against commands. Every write
/// made before a submit applies to every pass in it, so the second write reached the first render
/// too and both panels showed the game camera. The fix is a submission boundary: the game view is
/// drawn into its own encoder and submitted before the editor's uniforms are written back.
///
/// # Why the cameras point the same way
///
/// Pointing them in opposite directions proves nothing here: the editor culls against the **game**
/// camera even in edit mode (deliberately — it lets you watch what the game would drop), so a cube
/// behind the game camera is culled out of the batch list and missing from both pictures. Same
/// direction, four times the distance, is a difference no shared render can fake.
#[test]
fn the_game_view_shows_the_game_camera_not_the_editor_camera() {
    let _gpu = gpu_lock();
    if !pollster::block_on(Renderer::headless_adapter_available()) {
        return;
    }
    if pollster::block_on(Renderer::headless_adapter_is_software()) {
        return;
    }
    let (scene_px, game_px) = render_scene_and_game_targets(true);
    let differing = target_difference(&scene_px, &game_px);
    assert!(
        differing > scene_px.len() / 100,
        "the game target is byte-identical to the scene target ({differing} of {} bytes differ): \
         two cameras pointed in opposite directions produced the same picture, because only one \
         render happened",
        scene_px.len()
    );
}

/// The other half: a Game panel nobody is looking at costs nothing.
///
/// In the default layout Scene and Game are tabs of the same leaf, so at most one is ever on
/// screen — and the studio used to render the game camera as a full extra scene pass every frame
/// regardless, which measured as roughly 40% of the frame (2.6-3.0 ms down to 1.65-1.69 ms on this
/// machine). With the panel hidden that pass must not happen, and the game target then holds
/// whatever the cheap shared path put there: the editor's own picture, byte for byte.
///
/// This is the assertion that stops the gate being quietly inverted or dropped. Without it, a gate
/// that never fires would leave the test above green and the saving gone.
#[test]
fn a_hidden_game_panel_does_not_pay_for_a_second_render() {
    let _gpu = gpu_lock();
    if !pollster::block_on(Renderer::headless_adapter_available()) {
        return;
    }
    if pollster::block_on(Renderer::headless_adapter_is_software()) {
        return;
    }
    let (scene_px, game_px) = render_scene_and_game_targets(false);
    let differing = target_difference(&scene_px, &game_px);
    assert_eq!(
        differing, 0,
        "the game target differs from the scene target in {differing} bytes with the panel \
         hidden — the separate game-camera pass ran for a panel nobody is looking at"
    );
}

/// The "show grid" preference actually hides the grid.
///
/// It was a checkbox that wrote a value to disk and was read by nobody: the grid draw was gated on
/// play mode alone. Nothing pinned the grid at all, in either direction — `cargo test` could not
/// tell a viewport with a grid from one without.
///
/// Two renders of the same world, differing only in that one flag, compared over the whole frame.
/// A count rather than an eyeball: the grid is thin lines over a dark clear, so a threshold picked
/// from a single sample point would be luck.
#[test]
fn the_show_grid_preference_hides_the_grid() {
    let _gpu = gpu_lock();
    if !pollster::block_on(Renderer::headless_adapter_available()) {
        eprintln!("skipping: no GPU adapter");
        return;
    }
    if pollster::block_on(Renderer::headless_adapter_is_software()) {
        eprintln!("skipping: software adapter");
        return;
    }

    let with_grid = render_grid_scene(true).expect("first render");
    let without = render_grid_scene(false).expect("second render");

    let differing = with_grid
        .iter()
        .zip(without.iter())
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        differing > 200,
        "turning the grid off changed only {differing} bytes — the preference is not reaching the \
         grid draw"
    );

    // And the direction: the grid ADDS light lines over a dark clear, so the frame with it is the
    // brighter one. Without this the test would also pass if the flag inverted.
    let sum = |px: &[u8]| px.chunks_exact(4).map(|c| u64::from(c[0]) + u64::from(c[1]) + u64::from(c[2])).sum::<u64>();
    assert!(
        sum(&with_grid) > sum(&without),
        "the frame WITH the grid is not brighter than the one without — the flag is inverted"
    );
}

/// A scene that is nothing but the editor grid, rendered with `show_grid` set either way.
fn render_grid_scene(show_grid: bool) -> Option<Vec<u8>> {
    pollster::block_on(async {
        let mut renderer = Renderer::new_headless(W, H, None).await;
        let mut am = AssetManager::new();
        let mut world = World::new();
        let tex = am.create_white_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
        );

        // The grid, exactly as `gizmo_studio::setup` builds it.
        let grid = world.spawn();
        world.add_component(grid, Transform::new(Vec3::ZERO));
        world.add_component(
            grid,
            AssetManager::create_editor_grid_mesh(&renderer.device, 500.0),
        );
        let mut grid_mat = Material::new(tex);
        grid_mat.albedo = Vec4::ONE;
        grid_mat.material_type = gizmo::renderer::components::MaterialType::Grid;
        world.add_component(grid, grid_mat);
        world.add_component(grid, MeshRenderer::new());

        // Looking down at the grid from above, so it fills the frame.
        let cam = world.spawn();
        world.add_component(cam, Transform::new(Vec3::new(0.0, 6.0, 6.0)));
        world.add_component(
            cam,
            gizmo::renderer::components::Camera::new(
                std::f32::consts::FRAC_PI_4,
                0.1,
                1000.0,
                -std::f32::consts::FRAC_PI_2,
                -0.7,
                true,
            ),
        );
        world.spawn_bundle(gizmo::prelude::DirectionalLightBundle::default());

        let mut ed = gizmo::editor::EditorState::default();
        ed.prefs.show_grid = show_grid;
        world.insert_resource(ed);

        let state = StudioState {
            current_fps: 60.0,
            actual_dt: 1.0 / 60.0,
            editor_camera: cam.id(),
            game_camera: 4242,
            do_raycast: false,
            play: gizmo::systems::PlayLoop::new(),
            asset_watcher: None,
            gc_timer: 0.0,
            autosave_timer: 0.0,
            visible_entity_count: 0,
            draw_call_count: 0,
        };

        let format = renderer.config.format;
        let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("grid-target"),
            size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut enc = renderer
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        execute_render_pipeline(&mut world, &state, &mut enc, &view, &mut renderer, 0.0);

        let staging = renderer.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("grid-readback"),
            size: u64::from(W * H * BPP),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(W * BPP),
                    rows_per_image: Some(H),
                },
            },
            wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        );
        renderer.queue.submit(std::iter::once(enc.finish()));

        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = renderer
            .device
            .poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        rx.recv().expect("readback channel").expect("readback");
        let pixels = slice.get_mapped_range()
        // wgpu 30 made this fallible; the range is the whole buffer the test just mapped.
        .expect("a just-mapped buffer's full range is always valid").to_vec();
        staging.unmap();
        Some(pixels)
    })
}

/// `ShadowCasting::Off` removes the shadow and keeps the object; `Only` does the reverse.
///
/// The engine decided shadow casting from the *material* — unlit, skybox and grid were excluded and
/// everything else cast — so two objects sharing a material could not differ. This is the per-object
/// answer, and the three states are only meaningful if each one changes the picture in its own way,
/// which is what the three renders below compare.
#[test]
fn per_object_shadow_casting_controls_the_shadow_and_the_object() {
    use gizmo::renderer::components::ShadowCasting;

    let _gpu = gpu_lock();
    if !pollster::block_on(Renderer::headless_adapter_available()) {
        eprintln!("skipping: no GPU adapter");
        return;
    }
    if pollster::block_on(Renderer::headless_adapter_is_software()) {
        eprintln!("skipping: software adapter");
        return;
    }

    let on = shadow_scene(ShadowCasting::On).expect("On");
    let off = shadow_scene(ShadowCasting::Off).expect("Off");
    let only = shadow_scene(ShadowCasting::Only).expect("Only");

    // Sample points established by the existing shadow test: (75,215) sits in the cast shadow,
    // (200,215) on lit ground, and the cube covers the centre.
    let at = |px: &[u8], x: u32, y: u32| {
        let i = ((y * 256 + x) * BPP) as usize;
        let (a, b, c) = (px[i], px[i + 1], px[i + 2]);
        let (r, g, bl) = (c, b, a); // this surface is BGRA; the exact channel order is irrelevant
                                    // to a luma comparison, but keep it consistent
        0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(bl)
    };

    let shadow_on = at(&on, 75, 215);
    let lit_on = at(&on, 200, 215);
    assert!(shadow_on < lit_on * 0.90, "the control render has no shadow to remove");

    // Off: that patch is now lit like the rest of the floor.
    let shadow_off = at(&off, 75, 215);
    assert!(
        shadow_off > shadow_on * 1.10,
        "ShadowCasting::Off did not remove the shadow ({shadow_on:.1} → {shadow_off:.1})"
    );

    // Only: the shadow is back, and the cube itself is gone from the picture.
    let shadow_only = at(&only, 75, 215);
    assert!(
        shadow_only < lit_on * 0.90,
        "ShadowCasting::Only lost the shadow ({shadow_only:.1})"
    );
    // Counted, not sampled: a single point is a guess about where the cube landed, and the first
    // version of this assertion guessed wrong — it picked a spot that was floor in both renders and
    // reported "Only still drew the cube" from two identical background pixels.
    let cube_pixels_changed = on
        .chunks_exact(BPP as usize)
        .zip(only.chunks_exact(BPP as usize))
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        cube_pixels_changed > 200,
        "ShadowCasting::Only changed only {cube_pixels_changed} pixels — the cube is still being \
         drawn into the camera's picture"
    );
}

/// The shadow scenario from `the_editor_casts_a_shadow_onto_the_ground`, with the cube's casting
/// mode as a parameter. Returns the frame as raw bytes.
fn shadow_scene(mode: gizmo::renderer::components::ShadowCasting) -> Option<Vec<u8>> {
    const S: u32 = 256;
    Some(pollster::block_on(async {
        let mut renderer = Renderer::new_headless(S, S, None).await;
        let mut am = AssetManager::new();
        let mut world = World::new();
        let tex = am.create_white_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
        );

        let ground = world.spawn();
        world.add_component(ground, Transform::new(Vec3::new(0.0, -2.0, 0.0)));
        world.add_component(ground, AssetManager::create_plane(&renderer.device, 40.0));
        world.add_component(
            ground,
            Material::new(tex.clone()).with_pbr(Vec4::new(0.8, 0.8, 0.8, 1.0), 0.9, 0.0),
        );
        world.add_component(ground, MeshRenderer::new());

        let cube = world.spawn();
        world.add_component(cube, Transform::new(Vec3::ZERO));
        world.add_component(cube, AssetManager::create_cube(&renderer.device));
        world.add_component(
            cube,
            Material::new(tex).with_pbr(Vec4::new(0.9, 0.25, 0.25, 1.0), 0.6, 0.0),
        );
        world.add_component(cube, MeshRenderer::new().with_shadows(mode));

        let cam = world.spawn();
        world.add_component(cam, Transform::new(Vec3::new(0.0, 10.0, 10.0)));
        world.add_component(
            cam,
            gizmo::renderer::components::Camera::new(
                std::f32::consts::FRAC_PI_4,
                0.1,
                1000.0,
                -std::f32::consts::FRAC_PI_2,
                -0.6,
                true,
            ),
        );
        world.spawn_bundle(gizmo::prelude::DirectionalLightBundle {
            rotation: gizmo::math::Quat::from_rotation_z(-0.9)
                * gizmo::math::Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
            ..Default::default()
        });

        let state = StudioState {
            current_fps: 60.0,
            actual_dt: 1.0 / 60.0,
            editor_camera: cam.id(),
            game_camera: 4242,
            do_raycast: false,
            play: gizmo::systems::PlayLoop::new(),
            asset_watcher: None,
            gc_timer: 0.0,
            autosave_timer: 0.0,
            visible_entity_count: 0,
            draw_call_count: 0,
        };

        let format = renderer.config.format;
        let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow-mode-target"),
            size: wgpu::Extent3d { width: S, height: S, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut enc = renderer
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        execute_render_pipeline(&mut world, &state, &mut enc, &view, &mut renderer, 0.0);

        let staging = renderer.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(S * S * BPP),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(S * BPP),
                    rows_per_image: Some(S),
                },
            },
            wgpu::Extent3d { width: S, height: S, depth_or_array_layers: 1 },
        );
        renderer.queue.submit(std::iter::once(enc.finish()));

        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = renderer
            .device
            .poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        rx.recv().expect("channel").expect("map");
        let px = slice.get_mapped_range()
        // wgpu 30 made this fallible; the range is the whole buffer the test just mapped.
        .expect("a just-mapped buffer's full range is always valid").to_vec();
        staging.unmap();
        px
    }))
}


/// Mean RGB over a square of the frame.
fn mean_rgb(px: &[u8], x0: usize, y0: usize, size: usize) -> (f32, f32, f32) {
    let (mut r, mut g, mut b) = (0u64, 0u64, 0u64);
    for y in y0..y0 + size {
        for x in x0..x0 + size {
            let i = (y * W as usize + x) * 4;
            r += px[i] as u64;
            g += px[i + 1] as u64;
            b += px[i + 2] as u64;
        }
    }
    let n = (size * size) as f32;
    (r as f32 / n, g as f32 / n, b as f32 / n)
}

/// The toolbar's four shading chips each draw a different picture.
///
/// # The defect this guards
///
/// Three of the four did nothing at all. The chips write `EditorState::shading_mode`, which the
/// studio forwards into the scene uniform — but the modes were implemented only in
/// `deferred_lighting.wgsl`, and the studio's viewport renders **forward**, through `shader.wgsl`,
/// which never read the field. Measured before the fix, at this resolution: Normals, Albedo and
/// Wire were each **0 of 65536 bytes** different from Lit.
///
/// "Wire" was doubly broken: `deferred_lighting.wgsl` reads mode 3 as Roughness/Metallic, so even
/// on the path where the uniform mattered, the chip labelled Wire meant something else. It is a
/// pipeline, not a shading term — and `renderer.scene.wireframe_pipeline`, built from the same
/// shader with `PolygonMode::Line`, had existed unselected by anything in the workspace.
///
/// All six pairs are compared, not just each against Lit: two chips that both "work" by producing
/// the same picture are still one broken chip.
#[test]
fn every_shading_mode_draws_a_different_picture() {
    let _gpu = gpu_lock();
    if !pollster::block_on(Renderer::headless_adapter_available()) {
        return;
    }
    if pollster::block_on(Renderer::headless_adapter_is_software()) {
        return;
    }

    const NAMES: [&str; 4] = ["Lit", "Normals", "Albedo", "Wire"];
    let shots: Vec<Vec<u8>> = (0u32..4).map(|m| render_with_shading(false, m).0).collect();
    let total = shots[0].len();

    for a in 0..4 {
        for b in (a + 1)..4 {
            let differing = target_difference(&shots[a], &shots[b]);
            assert!(
                differing > total / 100,
                "{} and {} render the same picture ({differing} of {total} bytes differ) — one of \
                 those two toolbar chips does nothing",
                NAMES[a],
                NAMES[b],
            );
        }
    }
}

/// Wire draws edges, so the middle of a face is empty.
///
/// The pairwise test above only says the four pictures differ; this says the fourth one is a
/// *wireframe*. The centre of the cube sits on a face, and under `PolygonMode::Line` that face is
/// not filled — so the centre reads as background, while under Lit it reads as the cube.
#[test]
fn the_wire_mode_leaves_the_middle_of_a_face_empty() {
    let _gpu = gpu_lock();
    if !pollster::block_on(Renderer::headless_adapter_available()) {
        return;
    }
    if pollster::block_on(Renderer::headless_adapter_is_software()) {
        return;
    }

    let lit = render_with_shading(false, 0).0;
    let wire = render_with_shading(false, 3).0;

    // The top-left corner is empty in every mode: that is the pass's clear colour.
    let bg = mean_rgb(&lit, 0, 0, 8);
    let dist = |c: (f32, f32, f32)| {
        ((c.0 - bg.0).powi(2) + (c.1 - bg.1).powi(2) + (c.2 - bg.2).powi(2)).sqrt()
    };

    let centre = (W as usize / 2 - 8, H as usize / 2 - 8);
    let lit_centre = dist(mean_rgb(&lit, centre.0, centre.1, 16));
    let wire_centre = dist(mean_rgb(&wire, centre.0, centre.1, 16));

    assert!(
        lit_centre > 20.0,
        "the filled render has nothing at the centre of the frame ({lit_centre:.1} from the \
         background) — the fixture stopped putting a cube there and this test measures nothing"
    );
    assert!(
        wire_centre < lit_centre / 2.0,
        "the centre of the face is as solid in Wire as in Lit (Wire {wire_centre:.1} vs Lit \
         {lit_centre:.1} from the background) — the wireframe pipeline is not being selected"
    );
}

