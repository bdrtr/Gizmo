use gizmo_core::Entity;
#[cfg(feature = "physics")]
use super::physics::*;
use crate::core::World;
use crate::math::{Mat4, Vec3};
use crate::renderer::{
    components::{Camera, Material, Mesh, MeshRenderer},
    Renderer,
};
use bytemuck;
use wgpu;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
/// Whether the frame is drawn as wireframe.
#[allow(clippy::derivable_impls)]
pub struct WireframeConfig {
    /// Draw everything as wireframe, whatever the individual materials say.
    pub global: bool,
    /// One colour for every wireframe line (linear RGB, 0..1). `None` keeps each surface's lit
    /// colour — which is nearly invisible on a light material (`wireframe` measured the edge
    /// energy at ×1.20). Native only: the web has no line mode.
    pub color: Option<[f32; 3]>,
}


/// Draw the fluid's screen-space surface onto the lit HDR image — the particles the frame's step
/// simulated ([`GpuFluidSystem::active_particles`](crate::renderer::gpu_fluid::GpuFluidSystem::active_particles),
/// written by [`draw_ahead_of_views`]), nothing when it simulated none.
///
/// It runs after the geometry pass has written depth (it reads it) and before post-processing reads
/// the HDR texture. It is a function here rather than a line in each pipeline
/// because the editor drew no fluid at all until 2026-08-19, and a second implementation is how that
/// comes back.
#[cfg(feature = "render")]
pub fn record_fluid_surface(encoder: &mut wgpu::CommandEncoder, renderer: &crate::renderer::Renderer) {
    let Some(fluid) = &renderer.gpu_fluid else {
        return;
    };
    fluid.render_ssfr(
        encoder,
        &renderer.post.hdr_texture,
        &renderer.post.hdr_texture_view,
        &renderer.depth_texture_view,
        renderer.scene.view_bind_group(),
        fluid.active_particles,
    );
}

/// Draw the GPU particles the frame's step simulated
/// ([`GpuParticleSystem::active_particles`](crate::renderer::gpu_particles::GpuParticleSystem::active_particles),
/// written by [`draw_ahead_of_views`]) onto the lit HDR image, in a pass of their own — nothing when
/// it simulated none, as it does while no particle is alive.
///
/// # Why a pass of their own
///
/// The particle fragment shader SAMPLES the scene depth, so particles fade out against geometry
/// instead of cutting into it. A texture cannot be a depth attachment and a sampled texture in the
/// same pass, so this pass has no depth attachment and binds the depth texture instead; occlusion is
/// done in the shader.
///
/// # Why one function for both pictures
///
/// The editor had its own copy. On 2026-07-12 the particle pipeline gained its depth binding and lost
/// its depth-stencil state; the game path was updated that day and the copy was not, so the studio
/// set the pipeline inside its main pass — a wgpu validation error — and panicked on its first frame
/// for a month. Later the copy drew the whole ring every frame, alive or not, where the game draws
/// nothing while nothing is alive.
#[cfg(feature = "render")]
pub fn record_gpu_particles(encoder: &mut wgpu::CommandEncoder, renderer: &crate::renderer::Renderer) {
    let Some(particles) = &renderer.gpu_particles else {
        return;
    };
    if particles.active_particles == 0 {
        return;
    }
    let depth_bg = particles.create_depth_bind_group(&renderer.device, &renderer.depth_texture_view, renderer.scene.view_cluster_buffers(), &renderer.scene.cookies);
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Particle Soft Pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &renderer.post.hdr_texture_view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    particles.render_pass(&mut pass, renderer.scene.view_bind_group(), &depth_bg, particles.active_particles);
}

/// Guarantee every renderable mesh has a current `GlobalTransform` before the
/// draw query runs.
///
/// The draw query below requires `(&Mesh, &GlobalTransform, &Material)` and reads
/// the world matrix from `GlobalTransform`, but physics/gameplay write only the
/// local `Transform`. Without this step a plain `spawn((Transform, Mesh, Material))`
/// renders nothing (the "empty screen" footgun) and callers had to hand-run the
/// transform systems each frame. Here we (1) backfill a `GlobalTransform` onto any
/// mesh that lacks one, then (2) refresh local matrices and propagate them to
/// `GlobalTransform` — the "update transforms right before the pass" TODO.
///
/// **Public because both render paths need it and only one had it.** The editor's forward pipeline
/// ran `TransformSyncSystem` + `TransformPropagateSystem` — which *update* a `GlobalTransform` but
/// never *add* one, which is exactly why step (1) exists here as a separate pass. So a
/// `spawn((Transform, Mesh, Material))` rendered in the game and silently drew nothing in the
/// editor: the same footgun this function was written to close, still loaded on the other path.
/// Measured, not inferred — `gizmo-studio/tests/studio_render_pixels.rs` renders that exact entity
/// and reads the pixels back.
///
/// The visible cost of the gap was `setup.rs` adding `GlobalTransform::default()` by hand to nine
/// entities in a row: a tax the author paid per spawn, for a component the renderer can supply.
pub fn ensure_global_transforms(world: &mut World) {
    use crate::core::query::Without;
    use crate::core::system::System;
    use gizmo_physics_core::components::{GlobalTransform, Transform};

    // Collect first: `add_component` is a structural change and can't run while a
    // query borrow is live.
    //
    // Every entity with a `Transform`, not only those with a `Mesh`: a `GlobalTransform` is derived
    // and a scene does not save one, so a world label, a sprite, a light or a probe read from a file
    // had a place and no world position — a world `Text` loaded that way was skipped as unplaced.
    let mut missing = Vec::new();
    {
        let q = world.query_filtered::<(Entity, &Transform), Without<GlobalTransform>>();
        for (entity, ..) in q.iter() {
            missing.push(entity);
        }
    }
    for entity in missing {
        world.add_component(entity, GlobalTransform::default());
    }

    let mut sync = crate::systems::transform::TransformSyncSystem;
    let mut propagate = crate::systems::transform::TransformPropagateSystem;
    sync.run(world, 0.0);
    propagate.run(world, 0.0);
}

/// ONE-LINE scene render setup for a manual App (`set_setup`/`set_update`/`set_ui`).
///
/// ROOT-FOOTGUN SOLUTION: a manual App DOES NOT DRAW the 3D scene if `set_render` is not
/// given (the egui HUD is visible but the scene stays BLACK — silently). `with_simple_scene`
/// does this itself; for a manual App this extension provides the same in one line (by
/// turning the heavy/optional passes — SSR/SSGI/volumetric/TAA + GPU fluid/physics — off;
/// GPU particles stay on).
///
/// ```no_run
/// use gizmo::prelude::*;
/// use gizmo::systems::AppSceneRenderExt;
///
/// App::<()>::new("Demo", 1280, 720)
///     .add_plugin(TransformPlugin)
///     .set_setup(|_world, _renderer| {})
///     .set_update(|_world, _state, _dt, _input| {})
///     .with_scene_render()   // <- bu olmadan ekran siyah
///     .run()
///     .expect("the application failed to run");
/// ```
pub trait AppSceneRenderExt {
    /// Sets the render stage up so that the scene is drawn as [`default_render_pass`] draws it,
    /// from a copy of the game world ([`RenderWorld`]).
    fn with_scene_render(self) -> Self;
}

impl<State: 'static> AppSceneRenderExt for gizmo_app::App<State> {
    fn with_scene_render(self) -> Self {
        self.set_render_stage(RenderWorld::new().with_settings(|_, renderer| {
            // No `gpu_fluid = None`: none is built until a scene has one, and taking it away here
            // took a `FluidSimulation`'s water away with it (`Renderer::fluid_or_build`).
            renderer.gpu_physics = None;
            renderer.ssr = None;
            renderer.ssgi = None;
            renderer.volumetric = None;
            renderer.taa = None;
        }))
    }
}

/// Offsets a projection matrix by a sub-pixel **NDC** jitter, in the column that projection's own
/// `w` row makes correct.
///
/// **Which column it is depends on the projection, and getting it wrong is invisible until it
/// isn't.** A perspective matrix divides by `w = −vz`, so an offset written into the z column
/// (`M[0][2]`, `M[1][2]`) survives the divide as a constant NDC shift — the standard trick, and
/// what this code did unconditionally. An **orthographic** matrix has `w = 1`: there is no
/// divide, so the very same edit turns a sub-pixel offset into one **proportional to view
/// depth**. Measured 2026-08-22 on a static orthographic scene (`demo/src/bin/orthographic.rs`,
/// camera 8.7 units out): ±0.5 px of intended jitter became ±4 px of actual shift, TAA never
/// converged, and consecutive frames of a scene whose every transform was bit-identical differed
/// in **2.1 %** of their pixels — a visible shimmer. With the jitter in the translation column
/// the same two frames are **bit-identical**.
///
/// The discriminator is the matrix itself rather than a flag passed alongside it: `M[3][2]`
/// (`z_axis.w`) is how much `clip.w` depends on view depth, which is exactly the question being
/// asked, and it stays right for any projection assembled elsewhere. (Naming the camera's
/// projection *enum* here would also be a lie to `render_parity`'s inventory, which reads this
/// directory as "what the game path knows about" — and this function knows a matrix, not a mode.)
fn jitter_projection(mut proj: crate::math::Mat4, jx: f32, jy: f32) -> crate::math::Mat4 {
    if proj.z_axis.w.abs() > 1e-6 {
        // Perspective: clip.w = −vz, so new_clip.x = clip.x − jx·vz lands as NDC.x + jx.
        proj.z_axis.x -= jx;
        proj.z_axis.y -= jy;
    } else {
        // Orthographic: clip.w = 1, so the offset goes straight into the translation column.
        proj.w_axis.x += jx;
        proj.w_axis.y += jy;
    }
    proj
}

/// What this view costs, as the editor counts it (`gizmo-studio`'s render pipeline): one draw call
/// per batch with camera-visible instances, triangles counted with their instances. Kept in
/// [`Renderer::render_stats`] — the draw only reads the world — and published to the world as the
/// [`RenderStats`](crate::renderer::components::RenderStats) resource by [`publish_render_stats`],
/// so a game can read its own draw-call count — before, only the editor filled it. Under split
/// screen it holds the last view drawn. GPU memory is sampled about once a second, as the editor
/// does, since the allocator report walks every live allocation.
fn record_render_stats(renderer: &mut Renderer, items: &[batching::DrawItem], elapsed: f32) {
    use crate::renderer::components::RenderStats;
    let mut stats = RenderStats::default();
    for item in items.iter().filter(|i| i.visible_in_camera && i.camera_count > 0) {
        stats.draw_calls += 1;
        stats.instances += item.camera_count;
        // Each instance's own range: a batch's instances may be different meshes sharing buffers.
        let range = item.first_instance as usize..(item.first_instance + item.camera_count) as usize;
        stats.triangles += item.ranges.get(range).map_or(0, |r| r.iter().map(|r| r.count / 3).sum());
    }
    let previous = renderer
        .render_stats
        .map(|r| (r.gpu_allocated_bytes, r.gpu_sampled_at))
        .unwrap_or((None, f32::NEG_INFINITY));
    if elapsed - previous.1 >= 1.0 {
        stats.gpu_allocated_bytes = renderer.device.generate_allocator_report().map(|r| r.total_allocated_bytes);
        stats.gpu_sampled_at = elapsed;
    } else {
        stats.gpu_allocated_bytes = previous.0;
        stats.gpu_sampled_at = previous.1;
    }
    renderer.render_stats = Some(stats);
}

/// Publishes what the last view drawn cost ([`Renderer::render_stats`]) as `world`'s
/// [`RenderStats`](crate::renderer::components::RenderStats) resource. The draw cannot do it — it
/// takes the world it draws read-only — so the caller that owns the world does, after the draw:
/// [`default_render_pass`] and [`render_viewports`] call it. Nothing is published before a view
/// has been drawn.
pub fn publish_render_stats(world: &mut World, renderer: &Renderer) {
    if let Some(stats) = renderer.render_stats {
        world.insert_resource(stats);
    }
}

/// A batteries-included render path, serving only to light the models and put them on screen
/// quickly.
/// It is used to avoid writing hundreds of lines of code in freshly opened, empty projects
/// like `tut`.
///
/// **Two halves and a hand-over**: [`prepare_frame`] does everything a frame does once that
/// writes what the game can see (clocks, transforms, materials, animation, particle emitters, the
/// debug-line frame); [`draw_frame`] draws, reading the world and writing only the renderer; and
/// [`publish_render_stats`] puts what the draw cost back into the world. A host that draws from a
/// copy of its world (`gizmo_core::extract::Extractor`) calls the three itself, preparing the
/// original and drawing the copy.
#[tracing::instrument(skip_all, name = "render_system")]
pub fn default_render_pass(
    world: &mut World,
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    renderer: &mut Renderer,
) {
    prepare_frame(world, renderer);
    // Bodies drawn between their physics steps, for this draw alone (`systems::interpolation`).
    #[cfg(feature = "physics")]
    let replaced = {
        let overrides = crate::systems::interpolation::render_overrides(world);
        crate::systems::interpolation::apply_overrides(world, &overrides)
    };
    draw_frame(world, encoder, view, renderer);
    #[cfg(feature = "physics")]
    crate::systems::interpolation::restore(world, &replaced);
    publish_render_stats(world, renderer);
}

/// Draws one frame of `world` into `view`, reading the world and writing only the renderer — the
/// half of [`default_render_pass`] after [`prepare_frame`].
///
/// **Split screen is this same call.** A scene with any camera that has a `viewport` is drawn by
/// [`draw_viewports`], one view per such camera; every host that already calls this —
/// `SimpleApp`, the editor's game view, a hand-written render loop — gets it without a second
/// entry point to know about.
///
/// A `CameraShake` is in the picture and nowhere else: the draw shakes its view of the camera
/// ([`shake_view`]) and never moves the camera the game owns.
pub fn draw_frame(
    world: &World,
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    renderer: &mut Renderer,
) {
    let mut laps = Laps::start();
    // The GPU's times of frames it has finished, into the draw's laps (`gizmo_renderer::gpu_timer`).
    {
        let Renderer { gpu_timer, device, draw_laps, .. } = &mut *renderer;
        if let Some(timer) = gpu_timer.as_mut() {
            timer.collect(device, draw_laps);
        }
    }
    if any_camera_has_a_viewport(world) {
        draw_viewports(world, encoder, view, renderer);
        laps.draw_lap(renderer, "render.viewports");
        end_gpu_frame(renderer, encoder);
        return;
    }
    let camera = crate::renderer::components::active_camera(world);
    draw_ahead_of_views(world, renderer, SimulationDetail::NearestOf(&[camera_world_position(world, camera)]));
    laps.draw_lap(renderer, "render.ahead");
    // The texture cameras were drawn first, each with its mirrors; the frame's own mirrors last, so
    // they are what the frame's view samples.
    capture_planar_reflections(world, renderer, camera, renderer_aspect(renderer));
    laps.draw_lap(renderer, "render.mirrors");
    draw_camera_view(world, encoder, view, renderer, camera, true, None, true, None);
    laps.draw_lap(renderer, "render.camera");
    end_gpu_frame(renderer, encoder);
}

/// Resolves the frame's GPU timestamps in `encoder`, the frame's last — submitted after every view's.
fn end_gpu_frame(renderer: &mut Renderer, encoder: &mut wgpu::CommandEncoder) {
    if let Some(timer) = renderer.gpu_timer.as_mut() {
        timer.end_frame(encoder);
    }
}

/// How much of each GPU simulation a frame runs: the particles' and the fluid's share of what they
/// hold. Every view of the frame draws what the step ran — see [`draw_ahead_of_views`].
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum SimulationDetail<'a> {
    /// The game's: less the farther the nearest camera about to draw stands — the particles from
    /// the origin, the fluid from its tank. These are the frame's own views' cameras;
    /// [`draw_ahead_of_views`] adds every camera that draws into a texture.
    NearestOf(&'a [Vec3]),
    /// All of it: an editor shows what is there, as it shows every mesh at full detail (the
    /// `lod_vbufs` entry in the render-parity exceptions).
    Full,
}

/// **What a frame does before its own view or views**, in this order and the same in every host —
/// [`draw_frame`], split screen, the editor's viewport:
///
/// 1. the irradiance volume, when it asks for a bake ([`capture_irradiance_volume`]), then the
///    reflection probes that ask for a capture ([`capture_reflection_probes`]) — so a probe's
///    capture shows the volume's light, as the frame will;
/// 2. the frame's start: the mirrors' captures counted from zero, and the temporal state of every
///    view whose camera is gone dropped ([`Renderer::retain_view_histories`]);
/// 3. the GPU simulations, stepped once for every view and in a submission of their own — the
///    volumetric smoke, the GPU rigid bodies (and the request for their readback), the fluid
///    (as many particles as its settings hold — none without a `FluidSimulation`) and the particles, each to the share `detail` asks
///    for. The share is written to the system, and it is what every view of the frame draws;
/// 4. every camera that draws into a texture ([`CameraTarget`](crate::renderer::components::CameraTarget)),
///    each with its mirrors and in a submission of its own.
///
/// The editor did its own version of 3 and 4 until 2026-11-10, and it drifted the way copies do: it
/// stepped its particles and GPU bodies after its monitors, which drew them a step behind; it drew
/// its monitors at full detail with the fluid's switch never looked at, so a scene that had not
/// turned the fluid on showed the fluid's unstepped particles on every monitor; it dispatched and
/// drew the whole particle ring with nothing alive; and it never asked for the GPU bodies' readback,
/// which their entities follow.
pub fn draw_ahead_of_views(world: &World, renderer: &mut Renderer, detail: SimulationDetail) {
    // What the last frame drew is what this one's views see into (`ShadowSettings::fit_to_scene`) —
    // the captures below included.
    renderer.scene_extent.begin_frame();
    capture_irradiance_volume(world, renderer);
    capture_reflection_probes(world, renderer);
    renderer.planar.begin_frame();
    renderer.retain_view_histories(|camera| world.get::<Camera>(camera).is_some());
    renderer.retain_view_targets();
    renderer.skeletons_seen.begin_frame();
    let mut cameras = Vec::new();
    let detail = match detail {
        SimulationDetail::NearestOf(views) => {
            cameras.extend_from_slice(views);
            cameras.extend(target_camera_positions(world));
            SimulationDetail::NearestOf(&cameras)
        }
        SimulationDetail::Full => SimulationDetail::Full,
    };
    step_gpu_simulations_ahead(world, renderer, detail);
    draw_camera_targets(world, renderer);
}

/// The renderer's own shape — the frame's, when its view is the whole target.
fn renderer_aspect(renderer: &Renderer) -> f32 {
    renderer.size.width as f32 / renderer.size.height.max(1) as f32
}

/// Whether any camera in `world` asks for a [`Viewport`](crate::renderer::components::Viewport).
fn any_camera_has_a_viewport(world: &World) -> bool {
    use crate::core::query::Without;
    use crate::renderer::components::CameraTarget;
    world.query_filtered::<(Entity, &Camera), Without<CameraTarget>>().iter()
        .any(|(_, c)| c.viewport.is_some())
}

/// **The cameras that draw into a texture of the game's own** ([`CameraTarget`](crate::renderer::components::CameraTarget)),
/// each into its texture, in [`order`](Camera::order) — before the frame's own view, which this
/// does not touch.
///
/// Each is a whole view through the game path's pipeline, as a split-screen view is: at the
/// texture's aspect, rendered at the renderer's size and resampled into the texture by the last
/// post pass, with no HUD, and with a temporal history of its own — its camera's
/// ([`Renderer::with_view_history`]). And each is recorded into **an encoder of its own and
/// submitted at once**: the draw's uploads (`queue.write_buffer`) order against submission, so two
/// views recorded into one encoder would both draw whatever the last one wrote — the frame's own
/// view would come out as the last texture camera's.
///
/// The frame's GPU simulations were stepped before this, once for every camera and in a submission
/// of their own ([`draw_ahead_of_views`]), so a texture camera draws the step the frame's own view
/// draws.
fn draw_camera_targets(world: &World, renderer: &mut Renderer) {
    use crate::renderer::components::CameraTarget;
    let mut targets: Vec<(i32, Entity, CameraTarget)> = world
        .query::<(Entity, &Camera, &CameraTarget)>()
        .iter()
        .map(|(entity, camera, target)| (camera.order, entity, target.clone()))
        .collect();
    targets.sort_by_key(|&(order, id, _)| (order, id));
    for (_, camera, target) in targets {
        draw_camera_into(world, renderer, camera, target.view(), target.size(), ViewOptions::default());
    }
}

/// **One camera's whole picture, through the game path's pipeline, into a texture of the host's**:
/// `view`, of `size` — what a [`CameraTarget`](crate::renderer::components::CameraTarget) camera
/// gets every frame, for a host that owns the texture itself. The editor's Game panel is this
/// call; it drew the game camera through the editor's own forward path until 2026-11-22, which
/// is not the picture the game draws (no deferred lighting, no screen-space effects, no TAA).
///
/// At the texture's size ([`Renderer::with_view_targets`]) and aspect, with its mirrors captured
/// for it just before it, the camera's own temporal history ([`Renderer::with_view_history`]), and
/// what `options` adds (the HUD, an editor's furniture). Recorded into an encoder of its own and
/// **submitted at once**: the draw's
/// uploads (`queue.write_buffer`) order against submission, so a view recorded into a caller's
/// encoder would draw whatever the caller wrote last. Call it before recording anything of the
/// caller's own that writes the same buffers.
///
/// Reads the world: a host runs [`prepare_frame`] and [`draw_ahead_of_views`] first, once for every
/// view of its frame.
pub fn draw_camera_into(
    world: &World,
    renderer: &mut Renderer,
    camera: Entity,
    view: &wgpu::TextureView,
    (width, height): (u32, u32),
    options: ViewOptions,
) {
    // Its mirrors, captured for it, just before it.
    capture_planar_reflections(world, renderer, Some(camera), width as f32 / height.max(1) as f32);
    let mut encoder = renderer.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("camera_target"),
    });
    renderer.with_view_targets((width, height), |renderer| {
        renderer.with_view_history(Some(camera), |renderer| {
            // For this view's own draw only: its mirrors, captured above, are the game's.
            renderer.draws_editor_furniture = options.furniture;
            renderer.hides_grid = options.no_grid;
            let game_shading = renderer.shading_mode;
            if let Some(shading) = options.shading {
                renderer.shading_mode = shading.mode();
                renderer.wire_only = shading == ViewShading::Wire;
            }
            draw_camera_view(
                world,
                &mut encoder,
                view,
                renderer,
                Some(camera),
                true,
                Some([0.0, 0.0, width as f32, height as f32]),
                options.hud,
                None,
            );
            renderer.draws_editor_furniture = false;
            renderer.hides_grid = false;
            renderer.shading_mode = game_shading;
            renderer.wire_only = false;
        });
    });
    renderer.queue.submit(std::iter::once(encoder.finish()));
}

/// What [`draw_camera_into`] draws past the camera's own picture.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ViewOptions {
    /// The frame's HUD over the picture, as the game draws it over its own.
    pub hud: bool,
    /// The editor's furniture too (`EditorOnly`: the grid, light icons, gizmo handles): the view is
    /// the editor's Scene panel. Every other view leaves it out.
    pub furniture: bool,
    /// How the view is shaded, when not the way the game shades it (`Renderer::shading_mode`): an
    /// editor's toolbar.
    pub shading: Option<ViewShading>,
    /// Leave out what draws with the `Grid` material (`Renderer::hides_grid`): an editor whose grid
    /// preference is off.
    pub no_grid: bool,
}

/// One of the pictures an editor's toolbar switches its view between.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ViewShading {
    /// Lit, as the game is.
    Lit,
    /// World normals (`Renderer::shading_mode` 1).
    Normals,
    /// The surfaces' albedo, unlit (`Renderer::shading_mode` 2).
    Albedo,
    /// World geometry as lines only (`Renderer::wire_only`), lit. Native only: the forward-only
    /// path (the web, a phone) has no line mode, and its "wireframe" is a fill.
    Wire,
}

impl ViewShading {
    /// The `Renderer::shading_mode` the view is drawn with: the lighting's own picture, or one of
    /// its debug ones. `Wire` is lit lines.
    fn mode(self) -> u32 {
        match self {
            Self::Lit | Self::Wire => 0,
            Self::Normals => 1,
            Self::Albedo => 2,
        }
    }
}

impl ViewOptions {
    /// With the HUD: the game's own picture (the editor's Game panel).
    #[must_use]
    pub fn with_hud(mut self) -> Self {
        self.hud = true;
        self
    }

    /// With the editor's furniture: the editor's Scene panel.
    #[must_use]
    pub fn with_furniture(mut self) -> Self {
        self.furniture = true;
        self
    }

    /// Shaded `shading`'s way rather than the game's.
    #[must_use]
    pub fn with_shading(mut self, shading: ViewShading) -> Self {
        self.shading = Some(shading);
        self
    }

    /// Without the grid: what draws with the `Grid` material is left out.
    #[must_use]
    pub fn without_grid(mut self) -> Self {
        self.no_grid = true;
        self
    }
}

/// Where the cameras that draw into textures stand — for the level of detail the frame's GPU
/// simulations are stepped at, which the nearest camera that draws sets.
fn target_camera_positions(world: &World) -> Vec<Vec3> {
    use crate::renderer::components::CameraTarget;
    let ids: Vec<Entity> =
        world.query::<(Entity, &CameraTarget)>().iter().map(|(entity, _)| entity).collect();
    ids.into_iter().map(|id| camera_world_position(world, Some(id))).collect()
}

/// **Split screen:** every camera with a [`viewport`](crate::renderer::components::Camera::viewport)
/// drawn into its own rectangle of `view`, in [`order`](crate::renderer::components::Camera::order),
/// then this frame's HUD once over the whole target. With no such camera this is
/// [`default_render_pass`] (which also comes here by itself when one exists): [`prepare_frame`],
/// [`draw_viewports`], [`publish_render_stats`].
///
/// What a view gets of its own, and why each one is needed:
/// - **its camera, at its rectangle's aspect** — the scene is rendered at the renderer's full
///   internal size and resampled into the rectangle by the last post pass, so a half-width view
///   is not stretched and is resolved at full width;
/// - **its own command submission.** The draw path uploads camera-dependent data with
///   `queue.write_buffer` — sprite vertices, the culled instance list, the post-process grade —
///   and those writes order against *submission*, not against recording: two views recorded into
///   one encoder would both draw whatever the last view wrote. So each view is recorded into an
///   encoder of its own and submitted before the next view writes. **`encoder` itself receives
///   only the HUD**, so anything recorded into it before this call executes *after* the views;
///   record such work after this call, or submit it first;
/// - **its own TAA and SSGI history** ([`Renderer::with_view_history`]), so neither accumulates
///   another camera's picture into this one's.
///
/// What a frame does **once**, not once per view: transforms, materials, animation, GPU physics,
/// fluid and particle simulation (at the level of detail of the nearest camera). Drawn per view,
/// a split screen would play every animation and emit every particle twice as fast.
pub fn render_viewports(
    world: &mut World,
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    renderer: &mut Renderer,
) {
    prepare_frame(world, renderer);
    draw_viewports(world, encoder, view, renderer);
    publish_render_stats(world, renderer);
}

/// The draw half of [`render_viewports`]: reads `world`, writes only the renderer. A frame with no
/// camera rectangle is drawn whole, through the active camera.
pub fn draw_viewports(
    world: &World,
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    renderer: &mut Renderer,
) {
    let mut views: Vec<(i32, Entity, crate::renderer::components::Viewport)> = world
        .query_filtered::<(Entity, &Camera), crate::core::query::Without<crate::renderer::components::CameraTarget>>()
        .iter()
        .filter_map(|(entity, c)| c.viewport.map(|v| (c.order, entity, v)))
        .collect();
    // The camera the frame is drawn through, when it has no rectangle of its own, is the whole
    // target — the base a picture-in-picture view (a minimap, a mirror) is drawn over. It was
    // skipped: one small viewport anywhere and the main view went black.
    if !views.is_empty() {
        let main = crate::renderer::components::active_camera(world);
        let full = main.and_then(|id| world.query::<&Camera>().get(id).filter(|c| c.viewport.is_none()).map(|c| (c.order, id)));
        if let Some((order, id)) = full {
            views.push((order, id, crate::renderer::components::Viewport::new(0.0, 0.0, 1.0, 1.0)));
        }
    }
    if views.is_empty() {
        let camera = crate::renderer::components::active_camera(world);
        draw_ahead_of_views(world, renderer, SimulationDetail::NearestOf(&[camera_world_position(world, camera)]));
        capture_planar_reflections(world, renderer, camera, renderer_aspect(renderer));
        draw_camera_view(world, encoder, view, renderer, camera, true, None, true, None);
        return;
    }
    views.sort_by_key(|&(order, id, _)| (order, id));
    let size = view.texture().size();

    // The probes, the GPU simulations stepped for every view and the texture cameras, each
    // submitted before the split's own views.
    let positions: Vec<Vec3> = views.iter().map(|&(_, id, _)| camera_world_position(world, Some(id))).collect();
    draw_ahead_of_views(world, renderer, SimulationDetail::NearestOf(&positions));

    let new_encoder = |renderer: &Renderer| {
        renderer.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("split_screen_view"),
        })
    };
    let mut frame = new_encoder(renderer);
    // The views write only their own rectangles; whatever none of them covers is black, not last
    // frame's picture.
    frame.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Split Screen Clear"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });

    for (index, &(_, camera, viewport)) in views.iter().enumerate() {
        let region = viewport.to_pixels(size.width, size.height);
        // Its mirrors, captured for it at its rectangle's shape, just before it: the capture is
        // submitted now and the view after it.
        let simple = world
            .query::<&Camera>()
            .get(camera)
            .is_some_and(|c| c.detail == crate::renderer::components::CameraDetail::Simple);
        if !simple {
            capture_planar_reflections(world, renderer, Some(camera), region[2] / region[3].max(1.0));
        }
        // Drawn at its rectangle's size, into targets of that size (`Renderer::with_view_targets`).
        // The first view is the frame's own and draws with the renderer's temporal state when it
        // fills the frame; every other one — and a first one of a size of its own — with its camera's.
        let shown = rectangle_size(region);
        let own_size = shown != (renderer.size.width, renderer.size.height);
        renderer.with_view_targets(shown, |renderer| {
            renderer.with_view_history((index > 0 || own_size).then_some(camera), |renderer| {
                draw_camera_view(world, &mut frame, view, renderer, Some(camera), true, Some(region), false, None);
            });
        });
        let mut laps = Laps::start();
        let done = std::mem::replace(&mut frame, new_encoder(renderer));
        renderer.queue.submit(std::iter::once(done.finish()));
        laps.draw_lap(renderer, "view.submit");
    }
    tracing::trace!(views = views.len(), "[Render] split screen");
    renderer.record_overlay(encoder, view);
}

/// **Reflection probes:** renders the scene into each probe's slot of the cube array when it asks
/// for it — the first frame it exists, after `ReflectionProbe::request_capture`, or when the set of
/// probes changes — prefilters the mips, and points the lighting pass at the set. With no probe,
/// puts the procedural sky back.
///
/// Each face is a full draw from the probe, 90° and square, stopped at the HDR target and copied
/// into the cube; each in **its own submission**, for the reason `render_viewports` gives (the draw
/// path's `queue.write_buffer` uploads order against submission). For the capture the renderer's
/// TAA, SSGI and motion-blur history are set aside and put back: a probe face is one still frame,
/// not part of the camera's sequence.
///
/// `default_render_pass` calls it every frame (it returns at once when nothing needs capturing).
/// **`gizmo-studio` calls it too**, before its own passes: the capture draws the world through this
/// path, which skips `EditorOnly` furniture, so the editor's probes match the game's.
///
/// Draw-side: it reads the world and writes only the renderer. Which request each slot was captured
/// at is the renderer's record ([`ReflectionProbeGpu::captured`](crate::renderer::reflection::ReflectionProbeGpu::captured)),
/// compared with the probe's [`capture_request`](crate::renderer::components::ReflectionProbe::capture_request) —
/// the probe component is never written back, so a copy of the world draws its probes the same way.
pub fn capture_reflection_probes(world: &World, renderer: &mut Renderer) {
    use crate::renderer::components::ReflectionProbe;
    // The forward-only path reads its probes on group 0 (`SceneState::set_forward_reflections`) and takes
    // an environment map through its own cube (`SceneState::set_environment_map`), so a map takes no slot
    // here, and its dirty flag is left for that cube.
    let forward_only = cfg!(gizmo_forward_only);
    use crate::renderer::reflection::{ReflectionProbeGpu, FACES, MAX_PROBES};
    let mut probes: Vec<(Entity, ReflectionProbe)> =
        world.query::<(Entity, &ReflectionProbe)>().iter().map(|(entity, p)| (entity, *p)).collect();
    probes.sort_by_key(|&(id, _)| id);
    // An environment map takes the last slot.
    let environment = renderer.environment_map.is_some() && !forward_only;
    probes.truncate(MAX_PROBES - usize::from(environment));
    if probes.is_empty() && !environment {
        if !forward_only {
            renderer.environment_map_dirty = false;
        }
        if renderer.reflection_probe.take().is_some() {
            if let Some(def) = renderer.deferred.as_mut() {
                def.irradiance.set_reflection(&renderer.device, &renderer.queue, None);
            }
            if forward_only && renderer.scene.set_forward_reflections(&renderer.device, &renderer.queue, None) {
                crate::renderer::pipeline::rebuild_pipelines(renderer);
            }
        }
        return;
    }
    let owners: Vec<Entity> = probes.iter().map(|&(id, _)| id).collect();
    let size = probes
        .iter()
        .map(|(_, p)| p.resolution.clamp(8, 1024).next_power_of_two())
        .chain(renderer.environment_map.as_ref().map(|m| m.resolution.clamp(8, 1024).next_power_of_two()))
        .max()
        .unwrap_or(128);
    let slots = owners.len() + usize::from(environment);
    // Not an entity's: an environment map is in the owner list as this.
    const ENVIRONMENT_OWNER: u32 = u32::MAX;
    let owner_ids: Vec<u32> =
        owners.iter().map(|e| e.id()).chain(environment.then_some(ENVIRONMENT_OWNER)).collect();
    // A different set of probes, or a different size, is a new array — and every slot in it is
    // captured afresh.
    let rebuild = renderer
        .reflection_probe
        .as_ref()
        .is_none_or(|g| g.size != size || g.owners != owner_ids);
    if rebuild {
        let mut gpu = ReflectionProbeGpu::new(&renderer.device, size, slots as u32);
        gpu.owners = owner_ids;
        renderer.reflection_probe = Some(gpu);
    }
    // A slot is captured when it holds nothing yet, another probe, or an older request of this one.
    let asked = |slot: usize| Some((probes[slot].0, probes[slot].1.capture_request()));
    let pending: Vec<usize> = {
        let held = renderer.reflection_probe.as_ref().map_or(&[][..], |g| g.captured.as_slice());
        (0..probes.len()).filter(|&slot| rebuild || held.get(slot).copied().flatten() != asked(slot)).collect()
    };
    let positions: Vec<Vec3> = owners.iter().map(|&id| camera_world_position(world, Some(id))).collect();

    // The image goes in ahead of any probe, whose capture will show it behind the scene.
    let environment_changed = environment && (rebuild || renderer.environment_map_dirty);
    if environment_changed {
        if let (Some(gpu), Some(map)) = (renderer.reflection_probe.as_ref(), renderer.environment_map.as_ref()) {
            let slot = (slots - 1) as u32;
            gpu.write_environment(&renderer.queue, slot, map, 1.0);
            let mut encoder = renderer.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("environment_map_prefilter"),
            });
            gpu.record_prefilter(&renderer.device, &mut encoder, slot);
            renderer.queue.submit(std::iter::once(encoder.finish()));
        }
    }
    if !forward_only {
        renderer.environment_map_dirty = false;
    }

    let taa = if pending.is_empty() { None } else { renderer.taa.take() };
    let ssgi = if pending.is_empty() { None } else { renderer.ssgi.take() };
    let history = if pending.is_empty() { None } else { renderer.previous_view_proj.take() };
    for &slot in &pending {
        let probe = probes[slot].1;
        if let Some(gpu) = renderer.reflection_probe.as_mut() {
            gpu.captured_at[slot] = positions[slot].to_array();
            gpu.captured[slot] = asked(slot);
        }
        for face in 0..FACES {
            let (forward, up) = crate::renderer::reflection::face_basis(face);
            let mut encoder = renderer.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("reflection_probe_face"),
            });
            let fixed = FixedView {
                position: positions[slot],
                forward,
                up,
                near: probe.near,
                far: probe.far.max(probe.near + 0.1),
                projection: None,
                aspect: 1.0,
                bounce: false,
            };
            // The HDR target stands in for the output view: a capture stops before the post
            // chain, so nothing is ever written through it.
            let hdr = renderer.post.hdr_texture_view.clone();
            draw_camera_view(world, &mut encoder, &hdr, renderer, None, false, None, false, Some(fixed));
            if let Some(gpu) = renderer.reflection_probe.as_ref() {
                gpu.record_face(&renderer.device, &mut encoder, &renderer.post.hdr_texture_view, slot as u32, face);
            }
            renderer.queue.submit(std::iter::once(encoder.finish()));
        }
        if let Some(gpu) = renderer.reflection_probe.as_ref() {
            let mut encoder = renderer.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("reflection_probe_prefilter"),
            });
            gpu.record_prefilter(&renderer.device, &mut encoder, slot as u32);
            renderer.queue.submit(std::iter::once(encoder.finish()));
        }
    }
    if !pending.is_empty() {
        renderer.taa = taa;
        renderer.ssgi = ssgi;
        renderer.previous_view_proj = history;
    }

    // The parameters go up whenever they change, not only on a capture: an `intensity`,
    // `influence` or `blend_distance` edited later takes effect on the next frame. Positions are
    // where each slot was captured, not where the entity is now (see `captured_at`).
    let mut probes_toggled = false;
    if let Some(gpu) = renderer.reflection_probe.as_mut() {
        let sources: Vec<_> = probes
            .iter()
            .zip(&gpu.captured_at)
            .map(|((_, p), at)| {
                crate::renderer::irradiance::ReflectionSource::new(p.intensity, *at)
                    .with_box(p.influence.map(|h| h.to_array()), p.blend_distance)
            })
            .chain(renderer.environment_map.as_ref().map(|m| {
                crate::renderer::irradiance::ReflectionSource::new(m.intensity, [0.0; 3])
            }))
            .collect();
        if environment_changed || !pending.is_empty() || sources != gpu.uploaded {
            gpu.uploaded = sources.clone();
            let mut set = crate::renderer::irradiance::ReflectionSet::new(&gpu.cube_view, gpu.mip_count, sources);
            set.environment = environment.then_some(slots - 1);
            if let Some(def) = renderer.deferred.as_mut() {
                def.irradiance.set_reflection(&renderer.device, &renderer.queue, Some(set));
            } else if forward_only {
                probes_toggled = renderer.scene.set_forward_reflections(&renderer.device, &renderer.queue, Some(set));
            }
        }
    }
    // Probes appearing or going on the forward-only path build its lit pipelines again with `PROBES` on
    // or off, so a scene without them pays nothing.
    if probes_toggled {
        crate::renderer::pipeline::rebuild_pipelines(renderer);
    }
    if pending.is_empty() {
        return;
    }
    tracing::debug!(captured = pending.len(), probes = owners.len(), size, "[Render] reflection probes captured");
}

/// Bakes the scene's [`IrradianceVolume`] when it asks for it: on the first frame it exists, after it
/// changes, and after [`IrradianceVolume::request_capture`]. From each probe the scene is drawn six
/// ways as a reflection probe's faces are — each face **its own submission**, for the reason
/// [`capture_reflection_probes`] gives — with the sky left out (`FixedView::bounce`), and the faces
/// projected into the probe's spherical harmonics on the GPU
/// ([`IrradianceState`](crate::renderer::irradiance::IrradianceState)'s `begin_bake` and the rest).
/// TAA, SSGI and the motion history are set aside for it and put back, as for a probe.
///
/// Which volume and which request the uploaded grid was baked from is the renderer's record
/// (`IrradianceState::baked_from`); the component is never written back. A volume that goes takes
/// its bake with it; a grid uploaded by hand, with no volume, is left alone.
///
/// [`IrradianceVolume`]: crate::renderer::irradiance::IrradianceVolume
/// [`IrradianceVolume::request_capture`]: crate::renderer::irradiance::IrradianceVolume::request_capture
pub fn capture_irradiance_volume(world: &World, renderer: &mut Renderer) {
    use crate::renderer::irradiance::IrradianceVolume;
    use crate::renderer::reflection::FACES;
    use std::hash::{Hash, Hasher};
    let volume = world.query::<&IrradianceVolume>().iter().next().copied();
    let Some(def) = renderer.deferred.as_mut() else { return };
    let Some(volume) = volume else {
        // Its volume gone, a bake goes with it.
        if def.irradiance.baked_from.take().is_some() {
            let empty = crate::renderer::gi::ProbeGrid::new(Vec3::ZERO, Vec3::ZERO, [0, 0, 0]);
            def.irradiance.upload(&renderer.device, &renderer.queue, &empty);
        }
        return;
    };
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for v in [volume.min, volume.max, volume.ambient] {
        v.to_array().iter().for_each(|f| f.to_bits().hash(&mut hasher));
    }
    volume.resolution.hash(&mut hasher);
    volume.capture_request().hash(&mut hasher);
    let fingerprint = hasher.finish();
    if def.irradiance.baked_from == Some(fingerprint) {
        return;
    }

    let grid = crate::renderer::gi::ProbeGrid::new(volume.min, volume.max, volume.resolution);
    def.irradiance.begin_bake(&renderer.device, &renderer.queue, &grid, volume.ambient);
    let taa = renderer.taa.take();
    let ssgi = renderer.ssgi.take();
    let history = renderer.previous_view_proj.take();
    for (index, probe) in grid.probes.iter().enumerate() {
        for face in 0..FACES {
            let (forward, up) = crate::renderer::reflection::face_basis(face);
            let mut encoder = renderer
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("irradiance_volume_face") });
            let fixed = FixedView {
                position: probe.position,
                forward,
                up,
                near: 0.05,
                far: 1000.0,
                projection: None,
                aspect: 1.0,
                bounce: true,
            };
            // The HDR target stands in for the output view: a capture stops before the post chain.
            let hdr = renderer.post.hdr_texture_view.clone();
            draw_camera_view(world, &mut encoder, &hdr, renderer, None, false, None, false, Some(fixed));
            if let Some(def) = renderer.deferred.as_ref() {
                def.irradiance.record_bake_face(&renderer.device, &mut encoder, &renderer.post.hdr_texture_view, face);
                if face + 1 == FACES {
                    def.irradiance.record_bake_probe(&mut encoder, index);
                }
            }
            renderer.queue.submit(std::iter::once(encoder.finish()));
        }
    }
    let mut encoder = renderer
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("irradiance_volume_finish") });
    if let Some(def) = renderer.deferred.as_mut() {
        def.irradiance.record_bake_finish(&mut encoder);
        def.irradiance.baked_from = Some(fingerprint);
    }
    renderer.queue.submit(std::iter::once(encoder.finish()));
    renderer.taa = taa;
    renderer.ssgi = ssgi;
    renderer.previous_view_proj = history;
    renderer.skip_sky = false;
    tracing::debug!(probes = grid.probes.len(), "[Render] irradiance volume baked");
}

/// Draws the scene again for each [`PlanarReflection`] `camera` can see from the front, into that
/// mirror's texture, before that camera's view is drawn — `aspect` is the view's shape. A mirror out
/// of view or seen from behind costs nothing and draws plain in this view. See
/// [`crate::renderer::planar`].
///
/// **Once per view, each just before its view.** A frame with several views (a texture camera, a
/// split-screen view) captures for each in turn: every capture and every view is a submission of its
/// own, so the GPU runs them as capture, view, capture, view, and each view samples the reflection
/// made for its own camera. The frame's own view captures last.
fn capture_planar_reflections(
    world: &World,
    renderer: &mut Renderer,
    camera: Option<Entity>,
    aspect: f32,
) {
    use crate::renderer::components::PlanarReflection;
    use crate::renderer::planar::{oblique_projection, reflect_direction, reflect_point};
    renderer.planar.begin_view();
    let mirrors: Vec<(Entity, PlanarReflection)> =
        world.query::<(Entity, &PlanarReflection)>().iter().map(|(entity, r)| (entity, *r)).collect();
    let owners: Vec<Entity> = mirrors.iter().map(|m| m.0).collect();
    renderer.planar.retain(&owners.iter().map(|e| e.id()).collect::<Vec<_>>());
    let Some(camera) = camera.filter(|_| !mirrors.is_empty()) else { return };
    // The camera as this frame draws it — shaken, if it shakes — so the mirror shakes with the view.
    let Some((cam, eye, turn)) = drawn_camera(world, camera) else { return };
    let projection = cam.get_projection(aspect);
    let forward = Camera::front(turn);
    let up = Camera::right(turn).cross(forward);
    let frustum = crate::math::Frustum::from_matrix(&(projection * Camera::view(eye, turn)));

    // Which mirrors, decided before anything is drawn.
    let visible: Vec<(Entity, gizmo_math::Vec4)> = {
        let transforms = world.query::<&gizmo_physics_core::components::GlobalTransform>();
        let meshes = world.query::<&crate::renderer::components::Mesh>();
        let renderers = world.query::<&MeshRenderer>();
        let hidden = collect_hidden(
            world,
            &world.query::<&crate::core::component::Children>(),
        );
        mirrors
            .iter()
            .filter_map(|(id, reflection)| {
                renderers.get(*id)?;
                if hidden.contains(id) {
                    return None;
                }
                let model = transforms.get(*id)?.matrix;
                let plane = reflection.world_plane(&model)?;
                let in_view = meshes.get(*id).is_none_or(|mesh| {
                    let placed = model * Mat4::from_translation(mesh.center_offset);
                    frustum.intersects_aabb(mesh.bounds.transform(&placed))
                });
                (in_view && plane.truncate().dot(eye) + plane.w > 0.0).then_some((*id, plane))
            })
            .collect()
    };
    if visible.is_empty() {
        return;
    }

    let size = (renderer.size.width, renderer.size.height);
    let taa = renderer.taa.take();
    let ssgi = renderer.ssgi.take();
    let history = renderer.previous_view_proj.take();
    for (owner, plane) in visible {
        if !renderer.planar.prepare(owner.id(), size) {
            let texture = renderer.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("planar_reflection"),
                size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: renderer.post.hdr_texture.format(),
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let sampler = renderer.device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("planar_reflection"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            });
            let bind_group = {
                let mut assets = renderer.asset_manager.write().unwrap_or_else(|e| e.into_inner());
                crate::renderer::asset::AssetManager::material().base_colour(&view).sampler(&sampler).label("planar_reflection").build(
                    &mut assets,
                    &renderer.device,
                    &renderer.queue,
                    &renderer.scene.texture_bind_group_layout,
                )
            };
            renderer.planar.insert(owner.id(), texture, bind_group);
        }

        let normal = plane.truncate();
        let (position, look, look_up) =
            (reflect_point(eye, plane), reflect_direction(forward, normal), reflect_direction(up, normal));
        let oblique = oblique_projection(projection, Mat4::look_to_rh(position, look, look_up), plane);
        let fixed = FixedView {
            position,
            forward: look,
            up: look_up,
            near: cam.near,
            far: cam.far,
            projection: Some((oblique, cam.fov)),
            aspect,
            bounce: false,
        };
        let mut encoder = renderer
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("planar_reflection") });
        // The HDR target stands in for the output view: a capture stops before the post chain.
        let hdr = renderer.post.hdr_texture_view.clone();
        draw_camera_view(world, &mut encoder, &hdr, renderer, Some(camera), true, None, false, Some(fixed));
        let Renderer { planar, post, .. } = renderer;
        planar.record_copy(&mut encoder, &post.hdr_texture, owner.id());
        renderer.queue.submit(std::iter::once(encoder.finish()));
    }
    renderer.taa = taa;
    renderer.ssgi = ssgi;
    renderer.previous_view_proj = history;
}

/// Where `camera` is in the world, as this frame draws it: [`drawn_camera`]'s position for a
/// camera, [`placed_at`] for anything else (a reflection probe) — so the level of detail and the
/// picture never disagree about where the camera stands.
fn camera_world_position(world: &World, camera: Option<Entity>) -> Vec3 {
    let Some(camera) = camera else {
        return Vec3::ZERO;
    };
    drawn_camera(world, camera).map_or_else(|| placed_at(world, camera).0, |(_, at, _)| at)
}

/// The camera `id` as this frame draws it — its `Camera`, where it stands and how it is turned,
/// with this instant's [`CameraShake`](crate::renderer::components::CameraShake) applied to the
/// pose ([`shake_view`]).
/// `None` when `id` is not a camera. Every view of a camera is built from this, and it writes
/// nothing: the camera the game owns stays where the game put it.
fn drawn_camera(world: &World, id: Entity) -> Option<(Camera, Vec3, gizmo_math::Quat)> {
    let camera = world.query::<&Camera>().get(id).copied()?;
    let (at, turn) = placed_at(world, id);
    let (at, turn) = shake_view(world, id, at, turn);
    Some((camera, at, turn))
}

/// Where entity `id` is and how it is turned: its synced `GlobalTransform`, else its own
/// `Transform`, else the origin, unrotated. A camera's aim is this rotation.
///
/// The `Transform` fallback matters: a hand-built camera that only got a Transform + Camera (no
/// GlobalTransform) was silently skipped and the view stuck at the origin — nothing read the
/// Transform that gameplay/WASD moved. A custom App may not register the transform systems at all,
/// so a camera's GlobalTransform is easily missing or a frame stale.
fn placed_at(world: &World, id: Entity) -> (Vec3, gizmo_math::Quat) {
    world
        .query::<&gizmo_physics_core::components::GlobalTransform>()
        .get(id)
        .map(|g| {
            let (_, rotation, at) = g.matrix.to_scale_rotation_translation();
            (at, rotation)
        })
        .or_else(|| {
            world
                .query::<&gizmo_physics_core::components::Transform>()
                .get(id)
                .map(|t| (t.position, t.rotation))
        })
        .unwrap_or((Vec3::ZERO, gizmo_math::Quat::IDENTITY))
}

/// The per-frame clocks a game can see, advanced once a frame: tweens, colour tweens, typewriters
/// and camera shakes on the animation clock (so time scale and pause apply), the screen fade on
/// the real frame time (a fade into a pause menu must not stop with the game it is covering) —
/// and, before them, each `LocalizedText`'s label ([`resolve_localized_texts`]), which a
/// typewriter types and a missing key is recorded from.
///
/// They live with drawing because they are what moves on screen, but game logic waits on them —
/// a level that changes when the fade has gone dark, a door that opens when its tween ends — so a
/// frame driven without drawing has to run them too. `prepare_frame` and
/// [`HeadlessFrames`](crate::simple::HeadlessFrames) both call this one function, so the two
/// cannot drift apart.
pub fn advance_frame_clocks(world: &mut World) {
    // Labels first: a typewriter types what its `LocalizedText` resolves to this frame.
    resolve_localized_texts(world);
    let tween_dt = world.get_resource::<gizmo_core::time::Time>().map(|t| t.dt()).unwrap_or(0.0);
    crate::renderer::components::color_tween_system(world, tween_dt);
    // The animation clock — tweens, then keyed transforms (a door, a platform, a cutscene camera)
    // and keyed fields (a lamp that pulses, a sprite that fades) — AFTER the colour tweens: a keyed
    // track wins over a colour tween on the same field, deliberately. One function, which the
    // headless loop of a build without `render` runs too. The registry of animatable fields is
    // made (with the engine's in it) before the driver looks for it. No turbofish here on purpose:
    // the draw-read scan in `render_world` reads `::<` calls in this directory as reads the draw
    // makes, and the registry is the game world's alone.
    crate::renderer::property_animation::animatable_properties(world);
    crate::renderer::keyed_animation::advance_animation(world, tween_dt);
    typewriter_system(world, tween_dt);
    // The trauma wearing off is game state (`CameraShake::trauma`); the shaken view is the draw's.
    advance_camera_shakes(world, tween_dt);
    let real_dt = world.get_resource::<gizmo_core::time::Time>().map(|t| t.raw_dt()).unwrap_or(0.0);
    crate::renderer::screen_fade::screen_fade_system(world, real_dt);
}

/// The part of a frame that happens **once**, however many cameras then draw it, and **everything a
/// frame writes that the game can see**: meshes for terrains, cloth and soft bodies, the frame
/// clocks ([`advance_frame_clocks`]: tweens, typewriters, camera shakes, the screen fade),
/// transforms, materials, animation, bone attachments, the CPU side of GPU physics, particle
/// emitters ([`ParticleSpawns`]), the water the underwater fog asks about (`WaterVolumes`, with the
/// `physics` feature) and the end of the debug-line frame ([`GizmoFrame`](crate::renderer::GizmoFrame)).
///
/// After it, [`draw_frame`] needs the world only to read — so it can be given a copy of this one.
pub fn prepare_frame(world: &mut World, renderer: &mut Renderer) {
    // The last draw's parts, into the profile (a frame late: the draw finished after its preparation).
    if let Some(mut profiler) = world.get_resource_mut::<gizmo_core::profiler::FrameProfiler>() {
        for (name, time) in renderer.draw_laps.drain(..) {
            profiler.record_scope(name, time);
        }
    }
    renderer.draw_laps.clear();
    let mut laps = Laps::start();
    // A `DespawnAfter` with no `LifetimePlugin` used to pile entities up in silence; say so, once.
    crate::systems::lifetime::warn_if_inert(world);
    // One step of object-motion history per frame, however many views then draw it.
    renderer.motion_history.begin_frame();
    // **A terrain is a recipe, and the mesh built from it is what gets drawn.** A loaded scene
    // carries the recipe alone — `Mesh` owns GPU buffers, so no file can hold one — and until
    // 2026-08-19 the only code that performed the conversion was the editor's, driven by a queue
    // of editor edits. So a saved level came back with an entity that says it is a terrain and
    // draws nothing, and an exported game never had one at all. Run before the transform pass
    // below so the mesh it adds gets its `GlobalTransform` on the same frame.
    #[cfg(feature = "physics")]
    crate::systems::terrain::terrain_mesh_system(world, renderer);
    // A `Cloth` component's mesh, from where its nodes are now (it was never drawn).
    #[cfg(feature = "physics-soft")]
    crate::systems::cloth::cloth_mesh_system(world);
    #[cfg(feature = "physics-soft")]
    crate::systems::cloth::rope_mesh_system(world);
    #[cfg(feature = "physics-soft")]
    crate::systems::cloth::soft_body_mesh_system(world);

    // **Materials that a scene describes are made before anything edits them.** A scene that has
    // just loaded carries `MaterialDesc`s and no `Material`; the clocks below tween materials, and a
    // tween over a material not made yet skipped that frame — a reopened scene, or ⏹, put every
    // colour tween one frame behind the one that was saved (measured: 4622 of 16384 pixels).
    crate::renderer::material_sync::resolve_material_descriptions(world, renderer);

    // **Tweens first**, before the transform pass below so a tweened `Transform` is drawn where it
    // is this frame.
    advance_frame_clocks(world);

    // Every renderable object needs an up-to-date `GlobalTransform` (the draw query
    // below requires it, and physics/gameplay only write the local `Transform`).
    // Realize the long-standing "update_transforms right before the pass" TODO here
    // so a caller that just spawned `Transform + Mesh + Material` is not silently
    // culled (the classic "empty screen" footgun) and doesn't have to hand-run the
    // transform systems every frame.
    ensure_global_transforms(world);

    // **Materials, both directions.** A scene that has just loaded carries `MaterialDesc`s and no
    // `Material` — descriptions are what a file can hold — so they were resolved above, before the
    // clocks; here a description is written back for every live material, so the next save has
    // something to write. Until 2026-08-19 neither happened and a saved scene lost every material
    // a user had authored.
    // A material whose texture or maps changed since its bind group was built — in the editor, by
    // the game, by an undo — is built again from them.
    crate::renderer::material_sync::rebind_changed_materials(world, renderer);
    // A material built from nodes (`SurfaceGraph`) is drawn with its graph's surface: built here
    // when the graph is new or changed, and set again over whatever a load or an edit wrote into
    // the material's type — before the descriptions are synced, so a save writes the type drawn.
    crate::renderer::shader_graph::resolve_surface_graphs(world, renderer);
    crate::renderer::material_sync::sync_material_descriptions(world);
    // And every mesh built in code that a key can build again names itself as its `MeshSource`, so
    // a save writes it (`mesh_sync`).
    crate::renderer::mesh_sync::describe_meshes(world);
    // Every text names the face it draws in, and a text read from a scene finds its face by name
    // (`text_sync`).
    crate::renderer::text_sync::sync_text_fonts(world, renderer);
    // And every sprite and tilemap its image (`sprite_sync`).
    crate::renderer::sprite_sync::sync_sprite_images(world, renderer);
    // And every decal its description, the texture by name (`sync_decals`).
    crate::renderer::components::sync_decals(world, renderer);
    // And every LOD group its description, its meshes by key (`sync_lod_groups`).
    crate::renderer::components::sync_lod_groups(world, renderer);
    // And every camera drawing into a texture its size (`sync_camera_targets`).
    crate::renderer::components::sync_camera_targets(world, renderer);
    laps.lap(world, "render.sync");

    // **Advance skeletal animation.** `collect_draw_items` below reads `Skeleton` for its skinning
    // matrices, and until this call nothing in the engine ever advanced the pose it reads: the two
    // systems live in `gizmo-renderer`, `current_time += dt · speed` appears nowhere else in the
    // workspace, and no schedule, plugin or demo invoked either of them. The engine was drawing a
    // pose it never stepped.
    //
    // They are called from the render pass rather than from a schedule because of their
    // signatures: both need a `wgpu::Queue` to upload the skin matrices, which no ordinary system
    // has, and that is very likely why they were never wired anywhere. Here is the one place that
    // holds the world, the queue, and a position before the draw path reads the result.
    //
    // Player first, state machine second: an entity carrying both is a caller error, and if one
    // has to win it should be the higher-level driver. The two capabilities are
    // `AnimationPlayer` (a clip and a playhead) and `AnimationStateMachine` (states and
    // transitions) — named here rather than left implicit in the system names, because the
    // render-parity inventory reads these paths for the capabilities they know, and a type only
    // ever touched through a system is invisible otherwise. The studio's pipeline drives both
    // as well; it drove only the player until 2026-08-19.
    // A glTF model read from a scene file gets its skeletons and clips back from the model's file
    // before anything poses it — and a model asked for by name alone (a `GltfModel` with nothing
    // under it) is built from its file first, so the resolver finds its meshes.
    crate::gltf_build::build_requested_gltf_models(world, renderer);
    crate::renderer::gltf_model::resolve_gltf_models(world, renderer);
    // A model whose clips the editor has changed keeps those clips for the save to write.
    crate::renderer::gltf_model::sync_clip_edits(world);
    laps.lap(world, "render.models");
    let animation_dt = world
        .get_resource::<gizmo_core::time::Time>()
        .map(|t| t.dt())
        .unwrap_or(0.0);
    // Posed every frame, the clock stopped or not: a paused game's playhead is still where the
    // pose is read from, and the editor's timeline moves it while ⏸ holds dt at 0. Until 2026-11-13
    // these ran only on a clock that ran, and scrubbing a paused game changed nothing on screen. At
    // dt 0 a player's time does not move; only its pose is evaluated.
    // Each skeleton as often as the last frames drew it big (`gizmo_renderer::animation_lod`).
    crate::renderer::animation_update_system_seen(world, animation_dt, &renderer.queue, &renderer.skeletons_seen);
    crate::renderer::animation_state_machine_update_system_seen(world, animation_dt, &renderer.queue, &renderer.skeletons_seen);
    apply_root_motion(world);
    if animation_dt > 0.0 {
        // Sprite-sheet animation rides the same clock: `set_time_scale` and pause apply to it.
        crate::renderer::components::sprite_animation_system(world, animation_dt);
    }

    // **Limbs reach for their targets.** Every `IkChains` bends its skeleton's drawn pose, on top of
    // what the two drivers above made of it this frame and before anything follows a bone — a
    // sword in a reaching hand goes where the hand went (`ik`).
    apply_ik_chains_system(world, &renderer.queue);
    laps.lap(world, "render.animation");

    // **Bones carry what is attached to them.** `BoneAttachmentSystem` places every entity
    // parented to a joint — a sword in a hand, a hat on a head — onto the pose the animation
    // systems just wrote, so it belongs immediately after them and before anything reads a
    // transform. The game path did not run it at all: an attachment followed the animation in the
    // editor's viewport (which does run it) and stayed frozen in an exported game. The mirror
    // image of the state machine, which was engine-only until the same day.
    {
        let mut bone_attachment = crate::systems::transform::BoneAttachmentSystem;
        gizmo_core::system::System::run(&mut bone_attachment, world, animation_dt);
    }

    // **The scene's fluid**, and what stands in it: the solver is given the `FluidSimulation` (no
    // water without one) and every `FluidInteractor` where its transform now is (`fluid`).
    fluid_scene_system(world, renderer, animation_dt);

    // **The decals, on the forward-only path**, which projects them in its lit shader: their textures
    // in the decal array, the decals where that shader reads them (`decals`). The desktop draws its
    // own over the G-buffer, view by view.
    if cfg!(gizmo_forward_only) {
        decals::prepare_forward_decals(world, renderer);
    }

    // ECS veri GPU'ya basılır ve GPU verisi ECS'ye alınır (GPU-fizik yolu — `physics` ister)
    #[cfg(feature = "physics")]
    {
        gpu_physics_submit_system(world, renderer);
        gpu_physics_readback_system(world, renderer);
    }

    // **Particle emitters**, stepped here and not in the draw: an emitter's burst is taken and its
    // timer runs, which is the game's state. What they put out this frame is `ParticleSpawns`, which
    // the draw uploads. Only with a GPU particle system to receive it — without one an emitter
    // keeps its burst, as it always has.
    if renderer.gpu_particles.is_some() {
        let dt = world.get_resource::<gizmo_core::time::Time>().map_or(0.016, |t| t.dt());
        step_particle_emitters(world, dt);
        #[cfg(feature = "physics")]
        particles::add_fracture_dust(world);
    } else {
        world.remove_resource::<ParticleSpawns>();
    }

    // **The water**, for the draw's underwater fog: the physics world's zones, handed over as a
    // resource a render world can hold (see `WaterVolumes`).
    #[cfg(feature = "physics")]
    water::publish_water_volumes(world);

    // **The debug-line frame ends here**, before the draw — see `end_debug_line_frame`.
    end_debug_line_frame(world);
    laps.lap(world, "render.effects");
}

/// Times the parts of a frame into the world's `FrameProfiler`, when it has one: each
/// [`lap`](Self::lap) records the time since the last as its own scope. What lets a game see which
/// part of the engine's frame its time went to (`dunya --bench`, its F3 meter).
struct Laps {
    last: gizmo_core::time::Instant,
}

impl Laps {
    fn start() -> Self {
        Self { last: gizmo_core::time::Instant::now() }
    }

    /// The preparation's half: straight into the world's profiler (the game world, at the sync point).
    fn lap(&mut self, world: &World, name: &'static str) {
        let now = gizmo_core::time::Instant::now();
        if let Some(mut profiler) = world.get_resource_mut::<gizmo_core::profiler::FrameProfiler>() {
            profiler.record_scope(name, now - self.last);
        }
        self.last = now;
    }

    /// The draw's half: into [`Renderer::draw_laps`](crate::renderer::Renderer::draw_laps), which the
    /// next preparation moves into the profiler — the draw's world is a copy with none.
    fn draw_lap(&mut self, renderer: &mut Renderer, name: &'static str) {
        let now = gizmo_core::time::Instant::now();
        renderer.draw_laps.push((name, now - self.last));
        self.last = now;
    }
}

/// Ends the frame's debug lines: what the game drew into [`Gizmos`](crate::renderer::Gizmos) this
/// frame is moved into the [`GizmoFrame`](crate::renderer::GizmoFrame) resource the draw reads, and
/// the game's `Gizmos` starts the next frame empty — the last fixed step's lines stay until the next
/// step (see `Gizmos`). With no `Gizmos` there is no frame to draw.
///
/// The draw used to clear the game's lines after drawing them, which is a write into the game's
/// world. [`prepare_frame`] calls this, and so does [`HeadlessFrames`](crate::simple::HeadlessFrames):
/// a frame played without drawing still ends its lines, where they used to pile up for the whole run.
pub(crate) fn end_debug_line_frame(world: &mut World) {
    let lines = world.get_resource_mut::<crate::renderer::Gizmos>().map(|mut g| g.take_frame());
    match lines {
        Some(frame) => world.insert_resource(frame),
        None => {
            world.remove_resource::<crate::renderer::GizmoFrame>();
        }
    }
}

/// The frame's GPU simulations ([`step_gpu_simulations`]), stepped once **in a submission of their
/// own, ahead of every view**. A mirror's capture and a texture camera are each submitted before the
/// frame's own encoder, so a step recorded into that encoder reached the frame's view and not theirs:
/// they drew every particle, the fluid and the GPU bodies one step behind it
/// (`a_texture_camera_where_the_frames_camera_stands_draws_the_frames_picture`).
fn step_gpu_simulations_ahead(world: &World, renderer: &mut Renderer, detail: SimulationDetail) {
    let mut step = renderer.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("gpu_simulations"),
    });
    step_gpu_simulations(world, &mut step, renderer, detail);
    renderer.queue.submit(std::iter::once(step.finish()));
}

/// The share of the particles and of the fluid one camera standing at `position` asks for: less the
/// farther it stands from where the particles are (`GpuParticleSystem::region`; `None` takes them as
/// beside it) and from the fluid's tank (its box, `FluidSettings::bounds_min`/`bounds_max`). Both were
/// measured from fixed points near the world's origin until 2026-11-20, so a camera 200 m from it drew
/// no particle at all, wherever they were: a city a few hundred metres off the origin, as
/// `nfsu2-gizmo`'s is, had none in any street.
fn detail_from(position: Vec3, particles: Option<(Vec3, Vec3)>, fluid: (Vec3, Vec3)) -> (f32, f32) {
    let by_distance = |distance: f32, steps: [f32; 3]| {
        if distance < steps[0] {
            1.0
        } else if distance < steps[1] {
            0.5
        } else if distance < steps[2] {
            0.1
        } else {
            0.0
        }
    };
    // How far `position` stands from the box `lo`–`hi`: 0 inside it.
    let to_box = |(lo, hi): (Vec3, Vec3)| (lo - position).max(position - hi).max(Vec3::ZERO).length();
    (
        by_distance(particles.map_or(0.0, to_box), [50.0, 100.0, 200.0]),
        by_distance(to_box(fluid), [40.0, 80.0, 150.0]),
    )
}

/// GPU simulation for the frame — the smoke, the GPU bodies, the fluid, the particles — recorded
/// into `encoder` once, each to the share `detail` asks for. The share each ran is written to it
/// (`active_particles`), and that is what every view of the frame draws.
fn step_gpu_simulations(
    world: &World,
    encoder: &mut wgpu::CommandEncoder,
    renderer: &mut Renderer,
    detail: SimulationDetail,
) {
    // The volumetric smoke's step, once for the frame: each view only raymarches the grid.
    if let Some(smoke) = &renderer.smoke {
        let (time, dt) = world
            .get_resource::<gizmo_core::time::Time>()
            .map(|t| (t.elapsed() as f32, t.dt()))
            .unwrap_or((0.0, 1.0 / 60.0));
        smoke.advect(encoder, &renderer.queue, time, dt);
    }

    if let Some(physics) = renderer.gpu_physics.as_mut() {
        // Her frame başında sıradaki state'i çekmek için WGPU CommandEncoder'a asenkron mapping iste.
        physics.request_readback(encoder);

        // On the game's clock, as the CPU bodies are: whole steps of the bodies' own fixed step for
        // the time the frame holds — none under ⏸, the time scale applied (`GpuPhysicsSystem::advance`).
        // A world with no clock gets one step a frame, as every frame used to.
        let elapsed = world
            .get_resource::<gizmo_core::time::Time>()
            .map_or(physics.clock().fixed_dt(), |t| t.dt());
        physics.advance(encoder, &renderer.queue, elapsed);
        physics.debug_compute_pass(encoder);
    }

    // Every view is simulated for: the nearest camera sets the level of detail. One camera — the
    // ordinary frame — gives exactly what that camera alone always gave.
    let particles_at = renderer.gpu_particles.as_ref().and_then(|p| p.region());
    let tank = renderer.gpu_fluid.as_ref().map_or((Vec3::ZERO, Vec3::ZERO), |f| {
        let s = f.settings();
        (Vec3::from_array(s.bounds_min), Vec3::from_array(s.bounds_max))
    });
    let (particle_share, fluid_share) = match detail {
        SimulationDetail::Full => (1.0, 1.0),
        SimulationDetail::NearestOf([]) => detail_from(Vec3::ZERO, particles_at, tank),
        SimulationDetail::NearestOf(cameras) => cameras
            .iter()
            .map(|&p| detail_from(p, particles_at, tank))
            .fold((0.0f32, 0.0f32), |(p0, f0), (p1, f1)| (p0.max(p1), f0.max(f1))),
    };
    // No fluid unless the scene has one, whoever asks for how much: its settings hold no particles
    // then, and a zero count makes both the step and `render_ssfr` return at once — the default
    // pool's SSFR surface once rendered over every scene as a mottled overlay that read like broken
    // shadows.
    // On the game's clock, as the GPU bodies are: no step under ⏸, slower under a time scale.
    if let Some(fluid) = renderer.gpu_fluid.as_mut() {
        fluid.active_particles = (fluid.settings().particles as f32 * fluid_share) as u32;
        let elapsed = world.get_resource::<gizmo_core::time::Time>().map(|t| t.dt());
        fluid.advance(encoder, &renderer.queue, elapsed, fluid.active_particles);
    }

    if let Some(particles) = renderer.gpu_particles.as_mut() {
        let (dt, time) = world
            .get_resource::<gizmo_core::time::Time>()
            .map(|t| (t.dt(), t.elapsed() as f32))
            .unwrap_or((0.016, 0.0));
        particles.update_params(&renderer.queue, dt, time); // time → curl-noise evrimi
        // Fill it from the scene: what `prepare_frame` stepped out of the emitters this frame.
        // Without this the engine steps and draws a particle set that nothing ever puts anything
        // into: the emitter-to-GPU bridge existed only inside `gizmo-studio`, so a
        // `ParticleEmitter` on an entity emitted nothing for anyone using this pass. See
        // `particles.rs`.
        if let Some(spawns) = world.get_resource::<ParticleSpawns>() {
            // Once per batch: the draw reads this, it does not clear it (see `ParticleSpawns`).
            particles.spawn_batch(&renderer.queue, spawns.batch(), spawns.particles());
        }
        // Nothing alive: no dispatch over the whole ring, and no draw of it (see
        // `GpuParticleSystem::is_idle`).
        particles.active_particles = if particles.is_idle() {
            0
        } else {
            (particles.max_particles as f32 * particle_share) as u32
        };
        particles.compute_pass(encoder, particles.active_particles);
    }
}

/// Moves each entity whose [`RootMotion`](crate::renderer::components::RootMotion) says `apply` by
/// the travel the animation drivers just took out of its pose — turned and scaled by its own
/// transform, since the travel is in the model's space. A game that moves the entity some other way
/// (a character controller) leaves `apply` off and reads the delta.
pub fn apply_root_motion(world: &mut World) {
    use crate::renderer::components::RootMotion;
    for (_, mut transform, motion) in world.query_mut::<(Entity, &mut gizmo_physics_core::components::Transform, &RootMotion)>().iter_mut() {
        if motion.apply && motion.delta != Vec3::ZERO {
            let step = transform.rotation * (motion.delta * transform.scale);
            transform.position += step;
            transform.update_local_matrix();
        }
    }
}

/// A view that is not a camera entity: a reflection probe's cube face (square, 90°), or a camera
/// reflected in a mirror (its own `projection`). Looks along `forward` with `up`; drawn into the HDR
/// target and stopped there — no post chain, no HUD.
#[derive(Clone, Copy)]
struct FixedView {
    position: Vec3,
    forward: Vec3,
    up: Vec3,
    near: f32,
    far: f32,
    /// In place of the 90° square one, with the field of view it came from.
    projection: Option<(Mat4, f32)>,
    /// The shape of the view: 1 for a probe's face, the reflected view's for a mirror. Everything
    /// else the view fits to its shape — the shadow cascades — takes it from here, so it cannot
    /// disagree with `projection`.
    aspect: f32,
    /// An irradiance volume's capture: the light bounced off the scene alone, the sky left out —
    /// the background black, no environment map, skybox or backdrop (`Renderer::skip_sky`).
    bounce: bool,
}

/// The size, in whole pixels, of a view's rectangle `[x, y, w, h]` — what it is drawn at.
fn rectangle_size(region: [f32; 4]) -> (u32, u32) {
    (region[2].round().max(1.0) as u32, region[3].round().max(1.0) as u32)
}

/// The shape a view is drawn at: a capture's own, else its rectangle's (a split-screen view, a
/// camera drawing into a texture), else the whole target's.
fn view_aspect(capture: Option<&FixedView>, region: Option<[f32; 4]>, size: (u32, u32)) -> f32 {
    match (capture, region) {
        (Some(fixed), _) => fixed.aspect,
        (None, Some([_, _, w, h])) => w / h,
        (None, None) if size.1 > 0 => size.0 as f32 / size.1 as f32,
        (None, None) => 1.0,
    }
}

// ============================================================
//  RenderContext Kolaylık Metodu
//  `ctx.default_render(world)` ile varsayılan pipeline çalışır.
// ============================================================

/// Convenience methods added on top of `RenderContext`.
/// Automatically included with `use gizmo::prelude::*;`.
pub trait RenderContextExt {
    /// Runs the engine's default render pipeline.
    /// Deferred rendering, shadows, SSAO, SSR, TAA and post-processing are included.
    ///
    /// ```
    /// use gizmo::prelude::*;
    /// # struct GameState;
    /// fn render(world: &mut World, _state: &GameState, ctx: &mut RenderContext) {
    ///     ctx.disable_gpu_compute();
    ///     ctx.default_render(world);
    /// }
    /// # // The `App::set_simple_render` bound: for<'a> FnMut(&mut World, &State, &mut RenderContext<'a>)
    /// # let _: fn(&mut World, &GameState, &mut RenderContext<'_>) = render;
    /// ```
    fn default_render(&mut self, world: &mut crate::core::World);
}

impl<'a> RenderContextExt for crate::renderer::RenderContext<'a> {
    fn default_render(&mut self, world: &mut crate::core::World) {
        let (encoder, view, renderer) = self.parts_mut();
        default_render_pass(world, encoder, view, renderer);
    }
}

mod batching;
pub use batching::{clear_render_cache, draws_blended, DrawItem, RenderCache};
use batching::BakedRoute;
mod kept;

pub mod visibility_index;
mod passes;

mod render_world;
pub use render_world::{render_extractor, RenderWorld};

mod decals;
pub use decals::{collect_decals, prepare_forward_decals};

mod particles;
pub use particles::{emit, step_particle_emitters, ParticleSpawns};
#[cfg(feature = "physics")]
pub use particles::fracture_dust;

mod sprite;
pub use sprite::record_sprites;

mod shake;
pub use shake::{advance_camera_shakes, shake_view};

mod ik;
pub use ik::apply_ik_chains_system;

mod fluid;
pub use fluid::{fluid_scene_system, fluid_settings};
#[cfg(feature = "physics")]
pub mod water;
#[cfg(feature = "physics")]
pub use water::WaterVolumes;

mod text;
pub use text::{record_text, resolve_localized_texts, typewriter_system, LocalizedLabel, Typewriter};

mod hidden;
pub use hidden::collect_hidden;

mod view;
use view::draw_camera_view;

mod shared;
pub use shared::{
    active_camera_grade, collect_scene_lights, collect_scene_setup, SceneLights,
    SceneSetup, SceneSetupInputs,
};

#[cfg(test)]
mod frame_split_tests;

#[cfg(test)]
mod keyed_animation_tests;

#[cfg(test)]
mod taa_jitter_tests;

#[cfg(test)]
mod golden_render_tests;
