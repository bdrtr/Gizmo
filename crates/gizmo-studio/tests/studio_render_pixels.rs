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
            physics_accumulator: 0.0,
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
        let pixels = slice.get_mapped_range().to_vec();
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
