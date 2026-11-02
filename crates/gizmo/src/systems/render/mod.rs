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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
/// Whether the frame is drawn as wireframe.
#[allow(clippy::derivable_impls)]
pub struct WireframeConfig {
    /// Draw everything as wireframe, whatever the individual materials say.
    pub global: bool,
}


/// Step the GPU fluid simulation for this frame.
///
/// `active_particles` is how much of the fluid to simulate — the caller's level-of-detail
/// decision, because the two pictures make it differently: a game scales it by camera distance,
/// the editor's viewport runs the whole thing (an editor shows what is there, which is the same
/// reasoning as the `lod_vbufs` entry in the render-parity exceptions).
///
/// Nothing is recorded for a scene with no fluid, and the underlying pass returns immediately at
/// zero particles.
///
/// It is a function here rather than a line in each pipeline for the reason `record_forward_decals`
/// is: the editor drew no fluid at all until 2026-08-19, and a second implementation is how that
/// comes back.
#[cfg(feature = "render")]
pub fn step_gpu_fluid(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &crate::renderer::Renderer,
    active_particles: u32,
) {
    let Some(fluid) = &renderer.gpu_fluid else {
        return;
    };
    fluid.compute_pass(encoder, &renderer.queue, true, active_particles);
}

/// Draw the fluid's screen-space surface onto the lit HDR image.
///
/// The other half of [`step_gpu_fluid`], and it runs where the decals do: after the geometry pass
/// has written depth (it reads it) and before post-processing reads the HDR texture. The particles
/// themselves are drawn inside the forward pass; this is the surface reconstruction over them.
#[cfg(feature = "render")]
pub fn record_fluid_surface(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &crate::renderer::Renderer,
    active_particles: u32,
) {
    let Some(fluid) = &renderer.gpu_fluid else {
        return;
    };
    fluid.render_ssfr(
        encoder,
        &renderer.post.hdr_texture,
        &renderer.post.hdr_texture_view,
        &renderer.depth_texture_view,
        renderer.scene.view_bind_group(),
        active_particles,
    );
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
    let mut missing = Vec::new();
    if let Some(q) = world.query::<(&Mesh, &Transform, Without<GlobalTransform>)>() {
        for (id, _) in q.iter() {
            missing.push(id);
        }
    }
    for id in missing {
        if let Some(e) = world.get_entity(id) {
            world.add_component(e, GlobalTransform::default());
        }
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
    /// Sets `set_render` up so that the scene is drawn with [`default_render_pass`].
    fn with_scene_render(self) -> Self;
}

impl<State: 'static> AppSceneRenderExt for gizmo_app::App<State> {
    fn with_scene_render(self) -> Self {
        self.set_render(|world, _state, encoder, view, renderer, _light_time| {
            renderer.gpu_fluid = None;
            renderer.gpu_physics = None;
            renderer.ssr = None;
            renderer.ssgi = None;
            renderer.volumetric = None;
            renderer.taa = None;
            default_render_pass(world, encoder, view, renderer);
        })
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
/// per batch with camera-visible instances, triangles counted with their instances. Published as
/// the world's [`RenderStats`](crate::renderer::components::RenderStats) resource, so a game can
/// read its own draw-call count — before, only the editor filled it. Under split screen it holds
/// the last view drawn. GPU memory is sampled about once a second, as the editor does, since the
/// allocator report walks every live allocation.
fn publish_render_stats(world: &mut World, renderer: &Renderer, items: &[batching::DrawItem], elapsed: f32) {
    use crate::renderer::components::RenderStats;
    let mut stats = RenderStats::default();
    for item in items.iter().filter(|i| i.visible_in_camera && i.camera_count > 0) {
        stats.draw_calls += 1;
        stats.instances += item.camera_count;
        let indices = if item.ibuf.is_some() { item.index_count } else { item.vertex_count };
        stats.triangles += (indices / 3) * item.camera_count;
    }
    let previous = world
        .get_resource::<RenderStats>()
        .map(|r| (r.gpu_allocated_bytes, r.gpu_sampled_at))
        .unwrap_or((None, f32::NEG_INFINITY));
    if elapsed - previous.1 >= 1.0 {
        stats.gpu_allocated_bytes = renderer.device.generate_allocator_report().map(|r| r.total_allocated_bytes);
        stats.gpu_sampled_at = elapsed;
    } else {
        stats.gpu_allocated_bytes = previous.0;
        stats.gpu_sampled_at = previous.1;
    }
    world.insert_resource(stats);
}

/// A batteries-included render path, serving only to light the models and put them on screen
/// quickly.
/// It is used to avoid writing hundreds of lines of code in freshly opened, empty projects
/// like `tut`.
#[tracing::instrument(skip_all, name = "render_system")]
pub fn default_render_pass(
    world: &mut World,
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    renderer: &mut Renderer,
) {
    // A `DespawnAfter` with no `LifetimePlugin` used to pile entities up in silence; say so, once.
    crate::systems::lifetime::warn_if_inert(world);
    // **Split screen is this same call.** A scene with any camera that has a `viewport` is drawn
    // by `render_viewports`, one view per such camera; every host that already calls this —
    // `SimpleApp`, the editor's game view, a hand-written render loop — gets it without a second
    // entry point to know about.
    if any_camera_has_a_viewport(world) {
        render_viewports(world, encoder, view, renderer);
        return;
    }
    prepare_frame(world, renderer);
    capture_reflection_probes(world, renderer);
    let camera = crate::renderer::components::active_camera(world);
    let lods = step_gpu_simulations(world, encoder, renderer, &[camera_world_position(world, camera)]);
    draw_camera_view(world, encoder, view, renderer, camera, lods, None, true, None);
}

/// Whether any camera in `world` asks for a [`Viewport`](crate::renderer::components::Viewport).
fn any_camera_has_a_viewport(world: &World) -> bool {
    world
        .borrow::<Camera>()
        .iter()
        .any(|(_, c)| c.viewport.is_some())
}

/// **Split screen:** every camera with a [`viewport`](crate::renderer::components::Camera::viewport)
/// drawn into its own rectangle of `view`, in [`order`](crate::renderer::components::Camera::order),
/// then this frame's HUD once over the whole target. With no such camera this is
/// [`default_render_pass`] (which also comes here by itself when one exists).
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
    let mut views: Vec<(i32, u32, crate::renderer::components::Viewport)> = world
        .borrow::<Camera>()
        .iter()
        .filter_map(|(id, c)| c.viewport.map(|v| (c.order, id, v)))
        .collect();
    if views.is_empty() {
        prepare_frame(world, renderer);
        capture_reflection_probes(world, renderer);
        let camera = crate::renderer::components::active_camera(world);
        let lods =
            step_gpu_simulations(world, encoder, renderer, &[camera_world_position(world, camera)]);
        draw_camera_view(world, encoder, view, renderer, camera, lods, None, true, None);
        return;
    }
    views.sort_by_key(|&(order, id, _)| (order, id));
    let size = view.texture().size();

    prepare_frame(world, renderer);
    capture_reflection_probes(world, renderer);
    let positions: Vec<Vec3> =
        views.iter().map(|&(_, id, _)| camera_world_position(world, Some(id))).collect();

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
    let lods = step_gpu_simulations(world, &mut frame, renderer, &positions);

    for (index, &(_, camera, viewport)) in views.iter().enumerate() {
        let region = viewport.to_pixels(size.width, size.height);
        renderer.with_view_history(index, |renderer| {
            draw_camera_view(world, &mut frame, view, renderer, Some(camera), lods, Some(region), false, None);
        });
        let done = std::mem::replace(&mut frame, new_encoder(renderer));
        renderer.queue.submit(std::iter::once(done.finish()));
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
pub fn capture_reflection_probes(world: &mut World, renderer: &mut Renderer) {
    use crate::renderer::components::ReflectionProbe;
    use crate::renderer::reflection::{ReflectionProbeGpu, FACES, MAX_PROBES};
    let mut probes: Vec<(u32, ReflectionProbe)> =
        world.borrow::<ReflectionProbe>().iter().map(|(id, p)| (id, *p)).collect();
    probes.sort_by_key(|&(id, _)| id);
    probes.truncate(MAX_PROBES);
    if probes.is_empty() {
        if renderer.reflection_probe.take().is_some() {
            if let Some(def) = renderer.deferred.as_mut() {
                def.irradiance.set_reflection(&renderer.device, &renderer.queue, None);
            }
        }
        return;
    }
    let owners: Vec<u32> = probes.iter().map(|&(id, _)| id).collect();
    let size = probes
        .iter()
        .map(|(_, p)| p.resolution.clamp(8, 1024).next_power_of_two())
        .max()
        .unwrap_or(128);
    // A different set of probes, or a different size, is a new array — and every slot in it is
    // captured afresh.
    let rebuild = renderer
        .reflection_probe
        .as_ref()
        .is_none_or(|g| g.size != size || g.owners != owners);
    if rebuild {
        let mut gpu = ReflectionProbeGpu::new(&renderer.device, size, owners.len() as u32);
        gpu.owners = owners.clone();
        renderer.reflection_probe = Some(gpu);
    }
    let pending: Vec<usize> = (0..probes.len()).filter(|&slot| rebuild || !probes[slot].1.captured).collect();
    let positions: Vec<Vec3> = owners.iter().map(|&id| camera_world_position(world, Some(id))).collect();

    let taa = if pending.is_empty() { None } else { renderer.taa.take() };
    let ssgi = if pending.is_empty() { None } else { renderer.ssgi.take() };
    let history = if pending.is_empty() { None } else { renderer.previous_view_proj.take() };
    for &slot in &pending {
        let probe = probes[slot].1;
        if let Some(gpu) = renderer.reflection_probe.as_mut() {
            gpu.captured_at[slot] = positions[slot].to_array();
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
            };
            // The HDR target stands in for the output view: a capture stops before the post
            // chain, so nothing is ever written through it.
            let hdr = renderer.post.hdr_texture_view.clone();
            draw_camera_view(world, &mut encoder, &hdr, renderer, None, (0.0, 0.0), None, false, Some(fixed));
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
    if let Some(gpu) = renderer.reflection_probe.as_mut() {
        let sources: Vec<_> = probes
            .iter()
            .zip(&gpu.captured_at)
            .map(|((_, p), at)| {
                crate::renderer::irradiance::ReflectionSource::new(p.intensity, *at)
                    .with_box(p.influence.map(|h| h.to_array()), p.blend_distance)
            })
            .collect();
        if !pending.is_empty() || sources != gpu.uploaded {
            gpu.uploaded = sources.clone();
            let set = crate::renderer::irradiance::ReflectionSet::new(&gpu.cube_view, gpu.mip_count, sources);
            if let Some(def) = renderer.deferred.as_mut() {
                def.irradiance.set_reflection(&renderer.device, &renderer.queue, Some(set));
            }
        }
    }
    if pending.is_empty() {
        return;
    }
    {
        let mut stored = world.borrow_mut::<ReflectionProbe>();
        for &slot in &pending {
            if let Some(mut p) = stored.get_mut(owners[slot]) {
                p.captured = true;
            }
        }
    }
    tracing::debug!(captured = pending.len(), probes = owners.len(), size, "[Render] reflection probes captured");
}

/// Where `camera` is in the world: its synced `GlobalTransform`, else its own `Transform`, else the
/// origin — the same fallback the camera's view matrix is built with, so the level of detail and
/// the picture never disagree about where the camera stands.
fn camera_world_position(world: &World, camera: Option<u32>) -> Vec3 {
    let Some(camera) = camera else {
        return Vec3::ZERO;
    };
    world
        .borrow::<gizmo_physics_core::components::GlobalTransform>()
        .get(camera)
        .map(|g| g.matrix.to_scale_rotation_translation().2)
        .or_else(|| {
            world
                .borrow::<gizmo_physics_core::components::Transform>()
                .get(camera)
                .map(|t| t.position)
        })
        .unwrap_or(Vec3::ZERO)
}

/// The part of a frame that happens **once**, however many cameras then draw it: meshes for
/// terrains, transforms, materials, animation, bone attachments, the CPU side of GPU physics.
fn prepare_frame(world: &mut World, renderer: &mut Renderer) {
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

    // Every renderable object needs an up-to-date `GlobalTransform` (the draw query
    // below requires it, and physics/gameplay only write the local `Transform`).
    // Realize the long-standing "update_transforms right before the pass" TODO here
    // so a caller that just spawned `Transform + Mesh + Material` is not silently
    // culled (the classic "empty screen" footgun) and doesn't have to hand-run the
    // transform systems every frame.
    ensure_global_transforms(world);

    // **Materials, both directions.** A scene that has just loaded carries `MaterialDesc`s and no
    // `Material` — descriptions are what a file can hold — so resolve first, building each one's
    // bind group; then write a description back for every live material, so the next save has
    // something to write. Until 2026-08-19 neither happened and a saved scene lost every material
    // a user had authored.
    crate::renderer::material_sync::resolve_material_descriptions(world, renderer);
    crate::renderer::material_sync::sync_material_descriptions(world);

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
    let animation_dt = world
        .get_resource::<gizmo_core::time::Time>()
        .map(|t| t.dt())
        .unwrap_or(0.0);
    if animation_dt > 0.0 {
        crate::renderer::animation_update_system(world, animation_dt, &renderer.queue);
        crate::renderer::animation_state_machine_update_system(
            world,
            animation_dt,
            &renderer.queue,
        );
        // Sprite-sheet animation rides the same clock: `set_time_scale` and pause apply to it.
        crate::renderer::components::sprite_animation_system(world, animation_dt);
    }

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

    // ECS veri GPU'ya basılır ve GPU verisi ECS'ye alınır (GPU-fizik yolu — `physics` ister)
    #[cfg(feature = "physics")]
    {
        gpu_physics_submit_system(world, renderer);
        gpu_physics_readback_system(world, renderer);
    }

}

/// GPU simulation for the frame — physics, fluid, particles — recorded into `encoder` once.
/// `cam_positions` are the cameras about to draw; the nearest sets the level of detail.
/// Returns `(particle_lod, fluid_lod)` for the draws.
fn step_gpu_simulations(
    world: &mut World,
    encoder: &mut wgpu::CommandEncoder,
    renderer: &mut Renderer,
    cam_positions: &[Vec3],
) -> (f32, f32) {
    if let Some(physics) = &renderer.gpu_physics {
        // Her frame başında sıradaki state'i çekmek için WGPU CommandEncoder'a asenkron mapping iste.
        physics.request_readback(encoder);

        physics.compute_pass(encoder);
        physics.debug_compute_pass(encoder);
    }

    // Compute LOD (Level of Detail) Scaling.
    // `fluid_lod == 0` disables the fluid entirely (both `compute_pass` and
    // `render_ssfr` early-return on a zero active count), so a scene that hasn't
    // opted into fluid never simulates or composites the default 100k-particle
    // ocean — previously its SSFR water surface rendered over every scene as a
    // mottled overlay that read like broken shadows.
    let fluid_pos = Vec3::new(0.0, 5.0, 0.0);
    let lods_for = |cam_pos: Vec3| {
    let dist_to_fluid = (cam_pos - fluid_pos).length();
    let fluid_lod = if !renderer.fluid_enabled {
        0.0
    } else if dist_to_fluid < 40.0 {
        1.0
    } else if dist_to_fluid < 80.0 {
        0.5
    } else if dist_to_fluid < 150.0 {
        0.1
    } else {
        0.0
    };

    let dist_to_origin = cam_pos.length();
    let particle_lod = if dist_to_origin < 50.0 {
        1.0
    } else if dist_to_origin < 100.0 {
        0.5
    } else if dist_to_origin < 200.0 {
        0.1
    } else {
        0.0
    };
    (particle_lod, fluid_lod)
    };
    // Every view is simulated for: the nearest camera sets the level of detail. One camera —
    // the ordinary frame — gives exactly what that camera alone always gave.
    let (particle_lod, fluid_lod) = if cam_positions.is_empty() {
        lods_for(Vec3::ZERO)
    } else {
        cam_positions
            .iter()
            .map(|&p| lods_for(p))
            .fold((0.0f32, 0.0f32), |(p0, f0), (p1, f1)| (p0.max(p1), f0.max(f1)))
    };

    // Gpu Fluid Processing
    let active_fluid = renderer
        .gpu_fluid
        .as_ref()
        .map(|f| (f.num_particles as f32 * fluid_lod) as u32)
        .unwrap_or(0);
    step_gpu_fluid(encoder, renderer, active_fluid);

    // Gpu Particles Processing
    if let Some(particles) = &renderer.gpu_particles {
        let active_parts = (particles.max_particles as f32 * particle_lod) as u32;
        let (dt, time) = world
            .get_resource::<gizmo_core::time::Time>()
            .map(|t| (t.dt(), t.elapsed() as f32))
            .unwrap_or((0.016, 0.0));
        particles.update_params(&renderer.queue, dt, time); // time → curl-noise evrimi
        // Fill it from the scene. Without this the engine steps and draws a particle set that
        // nothing ever puts anything into: the emitter-to-GPU bridge existed only inside
        // `gizmo-studio`, so a `ParticleEmitter` on an entity emitted nothing for anyone using
        // this pass. See `particles.rs`.
        spawn_from_emitters(world, particles, &renderer.queue, dt);
        particles.compute_pass(encoder, active_parts);
    }

    (particle_lod, fluid_lod)
}

/// A view that is not a camera entity: a reflection probe's cube face. Square, 90°, looking along
/// `forward` with `up`; drawn into the HDR target and stopped there — no post chain, no HUD.
#[derive(Clone, Copy)]
struct FixedView {
    position: Vec3,
    forward: Vec3,
    up: Vec3,
    near: f32,
    far: f32,
}

/// One camera's picture: from its matrices to the post chain, into `region` of `view` (the whole
/// of it when `None`), with the HUD when `overlay`. `camera` is an entity id; `None` draws from
/// the origin, as a frame with no camera always has. `lods` comes from [`step_gpu_simulations`].
/// With `capture`, that fixed view replaces the camera and the frame ends in the HDR target.
#[allow(clippy::too_many_arguments)]
fn draw_camera_view(
    world: &mut World,
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    renderer: &mut Renderer,
    camera: Option<u32>,
    lods: (f32, f32),
    region: Option<[f32; 4]>,
    overlay: bool,
    capture: Option<FixedView>,
) {
    // Post-process params are written AFTER the active camera is resolved (below), so the
    // single exposure knob can be the camera's exposure — see the update_post_process call
    // after camera selection. Exposure is applied ONCE here, over the whole composited HDR
    // (deferred geometry + sky + unlit), instead of being baked per-geometry in the
    // deferred pass and multiplied again by a separate global knob.

    // A split-screen view takes its own rectangle's shape; the whole target otherwise.
    let aspect = match region {
        Some([_, _, w, h]) => w / h,
        None if renderer.size.height > 0 => {
            renderer.size.width as f32 / renderer.size.height as f32
        }
        None => 1.0,
    };
    let mut proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, aspect, 0.1, 2000.0);
    let mut view_mat = Mat4::from_translation(Vec3::ZERO);
    let mut cam_pos = Vec3::ZERO;
    let mut cam_forward = Vec3::new(0.0, 0.0, -1.0);

    // TODO: Bütün nesnelerin (özellikle kamera ve çizilecek objelerin) global matrix'leri
    // bu pass çağrılmadan hemen önce bir `update_transforms(world)` sistemiyle güncellenmiş olmalıdır.

    let mut cam_exposure = 1.0;
    // Shadow cascades must follow the ACTIVE camera's near/far/fov, not hardcoded values
    // (otherwise splits/cascade matrices are wrong for any non-default camera).
    let mut cam_near = 0.1f32;
    let mut cam_far = 2000.0f32;
    let mut cam_fov = std::f32::consts::FRAC_PI_4;

    // The active camera's authored look, if it carries one. `None` means "graded by the
    // renderer's own fields", which is what every camera got before `PostProcess` existed and is
    // what a scene that has never been graded still gets. Read through `shared` so the editor's
    // viewport previews the grade by the same rule this frame applies it by.
    let grade = camera.and_then(|c| shared::camera_grade(world, c));

    // KAMERALARI BUL VE MATRIX YARAT
    let cameras = world.borrow::<Camera>();
    let global_transforms = world.borrow::<gizmo_physics_core::components::GlobalTransform>();
    let local_transforms = world.borrow::<gizmo_physics_core::components::Transform>();
    {
        // Pick the camera flagged `primary` — the convention maintained by
        // `spawn_camera`/`CameraBundle` (which keep a single primary) and used by
        // the audio listener. Fall back to the first camera if none is marked.
        // This makes selection deterministic instead of depending on the
        // (unstable) ECS iteration order.
        //
        // Through `components::active_camera` rather than written out here, because the editor's
        // environment panel and `shared::active_camera_grade` need the same answer and the panel
        // sits below this crate. A second copy of the rule is a second answer to "which camera is
        // the scene's", and this file has the history to show what that costs.
        let active_cam = camera;
        if let Some(active_cam) = active_cam {
            if let Some(cam) = cameras.get(active_cam) {
                // Camera world position: prefer a synced GlobalTransform (needed when the
                // camera is parented), but fall back to the camera's own Transform.position
                // when it has none. Without the fallback a hand-built camera that only got
                // a Transform + Camera (no GlobalTransform) was silently skipped and the
                // view stuck at the origin — nothing read the Transform that gameplay/WASD
                // moved. The transform-propagate system runs in the fixed-step schedule
                // BEFORE the user update, and a custom App may not register it at all, so
                // a camera's GlobalTransform is easily missing or a frame stale; the
                // Transform is written right before render and is always current.
                let pos = global_transforms
                    .get(active_cam)
                    .map(|g| g.matrix.to_scale_rotation_translation().2)
                    .or_else(|| local_transforms.get(active_cam).map(|t| t.position))
                    .unwrap_or(Vec3::ZERO);
                proj = cam.get_projection(aspect);
                view_mat = cam.get_view(pos);
                cam_pos = pos;
                cam_forward = cam.get_front();
                cam_exposure = cam.exposure;
                cam_near = cam.near;
                cam_far = cam.far;
                cam_fov = cam.fov;
            }
        }
    }
    if let Some(fixed) = capture {
        cam_fov = std::f32::consts::FRAC_PI_2;
        cam_near = fixed.near;
        cam_far = fixed.far;
        proj = Mat4::perspective_rh(cam_fov, 1.0, cam_near, cam_far);
        view_mat = Mat4::look_to_rh(fixed.position, fixed.forward, fixed.up);
        cam_pos = fixed.position;
        cam_forward = fixed.forward;
        cam_exposure = 1.0;
    }

    // ── Su-altı atmosferi: kamera bir fluid zone içindeyse derinlik-bazlı sis uygula (W3+W4).
    // W1 `water_at` sorgusu tekrar kullanılır (aynı su hacimleri hem buoyancy hem yüzme hem bu
    // sisi sürer). Sis rengi/yoğunluğu deniz için makul sabitler — demolarda tunable yapılabilir.
    // Sis rengi/yoğunluğu artık kameranın içinde bulunduğu FluidZone'dan gelir (her su hacmi
    // kendi su-altı görünümünü tanımlar) — eskiden burada sabitti.
    // Underwater fog comes from the FluidZone the camera sits in, which lives in the physics
    // world; without the `physics` feature there are no fluid volumes, so the scene simply
    // never renders as submerged.
    #[cfg(feature = "physics")]
    let underwater = world
        .get_resource::<crate::physics::world::PhysicsWorld>()
        .and_then(|pw| pw.water_at(cam_pos))
        .map(|s| crate::renderer::UnderwaterFog { color: s.fog_color, density: s.fog_density });
    #[cfg(not(feature = "physics"))]
    let underwater: Option<crate::renderer::UnderwaterFog> = None;


    // Save unjittered projection before applying TAA offset (needed for reprojection next frame).
    let unjittered_proj = proj;

    // ── TAA Halton jitter: subpixel offset applied to the projection ──
    if let Some(ref taa) = renderer.taa {
        if taa.enabled {
            let jp = crate::renderer::taa::TaaState::get_jitter(taa.frame_index);
            // Convert pixel jitter [−0.5, 0.5] to NDC offset (2 / viewport_size per axis)
            let jx = jp[0] * 2.0 / renderer.size.width as f32;
            let jy = jp[1] * 2.0 / renderer.size.height as f32;
            proj = jitter_projection(proj, jx, jy);
        }
    }

    let view_proj = proj * view_mat; // jittered — used for SceneUniforms
    let unjittered_view_proj = unjittered_proj * view_mat; // clean    — stored in TaaState for next frame

    // The active camera, in the one form both uniform blocks are built from. Assembled after the
    // TAA jitter so `view_proj` is the matrix this frame actually rasterises with.
    let camera = crate::renderer::CameraFrame {
        view_proj,
        position: cam_pos,
        forward: cam_forward,
        near: cam_near,
        far: cam_far,
        exposure: cam_exposure,
    };

    // Post-process params, now that the active camera (hence its exposure and depth range) is
    // known. `exposure` is the SINGLE exposure knob: the camera's exposure, applied once in the
    // post composite over the entire HDR. (Previously the deferred pass baked cam.exposure into
    // geometry AND post multiplied by a separate 1.15, which compounded and skipped sky/unlit;
    // folding both into one post-stage exposure fixes that.) Everything not named here is the
    // renderer's neutral default.
    // **The camera's own grade wins where it has one.** `PostProcess` is the authored look and
    // the only form of it a file can carry; the renderer's fields are the fallback for a camera
    // that carries none, which is every camera in every scene written before the component
    // existed. Exposure is not in the branch because it never moved: it has always come from
    // `Camera::exposure`, which is exactly why the component does not duplicate it.
    let post = match grade {
        Some(g) => crate::renderer::PostProcessUniforms {
            bloom_intensity: g.bloom_intensity,
            bloom_threshold: g.bloom_threshold,
            exposure: cam_exposure,
            chromatic_aberration: g.chromatic_aberration,
            vignette_intensity: g.vignette,
            film_grain_intensity: g.film_grain,
            dof_focus_dist: g.dof_focus_dist,
            dof_focus_range: g.dof_focus_range,
            // No `dof_enabled` gate here: a blur size of 0 IS the off state, which is why the
            // component's default carries 0 rather than the renderer's 4.0.
            dof_blur_size: g.dof_blur_size,
            motion_blur: g.motion_blur,
            ..Default::default()
        },
        None => crate::renderer::PostProcessUniforms {
            bloom_intensity: renderer.bloom_intensity,
            bloom_threshold: renderer.bloom_threshold,
            exposure: cam_exposure,
            chromatic_aberration: renderer.chromatic_aberration,
            // Was missing, and its absence was not visible: leaving it out let
            // `PostProcessUniforms::default()`'s 0.25 through, so an ungraded camera had a
            // vignette it never asked for and could not switch off.
            vignette_intensity: renderer.vignette_intensity,
            film_grain_intensity: renderer.film_grain_intensity,
            dof_focus_dist: renderer.dof_focus_dist,
            dof_focus_range: renderer.dof_focus_range,
            dof_blur_size: if renderer.dof_enabled { renderer.dof_blur_size } else { 0.0 },
            ..Default::default()
        },
    };
    // The curve is the renderer's in both branches: `PostProcess` is the authored *grade*, and a
    // grade that carried its own tone map would fight the camera it is composited with.
    let post = crate::renderer::PostProcessUniforms {
        tonemap_curve: renderer.tonemap_curve.as_f32(),
        tonemap_white_point: renderer.tonemap_white_point,
        ..post
    };
    // **Camera motion blur's one input**: where this frame's clip space was last frame. Both
    // matrices unjittered — TAA's sub-pixel jitter is not motion. The history is per view
    // (`Renderer::with_view_history` swaps it), and kept whether or not the blur is on, so turning
    // it on mid-game has a last frame to blur against.
    let reproject = match renderer.previous_view_proj {
        Some(previous) => Mat4::from_cols_array_2d(&previous) * unjittered_view_proj.inverse(),
        None => Mat4::IDENTITY,
    };
    // The G-buffer's velocity needs the same pair, before the history moves on.
    if let Some(def) = renderer.deferred.as_ref() {
        let now = unjittered_view_proj.to_cols_array_2d();
        def.motion.set_view_projs(&renderer.queue, now, renderer.previous_view_proj.unwrap_or(now));
    }
    renderer.previous_view_proj = Some(unjittered_view_proj.to_cols_array_2d());
    let post = crate::renderer::PostProcessUniforms {
        reproject: reproject.to_cols_array_2d(),
        ..post
    };
    renderer.update_post_process(
        &renderer.queue,
        post.with_camera(&camera).with_underwater(underwater),
    );

    // Elapsed time drives fluid caustics/wave animation in fluid_composite.wgsl
    // (it reads cascade_params.z); this slot was hardcoded to 0.0 → frozen water.
    let elapsed_time = world
        .get_resource::<gizmo_core::time::Time>()
        .map(|t| t.elapsed() as f32)
        .unwrap_or(0.0);

    // Lights, cascades and the whole scene block — via the shared setup helper, so the game and
    // studio renderers can only differ in what they pass it. The game always casts from the sun;
    // the editor's fallback to a point light is the other `ShadowCaster`.
    // Cookies a spot names by path are read from disk the first frame they are needed.
    #[cfg(not(target_arch = "wasm32"))]
    renderer.sync_light_cookies(world);
    let setup = collect_scene_setup(
        world,
        &SceneSetupInputs {
            camera,
            aspect,
            cam_fov,
            shadow_caster: ShadowCaster::SunOnly,
            environment: crate::renderer::EnvironmentFrame {
                preset: renderer.environment_preset,
                preset_2: renderer.environment_preset_2,
                blend_t: renderer.environment_blend_t,
                shading_mode: renderer.shading_mode,
            },
            point_shadows_enabled: renderer.point_shadows_enabled,
            elapsed_time,
            light_cookies: renderer.light_cookies.table(),
            shadow_settings: renderer.shadow_settings,
        },
    );
    let scene_lights = &setup.lights;
    let light_view_projs: [[[f32; 4]; 4]; 4] =
        setup.cascade_view_projs.map(|m| m.to_cols_array_2d());
    let lights_data = scene_lights.lights;

    #[allow(unused_assignments)]
    let mut point_light_view_projs = [gizmo_math::Mat4::IDENTITY; 6];
    // Build the point-shadow cube for the ONE designated caster (shared.rs picks the
    // first point light). Take its position/radius from the collected light array so the
    // CPU and the shader agree on which light owns the cube, and so a light with only a
    // Transform (no GlobalTransform) still casts — matching how it is lit.
    if renderer.point_shadows_enabled && scene_lights.shadow_point_index >= 0 {
        let idx = scene_lights.shadow_point_index as usize;
        let lp = lights_data[idx].position;
        let pos = gizmo_math::Vec3::new(lp[0], lp[1], lp[2]);
        // Far plane tracks the light radius (the shader decodes depth with the same far).
        let radius = lights_data[idx].color[3].max(1.0);
        let proj = gizmo_math::Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 1.0, 0.1, radius);
        point_light_view_projs = [
            proj * gizmo_math::Mat4::look_to_rh(pos, gizmo_math::Vec3::X, -gizmo_math::Vec3::Y),
            proj * gizmo_math::Mat4::look_to_rh(pos, gizmo_math::Vec3::NEG_X, -gizmo_math::Vec3::Y),
            proj * gizmo_math::Mat4::look_to_rh(pos, gizmo_math::Vec3::Y, gizmo_math::Vec3::Z),
            proj * gizmo_math::Mat4::look_to_rh(pos, gizmo_math::Vec3::NEG_Y, gizmo_math::Vec3::NEG_Z),
            proj * gizmo_math::Mat4::look_to_rh(pos, gizmo_math::Vec3::Z, -gizmo_math::Vec3::Y),
            proj * gizmo_math::Mat4::look_to_rh(pos, gizmo_math::Vec3::NEG_Z, -gizmo_math::Vec3::Y),
        ];

        for (i, view_proj) in point_light_view_projs.iter().enumerate() {
            renderer.queue.write_buffer(
                renderer.scene.view_point_shadow_buffer(i),
                0,
                bytemuck::bytes_of(&crate::renderer::gpu_types::ShadowVsUniform {
                    light_view_proj: view_proj.to_cols_array_2d(),
                }),
            );
        }
    }


    let scene_uniform_data = crate::renderer::SceneUniforms::new(&setup.frame);
    // Through the selector, not straight at `global_uniform_buffer`: with `active_view` set, the
    // passes below bind that view's group, so the camera has to land in that view's buffer.
    renderer.queue.write_buffer(
        renderer.scene.view_uniform_buffer(),
        0,
        bytemuck::cast_slice(&[scene_uniform_data]),
    );
    // The lighting shaders read their per-fragment light list from these buffers, so this upload is
    // not an optimisation — without it every cluster is empty and nothing is lit.
    renderer.scene.upload_clusters(&renderer.queue, &setup.clusters);
    for (i, light_view_proj) in light_view_projs.iter().enumerate() {
        renderer.queue.write_buffer(
            renderer.scene.view_shadow_cascade_buffer(i),
            0,
            bytemuck::bytes_of(&crate::renderer::gpu_types::ShadowVsUniform {
                light_view_proj: *light_view_proj,
            }),
        );
    }

    // Upload TAA params (prev_vp from last frame, current jitter, blend alpha)
    if let Some(ref mut taa) = renderer.taa {
        if taa.enabled {
            let jp = crate::renderer::taa::TaaState::get_jitter(taa.frame_index);
            let jx = jp[0] * 2.0 / renderer.size.width as f32;
            let jy = jp[1] * 2.0 / renderer.size.height as f32;
            let alpha = if taa.frame_index == 0 { 1.0f32 } else { 0.1f32 };
            taa.update_params(&renderer.queue, [jx, jy], alpha, cam_pos.to_array());
            taa.store_prev_vp(unjittered_view_proj.to_cols_array_2d());
        }
    }

    // Upload SSGI temporal-accumulation params (mirrors TAA: previous-frame unjittered
    // view-proj for reprojection + blend alpha). alpha=1.0 on the first frame / after a
    // reset so there is no stale history to reproject. Denoises the 1-spp raymarch grain.
    if let Some(ref mut ssgi) = renderer.ssgi {
        let alpha = if ssgi.frame_index == 0 { 1.0f32 } else { 0.1f32 };
        ssgi.update_params(&renderer.queue, alpha, cam_pos.to_array());
        ssgi.store_prev_vp(unjittered_view_proj.to_cols_array_2d());
    }

    // CPU batched instancing (replaces the GPU cull): walk the world, frustum-cull, group into
    // instanced batches and upload the instance buffer. Lives in `batching.rs`.
    let (draw_items, uploaded_instances) =
        batching::collect_draw_items(world, renderer, unjittered_view_proj, setup.cascade_view_projs, cam_pos);
    publish_render_stats(world, renderer, &draw_items, elapsed_time);

    // The GPU-physics cull reads the view's camera, so it is per view; its simulation is not.
    if let Some(physics) = &renderer.gpu_physics {
        physics.cull_pass(encoder, renderer.scene.view_bind_group());
    }
    let (particle_lod, fluid_lod) = lods;

    // GPU cull pass removed since we use CPU instancing

    // Resize deferred G-buffers if window changed; resize SSAO + TAA to match
    if let Some(ref mut def) = renderer.deferred {
        def.resize(&renderer.device, renderer.size.width, renderer.size.height);
    }
    {
        let w = renderer.size.width;
        let h = renderer.size.height;
        if let (Some(ssao), Some(def)) = (&mut renderer.ssao, &renderer.deferred) {
            if ssao.width != w || ssao.height != h {
                ssao.resize(&renderer.device, def, w, h);
            }
        }
        if let (Some(ssr), Some(def)) = (&mut renderer.ssr, &renderer.deferred) {
            if ssr.width != w || ssr.height != h {
                ssr.resize(&renderer.device, def, &renderer.post.hdr_texture_view, w, h);
            }
        }
        if let (Some(volumetric), Some(def)) = (&mut renderer.volumetric, &renderer.deferred) {
            if volumetric.width != w || volumetric.height != h {
                volumetric.resize(&renderer.device, def, w, h);
            }
        }
    }
    {
        let w = renderer.size.width;
        let h = renderer.size.height;
        if let (Some(taa), Some(def)) = (&mut renderer.taa, &renderer.deferred) {
            if taa.width != w || taa.height != h {
                taa.resize(
                    &renderer.device,
                    &renderer.post.hdr_texture_view,
                    &def.world_position_view,
                    w,
                    h,
                );
            }
        }
    }

    // Web şemasında gölge yok (4-grup limiti, forward shader'dan shadow örneklemesi
    // `load_shader_web` ile sökülür) — depth-only CSM/point geçitleri boşa GPU olur.
    #[cfg(not(target_arch = "wasm32"))]
    passes::record_shadow_passes(encoder, renderer, &draw_items, uploaded_instances);
    passes::record_deferred_geometry(
        encoder,
        renderer,
        world,
        &draw_items,
        uploaded_instances,
        cam_pos,
    );
    passes::record_ssao(encoder, renderer);
    // CPU-computed inverse of the (unjittered) view-projection for the volumetric smoke raymarch
    // (the WGSL inverse_mat4 returns a wrong inverse for the perspective matrix).
    let inv_view_proj = unjittered_view_proj.inverse().to_cols_array_2d();
    passes::record_forward_and_fluid(
        encoder, renderer, world, &draw_items, uploaded_instances, particle_lod, fluid_lod,
        inv_view_proj,
    );
    passes::record_screen_space_effects(encoder, renderer);
    // Advance SSGI temporal ping-pong / frame counter after its passes have run.
    if let Some(ref mut ssgi) = renderer.ssgi {
        ssgi.advance_frame();
    }
    passes::record_taa_and_overlays(encoder, renderer, world);
    // After the overlays and before post-processing: sprites and text are overlays too, and the
    // ones whose smear a temporal resolve makes most obvious. Sprites first, so a HUD label lands
    // on its HUD image; with the UNJITTERED matrix, so a sprite drawn past TAA does not shimmer.
    record_sprites(encoder, renderer, world, unjittered_view_proj);
    record_text(encoder, renderer, world);

    // A probe face stops at the HDR target: what it captures is radiance, not a graded picture.
    if capture.is_some() {
        return;
    }
    // Auto exposure adapts in real time: a paused or slowed game still lets the eye adjust.
    let exposure_dt = world
        .get_resource::<gizmo_core::time::Time>()
        .map(|t| t.raw_dt())
        .unwrap_or(1.0 / 60.0);
    renderer.update_auto_exposure(exposure_dt);
    renderer.run_post_processing_in(encoder, view, region, overlay);
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

pub mod visibility_index;
mod passes;

mod decals;
pub use decals::{collect_decals, record_forward_decals};

mod particles;
pub use particles::spawn_from_emitters;

mod sprite;
pub use sprite::record_sprites;

mod text;
pub use text::record_text;

mod hidden;
pub use hidden::collect_hidden;

mod shared;
pub use shared::{
    active_camera_grade, collect_scene_lights, collect_scene_setup, SceneLights, SceneSetup,
    SceneSetupInputs, ShadowCaster,
};

#[cfg(test)]
mod taa_jitter_tests {
    //! What [`jitter_projection`] has to be true of: the sub-pixel offset must land as the SAME
    //! NDC shift at every depth. It did not for an orthographic camera, and the symptom was a
    //! static scene that shimmered — see the function's own header for the measurement.

    use super::jitter_projection;
    use crate::math::{Mat4, Vec3, Vec4};

    /// Where a view-space point ends up in NDC, under `proj`.
    fn ndc(proj: Mat4, view_point: Vec3) -> (f32, f32) {
        let clip = proj * Vec4::new(view_point.x, view_point.y, view_point.z, 1.0);
        (clip.x / clip.w, clip.y / clip.w)
    }

    /// The jitter is a *sub-pixel* offset, so it must be the same shift at 2 m and at 200 m.
    /// Depth-dependence is precisely the defect: on an orthographic camera the offset used to
    /// scale with view depth, which is a several-pixel wobble rather than a sub-pixel one.
    #[test]
    fn orthographic_jitter_is_the_same_shift_at_every_depth() {
        let proj = Mat4::orthographic_rh(-4.0, 4.0, -3.0, 3.0, 0.1, 500.0);
        let (jx, jy) = (0.0011, -0.0007);
        let jittered = jitter_projection(proj, jx, jy);

        for depth in [-2.0f32, -20.0, -200.0] {
            let p = Vec3::new(1.0, -0.5, depth);
            let (x0, y0) = ndc(proj, p);
            let (x1, y1) = ndc(jittered, p);
            assert!(
                (x1 - x0 - jx).abs() < 1e-6 && (y1 - y0 - jy).abs() < 1e-6,
                "ortografik jitter {depth} m derinlikte kaydı: Δ=({}, {}) beklenen ({jx}, {jy})",
                x1 - x0,
                y1 - y0
            );
        }
    }

    /// The perspective path is the one that already worked; it is pinned so the fix cannot
    /// have moved it.
    #[test]
    fn perspective_jitter_is_the_same_shift_at_every_depth() {
        let proj = Mat4::perspective_rh(1.0, 16.0 / 9.0, 0.1, 500.0);
        let (jx, jy) = (0.0011, -0.0007);
        let jittered = jitter_projection(proj, jx, jy);

        for depth in [-2.0f32, -20.0, -200.0] {
            let p = Vec3::new(1.0, -0.5, depth);
            let (x0, y0) = ndc(proj, p);
            let (x1, y1) = ndc(jittered, p);
            assert!(
                (x1 - x0 - jx).abs() < 1e-6 && (y1 - y0 - jy).abs() < 1e-6,
                "perspektif jitter {depth} m derinlikte kaydı: Δ=({}, {})",
                x1 - x0,
                y1 - y0
            );
        }
    }

    /// Zero jitter must leave the matrix alone, whichever branch it takes — a frame with TAA
    /// disabled and one with a zero offset have to rasterise identically.
    #[test]
    fn zero_jitter_changes_nothing() {
        for proj in [
            Mat4::orthographic_rh(-4.0, 4.0, -3.0, 3.0, 0.1, 500.0),
            Mat4::perspective_rh(1.0, 16.0 / 9.0, 0.1, 500.0),
        ] {
            assert_eq!(jitter_projection(proj, 0.0, 0.0), proj);
        }
    }
}

/// Golden render test: drive the REAL [`default_render_pass`] over a minimal scene
/// (one lit cube + a camera + a sun) into an offscreen target and assert that geometry
/// actually reaches the framebuffer — a sizeable central region must differ from the
/// background. Unlike the renderer's clear-colour readback test, this exercises the full
/// pipeline (cull → batch → shadow/deferred/forward → post), so a regression in the
/// pass-recording split (or any pass) that drops geometry fails here instead of slipping
/// past CI. Needs a GPU adapter; runs in GPU-backed CI/dev.
#[cfg(test)]
mod golden_render_tests {
    use super::default_render_pass;

    use crate::bundles::{CameraBundle, DirectionalLightBundle};
    use crate::core::World;
    use crate::math::{Vec3, Vec4};
    use crate::physics::components::{GlobalTransform, Transform};
    use crate::renderer::asset::AssetManager;
    use crate::renderer::components::{Material, MeshRenderer};
    use crate::renderer::Renderer;

    /// The same scene renders the same whether it sits at the world origin or two kilometres away.
    ///
    /// It did not. The G-buffer's world-position target is `Rgba16Float`, and it held **absolute**
    /// coordinates: f16 quantises to 6 cm at 100 m from the origin, 50 cm at 1 km and a full metre
    /// at 2 km, against a nearest-cascade shadow texel of 4.3 mm. A city-sized level was therefore
    /// sampling its shadows, its view vector and its fog from a position rounded to the nearest
    /// half-metre — invisible near the origin, which is where every other test in this file sits,
    /// and worse the further out you built.
    ///
    /// The target cannot be widened: the four G-buffer attachments share a 32-byte-per-sample
    /// budget and are at 28. But the budget is about the bytes, not about what goes in them, so
    /// the position is stored relative to the camera now and every reader adds it back. Same eight
    /// bytes, values at view scale, centimetres anywhere in the world.
    ///
    /// Two kilometres is chosen to be where f16 costs a whole metre. The tolerance is loose on
    /// purpose — this asks whether the picture *survives the translation*, not whether two GPU
    /// runs are bit-identical.
    #[test]
    fn a_scene_renders_the_same_two_kilometres_from_the_origin() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available()) {
            eprintln!("skipping a_scene_renders_the_same_two_kilometres_from_the_origin: no GPU");
            return;
        }
        if pollster::block_on(Renderer::headless_adapter_is_software()) {
            eprintln!("skipping a_scene_renders_the_same_two_kilometres_from_the_origin: software");
            return;
        }
        pollster::block_on(async {
            let near = render_translated(Vec3::ZERO).await;
            let far = render_translated(Vec3::new(2000.0, 0.0, 2000.0)).await;
            assert_eq!(near.len(), far.len(), "same target size");

            let differing = near
                .iter()
                .zip(far.iter())
                .filter(|(a, b)| a.abs_diff(**b) > 8)
                .count();
            let ratio = differing as f32 / near.len() as f32;
            assert!(
                ratio < 0.02,
                "{:.1}% of the frame changed when the whole scene moved 2 km from the origin — \
                 the renderer is reading positions whose precision depends on where the level was \
                 built",
                100.0 * ratio
            );
        });
    }

    /// The cube scene of [`render_frame_with_mesh`], with everything — camera, cube and all —
    /// shifted by `offset`. Shifting *both* is what makes the comparison about world-origin
    /// distance and nothing else: the camera sees exactly the same thing either way.
    async fn render_translated(offset: Vec3) -> Vec<u8> {
        const W: u32 = 128;
        const H: u32 = 128;
        let mut renderer = crate::test_gpu::headless_renderer(W, H).await;
        let mut asset_manager = AssetManager::new();
        let mut world = World::new();

        let mesh = AssetManager::create_cube(&renderer.device);
        let tex = asset_manager.create_white_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
        );
        let mat = Material::new(tex).with_pbr(Vec4::new(0.9, 0.15, 0.15, 1.0), 0.0, 1.0);
        let cube = world.spawn();
        world.add_component(cube, Transform::new(offset));
        world.add_component(cube, mesh);
        world.add_component(cube, mat);
        world.add_component(cube, MeshRenderer::new());

        world.spawn_bundle(CameraBundle {
            position: Vec3::new(-6.0, 0.0, 0.0) + offset,
            yaw: 0.0,
            pitch: 0.0,
            primary: true,
            ..Default::default()
        });
        world.spawn_bundle(DirectionalLightBundle::default());

        render_world(&mut renderer, &mut world).await
    }

    /// Two different dark materials must not render identically.
    ///
    /// They did. The G-buffer's albedo target was `Rgba8Unorm` holding **linear** albedo, and
    /// linear 8-bit has almost no resolution where the eye has most: the whole perceptual range
    /// 0–32/255 gets **4 codes** in a linear target against 32 in an sRGB one, and the very first
    /// linear code already sits at a perceptual 12.7/255. Albedo 0.004 and 0.0045 both landed on
    /// code 1 and came out **byte-identical**, which is what this test measured before the format
    /// changed: `0 bytes differ`. It matters here more than it would in most engines — this one's
    /// flagship level is a night city whose frame medians sit at 1–14/255, i.e. entirely inside
    /// the range those four codes have to cover.
    ///
    /// The obvious version of this test does not work, and it is worth saying why: comparing two
    /// albedos the linear format *can* separate (0.004 and 0.006) makes the **linear** target look
    /// better, 2923 differing bytes against sRGB's 1649. That is not signal. Linear rounds 0.004
    /// down and 0.006 up, so it renders them two codes apart when they are one and a half apart —
    /// the extra difference is quantisation error, not detail. Only a pair the old format could
    /// not separate at all distinguishes precision from noise.
    #[test]
    fn two_different_dark_materials_do_not_render_identically() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available()) {
            eprintln!("skipping two_different_dark_materials_do_not_render_identically: no GPU");
            return;
        }
        if pollster::block_on(Renderer::headless_adapter_is_software()) {
            eprintln!("skipping two_different_dark_materials_do_not_render_identically: software");
            return;
        }
        pollster::block_on(async {
            let a = render_dark(0.004).await;
            let b = render_dark(0.0045).await;
            let differing = a.iter().zip(b.iter()).filter(|(x, y)| x != y).count();
            assert!(
                differing > 0,
                "albedo 0.004 and 0.0045 rendered byte-identically — the G-buffer cannot hold the \
                 difference, so every dark material in this range is the same material"
            );
        });
    }

    async fn render_dark(albedo: f32) -> Vec<u8> {
        const W: u32 = 128;
        const H: u32 = 128;
        let mut renderer = crate::test_gpu::headless_renderer(W, H).await;
        let mut asset_manager = AssetManager::new();
        let mut world = World::new();
        let mesh = AssetManager::create_cube(&renderer.device);
        let tex = asset_manager.create_white_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
        );
        let mat = Material::new(tex).with_pbr(Vec4::new(albedo, albedo, albedo, 1.0), 0.0, 1.0);
        let cube = world.spawn();
        world.add_component(cube, Transform::new(Vec3::ZERO));
        world.add_component(cube, mesh);
        world.add_component(cube, mat);
        world.add_component(cube, MeshRenderer::new());
        world.spawn_bundle(CameraBundle {
            position: Vec3::new(-6.0, 0.0, 0.0),
            primary: true,
            ..Default::default()
        });
        world.spawn_bundle(DirectionalLightBundle::default());
        render_world(&mut renderer, &mut world).await
    }

    /// A still camera on a still scene shimmers no more than the jitter sequence itself explains.
    ///
    /// This is what temporal anti-aliasing is *for*, so it is worth asserting rather than assuming.
    /// It caught a live regression: making the world-position G-buffer camera-relative broke TAA's
    /// reprojection, because that shader binds the same target under the name `t_position` and was
    /// not among the readers found by grepping for `t_world_position`. The history was then sampled
    /// from wherever the camera happened to be relative to the origin, so it never matched and the
    /// neighbourhood clamp dragged it back to the jittered current frame every frame. Measured on
    /// this scene: **4 918 bytes moving frame to frame, peaking at 33/255**, against 1 785 and 9
    /// once the reprojection was corrected — and zero with TAA switched off entirely, which is what
    /// established that TAA was the sole source rather than SSAO, SSGI or SSR.
    ///
    /// **The residual is not zero and this test does not pretend otherwise.** Even with the
    /// reprojection correct, the peak inter-frame swing on this scene is **18/255** — an
    /// eight-frame jitter sequence resolved through a hard min/max neighbourhood clamp cycles
    /// rather than converging, because the clamp keeps binding on edge pixels. That is a real
    /// open question and not what this guards; the bar is set to separate a working resolve
    /// (18) from a broken one (33–35), not to claim convergence.
    ///
    /// Variance clipping — mean ± σ instead of the min/max box — was the obvious candidate for
    /// the residual and is **refuted**: measured twice, once against the broken reprojection and
    /// once against the fixed one, it made the shimmer slightly *worse* (1 060 bytes against 942
    /// with the blend on pure history). The box stays.
    #[test]
    fn a_still_scene_shimmers_no_worse_than_its_own_jitter() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available()) {
            eprintln!("skipping a_still_camera_on_a_still_scene_settles: no GPU adapter");
            return;
        }
        if pollster::block_on(Renderer::headless_adapter_is_software()) {
            eprintln!("skipping a_still_camera_on_a_still_scene_settles: software adapter");
            return;
        }
        pollster::block_on(async {
            let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
            let mut asset_manager = AssetManager::new();
            let mut world = World::new();
            let mesh = AssetManager::create_cube(&renderer.device);
            let tex = asset_manager.create_white_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            );
            let mat = Material::new(tex).with_pbr(Vec4::new(0.9, 0.15, 0.15, 1.0), 0.0, 1.0);
            let cube = world.spawn();
            world.add_component(cube, Transform::new(Vec3::ZERO));
            world.add_component(cube, mesh);
            world.add_component(cube, mat);
            world.add_component(cube, MeshRenderer::new());
            world.spawn_bundle(CameraBundle {
                position: Vec3::new(-6.0, 0.0, 0.0),
                primary: true,
                ..Default::default()
            });
            world.spawn_bundle(DirectionalLightBundle::default());

            // Warm up first: the opening frames are the history filling and say nothing about
            // whether the resolve settles. Eight is past the jitter sequence's own period.
            let mut prev = render_world(&mut renderer, &mut world).await;
            for _ in 0..8 {
                prev = render_world(&mut renderer, &mut world).await;
            }
            let mut worst = 0u8;
            for _ in 0..6 {
                let cur = render_world(&mut renderer, &mut world).await;
                worst = prev
                    .iter()
                    .zip(cur.iter())
                    .map(|(a, b)| a.abs_diff(*b))
                    .max()
                    .unwrap_or(0)
                    .max(worst);
                prev = cur;
            }
            assert!(
                worst <= 24,
                "nothing moved and the picture swung by {worst}/255 between frames — at this size \
                 the temporal resolve is not resolving, it is replaying its own jitter (a broken \
                 reprojection measured 33–35 here, a correct one 18, and TAA switched off 0)"
            );
        });
    }

    /// The same probe, with an **orthographic** camera — the projection the perspective one
    /// cannot speak for.
    ///
    /// Its sibling above spawns a `CameraBundle`, which is perspective, so for as long as it has
    /// existed the jitter path was only ever tested through a matrix that divides by `w`. It does
    /// not, for an orthographic one, and [`jitter_projection`] is where that is now decided:
    /// writing the sub-pixel offset into the z column makes it scale with view depth when there
    /// is no perspective divide to undo it.
    ///
    /// Measured 2026-08-22 at 128×128 on this scene: **44/255** with the offset in
    /// the z column and **33/255** with it in the translation column, against the
    /// perspective probe's 18. In a demo-sized window it was worse still — `orthographic`
    /// swung 115/255 and 2.1 % of its pixels moved every frame on a scene whose transforms were
    /// bit-identical, which is what a person sees as shimmer; fixed, that window settles into a
    /// clean eight-frame cycle (frames 8 apart come out **byte-identical**) and the residual is
    /// the intended sub-pixel jitter.
    ///
    /// The bar is 38: between the two measurements, like its sibling's 24, and for the same
    /// reason — it separates a working resolve from a broken one rather than claiming
    /// convergence. The 33 it passes at is *not* small, and the open question the sibling
    /// records (a min/max clamp that cycles instead of settling) is the same one here.
    #[test]
    fn a_still_orthographic_scene_shimmers_no_worse_than_its_own_jitter() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available()) {
            eprintln!("skipping the orthographic shimmer probe: no GPU adapter");
            return;
        }
        if pollster::block_on(Renderer::headless_adapter_is_software()) {
            eprintln!("skipping the orthographic shimmer probe: software adapter");
            return;
        }
        pollster::block_on(async {
            let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
            let mut asset_manager = AssetManager::new();
            let mut world = World::new();
            let mesh = AssetManager::create_cube(&renderer.device);
            let tex = asset_manager.create_white_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            );
            let mat = Material::new(tex).with_pbr(Vec4::new(0.9, 0.15, 0.15, 1.0), 0.0, 1.0);
            let cube = world.spawn();
            world.add_component(cube, Transform::new(Vec3::ZERO));
            world.add_component(cube, mesh);
            world.add_component(cube, mat);
            world.add_component(cube, MeshRenderer::new());
            // Six units out, matching the perspective probe exactly so the two numbers can be
            // read against each other. Standing further back was tried and is worse, not better:
            // at 30 units the fixed resolve reads 45 rather than 33, because the camera-relative
            // position buffer the reprojection samples loses precision faster than the defect
            // grows. The defect's own depth scaling is measured where it matters instead — in a
            // demo-sized window, below.
            let camera = world.spawn_bundle(CameraBundle {
                position: Vec3::new(-6.0, 0.0, 0.0),
                primary: true,
                ..Default::default()
            });
            // `CameraBundle` carries no projection field, so the mode is set on the component the
            // bundle just wrote — the same three lines every orthographic demo needs.
            let mut cameras = world.borrow_mut::<crate::renderer::components::Camera>();
            if let Some(mut cam) = cameras.get_mut(camera.id()) {
                cam.projection =
                    crate::renderer::components::ProjectionMode::Orthographic { height: 6.0 };
            }
            drop(cameras);
            world.spawn_bundle(DirectionalLightBundle::default());

            let mut prev = render_world(&mut renderer, &mut world).await;
            for _ in 0..8 {
                prev = render_world(&mut renderer, &mut world).await;
            }
            let mut worst = 0u8;
            for _ in 0..6 {
                let cur = render_world(&mut renderer, &mut world).await;
                worst = prev
                    .iter()
                    .zip(cur.iter())
                    .map(|(a, b)| a.abs_diff(*b))
                    .max()
                    .unwrap_or(0)
                    .max(worst);
                prev = cur;
            }
            assert!(
                worst <= 38,
                "an orthographic camera on a still scene swung by {worst}/255 between frames —                  the jitter is being written into a column this projection does not divide away,                  so a sub-pixel offset became a depth-scaled one (see `jitter_projection`)"
            );
        });
    }

    /// Every pipeline this engine builds compiles on whatever backend is present.
    ///
    /// **Deliberately has no software-adapter guard, unlike every other test in this module.**
    /// That is the whole point of it. A backend compiles naga's generated target language with
    /// its own compiler — FXC on D3D12, the Metal compiler on macOS — and those reject things
    /// that are perfectly good WGSL. On 2026-08-14 the shadow PCF's `textureSampleCompare`, an
    /// implicit-derivative sample inside a conditional branch, was "gradient instruction used in
    /// a loop with varying iteration" to FXC, and the Deferred Lighting pipeline never built: the
    /// engine drew nothing at all on Windows. `gizmo-renderer`'s shader tests had been green
    /// throughout, because they type-check WGSL through naga and never reach a backend compiler.
    ///
    /// The only thing that catches this is creating the pipelines on that backend, and creating
    /// them is fast even on WARP — it is *rendering* through them that a software rasteriser
    /// cannot finish, which is why the rest of this module skips and this does not.
    #[test]
    fn every_pipeline_compiles_on_this_backend() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available()) {
            eprintln!("skipping every_pipeline_compiles_on_this_backend: no GPU adapter");
            return;
        }
        pollster::block_on(async {
            // Constructing the renderer is what builds them: deferred, gbuffer, both shadow
            // passes, forward, post-process, ssao/ssr/ssgi, the particle and fluid compute
            // pipelines. A backend rejecting any one of them fails here.
            let renderer = crate::test_gpu::headless_renderer(64, 64).await;
            assert!(
                renderer.surface.is_none(),
                "the headless renderer must have no surface"
            );
        });
    }

    /// The pass advances skeletal animation.
    ///
    /// It did not until 2026-08-14, and nothing noticed for a long time. `animation_update_system`
    /// and `animation_state_machine_update_system` were written, exported and documented as the
    /// thing that steps a player's clock, and no schedule, plugin, app or demo ever called either
    /// of them — `current_time += dt · speed` appears nowhere else in the workspace, so a skinned
    /// mesh rendered its bind pose for ever. The draw path *reads* `Skeleton` for its skinning
    /// matrices, which is what made the omission invisible: everything looked wired.
    ///
    /// So this asserts the wiring rather than the arithmetic. The arithmetic already has tests
    /// (`gizmo-renderer`'s `normalize_anim_time` covers looping, clamping and zero duration) and
    /// they all passed the entire time the feature was dead.
    #[test]
    fn default_render_pass_advances_skeletal_animation() {
        use crate::renderer::components::{AnimationClip, AnimationPlayer, SkeletonHierarchy};
        use gizmo_animation::skeletal::SkeletonJoint;
        use std::sync::Arc;

        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available()) {
            eprintln!(
                "skipping default_render_pass_advances_skeletal_animation: no GPU adapter"
            );
            return;
        }
        // WARP and friends can create these pipelines in seconds and cannot finish
        // rendering through them: `windows-latest` spent five and a half hours on this
        // file once the D3D12 shader fix let it get this far. Pipeline compilation is
        // the coverage that matters on such a runner and it is kept by
        // `every_pipeline_compiles_on_this_backend`, which builds the whole renderer.
        if pollster::block_on(Renderer::headless_adapter_is_software()) {
            eprintln!("skipping default_render_pass_advances_skeletal_animation: software adapter — see the note above");
            return;
        }
        pollster::block_on(async {
            let mut renderer = crate::test_gpu::headless_renderer(64, 64).await;
            let mut world = World::new();

            // A clock with a real delta. The pass reads `Time::dt()`, and a world without the
            // resource reads 0.0 and advances nothing — which is correct, and would also make
            // this test pass for the wrong reason if the resource were left out.
            let mut time = crate::core::time::Time::new();
            time.update(1.0 / 60.0);
            world.insert_resource(time);

            // One joint, one clip, one second long, empty tracks: the pose it evaluates to does
            // not matter here, only that the clock moves.
            let hierarchy = Arc::new(SkeletonHierarchy {
                joints: vec![SkeletonJoint {
                    name: "root".into(),
                    node_index: 0,
                    inverse_bind_matrix: crate::math::Mat4::IDENTITY,
                    parent_index: None,
                    local_bind_transform: crate::math::Mat4::IDENTITY,
                    bind_translation: Vec3::ZERO,
                    bind_rotation: crate::math::Quat::IDENTITY,
                    bind_scale: Vec3::ONE,
                }],
                root_transform: crate::math::Mat4::IDENTITY,
            });
            let clip = AnimationClip {
                name: "idle".into(),
                duration: 1.0,
                translations: Vec::new(),
                rotations: Vec::new(),
                scales: Vec::new(),
            };
            let rig = world.spawn();
            world.add_component(rig, Transform::new(Vec3::ZERO));
            world.add_component(rig, renderer.create_skeleton(hierarchy));
            world.add_component(
                rig,
                AnimationPlayer {
                    current_time: 0.0,
                    active_animation: 0,
                    loop_anim: true,
                    speed: 1.0,
                    animations: Arc::from(vec![clip]),
                    blend_time: 0.0,
                    blend_duration: 0.0,
                    prev_animation: None,
                    prev_time: 0.0,
                },
            );

            world.spawn_bundle(CameraBundle {
                position: Vec3::new(-6.0, 0.0, 0.0),
                primary: true,
                ..Default::default()
            });

            let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("anim_target"),
                size: wgpu::Extent3d {
                    width: 64,
                    height: 64,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: renderer.config.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = target.create_view(&wgpu::TextureViewDescriptor::default());
            let mut encoder = renderer
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

            default_render_pass(&mut world, &mut encoder, &view, &mut renderer);
            renderer.queue.submit(Some(encoder.finish()));

            let advanced = world
                .borrow::<AnimationPlayer>()
                .get(rig.id())
                .expect("the player is still there")
                .current_time;
            assert!(
                advanced > 0.0,
                "default_render_pass left current_time at {advanced} — nothing is advancing \
                 skeletal animation, which is the state the engine shipped in until this test"
            );
        });
    }

    #[test]
    fn default_render_pass_draws_a_cube_distinct_from_background() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available()) {
            eprintln!(
                "skipping default_render_pass_draws_a_cube_distinct_from_background: \
                 no GPU adapter available (headless render requires a GPU)"
            );
            return;
        }
        // WARP and friends can create these pipelines in seconds and cannot finish
        // rendering through them: `windows-latest` spent five and a half hours on this
        // file once the D3D12 shader fix let it get this far. Pipeline compilation is
        // the coverage that matters on such a runner and it is kept by
        // `every_pipeline_compiles_on_this_backend`, which builds the whole renderer.
        if pollster::block_on(Renderer::headless_adapter_is_software()) {
            eprintln!("skipping default_render_pass_draws_a_cube_distinct_from_background: software adapter — see the note above");
            return;
        }
        pollster::block_on(async {
            const W: u32 = 128;
            const H: u32 = 128;
            const BPP: u32 = 4; // every surface format used here is 4 bytes/pixel

            let mut renderer = crate::test_gpu::headless_renderer(W, H).await;
            let mut asset_manager = AssetManager::new();
            let mut world = World::new();

            // --- one cube at the origin (create_cube spans -1..1 → size 2) ---
            let mesh = AssetManager::create_cube(&renderer.device);
            let tex = asset_manager.create_white_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            );
            let mat = Material::new(tex).with_pbr(Vec4::new(0.9, 0.15, 0.15, 1.0), 0.0, 1.0);
            // Deliberately NO GlobalTransform: `default_render_pass` must backfill and
            // sync it from the Transform (the "spawn Transform+Mesh+Material and it just
            // renders" contract — regression guard for the empty-screen footgun).
            let cube = world.spawn();
            world.add_component(cube, Transform::new(Vec3::ZERO));
            world.add_component(cube, mesh);
            world.add_component(cube, mat);
            world.add_component(cube, MeshRenderer::new());

            // --- camera on -X looking toward +X (yaw 0 → front = +X), framing the cube ---
            world.spawn_bundle(CameraBundle {
                position: Vec3::new(-6.0, 0.0, 0.0),
                yaw: 0.0,
                pitch: 0.0,
                primary: true,
                ..Default::default()
            });
            // --- a sun so the cube is lit (role = Sun by default) ---
            world.spawn_bundle(DirectionalLightBundle::default());

            // --- run the REAL pipeline into an offscreen target ---
            let format = renderer.config.format;
            let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("golden-target"),
                size: wgpu::Extent3d {
                    width: W,
                    height: H,
                    depth_or_array_layers: 1,
                },
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

            default_render_pass(&mut world, &mut encoder, &view, &mut renderer);

            // --- copy the result out (W*BPP = 512 → already 256-aligned) ---
            let staging = renderer.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("golden-readback"),
                size: (W * H * BPP) as u64,
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
                wgpu::Extent3d {
                    width: W,
                    height: H,
                    depth_or_array_layers: 1,
                },
            );
            renderer.queue.submit(Some(encoder.finish()));

            let slice = staging.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |v| tx.send(v).unwrap());
            let _ = renderer.device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            });
            rx.recv().unwrap().unwrap();
            let data = slice.get_mapped_range()
                // wgpu 30 made this fallible; the range is the whole buffer we just mapped, so a
                // failure here is a programming error rather than a runtime condition.
                .expect("a just-mapped buffer's full range is always valid");

            let px = |x: u32, y: u32| -> [u8; 4] {
                let i = ((y * W + x) * BPP) as usize;
                [data[i], data[i + 1], data[i + 2], data[i + 3]]
            };
            let background = px(2, 2); // a corner — the cube never reaches here
            let centre = px(W / 2, H / 2);
            assert_ne!(
                centre, background,
                "centre pixel equals the corner/background — default_render_pass drew no geometry"
            );

            // the cube should cover a sizeable central region, not a stray pixel
            let mut differing = 0u32;
            for y in 0..H {
                for x in 0..W {
                    if px(x, y) != background {
                        differing += 1;
                    }
                }
            }
            let frac = differing as f32 / (W * H) as f32;
            assert!(
                frac > 0.05,
                "only {:.1}% of pixels differ from the background; the lit cube should fill a \
                 sizeable central region (regression dropping geometry?)",
                frac * 100.0
            );
        });
    }

    /// Render the standard lit cube through the real pipeline and return the frame's bytes.
    ///
    /// `point_shadows` sets [`Renderer::point_shadows_enabled`], which gates both the six
    /// point-shadow face passes and the shader's lookup into the cubemap they write.
    async fn render_frame(exposure: f32, point_shadows: bool) -> Vec<u8> {
        render_frame_with_mesh(exposure, point_shadows, AssetManager::create_cube).await
    }

    /// As [`render_frame`], but the caller supplies the geometry.
    ///
    /// The seam exists so two frames can differ **only** in how the same triangles reach the
    /// GPU — flat vertex list versus deduplicated vertices plus an index buffer. Everything
    /// downstream (camera, light, material, passes, readback) stays byte-for-byte the same
    /// code path, which is what makes an equality assertion on the two frames meaningful.
    async fn render_frame_with_mesh(
        exposure: f32,
        point_shadows: bool,
        make_mesh: impl FnOnce(&wgpu::Device) -> crate::renderer::components::Mesh,
    ) -> Vec<u8> {
        // The offscreen target `render_world` renders into is the same size; these size the
        // renderer itself.
        const W: u32 = 128;
        const H: u32 = 128;

        let mut renderer = crate::test_gpu::headless_renderer(W, H).await;
        renderer.point_shadows_enabled = point_shadows;
        let mut asset_manager = AssetManager::new();
        let mut world = World::new();

        let mesh = make_mesh(&renderer.device);
        let tex = asset_manager.create_white_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
        );
        let mat = Material::new(tex).with_pbr(Vec4::new(0.9, 0.15, 0.15, 1.0), 0.0, 1.0);
        let cube = world.spawn();
        world.add_component(cube, Transform::new(Vec3::ZERO));
        world.add_component(cube, GlobalTransform::default());
        world.add_component(cube, mesh);
        world.add_component(cube, mat);
        world.add_component(cube, MeshRenderer::new());

        world.spawn_bundle(CameraBundle {
            position: Vec3::new(-6.0, 0.0, 0.0),
            yaw: 0.0,
            pitch: 0.0,
            primary: true,
            exposure,
            ..Default::default()
        });
        world.spawn_bundle(DirectionalLightBundle::default());

        render_world(&mut renderer, &mut world).await
    }

    /// Two overlapping transparent surfaces of the SAME material blend the same way whichever
    /// order they were spawned in.
    ///
    /// They did not. The transparent pipeline writes no depth, so for blended geometry the draw
    /// order *is* the result — and while this path sorted transparent **batches** back-to-front, it
    /// appended each batch's instances in collection order. Two panes of one material are one
    /// batch, so their compositing was decided by ECS iteration order: a row of windows, a stack
    /// of glass, any blended prop instanced more than once. `gizmo-studio` sorted them and the
    /// engine did not, which is why the editor was right and the game was arbitrary.
    ///
    /// Spawn order is the probe because it is the thing that must not matter. Two renders of the
    /// same picture, built near-first and far-first, have to agree pixel for pixel.
    #[test]
    fn overlapping_transparents_of_one_material_do_not_depend_on_spawn_order() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available()) {
            eprintln!("skipping overlapping_transparents_of_one_material: no GPU");
            return;
        }
        // Software adapters need not apply. On the Windows runner the adapter is WARP, and a
        // deferred frame there means software-rasterising a 3072² × 4 shadow-map array: the job
        // that fixed the D3D12 shader error and let these tests actually render was still going
        // at 5.5 hours against ubuntu's 6 minutes. The per-job `timeout-minutes` turns that into a
        // report rather than a burnt runner; this turns it into a skip.
        if pollster::block_on(Renderer::headless_adapter_is_software()) {
            eprintln!("skipping overlapping_transparents_of_one_material: software adapter");
            return;
        }
        pollster::block_on(async {
            let near_first = render_two_panes(true).await;
            let far_first = render_two_panes(false).await;

            let differing = near_first
                .iter()
                .zip(far_first.iter())
                .filter(|(a, b)| a.abs_diff(**b) > 2)
                .count();
            assert_eq!(
                differing, 0,
                "{differing} bytes differ between the two spawn orders — the blend of two \
                 same-material transparents is being decided by iteration order"
            );
            // And the picture is not simply empty, which would make the comparison vacuous.
            let background = near_first[0];
            assert!(
                near_first.iter().filter(|b| **b != background).count() > 500,
                "the panes did not reach the framebuffer, so agreeing proves nothing"
            );
        });
    }

    /// Two overlapping semi-transparent quads sharing one material, spawned near-first or
    /// far-first. Same scene either way.
    async fn render_two_panes(near_first: bool) -> Vec<u8> {
        const W: u32 = 128;
        const H: u32 = 128;
        let mut renderer = crate::test_gpu::headless_renderer(W, H).await;
        let mut asset_manager = AssetManager::new();
        let mut world = World::new();

        let mesh = AssetManager::create_cube(&renderer.device);
        let tex = asset_manager.create_white_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
        );
        // DIFFERENT colours, one batch. The batch key is the material's *texture* bind group,
        // which both share because both were built from the same texture — so these two land in a
        // single batch and only their instance order can separate them. Two panes of the SAME
        // colour would prove nothing: `c over (c over bg)` is the same expression either way,
        // which is how the first version of this test passed with the fix removed.
        let green = Material::new(tex.clone())
            .with_pbr(Vec4::new(0.1, 0.9, 0.2, 0.5), 0.2, 0.0)
            .with_transparent(true);
        let red = Material::new(tex)
            .with_pbr(Vec4::new(0.9, 0.1, 0.1, 0.5), 0.2, 0.0)
            .with_transparent(true);

        // Flattened cubes standing in for panes, overlapping along the view axis.
        let spawn = |z: f32, mat: &Material, world: &mut World| {
            let e = world.spawn();
            world.add_component(
                e,
                Transform::new(Vec3::new(0.0, 0.0, z)).with_scale(Vec3::new(3.0, 3.0, 0.05)),
            );
            world.add_component(e, GlobalTransform::default());
            world.add_component(e, mesh.clone());
            world.add_component(e, mat.clone());
            world.add_component(e, MeshRenderer::new());
        };
        if near_first {
            spawn(2.0, &green, &mut world);
            spawn(-2.0, &red, &mut world);
        } else {
            spawn(-2.0, &red, &mut world);
            spawn(2.0, &green, &mut world);
        }

        world.spawn_bundle(CameraBundle {
            position: Vec3::new(0.0, 0.0, 12.0),
            yaw: -std::f32::consts::FRAC_PI_2,
            pitch: 0.0,
            primary: true,
            ..Default::default()
        });
        world.spawn_bundle(DirectionalLightBundle::default());

        super::batching::clear_render_cache();
        let frame = render_world(&mut renderer, &mut world).await;
        super::batching::clear_render_cache();
        frame
    }

    /// Every GPU test either refuses a software adapter or says why it does not.
    ///
    /// The Windows runner's adapter is WARP. A deferred frame there software-rasterises a
    /// 3072² × 4 shadow-map array, and the job that first let these tests render was still going
    /// at 5.5 hours against ubuntu's 6 minutes. Each job carries `timeout-minutes` for exactly
    /// that, but a timeout is a report, not a fix: the runner still burns 45 minutes and the job
    /// still fails.
    ///
    /// So the skip belongs next to the work, and this makes sure it stays there. It reads this
    /// file rather than trusting review — three tests added on 2026-08-15 checked only for the
    /// presence of an adapter, and would have taken the whole 45 minutes each.
    #[test]
    fn every_gpu_test_refuses_a_software_adapter() {
        /// Tests that deliberately run on WARP, and why.
        const DELIBERATE: &[(&str, &str)] = &[(
            "every_pipeline_compiles_on_this_backend",
            "its whole subject is whether the pipelines compile on THIS backend, so a software \
             adapter is coverage rather than cost — and it compiles rather than renders. It is \
             also the test that caught the D3D12 shader error.",
        )];

        let src = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/systems/render/mod.rs"),
        )
        .expect("this file");

        // Split into per-test bodies: from one `fn` to the next.
        let mut starts: Vec<(usize, String)> = Vec::new();
        for (i, _) in src.match_indices("#[test]") {
            let Some(fn_at) = src[i..].find("fn ") else { continue };
            let after = &src[i + fn_at + 3..];
            let name: String =
                after.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            starts.push((i, name));
        }
        assert!(starts.len() > 10, "only {} tests found", starts.len());

        let mut offenders = Vec::new();
        let mut checked = 0;
        for (n, (pos, name)) in starts.iter().enumerate() {
            let end = starts.get(n + 1).map_or(src.len(), |(p, _)| *p);
            let body = &src[*pos..end];
            // Only tests that open an adapter are in scope.
            if !body.contains("headless_adapter_available") {
                continue;
            }
            checked += 1;
            if body.contains("headless_adapter_is_software") {
                continue;
            }
            if DELIBERATE.iter().any(|(n, _)| n == name) {
                continue;
            }
            offenders.push(name.clone());
        }

        assert!(checked >= 12, "only {checked} GPU tests scanned");
        // A stale exemption is a failure too: a test that grew a software check should lose its
        // entry, or the list becomes a place things go to stop being looked at.
        for (name, _) in DELIBERATE {
            let Some((pos, _)) = starts.iter().find(|(_, n)| n == name) else {
                panic!("`{name}` is exempted here and no longer exists");
            };
            let end = starts
                .iter()
                .find(|(p, _)| p > pos)
                .map_or(src.len(), |(p, _)| *p);
            assert!(
                !src[*pos..end].contains("headless_adapter_is_software"),
                "`{name}` now refuses software adapters — delete its exemption"
            );
        }

        assert!(
            offenders.is_empty(),
            "these GPU tests would run on WARP and take the runner's whole 45 minutes:\n  {}\n\
             Add the `headless_adapter_is_software` guard, or exempt them here with a reason.",
            offenders.join("\n  ")
        );
    }

    /// A scene larger than the instance buffer grows the buffer instead of losing geometry.
    ///
    /// `Renderer::ensure_instance_capacity` has existed, and been unit-tested, since the buffer
    /// could grow at all — with `gizmo-studio` as its only caller. The engine's own path clamped
    /// the upload to `instance_capacity` and returned the truncated count, so past 8 192 instances
    /// a game dropped whatever the region split sacrificed while the editor, showing the same
    /// scene, drew all of it. The two-region layout was built to make that truncation degrade
    /// gracefully; it degrades nothing now, and stays as the guard for the day something refuses
    /// to grow.
    ///
    /// Asserted on the count that reaches the GPU rather than on pixels: the failure is geometry
    /// that never got uploaded, and 9 000 overlapping cubes look much the same either way.
    #[test]
    fn a_scene_past_the_instance_capacity_grows_the_buffer_instead_of_dropping_meshes() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available()) {
            eprintln!("skipping a_scene_past_the_instance_capacity_grows_the_buffer: no GPU");
            return;
        }
        // Software adapters need not apply. On the Windows runner the adapter is WARP, and a
        // deferred frame there means software-rasterising a 3072² × 4 shadow-map array: the job
        // that fixed the D3D12 shader error and let these tests actually render was still going
        // at 5.5 hours against ubuntu's 6 minutes. The per-job `timeout-minutes` turns that into a
        // report rather than a burnt runner; this turns it into a skip.
        if pollster::block_on(Renderer::headless_adapter_is_software()) {
            eprintln!("skipping a_scene_past_the_instance_capacity_grows_the_buffer: software adapter");
            return;
        }
        pollster::block_on(async {
            let mut renderer = crate::test_gpu::headless_renderer(64, 64).await;
            let start_capacity = renderer.scene.instance_capacity;
            // One more than the buffer holds is enough to prove the point; 9 000 keeps the test
            // honest if the starting capacity is ever raised to 8 192 exactly.
            let count = start_capacity + 808;

            let mut asset_manager = AssetManager::new();
            let mut world = World::new();
            let mesh = AssetManager::create_cube(&renderer.device);
            let tex = asset_manager.create_white_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            );
            let mat = Material::new(tex).with_pbr(Vec4::new(0.8, 0.8, 0.8, 1.0), 0.5, 0.0);
            // All in front of the camera and all sharing mesh + material, so they land in ONE
            // batch and the instance count is the only thing under test.
            for i in 0..count {
                let e = world.spawn();
                let x = (i % 100) as f32 * 0.05;
                let y = (i / 100) as f32 * 0.05;
                world.add_component(e, Transform::new(Vec3::new(x, y, -20.0)));
                world.add_component(e, GlobalTransform::default());
                world.add_component(e, mesh.clone());
                world.add_component(e, mat.clone());
                world.add_component(e, MeshRenderer::new());
            }
            world.spawn_bundle(CameraBundle {
                position: Vec3::new(0.0, 0.0, 40.0),
                yaw: 0.0,
                pitch: 0.0,
                primary: true,
                ..Default::default()
            });

            super::batching::clear_render_cache();
            let view_proj = gizmo_math::Mat4::perspective_rh(1.0, 1.0, 0.1, 500.0)
                * gizmo_math::Mat4::look_at_rh(
                    Vec3::new(0.0, 0.0, 40.0),
                    Vec3::new(0.0, 0.0, -20.0),
                    Vec3::Y,
                );
            let (_items, uploaded) = super::batching::collect_draw_items(
                &world,
                &mut renderer,
                view_proj,
                [gizmo_math::Mat4::IDENTITY; 4],
                Vec3::new(0.0, 0.0, 40.0),
            );

            assert!(
                count as u32 > start_capacity as u32,
                "the scene must exceed the starting capacity for this test to mean anything"
            );
            assert_eq!(
                uploaded, count as u32,
                "{} of {count} instances reached the GPU — the engine truncated instead of growing",
                uploaded
            );
            assert!(renderer.scene.instance_capacity >= count);
            super::batching::clear_render_cache();
        });
    }

    /// A double-sided material shows its back faces to the game, not only to the editor.
    ///
    /// `Material::with_double_sided` and its `is_double_sided` field have been public for as long
    /// as materials have, and until 2026-08-15 the only thing that read them was `gizmo-studio`'s
    /// forward pipeline. The engine's own path — the z-prepass and the G-buffer pass — culled back
    /// faces unconditionally, so a cloth, a leaf card or any open surface authored double-sided
    /// showed both faces in the editor viewport and lost one of them in the game. The class the
    /// architectural review named exactly: the engine exports a capability its own default path
    /// does not act on.
    ///
    /// The scene is a camera INSIDE a cube, which is the cheapest way to see nothing but back
    /// faces: with culling on the frame is empty, with the flag honoured the interior is drawn.
    ///
    /// **What it actually guards is the G-BUFFER pipeline selection.** This used to claim
    /// "reverting *either* pipeline selection in `passes::geometry`" fails it; that was measured
    /// on 2026-08-16 and is not true — reverting the z-prepass arm alone leaves this test green,
    /// with the old whole-frame assertion just as much as with the current one. It makes sense
    /// once stated: the prepass only writes depth, and in a scene whose sole occluder is the
    /// surface under test, a wrong depth arm changes no colour. The z-prepass arm is therefore
    /// **unguarded**, and saying so is worth more than a sentence that sounds stronger.
    /// # macOS: measured, unexplained, and NOT a threshold artefact
    ///
    /// This fails on the macOS runner and is ignored there. The first reading looked like a
    /// mis-calibrated threshold — the old whole-frame assertion wanted `> 0.5` and Metal produced
    /// 36.6% — so the assertion was rewritten to something a backend cannot argue with: the
    /// centre of the image. The camera sits inside a cube 16 units across looking at a wall 8
    /// units away, so the middle of the frame lands on that wall under *any* projection, and if
    /// the flag is honoured those pixels must change.
    ///
    /// Metal changes **43.8%** of them. Vulkan changes over 90%.
    ///
    /// That killed the threshold theory rather than confirming it: a framing difference cannot
    /// leave half the centre of a wall untouched. Something about how much of the interior gets
    /// drawn genuinely differs on Metal, and loosening the number further would hide a real
    /// rendering difference behind a green tick — which is the opposite of what this test is for.
    ///
    /// So it is `ignore`d on macOS with the measurement written down, not weakened everywhere.
    /// It still guards Linux and Windows, where reverting the G-buffer pipeline selection takes
    /// the centre to 0.0%. Diagnosing the Metal side needs a Mac to run it on; nobody here has
    /// one, and guessing at a GPU difference from another platform is how a wrong fix ships.
    #[cfg_attr(
        target_os = "macos",
        ignore = "Metal draws only 43.8% of the cube's interior where Vulkan draws >90% — a real \
                  backend difference, measured 2026-08-16, and not something to paper over by \
                  lowering the threshold"
    )]
    #[test]
    fn a_double_sided_material_is_drawn_from_behind() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available()) {
            eprintln!("skipping a_double_sided_material_is_drawn_from_behind: no GPU");
            return;
        }
        // Software adapters need not apply. On the Windows runner the adapter is WARP, and a
        // deferred frame there means software-rasterising a 3072² × 4 shadow-map array: the job
        // that fixed the D3D12 shader error and let these tests actually render was still going
        // at 5.5 hours against ubuntu's 6 minutes. The per-job `timeout-minutes` turns that into a
        // report rather than a burnt runner; this turns it into a skip.
        if pollster::block_on(Renderer::headless_adapter_is_software()) {
            eprintln!("skipping a_double_sided_material_is_drawn_from_behind: software adapter");
            return;
        }
        pollster::block_on(async {
            let one_sided = render_from_inside_a_cube(false).await;
            let two_sided = render_from_inside_a_cube(true).await;

            // The CENTRE is the part of this test that means the same thing on every backend.
            // The camera sits at the origin inside a cube 16 units across, looking at a wall 8
            // units away; the ray through the middle of the image lands on that wall under any
            // projection, so if the flag is honoured those pixels MUST change from background to
            // surface. What fraction of the *whole* frame changes is a framing question — how
            // much of the wall the projection covers — and that is exactly what differed between
            // backends: this assertion was `> 0.5` over the whole frame and macOS/Metal produced
            // 36.6%, failing a test whose subject was working there.
            //
            // Bytes, not pixels, was the other half of it: alpha never changes, so a quarter of
            // the bytes compared could never differ and the old ratio was capped at 0.75 before
            // any geometry was drawn. Both checks below count PIXELS and ignore alpha.
            let (centre_changed, centre_total) = changed_pixels(&one_sided, &two_sided, 32);
            let centre_ratio = centre_changed as f32 / centre_total as f32;
            assert!(
                centre_ratio > 0.9,
                "only {:.1}% of the frame's centre changed when the material was made \
                 double-sided — the camera is inside the cube, so the middle of the image is a \
                 wall the engine's deferred path is culling away",
                100.0 * centre_ratio
            );

            // A loose whole-frame floor as well. The defect this guards produces IDENTICAL
            // frames, so it fails at 0% either way; the floor is here so a backend that drew
            // only a sliver of the interior could not pass on the centre alone. It is set far
            // below every backend's real figure rather than at the edge of one of them.
            let (changed, total) = changed_pixels(&one_sided, &two_sided, 128);
            let ratio = changed as f32 / total as f32;
            assert!(
                ratio > 0.25,
                "only {:.1}% of the frame changed when the material was made double-sided — the \
                 engine's deferred path is culling the back faces it was told to keep",
                100.0 * ratio
            );
        });
    }

    /// The cube of [`render_frame_with_mesh`], with the camera inside it, so every visible
    /// triangle is a back face.
    async fn render_from_inside_a_cube(double_sided: bool) -> Vec<u8> {
        const W: u32 = 128;
        const H: u32 = 128;
        let mut renderer = crate::test_gpu::headless_renderer(W, H).await;
        let mut asset_manager = AssetManager::new();
        let mut world = World::new();

        let mesh = AssetManager::create_cube(&renderer.device);
        let tex = asset_manager.create_white_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
        );
        let mat = Material::new(tex)
            .with_pbr(Vec4::new(0.9, 0.15, 0.15, 1.0), 0.0, 1.0)
            .with_double_sided(double_sided);
        let cube = world.spawn();
        // Big enough that the camera at the origin sits well inside it and the walls clear the
        // near plane.
        world.add_component(cube, Transform::new(Vec3::ZERO).with_scale(Vec3::splat(8.0)));
        world.add_component(cube, GlobalTransform::default());
        world.add_component(cube, mesh);
        world.add_component(cube, mat);
        world.add_component(cube, MeshRenderer::new());

        world.spawn_bundle(CameraBundle {
            position: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            primary: true,
            ..Default::default()
        });
        world.spawn_bundle(DirectionalLightBundle::default());

        render_world(&mut renderer, &mut world).await
    }

    /// How many pixels of a centred `side`×`side` block differ between two 128×128 RGBA8 frames,
    /// and how many were looked at.
    ///
    /// A pixel counts as changed when any of R/G/B moves by more than 8. **Alpha is ignored on
    /// purpose**: these frames are opaque, so including it only dilutes every ratio by a fixed
    /// quarter and makes a threshold read as stricter than it is.
    ///
    /// `side == 128` covers the whole frame.
    fn changed_pixels(a: &[u8], b: &[u8], side: u32) -> (usize, usize) {
        const W: u32 = 128;
        const H: u32 = 128;
        const BPP: usize = 4;
        let side = side.min(W).min(H);
        let x0 = (W - side) / 2;
        let y0 = (H - side) / 2;
        let mut changed = 0;
        for y in y0..y0 + side {
            for x in x0..x0 + side {
                let i = (y as usize * W as usize + x as usize) * BPP;
                if (0..3).any(|c| a[i + c].abs_diff(b[i + c]) > 8) {
                    changed += 1;
                }
            }
        }
        (changed, (side * side) as usize)
    }

    /// Drive the REAL [`default_render_pass`] over `world` into a 128×128 offscreen target and
    /// read the frame back as RGBA8 bytes.
    ///
    /// Extracted so every golden test renders through byte-for-byte the same code path — the
    /// point of comparing two frames is that nothing between the scene and the bytes differs.
    async fn render_world(renderer: &mut Renderer, world: &mut World) -> Vec<u8> {
        const W: u32 = 128;
        const H: u32 = 128;
        const BPP: u32 = 4;

        let format = renderer.config.format;
        let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("frame-target"),
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
        default_render_pass(world, &mut encoder, &view, renderer);

        let staging = renderer.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame-readback"),
            size: (W * H * BPP) as u64,
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
        renderer.queue.submit(Some(encoder.finish()));

        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |v| tx.send(v).unwrap());
        let _ = renderer.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        rx.recv().unwrap().unwrap();
        let data = slice.get_mapped_range()
            // wgpu 30 made this fallible; the range is the whole buffer we just mapped, so a
            // failure here is a programming error rather than a runtime condition.
            .expect("a just-mapped buffer's full range is always valid");

        data.to_vec()
    }

    /// A unit cube as a flat triangle list: 36 vertices, six per face, each face carrying its
    /// own normal — so the only true duplicates are the two shared corners *within* a face,
    /// and a correct dedup lands on 24.
    ///
    /// Written out here rather than reusing [`AssetManager::create_cube`] because this test
    /// needs the VERTICES and that function only hands back a finished `Mesh`.
    fn cube_vertices() -> Vec<crate::renderer::gpu_types::Vertex> {
        const CORNERS: [[f32; 3]; 8] = [
            [-1.0, -1.0, -1.0],
            [1.0, -1.0, -1.0],
            [1.0, 1.0, -1.0],
            [-1.0, 1.0, -1.0],
            [-1.0, -1.0, 1.0],
            [1.0, -1.0, 1.0],
            [1.0, 1.0, 1.0],
            [-1.0, 1.0, 1.0],
        ];
        // (two triangles as corner indices, face normal)
        const FACES: [([usize; 6], [f32; 3]); 6] = [
            ([1, 0, 3, 1, 3, 2], [0.0, 0.0, -1.0]),
            ([4, 5, 6, 4, 6, 7], [0.0, 0.0, 1.0]),
            ([0, 4, 7, 0, 7, 3], [-1.0, 0.0, 0.0]),
            ([5, 1, 2, 5, 2, 6], [1.0, 0.0, 0.0]),
            ([3, 7, 6, 3, 6, 2], [0.0, 1.0, 0.0]),
            ([0, 1, 5, 0, 5, 4], [0.0, -1.0, 0.0]),
        ];
        const UVS: [[f32; 2]; 6] = [
            [0.0, 1.0],
            [1.0, 1.0],
            [1.0, 0.0],
            [0.0, 1.0],
            [1.0, 0.0],
            [0.0, 0.0],
        ];

        let mut out = Vec::with_capacity(36);
        for (corner_indices, normal) in FACES {
            for (k, corner) in corner_indices.into_iter().enumerate() {
                out.push(crate::renderer::gpu_types::Vertex {
                    position: CORNERS[corner],
                    color: [1.0, 1.0, 1.0, 1.0],
                    normal,
                    tex_coords: UVS[k],
                    ..Default::default()
                });
            }
        }
        out
    }

    /// The indexed draw path must produce the same picture as the flat one, to the byte.
    ///
    /// This is the only test that executes `draw_indexed` at all. Until it existed, both
    /// `Mesh::new_indexed`'s deduplication and the `record_draw` index branch were code that
    /// compiled and had never once run: the engine's only producer of indexed meshes is the
    /// glTF loader, and this repository ships no `.glb` (see `assets/README.md` — the models
    /// are third-party licensed and uncommitted), so every mesh in every other test is flat.
    ///
    /// Byte equality rather than a similarity threshold is the point. A wrong index buffer
    /// does not draw *nothing* — it draws the same triangle count from the wrong corners, and
    /// at 128×128 with one lit cube that can still look plausible. Only exact equality
    /// distinguishes "the geometry survived the round trip" from "something reasonable
    /// appeared on screen".
    #[test]
    fn an_indexed_mesh_renders_byte_identically_to_the_flat_one() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available()) {
            eprintln!(
                "skipping an_indexed_mesh_renders_byte_identically_to_the_flat_one: \
                 no GPU adapter available"
            );
            return;
        }
        // WARP and friends can create these pipelines in seconds and cannot finish
        // rendering through them: `windows-latest` spent five and a half hours on this
        // file once the D3D12 shader fix let it get this far. Pipeline compilation is
        // the coverage that matters on such a runner and it is kept by
        // `every_pipeline_compiles_on_this_backend`, which builds the whole renderer.
        if pollster::block_on(Renderer::headless_adapter_is_software()) {
            eprintln!("skipping an_indexed_mesh_renders_byte_identically_to_the_flat_one: software adapter — see the note above");
            return;
        }
        pollster::block_on(async {
            let vertices = cube_vertices();
            assert_eq!(vertices.len(), 36, "cube_vertices is meant to be a flat triangle list");

            let flat_verts = vertices.clone();
            let flat = render_frame_with_mesh(1.0, false, move |device| {
                crate::renderer::components::Mesh::from_vertices(device, &flat_verts, "flat_cube")
            })
            .await;

            let indexed_verts = vertices.clone();
            let indexed = render_frame_with_mesh(1.0, false, move |device| {
                let mesh = crate::renderer::components::Mesh::new_indexed(
                    device,
                    &indexed_verts,
                    Vec3::ZERO,
                    "indexed_cube".to_string(),
                );
                // Guard against a vacuous pass: if `new_indexed` ever stopped producing an
                // index buffer (or stopped deduplicating), the frames below would still match
                // — because both would be flat — and this test would silently stop testing
                // anything at all.
                assert!(
                    mesh.ibuf.is_some(),
                    "new_indexed produced no index buffer; the frame comparison would be vacuous"
                );
                assert_eq!(
                    mesh.index_count, 36,
                    "every original vertex must still be referenced exactly once"
                );
                assert_eq!(
                    mesh.vertex_count, 24,
                    "a cube with per-face normals has 24 distinct vertices; got {} \
                     (deduplication did not run, or merged across faces)",
                    mesh.vertex_count
                );
                // 24 unique vertices is far below the 65536 that `Uint16` can address, so this
                // mesh MUST take the narrow path — which is what makes the frame comparison
                // below a test of 16-bit indices rather than only of 32-bit ones. A buffer
                // written as u16 and bound as u32 does not crash; it draws the wrong triangles,
                // and only the byte comparison would catch it.
                assert_eq!(
                    mesh.index_format,
                    wgpu::IndexFormat::Uint16,
                    "a 24-vertex mesh must use 16-bit indices"
                );
                mesh
            })
            .await;

            assert_eq!(flat.len(), indexed.len(), "frame sizes differ");
            let differing = flat
                .iter()
                .zip(indexed.iter())
                .filter(|(a, b)| a != b)
                .count();
            assert_eq!(
                differing,
                0,
                "{differing} of {} bytes differ between the flat and indexed renders of the \
                 same cube — the index buffer, its format, or the draw_indexed range is wrong",
                flat.len()
            );
        });
    }

    // ── ITEM 7: the painted-backdrop path, rendered ────────────────────────────────────────
    //
    // These are the only tests in the tree that can see what a backdrop actually looks like.
    // Everything else about `MaterialType::Backdrop` is pinned as arithmetic or as pipeline
    // state (`gizmo_renderer::backdrop`, `batching::cmp_draw_order`); this drives the real
    // `default_render_pass` and reads the pixels back.

    /// How the full-screen test panel is materialised. The three arms are the three answers
    /// the engine can give to "draw my sky geometry", and the test compares them directly.
    #[derive(Clone, Copy, PartialEq, Debug)]
    enum PanelKind {
        /// The new path: the mesh's own pixels, camera-locked, behind everything.
        Backdrop,
        /// What the game ships with today's engine when it wants the artwork: correct pixels,
        /// but ordinary world geometry — it writes depth and stands in front of the world.
        Unlit,
        /// The other half of the report: correct depth, but the mesh's texture and vertex
        /// colour are discarded for an invented gradient.
        Skybox,
    }

    /// A screen-filling quad in the YZ plane (its normal points down −X, at a camera looking
    /// along +X), 8 units across, carrying `colour` as its VERTEX colour and a full 0..1 UV
    /// span.
    ///
    /// The colour rides the vertex attribute rather than the material albedo on purpose: it is
    /// the channel `sky.wgsl` throws away, so a green pixel on screen is evidence the mesh's
    /// own data survived to the framebuffer.
    fn panel_vertices(colour: [f32; 4]) -> Vec<crate::renderer::gpu_types::Vertex> {
        const R: f32 = 4.0;
        let v = |y: f32, z: f32, u: f32, w: f32| crate::renderer::gpu_types::Vertex {
            position: [0.0, y, z],
            color: colour,
            normal: [-1.0, 0.0, 0.0],
            tex_coords: [u, w],
            ..Default::default()
        };
        vec![
            v(-R, -R, 0.0, 1.0),
            v(-R, R, 1.0, 1.0),
            v(R, R, 1.0, 0.0),
            v(-R, -R, 0.0, 1.0),
            v(R, R, 1.0, 0.0),
            v(R, -R, 0.0, 0.0),
        ]
    }

    /// The full-screen test panel: how it is drawn, what texture it carries, and — for the
    /// world-space kinds only — where it is nailed down.
    #[derive(Clone, Copy)]
    struct Panel {
        kind: PanelKind,
        /// `false` = the 1×1 white texture, `true` = the 256² checkerboard.
        checkered: bool,
        /// The world point the panel sits two units in front of.
        ///
        /// Ignored for [`PanelKind::Backdrop`], whose transform is camera-relative BY
        /// CONSTRUCTION — that asymmetry is the property under test, not an oversight. Pass
        /// the camera position to put a world-space panel where a backdrop would appear;
        /// pass a fixed point to leave one behind as the camera drives away.
        anchor: Vec3,
    }

    /// The standard red world cube (optional) plus a green screen-filling panel (optional),
    /// rendered from `camera_pos` looking along +X.
    async fn render_panel_scene(
        camera_pos: Vec3,
        with_cube: bool,
        panel: Option<Panel>,
    ) -> Vec<u8> {
        const W: u32 = 128;
        const H: u32 = 128;

        let mut renderer = crate::test_gpu::headless_renderer(W, H).await;
        // Screen-space filters off. Each of them re-derives world positions and ray
        // directions from the camera, so each makes the final image depend on where the
        // camera IS — which would drown out the question these tests ask, namely whether the
        // BACKDROP moved. None of them touches the backdrop's own shading: they read the
        // G-buffer, which a backdrop never writes to.
        renderer.taa = None;
        renderer.ssr = None;
        renderer.ssgi = None;
        renderer.ssao = None;
        let mut asset_manager = AssetManager::new();
        let mut world = World::new();

        let white = asset_manager.create_white_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
        );

        if with_cube {
            let cube = world.spawn();
            world.add_component(cube, Transform::new(Vec3::ZERO));
            world.add_component(cube, GlobalTransform::default());
            world.add_component(cube, AssetManager::create_cube(&renderer.device));
            world.add_component(
                cube,
                Material::new(white.clone()).with_pbr(Vec4::new(0.9, 0.15, 0.15, 1.0), 0.0, 1.0),
            );
            world.add_component(cube, MeshRenderer::new());
        }

        if let Some(Panel { kind, checkered, anchor }) = panel {
            let tex = if checkered {
                asset_manager.create_checkerboard_texture(
                    &renderer.device,
                    &renderer.queue,
                    &renderer.scene.texture_bind_group_layout,
                )
            } else {
                white.clone()
            };
            let mat = match kind {
                PanelKind::Backdrop => Material::new(tex).with_backdrop(Vec4::ONE),
                PanelKind::Unlit => Material::new(tex).with_unlit(Vec4::ONE),
                // Exactly what `Commands::spawn_skybox` builds.
                PanelKind::Skybox => Material::new(tex).with_unlit(Vec4::ONE).with_skybox(),
            };
            // Two units along the camera's forward axis. For a backdrop that offset IS the
            // transform (the shader adds the camera position); for the world-space kinds it is
            // measured from `anchor`.
            let offset = Vec3::new(2.0, 0.0, 0.0);
            let pos = match kind {
                PanelKind::Backdrop => offset,
                PanelKind::Unlit | PanelKind::Skybox => anchor + offset,
            };
            let panel_entity = world.spawn();
            world.add_component(panel_entity, Transform::new(pos));
            world.add_component(panel_entity, GlobalTransform::default());
            world.add_component(
                panel_entity,
                crate::renderer::components::Mesh::from_vertices(
                    &renderer.device,
                    &panel_vertices([0.0, 1.0, 0.0, 1.0]),
                    "backdrop_panel",
                ),
            );
            world.add_component(panel_entity, mat);
            world.add_component(panel_entity, MeshRenderer::new());
        }

        world.spawn_bundle(CameraBundle {
            position: camera_pos,
            yaw: 0.0,
            pitch: 0.0,
            primary: true,
            ..Default::default()
        });
        world.spawn_bundle(DirectionalLightBundle::default());

        render_world(&mut renderer, &mut world).await
    }

    /// The pixel at `(x, y)` of a 128×128 RGBA8 frame.
    fn px(frame: &[u8], x: u32, y: u32) -> [u8; 4] {
        let i = ((y * 128 + x) * 4) as usize;
        [frame[i], frame[i + 1], frame[i + 2], frame[i + 3]]
    }

    /// Green is the dominant channel by a clear margin — i.e. this pixel is the panel's own
    /// colour and not a tone-mapped sky gradient or a red cube.
    fn is_green(p: [u8; 4]) -> bool {
        p[1] as i32 > p[0] as i32 + 20 && p[1] as i32 > p[2] as i32 + 20
    }

    /// Both halves of the report, in one frame each: the backdrop must show the MESH's pixels
    /// (which `Skybox` discards) AND stay behind the world (which `Unlit` does not).
    ///
    /// The scene is a red PBR cube 6 units from the camera with a green screen-filling panel
    /// at 2 units — squarely between the camera and the cube. So the centre pixel answers
    /// "did the panel occlude the world?" and a corner pixel answers "did the panel reach the
    /// screen, with its own colour?".
    #[test]
    fn a_backdrop_shows_the_meshs_own_pixels_and_stays_behind_the_world() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available()) {
            eprintln!(
                "skipping a_backdrop_shows_the_meshs_own_pixels_and_stays_behind_the_world: \
                 no GPU adapter available"
            );
            return;
        }
        // WARP and friends can create these pipelines in seconds and cannot finish
        // rendering through them: `windows-latest` spent five and a half hours on this
        // file once the D3D12 shader fix let it get this far. Pipeline compilation is
        // the coverage that matters on such a runner and it is kept by
        // `every_pipeline_compiles_on_this_backend`, which builds the whole renderer.
        if pollster::block_on(Renderer::headless_adapter_is_software()) {
            eprintln!("skipping a_backdrop_shows_the_meshs_own_pixels_and_stays_behind_the_world: software adapter — see the note above");
            return;
        }
        pollster::block_on(async {
            let cam = Vec3::new(-6.0, 0.0, 0.0);
            // Anchored at the camera, so all three kinds put the panel on the same pixels and
            // the frames differ only in HOW it is drawn.
            let at = |kind| Panel { kind, checkered: false, anchor: cam };
            let no_panel = render_panel_scene(cam, true, None).await;
            let backdrop = render_panel_scene(cam, true, Some(at(PanelKind::Backdrop))).await;
            let unlit = render_panel_scene(cam, true, Some(at(PanelKind::Unlit))).await;
            let skybox = render_panel_scene(cam, true, Some(at(PanelKind::Skybox))).await;

            // Premise: without a panel the corner is background and the centre is the cube.
            assert!(
                !is_green(px(&no_panel, 8, 8)),
                "premise broken: the empty background is already green"
            );
            let bare_centre = px(&no_panel, 64, 64);
            assert!(
                !is_green(bare_centre),
                "premise broken: the red cube renders green ({bare_centre:?})"
            );

            // (a) The backdrop reaches the screen carrying the mesh's OWN vertex colour.
            let corner = px(&backdrop, 8, 8);
            assert!(
                is_green(corner),
                "the backdrop did not reach the screen with the mesh's own colour ({corner:?})"
            );

            // (b) …and it did NOT occlude the world 4 units behind it.
            let centre = px(&backdrop, 64, 64);
            assert!(
                !is_green(centre),
                "the backdrop painted over the cube ({centre:?}) — it is writing depth or \
                 winning the depth test"
            );

            // The `Unlit` arm is the reported symptom: same geometry, same place, and the
            // panel stands in front of the world.
            let unlit_centre = px(&unlit, 64, 64);
            assert!(
                is_green(unlit_centre),
                "premise broken: an `Unlit` panel 2 units from the camera is SUPPOSED to \
                 occlude a cube at 6 ({unlit_centre:?}); if it no longer does, this test is no \
                 longer distinguishing the two materials"
            );

            // The `Skybox` arm is the other reported half: correct depth, but the mesh's own
            // pixels are thrown away for an invented gradient.
            let sky_corner = px(&skybox, 8, 8);
            assert!(
                !is_green(sky_corner),
                "premise broken: `Skybox` is supposed to DISCARD the mesh's vertex colour and \
                 generate a gradient, but the corner came out green ({sky_corner:?})"
            );
            assert_ne!(
                sky_corner, corner,
                "the backdrop and the skybox produced the same pixel — one of them is not \
                 running its own shader"
            );
        });
    }

    /// The texture half, which the vertex-colour test above cannot see: swap the panel's
    /// texture and nothing else. `backdrop.wgsl` samples it, so the frames must differ; with
    /// `sky.wgsl` (zero `textureSample` calls) they would be byte-identical.
    #[test]
    fn a_backdrops_texture_reaches_the_screen() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available()) {
            eprintln!("skipping a_backdrops_texture_reaches_the_screen: no GPU adapter available");
            return;
        }
        // Software adapters build these pipelines in seconds and cannot finish rendering
        // through them — see the longer note on the first of these tests.
        if pollster::block_on(Renderer::headless_adapter_is_software()) {
            eprintln!("skipping a_backdrops_texture_reaches_the_screen: software adapter");
            return;
        }
        pollster::block_on(async {
            let cam = Vec3::new(-6.0, 0.0, 0.0);
            let with_tex = |checkered| Panel { kind: PanelKind::Backdrop, checkered, anchor: cam };
            let plain = render_panel_scene(cam, false, Some(with_tex(false))).await;
            let checkered = render_panel_scene(cam, false, Some(with_tex(true))).await;

            let differing = plain.iter().zip(checkered.iter()).filter(|(a, b)| a != b).count();
            assert!(
                differing > plain.len() / 10,
                "only {differing} of {} bytes changed when the backdrop's TEXTURE was swapped \
                 for a checkerboard — the shader is not sampling it",
                plain.len()
            );

            // The same swap under `Skybox`, which is the measurement the report made by
            // grepping (`grep -c textureSample sky.wgsl` → 0), taken here in pixels: the
            // texture makes no difference at all, because nothing ever reads it.
            let sky = |checkered| Panel { kind: PanelKind::Skybox, checkered, anchor: cam };
            let sky_plain = render_panel_scene(cam, false, Some(sky(false))).await;
            let sky_checkered = render_panel_scene(cam, false, Some(sky(true))).await;
            assert_eq!(
                sky_plain, sky_checkered,
                "premise broken: `Skybox` is supposed to ignore the mesh's texture entirely, \
                 so swapping it must change nothing — if it now does, this test is no longer \
                 measuring what distinguishes the two materials"
            );
        });
    }

    /// The largest single-channel difference between two frames.
    fn max_channel_delta(a: &[u8], b: &[u8]) -> u8 {
        a.iter().zip(b.iter()).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0)
    }

    /// Property (2): locked to the camera. Two cameras with the same orientation, 900 units
    /// apart, over a scene whose only content is the backdrop — so any real difference in the
    /// frame is the backdrop having moved.
    ///
    /// The bar is two 8-bit levels rather than byte equality, and the reason is arithmetic,
    /// not slack. The lock adds the camera position in world space and the view matrix
    /// subtracts it again, both in f32: at 900 units that round trip loses a few ULPs of
    /// mantissa. (The alternative — uploading a translation-free view-projection alongside
    /// `view_proj` — buys exactness for another 64 bytes in `SceneUniforms` and a field every
    /// construction site must fill; at a relative error of ~1e-7 on scenery painted at
    /// infinity, it is not worth it.)
    ///
    /// Two levels is a strong bound here, not a loose one: the panel wears a checkerboard
    /// whose own light/dark step is ~150 levels, so geometry that had genuinely SHIFTED — even
    /// by a fraction of a pixel — would move checker edges by tens of levels, and an unlocked
    /// panel leaves the frame altogether. The premise arm below measures that.
    #[test]
    fn a_backdrop_is_locked_to_the_camera() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available()) {
            eprintln!("skipping a_backdrop_is_locked_to_the_camera: no GPU adapter available");
            return;
        }
        // Software adapters build these pipelines in seconds and cannot finish rendering
        // through them — see the longer note on the first of these tests.
        if pollster::block_on(Renderer::headless_adapter_is_software()) {
            eprintln!("skipping a_backdrop_is_locked_to_the_camera: software adapter");
            return;
        }
        pollster::block_on(async {
            let here = Vec3::new(-6.0, 0.0, 0.0);
            let far_away = Vec3::new(-6.0, 0.0, 900.0);
            // `anchor` is unread for a backdrop — the transform is camera-relative, which is
            // precisely what makes the two frames comparable.
            let bd = Panel { kind: PanelKind::Backdrop, checkered: true, anchor: here };

            let a = render_panel_scene(here, false, Some(bd)).await;
            let b = render_panel_scene(far_away, false, Some(bd)).await;
            let delta = max_channel_delta(&a, &b);
            assert!(
                delta <= 2,
                "the frame changed by up to {delta} levels when the camera moved 900 units — \
                 the backdrop is not locked to it (f32 cancellation alone cannot exceed 2)"
            );

            // Premise: the SAME panel as ordinary world geometry does not survive the move. It
            // is nailed to `here` in both frames, so once the camera has driven 900 units away
            // it is nowhere near the view — which is exactly the failure a camera lock exists
            // to prevent, and the reason the game's 500-unit skybox cube runs out.
            let unlit = Panel { kind: PanelKind::Unlit, checkered: true, anchor: here };
            let ua = render_panel_scene(here, false, Some(unlit)).await;
            let ub = render_panel_scene(far_away, false, Some(unlit)).await;
            let unlocked_delta = max_channel_delta(&ua, &ub);
            assert!(
                unlocked_delta > 20,
                "premise broken: an unlocked panel is supposed to be left behind when the \
                 camera drives away from it, but the frame only moved by {unlocked_delta} \
                 levels — so the {delta}-level bar above is not distinguishing anything"
            );
        });
    }

    /// The mean of every RGB byte in a frame (alpha excluded).
    async fn render_mean_brightness(exposure: f32) -> f32 {
        let data = render_frame(exposure, false).await;
        let sum: u64 = data.as_chunks::<4>().0.iter().map(|p| p[0] as u64 + p[1] as u64 + p[2] as u64).sum();
        sum as f32 / (data.len() / 4 * 3) as f32
    }

    /// Exposure is a SINGLE post-process knob applied over the whole composited HDR (the
    /// deferred pass no longer bakes it in). This guards that rework: a higher camera exposure
    /// must brighten the frame. If exposure were detached (or the deferred→post move dropped
    /// the wiring), the two renders would match and this fails. (Tone-mapping is non-linear so
    /// we assert monotonic increase, not an exact 2x.)
    #[test]
    fn camera_exposure_brightens_the_frame() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available()) {
            eprintln!("skipping camera_exposure_brightens_the_frame: no GPU adapter available");
            return;
        }
        // Software adapters build these pipelines in seconds and cannot finish rendering
        // through them — see the longer note on the first of these tests.
        if pollster::block_on(Renderer::headless_adapter_is_software()) {
            eprintln!("skipping camera_exposure_brightens_the_frame: software adapter");
            return;
        }
        pollster::block_on(async {
            let dim = render_mean_brightness(1.0).await;
            let bright = render_mean_brightness(2.0).await;
            assert!(
                bright > dim + 1.0,
                "higher camera exposure must brighten the scene, but exp=1.0 mean={dim:.2} \
                 vs exp=2.0 mean={bright:.2} (exposure not applied / detached from post?)"
            );
        });
    }

    /// The six point-shadow face passes are skipped when nothing samples them, and skipping
    /// them changes no pixel.
    ///
    /// `Renderer::point_shadows_enabled` defaults to false, and `deferred_lighting.wgsl` already
    /// gates its cubemap lookup on the uniform written from that same bool. Until this was fixed
    /// the passes ran anyway — six depth passes a frame, two draws per lit item each, twelve of
    /// the twenty-three draws a lit batch costs, into a cubemap nothing read.
    ///
    /// Rendering the same scene both ways and demanding byte-identical output is what proves the
    /// skipped work was unobserved. It is also what catches the dangerous version of this change:
    /// gate something the shader *does* sample — the cascades, say — and the frames diverge here
    /// rather than in someone's screenshot.
    #[test]
    fn skipping_the_point_shadow_passes_changes_no_pixel() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available()) {
            eprintln!("skipping skipping_the_point_shadow_passes_changes_no_pixel: no GPU adapter");
            return;
        }
        // Software adapters build these pipelines in seconds and cannot finish rendering
        // through them — see the longer note on the first of these tests.
        if pollster::block_on(Renderer::headless_adapter_is_software()) {
            eprintln!("skipping skipping_the_point_shadow_passes_changes_no_pixel: software adapter");
            return;
        }
        pollster::block_on(async {
            let gated = render_frame(1.0, false).await;
            let recorded = render_frame(1.0, true).await;
            assert_eq!(gated.len(), recorded.len(), "same target, same byte count");
            let differing = gated.iter().zip(&recorded).filter(|(a, b)| a != b).count();
            assert_eq!(
                differing, 0,
                "{differing} bytes differ between a frame with the point-shadow passes recorded                  and one with them skipped — this scene has no point light, so they cannot be                  observable; a gate was put on a pass something samples"
            );
            // And the frame is a real render, not two identical blank targets.
            assert!(
                gated.as_chunks::<4>().0.iter().any(|p| p[..3] != gated[..3]),
                "the frame is uniform — nothing was drawn, so the comparison proves nothing"
            );
        });
    }

    /// Which screen-space pass to switch off for a comparison render.
    #[derive(Clone, Copy, PartialEq)]
    enum Effect {
        None,
        Ssgi,
        Ssr,
        /// The same two passes switched off the *reversible* way — the state stays built, the
        /// `enabled` flag is cleared. A frame from these must be identical to a frame from the
        /// destructive variant above, or the flag is not a real off switch.
        SsgiDisabled,
        SsrDisabled,
        /// SSR with its parameters written out by hand as the literals that used to stand in
        /// `ssr.wgsl`. Must be pixel-identical to the defaults.
        SsrLiterals,
        /// SSR whose roughness cut-out rejects every surface. Nothing may reflect.
        SsrNoRough,
        /// SSR whose surfaces are assumed 1 mm thick, so no depth difference counts as a hit.
        SsrThin,
    }

    /// A lit cube standing on a wide mirror floor, seen from a raised camera — a scene where a
    /// screen-space gather has something to find: a smooth surface to reflect in, and a bright
    /// neighbour on screen to bounce from. `disabled` switches one pass off so a caller can
    /// compare like with like.
    async fn render_mirror_scene(disabled: Effect) -> Vec<u8> {
        const W: u32 = 128;
        const H: u32 = 128;

        let mut renderer = crate::test_gpu::headless_renderer(W, H).await;
        match disabled {
            Effect::None => {}
            Effect::Ssgi => renderer.ssgi = None,
            Effect::Ssr => renderer.ssr = None,
            Effect::SsgiDisabled => {
                renderer.ssgi.as_mut().expect("ssgi built").enabled = false;
            }
            Effect::SsrDisabled => {
                renderer.ssr.as_mut().expect("ssr built").enabled = false;
            }
            Effect::SsrLiterals => {
                let p = &mut renderer.ssr.as_mut().expect("ssr built").params;
                p.roughness_cutoff = 0.5;
                p.fade_start = 0.1;
                p.fade_end = 0.5;
                p.step_size = 1.0;
                p.max_steps = 20.0;
                p.thickness = 1.0;
                p.start_offset = 0.1;
                p.edge_fade = 0.1;
            }
            Effect::SsrNoRough => {
                let p = &mut renderer.ssr.as_mut().expect("ssr built").params;
                p.roughness_cutoff = 0.0;
            }
            Effect::SsrThin => {
                let p = &mut renderer.ssr.as_mut().expect("ssr built").params;
                p.thickness = 0.001;
            }
        }
        let mut asset_manager = AssetManager::new();
        let mut world = World::new();

        let tex = asset_manager.create_white_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
        );

        // Ground: a flattened cube, top face at y = -1, and polished — SSR rejects anything
        // rougher than 0.5 outright, so a matte floor would measure the fixture, not the pass.
        let floor = world.spawn();
        world.add_component(
            floor,
            Transform::new(Vec3::new(0.0, -1.2, 0.0)).with_scale(Vec3::new(20.0, 0.2, 20.0)),
        );
        world.add_component(floor, GlobalTransform::default());
        world.add_component(floor, AssetManager::create_cube(&renderer.device));
        world.add_component(
            floor,
            Material::new(tex.clone()).with_pbr(Vec4::new(0.9, 0.9, 0.9, 1.0), 1.0, 0.05),
        );
        world.add_component(floor, MeshRenderer::new());

        // A unit cube whose bottom face lands exactly on the floor: both what the floor has to
        // reflect and what a hemisphere ray leaving the floor can land on.
        let cube = world.spawn();
        world.add_component(cube, Transform::new(Vec3::ZERO));
        world.add_component(cube, GlobalTransform::default());
        world.add_component(cube, AssetManager::create_cube(&renderer.device));
        world.add_component(
            cube,
            Material::new(tex).with_pbr(Vec4::new(0.9, 0.15, 0.15, 1.0), 0.0, 0.6),
        );
        world.add_component(cube, MeshRenderer::new());

        world.spawn_bundle(CameraBundle {
            position: Vec3::new(-5.0, 2.0, 0.0),
            yaw: 0.0,
            pitch: -0.35,
            primary: true,
            ..Default::default()
        });
        world.spawn_bundle(DirectionalLightBundle::default());

        render_world(&mut renderer, &mut world).await
    }

    /// **SSR and SSGI have to reach the frame.** Both march the G-buffer, and both tested its
    /// written-flag with a strict `> 0.5` while `gbuffer.wgsl` packs that flag as
    /// `(0.5 + 0.49·anisotropy) + floor(100·subsurface)` — **exactly 0.5** for an ordinary
    /// material, and exactly representable in the Rgba16Float target it is stored in. Every hit
    /// candidate was therefore rejected, both passes returned black for the whole frame, and
    /// their additive apply added nothing: measured on four different scenes, the picture was
    /// byte-identical with the pass running and with the pass removed (0 of 65536 bytes moved).
    ///
    /// Nothing else could have caught it. The states were constructed, the passes recorded and
    /// executed every frame, the shaders compiled — only the picture knew. Note the entry gates
    /// in the same two shaders (`w < 0.5` → skip) always agreed with the encoder; it was the
    /// inner hit test that did not, which is why the effects looked alive from every side but
    /// the output.
    ///
    /// The floors are far below what the fix measured (SSGI 2005, SSR 426 of 16384 pixels):
    /// this guards *that the pass contributes at all*, not any tuning of it.
    #[test]
    fn screen_space_reflections_and_gi_reach_the_frame() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available()) {
            eprintln!("skipping: no GPU adapter");
            return;
        }
        if pollster::block_on(Renderer::headless_adapter_is_software()) {
            eprintln!("skipping: software adapter");
            return;
        }
        pollster::block_on(async {
            let all_on = render_mirror_scene(Effect::None).await;
            for (name, effect, floor) in
                [("SSGI", Effect::Ssgi, 400usize), ("SSR", Effect::Ssr, 80usize)]
            {
                let off = render_mirror_scene(effect).await;
                let (changed, total) = changed_pixels(&all_on, &off, 128);
                assert!(
                    changed >= floor,
                    "removing {name} changed {changed}/{total} pixels — the pass runs but does \
                     not reach the frame (expected at least {floor})",
                );
            }
        });
    }

    /// The reversible off switch has to produce the *same* frame as the destructive one.
    ///
    /// Before this, the only way to switch a screen-space pass off was `renderer.ssr = None`,
    /// which frees the GPU objects: a comparison render could be turned off but not back on, and
    /// the engine's own tests above are written that way because there was nothing else. An
    /// `enabled: bool` is only worth having if it means exactly the same thing to the frame, so
    /// that is what this measures — pixel for pixel, not approximately.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn switching_an_effect_off_reversibly_renders_the_same_frame_as_destroying_it() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available()) {
            eprintln!("skipping: no GPU adapter");
            return;
        }
        if pollster::block_on(Renderer::headless_adapter_is_software()) {
            eprintln!("skipping: software adapter");
            return;
        }
        pollster::block_on(async {
            let all_on = render_mirror_scene(Effect::None).await;
            for (name, destroyed, disabled) in [
                ("SSGI", Effect::Ssgi, Effect::SsgiDisabled),
                ("SSR", Effect::Ssr, Effect::SsrDisabled),
            ] {
                let gone = render_mirror_scene(destroyed).await;
                let off = render_mirror_scene(disabled).await;

                // 1. The flag must reach the frame at all — otherwise an all-equal result below
                //    would pass for the wrong reason (the effect never showing up anywhere).
                let (reaches, _) = changed_pixels(&all_on, &off, 128);
                assert!(
                    reaches > 0,
                    "{name}: clearing `enabled` changed nothing — the flag is not consulted",
                );

                // 2. And the two ways of switching it off must agree exactly.
                let (differ, total) = changed_pixels(&gone, &off, 128);
                assert_eq!(
                    differ, 0,
                    "{name}: `enabled = false` and `= None` disagree on {differ}/{total} pixels — \
                     the reversible switch is not the same switch",
                );
            }
        });
    }

    /// Four tone-mapping curves, and the default is still the one every scene was authored under.
    ///
    /// ACES was hard-coded in `post_process.wgsl`. Selecting between curves is only safe if the
    /// default is bit-for-bit what it was — otherwise every existing game gets restyled by an
    /// upgrade — so that is checked as an identity, and the other three are checked to differ from
    /// it and from each other.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn the_tonemap_curves_differ_and_aces_is_unchanged() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        use gizmo_renderer::gpu_types::TonemapCurve;
        pollster::block_on(async {
            let aces = render_bright_scene(TonemapCurve::Aces).await;
            let none = render_bright_scene(TonemapCurve::None).await;
            let reinhard = render_bright_scene(TonemapCurve::Reinhard).await;
            let extended = render_bright_scene(TonemapCurve::ReinhardExtended).await;

            // Every pair must differ, or a curve is selecting nothing.
            for (an, a) in [
                ("None", &none),
                ("Reinhard", &reinhard),
                ("ReinhardExtended", &extended),
            ] {
                let (changed, total) = changed_pixels(&aces, a, 128);
                assert!(
                    changed >= 200,
                    "{an} differs from ACES on only {changed}/{total} pixels — the curve \
                     selector is not reaching the shader",
                );
            }
            let (r_vs_e, total) = changed_pixels(&reinhard, &extended, 128);
            assert!(
                r_vs_e >= 100,
                "Reinhard and its extended form differ on only {r_vs_e}/{total} pixels — the \
                 white point is not being applied",
            );

            // And the ordering the curves imply: on a bright frame, ACES holds highlights higher
            // than Reinhard, which pulls everything down. Directional, so a build that shuffled
            // the enum's numbering fails here rather than passing on "they differ".
            let mean = |f: &[u8]| -> f64 {
                let (px, _) = f.as_chunks::<4>();
                px.iter()
                    .map(|p| f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2]))
                    .sum::<f64>()
                    / (px.len().max(1) * 3) as f64
            };
            assert!(
                mean(&aces) > mean(&reinhard),
                "ACES ({:.1}) should hold a bright frame above Reinhard ({:.1})",
                mean(&aces),
                mean(&reinhard),
            );
        });
    }

    /// A brightly lit white floor — bright enough that the curve's shoulder is what decides the
    /// output. A dim scene would sit on the linear part of every curve and they would all agree.
    async fn render_bright_scene(curve: gizmo_renderer::gpu_types::TonemapCurve) -> Vec<u8> {
        const W: u32 = 128;
        let mut renderer = crate::test_gpu::headless_renderer(W, W).await;
        renderer.taa = None;
        renderer.tonemap_curve = curve;
        let mut asset_manager = AssetManager::new();
        let mut world = World::new();
        let tex = asset_manager.create_white_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
        );

        let floor = world.spawn();
        world.add_component(
            floor,
            Transform::new(Vec3::new(0.0, -1.0, 0.0)).with_scale(Vec3::new(14.0, 0.2, 14.0)),
        );
        world.add_component(floor, GlobalTransform::default());
        world.add_component(floor, AssetManager::create_cube(&renderer.device));
        world.add_component(
            floor,
            Material::new(tex).with_pbr(Vec4::new(1.0, 1.0, 1.0, 1.0), 0.8, 0.0),
        );
        world.add_component(floor, MeshRenderer::new());

        world.spawn_bundle(CameraBundle {
            position: Vec3::new(0.0, 1.5, 4.0),
            yaw: -std::f32::consts::FRAC_PI_2,
            pitch: -0.3,
            primary: true,
            ..Default::default()
        });
        // Deliberately over-bright: the point is to land past every curve's linear region.
        world.spawn_bundle(DirectionalLightBundle {
            intensity: 8.0,
            ..Default::default()
        });

        render_world(&mut renderer, &mut world).await
    }

    /// Moving SSR's constants out of the shader has to be a move, not an edit.
    ///
    /// Same reasoning as `the_defaults_are_the_shader_literals_they_replaced` does for volumetric:
    /// `SsrParams::default()` claims to be the values that stood in `ssr.wgsl`, and if the claim
    /// is wrong every reflective surface in every existing scene changed silently. Checked as an
    /// identity — the default march must be pixel-identical to one whose parameters are typed out
    /// by hand.
    ///
    /// Plus one field proved to reach the shader, because an all-equal result would otherwise pass
    /// for the wrong reason: a params buffer nobody reads.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn the_ssr_defaults_are_the_shader_literals() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        pollster::block_on(async {
            let defaults = render_mirror_scene(Effect::None).await;
            let literals = render_mirror_scene(Effect::SsrLiterals).await;
            let (differ, total) = changed_pixels(&defaults, &literals, 128);
            assert_eq!(
                differ, 0,
                "the defaults disagree with the shader literals on {differ}/{total} pixels — \
                 moving the numbers out of the shader changed what they render",
            );

            // Two fields proved to reach the shader, from opposite ends of the march: the
            // roughness cut-out, which decides whether a ray is cast at all, and the thickness,
            // which decides whether a hit counts.
            for (name, effect) in [
                ("roughness_cutoff", Effect::SsrNoRough),
                ("thickness", Effect::SsrThin),
            ] {
                let moved = render_mirror_scene(effect).await;
                let (changed, total) = changed_pixels(&defaults, &moved, 128);
                assert!(
                    changed > 0,
                    "{name} changed {changed}/{total} pixels — the params buffer is not being read",
                );
            }

            // `max_steps` is deliberately not among them, and that is a measurement rather than an
            // omission: on this scene the reflection is found on the **first** step, so cutting
            // the march from 20 to 4 — or to 1 — changes nothing. It is a range knob, and this
            // fixture has no range to lose. Asserting on it here would be asserting on the
            // fixture's geometry.
        });
    }

    /// Camera looking straight INTO the sun over open sky, with a pillar to break the beam.
    /// `DirectionalLightBundle::default()` is `rotation_x(-π/4)`, and `sun_dir = rot·(0,0,-1)`
    /// = (0, -0.707, -0.707), so the direction *toward* the sun is (0, +0.707, +0.707):
    /// yaw = π/2, pitch = π/4 aims the view ray exactly along it — `cos_theta = 1`, the peak of
    /// the Henyey-Greenstein lobe. Sky pixels also march the full 100-unit default instead of
    /// stopping a few metres away on the floor.
    async fn render_sunbeam(volumetric: bool) -> Vec<u8> {
        render_sunbeam_with(volumetric, |_| {}).await
    }

    /// The same scene, with a chance to reshape the march before it runs. `tweak` is handed
    /// `VolumetricParams` — the six numbers that used to be shader literals.
    async fn render_sunbeam_with(
        volumetric: bool,
        tweak: impl FnOnce(&mut gizmo_renderer::volumetric::VolumetricParams),
    ) -> Vec<u8> {
        render_sunbeam_full(volumetric, false, tweak).await
    }

    /// As above, plus an optional point light standing in the beam — `bulb_scatter` is the one
    /// parameter the sun alone cannot exercise, since the bulb loop only runs for point and spot
    /// lights.
    async fn render_sunbeam_full(
        volumetric: bool,
        bulb: bool,
        tweak: impl FnOnce(&mut gizmo_renderer::volumetric::VolumetricParams),
    ) -> Vec<u8> {
        const W: u32 = 128;
        let mut renderer = crate::test_gpu::headless_renderer(W, W).await;
        if let Some(vol) = renderer.volumetric.as_mut() {
            tweak(&mut vol.params);
        }
        if !volumetric {
            renderer.volumetric = None;
        }
        let mut asset_manager = AssetManager::new();
        let mut world = World::new();
        let tex = asset_manager.create_white_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
        );

        let floor = world.spawn();
        world.add_component(
            floor,
            Transform::new(Vec3::new(0.0, -2.0, 0.0)).with_scale(Vec3::new(40.0, 0.2, 40.0)),
        );
        world.add_component(floor, GlobalTransform::default());
        world.add_component(floor, AssetManager::create_cube(&renderer.device));
        world.add_component(
            floor,
            Material::new(tex.clone()).with_pbr(Vec4::new(0.5, 0.5, 0.5, 1.0), 0.0, 0.9),
        );
        world.add_component(floor, MeshRenderer::new());

        // A slab standing between the camera and the sun: what the cascades have to shadow if
        // the march is to produce a shaft rather than a flat haze.
        let pillar = world.spawn();
        world.add_component(
            pillar,
            Transform::new(Vec3::new(0.0, 2.0, 4.0)).with_scale(Vec3::new(4.0, 4.0, 0.4)),
        );
        world.add_component(pillar, GlobalTransform::default());
        world.add_component(pillar, AssetManager::create_cube(&renderer.device));
        world.add_component(
            pillar,
            Material::new(tex).with_pbr(Vec4::new(0.2, 0.2, 0.2, 1.0), 0.0, 0.9),
        );
        world.add_component(pillar, MeshRenderer::new());

        world.spawn_bundle(CameraBundle {
            position: Vec3::new(0.0, 1.0, -6.0),
            yaw: std::f32::consts::FRAC_PI_2,
            pitch: std::f32::consts::FRAC_PI_4,
            primary: true,
            ..Default::default()
        });
        world.spawn_bundle(DirectionalLightBundle::default());
        if bulb {
            // A bulb standing between the camera and the pillar, well inside its own radius so the
            // march passes through it. `bulb_scatter` is the one parameter the sun cannot exercise:
            // the loop it scales runs only for point and spot lights.
            world.spawn_bundle(crate::bundles::PointLightBundle {
                position: Vec3::new(0.0, 1.0, 2.0),
                color: Vec3::new(1.0, 0.85, 0.6),
                intensity: 20.0,
                radius: 14.0,
            });
        }

        render_world(&mut renderer, &mut world).await
    }

    /// **The god rays have to reach the frame too.** Volumetric was nearly written off with the
    /// two passes above: on the scenes that answered the SSR/SSGI question it moved bytes but
    /// nothing a person could see (max delta 5–8 of 255), which reads exactly like a pass that
    /// runs and contributes nothing. It was the fixture. `sun_intensity · phase · march length`
    /// is the whole contribution, and all three collapse when the camera does not face the sun:
    /// the Henyey-Greenstein lobe at `g = 0.55` is 0.61 straight into the sun and 0.015 away from
    /// it — a factor of 40 — and a view ray that lands on nearby geometry marches 6 units instead
    /// of the sky's 100. Aimed properly it moves 14.5 % of the frame with a max delta of 22.
    ///
    /// So this guards volumetric on the only ground where the question is meaningful, and it is
    /// the standing counter-example to reading one scene's zero as a dead pass.
    #[test]
    fn volumetric_god_rays_reach_the_frame() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        pollster::block_on(async {
            let on = render_sunbeam(true).await;
            let off = render_sunbeam(false).await;
            let (changed, total) = changed_pixels(&on, &off, 128);
            assert!(
                changed >= 400,
                "removing the volumetric pass changed {changed}/{total} pixels with the camera \
                 pointed into the sun — the god rays are not reaching the frame (measured 2376)",
            );
        });
    }

    /// Moving a constant out of a shader has to be a move, not an edit.
    ///
    /// The six volumetric numbers were literals in `volumetric.wgsl`; `VolumetricParams::default()`
    /// claims to be those same values. If the claim is wrong the effect changed silently for every
    /// existing scene, so it is checked the only way that means anything: the default march must be
    /// pixel-identical to a march whose parameters are written out by hand as the literals that
    /// stood in the shader.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn the_defaults_are_the_shader_literals_they_replaced() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        pollster::block_on(async {
            let defaults = render_sunbeam(true).await;
            let literals = render_sunbeam_with(true, |p| {
                p.phase_g = 0.55;
                p.steps = 16.0;
                p.max_distance = 100.0;
                p.sun_scatter = 0.0015;
                p.bulb_scatter = 0.0008;
                p.shadow_bias = 0.16;
            })
            .await;
            let (differ, total) = changed_pixels(&defaults, &literals, 128);
            assert_eq!(
                differ, 0,
                "the defaults disagree with the shader literals on {differ}/{total} pixels — \
                 moving the numbers out of the shader changed what they render",
            );
        });
    }

    /// Every one of the six has to actually reach the frame, or it is a field, not a control.
    ///
    /// Each parameter needs the scene that can show it, and finding those was most of the work:
    ///
    /// * `bulb_scatter` scales a loop that runs only for point and spot lights, so the sun alone
    ///   cannot move it. Measured 0/16384 without a lamp and **3590** with one.
    /// * `shadow_bias` has to be pushed *negative*. Raising it changes nothing because this scene
    ///   has no volumetric shadow to begin with — every march sample is already lit, and "even
    ///   more lit" is not a state. Measured 0 at +8 and **2379** at −40, which is also what proves
    ///   the shadow branch is entered at all.
    /// * `steps` is the odd one, and it is handled in its own test below, because measuring it
    ///   with this one's threshold gives a confident zero for the wrong reason.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn every_volumetric_parameter_changes_the_frame() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        pollster::block_on(async {
            let base = render_sunbeam(true).await;
            for (name, floor, tweak) in [
                (
                    "phase_g",
                    600,
                    Box::new(|p: &mut gizmo_renderer::volumetric::VolumetricParams| p.phase_g = 0.0)
                        as Box<dyn FnOnce(&mut gizmo_renderer::volumetric::VolumetricParams)>,
                ),
                (
                    "max_distance",
                    150,
                    Box::new(|p: &mut gizmo_renderer::volumetric::VolumetricParams| {
                        p.max_distance = 5.0
                    }),
                ),
                (
                    "sun_scatter",
                    1200,
                    Box::new(|p: &mut gizmo_renderer::volumetric::VolumetricParams| {
                        p.sun_scatter = 0.006
                    }),
                ),
                (
                    "shadow_bias",
                    1000,
                    Box::new(|p: &mut gizmo_renderer::volumetric::VolumetricParams| {
                        p.shadow_bias = -40.0
                    }),
                ),
            ] {
                let moved = render_sunbeam_with(true, tweak).await;
                let (changed, total) = changed_pixels(&base, &moved, 128);
                assert!(
                    changed >= floor,
                    "{name} changed {changed}/{total} pixels, expected at least {floor} — the \
                     field is not reaching the shader",
                );
            }

            // The bulb loop needs a bulb. Both runs have the lamp; the only difference is the
            // coefficient, so a lit scene cannot pass this by being lit.
            let bulb_base = render_sunbeam_full(true, true, |_| {}).await;
            let bulb_moved = render_sunbeam_full(true, true, |p| p.bulb_scatter = 0.05).await;
            let (changed, total) = changed_pixels(&bulb_base, &bulb_moved, 128);
            assert!(
                changed >= 1500,
                "bulb_scatter changed {changed}/{total} pixels with a point light in the beam, \
                 expected at least 1500 (measured 3590)",
            );
        });
    }

    /// `steps` is a cost knob, not a look knob, and the measurement has to be told that.
    ///
    /// Scattering accumulates as `Σ contribution × step_size` with `step_size = max_distance /
    /// steps`, so over a stretch where the contribution is constant the sum is `contribution ×
    /// max_distance` — **identical for any step count**. Step count only changes how finely a
    /// *varying* contribution is sampled, which is accuracy, not value.
    ///
    /// So `changed_pixels` reports 0/16384 for 4 steps against 64, at its 8-per-channel threshold,
    /// and that zero means "the difference is small", not "the field is dead". Read without the
    /// threshold the same pair differs on **2755** pixels with a largest channel delta of **6**.
    /// This test asserts the shape that is actually true: many pixels move, none of them far.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn the_step_count_refines_the_march_without_changing_what_it_renders() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        pollster::block_on(async {
            // With the lamp: its sphere is where the contribution varies along the ray, so this is
            // where step count can matter at all.
            let coarse = render_sunbeam_full(true, true, |p| p.steps = 4.0).await;
            let fine = render_sunbeam_full(true, true, |p| p.steps = 64.0).await;

            let (coarse_px, _) = coarse.as_chunks::<4>();
            let (fine_px, _) = fine.as_chunks::<4>();
            let differing = coarse_px
                .iter()
                .zip(fine_px)
                .filter(|(a, b)| (0..3).any(|c| a[c] != b[c]))
                .count();
            let largest = coarse_px
                .iter()
                .zip(fine_px)
                .flat_map(|(a, b)| (0..3).map(move |c| a[c].abs_diff(b[c])))
                .max()
                .unwrap_or(0);

            assert!(
                differing >= 1000,
                "4 steps and 64 steps differ on only {differing} pixels — `steps` is not reaching \
                 the shader (measured 2755)",
            );
            assert!(
                largest <= 24,
                "4 steps and 64 steps differ by up to {largest} per channel — refining the march \
                 should not change what it renders this much (measured 6)",
            );
        });
    }

    /// Two cameras, one encoder, one submit — which is what `SceneView` exists for.
    ///
    /// Camera matrices used to live in one buffer written with `queue.write_buffer`, and those
    /// writes order against *submission*, not against recording. So two passes recorded into one
    /// encoder both read whichever camera was written last, and the only workaround was an encoder
    /// and a submit per camera. This renders the same world twice into two targets from two
    /// cameras in one encoder, and requires the two targets to differ.
    ///
    /// It also pins the failure it replaces: the same pair of passes *without* a second view must
    /// come out identical. Without that half, a test that only asserts "they differ" would pass on
    /// a build where the second camera happened to be sampled by luck.
    ///
    /// The light casts shadows, and that is not incidental. It used to be `Generic`, because
    /// `SceneView` separated the camera uniform while the cascades and cluster table — *derived*
    /// from the camera into their own buffers — stayed shared, so with a sun the two passes
    /// contaminated each other and the offscreen target's lower band read 86.14 against the
    /// per-camera-submit workaround's 63.07. Per-view derived state (2026-08-24) brought it to
    /// **62.82**, so this now guards the scene that actually exercised the gap.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn two_cameras_in_one_encoder_render_two_different_frames() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        pollster::block_on(async {
            // The reference: the first camera drawn by itself, nothing to collide with. A correct
            // two-pass run has to reproduce this exactly, and "the two passes differ from each
            // other" — this test's original claim — does not imply it.
            let alone = render_first_camera_alone().await;

            // Without a view, the FIRST pass must not match the reference. The trap stated as an
            // observation rather than assumed: both passes read whichever camera was written last,
            // so it is the first that renders the wrong one.
            let (a_shared, _) = render_two_cameras(false).await;
            let (trapped, total) = changed_pixels(&a_shared, &alone, 128);
            assert!(
                trapped >= 500,
                "sharing one camera uniform left the first pass only {trapped}/{total} pixels \
                 from the reference — the trap did not reproduce, so what follows proves nothing",
            );

            // With a view, exactly. This is the claim the camera uniform alone could not satisfy:
            // the cascade split is derived from the camera into its own uniform, and while that
            // stayed shared the first pass drew the other camera's shadows.
            let (a, b) = render_two_cameras(true).await;
            let (drift, total) = changed_pixels(&a, &alone, 128);
            assert_eq!(
                drift, 0,
                "the first pass differs from that camera rendered alone on {drift}/{total} \
                 pixels — something derived from the camera is still shared between the passes",
            );

            // And the two passes really are two different frames, so a build that rendered the
            // same camera twice could not pass the assertion above by accident.
            let (changed, total) = changed_pixels(&a, &b, 128);
            assert!(
                changed >= 1000,
                "the two passes differ on only {changed}/{total} pixels — the second camera is \
                 not reaching its own pass",
            );
        });
    }

    /// A camera drawn into `viewport` of the 128² target: placed at `x`, 6 m back, looking down −Z.
    fn split_camera(world: &mut World, x: f32, viewport: crate::renderer::components::Viewport, primary: bool) {
        let cam = world.spawn_bundle(CameraBundle {
            position: Vec3::new(x, 0.0, 6.0),
            yaw: -std::f32::consts::FRAC_PI_2,
            pitch: 0.0,
            primary,
            ..Default::default()
        });
        if let Some(mut c) = world.borrow_mut::<crate::renderer::components::Camera>().get_mut(cam.id()) {
            c.viewport = Some(viewport);
        }
    }

    /// Two cubes 40 m apart — red at x = −20, blue at x = +20 — under a sun, so that a camera in
    /// front of one cannot see the other: which colour a half of the frame shows says which camera
    /// drew it.
    fn split_screen_scene(renderer: &Renderer, world: &mut World) {
        let mut assets = AssetManager::new();
        let tex = assets.create_white_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
        );
        for (x, colour) in [(-20.0, Vec4::new(0.9, 0.05, 0.05, 1.0)), (20.0, Vec4::new(0.05, 0.05, 0.9, 1.0))] {
            let cube = world.spawn();
            world.add_component(cube, Transform::new(Vec3::new(x, 0.0, 0.0)));
            world.add_component(cube, GlobalTransform::default());
            world.add_component(cube, AssetManager::create_cube(&renderer.device));
            world.add_component(cube, Material::new(tex.clone()).with_pbr(colour, 0.5, 0.0));
            world.add_component(cube, MeshRenderer::new());
        }
        world.spawn_bundle(DirectionalLightBundle { intensity: 3.0, ..Default::default() });
    }

    /// `(red, blue, red bounding box [x0, y0, x1, y1])` over the pixels of the 128²-frame with
    /// `x0 <= x < x1`.
    fn split_colours(frame: &[u8], x0: usize, x1: usize) -> (usize, usize, [usize; 4]) {
        let (mut red, mut blue) = (0, 0);
        let mut bbox = [usize::MAX, usize::MAX, 0, 0];
        for y in 0..128usize {
            for x in x0..x1 {
                let p = &frame[(y * 128 + x) * 4..][..3];
                let (r, g, b) = (p[0] as i32, p[1] as i32, p[2] as i32);
                if r > b + 40 && r > g + 40 {
                    red += 1;
                    bbox = [bbox[0].min(x), bbox[1].min(y), bbox[2].max(x + 1), bbox[3].max(y + 1)];
                }
                if b > r + 40 && b > g + 20 {
                    blue += 1;
                }
            }
        }
        (red, blue, bbox)
    }

    /// **Split screen:** two cameras with viewports, the left and right halves, each draws its own
    /// cube into its own half — and at its half's aspect ratio, so a cube seen face-on stays
    /// square instead of being squeezed to half its width.
    ///
    /// Discriminating three ways. Writing the whole target from every view (no region) leaves
    /// only the last camera's picture, so the left half shows blue. Recording both views into one
    /// submission draws the left view with the instance list the right view culled last — no red
    /// cube at all. And building the projection at the target's aspect instead of the half's
    /// gives a red box half as wide as it is tall.
    ///
    /// Then TAA on: each view keeps its own history (`Renderer::with_view_history`). With one
    /// history between the two, every frame blends the other camera's picture into this one —
    /// measured, though, TAA's neighbourhood clamp hides most of that (the left half's red count
    /// drops 2067 → 1933, too close to threshold reliably), so the test pins the cause instead:
    /// the renderer's own TAA has stepped once per frame. Shared, both views step it — twice per
    /// frame — which is also what sends each view's jitter through every other Halton sample.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn split_screen_draws_each_camera_into_its_own_half() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        use crate::renderer::components::Viewport;
        pollster::block_on(async {
            let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
            renderer.taa = None;
            renderer.ssgi = None;
            let mut world = World::new();
            split_screen_scene(&renderer, &mut world);
            split_camera(&mut world, -20.0, Viewport::left_half(), true);
            split_camera(&mut world, 20.0, Viewport::right_half(), false);
            let frame = render_world(&mut renderer, &mut world).await;

            let (left_red, left_blue, bbox) = split_colours(&frame, 0, 64);
            let (right_red, right_blue, _) = split_colours(&frame, 64, 128);
            assert!(left_red >= 150, "the left half shows only {left_red} red pixels — camera A is not in it");
            assert!(right_blue >= 150, "the right half shows only {right_blue} blue pixels — camera B is not in it");
            assert!(left_blue <= 5 && right_red <= 5, "colours crossed halves: {left_blue} blue left, {right_red} red right");
            let (w, h) = ((bbox[2] - bbox[0]) as f32, (bbox[3] - bbox[1]) as f32);
            assert!(
                (0.8..=1.25).contains(&(w / h)),
                "a face-on cube is {w}×{h} px in its half — the projection is not using the half's aspect",
            );

            // TAA on: four frames, each view with its own history.
            let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
            renderer.ssgi = None;
            let mut frame = Vec::new();
            for _ in 0..4 {
                frame = render_world(&mut renderer, &mut world).await;
            }
            let (left_red, left_blue, _) = split_colours(&frame, 0, 64);
            let (right_red, right_blue, _) = split_colours(&frame, 64, 128);
            let steps = renderer.taa.as_ref().map(|t| t.frame_index);
            assert_eq!(steps, Some(4), "four split-screen frames stepped view 0's TAA {steps:?} times — the views share it");
            assert!(left_red >= 150 && right_blue >= 150, "with TAA: {left_red} red left, {right_blue} blue right");
            assert!(
                left_blue <= 5 && right_red <= 5,
                "with TAA the views bleed into each other ({left_blue} blue left, {right_red} red \
                 right) — they share one history",
            );
        });
    }

    /// **CAS reaches the frame and gives back what TAA blurs.** Four frames of the split-screen
    /// scene with TAA on, then the same with CAS: the summed horizontal gradient — how much edge the
    /// picture has — must rise, and nothing else may: the pass is opt-in, so with it present but
    /// disabled the frame must be identical to a renderer without it.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn cas_sharpens_the_frame_and_changes_nothing_while_off() {
        use crate::renderer::components::Viewport;
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        fn edge(frame: &[u8]) -> u64 {
            let mut sum = 0u64;
            for y in 0..128usize {
                for x in 1..128usize {
                    let (a, b) = ((y * 128 + x - 1) * 4, (y * 128 + x) * 4);
                    sum += (0..3).map(|c| frame[a + c].abs_diff(frame[b + c]) as u64).sum::<u64>();
                }
            }
            sum
        }
        pollster::block_on(async {
            let mut frames = Vec::new();
            for mode in 0..3 {
                let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
                renderer.ssgi = None;
                match mode {
                    0 => renderer.cas = None,
                    1 => {} // present, disabled
                    _ => {
                        if let Some(cas) = renderer.cas.as_mut() {
                            cas.enabled = true;
                            cas.set_sharpness(&renderer.queue, 1.0);
                        }
                    }
                }
                let mut world = World::new();
                split_screen_scene(&renderer, &mut world);
                split_camera(&mut world, -20.0, Viewport::FULL, true);
                let mut frame = Vec::new();
                for _ in 0..4 {
                    frame = render_world(&mut renderer, &mut world).await;
                }
                frames.push(frame);
            }
            let (off_drift, total) = changed_pixels(&frames[0], &frames[1], 128);
            assert_eq!(off_drift, 0, "CAS present but disabled changed {off_drift}/{total} pixels");
            let (soft, sharp) = (edge(&frames[0]), edge(&frames[2]));
            assert!(
                sharp as f64 > soft as f64 * 1.05,
                "with CAS the frame's edge sum is {sharp}, without {soft} — it is not sharpening",
            );
        });
    }

    /// **Auto exposure closes the loop**: the same sunlit floor under a sun ten times brighter.
    /// Measured mean brightness: 232 against 119 without it (1.95×, the bright one held down only
    /// by tone mapping), 171 against 135 with it (1.27× — the rest is light that does not scale
    /// with the sun). And while it is present but off, the frame is bit-identical to a renderer
    /// without it: the composite multiplies by a buffer holding exactly 1.0.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn auto_exposure_evens_out_a_bright_and_a_dark_scene() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        fn mean(frame: &[u8]) -> f64 {
            let (px, _) = frame.as_chunks::<4>();
            px.iter().map(|p| (p[0] as f64 + p[1] as f64 + p[2] as f64) / 3.0).sum::<f64>() / px.len() as f64
        }
        pollster::block_on(async {
            // mode 0: no auto exposure at all · 1: present, disabled · 2: enabled.
            let mut means = [[0.0f64; 2]; 3];
            let mut frames: Vec<Vec<u8>> = Vec::new();
            for (mode, mode_means) in means.iter_mut().enumerate() {
                for (i, sun) in [8.0f32, 0.8].into_iter().enumerate() {
                    let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
                    renderer.taa = None;
                    renderer.ssgi = None;
                    match mode {
                        0 => renderer.auto_exposure = None,
                        1 => {}
                        _ => {
                            if let Some(ae) = renderer.auto_exposure.as_mut() {
                                ae.enabled = true;
                            }
                        }
                    }
                    // A sunlit floor filling the frame, no sky in view: the sun is what the frame's
                    // brightness is made of. (Looking at the horizon instead, the sky dominates and
                    // a sun ten times brighter moved the mean only 1.17×.)
                    let mut world = World::new();
                    let mut assets = AssetManager::new();
                    let tex = assets.create_white_texture(
                        &renderer.device,
                        &renderer.queue,
                        &renderer.scene.texture_bind_group_layout,
                    );
                    let floor = world.spawn();
                    world.add_component(
                        floor,
                        Transform::new(Vec3::new(0.0, -1.0, 0.0)).with_scale(Vec3::new(60.0, 0.2, 60.0)),
                    );
                    world.add_component(floor, GlobalTransform::default());
                    world.add_component(floor, AssetManager::create_cube(&renderer.device));
                    world.add_component(
                        floor,
                        Material::new(tex).with_pbr(Vec4::new(0.6, 0.6, 0.6, 1.0), 0.8, 0.0),
                    );
                    world.add_component(floor, MeshRenderer::new());
                    world.spawn_bundle(DirectionalLightBundle { intensity: sun, ..Default::default() });
                    world.spawn_bundle(CameraBundle {
                        position: Vec3::new(0.0, 3.0, 0.0),
                        yaw: -std::f32::consts::FRAC_PI_2,
                        pitch: -1.2,
                        primary: true,
                        ..Default::default()
                    });
                    let frame = render_world(&mut renderer, &mut world).await;
                    mode_means[i] = mean(&frame);
                    frames.push(frame);
                }
            }
            let (drift, total) = changed_pixels(&frames[0], &frames[2], 128);
            assert_eq!(drift, 0, "auto exposure present but off changed {drift}/{total} pixels");
            let off_ratio = means[0][0] / means[0][1];
            let on_ratio = means[2][0] / means[2][1];
            assert!(off_ratio > 1.5, "a sun ten times brighter only changed the frame {off_ratio:.2}× — the scene does not test anything");
            assert!(
                on_ratio < 1.0 + (off_ratio - 1.0) * 0.5,
                "with auto exposure the bright/dark ratio is {on_ratio:.2}, without {off_ratio:.2} — it is not evening them out",
            );
        });
    }

    /// **Camera motion blur** (`PostProcess::motion_blur`) smears what the camera's own motion moved
    /// and nothing else. Against the same frame with the shutter at 0:
    /// - a camera that turned since the last frame loses edge energy, and a half-open shutter
    ///   loses less than a fully open one;
    /// - a camera that did **not** move renders bit-identically. What that catches is a history
    ///   that is stale or carries TAA's jitter — not the matrix's direction: with no motion every
    ///   order of the pair is identity, and the kernel is symmetric, so a reversed reprojection
    ///   blurs exactly as much;
    /// - the first frame, with no last frame to compare, renders bit-identically too.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn motion_blur_smears_camera_motion_and_leaves_a_still_camera_alone() {
        use crate::renderer::components::{Camera, PostProcess};
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        // Squared, not absolute: a smeared edge spreads the same total step over more pixels, so
        // the sum of |differences| barely moves (measured: 4 %) while its square — how *steep*
        // the steps are — falls.
        fn edge(frame: &[u8]) -> u64 {
            let mut sum = 0u64;
            for y in 0..128usize {
                for x in 1..128usize {
                    let (a, b) = ((y * 128 + x - 1) * 4, (y * 128 + x) * 4);
                    sum += (0..3).map(|c| (frame[a + c].abs_diff(frame[b + c]) as u64).pow(2)).sum::<u64>();
                }
            }
            sum
        }
        // Two frames; between them the camera turns by `turn` radians. Returns both frames.
        async fn two_frames(shutter: f32, turn: f32) -> (Vec<u8>, Vec<u8>) {
            let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
            renderer.taa = None;
            renderer.ssgi = None;
            let mut world = World::new();
            split_screen_scene(&renderer, &mut world);
            let cam = world.spawn_bundle(CameraBundle {
                position: Vec3::new(-20.0, 0.0, 4.0),
                yaw: -std::f32::consts::FRAC_PI_2,
                primary: true,
                ..Default::default()
            });
            // No grain: it is added after the blur, everywhere, and would be most of the edge
            // energy measured — with it, the smear moved the sum only 5 %.
            let mut grade = PostProcess::default();
            grade.film_grain = 0.0;
            grade.motion_blur = shutter;
            world.add_component(cam, grade);
            let first = render_world(&mut renderer, &mut world).await;
            if let Some(mut c) = world.borrow_mut::<Camera>().get_mut(cam.id()) {
                c.yaw += turn;
            }
            let second = render_world(&mut renderer, &mut world).await;
            (first, second)
        }
        pollster::block_on(async {
            let (sharp_first, sharp_turned) = two_frames(0.0, 0.06).await;
            let (blur_first, blur_turned) = two_frames(1.0, 0.06).await;
            let (_, half_turned) = two_frames(0.5, 0.06).await;
            let (first_drift, total) = changed_pixels(&sharp_first, &blur_first, 128);
            assert_eq!(first_drift, 0, "the first frame has no last frame, yet the shutter changed {first_drift}/{total} pixels");
            let (sharp, smeared) = (edge(&sharp_turned), edge(&blur_turned));
            assert!(
                (smeared as f64) < sharp as f64 * 0.9,
                "a turning camera's frame keeps edge energy {smeared} with the shutter open, {sharp} closed — it is not blurred",
            );
            let half = edge(&half_turned);
            assert!(
                smeared < half && half < sharp,
                "edge energy should fall with the shutter: 0 → {sharp}, 0.5 → {half}, 1 → {smeared}",
            );

            let (_, sharp_still) = two_frames(0.0, 0.0).await;
            let (_, blur_still) = two_frames(1.0, 0.0).await;
            let (still_drift, total) = changed_pixels(&sharp_still, &blur_still, 128);
            assert_eq!(
                still_drift, 0,
                "a camera that did not move was smeared on {still_drift}/{total} pixels — the history is stale or carries something that is not motion",
            );
        });
    }

    /// A closed room around `centre`: six walls 5 m out, each its own colour (+X red, −X green,
    /// +Y blue, −Y yellow, +Z magenta, −Z cyan), plus a white quarter on the +X wall (y > 0,
    /// z > 0) and on the ceiling (x > 0, z > 0) — so a face mirrored or flipped puts the white in
    /// the wrong quarter — and a point light in the middle. Returns the albedo texture used.
    fn probe_room(renderer: &Renderer, world: &mut World, centre: Vec3) {
        let mut assets = AssetManager::new();
        let tex = assets.create_white_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
        );
        let mut slab = |pos: Vec3, scale: Vec3, colour: Vec4| {
            let e = world.spawn();
            world.add_component(e, Transform::new(centre + pos).with_scale(scale));
            world.add_component(e, GlobalTransform::default());
            world.add_component(e, AssetManager::create_cube(&renderer.device));
            world.add_component(e, Material::new(tex.clone()).with_pbr(colour, 0.9, 0.0));
            world.add_component(e, MeshRenderer::new());
        };
        // `create_cube` spans −1..1, so a scale is a HALF-extent: these walls are 10.4 m square
        // and 0.4 m thick, their inner faces 4.8 m out.
        let (d, big, thin) = (5.0, 5.2, 0.2);
        slab(Vec3::new(d, 0.0, 0.0), Vec3::new(thin, big, big), Vec4::new(0.9, 0.05, 0.05, 1.0));
        slab(Vec3::new(-d, 0.0, 0.0), Vec3::new(thin, big, big), Vec4::new(0.05, 0.9, 0.05, 1.0));
        slab(Vec3::new(0.0, d, 0.0), Vec3::new(big, thin, big), Vec4::new(0.05, 0.05, 0.9, 1.0));
        slab(Vec3::new(0.0, -d, 0.0), Vec3::new(big, thin, big), Vec4::new(0.9, 0.9, 0.05, 1.0));
        slab(Vec3::new(0.0, 0.0, d), Vec3::new(big, big, thin), Vec4::new(0.9, 0.05, 0.9, 1.0));
        slab(Vec3::new(0.0, 0.0, -d), Vec3::new(big, big, thin), Vec4::new(0.05, 0.9, 0.9, 1.0));
        // The white quarters, just inside their walls and starting half a metre off the axis, so
        // the axis direction itself meets plain wall rather than a quarter's edge.
        slab(Vec3::new(d - 0.35, 3.0, 3.0), Vec3::new(0.1, 2.5, 2.5), Vec4::new(1.0, 1.0, 1.0, 1.0));
        slab(Vec3::new(3.0, d - 0.35, 3.0), Vec3::new(2.5, 0.1, 2.5), Vec4::new(1.0, 1.0, 1.0, 1.0));
        world.spawn_bundle(crate::bundles::PointLightBundle {
            position: centre,
            color: Vec3::ONE,
            intensity: 60.0,
            radius: 30.0,
        });
    }

    /// Samples mip 0 of `cube` along each of `dirs` on the GPU, through the hardware's own cube
    /// addressing — the same lookup the lighting pass makes.
    fn sample_cube(renderer: &Renderer, cube: &wgpu::TextureView, dirs: &[Vec3]) -> Vec<[f32; 4]> {
        sample_cube_lod(renderer, cube, dirs, 0.0)
    }

    /// [`sample_cube`] at mip `lod`.
    fn sample_cube_lod(renderer: &Renderer, cube: &wgpu::TextureView, dirs: &[Vec3], lod: f32) -> Vec<[f32; 4]> {
        use wgpu::util::DeviceExt;
        let device = &renderer.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sample_cube"),
            source: wgpu::ShaderSource::Wgsl(
                "@group(0) @binding(0) var t: texture_cube<f32>;
                 @group(0) @binding(1) var s: sampler;
                 @group(0) @binding(2) var<storage, read> dirs: array<vec4<f32>>;
                 @group(0) @binding(3) var<storage, read_write> out: array<vec4<f32>>;
                 @compute @workgroup_size(1) fn main(@builtin(global_invocation_id) id: vec3<u32>) {
                     out[id.x] = textureSampleLevel(t, s, dirs[id.x].xyz, dirs[id.x].w);
                 }"
                .into(),
            ),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("sample_cube"),
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let data: Vec<[f32; 4]> = dirs.iter().map(|d| [d.x, d.y, d.z, lod]).collect();
        let dir_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&data),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let size = (dirs.len() * 16) as u64;
        let out_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(cube) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: dir_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: out_buf.as_entire_binding() },
            ],
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(dirs.len() as u32, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&out_buf, 0, &staging, 0, size);
        renderer.queue.submit(Some(encoder.finish()));
        let slice = staging.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        let out = bytemuck::cast_slice::<u8, [f32; 4]>(&slice.get_mapped_range().expect("mapped")).to_vec();
        out
    }

    /// **A reflection probe captures the room the right way round.** Sampled through the
    /// hardware's cube addressing, each axis direction reads its own wall's colour, and the white
    /// quarters land where they are — on the +X wall (a side face) and on the ceiling (+Y, whose
    /// "up" is a different axis) — and not in the mirrored or flipped quarter. A face rendered the
    /// wrong way round fails here even when its centre is right.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn a_reflection_probe_captures_each_direction_the_right_way_round() {
        use crate::renderer::components::ReflectionProbe;
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        pollster::block_on(async {
            let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
            renderer.taa = None;
            renderer.ssgi = None;
            let mut world = World::new();
            probe_room(&renderer, &mut world, Vec3::ZERO);
            world.spawn_bundle((Transform::new(Vec3::ZERO), GlobalTransform::default(), ReflectionProbe::new()));
            world.spawn_bundle(CameraBundle { position: Vec3::new(0.0, 0.0, 3.0), primary: true, ..Default::default() });
            render_world(&mut renderer, &mut world).await;
            assert!(
                world.borrow::<ReflectionProbe>().iter().all(|(_, p)| p.captured),
                "the probe did not capture on its first frame"
            );
            // Slot 0 of the cube array, as a plain cube.
            let cube = renderer.reflection_probe.as_ref().expect("a probe was captured").texture.create_view(
                &wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::Cube),
                    base_array_layer: 0,
                    array_layer_count: Some(6),
                    ..Default::default()
                },
            );
            let n = |x: f32, y: f32, z: f32| Vec3::new(x, y, z).normalize();
            let cases: [(&str, Vec3, &str); 12] = [
                ("+X", Vec3::X, "red"),
                ("-X", Vec3::NEG_X, "green"),
                ("+Y", Vec3::Y, "blue"),
                ("-Y", Vec3::NEG_Y, "yellow"),
                ("+Z", Vec3::Z, "magenta"),
                ("-Z", Vec3::NEG_Z, "cyan"),
                ("+X up +Z", n(1.0, 0.5, 0.5), "white"),
                ("+X up -Z", n(1.0, 0.5, -0.5), "red"),
                ("+X down +Z", n(1.0, -0.5, 0.5), "red"),
                ("+Y +X +Z", n(0.5, 1.0, 0.5), "white"),
                ("+Y -X +Z", n(-0.5, 1.0, 0.5), "blue"),
                ("+Y +X -Z", n(0.5, 1.0, -0.5), "blue"),
            ];
            let dirs: Vec<Vec3> = cases.iter().map(|c| c.1).collect();
            let got = sample_cube(&renderer, &cube, &dirs);
            let classify = |c: [f32; 4]| {
                let (r, g, b) = (c[0], c[1], c[2]);
                let m = r.max(g).max(b).max(1e-6);
                let (r, g, b) = (r / m > 0.5, g / m > 0.5, b / m > 0.5);
                match (r, g, b) {
                    (true, false, false) => "red",
                    (false, true, false) => "green",
                    (false, false, true) => "blue",
                    (true, true, false) => "yellow",
                    (true, false, true) => "magenta",
                    (false, true, true) => "cyan",
                    (true, true, true) => "white",
                    _ => "black",
                }
            };
            let wrong: Vec<String> = cases
                .iter()
                .zip(&got)
                .filter(|((_, _, want), c)| classify(**c) != *want)
                .map(|((name, _, want), c)| format!("{name}: want {want}, got {} {:?}", classify(*c), &c[..3]))
                .collect();
            assert!(wrong.is_empty(), "the probe's cube is the wrong way round:\n{}", wrong.join("\n"));

            // **The rough mips are prefiltered.** At the roughest mip a direction gathers light
            // from a wide lobe, so opposite walls' colours move toward each other and lose
            // saturation. A prefilter that only copied mip 0 down would leave them as far apart
            // and as saturated as at mip 0.
            let top = renderer.reflection_probe.as_ref().map(|g| g.mip_count - 1).unwrap() as f32;
            let axes = [Vec3::X, Vec3::NEG_X];
            let sharp = sample_cube_lod(&renderer, &cube, &axes, 0.0);
            let rough = sample_cube_lod(&renderer, &cube, &axes, top);
            let sat = |c: [f32; 4]| {
                let (hi, lo) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
                (hi - lo) / hi.max(1e-6)
            };
            let gap = |a: [f32; 4], b: [f32; 4]| {
                let (na, nb) = (a[0] + a[1] + a[2], b[0] + b[1] + b[2]);
                (0..3).map(|i| (a[i] / na.max(1e-6) - b[i] / nb.max(1e-6)).abs()).sum::<f32>()
            };
            assert!(
                sat(rough[0]) < sat(sharp[0]) * 0.8,
                "the roughest mip of +X is as saturated as mip 0 ({} vs {}) — the mips are not prefiltered",
                sat(rough[0]),
                sat(sharp[0])
            );
            assert!(
                gap(rough[0], rough[1]) < gap(sharp[0], sharp[1]) * 0.7,
                "opposite walls are as different at the roughest mip ({}) as at mip 0 ({})",
                gap(rough[0], rough[1]),
                gap(sharp[0], sharp[1])
            );
        });
    }

    /// **A mirror reflects the room, not the sky.** A metallic, nearly smooth cube faces the
    /// camera in the probe room; its front face reflects straight back, at the magenta wall behind
    /// the camera. With a probe the middle of that face is magenta; without one it reflects the
    /// procedural sky. And taking the probe away again restores the no-probe frame exactly — the
    /// specular term is back on the sky, bit for bit.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn a_mirror_reflects_the_probe_room_and_not_the_sky() {
        use crate::renderer::components::ReflectionProbe;
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        pollster::block_on(async {
            let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
            renderer.taa = None;
            renderer.ssgi = None;
            renderer.ssr = None;
            let mut world = World::new();
            probe_room(&renderer, &mut world, Vec3::ZERO);
            let mut assets = AssetManager::new();
            let tex = assets.create_white_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            );
            let mirror = world.spawn();
            world.add_component(mirror, Transform::new(Vec3::new(0.0, 0.0, -2.0)).with_scale(Vec3::splat(0.8)));
            world.add_component(mirror, GlobalTransform::default());
            world.add_component(mirror, AssetManager::create_cube(&renderer.device));
            world.add_component(mirror, Material::new(tex).with_pbr(Vec4::new(0.95, 0.95, 0.95, 1.0), 0.05, 1.0));
            world.add_component(mirror, MeshRenderer::new());
            world.spawn_bundle(CameraBundle {
                position: Vec3::new(0.0, 0.0, 2.0),
                yaw: -std::f32::consts::FRAC_PI_2,
                primary: true,
                ..Default::default()
            });
            // The mirror's face, minus its middle (the point light's highlight is there): is it
            // magenta — red and blue above green? Tone mapping lifts the reflected wall to a pale
            // pink, around (230, 190, 230), hence the modest margin.
            let magenta = |frame: &[u8]| {
                let mut n = 0;
                for y in 42..86usize {
                    for x in 42..86usize {
                        if (54..74).contains(&x) && (54..74).contains(&y) {
                            continue;
                        }
                        let p = &frame[(y * 128 + x) * 4..][..3];
                        if p[0] as i32 > p[1] as i32 + 15 && p[2] as i32 > p[1] as i32 + 15 {
                            n += 1;
                        }
                    }
                }
                n
            };
            let sky = render_world(&mut renderer, &mut world).await;
            let probe = world.spawn_bundle((Transform::new(Vec3::ZERO), GlobalTransform::default(), ReflectionProbe::new()));
            let room = render_world(&mut renderer, &mut world).await;
            world.despawn(probe);
            let back = render_world(&mut renderer, &mut world).await;

            assert!(magenta(&sky) <= 20, "without a probe the mirror already shows {} magenta pixels", magenta(&sky));
            assert!(magenta(&room) >= 1200, "with a probe the mirror shows only {}/1536 magenta pixels", magenta(&room));
            let (drift, total) = changed_pixels(&sky, &back, 128);
            assert_eq!(drift, 0, "removing the probe left {drift}/{total} pixels different from the no-probe frame");
        });
    }

    /// **Box-projected parallax.** The probe sits in the middle of the room; a mirror and the
    /// camera stand 3 m to its side. Behind the camera the +Z wall is white on the mirror's side
    /// (x > 0.5) and magenta in the middle. By direction alone the mirror sees what the *probe*
    /// sees straight ahead — magenta; traced to the room's box, it sees the wall it actually faces
    /// — white.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn a_probe_box_puts_the_reflection_where_the_mirror_faces() {
        use crate::renderer::components::ReflectionProbe;
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        async fn mirror_frame(influence: Option<Vec3>) -> Vec<u8> {
            let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
            renderer.taa = None;
            renderer.ssgi = None;
            renderer.ssr = None;
            let mut world = World::new();
            // Off the world origin, so a lookup that forgot to subtract the probe's position from the
            // traced point would aim somewhere else entirely.
            let c = Vec3::new(20.0, 0.0, 10.0);
            probe_room(&renderer, &mut world, c);
            let mut assets = AssetManager::new();
            let tex = assets.create_white_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            );
            let mut slab = |pos: Vec3, scale: Vec3, colour: Vec4, roughness: f32, metallic: f32| {
                let e = world.spawn();
                world.add_component(e, Transform::new(pos).with_scale(scale));
                world.add_component(e, GlobalTransform::default());
                world.add_component(e, AssetManager::create_cube(&renderer.device));
                world.add_component(e, Material::new(tex.clone()).with_pbr(colour, roughness, metallic));
                world.add_component(e, MeshRenderer::new());
            };
            // The +Z wall's x > 0.5 half, white, just inside it.
            slab(c + Vec3::new(3.0, 0.0, 4.65), Vec3::new(2.5, 5.0, 0.1), Vec4::ONE, 0.9, 0.0);
            // The mirror, 3 m off the probe.
            slab(c + Vec3::new(3.0, 0.0, -2.0), Vec3::splat(0.8), Vec4::new(0.95, 0.95, 0.95, 1.0), 0.05, 1.0);
            let mut probe = ReflectionProbe::new();
            probe.influence = influence;
            world.spawn_bundle((Transform::new(c), GlobalTransform::default(), probe));
            world.spawn_bundle(CameraBundle {
                position: c + Vec3::new(3.0, 0.0, 2.0),
                yaw: -std::f32::consts::FRAC_PI_2,
                primary: true,
                ..Default::default()
            });
            render_world(&mut renderer, &mut world).await
        }
        // (white, magenta) over the mirror's face, minus its middle.
        fn colours(frame: &[u8]) -> (usize, usize) {
            let (mut white, mut magenta) = (0, 0);
            for y in 42..86usize {
                for x in 42..86usize {
                    if (54..74).contains(&x) && (54..74).contains(&y) {
                        continue;
                    }
                    let p = &frame[(y * 128 + x) * 4..][..3];
                    let (r, g, b) = (p[0] as i32, p[1] as i32, p[2] as i32);
                    if r > g + 15 && b > g + 15 {
                        magenta += 1;
                    } else if r.min(g).min(b) > 150 && r.max(g).max(b) - r.min(g).min(b) < 25 {
                        white += 1;
                    }
                }
            }
            (white, magenta)
        }
        pollster::block_on(async {
            let (white_dir, magenta_dir) = colours(&mirror_frame(None).await);
            let (white_box, magenta_box) = colours(&mirror_frame(Some(Vec3::splat(4.8))).await);
            // Measured: by direction 1008 magenta / 528 white (the face's edges reflect at angles
            // that reach the white half even from the probe); with the box 1536 white / 0 magenta.
            assert!(
                magenta_dir > white_dir && magenta_dir >= 700,
                "by direction alone the mirror should mostly see the probe's straight-ahead magenta: {magenta_dir} magenta, {white_dir} white",
            );
            assert!(
                white_box >= 1400 && magenta_box <= 50,
                "with the room's box it should see the white it faces: {white_box} white, {magenta_box} magenta",
            );
        });
    }

    /// **A clear coat reflects the probe too.** A dark, rough, non-metallic cube in the probe room:
    /// its base reflects almost nothing sharp, so what a clear coat adds on top is a near-mirror
    /// reflection of the magenta wall behind the camera. With the coat the face turns magenta
    /// against the same cube without it; if the coat still read the procedural sky it would add a
    /// grey-blue sheen instead.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn a_clear_coat_reflects_the_probe_room() {
        use crate::renderer::components::ReflectionProbe;
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        async fn coated(coat: f32) -> f64 {
            let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
            renderer.taa = None;
            renderer.ssgi = None;
            renderer.ssr = None;
            let mut world = World::new();
            probe_room(&renderer, &mut world, Vec3::ZERO);
            let mut assets = AssetManager::new();
            let tex = assets.create_white_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            );
            let mut material = Material::new(tex).with_pbr(Vec4::new(0.04, 0.04, 0.04, 1.0), 0.9, 0.0);
            material.clear_coat = coat;
            let cube = world.spawn();
            world.add_component(cube, Transform::new(Vec3::new(0.0, 0.0, -2.0)).with_scale(Vec3::splat(0.8)));
            world.add_component(cube, GlobalTransform::default());
            world.add_component(cube, AssetManager::create_cube(&renderer.device));
            world.add_component(cube, material);
            world.add_component(cube, MeshRenderer::new());
            world.spawn_bundle((Transform::new(Vec3::ZERO), GlobalTransform::default(), ReflectionProbe::new()));
            world.spawn_bundle(CameraBundle {
                position: Vec3::new(0.0, 0.0, 2.0),
                yaw: -std::f32::consts::FRAC_PI_2,
                primary: true,
                ..Default::default()
            });
            let frame = render_world(&mut renderer, &mut world).await;
            // Mean magenta excess, (r + b)/2 − g, over the face minus its middle.
            let (mut sum, mut n) = (0.0f64, 0.0f64);
            for y in 42..86usize {
                for x in 42..86usize {
                    if (54..74).contains(&x) && (54..74).contains(&y) {
                        continue;
                    }
                    let p = &frame[(y * 128 + x) * 4..][..3];
                    sum += (p[0] as f64 + p[2] as f64) / 2.0 - p[1] as f64;
                    n += 1.0;
                }
            }
            sum / n
        }
        pollster::block_on(async {
            let (bare, coat) = (coated(0.0).await, coated(1.0).await);
            // Measured: bare 0.16, coated 1.79, and 0.17 with the coat still reading the sky. Small
            // in absolute terms — a coat reflects ~4 % head-on — but ten times the bare cube's and
            // nothing at all from the sky.
            assert!(
                coat > bare + 1.0,
                "a clear coat added {:.2} of magenta (bare {bare:.2}, coated {coat:.2}) — it is not reflecting the probe",
                coat - bare
            );
        });
    }

    /// **Two probes, two rooms.** Two closed rooms 30 m apart, each with its own boxed probe and a
    /// mirror facing the wall behind its camera — magenta in room A, painted white in room B. Each
    /// mirror must reflect its own room: room B's mirror reads white only if its pixels pick B's
    /// slot of the cube array, and reads sky (no box covers it) if only the first probe is used,
    /// or magenta if every probe samples slot 0.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn each_room_s_mirror_reflects_its_own_probe() {
        use crate::renderer::components::{Camera, ReflectionProbe};
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        pollster::block_on(async {
            let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
            renderer.taa = None;
            renderer.ssgi = None;
            renderer.ssr = None;
            let mut world = World::new();
            let (a, b) = (Vec3::ZERO, Vec3::new(30.0, 0.0, 0.0));
            probe_room(&renderer, &mut world, a);
            probe_room(&renderer, &mut world, b);
            let mut assets = AssetManager::new();
            let tex = assets.create_white_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            );
            let mut slab = |pos: Vec3, scale: Vec3, colour: Vec4, roughness: f32, metallic: f32| {
                let e = world.spawn();
                world.add_component(e, Transform::new(pos).with_scale(scale));
                world.add_component(e, GlobalTransform::default());
                world.add_component(e, AssetManager::create_cube(&renderer.device));
                world.add_component(e, Material::new(tex.clone()).with_pbr(colour, roughness, metallic));
                world.add_component(e, MeshRenderer::new());
            };
            // Room B's +Z wall, painted white over its whole inner face.
            slab(b + Vec3::new(0.0, 0.0, 4.65), Vec3::new(4.8, 4.8, 0.1), Vec4::ONE, 0.9, 0.0);
            for c in [a, b] {
                slab(c + Vec3::new(0.0, 0.0, -2.0), Vec3::splat(0.8), Vec4::new(0.95, 0.95, 0.95, 1.0), 0.05, 1.0);
            }
            for c in [a, b] {
                world.spawn_bundle((
                    Transform::new(c),
                    GlobalTransform::default(),
                    ReflectionProbe::new().with_influence(Vec3::splat(4.8)),
                ));
            }
            let cam_a = world.spawn_bundle(CameraBundle {
                position: a + Vec3::new(0.0, 0.0, 2.0),
                yaw: -std::f32::consts::FRAC_PI_2,
                primary: true,
                ..Default::default()
            });
            let cam_b = world.spawn_bundle(CameraBundle {
                position: b + Vec3::new(0.0, 0.0, 2.0),
                yaw: -std::f32::consts::FRAC_PI_2,
                primary: false,
                ..Default::default()
            });
            // (white, magenta) over the mirror's face, minus its middle.
            let colours = |frame: &[u8]| {
                let (mut white, mut magenta) = (0, 0);
                for y in 42..86usize {
                    for x in 42..86usize {
                        if (54..74).contains(&x) && (54..74).contains(&y) {
                            continue;
                        }
                        let p = &frame[(y * 128 + x) * 4..][..3];
                        let (r, g, b) = (p[0] as i32, p[1] as i32, p[2] as i32);
                        if r > g + 15 && b > g + 15 {
                            magenta += 1;
                        } else if r.min(g).min(b) > 150 && r.max(g).max(b) - r.min(g).min(b) < 25 {
                            white += 1;
                        }
                    }
                }
                (white, magenta)
            };
            // Mean magenta excess, (r + b)/2 − g, over the same ring: a fade is a matter of degree.
            let excess = |frame: &[u8]| {
                let (mut sum, mut n) = (0.0f64, 0.0f64);
                for y in 42..86usize {
                    for x in 42..86usize {
                        if (54..74).contains(&x) && (54..74).contains(&y) {
                            continue;
                        }
                        let p = &frame[(y * 128 + x) * 4..][..3];
                        sum += (p[0] as f64 + p[2] as f64) / 2.0 - p[1] as f64;
                        n += 1.0;
                    }
                }
                sum / n
            };
            let frame_a = render_world(&mut renderer, &mut world).await;
            let in_a = colours(&frame_a);
            let excess_in = excess(&frame_a);
            for (id, primary) in [(cam_a.id(), false), (cam_b.id(), true)] {
                if let Some(mut c) = world.borrow_mut::<Camera>().get_mut(id) {
                    c.primary = primary;
                }
            }
            let in_b = colours(&render_world(&mut renderer, &mut world).await);
            assert!(in_a.1 >= 1200, "room A's mirror should reflect its magenta wall: {in_a:?} (white, magenta)");
            assert!(in_b.0 >= 1200, "room B's mirror should reflect its white wall: {in_b:?} (white, magenta)");

            // **The fade, and a parameter edited after the capture.** The probes fading in over
            // 20 m instead of 0.5: room A's mirror, ~3.6 m inside its box, now weighs its probe at
            // under a fifth and the sky at the rest. Measured as mean magenta excess, not a pixel
            // count: the wall is so much brighter than the sky in HDR that a fifth of it still
            // tone-maps to pink — the fade shows as a fall in degree. (Room A, not B: the procedural
            // sky reflects a light grey the white test cannot tell from the white wall.) Nothing is
            // recaptured; the change has to reach the lighting pass on its own.
            for (_, mut p) in world.borrow_mut::<ReflectionProbe>().iter_mut() {
                p.blend_distance = 20.0;
            }
            for (id, primary) in [(cam_a.id(), true), (cam_b.id(), false)] {
                if let Some(mut c) = world.borrow_mut::<Camera>().get_mut(id) {
                    c.primary = primary;
                }
            }
            let excess_faded = excess(&render_world(&mut renderer, &mut world).await);
            // Measured: 39.17 fully in, 23.14 faded; 39.17 again with no fade, or with the edit
            // only uploaded on a capture.
            assert!(
                excess_faded < excess_in * 0.6,
                "with a 20 m fade room A's mirror keeps {excess_faded:.2} of magenta excess (fully in: \
                 {excess_in:.2}) — the fade or the edited parameter does not reach the lighting pass",
            );
        });
    }

    /// **Transparent surfaces reflect the probes too.** A half-transparent metal cube — drawn by the
    /// forward pass, not the deferred one — in the probe room, facing the magenta wall behind the
    /// camera. With a probe its face gains magenta; without one, the forward pass's hemispheric
    /// stand-in gives none of it.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn a_transparent_surface_reflects_the_probe_room() {
        use crate::renderer::components::ReflectionProbe;
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        async fn glass(with_probe: bool) -> f64 {
            let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
            renderer.taa = None;
            renderer.ssgi = None;
            renderer.ssr = None;
            let mut world = World::new();
            probe_room(&renderer, &mut world, Vec3::ZERO);
            let mut assets = AssetManager::new();
            let tex = assets.create_white_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            );
            let cube = world.spawn();
            world.add_component(cube, Transform::new(Vec3::new(0.0, 0.0, -2.0)).with_scale(Vec3::splat(0.8)));
            world.add_component(cube, GlobalTransform::default());
            world.add_component(cube, AssetManager::create_cube(&renderer.device));
            world.add_component(cube, Material::new(tex).with_pbr(Vec4::new(0.95, 0.95, 0.95, 0.5), 0.05, 1.0));
            world.add_component(cube, MeshRenderer::new());
            if with_probe {
                world.spawn_bundle((Transform::new(Vec3::ZERO), GlobalTransform::default(), ReflectionProbe::new()));
            }
            world.spawn_bundle(CameraBundle {
                position: Vec3::new(0.0, 0.0, 2.0),
                yaw: -std::f32::consts::FRAC_PI_2,
                primary: true,
                ..Default::default()
            });
            let frame = render_world(&mut renderer, &mut world).await;
            let (mut sum, mut n) = (0.0f64, 0.0f64);
            for y in 42..86usize {
                for x in 42..86usize {
                    if (54..74).contains(&x) && (54..74).contains(&y) {
                        continue;
                    }
                    let p = &frame[(y * 128 + x) * 4..][..3];
                    sum += (p[0] as f64 + p[2] as f64) / 2.0 - p[1] as f64;
                    n += 1.0;
                }
            }
            sum / n
        }
        pollster::block_on(async {
            let (bare, probed) = (glass(false).await, glass(true).await);
            // Measured: 0.38 without, 4.80 with, and 0.39 with the forward shader ignoring the
            // probes. Small — the forward pass's own IBL term is a light one and the cube is half
            // see-through — but twelve times the bare figure.
            assert!(
                probed > bare + 2.0,
                "a transparent metal face gained {:.2} of magenta from the probe (without {bare:.2}, with \
                 {probed:.2}) — the forward pass is not reading it",
                probed - bare
            );
        });
    }

    /// **A bigger sun casts softer shadows.** A 3 m pillar under a 45° sun throws a long shadow; the
    /// camera looks straight down on it. The far end of that shadow is ~4 m from the pillar, where a
    /// penumbra `2·d·tan θ` wide is widest. Counted as the pixels whose brightness lies between the
    /// frame's shadow and lit levels: `angular_radius` 0 (the default) keeps the fixed near-crisp
    /// edge; 0.02 and 0.06 rad must widen it, and in that order.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn a_bigger_sun_casts_softer_shadows() {
        use crate::renderer::components::{DirectionalLight, LightRole};
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        async fn penumbra_pixels(angular_radius: f32) -> usize {
            let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
            renderer.taa = None;
            renderer.ssgi = None;
            renderer.ssao = None;
            let mut world = World::new();
            let mut assets = AssetManager::new();
            let tex = assets.create_white_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            );
            for (pos, scale) in [
                (Vec3::new(0.0, -0.2, 0.0), Vec3::new(20.0, 0.2, 20.0)),
                (Vec3::new(0.0, 1.5, 0.0), Vec3::new(0.3, 1.5, 0.3)),
            ] {
                let e = world.spawn();
                world.add_component(e, Transform::new(pos).with_scale(scale));
                world.add_component(e, GlobalTransform::default());
                world.add_component(e, AssetManager::create_cube(&renderer.device));
                world.add_component(e, Material::new(tex.clone()).with_pbr(Vec4::new(0.8, 0.8, 0.8, 1.0), 0.9, 0.0));
                world.add_component(e, MeshRenderer::new());
            }
            let sun_dir = Vec3::new(1.0, -1.0, 0.0).normalize();
            let sun = world.spawn();
            world.add_component(
                sun,
                Transform::new(Vec3::ZERO).with_rotation(crate::math::Quat::from_rotation_arc(Vec3::NEG_Z, sun_dir)),
            );
            world.add_component(sun, GlobalTransform::default());
            world.add_component(
                sun,
                DirectionalLight::new(Vec3::ONE, 3.0, LightRole::Sun).with_angular_radius(angular_radius),
            );
            world.spawn_bundle(CameraBundle {
                position: Vec3::new(2.0, 7.0, 0.0),
                yaw: 0.0,
                pitch: -std::f32::consts::FRAC_PI_2 + 0.01,
                primary: true,
                ..Default::default()
            });
            let frame = render_world(&mut renderer, &mut world).await;
            let luma: Vec<f32> = frame.as_chunks::<4>().0.iter().map(|p| (p[0] as f32 + p[1] as f32 + p[2] as f32) / 3.0).collect();
            let (lo, hi) = luma.iter().fold((f32::MAX, 0.0f32), |(lo, hi), &v| (lo.min(v), hi.max(v)));
            let span = hi - lo;
            // Only along the shadow (rows 40..80, columns 40..90 of the 128² frame): counted over
            // the whole frame the umbra shrinking as the penumbra grows hides most of the change.
            (40..80usize)
                .flat_map(|y| (40..90usize).map(move |x| y * 128 + x))
                .filter(|&i| luma[i] > lo + 0.2 * span && luma[i] < hi - 0.2 * span)
                .count()
        }
        pollster::block_on(async {
            let (sharp, mid, soft) = (penumbra_pixels(0.0).await, penumbra_pixels(0.02).await, penumbra_pixels(0.06).await);
            // Measured: 18 / 91 / 196. With the shader ignoring the radius, or reading the cascade's
            // scale from `m[0][0]` alone (near zero under a 45° sun), all three are 18. The 128-texel
            // cap is a quality bound the test does not pin: at 16 the soft case reads 165.
            assert!(
                soft > sharp * 2 && mid > sharp && soft > mid,
                "penumbra pixels should grow with the sun: 0 → {sharp}, 0.02 → {mid}, 0.06 → {soft}",
            );
        });
    }

    /// **Contact shadows catch what the shadow map cannot.** A 3 cm step on a floor under a sun
    /// 15° above the horizon: the step's shadow, ~11 cm long, is shorter than the shadow map's
    /// depth bias, so the cascades lose it. With `contact_shadow_length` the screen-space march
    /// finds the step and darkens the strip behind it; with 0 (the default) the frame is exactly
    /// what it was before contact shadows existed.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn contact_shadows_find_a_step_the_shadow_map_misses() {
        use crate::renderer::components::{DirectionalLight, LightRole};
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        async fn step_frame(contact: f32) -> Vec<u8> {
            let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
            renderer.taa = None;
            renderer.ssgi = None;
            renderer.ssao = None;
            let mut world = World::new();
            let mut assets = AssetManager::new();
            let tex = assets.create_white_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            );
            for (pos, scale) in [
                (Vec3::new(0.0, -0.2, 0.0), Vec3::new(20.0, 0.2, 20.0)),
                (Vec3::new(0.0, 0.015, 0.0), Vec3::new(0.6, 0.015, 0.6)),
            ] {
                let e = world.spawn();
                world.add_component(e, Transform::new(pos).with_scale(scale));
                world.add_component(e, GlobalTransform::default());
                world.add_component(e, AssetManager::create_cube(&renderer.device));
                world.add_component(e, Material::new(tex.clone()).with_pbr(Vec4::new(0.8, 0.8, 0.8, 1.0), 0.9, 0.0));
                world.add_component(e, MeshRenderer::new());
            }
            let elevation = 15f32.to_radians();
            let sun_dir = Vec3::new(elevation.cos(), -elevation.sin(), 0.0);
            let sun = world.spawn();
            world.add_component(
                sun,
                Transform::new(Vec3::ZERO).with_rotation(crate::math::Quat::from_rotation_arc(Vec3::NEG_Z, sun_dir)),
            );
            world.add_component(sun, GlobalTransform::default());
            world.add_component(
                sun,
                DirectionalLight::new(Vec3::ONE, 3.0, LightRole::Sun).with_contact_shadows(contact),
            );
            world.spawn_bundle(CameraBundle {
                position: Vec3::new(0.6, 1.2, 0.0),
                yaw: 0.0,
                pitch: -std::f32::consts::FRAC_PI_2 + 0.01,
                primary: true,
                ..Default::default()
            });
            render_world(&mut renderer, &mut world).await
        }
        // Pixels darker than 60 % of the frame's brightest — the shadowed ones.
        fn dark(frame: &[u8]) -> usize {
            let luma: Vec<f32> = frame.as_chunks::<4>().0.iter().map(|p| (p[0] as f32 + p[1] as f32 + p[2] as f32) / 3.0).collect();
            let hi = luma.iter().cloned().fold(0.0f32, f32::max);
            luma.iter().filter(|&&v| v < hi * 0.6).count()
        }
        pollster::block_on(async {
            let (off, on) = (step_frame(0.0).await, step_frame(0.3).await);
            // Measured: 363 dark pixels without (the step's own unlit side), 697 with (the strip
            // behind it as well).
            assert!(dark(&on) > dark(&off) + 50, "contact shadows darkened {} more pixels", dark(&on) as i64 - dark(&off) as i64);
            // And only there: open floor, the top and bottom 40 rows (the step crosses the middle),
            // must not change at all. A march that went away from the sun dives into the floor and
            // shadows the floor itself everywhere — it passes the count above and fails this.
            let rows = |frame: &[u8]| -> Vec<u8> {
                (0..40usize).chain(88..128).flat_map(|y| frame[y * 512..(y + 1) * 512].to_vec()).collect()
            };
            let (moved, total) = (
                rows(&off).iter().zip(rows(&on)).filter(|(a, b)| a.abs_diff(*b) > 8).count(),
                80 * 512,
            );
            assert!(moved == 0, "contact shadows changed {moved}/{total} bytes of open floor — false self-shadowing");
        });
    }

    /// **A sphere light's highlight is as big as the sphere.** A glossy metal floor reflects a point
    /// light hanging a metre above it. With `source_radius` 0 (the default) the highlight is the
    /// old soft spot; with 0.3 m it is the sphere's reflection, a disc — its saturated core must
    /// cover clearly more of the frame, while the frame's mean brightness stays put: the
    /// representative-point method renormalises the wider lobe, so a bigger source spreads the same
    /// energy rather than adding any. (The peak is no measure here: both clip at 255.)
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn a_sphere_light_spreads_its_highlight_without_adding_energy() {
        use crate::renderer::components::PointLight;
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        async fn highlight(source_radius: f32) -> (usize, f32) {
            let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
            renderer.taa = None;
            renderer.ssgi = None;
            renderer.ssr = None;
            let mut world = World::new();
            let mut assets = AssetManager::new();
            let tex = assets.create_white_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            );
            let floor = world.spawn();
            world.add_component(floor, Transform::new(Vec3::new(0.0, -0.2, 0.0)).with_scale(Vec3::new(20.0, 0.2, 20.0)));
            world.add_component(floor, GlobalTransform::default());
            world.add_component(floor, AssetManager::create_cube(&renderer.device));
            world.add_component(floor, Material::new(tex).with_pbr(Vec4::new(0.9, 0.9, 0.9, 1.0), 0.15, 1.0));
            world.add_component(floor, MeshRenderer::new());
            let lamp = world.spawn();
            world.add_component(lamp, Transform::new(Vec3::new(0.0, 1.0, 0.0)));
            world.add_component(lamp, GlobalTransform::default());
            world.add_component(lamp, PointLight::new(Vec3::ONE, 8.0, 20.0).with_source_radius(source_radius));
            world.spawn_bundle(CameraBundle {
                position: Vec3::new(0.0, 1.5, 3.0),
                yaw: -std::f32::consts::FRAC_PI_2,
                pitch: -0.55,
                primary: true,
                ..Default::default()
            });
            let frame = render_world(&mut renderer, &mut world).await;
            let luma: Vec<f32> = frame.as_chunks::<4>().0.iter().map(|p| (p[0] as f32 + p[1] as f32 + p[2] as f32) / 3.0).collect();
            let mean = luma.iter().sum::<f32>() / luma.len() as f32;
            (luma.iter().filter(|&&v| v >= 250.0).count(), mean)
        }
        pollster::block_on(async {
            let (point_area, point_mean) = highlight(0.0).await;
            let (sphere_area, sphere_mean) = highlight(0.3).await;
            // Measured: 186 px / mean 88.20 as a point, 464 px / 87.79 as a 0.3 m sphere. Without the
            // (α/α')² renormalisation the sphere reads 829 px / 98.25 — energy added; with the
            // radius never uploaded, the point's numbers exactly.
            assert!(
                sphere_area > point_area * 2,
                "the saturated highlight covers {sphere_area} px with a 0.3 m source, {point_area} as a point",
            );
            assert!(
                (sphere_mean - point_mean).abs() < point_mean * 0.05,
                "the frame's mean went from {point_mean:.2} to {sphere_mean:.2} — energy was added, not spread",
            );
        });
    }

    /// **The G-buffer's velocity is object motion as well as camera motion.** Two cubes under a
    /// still camera; between two frames one slides 0.3 m to the right and the other stays. On the
    /// second frame the still cube's velocity must be exactly zero and the moving one's must point
    /// right by about the distance it moved on screen. Before `crate::motion` there was no per-object
    /// history at all — only the camera's own reprojection — so a moving object under a still camera
    /// had no velocity anywhere.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn the_velocity_target_records_object_motion_and_nothing_else() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        fn half(bits: u16) -> f32 {
            let sign = if bits >> 15 == 1 { -1.0 } else { 1.0 };
            let exp = ((bits >> 10) & 0x1f) as i32;
            let frac = (bits & 0x3ff) as f32;
            sign * match exp {
                0 => frac * 2f32.powi(-24),
                31 => f32::INFINITY,
                _ => (1.0 + frac / 1024.0) * 2f32.powi(exp - 15),
            }
        }
        pollster::block_on(async {
            let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
            renderer.taa = None;
            renderer.ssgi = None;
            let mut world = World::new();
            let mut assets = AssetManager::new();
            let tex = assets.create_white_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            );
            let mut cube = |x: f32| {
                let e = world.spawn();
                world.add_component(e, Transform::new(Vec3::new(x, 0.0, 0.0)).with_scale(Vec3::splat(0.5)));
                world.add_component(e, GlobalTransform::default());
                world.add_component(e, AssetManager::create_cube(&renderer.device));
                world.add_component(e, Material::new(tex.clone()).with_pbr(Vec4::new(0.8, 0.8, 0.8, 1.0), 0.6, 0.0));
                world.add_component(e, MeshRenderer::new());
                e
            };
            let still = cube(-1.2);
            let moving = cube(1.0);
            world.spawn_bundle(DirectionalLightBundle { intensity: 3.0, ..Default::default() });
            world.spawn_bundle(CameraBundle {
                position: Vec3::new(0.0, 0.0, 6.0),
                yaw: -std::f32::consts::FRAC_PI_2,
                primary: true,
                ..Default::default()
            });
            render_world(&mut renderer, &mut world).await;
            // The first frame has no last frame: every velocity is zero, not a streak from
            // wherever an unseen entity is assumed to have been.
            {
                let tex = &renderer.deferred.as_ref().expect("the deferred path").velocity_tex;
                let first = read_target(&renderer, tex, 128).await;
                let moved = first.as_chunks::<2>().0.iter().filter(|h| half(u16::from_le_bytes(**h)) != 0.0).count();
                assert_eq!(moved, 0, "the first frame has {moved} non-zero velocity components");
            }
            if let Some(mut t) = world.borrow_mut::<Transform>().get_mut(moving.id()) {
                t.position.x += 0.3;
            }
            render_world(&mut renderer, &mut world).await;
            let tex = &renderer.deferred.as_ref().expect("the deferred path").velocity_tex;
            let bytes = read_target(&renderer, tex, 128).await;
            let velocity = |x: usize, y: usize| {
                let i = (y * 128 + x) * 4;
                (half(u16::from_le_bytes([bytes[i], bytes[i + 1]])), half(u16::from_le_bytes([bytes[i + 2], bytes[i + 3]])))
            };
            // Measured placement: the 1 m moving cube spans pixels 78–98 of the 128 (20 px a metre
            // at its depth), the still one (x = −1.2) around pixels 32–51.
            let mut still_max = 0.0f32;
            for y in 58..70 {
                for x in 36..48 {
                    let (vx, vy) = velocity(x, y);
                    still_max = still_max.max(vx.abs()).max(vy.abs());
                }
            }
            let (mut moving_sum, mut n) = (0.0f32, 0.0f32);
            for y in 58..70 {
                for x in 82..96 {
                    moving_sum += velocity(x, y).0;
                    n += 1.0;
                }
            }
            let moving_mean = moving_sum / n;
            // 0.3 m at 20 px a metre is 6 px: 6 / 128 = 0.047 of the screen. Measured 0.047.
            let _ = still;
            assert_eq!(still_max, 0.0, "a cube that did not move has velocity up to {still_max}");
            assert!(
                (0.040..0.055).contains(&moving_mean),
                "the moving cube's velocity is {moving_mean} of the screen to the right; 0.047 expected",
            );
        });
    }

    /// **Object motion blur.** Under a still camera, one cube slides 0.3 m between two frames and
    /// one stays. With the shutter open the moving cube's edges smear — its strip of the frame loses
    /// edge energy against the same frame with the shutter shut — and the still cube's strip is
    /// identical to the shutter shut, to the byte. Camera-only blur (before the G-buffer had a
    /// velocity) left the moving cube as sharp as the still one.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn object_motion_blur_smears_what_moved_and_nothing_else() {
        use crate::renderer::components::PostProcess;
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        async fn second_frame(shutter: f32) -> Vec<u8> {
            let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
            renderer.taa = None;
            renderer.ssgi = None;
            let mut world = World::new();
            let mut assets = AssetManager::new();
            let tex = assets.create_white_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            );
            let mut cube = |x: f32| {
                let e = world.spawn();
                world.add_component(e, Transform::new(Vec3::new(x, 0.0, 0.0)).with_scale(Vec3::splat(0.5)));
                world.add_component(e, GlobalTransform::default());
                world.add_component(e, AssetManager::create_cube(&renderer.device));
                world.add_component(e, Material::new(tex.clone()).with_pbr(Vec4::new(0.9, 0.3, 0.2, 1.0), 0.6, 0.0));
                world.add_component(e, MeshRenderer::new());
                e
            };
            let _still = cube(-1.2);
            let moving = cube(1.0);
            world.spawn_bundle(DirectionalLightBundle { intensity: 3.0, ..Default::default() });
            let cam = world.spawn_bundle(CameraBundle {
                position: Vec3::new(0.0, 0.0, 6.0),
                yaw: -std::f32::consts::FRAC_PI_2,
                primary: true,
                ..Default::default()
            });
            let mut grade = PostProcess::default();
            grade.film_grain = 0.0;
            grade.motion_blur = shutter;
            world.add_component(cam, grade);
            render_world(&mut renderer, &mut world).await;
            if let Some(mut t) = world.borrow_mut::<Transform>().get_mut(moving.id()) {
                t.position.x += 0.3;
            }
            render_world(&mut renderer, &mut world).await
        }
        // Squared horizontal gradient over columns x0..x1 — how steep the edges there are.
        fn edge(frame: &[u8], x0: usize, x1: usize) -> u64 {
            let mut sum = 0u64;
            for y in 0..128usize {
                for x in x0.max(1)..x1 {
                    let (a, b) = ((y * 128 + x - 1) * 4, (y * 128 + x) * 4);
                    sum += (0..3).map(|c| (frame[a + c].abs_diff(frame[b + c]) as u64).pow(2)).sum::<u64>();
                }
            }
            sum
        }
        pollster::block_on(async {
            let (shut, open) = (second_frame(0.0).await, second_frame(1.0).await);
            // The still cube spans about pixels 32–51, the moving one 78–98 (see the velocity test).
            let still = |f: &[u8]| -> Vec<u8> { (0..128usize).flat_map(|y| f[(y * 128 + 26) * 4..(y * 128 + 58) * 4].to_vec()).collect() };
            assert_eq!(still(&shut), still(&open), "the still cube's strip changed with the shutter open");
            let (sharp, smeared) = (edge(&shut, 70, 110), edge(&open, 70, 110));
            // Measured: 1 122 066 shut, 313 386 open. Without the neighbour-max search the smear
            // stays inside the silhouette (700 320) and the next assertion fails.
            //
            // The trail leaves the silhouette: background just right of the moving cube (columns
            // 99–104, its rows) changes with the shutter open. Velocity exists only where the cube
            // is now, so only the neighbour-max search can carry it out there.
            let trail = (58..70usize)
                .flat_map(|y| (99..105usize).map(move |x| (y * 128 + x) * 4))
                .filter(|&i| (0..3).any(|c| shut[i + c].abs_diff(open[i + c]) > 4))
                .count();
            assert!(trail >= 20, "only {trail} of 72 background pixels beside the moving cube show its trail");
            assert!(
                (smeared as f64) < sharp as f64 * 0.8,
                "the moving cube keeps edge energy {smeared} with the shutter open, {sharp} shut — it is not blurred",
            );
        });
    }

    /// **A tube light's highlight is a streak along the tube.** The glossy floor of the sphere-light
    /// test, under a 1 m tube (`source_length`, along the entity's local X = the screen's horizontal)
    /// against a sphere of the same 5 cm thickness. The saturated highlight's width-to-height ratio
    /// must grow clearly, and the frame's mean must not run away: the lobe is renormalised along the
    /// length, so a longer tube spreads its light.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn a_tube_light_stretches_its_highlight_along_the_tube() {
        use crate::renderer::components::PointLight;
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        // (width / height of the ≥ 250 highlight, frame mean)
        async fn highlight(length: f32) -> (f32, f32) {
            let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
            renderer.taa = None;
            renderer.ssgi = None;
            renderer.ssr = None;
            let mut world = World::new();
            let mut assets = AssetManager::new();
            let tex = assets.create_white_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            );
            let floor = world.spawn();
            world.add_component(floor, Transform::new(Vec3::new(0.0, -0.2, 0.0)).with_scale(Vec3::new(20.0, 0.2, 20.0)));
            world.add_component(floor, GlobalTransform::default());
            world.add_component(floor, AssetManager::create_cube(&renderer.device));
            world.add_component(floor, Material::new(tex).with_pbr(Vec4::new(0.9, 0.9, 0.9, 1.0), 0.15, 1.0));
            world.add_component(floor, MeshRenderer::new());
            let lamp = world.spawn();
            world.add_component(lamp, Transform::new(Vec3::new(0.0, 1.0, 0.0)));
            world.add_component(lamp, GlobalTransform::default());
            world.add_component(
                lamp,
                PointLight::new(Vec3::ONE, 8.0, 20.0).with_source_radius(0.05).with_source_length(length),
            );
            world.spawn_bundle(CameraBundle {
                position: Vec3::new(0.0, 1.5, 3.0),
                yaw: -std::f32::consts::FRAC_PI_2,
                pitch: -0.55,
                primary: true,
                ..Default::default()
            });
            let frame = render_world(&mut renderer, &mut world).await;
            let luma: Vec<f32> = frame.as_chunks::<4>().0.iter().map(|p| (p[0] as f32 + p[1] as f32 + p[2] as f32) / 3.0).collect();
            let mut bbox = [usize::MAX, usize::MAX, 0usize, 0usize];
            for (i, &v) in luma.iter().enumerate() {
                if v >= 250.0 {
                    let (x, y) = (i % 128, i / 128);
                    bbox = [bbox[0].min(x), bbox[1].min(y), bbox[2].max(x + 1), bbox[3].max(y + 1)];
                }
            }
            let aspect = (bbox[2].saturating_sub(bbox[0])) as f32 / (bbox[3].saturating_sub(bbox[1])).max(1) as f32;
            (aspect, luma.iter().sum::<f32>() / luma.len() as f32)
        }
        pollster::block_on(async {
            let (sphere_aspect, sphere_mean) = highlight(0.0).await;
            let (tube_aspect, tube_mean) = highlight(1.0).await;
            // Measured: aspect 0.72 → 3.36, mean 85.82 → 87.74 (+2 %). Without the line
            // renormalisation the mean reads 95.38 (+11 %); with the axis never uploaded the
            // tube collapses to its centre and the aspect to 0.30.
            assert!(tube_aspect > sphere_aspect * 1.5, "the tube's highlight is {tube_aspect:.2} wide per height, the sphere's {sphere_aspect:.2}");
            assert!(tube_mean < sphere_mean * 1.07, "the frame's mean went {sphere_mean:.2} → {tube_mean:.2} — energy added along the tube");
        });
    }

    /// A rectangle light seen from straight below, over a matte floor. Three things, each a
    /// different way to get it wrong:
    ///
    /// - a small panel (5 cm) facing down lights the floor under it like a point light of the same
    ///   intensity — the radiance is `intensity / area`, so the polygon integral must come out to
    ///   the point light's inverse square;
    /// - the same panel turned to face up lights **nothing**: it is one-sided, and its
    ///   orientation must actually reach the shader;
    /// - a 1.5 m panel spreads its light: the floor's lit area grows while the spot under it
    ///   does not brighten past the small panel's.
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn a_rect_light_lights_one_side_like_a_point_light_seen_face_on() {
        use crate::renderer::components::{PointLight, RectLight};
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        enum Lamp {
            None,
            Point,
            Rect { size: f32, facing_down: bool },
        }
        // (mean luma of the 8×8 centre, pixels brighter than 150 — the unlit floor reads 66,
        // mean of the left half, mean of the right half)
        async fn floor(lamp: Lamp) -> (f32, usize, f32, f32) {
            let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
            renderer.taa = None;
            renderer.ssgi = None;
            renderer.ssr = None;
            let mut world = World::new();
            let mut assets = AssetManager::new();
            let tex = assets.create_white_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            );
            let floor = world.spawn();
            world.add_component(floor, Transform::new(Vec3::new(0.0, -0.2, 0.0)).with_scale(Vec3::new(20.0, 0.2, 20.0)));
            world.add_component(floor, GlobalTransform::default());
            world.add_component(floor, AssetManager::create_cube(&renderer.device));
            world.add_component(floor, Material::new(tex).with_pbr(Vec4::new(0.8, 0.8, 0.8, 1.0), 1.0, 0.0));
            world.add_component(floor, MeshRenderer::new());
            let half_pi = std::f32::consts::FRAC_PI_2;
            match lamp {
                Lamp::None => {}
                Lamp::Point => {
                    let e = world.spawn();
                    world.add_component(e, Transform::new(Vec3::new(0.0, 1.0, 0.0)));
                    world.add_component(e, GlobalTransform::default());
                    world.add_component(e, PointLight::new(Vec3::ONE, 3.0, 20.0));
                }
                Lamp::Rect { size, facing_down } => {
                    // The panel emits along its local −Z: −90° about X turns that to −Y, and
                    // −90° about Y to +X — standing on edge, lighting one side of the room.
                    let rotation = if facing_down {
                        gizmo_math::Quat::from_rotation_x(-half_pi)
                    } else {
                        gizmo_math::Quat::from_rotation_y(-half_pi)
                    };
                    let e = world.spawn();
                    world.add_component(e, Transform::new(Vec3::new(0.0, 1.0, 0.0)).with_rotation(rotation));
                    world.add_component(e, GlobalTransform::default());
                    world.add_component(e, RectLight::new(Vec3::ONE, 3.0, 20.0, size, size));
                }
            }
            world.spawn_bundle(CameraBundle {
                position: Vec3::new(0.0, 3.0, 0.0),
                yaw: -half_pi,
                pitch: -1.5,
                primary: true,
                ..Default::default()
            });
            let frame = render_world(&mut renderer, &mut world).await;
            let luma: Vec<f32> = frame.as_chunks::<4>().0.iter().map(|p| (p[0] as f32 + p[1] as f32 + p[2] as f32) / 3.0).collect();
            let mut centre = 0.0;
            for y in 60..68 {
                for x in 60..68 {
                    centre += luma[y * 128 + x];
                }
            }
            let half = |left: bool| {
                let xs = if left { 0..64 } else { 64..128 };
                let mut sum = 0.0;
                for y in 0..128 {
                    for x in xs.clone() {
                        sum += luma[y * 128 + x];
                    }
                }
                sum / (64.0 * 128.0)
            };
            (centre / 64.0, luma.iter().filter(|&&v| v > 150.0).count(), half(true), half(false))
        }
        pollster::block_on(async {
            let (dark, _, dark_l, dark_r) = floor(Lamp::None).await;
            let (point, _, _, _) = floor(Lamp::Point).await;
            let (small, small_lit, _, _) = floor(Lamp::Rect { size: 0.05, facing_down: true }).await;
            let (_, _, side_l, side_r) = floor(Lamp::Rect { size: 0.5, facing_down: false }).await;
            let (big, big_lit, _, _) = floor(Lamp::Rect { size: 1.5, facing_down: true }).await;
            // Measured: unlit 66.24 (halves 61.39 / 61.39), point 228.45, 5 cm panel 226.71 with
            // 5258 pixels over 150, on edge 61.41 behind / 115.33 in front, 1.5 m panel 202.64 with
            // 6018. Caught: orientation never uploaded (148.34 under the panel — it faces −Z),
            // diffuse taken from the centre (6018 → 5244: no spread), the area halved (247.64),
            // the volumetric pass scattering behind the panel (67.33 behind).
            assert!(point > dark + 40.0, "the point light must light the floor at all: {dark:.2} → {point:.2}");
            assert!((small - point).abs() < 0.05 * (point - dark), "a 5 cm panel face on is a point light of the same intensity: {small:.2} vs {point:.2}");
            // Standing on edge, facing +X: one half of the screen is behind it — floor and air.
            let (behind, behind_dark, front, front_dark) =
                if side_l < side_r { (side_l, dark_l, side_r, dark_r) } else { (side_r, dark_r, side_l, dark_l) };
            assert!((behind - behind_dark).abs() < 0.5, "behind the panel nothing is lit, floor or air: {behind:.2}, unlit {behind_dark:.2}");
            assert!(front > front_dark + 10.0, "in front of it the floor is: {front_dark:.2} → {front:.2}");
            assert!(big_lit > small_lit * 11 / 10, "a 1.5 m panel spreads its light: {small_lit} → {big_lit} lit pixels");
            assert!(big < small, "…without brightening the spot under it: {small:.2} → {big:.2}");
        });
    }

    /// A rectangle light's reflection in a glossy floor is the panel's shape, without the energy
    /// running away. The camera and floor are the tube test's; a 1.2 m × 5 cm panel (long along the
    /// screen's horizontal) against a 5 cm square. The saturated highlight's width-to-height ratio
    /// must grow clearly, and the frame's mean must hold — the lobe is renormalised across each
    /// extent.
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn a_rect_light_reflects_as_a_panel() {
        use crate::renderer::components::RectLight;
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        // (width / height of the ≥ 250 highlight, frame mean)
        async fn highlight(width: f32) -> (f32, f32) {
            let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
            renderer.taa = None;
            renderer.ssgi = None;
            renderer.ssr = None;
            let mut world = World::new();
            let mut assets = AssetManager::new();
            let tex = assets.create_white_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            );
            let floor = world.spawn();
            world.add_component(floor, Transform::new(Vec3::new(0.0, -0.2, 0.0)).with_scale(Vec3::new(20.0, 0.2, 20.0)));
            world.add_component(floor, GlobalTransform::default());
            world.add_component(floor, AssetManager::create_cube(&renderer.device));
            world.add_component(floor, Material::new(tex).with_pbr(Vec4::new(0.9, 0.9, 0.9, 1.0), 0.15, 1.0));
            world.add_component(floor, MeshRenderer::new());
            let lamp = world.spawn();
            world.add_component(
                lamp,
                Transform::new(Vec3::new(0.0, 1.0, 0.0))
                    .with_rotation(gizmo_math::Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
            );
            world.add_component(lamp, GlobalTransform::default());
            world.add_component(lamp, RectLight::new(Vec3::ONE, 8.0, 20.0, width, 0.05));
            world.spawn_bundle(CameraBundle {
                position: Vec3::new(0.0, 1.5, 3.0),
                yaw: -std::f32::consts::FRAC_PI_2,
                pitch: -0.55,
                primary: true,
                ..Default::default()
            });
            let frame = render_world(&mut renderer, &mut world).await;
            let luma: Vec<f32> = frame.as_chunks::<4>().0.iter().map(|p| (p[0] as f32 + p[1] as f32 + p[2] as f32) / 3.0).collect();
            let mut bbox = [usize::MAX, usize::MAX, 0usize, 0usize];
            for (i, &v) in luma.iter().enumerate() {
                if v >= 250.0 {
                    let (x, y) = (i % 128, i / 128);
                    bbox = [bbox[0].min(x), bbox[1].min(y), bbox[2].max(x + 1), bbox[3].max(y + 1)];
                }
            }
            let aspect = (bbox[2].saturating_sub(bbox[0])) as f32 / (bbox[3].saturating_sub(bbox[1])).max(1) as f32;
            (aspect, luma.iter().sum::<f32>() / luma.len() as f32)
        }
        pollster::block_on(async {
            let (square_aspect, square_mean) = highlight(0.05).await;
            let (panel_aspect, panel_mean) = highlight(1.2).await;
            // Measured: aspect 0.80 → 4.44, mean 80.89 → 83.65 (+3 %). Without the
            // renormalisation the mean reads 94.47 (+14 % over its own square).
            assert!(panel_aspect > square_aspect * 1.5, "the panel's highlight is {panel_aspect:.2} wide per height, the square's {square_aspect:.2}");
            assert!(panel_mean < square_mean * 1.07, "the frame's mean went {square_mean:.2} → {panel_mean:.2} — energy added across the panel");
        });
    }

    /// A spot light's cookie is projected through its cone, the right way round. A spot points
    /// straight down at a matte floor, seen from above (world +X is the screen's right — measured in
    /// the rectangle-light test). Its cookie is black on the left, white on the right, so the pool
    /// of light must be cut in half: the right half lit, the left half as dark as no light at all.
    /// Without a cookie both halves are lit alike, and a cookie named but never found (no such
    /// file) must behave as no cookie rather than as layer 0's image or as black.
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn a_spot_light_projects_its_cookie_the_right_way_round() {
        use crate::renderer::components::SpotLight;
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        // (mean luma of the left half, of the right half); `lit = false` leaves the lamp out.
        async fn halves(cookie: Option<&str>, lit: bool) -> (f32, f32) {
            let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
            renderer.taa = None;
            renderer.ssgi = None;
            renderer.ssr = None;
            renderer.volumetric = None;
            // 2×1: black, white.
            renderer
                .insert_light_cookie("half", &[0, 0, 0, 255, 255, 255, 255, 255], 2, 1)
                .expect("one cookie fits");
            let mut world = World::new();
            let mut assets = AssetManager::new();
            let tex = assets.create_white_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            );
            let floor = world.spawn();
            world.add_component(floor, Transform::new(Vec3::new(0.0, -0.2, 0.0)).with_scale(Vec3::new(20.0, 0.2, 20.0)));
            world.add_component(floor, GlobalTransform::default());
            world.add_component(floor, AssetManager::create_cube(&renderer.device));
            world.add_component(floor, Material::new(tex).with_pbr(Vec4::new(0.8, 0.8, 0.8, 1.0), 1.0, 0.0));
            world.add_component(floor, MeshRenderer::new());
            let half_pi = std::f32::consts::FRAC_PI_2;
            let lamp = world.spawn();
            // Spots aim along local −Z: −90° about X aims straight down.
            world.add_component(
                lamp,
                Transform::new(Vec3::new(0.0, 1.5, 0.0)).with_rotation(gizmo_math::Quat::from_rotation_x(-half_pi)),
            );
            world.add_component(lamp, GlobalTransform::default());
            let mut spot = SpotLight::new(Vec3::ONE, if lit { 6.0 } else { 0.0 }, 20.0, 0.5, 0.6);
            spot.cookie = cookie.map(str::to_string);
            world.add_component(lamp, spot);
            world.spawn_bundle(CameraBundle {
                position: Vec3::new(0.0, 3.0, 0.0),
                yaw: -half_pi,
                pitch: -1.5,
                primary: true,
                ..Default::default()
            });
            let frame = render_world(&mut renderer, &mut world).await;
            let luma: Vec<f32> = frame.as_chunks::<4>().0.iter().map(|p| (p[0] as f32 + p[1] as f32 + p[2] as f32) / 3.0).collect();
            let half = |left: bool| {
                let xs = if left { 0..64 } else { 64..128 };
                let mut sum = 0.0;
                for y in 0..128 {
                    for x in xs.clone() {
                        sum += luma[y * 128 + x];
                    }
                }
                sum / (64.0 * 128.0)
            };
            (half(true), half(false))
        }
        pollster::block_on(async {
            let (dark_l, dark_r) = halves(None, false).await;
            let (plain_l, plain_r) = halves(None, true).await;
            let (cut_l, cut_r) = halves(Some("half"), true).await;
            let (missing_l, missing_r) = halves(Some("no/such/cookie.png"), true).await;
            // Measured: unlit 61.39 / 61.39, plain 93.82 / 93.80, cut 65.56 / 88.48 (the 2-texel
            // image's bilinear ramp crosses the middle), missing = plain. Caught: u mirrored
            // (88.50 / 65.56), the layer off by one (an empty, black layer: 61.39 both), the cookie
            // sampled but not applied and never packed (both = plain).
            assert!(plain_l > dark_l + 10.0, "the spot must light the floor: {dark_l:.2} → {plain_l:.2}");
            assert!((plain_l - plain_r).abs() < 0.5, "without a cookie the pool is symmetric: {plain_l:.2} / {plain_r:.2}");
            assert!(
                cut_l - dark_l < 0.2 * (plain_l - dark_l),
                "the cookie's black half blocks the light: left {cut_l:.2}, unlit {dark_l:.2}, plain {plain_l:.2}"
            );
            assert!(
                cut_r - dark_r > 0.6 * (plain_r - dark_r),
                "its white half lets it through: right {cut_r:.2}, unlit {dark_r:.2}, plain {plain_r:.2}"
            );
            assert!(
                (missing_l - plain_l).abs() < 0.1 && (missing_r - plain_r).abs() < 0.1,
                "a cookie that never loaded is no cookie: {missing_l:.2}/{missing_r:.2} vs {plain_l:.2}/{plain_r:.2}"
            );
        });
    }

    /// Each `BlendMode` combines with what is behind it the way it says. A half-transparent grey
    /// panel (0.5, alpha 0.5) in front of a blue-grey wall, all unlit so the arithmetic is the
    /// blend's alone; the 8×8 centre of the frame is the panel over the wall. Tone mapping bends
    /// the numbers, so the test asks for the orderings the four equations imply rather than for
    /// values: additive brightens every channel past the wall, multiply darkens every one below
    /// it, premultiplied adds the 0.25 alpha would have removed. The same additive panel lit (PBR)
    /// must brighten too — the lit route has its own variants.
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn each_blend_mode_combines_with_what_is_behind_it() {
        use crate::renderer::components::BlendMode;
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        // Mean RGB of the centre with the panel blending by `mode` (`None`: no panel).
        async fn centre(mode: Option<BlendMode>, lit: bool) -> [f32; 3] {
            let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
            renderer.taa = None;
            renderer.ssgi = None;
            renderer.ssr = None;
            renderer.volumetric = None;
            let mut world = World::new();
            let mut assets = AssetManager::new();
            let tex = assets.create_white_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            );
            let wall = world.spawn();
            world.add_component(wall, Transform::new(Vec3::new(0.0, 0.0, -6.0)).with_scale(Vec3::new(20.0, 20.0, 0.1)));
            world.add_component(wall, GlobalTransform::default());
            world.add_component(wall, AssetManager::create_cube(&renderer.device));
            world.add_component(wall, Material::new(tex.clone()).with_unlit(Vec4::new(0.2, 0.35, 0.6, 1.0)));
            world.add_component(wall, MeshRenderer::new());
            if let Some(mode) = mode {
                let panel = world.spawn();
                world.add_component(panel, Transform::new(Vec3::new(0.0, 0.0, -3.0)).with_scale(Vec3::new(1.0, 1.0, 0.05)));
                world.add_component(panel, GlobalTransform::default());
                world.add_component(panel, AssetManager::create_cube(&renderer.device));
                let colour = Vec4::new(0.5, 0.5, 0.5, 0.5);
                let material = if lit {
                    Material::new(tex).with_pbr(colour, 0.9, 0.0).with_emissive(Vec3::splat(0.5))
                } else {
                    Material::new(tex).with_unlit(colour)
                };
                world.add_component(panel, material.with_blend_mode(mode));
                world.add_component(panel, MeshRenderer::new());
            }
            world.spawn_bundle(CameraBundle {
                position: Vec3::ZERO,
                yaw: -std::f32::consts::FRAC_PI_2,
                pitch: 0.0,
                primary: true,
                ..Default::default()
            });
            let frame = render_world(&mut renderer, &mut world).await;
            let mut sum = [0.0f32; 3];
            for y in 60..68 {
                for x in 60..68 {
                    let p = &frame[(y * 128 + x) * 4..];
                    for c in 0..3 {
                        sum[c] += p[c] as f32 / 64.0;
                    }
                }
            }
            sum
        }
        pollster::block_on(async {
            let wall = centre(None, false).await;
            let alpha = centre(Some(BlendMode::Alpha), false).await;
            let add = centre(Some(BlendMode::Additive), false).await;
            let mul = centre(Some(BlendMode::Multiply), false).await;
            let pre = centre(Some(BlendMode::Premultiplied), false).await;
            let lit_alpha = centre(Some(BlendMode::Alpha), true).await;
            let lit_add = centre(Some(BlendMode::Additive), true).await;
            // Measured (R, G, B): wall 148.8 186.4 214.1 · alpha 197.5 201.8 208.1 · additive
            // 236.0 240.1 244.5 · multiply 59.1 90.7 128.0 · premultiplied 238.7 239.6 241.0 ·
            // lit alpha 210.9 210.0 214.5 · lit additive 241.5 243.0 246.4 (re-measured after
            // the forward shader began adding the panel's 0.5 emissive; 142.5 … / 191.0 … before).
            for c in 0..3 {
                assert!(add[c] > wall[c] + 5.0, "additive brightens every channel: {add:?} over {wall:?}");
                assert!(mul[c] < wall[c] - 20.0, "multiply darkens every channel: {mul:?} over {wall:?}");
                assert!(pre[c] > alpha[c] + 5.0, "premultiplied keeps what alpha removes: {pre:?} vs {alpha:?}");
                assert!(lit_add[c] > wall[c] + 3.0, "the lit route blends additively too: {lit_add:?} over {wall:?}");
                assert!(lit_add[c] > lit_alpha[c] + 10.0, "lit additive is not lit alpha: {lit_add:?} vs {lit_alpha:?}");
            }
            // Alpha is a mix: grey 0.5 lifts the wall's red (0.2) and lowers its blue (0.6).
            assert!(alpha[0] > wall[0] && alpha[2] < wall[2], "alpha mixes: {alpha:?} over {wall:?}");
        });
    }

    /// `Material::with_emissive` makes a surface glow, unlit and in HDR, on both draw paths. A
    /// black-albedo cube in a scene with no light at all, so every bit of its brightness is
    /// emission: opaque (deferred, RT5) and transparent (forward). Before RT5 the per-instance
    /// emissive was read by `baked_lit.wgsl` alone — the deferred cube stayed black — and a glTF
    /// emissive was added to the 8-bit albedo, lit and clamped at 1.
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn emissive_glows_on_both_paths_and_is_not_clamped() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        // Mean red of the centre.
        async fn centre_red(emissive: f32, transparent: bool) -> f32 {
            let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
            renderer.taa = None;
            renderer.ssgi = None;
            renderer.ssr = None;
            renderer.volumetric = None;
            renderer.bloom_intensity = 0.0;
            let mut world = World::new();
            let mut assets = AssetManager::new();
            let tex = assets.create_white_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            );
            let cube = world.spawn();
            world.add_component(cube, Transform::new(Vec3::new(0.0, 0.0, -4.0)));
            world.add_component(cube, GlobalTransform::default());
            world.add_component(cube, AssetManager::create_cube(&renderer.device));
            let alpha = if transparent { 0.999 } else { 1.0 };
            let material = Material::new(tex)
                .with_pbr(Vec4::new(0.0, 0.0, 0.0, alpha), 0.9, 0.0)
                .with_emissive(Vec3::new(emissive, emissive * 0.25, 0.0));
            world.add_component(cube, if transparent { material.with_transparent(true) } else { material });
            world.add_component(cube, MeshRenderer::new());
            world.spawn_bundle(CameraBundle {
                position: Vec3::ZERO,
                yaw: -std::f32::consts::FRAC_PI_2,
                pitch: 0.0,
                primary: true,
                ..Default::default()
            });
            let frame = render_world(&mut renderer, &mut world).await;
            let mut red = 0.0;
            for y in 60..68 {
                for x in 60..68 {
                    red += frame[(y * 128 + x) * 4] as f32 / 64.0;
                }
            }
            red
        }
        pollster::block_on(async {
            let mut found = Vec::new();
            for transparent in [false, true] {
                let none = centre_red(0.0, transparent).await;
                let one = centre_red(1.0, transparent).await;
                let four = centre_red(4.0, transparent).await;
                found.push((none, one, four));
            }
            // Measured (centre red, emissive 0 / 1 / 4): opaque 6.0 / 231.3 / 251.8, transparent
            // 0.0 / 231.6 / 251.9 — the two paths agree. Caught: the lighting pass not adding
            // RT5 (opaque stays at 6), the forward shader not adding it (transparent at 0), and
            // the emission clamped to 1 (4 reads as 1).
            for (path, (none, one, four)) in ["opaque", "transparent"].iter().zip(&found) {
                assert!(*none < 20.0, "{path}: no light, no emission, dark: {none:.1}");
                assert!(*one > none + 150.0, "{path}: emissive 1 glows: {none:.1} → {one:.1}");
                assert!(*four > one + 10.0, "{path}: emissive 4 is brighter still — HDR, not clamped: {one:.1} → {four:.1}");
            }
            assert!((found[0].1 - found[1].1).abs() < 5.0, "both paths emit alike: {found:?}");
        });
    }

    /// The deferred PBR path reads vertex colour, as the forward shader always has. A triangle with
    /// a red, a green and a blue corner, white material, lit by a sun; each corner region's
    /// dominant channel must be its own. Before, the G-buffer ignored the attribute — `vertex_colors`
    /// measured a channel separation of 0.9 under `Pbr` against 60.6 under `BakedLit`.
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn the_deferred_path_reads_vertex_colour() {
        use crate::renderer::gpu_types::Vertex;
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        pollster::block_on(async {
            let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
            renderer.taa = None;
            renderer.ssgi = None;
            renderer.ssr = None;
            renderer.volumetric = None;
            let mut world = World::new();
            let mut assets = AssetManager::new();
            let tex = assets.create_white_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            );
            let corners = [
                ([-1.0f32, -0.9, 0.0], [1.0f32, 0.0, 0.0]),
                ([1.0, -0.9, 0.0], [0.0, 1.0, 0.0]),
                ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
            ];
            let vertices: Vec<Vertex> = corners
                .iter()
                .map(|(p, c)| Vertex {
                    position: *p,
                    normal: [0.0, 0.0, 1.0],
                    color: [c[0], c[1], c[2], 1.0],
                    tex_coords: [0.5, 0.5],
                    ..Default::default()
                })
                .collect();
            let tri = world.spawn();
            world.add_component(tri, Transform::new(Vec3::new(0.0, 0.0, -2.5)));
            world.add_component(tri, GlobalTransform::default());
            world.add_component(tri, crate::renderer::components::Mesh::from_vertices(&renderer.device, &vertices, "vertex_colour_test"));
            world.add_component(tri, Material::new(tex).with_pbr(Vec4::ONE, 0.6, 0.0));
            world.add_component(tri, MeshRenderer::new());
            world.spawn_bundle(crate::bundles::DirectionalLightBundle {
                rotation: gizmo_math::Quat::from_rotation_x(-0.2),
                intensity: 3.0,
                ..Default::default()
            });
            world.spawn_bundle(CameraBundle {
                position: Vec3::ZERO,
                yaw: -std::f32::consts::FRAC_PI_2,
                pitch: 0.0,
                primary: true,
                ..Default::default()
            });
            let frame = render_world(&mut renderer, &mut world).await;
            // Mean RGB of a 6×6 patch at pixel (cx, cy).
            let patch = |cx: usize, cy: usize| {
                let mut sum = [0.0f32; 3];
                for y in cy - 3..cy + 3 {
                    for x in cx - 3..cx + 3 {
                        for c in 0..3 {
                            sum[c] += frame[(y * 128 + x) * 4 + c] as f32 / 36.0;
                        }
                    }
                }
                sum
            };
            // Near each corner, a little inside the triangle (camera 2.5 m away, 90° vertical FOV
            // ⇒ 1 m ≈ 25.6 px).
            let red = patch(64 - 20, 64 + 18);
            let green = patch(64 + 20, 64 + 18);
            let blue = patch(64, 64 - 18);
            // Measured (RGB): red corner 216.7 146.7 179.5, green 146.3 217.1 179.6, blue 159.2
            // 159.6 223.5. Ignoring the vertex colour again gives three identical greys.
            for (name, rgb, own) in [("red", red, 0), ("green", green, 1), ("blue", blue, 2)] {
                for other in (0..3).filter(|c| *c != own) {
                    assert!(rgb[own] > rgb[other] + 25.0, "the {name} corner is {name}: {rgb:?}");
                }
            }
        });
    }

    /// The game path publishes `RenderStats`, counted as the editor counts them. Three cubes
    /// sharing a mesh and a material are one batch: 1 draw call, 3 instances, 3 × 12 triangles. A
    /// fourth that only casts a shadow (`ShadowCasting::Only`) is not drawn and must not count.
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn the_game_path_publishes_render_stats() {
        use crate::renderer::components::{RenderStats, ShadowCasting};
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        pollster::block_on(async {
            let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
            let mut world = World::new();
            let mut assets = AssetManager::new();
            let tex = assets.create_white_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            );
            let mesh = AssetManager::create_cube(&renderer.device);
            let material = Material::new(tex).with_pbr(Vec4::new(0.7, 0.7, 0.7, 1.0), 0.5, 0.0);
            for (i, shadows) in [ShadowCasting::On, ShadowCasting::On, ShadowCasting::On, ShadowCasting::Only].into_iter().enumerate() {
                let e = world.spawn();
                world.add_component(e, Transform::new(Vec3::new(i as f32 * 1.5 - 2.25, 0.0, -6.0)).with_scale(Vec3::splat(0.5)));
                world.add_component(e, GlobalTransform::default());
                world.add_component(e, mesh.clone());
                world.add_component(e, material.clone());
                world.add_component(e, MeshRenderer::new().with_shadows(shadows));
            }
            world.spawn_bundle(CameraBundle {
                position: Vec3::ZERO,
                yaw: -std::f32::consts::FRAC_PI_2,
                pitch: 0.0,
                primary: true,
                ..Default::default()
            });
            let _ = render_world(&mut renderer, &mut world).await;
            let stats = *world.get_resource::<RenderStats>().expect("the game path publishes RenderStats");
            assert_eq!((stats.draw_calls, stats.instances, stats.triangles), (1, 3, 36), "{stats:?}");
        });
    }

    /// A `Skybox` material draws in the game path. `CAPABILITY_GAPS` §D10 recorded it drawing
    /// nothing ("cause unknown", `atmosphere`: 0 pixels changed) with a long elimination list; on
    /// 2026-11-02 it no longer reproduces — a sky cube around the camera repaints every pixel of
    /// this frame, and `atmosphere` differs in 10066 pixels at frame 90, all of them sky. Pinned
    /// here so it cannot quietly return.
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn a_skybox_draws_in_the_game_path() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        async fn frame(sky: bool) -> Vec<u8> {
            let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
            let mut world = World::new();
            let mut assets = AssetManager::new();
            let tex = assets.create_white_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout);
            if sky {
                let e = world.spawn();
                world.add_component(e, Transform::new(Vec3::ZERO).with_scale(Vec3::splat(50.0)));
                world.add_component(e, GlobalTransform::default());
                world.add_component(e, AssetManager::create_cube(&renderer.device));
                world.add_component(e, Material::new(tex).with_skybox());
                world.add_component(e, MeshRenderer::new());
            }
            world.spawn_bundle(crate::bundles::DirectionalLightBundle::default());
            world.spawn_bundle(CameraBundle { position: Vec3::ZERO, yaw: -std::f32::consts::FRAC_PI_2, pitch: 0.2, primary: true, ..Default::default() });
            render_world(&mut renderer, &mut world).await
        }
        pollster::block_on(async {
            let a = frame(false).await;
            let b = frame(true).await;
            // Measured: 49152 bytes — every RGB byte of the 128² frame; the centre went from
            // (59, 61, 64) to (243, 249, 252).
            let diff = a.iter().zip(&b).filter(|(x, y)| x != y).count();
            assert!(diff > 45_000, "the sky cube repaints the frame: {diff} of 49152 colour bytes changed");
        });
    }

    /// A `Grid` material draws as a grid in the game, as in the editor: lines, not a flat plane.
    /// Seen from above, the lower half of the frame must be full of sharp horizontal edges; the
    /// same plane routed any other way (deferred PBR, as it was, or the plain unlit pipeline)
    /// has almost none.
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn a_grid_material_draws_a_grid_in_the_game() {
        use crate::renderer::components::MaterialType;
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        async fn edges(grid: bool) -> usize {
            let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
            renderer.taa = None;
            let mut world = World::new();
            let mut assets = AssetManager::new();
            let tex = assets.create_white_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout);
            let mut material = Material::new(tex).with_pbr(Vec4::new(0.5, 0.5, 0.5, 1.0), 0.9, 0.0);
            if grid {
                material.material_type = MaterialType::Grid;
            }
            let e = world.spawn();
            world.add_component(e, Transform::new(Vec3::ZERO));
            world.add_component(e, GlobalTransform::default());
            world.add_component(e, AssetManager::create_plane(&renderer.device, 40.0));
            world.add_component(e, material);
            world.add_component(e, MeshRenderer::new());
            world.spawn_bundle(crate::bundles::DirectionalLightBundle::default());
            world.spawn_bundle(CameraBundle {
                position: Vec3::new(0.0, 3.0, 0.0),
                yaw: -std::f32::consts::FRAC_PI_2,
                pitch: -1.2,
                primary: true,
                ..Default::default()
            });
            let frame = render_world(&mut renderer, &mut world).await;
            let luma = |x: usize, y: usize| {
                let p = &frame[(y * 128 + x) * 4..];
                (p[0] as i32 + p[1] as i32 + p[2] as i32) / 3
            };
            let mut n = 0;
            for y in 64..128 {
                for x in 0..127 {
                    if (luma(x, y) - luma(x + 1, y)).abs() > 12 {
                        n += 1;
                    }
                }
            }
            n
        }
        pollster::block_on(async {
            let plane = edges(false).await;
            let grid = edges(true).await;
            // Measured: 0 edges for the PBR plane, 166 for the grid. Without the forward branch the
            // grid falls to the unlit pipeline and reads as a plane again.
            assert!(plane < 20, "a plain plane has no lines: {plane}");
            assert!(grid > plane + 80, "the grid draws its lines: {plane} → {grid}");
        });
    }

    /// `bloom_radius` and `bloom_iterations` widen the halo. A small, very bright cube (about
    /// 12 px across) in a dark scene; the halo is the mean brightness of a ring 8–24 px from its
    /// centre. The default blur's taps are one full-frame texel apart, so its halo barely leaves
    /// the cube — a ring at 25 px read the same with bloom on and off. Before these two existed
    /// that reach was fixed (`bloom_3d`: exactly 0.00 beyond 80 px at every intensity).
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn bloom_radius_and_iterations_widen_the_halo() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        // Mean luma of the ring's horizontal arms, of its vertical arms (the blur is separable, so
        // each axis has its own step), and of the cube's centre.
        async fn halo(radius: f32, iterations: u32, intensity: f32) -> ([f32; 2], f32) {
            let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
            renderer.taa = None;
            renderer.ssgi = None;
            renderer.ssr = None;
            renderer.volumetric = None;
            renderer.bloom_radius = radius;
            renderer.bloom_iterations = iterations;
            renderer.bloom_intensity = intensity;
            let mut world = World::new();
            let mut assets = AssetManager::new();
            let tex = assets.create_white_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            );
            let glow = world.spawn();
            world.add_component(glow, Transform::new(Vec3::new(0.0, 0.0, -5.0)).with_scale(Vec3::splat(0.2)));
            world.add_component(glow, GlobalTransform::default());
            world.add_component(glow, AssetManager::create_cube(&renderer.device));
            world.add_component(glow, Material::new(tex).with_unlit(Vec4::new(30.0, 30.0, 30.0, 1.0)));
            world.add_component(glow, MeshRenderer::new());
            world.spawn_bundle(CameraBundle {
                position: Vec3::ZERO,
                yaw: -std::f32::consts::FRAC_PI_2,
                pitch: 0.0,
                primary: true,
                ..Default::default()
            });
            let frame = render_world(&mut renderer, &mut world).await;
            let luma = |x: usize, y: usize| {
                let p = &frame[(y * 128 + x) * 4..];
                (p[0] as f32 + p[1] as f32 + p[2] as f32) / 3.0
            };
            let (mut arms, mut n) = ([0.0f32; 2], [0.0f32; 2]);
            for y in 0..128 {
                for x in 0..128 {
                    let (dx, dy) = (x as f32 - 63.5, y as f32 - 63.5);
                    let d = (dx * dx + dy * dy).sqrt();
                    if (8.0..24.0).contains(&d) {
                        // Horizontal arm: within 30° of the X axis; vertical: of the Y axis.
                        let axis = if dy.abs() < dx.abs() * 0.577 {
                            0
                        } else if dx.abs() < dy.abs() * 0.577 {
                            1
                        } else {
                            continue;
                        };
                        arms[axis] += luma(x, y);
                        n[axis] += 1.0;
                    }
                }
            }
            ([arms[0] / n[0], arms[1] / n[1]], luma(64, 64))
        }
        pollster::block_on(async {
            let (none, _) = halo(1.0, 1, 0.0).await;
            let (base, centre) = halo(1.0, 1, 0.8).await;
            let (wide, _) = halo(3.0, 1, 0.8).await;
            let (soft, _) = halo(1.0, 4, 0.8).await;
            // Measured (horizontal / vertical arm): no bloom 61.29 / 61.31, default 67.99 / 67.99
            // (centre 255), radius 3 132.80 / 132.82, four rounds 101.02 / 101.03. Caught: either
            // step never rewritten (that arm stays at the default's) and the rounds ignored.
            assert!(centre > 250.0, "the cube is saturated: {centre:.1}");
            for (axis, name) in [(0, "horizontal"), (1, "vertical")] {
                assert!(base[axis] > none[axis] + 3.0, "the default bloom reaches the {name} arm: {none:?} → {base:?}");
                assert!(wide[axis] > base[axis] + 20.0, "radius 3 spreads the {name} arm: {base:?} → {wide:?}");
                assert!(soft[axis] > base[axis] + 10.0, "four rounds spread the {name} arm: {base:?} → {soft:?}");
            }
        });
    }

    /// What a split-screen frame does once, it must do once: the HUD is drawn a single time over
    /// the whole target (drawn per view, a half-transparent sprite is laid over itself and comes
    /// out brighter), and the frame's animation advances by one `dt` however many cameras draw.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn a_split_screen_frame_draws_the_hud_and_steps_animation_once() {
        use crate::renderer::components::{Sprite, SpriteAnimation, Viewport};
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        pollster::block_on(async {
            let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
            renderer.taa = None;
            renderer.ssgi = None;
            let mut world = World::new();
            split_screen_scene(&renderer, &mut world);
            // The top two quarters; the bottom half is no view's, so it is cleared to black.
            split_camera(&mut world, -20.0, Viewport::quarter(0), true);
            split_camera(&mut world, 20.0, Viewport::quarter(1), false);

            let white = renderer
                .load_sprite_rgba(&[255; 4], 1, 1, gizmo_renderer::sprite::SpriteFilter::Nearest)
                .unwrap();
            // Half-transparent white over that black, then 30 fps animation stepped by 1/20 s:
            // one step is frame 1, two would be frame 3.
            world.spawn_bundle((
                Sprite::screen(white, gizmo_math::Vec2::splat(24.0), gizmo_math::Vec2::new(40.0, 90.0))
                    .with_color(Vec4::new(1.0, 1.0, 1.0, 0.5)),
                SpriteAnimation::new(4, 1, 0, 3, 30.0),
            ));
            let mut time = crate::core::time::Time::new();
            time.update(1.0 / 20.0);
            world.insert_resource(time);

            // CAS on: it is the last pass, so it is the one that has to write only the view's
            // rectangle — the HUD and the black bottom half below prove it did.
            if let Some(cas) = renderer.cas.as_mut() {
                cas.enabled = true;
            }
            let frame = render_world(&mut renderer, &mut world).await;
            let hud = frame[(102 * 128 + 52) * 4];
            assert!(
                (170..=205).contains(&hud),
                "the HUD reads {hud} over black: linear 0.5 is sRGB ~188 drawn once (measured 241 drawn once per view)",
            );
            assert_eq!(frame[(120 * 128 + 10) * 4], 0, "the part of the target no view covers is not black");
            let step = world
                .borrow::<SpriteAnimation>()
                .iter()
                .map(|(_, a)| a.frame())
                .next()
                .unwrap();
            assert_eq!(step, 1, "two views advanced the animation to frame {step}, not 1");
        });
    }

    /// Records two passes into ONE encoder from two cameras and reads both targets back.
    ///
    /// With `separate_views`, the first pass binds a `SceneView` of its own; without, both passes
    /// share the frame's buffer, which is the failure being pinned.
    async fn render_two_cameras(separate_views: bool) -> (Vec<u8>, Vec<u8>) {
        const W: u32 = 128;
        let mut renderer = crate::test_gpu::headless_renderer(W, W).await;
        renderer.taa = None;
        renderer.ssgi = None;
        let mut asset_manager = AssetManager::new();
        let mut world = World::new();
        let cam_a = two_camera_scene(&renderer, &mut asset_manager, &mut world);

        if separate_views {
            let v = gizmo_renderer::pipeline::SceneView::new(
                &renderer.device,
                &renderer.scene,
                "test_view",
            );
            renderer.scene.views.push(v);
        }

        let (target_a, view_a) = mk_target(&renderer, W);
        let (target_b, view_b) = mk_target(&renderer, W);

        let mut encoder = renderer
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("two_cameras"),
            });

        // Pass one: camera A, at x = 0.
        if separate_views {
            renderer.scene.active_view = Some(0);
        }
        default_render_pass(&mut world, &mut encoder, &view_a, &mut renderer);
        renderer.scene.active_view = None;

        // Pass two: move the camera well to the side. Same entity — what matters is that the
        // second write lands after the first pass was *recorded*, which is the whole trap.
        if let Some(mut q) = world.query_mut::<crate::core::query::Mut<Transform>>() {
            for (id, mut t) in q.iter_mut() {
                if id == cam_a.id() {
                    t.position = Vec3::new(-4.0, 0.5, 5.0);
                }
            }
        }
        default_render_pass(&mut world, &mut encoder, &view_b, &mut renderer);

        renderer.queue.submit(std::iter::once(encoder.finish()));

        let a = read_target(&renderer, &target_a, W).await;
        let b = read_target(&renderer, &target_b, W).await;
        (a, b)
    }

    /// The scene both two-camera fixtures use: an off-centre cube, a floor to receive its shadow,
    /// a shadow-casting sun, and one camera.
    ///
    /// Shared so the single-camera reference and the two-pass run cannot drift — a reference that
    /// rendered a different scene would compare nothing.
    fn two_camera_scene(
        renderer: &Renderer,
        asset_manager: &mut AssetManager,
        world: &mut World,
    ) -> crate::core::Entity {
        let tex = asset_manager.create_white_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
        );
        let tex2 = tex.clone();

        // An off-centre cube: where it lands in frame is what separates the two cameras.
        let cube = world.spawn();
        world.add_component(cube, Transform::new(Vec3::new(1.2, 0.0, 0.0)));
        world.add_component(cube, GlobalTransform::default());
        world.add_component(cube, AssetManager::create_cube(&renderer.device));
        world.add_component(
            cube,
            Material::new(tex).with_pbr(Vec4::new(0.9, 0.5, 0.2, 1.0), 0.5, 0.0),
        );
        world.add_component(cube, MeshRenderer::new());

        // A floor for the cube to cast onto. Without something receiving a shadow, the cascade
        // split is unobservable and a test comparing frames measures only the camera.
        let floor = world.spawn();
        world.add_component(
            floor,
            Transform::new(Vec3::new(0.0, -1.5, 0.0)).with_scale(Vec3::new(12.0, 0.2, 12.0)),
        );
        world.add_component(floor, GlobalTransform::default());
        world.add_component(floor, AssetManager::create_cube(&renderer.device));
        world.add_component(
            floor,
            Material::new(tex2).with_pbr(Vec4::new(0.85, 0.85, 0.85, 1.0), 0.9, 0.0),
        );
        world.add_component(floor, MeshRenderer::new());

        // A shadow-casting Sun, deliberately. This used to be `Generic`, because the cascades were
        // shared state `SceneView` did not separate — with the sun in, the two passes contaminated
        // each other's shadows and the test measured that instead of the camera. Per-view cascade
        // uniforms (2026-08-24) closed it, so the harder scene is now the one worth guarding.
        world.spawn_bundle(DirectionalLightBundle {
            intensity: 3.0,
            ..Default::default()
        });
        // A point light, so the **cluster table** is exercised too. Clusters are a view-space
        // grid built from the camera's frustum; with only a directional sun in the scene the
        // table is empty and separating it per view would be unmeasurable.
        world.spawn_bundle(crate::bundles::PointLightBundle {
            position: Vec3::new(2.0, 1.0, 1.0),
            color: Vec3::new(1.0, 0.8, 0.6),
            intensity: 14.0,
            radius: 9.0,
        });

        // One camera, moved between the two recordings. Moving it is what makes the trap
        // reproducible: the second write has to land after the first pass is already recorded.
        world.spawn_bundle(CameraBundle {
            position: Vec3::new(0.0, 0.5, 5.0),
            yaw: -std::f32::consts::FRAC_PI_2,
            pitch: 0.0,
            primary: true,
            ..Default::default()
        })

    }

    /// The **first** camera's frame, drawn alone in its own encoder — no second pass to collide
    /// with, so this is what a correct two-pass *first* frame has to equal.
    ///
    /// The first, not the second, and finding that out was the measurement. `queue.write_buffer`
    /// orders against submission, so when the two passes share a uniform they both read whichever
    /// camera was written **last** — which is the second one. The second pass is therefore right
    /// by accident; the first is the one that renders someone else's camera.
    async fn render_first_camera_alone() -> Vec<u8> {
        const W: u32 = 128;
        let mut renderer = crate::test_gpu::headless_renderer(W, W).await;
        renderer.taa = None;
        renderer.ssgi = None;
        let mut asset_manager = AssetManager::new();
        let mut world = World::new();
        two_camera_scene(&renderer, &mut asset_manager, &mut world);
        // Left where `two_camera_scene` puts it: this *is* the first recording's camera.
        render_world(&mut renderer, &mut world).await
    }

    /// A colour target the render pass can draw into and `read_target` can copy out of.
    fn mk_target(renderer: &Renderer, w: u32) -> (wgpu::Texture, wgpu::TextureView) {
        let t = renderer.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("two_cameras_target"),
            size: wgpu::Extent3d {
                width: w,
                height: w,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: renderer.config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let v = t.create_view(&wgpu::TextureViewDescriptor::default());
        (t, v)
    }

    /// Copies a target back as RGBA8 bytes. `w * 4` is 512 here, already 256-aligned.
    async fn read_target(renderer: &Renderer, tex: &wgpu::Texture, w: u32) -> Vec<u8> {
        let staging = renderer.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("two_cameras_readback"),
            size: (w * w * 4) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut enc = renderer
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(w * 4),
                    rows_per_image: Some(w),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: w,
                depth_or_array_layers: 1,
            },
        );
        renderer.queue.submit(std::iter::once(enc.finish()));
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |v| tx.send(v).unwrap());
        let _ = renderer.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        rx.recv().unwrap().unwrap();
        let out = slice
            .get_mapped_range()
            .expect("a just-mapped buffer's full range is always valid")
            .to_vec();
        staging.unmap();
        out
    }

    /// A baked probe grid has to reach the frame, and reach it as the CPU path computes it.
    ///
    /// `gi::ProbeGrid` was written, tested and unplugged — `to_gpu_data` had exactly one caller,
    /// its own test. Uploading it is only half the job: the shader's sampling has to be the same
    /// sampling, or the GPU path and the CPU path are two different features that happen to share
    /// a name.
    ///
    /// So this checks both halves. The grid changes the frame at all, and it changes it in the
    /// direction the coefficients say: a grid baked from a red light makes the red channel rise
    /// more than the blue.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn a_baked_probe_grid_reaches_the_frame_with_the_right_colour() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        pollster::block_on(async {
            let none = render_with_probes(None).await;
            let red = render_with_probes(Some(Vec3::new(1.0, 0.05, 0.05))).await;
            let blue = render_with_probes(Some(Vec3::new(0.05, 0.05, 1.0))).await;

            let (lit, total) = changed_pixels(&none, &red, 128);
            assert!(
                lit >= 500,
                "a baked grid changed {lit}/{total} pixels — it is not reaching the lighting pass",
            );

            // Direction, not just magnitude. Mean channel deltas against the unlit reference.
            let d_red = mean_channel_delta(&none, &red);
            let d_blue = mean_channel_delta(&none, &blue);
            assert!(
                d_red[0] > d_red[2] + 1.0,
                "a red-baked grid raised red by {:.2} and blue by {:.2} — the coefficients are \
                 not being evaluated, only their magnitude",
                d_red[0],
                d_red[2],
            );
            assert!(
                d_blue[2] > d_blue[0] + 1.0,
                "a blue-baked grid raised blue by {:.2} and red by {:.2}",
                d_blue[2],
                d_blue[0],
            );
        });
    }

    /// Mean per-channel difference between two RGBA8 frames, as `[r, g, b]`.
    fn mean_channel_delta(a: &[u8], b: &[u8]) -> [f32; 3] {
        let (a_px, _) = a.as_chunks::<4>();
        let (b_px, _) = b.as_chunks::<4>();
        let mut sums = [0.0f64; 3];
        for (pa, pb) in a_px.iter().zip(b_px) {
            for c in 0..3 {
                sums[c] += f64::from(pb[c]) - f64::from(pa[c]);
            }
        }
        let n = a_px.len().max(1) as f64;
        [
            (sums[0] / n) as f32,
            (sums[1] / n) as f32,
            (sums[2] / n) as f32,
        ]
    }

    /// A white floor under a dim sun, optionally with a probe grid baked from one coloured light.
    ///
    /// The floor is white and rough so the indirect term is the only thing distinguishing the
    /// three runs; a coloured material would tint the result and hide a grid that did nothing.
    async fn render_with_probes(probe_colour: Option<Vec3>) -> Vec<u8> {
        const W: u32 = 128;
        let mut renderer = crate::test_gpu::headless_renderer(W, W).await;
        renderer.taa = None;
        let mut asset_manager = AssetManager::new();
        let mut world = World::new();
        let tex = asset_manager.create_white_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
        );

        let floor = world.spawn();
        world.add_component(
            floor,
            Transform::new(Vec3::new(0.0, -1.0, 0.0)).with_scale(Vec3::new(12.0, 0.2, 12.0)),
        );
        world.add_component(floor, GlobalTransform::default());
        world.add_component(floor, AssetManager::create_cube(&renderer.device));
        world.add_component(
            floor,
            Material::new(tex).with_pbr(Vec4::new(1.0, 1.0, 1.0, 1.0), 0.95, 0.0),
        );
        world.add_component(floor, MeshRenderer::new());

        world.spawn_bundle(CameraBundle {
            position: Vec3::new(0.0, 1.5, 4.0),
            yaw: -std::f32::consts::FRAC_PI_2,
            pitch: -0.35,
            primary: true,
            ..Default::default()
        });
        // Dim, so the indirect term is not lost under direct light.
        world.spawn_bundle(DirectionalLightBundle {
            intensity: 0.3,
            ..Default::default()
        });

        if let Some(colour) = probe_colour {
            let mut grid = gizmo_renderer::gi::ProbeGrid::new(
                Vec3::new(-8.0, -2.0, -8.0),
                Vec3::new(8.0, 4.0, 8.0),
                [3, 3, 3],
            );
            // Bake by hand rather than through `add_directional_light`, so the test states the
            // coefficients it expects to see rather than trusting a second function to produce
            // them. L0 alone is a uniform sky of this colour.
            for p in &mut grid.probes {
                p.coeffs.l0 = colour;
            }
            if let Some(def) = renderer.deferred.as_mut() {
                let (device, queue) = (&renderer.device, &renderer.queue);
                def.irradiance.upload(device, queue, &grid);
                assert_eq!(def.irradiance.probe_count, 27, "the grid did not upload");
            }
        }

        render_world(&mut renderer, &mut world).await
    }

    /// The spatial index must not change what is drawn — only how fast the decision is reached.
    ///
    /// This is the assertion that makes the index safe to turn on. An index in front of the
    /// batcher's walk is a *cull*: anything it does not return is not drawn. So the only way it can
    /// be correct is by returning everything the linear walk would have kept, and the only way to
    /// know that is to render both and compare.
    ///
    /// Pixel-identical, not approximately: a candidate set missing one entity shows up as a hole,
    /// and a hole is exactly what a threshold would forgive.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn the_spatial_index_renders_the_same_frame_as_the_linear_walk() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        pollster::block_on(async {
            let linear = render_scattered_cubes(false).await;
            let indexed = render_scattered_cubes(true).await;
            let (differ, total) = changed_pixels(&linear, &indexed, 128);
            assert_eq!(
                differ, 0,
                "the indexed path differs from the linear one on {differ}/{total} pixels — the \
                 index dropped something the walk would have drawn",
            );

            // And the scene is not trivially empty: a blank frame compared against a blank frame
            // would pass the assertion above and prove nothing.
            let empty = render_scattered_cubes_empty().await;
            let (drawn, _) = changed_pixels(&linear, &empty, 128);
            assert!(
                drawn >= 500,
                "the fixture drew only {drawn} pixels of geometry — it cannot detect a dropped \
                 entity",
            );
        });
    }

    /// A grid of cubes spread across the view, half of them behind the camera.
    ///
    /// Spread on purpose: a cluster in one spot lands in one BVH leaf and the index would agree
    /// with the walk for the wrong reason.
    async fn render_scattered_cubes(with_index: bool) -> Vec<u8> {
        const W: u32 = 128;
        let mut renderer = crate::test_gpu::headless_renderer(W, W).await;
        renderer.taa = None;
        let mut asset_manager = AssetManager::new();
        let mut world = World::new();
        scattered_cube_scene(&renderer, &mut asset_manager, &mut world, 64);

        if with_index {
            // `GlobalTransform` has to be current before the index reads bounds from it — the
            // transform systems have not run yet at this point in a headless fixture.
            crate::systems::ensure_global_transforms(&mut world);
            let mut index = crate::systems::render::visibility_index::VisibilityIndex::default();
            index.rebuild_from(&world);
            assert!(!index.is_empty(), "the index rebuilt to nothing");
            world.insert_resource(index);
        }

        render_world(&mut renderer, &mut world).await
    }

    /// The same scene with no cubes — the reference for "did anything draw at all".
    async fn render_scattered_cubes_empty() -> Vec<u8> {
        const W: u32 = 128;
        let mut renderer = crate::test_gpu::headless_renderer(W, W).await;
        renderer.taa = None;
        let mut asset_manager = AssetManager::new();
        let mut world = World::new();
        scattered_cube_scene(&renderer, &mut asset_manager, &mut world, 0);
        render_world(&mut renderer, &mut world).await
    }

    /// `n` cubes on a grid in front of the camera, plus a light and a camera.
    fn scattered_cube_scene(
        renderer: &Renderer,
        asset_manager: &mut AssetManager,
        world: &mut World,
        n: usize,
    ) {
        let tex = asset_manager.create_white_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
        );
        let mesh = AssetManager::create_cube(&renderer.device);
        for i in 0..n {
            // An 8×8 grid spanning the view, **all at one depth**. Varying z was the first version
            // and it made the test blind: cubes at different depths overlap in perspective, so
            // removing one from the index changed no pixels — it was hidden behind a nearer one.
            // A single plane means every cube owns pixels of its own, which is what lets a dropped
            // entity show up. The BVH still has x and y to split on.
            let (gx, gy) = (i % 8, i / 8);
            let x = (gx as f32 - 3.5) * 1.1;
            let y = (gy as f32 - 3.5) * 1.1;
            let z = -6.0;
            world.spawn_bundle((
                Transform::new(Vec3::new(x, y, z)).with_scale(Vec3::splat(0.4)),
                GlobalTransform::default(),
                mesh.clone(),
                Material::new(tex.clone()).with_pbr(
                    Vec4::new(0.9, 0.5 + (i % 3) as f32 * 0.15, 0.3, 1.0),
                    0.5,
                    0.0,
                ),
                MeshRenderer::new(),
            ));
        }
        world.spawn_bundle(CameraBundle {
            position: Vec3::new(0.0, 0.0, 2.0),
            yaw: -std::f32::consts::FRAC_PI_2,
            pitch: 0.0,
            primary: true,
            ..Default::default()
        });
        world.spawn_bundle(DirectionalLightBundle {
            intensity: 3.0,
            ..Default::default()
        });
    }

    /// A registered material draws with its own shader, and a dangling id draws nothing.
    ///
    /// Two halves, and the second is the one worth having. `MaterialType::Custom(id)` routes
    /// through `MaterialRegistry`; an id with no entry behind it must draw **nothing** rather than
    /// falling through to PBR. A silent fallback is exactly how `routing.rs`'s own module docs
    /// describe two capabilities dying — an object shaded as something else is a wrong answer,
    /// while an object that vanishes is a question.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn a_registered_material_draws_and_a_dangling_id_draws_nothing() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        pollster::block_on(async {
            let pbr = render_custom_material_cube(CustomCase::Pbr).await;
            let custom = render_custom_material_cube(CustomCase::Registered).await;
            let dangling = render_custom_material_cube(CustomCase::Dangling).await;
            let empty = render_custom_material_cube(CustomCase::NoCube).await;

            let (differs, total) = changed_pixels(&pbr, &custom, 128);
            assert!(
                differs >= 300,
                "a registered material changed {differs}/{total} pixels against PBR — its shader \
                 is not reaching the frame",
            );

            let (leaked, total) = changed_pixels(&dangling, &empty, 128);
            assert_eq!(
                leaked, 0,
                "an unregistered id drew {leaked}/{total} pixels — it fell through to some \
                 engine pipeline instead of drawing nothing",
            );
        });
    }

    /// Which material the cube in `render_custom_material_cube` carries.
    #[derive(Clone, Copy, PartialEq)]
    enum CustomCase {
        /// The engine's own PBR — the reference.
        Pbr,
        /// A registered custom material.
        Registered,
        /// `MaterialType::Custom` with an id nothing was registered under.
        Dangling,
        /// No cube at all — what "drew nothing" has to look like.
        NoCube,
    }

    /// The smallest custom shader that is unmistakably not one of the engine's: flat magenta.
    ///
    /// It declares only groups 0 and `#{INSTANCE_GROUP}`, which is the minimum a forward draw
    /// needs, and it goes through the same composer the engine's shaders do — so `#import` and the
    /// group-index defs work here exactly as they do in `unlit.wgsl`.
    const TEST_CUSTOM_WGSL: &str = r#"
#import gizmo::common::{SceneUniforms}
@group(0) @binding(0) var<uniform> scene: SceneUniforms;
struct InstanceRaw {
    m0: vec4<f32>, m1: vec4<f32>, m2: vec4<f32>, m3: vec4<f32>,
    albedo_color: vec4<f32>, pbr: vec4<f32>, ambient: vec4<f32>, emissive: vec4<f32>,
};
@group(#{INSTANCE_GROUP}) @binding(0) var<storage, read> instances: array<InstanceRaw>;
struct VIn {
    @location(0) position: vec3<f32>, @location(1) color: vec3<f32>,
    @location(2) normal: vec3<f32>, @location(3) tex_coords: vec2<f32>,
    @location(4) joint_indices: vec4<u32>, @location(5) joint_weights: vec4<f32>,
};
struct VOut { @builtin(position) clip_position: vec4<f32> };
@vertex
fn vs_main(@builtin(instance_index) idx: u32, input: VIn) -> VOut {
    let i = instances[idx];
    let model = mat4x4<f32>(i.m0, i.m1, i.m2, i.m3);
    var out: VOut;
    out.clip_position = scene.view_proj * model * vec4<f32>(input.position, 1.0);
    return out;
}
@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    return vec4<f32>(1.0, 0.0, 1.0, 1.0);
}
"#;

    /// One cube in front of a camera, shaded four different ways.
    async fn render_custom_material_cube(case: CustomCase) -> Vec<u8> {
        const W: u32 = 128;
        let mut renderer = crate::test_gpu::headless_renderer(W, W).await;
        let mut asset_manager = AssetManager::new();
        let mut world = World::new();
        let tex = asset_manager.create_white_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
        );

        let id = {
            let m = gizmo_renderer::custom_material::CustomMaterial::from_wgsl(
                &renderer.device,
                &renderer.scene,
                "test_custom",
                TEST_CUSTOM_WGSL,
                gizmo_renderer::custom_material::CustomPipelineOptions::default(),
            );
            renderer.custom_materials.register(m)
        };

        if case != CustomCase::NoCube {
            let cube = world.spawn();
            world.add_component(cube, Transform::new(Vec3::ZERO));
            world.add_component(cube, GlobalTransform::default());
            world.add_component(cube, AssetManager::create_cube(&renderer.device));
            let mut mat =
                Material::new(tex).with_pbr(Vec4::new(0.8, 0.4, 0.2, 1.0), 0.5, 0.0);
            mat.material_type = match case {
                CustomCase::Pbr => gizmo_renderer::components::MaterialType::Pbr,
                CustomCase::Registered => {
                    gizmo_renderer::components::MaterialType::Custom(id)
                }
                // One past the end: registered ids start at 0, so this resolves to nothing.
                CustomCase::Dangling => gizmo_renderer::components::MaterialType::Custom(
                    gizmo_renderer::custom_material::MaterialId(id.0 + 1),
                ),
                CustomCase::NoCube => unreachable!(),
            };
            world.add_component(cube, mat);
            world.add_component(cube, MeshRenderer::new());
        }

        world.spawn_bundle(CameraBundle {
            position: Vec3::new(0.0, 0.0, 4.0),
            yaw: -std::f32::consts::FRAC_PI_2,
            ..Default::default()
        });
        world.spawn_bundle(DirectionalLightBundle::default());

        render_world(&mut renderer, &mut world).await
    }

    /// A normal map bound through `AssetManager::material` has to reach the shading.
    ///
    /// The 7-entry material layout has always had a normal slot; what it did not have was a public
    /// door, so a hand-built scene could bind base colour and nothing else and the only way to add
    /// surface detail was to add geometry. This renders one flat quad twice — once with the
    /// builder's defaults, once with a generated normal map as its only non-default slot — and
    /// requires the second to be shaded differently.
    ///
    /// The map is `Rgba8Unorm` on purpose. A normal map is data, not colour; as sRGB the engine
    /// would linearise it and every slope would come out wrong, which is the kind of thing that
    /// looks plausible in a screenshot and is wrong everywhere.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn a_normal_map_bound_through_the_builder_changes_the_shading() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        pollster::block_on(async {
            let plain = render_normal_mapped_quad(false).await;
            let mapped = render_normal_mapped_quad(true).await;
            let (changed, total) = changed_pixels(&plain, &mapped, 128);
            assert!(
                changed >= 400,
                "binding a normal map changed {changed}/{total} pixels — it is not reaching the \
                 shading",
            );
        });
    }

    /// One flat quad under a raking light. `mapped` binds a generated normal map through the
    /// builder; otherwise the two runs are identical, geometry included.
    async fn render_normal_mapped_quad(mapped: bool) -> Vec<u8> {
        const W: u32 = 128;
        const MAP: u32 = 256;
        let renderer = crate::test_gpu::headless_renderer(W, W).await;
        let mut asset_manager = AssetManager::new();
        let mut world = World::new();

        let bind_group = if mapped {
            // Ridges along one axis: a raking light turns them into stripes, and a flat normal
            // cannot produce stripes at all.
            let mut pixels = vec![0u8; (MAP * MAP * 4) as usize];
            for y in 0..MAP {
                for x in 0..MAP {
                    let u = x as f32 / MAP as f32;
                    let slope = (u * std::f32::consts::TAU * 8.0).sin() * 0.9;
                    let n = Vec3::new(-slope, 0.0, 1.0).normalize();
                    let i = ((y * MAP + x) * 4) as usize;
                    pixels[i] = ((n.x * 0.5 + 0.5) * 255.0) as u8;
                    pixels[i + 1] = ((n.y * 0.5 + 0.5) * 255.0) as u8;
                    pixels[i + 2] = ((n.z * 0.5 + 0.5) * 255.0) as u8;
                    pixels[i + 3] = 255;
                }
            }
            let tex = renderer.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("test_normal_map"),
                size: wgpu::Extent3d {
                    width: MAP,
                    height: MAP,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            renderer.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(MAP * 4),
                    rows_per_image: Some(MAP),
                },
                wgpu::Extent3d {
                    width: MAP,
                    height: MAP,
                    depth_or_array_layers: 1,
                },
            );
            let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
            AssetManager::material()
                .normal(&view)
                .label("test_mapped")
                .build(
                    &mut asset_manager,
                    &renderer.device,
                    &renderer.queue,
                    &renderer.scene.texture_bind_group_layout,
                )
        } else {
            // The same door, everything default — so the comparison is the map and nothing else.
            AssetManager::material().label("test_plain").build(
                &mut asset_manager,
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            )
        };

        let quad = world.spawn();
        world.add_component(
            quad,
            Transform::new(Vec3::new(0.0, 0.0, 0.0)).with_scale(Vec3::new(3.0, 3.0, 0.1)),
        );
        world.add_component(quad, GlobalTransform::default());
        world.add_component(quad, AssetManager::create_cube(&renderer.device));
        world.add_component(
            quad,
            Material::new(bind_group).with_pbr(Vec4::new(0.8, 0.7, 0.6, 1.0), 0.6, 0.0),
        );
        world.add_component(quad, MeshRenderer::new());

        world.spawn_bundle(CameraBundle {
            position: Vec3::new(0.0, 0.0, 5.0),
            yaw: -std::f32::consts::FRAC_PI_2,
            ..Default::default()
        });
        // Raking light: almost along the surface, so a slope change is a brightness change.
        world.spawn_bundle(DirectionalLightBundle {
            rotation: gizmo_math::Quat::from_rotation_y(1.35) * gizmo_math::Quat::from_rotation_x(-0.15),
            intensity: 3.5,
            ..Default::default()
        });

        let mut renderer = renderer;
        render_world(&mut renderer, &mut world).await
    }

    /// Same scene, but with a `ParticleEmitter` on the cube, rendered over several frames from
    /// ONE renderer — particles need time to be spawned and to move.
    async fn render_emitter_scene(with_emitter: bool, frames: u32) -> Vec<u8> {
        const W: u32 = 128;
        let mut renderer = crate::test_gpu::headless_renderer(W, W).await;
        let mut asset_manager = AssetManager::new();
        let mut world = World::new();
        world.insert_resource(gizmo_core::time::Time::new());
        let tex = asset_manager.create_white_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
        );
        let floor = world.spawn();
        world.add_component(
            floor,
            Transform::new(Vec3::new(0.0, -1.2, 0.0)).with_scale(Vec3::new(20.0, 0.2, 20.0)),
        );
        world.add_component(floor, GlobalTransform::default());
        world.add_component(floor, AssetManager::create_cube(&renderer.device));
        world.add_component(
            floor,
            Material::new(tex.clone()).with_pbr(Vec4::new(0.8, 0.8, 0.8, 1.0), 0.0, 0.9),
        );
        world.add_component(floor, MeshRenderer::new());

        let emitter = world.spawn();
        world.add_component(emitter, Transform::new(Vec3::new(0.0, -0.5, 0.0)));
        world.add_component(emitter, GlobalTransform::default());
        if with_emitter {
            world.add_component(
                emitter,
                {
                    let mut e = crate::renderer::components::ParticleEmitter::with_rate(400.0);
                    e.size_start = 0.6;
                    e
                },
            );
        }

        world.spawn_bundle(CameraBundle {
            position: Vec3::new(-5.0, 1.0, 0.0),
            yaw: 0.0,
            pitch: -0.1,
            primary: true,
            ..Default::default()
        });
        world.spawn_bundle(DirectionalLightBundle::default());

        let mut last = Vec::new();
        for _ in 0..frames {
            if let Some(mut t) = world.get_resource_mut::<gizmo_core::time::Time>() {
                t.update(1.0 / 60.0);
            }
            last = render_world(&mut renderer, &mut world).await;
        }
        last
    }

    /// **A `ParticleEmitter` has to produce particles anyone can see.**
    ///
    /// The emitter-to-GPU bridge is young and was missing for everyone outside the studio once
    /// already (see `spawn_from_emitters`), and the particle system is one of two GPU subsystems
    /// this path builds for *every* renderer whether the scene wants them or not. Measured on the
    /// way in: with no emitter in the world, switching the particle system off changes 0 of 65536
    /// bytes — it is idle, not idling loudly — and with one, it paints 197 pixels of 16384 after
    /// ten frames and 355 after sixty, growing as the spawn accumulates. The floor here is set
    /// far below that: this asks whether particles reach the frame at all.
    #[test]
    fn particles_from_an_emitter_reach_the_frame() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        pollster::block_on(async {
            let with = render_emitter_scene(true, 10).await;
            let without = render_emitter_scene(false, 10).await;
            let (changed, total) = changed_pixels(&with, &without, 128);
            assert!(
                changed >= 40,
                "an emitter spawning 400 particles a second changed {changed}/{total} pixels over                  ten frames — nothing is reaching the frame (measured 197)",
            );
        });
    }

    /// A matte floor with — optionally — a decal projector volume straddling its surface.
    async fn render_decal_scene(with_decal: bool) -> Vec<u8> {
        const W: u32 = 128;
        let mut renderer = crate::test_gpu::headless_renderer(W, W).await;
        let mut asset_manager = AssetManager::new();
        let mut world = World::new();
        let tex = asset_manager.create_white_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
        );

        // Matte floor, top face at y = -1.1.
        let floor = world.spawn();
        world.add_component(
            floor,
            Transform::new(Vec3::new(0.0, -1.2, 0.0)).with_scale(Vec3::new(20.0, 0.2, 20.0)),
        );
        world.add_component(floor, GlobalTransform::default());
        world.add_component(floor, AssetManager::create_cube(&renderer.device));
        world.add_component(
            floor,
            Material::new(tex.clone()).with_pbr(Vec4::new(0.8, 0.8, 0.8, 1.0), 0.0, 0.9),
        );
        world.add_component(floor, MeshRenderer::new());

        if with_decal {
            // A projector volume straddling the floor surface, right where the camera looks.
            let d = world.spawn();
            world.add_component(
                d,
                Transform::new(Vec3::new(0.0, -1.1, 0.0)).with_scale(Vec3::splat(3.0)),
            );
            world.add_component(d, GlobalTransform::default());
            world.add_component(
                d,
                crate::renderer::components::Decal::new(
                    tex.clone(),
                    Vec4::new(1.0, 0.0, 0.0, 1.0),
                ),
            );
        }

        world.spawn_bundle(CameraBundle {
            position: Vec3::new(-5.0, 2.0, 0.0),
            yaw: 0.0,
            pitch: -0.35,
            primary: true,
            ..Default::default()
        });
        world.spawn_bundle(DirectionalLightBundle::default());

        render_world(&mut renderer, &mut world).await
    }

    /// Render a floor lit by `MAX_LIGHTS` point lights stacked in one spot, where only the
    /// **last-ranked** one emits any light (`white_last`) or none of them do.
    ///
    /// All the lights share a position, radius and intensity, so they score identically in
    /// `collect_scene_lights` and the ranking falls through to its last tie-break — entity id.
    /// Spawn order therefore decides slots, and the light spawned last lands in slot
    /// `MAX_LIGHTS - 1`. That is the point: it is the only way to put a light *at the end* of the
    /// array, because every rule above that tie-break exists to put a useful light at the front.
    async fn render_last_slot_light_scene(white_last: bool) -> Vec<u8> {
        const W: u32 = 128;
        let mut renderer = crate::test_gpu::headless_renderer(W, W).await;
        let mut asset_manager = AssetManager::new();
        let mut world = World::new();
        let tex = asset_manager.create_white_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
        );

        let floor = world.spawn();
        world.add_component(
            floor,
            Transform::new(Vec3::new(0.0, -1.2, 0.0)).with_scale(Vec3::new(20.0, 0.2, 20.0)),
        );
        world.add_component(floor, GlobalTransform::default());
        world.add_component(floor, AssetManager::create_cube(&renderer.device));
        world.add_component(
            floor,
            Material::new(tex.clone()).with_pbr(Vec4::new(0.8, 0.8, 0.8, 1.0), 0.0, 0.9),
        );
        world.add_component(floor, MeshRenderer::new());

        let n = crate::renderer::MAX_LIGHTS;
        for i in 0..n {
            let lit = white_last && i == n - 1;
            let e = world.spawn();
            world.add_component(e, Transform::new(Vec3::new(0.0, 0.5, 0.0)));
            world.add_component(e, GlobalTransform::default());
            world.add_component(
                e,
                crate::renderer::components::PointLight::new(
                    if lit { Vec3::ONE } else { Vec3::ZERO },
                    40.0,
                    8.0,
                ),
            );
        }

        world.spawn_bundle(CameraBundle {
            position: Vec3::new(-5.0, 2.0, 0.0),
            yaw: 0.0,
            pitch: -0.35,
            primary: true,
            ..Default::default()
        });
        // Deliberately no sun: the whole image must come from the light in the last slot, so a
        // shader that stops early produces a measurably darker frame rather than a slightly
        // different one.

        render_world(&mut renderer, &mut world).await
    }

    /// A floor lit (or not) by ONE small point light sitting just above it.
    ///
    /// What this guards: a *small* light is lit at all. Every other frame guard uses lights that
    /// cover most of the view, so none of them would notice clustering dropping the small ones —
    /// a light assigned to no cluster, or a fragment reading a cluster list that is never uploaded,
    /// both show up here.
    async fn render_small_clustered_light_scene(lit: bool) -> Vec<u8> {
        const W: u32 = 128;
        let mut renderer = crate::test_gpu::headless_renderer(W, W).await;
        let mut asset_manager = AssetManager::new();
        let mut world = World::new();
        let tex = asset_manager.create_white_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
        );

        // A long floor receding away from the camera, so depth varies across the frame.
        let floor = world.spawn();
        world.add_component(
            floor,
            Transform::new(Vec3::new(0.0, -1.0, -20.0)).with_scale(Vec3::new(6.0, 0.1, 40.0)),
        );
        world.add_component(floor, GlobalTransform::default());
        world.add_component(floor, AssetManager::create_cube(&renderer.device));
        world.add_component(
            floor,
            Material::new(tex.clone()).with_pbr(Vec4::new(0.9, 0.9, 0.9, 1.0), 0.0, 0.9),
        );
        world.add_component(floor, MeshRenderer::new());

        if lit {
            let e = world.spawn();
            world.add_component(e, Transform::new(Vec3::new(0.0, -0.4, -8.0)));
            world.add_component(e, GlobalTransform::default());
            world.add_component(
                e,
                crate::renderer::components::PointLight::new(Vec3::ONE, 30.0, 2.0),
            );
        }

        world.spawn_bundle(CameraBundle {
            position: Vec3::new(0.0, 0.6, 0.0),
            yaw: -std::f32::consts::FRAC_PI_2,
            pitch: -0.15,
            primary: true,
            ..Default::default()
        });
        // No sun: the only light in the frame is the one being clustered.

        render_world(&mut renderer, &mut world).await
    }

    /// **A small clustered light must still reach the frame.**
    ///
    /// # What this does NOT guard, measured rather than assumed
    ///
    /// It does not catch a CPU↔shader cluster *mapping* error. Adding `+ 1u` to the shader's slice
    /// index left this guard green (and every other frame guard too): a light big enough to see is a
    /// light the CPU assigned to the neighbouring slices as well, so reading the wrong one still
    /// finds it. Shrinking the light until that stopped being true would have made the lit patch too
    /// small to measure reliably.
    ///
    /// The mapping is guarded instead by `gizmo_renderer::clustered`'s GPU mirror test, which runs
    /// the shader's own `gizmo_cluster_index` over sample points and compares against the CPU's
    /// `cluster_of_point` — that one fails on 93 of 97 points for a one-slice error and 73 of 97 for
    /// a one-tile error. This test is kept for the thing it *does* cover, and this note is here
    /// because a guard whose claim is wrong is worse than no guard.
    #[test]
    fn a_small_clustered_light_reaches_the_frame() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        pollster::block_on(async {
            let lit = render_small_clustered_light_scene(true).await;
            let dark = render_small_clustered_light_scene(false).await;
            let (changed, total) = changed_pixels(&lit, &dark, 128);
            assert!(
                changed >= 100,
                "a small point light changed {changed}/{total} pixels — the fragment's cluster and \
                 the light's cluster do not agree (measured 1213)",
            );
        });
    }

    /// **The last light slot has to reach the frame.** The array was `[LightData; 10]` in Rust and
    /// `array<LightData, 10>` hand-written in seven shaders, and raising it to 32 (2026-08-17) is
    /// only real if the shaders read the tail they now declare. A loop that still stopped at ten,
    /// or one shader left at the old length reading a misaligned block, both show up here as a
    /// frame that does not change.
    ///
    /// Verified to fail: clamping the light loop back to `min(num_lights, 10u)` takes this from
    /// 11631 changed pixels to **0**. That took clamping *three* shaders — `deferred_lighting`,
    /// `shader` (the forward pass) and `volumetric` — because this frame is lit by more than one of
    /// them and either alone kept the light visible. Which is the argument for a guard that
    /// measures the **frame** rather than a shader: it covers every consumer, including the ones
    /// whoever raises the cap next does not know about.
    #[test]
    fn the_last_light_slot_reaches_the_frame() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        pollster::block_on(async {
            let lit = render_last_slot_light_scene(true).await;
            let dark = render_last_slot_light_scene(false).await;
            let (changed, total) = changed_pixels(&lit, &dark, 128);
            assert!(
                changed >= 2_000,
                "a light in slot MAX_LIGHTS-1 changed {changed}/{total} pixels — the shaders are \
                 not reading the end of the array they declare (measured 11631)",
            );
        });
    }

    /// **A decal has to land on the floor.** (Named to match the two guards above: one
    /// `cargo test reach_the_frame` runs the whole family.) The decal pass had no test of any
    /// kind: it blends
    /// into the G-buffer's albedo target from inside the deferred geometry pass, so nothing in
    /// the editor's forward path can see it (documented separately, and still open) and nothing
    /// in the game path was measuring it either.
    ///
    /// It reads the same written-flag SSR and SSGI got wrong, and it read it in the third and
    /// most fragile spelling — an exact `== 0.0`, now `< 0.5` like every other reader. That
    /// rewrite cannot change a pixel today (the encoder never produces a value in `(0, 0.5)`),
    /// which is exactly why it needs a measurement standing behind it rather than an argument:
    /// a red projector over a white floor moves 1133 of 16384 pixels, max delta 75.
    #[test]
    fn decals_reach_the_frame() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        pollster::block_on(async {
            let with = render_decal_scene(true).await;
            let without = render_decal_scene(false).await;
            let (changed, total) = changed_pixels(&with, &without, 128);
            assert!(
                changed >= 200,
                "spawning a decal over the floor changed {changed}/{total} pixels — it is not \
                 reaching the frame (measured 1133)",
            );
        });
    }

    /// The water route reaches the frame, and the waves in it actually move.
    ///
    /// `water.wgsl`, `water_pipeline` and `MaterialType::Water` were all written, all compiled
    /// every run, and **no pass ever called `set_pipeline` with any of them** — the material type
    /// routed to exactly `Pbr`'s answer, so a water surface was shaded as ordinary deferred PBR.
    /// That is the half this test's first assertion closes.
    ///
    /// The second half is the defect the wiring uncovered. The shader took its clock from
    /// `scene.camera_pos.w`, a slot `frame_uniforms.rs` fills with a constant `1.0` and
    /// `gpu_types.rs` documents as unused; elapsed time is `cascade_params.z`. So even once bound,
    /// the ocean would have been a fixed displaced surface — frozen at t = 1.0, which looks
    /// entirely plausible in a still frame. Comparing two *times* is the only thing that catches
    /// it, and this assertion goes red the moment the shader reads the old slot again.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn the_water_surface_is_drawn_by_its_own_pipeline_and_moves() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        pollster::block_on(async {
            let pbr = render_water_scene(false, 0.0).await;
            let t0 = render_water_scene(true, 0.0).await;
            let t2 = render_water_scene(true, 2.0).await;

            // Measured 10 568/16 384 wired, and exactly **0** with the `Water` arm of `route`
            // put back the way it was — the two frames were bit-identical, which is what "the
            // pipeline compiled and nothing bound it" looks like from the outside.
            let (routed, total) = changed_pixels(&pbr, &t0, 128);
            assert!(
                routed >= 4000,
                "the same surface shaded as water and as PBR differs in {routed}/{total} pixels \
                 (measured 10 568) — `MaterialType::Water` is falling through to the deferred PBR \
                 path again",
            );

            // Measured 3 730/16 384, and exactly **0** with `let time = scene.camera_pos.w`
            // restored. Zero, not "few": a constant clock gives the identical frame twice.
            let (moved, _) = changed_pixels(&t0, &t2, 128);
            assert!(
                moved >= 1200,
                "two seconds of elapsed time moved {moved}/{total} pixels of the ocean (measured \
                 3 730) — the Gerstner phase is not advancing, so the shader is reading a \
                 constant for time",
            );
        });
    }

    /// A tessellated water plane under a sun, at a given elapsed time.
    ///
    /// `water` selects between `Material::with_water` and a `with_pbr` carrying the **same**
    /// albedo, roughness and metallic — so the two frames differ by the pipeline and nothing else.
    /// The plane is subdivided because the displacement is per-vertex: on the four-vertex
    /// `create_plane` the whole ocean is a quad with four moving corners.
    async fn render_water_scene(water: bool, elapsed: f32) -> Vec<u8> {
        const W: u32 = 128;
        let mut renderer = crate::test_gpu::headless_renderer(W, W).await;
        renderer.taa = None;
        let mut asset_manager = AssetManager::new();
        let mut world = World::new();
        let tex = asset_manager.create_white_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
        );

        // The clock the shader reads, advanced in whole `Time::update` steps — `max_dt` caps a
        // single one at 50 ms, so asking for two seconds in one call would silently give 0.05.
        let mut time = gizmo_core::time::Time::new();
        while (time.elapsed() as f32) < elapsed {
            time.update(0.05);
        }
        world.insert_resource(time);

        let albedo = Vec4::new(0.10, 0.35, 0.50, 1.0);
        let material = if water {
            Material::new(tex).with_water(albedo)
        } else {
            // The numbers `with_water` sets, written out: same colour, same 0.05 roughness, same
            // metallic. Whatever separates the two frames is the pipeline.
            Material::new(tex).with_pbr(albedo, 0.05, 0.0)
        };

        let surface = world.spawn();
        world.add_component(surface, Transform::new(Vec3::ZERO));
        world.add_component(surface, GlobalTransform::default());
        world.add_component(
            surface,
            AssetManager::create_plane_subdivided(&renderer.device, 60.0, 48),
        );
        world.add_component(surface, material);
        world.add_component(surface, MeshRenderer::new());

        // Low and looking along the surface: a grazing view is where water stops looking like a
        // flat blue floor, and it is what the Fresnel term in `water.wgsl` shapes.
        world.spawn_bundle(CameraBundle {
            position: Vec3::new(0.0, 3.0, 16.0),
            yaw: -std::f32::consts::FRAC_PI_2,
            pitch: -0.22,
            primary: true,
            ..Default::default()
        });
        world.spawn_bundle(DirectionalLightBundle::default());

        render_world(&mut renderer, &mut world).await
    }

    /// Text reaches the frame, lands where it was told to, and a world label is occluded.
    ///
    /// The engine could not draw a glyph before this: `docs/CAPABILITY_GAPS.md` §A2, the gap its
    /// §E names first. Three assertions, because "some pixels changed" would pass for a renderer
    /// that draws the right number of boxes in the wrong place:
    ///
    /// 1. **It draws at all** — the frame differs from the same scene with no `Text`.
    /// 2. **It draws where it was put** — screen text in the top-left changes the top-left and
    ///    leaves the bottom-right alone. That is the assertion a y-flip error fails.
    /// 3. **A world label is depth-tested** — the same label vanishes behind a wall put in front
    ///    of it, which is the only thing separating the world pipeline from the screen one.
    ///
    /// The font is built in memory (`gizmo_renderer::text::synthetic`): this repository ships no
    /// typeface, and a test that reaches for a system font asserts against whichever one the
    /// machine happened to have.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn text_reaches_the_frame_where_it_was_placed_and_a_world_label_is_occluded() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        pollster::block_on(async {
            let blank = render_text_scene(TextCase::None).await;
            let screen = render_text_scene(TextCase::ScreenTopLeft).await;

            let (changed, total) = changed_pixels(&blank, &screen, 128);
            assert!(
                changed >= 300,
                "drawing text changed {changed}/{total} pixels (measured 869) — no glyph reached \
                 the frame",
            );

            // Where it landed. The string is anchored at (8, 8) in window pixels, so it belongs in
            // the top-left quadrant and nowhere near the bottom-right.
            let (tl, br) = (quadrant_changes(&blank, &screen, 0, 0), quadrant_changes(&blank, &screen, 64, 64));
            assert!(
                tl > 300 && br == 0,
                "text anchored at (8, 8) changed {tl} pixels top-left and {br} bottom-right \
                 (measured 869 and 0) — it is drawn at the wrong end of the screen, which is what \
                 a flipped y axis looks like",
            );

            // Depth. The same world label, once in the open and once behind a wall.
            let open = render_text_scene(TextCase::WorldLabel).await;
            let hidden = render_text_scene(TextCase::WorldLabelBehindWall).await;
            let wall_only = render_text_scene(TextCase::WallOnly).await;
            let (visible, _) = changed_pixels(&blank, &open, 128);
            assert!(
                visible >= 500,
                "a world label changed {visible} pixels in the open (measured 1 702) — it is not \
                 being drawn",
            );
            let (leaked, _) = changed_pixels(&wall_only, &hidden, 128);
            assert_eq!(
                leaked, 0,
                "a world label behind a wall changed {leaked} pixels — the world text pipeline is \
                 not depth-tested, so every label draws through the level. Measured 1 589 with \
                 `CompareFunction::Always` in its place, and 0 with the test back.",
            );
        });
    }

    /// `Sprite` reaches the frame, in the right place, in the right order, and not through walls.
    ///
    /// Each half is asserted as a pair against a reference, because a sprite that is never drawn
    /// passes every "is it hidden" check on its own:
    ///
    /// 1. **Screen placement** — a sprite at (8, 8) changes the top-left quadrant and leaves the
    ///    bottom-right alone (the y flip).
    /// 2. **World depth** — a world sprite is visible in the open and changes 0 pixels behind a
    ///    wall (the world pipeline's depth test).
    /// 3. **Order** — two identical opaque screen sprites: the higher `layer` wins whichever was
    ///    spawned first, so the frame equals that sprite alone. Two world sprites: the nearer one
    ///    wins although it was spawned first, which is the back-to-front sort and not queue order.
    /// 4. **`IsHidden`** — a hidden sprite draws nothing.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn sprites_reach_the_frame_in_place_in_order_and_behind_walls() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        use SpriteCase as C;
        pollster::block_on(async {
            let blank = render_sprite_scene(&[]).await;

            let screen = render_sprite_scene(&[C::Screen { layer: 0, blue: false }]).await;
            let (tl, br) = (quadrant_changes(&blank, &screen, 0, 0), quadrant_changes(&blank, &screen, 64, 64));
            assert!(
                tl > 300 && br == 0,
                "a 24 px sprite at (8, 8) changed {tl} pixels top-left and {br} bottom-right — \
                 either nothing is drawn or it is drawn at the wrong end of the screen",
            );

            let open = render_sprite_scene(&[C::World { z: 0.0, blue: false }]).await;
            let (visible, total) = changed_pixels(&blank, &open, 128);
            assert!(visible >= 300, "a world sprite changed {visible}/{total} pixels — it is not drawn");
            let wall = render_sprite_scene(&[C::Wall]).await;
            let behind = render_sprite_scene(&[C::Wall, C::World { z: 0.0, blue: false }]).await;
            let (leaked, _) = changed_pixels(&wall, &behind, 128);
            assert_eq!(leaked, 0, "a world sprite behind a wall changed {leaked} pixels — not depth-tested (measured 784 with \
                 `CompareFunction::Always` in the world pipeline, 0 with `LessEqual`)");

            let blue_alone = render_sprite_scene(&[C::Screen { layer: 1, blue: true }]).await;
            let layered = render_sprite_scene(&[C::Screen { layer: 1, blue: true }, C::Screen { layer: 0, blue: false }]).await;
            let (diff, _) = changed_pixels(&blue_alone, &layered, 128);
            assert_eq!(diff, 0, "the layer-0 sprite spawned second was painted over the layer-1 one ({diff} px)");

            let near_alone = render_sprite_scene(&[C::World { z: 1.5, blue: true }]).await;
            let sorted = render_sprite_scene(&[C::World { z: 1.5, blue: true }, C::World { z: 0.0, blue: false }]).await;
            let (diff, _) = changed_pixels(&near_alone, &sorted, 128);
            assert_eq!(
                diff, 0,
                "the farther world sprite, spawned second, showed over the nearer one ({diff} px) — \
                 world sprites are drawn in queue order instead of back to front (measured 890 with the \
                 depth key reversed)",
            );

            let hidden = render_sprite_scene(&[C::HiddenScreen]).await;
            let (diff, _) = changed_pixels(&blank, &hidden, 128);
            assert_eq!(diff, 0, "an `IsHidden` sprite changed {diff} pixels");
        });
    }

    /// A `Tilemap` draws exactly what the same tiles draw as separate sprites — pixel for pixel —
    /// and the whole map is one draw call.
    ///
    /// Compared against sprites rather than against "something changed", because a map that put
    /// every tile one cell off, or showed the wrong tileset cell, or hung up instead of down would
    /// still change pixels. The sprites are placed by hand from the documented rule (top-left
    /// origin, rows going down), so this pins the rule as well as the drawing.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn a_tilemap_draws_what_its_tiles_would_as_sprites_in_one_draw_call() {
        use gizmo_math::Vec2;
        use gizmo_renderer::components::{Sprite, Tilemap};
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        pollster::block_on(async {
            let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
            renderer.taa = None;
            // A 2 × 1 tileset: cell 0 red, cell 1 blue.
            let tileset = renderer
                .load_sprite_rgba(&[255, 0, 0, 255, 0, 0, 255, 255], 2, 1, gizmo_renderer::sprite::SpriteFilter::Nearest)
                .unwrap();
            let rows = ["01.", ".10"];
            let origin = Vec3::new(-1.5, 1.0, 0.0);
            let camera = || CameraBundle {
                position: Vec3::new(0.0, 0.0, 6.0),
                yaw: -std::f32::consts::FRAC_PI_2,
                primary: true,
                ..Default::default()
            };

            let mut blank = World::new();
            blank.spawn_bundle(camera());
            let empty = render_world(&mut renderer, &mut blank).await;

            let mut as_map = World::new();
            as_map.spawn_bundle(camera());
            let map = Tilemap::from_rows(tileset, 2, 1, Vec2::ONE, &rows, |c| c.to_digit(10));
            as_map.spawn_bundle((Transform::new(origin), GlobalTransform::default(), map));
            let map_frame = render_world(&mut renderer, &mut as_map).await;
            let calls = renderer.sprites.as_ref().map(|s| s.draw_calls());

            let mut as_sprites = World::new();
            as_sprites.spawn_bundle(camera());
            for (y, row) in rows.iter().enumerate() {
                for (x, c) in row.chars().enumerate() {
                    let Some(cell) = c.to_digit(10) else { continue };
                    let pos = origin + Vec3::new(x as f32, -(y as f32), 0.0);
                    let sprite = Sprite::world(tileset, Vec2::ONE).with_anchor(Vec2::ZERO).with_frame(cell, 2, 1);
                    as_sprites.spawn_bundle((Transform::new(pos), GlobalTransform::default(), sprite));
                }
            }
            let sprite_frame = render_world(&mut renderer, &mut as_sprites).await;

            let (drawn, total) = changed_pixels(&empty, &map_frame, 128);
            assert!(drawn >= 500, "the tilemap changed {drawn}/{total} pixels — it is not drawn");
            let (diff, _) = changed_pixels(&sprite_frame, &map_frame, 128);
            assert_eq!(diff, 0, "the tilemap differs from its tiles drawn as sprites in {diff} pixels");
            assert_eq!(calls, Some((1, 0)), "a map of one tileset must be one world draw call");

            // A map far bigger than the view queues what is on screen, not all of it — and draws
            // the same picture as a map holding only the tiles around the view.
            let mut huge = World::new();
            huge.spawn_bundle(camera());
            let mut big = Tilemap::new(tileset, 2, 1, 2000, 2000, Vec2::ONE);
            big.fill(0, 0, 2000, 2000, Some(1));
            huge.spawn_bundle((Transform::new(Vec3::new(-1000.0, 1000.0, 0.0)), GlobalTransform::default(), big));
            let huge_frame = render_world(&mut renderer, &mut huge).await;
            let queued = renderer.sprites.as_ref().map(|s| s.queued().0).unwrap_or(0);
            eprintln!("2000 × 2000 map: {queued} tiles queued");
            assert!(queued > 0 && queued < 2_000, "{queued} of 4 000 000 tiles queued — the map is not culled");

            let mut local = World::new();
            local.spawn_bundle(camera());
            let mut patch = Tilemap::new(tileset, 2, 1, 40, 40, Vec2::ONE);
            patch.fill(0, 0, 40, 40, Some(1));
            local.spawn_bundle((Transform::new(Vec3::new(-20.0, 20.0, 0.0)), GlobalTransform::default(), patch));
            let local_frame = render_world(&mut renderer, &mut local).await;
            let (diff, _) = changed_pixels(&local_frame, &huge_frame, 128);
            assert_eq!(diff, 0, "culling dropped {diff} pixels of tiles that were on screen");
        });
    }

    /// A HUD colour reaches the screen as authored — not exposed, bloomed, tone-mapped or
    /// vignetted with the scene.
    ///
    /// A pure-green opaque screen sprite must read back as pure green. Drawn into the HDR target
    /// (where screen sprites and text went until the post-tonemap overlay existed) it did not.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn a_screen_sprite_keeps_its_authored_colour() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        pollster::block_on(async {
            let frame = render_sprite_scene(&[SpriteCase::Pure]).await;
            // The middle of the 24 px sprite at (8, 8).
            let i = (20 * 128 + 20) * 4;
            let px = [frame[i], frame[i + 1], frame[i + 2]];
            eprintln!("pure green HUD pixel read back as {px:?}");
            // Measured (0, 219, 0) through the HDR pass, (0, 255, 0) through the overlay.
            assert!(
                px[1] >= 250 && px[0] <= 5 && px[2] <= 5,
                "a (0, 1, 0) screen sprite read back as {px:?} — it is going through the scene's \
                 tone mapping instead of over it",
            );
        });
    }

    /// A HUD removed from the world leaves the screen on the next frame — sprite and text both.
    ///
    /// The overlay is drawn from what each renderer last *uploaded*, and `record_text` returns
    /// early on a frame with no `Text` at all. Without `begin_frame` forgetting the upload counts
    /// on that path, the last label would be repainted forever.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn a_removed_hud_does_not_stay_on_screen() {
        use gizmo_renderer::components::{Sprite, Text};
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        pollster::block_on(async {
            let mut renderer = crate::test_gpu::headless_renderer(128, 128).await;
            renderer.taa = None;
            let mut world = World::new();
            world.spawn_bundle(CameraBundle {
                position: Vec3::new(0.0, 0.0, 6.0),
                yaw: -std::f32::consts::FRAC_PI_2,
                primary: true,
                ..Default::default()
            });
            let white = renderer
                .load_sprite_rgba(&[255; 4], 1, 1, gizmo_renderer::sprite::SpriteFilter::Nearest)
                .unwrap();
            let font = renderer
                .load_font(gizmo_renderer::text::synthetic::synthetic_face())
                .unwrap();

            let before = render_world(&mut renderer, &mut world).await;
            let sprite = world.spawn_bundle((Sprite::screen(
                white,
                gizmo_math::Vec2::splat(24.0),
                gizmo_math::Vec2::splat(8.0),
            )
            .with_color(Vec4::new(0.0, 1.0, 0.0, 1.0)),));
            let label = world.spawn_bundle((Text::screen("AAA", font, 24.0, gizmo_math::Vec2::new(8.0, 72.0)),));
            let with_hud = render_world(&mut renderer, &mut world).await;
            let shown = quadrant_changes(&before, &with_hud, 0, 0) + quadrant_changes(&before, &with_hud, 0, 64);
            assert!(shown > 300, "the HUD changed only {shown} pixels — it is not drawn, so its removal proves nothing");

            world.remove_component::<Sprite>(sprite);
            world.remove_component::<Text>(label);
            let after = render_world(&mut renderer, &mut world).await;
            let left = quadrant_changes(&before, &after, 0, 0) + quadrant_changes(&before, &after, 0, 64);
            assert_eq!(
                left, 0,
                "{left} pixels of a removed HUD are still on screen (measured 468 with \
                 `record_text`'s early return not calling `begin_frame`)",
            );
        });
    }

    /// On a target that is not sRGB the overlay encodes its colour itself.
    ///
    /// Linear 0.5 grey is sRGB 188. Written raw into an `Rgba8Unorm` target it reads 128 — a HUD a
    /// gamma step too dark — so both targets must read the same ~188.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn the_overlay_encodes_its_colour_on_a_linear_target() {
        use gizmo_renderer::components::Sprite;
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        pollster::block_on(async {
            let mut read = Vec::new();
            for format in [wgpu::TextureFormat::Rgba8UnormSrgb, wgpu::TextureFormat::Rgba8Unorm] {
                let mut renderer = crate::test_gpu::headless_renderer_as(128, 128, Some(format)).await;
                renderer.taa = None;
                let mut world = World::new();
                world.spawn_bundle(CameraBundle { primary: true, ..Default::default() });
                let white = renderer
                    .load_sprite_rgba(&[255; 4], 1, 1, gizmo_renderer::sprite::SpriteFilter::Nearest)
                    .unwrap();
                world.spawn_bundle((Sprite::screen(white, gizmo_math::Vec2::splat(24.0), gizmo_math::Vec2::splat(8.0))
                    .with_color(Vec4::new(0.5, 0.5, 0.5, 1.0)),));
                let frame = render_world(&mut renderer, &mut world).await;
                let i = (20 * 128 + 20) * 4;
                read.push((format, [frame[i], frame[i + 1], frame[i + 2]]));
            }
            eprintln!("linear 0.5 grey read back as {read:?}");
            for (format, px) in &read {
                assert!(
                    px.iter().all(|&c| c.abs_diff(188) <= 2),
                    "{format:?}: linear 0.5 read back as {px:?}, expected sRGB 188 — a linear target \
                     needs the `fs_main_encoded` entry (measured 128 with it bypassed)",
                );
            }
        });
    }

    /// One thing [`render_sprite_scene`] puts in the world.
    #[derive(Clone, Copy)]
    enum SpriteCase {
        /// A 24 px opaque screen sprite at (8, 8).
        Screen { layer: i32, blue: bool },
        /// The same, carrying `IsHidden`.
        HiddenScreen,
        /// The same, pure green `(0, 1, 0, 1)`.
        Pure,
        /// A 1.5-unit opaque world sprite on the camera's axis at depth `z`.
        World { z: f32, blue: bool },
        /// A wall at z = 3, between the camera and the origin.
        Wall,
    }

    /// The text scene's camera and light, a floor, and `cases` in order.
    async fn render_sprite_scene(cases: &[SpriteCase]) -> Vec<u8> {
        use gizmo_renderer::components::Sprite;
        use gizmo_renderer::sprite::SpriteFilter;
        const W: u32 = 128;
        let mut renderer = crate::test_gpu::headless_renderer(W, W).await;
        renderer.taa = None;
        let mut asset_manager = AssetManager::new();
        let mut world = World::new();
        let tex = asset_manager.create_white_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
        );
        world.spawn_bundle(CameraBundle {
            position: Vec3::new(0.0, 0.0, 6.0),
            yaw: -std::f32::consts::FRAC_PI_2,
            pitch: 0.0,
            primary: true,
            ..Default::default()
        });
        world.spawn_bundle(DirectionalLightBundle::default());

        let white = renderer
            .load_sprite_rgba(&[255; 4], 1, 1, SpriteFilter::Nearest)
            .expect("a 1×1 image must load");
        let tint = |blue: bool| if blue { Vec4::new(0.1, 0.2, 1.0, 1.0) } else { Vec4::new(1.0, 0.2, 0.1, 1.0) };
        for case in cases {
            let e = world.spawn();
            match *case {
                SpriteCase::Screen { layer, blue } => world.add_component(
                    e,
                    Sprite::screen(white, gizmo_math::Vec2::splat(24.0), gizmo_math::Vec2::splat(8.0))
                        .with_layer(layer)
                        .with_color(tint(blue)),
                ),
                SpriteCase::Pure => world.add_component(
                    e,
                    Sprite::screen(white, gizmo_math::Vec2::splat(24.0), gizmo_math::Vec2::splat(8.0))
                        .with_color(Vec4::new(0.0, 1.0, 0.0, 1.0)),
                ),
                SpriteCase::HiddenScreen => {
                    world.add_component(
                        e,
                        Sprite::screen(white, gizmo_math::Vec2::splat(24.0), gizmo_math::Vec2::splat(8.0)),
                    );
                    world.add_component(e, gizmo_core::component::IsHidden);
                }
                SpriteCase::World { z, blue } => {
                    world.add_component(e, Transform::new(Vec3::new(0.0, 0.0, z)));
                    world.add_component(e, GlobalTransform::default());
                    world.add_component(e, Sprite::world(white, gizmo_math::Vec2::splat(1.5)).with_color(tint(blue)));
                }
                SpriteCase::Wall => {
                    world.add_component(
                        e,
                        Transform::new(Vec3::new(0.0, 0.0, 3.0)).with_scale(Vec3::new(3.0, 3.0, 0.2)),
                    );
                    world.add_component(e, GlobalTransform::default());
                    world.add_component(e, AssetManager::create_cube(&renderer.device));
                    world.add_component(
                        e,
                        Material::new(tex.clone()).with_pbr(Vec4::new(0.8, 0.3, 0.3, 1.0), 0.7, 0.0),
                    );
                    world.add_component(e, MeshRenderer::new());
                }
            }
        }
        render_world(&mut renderer, &mut world).await
    }

    /// How many pixels of one 64×64 quadrant differ between two frames.
    fn quadrant_changes(a: &[u8], b: &[u8], x0: u32, y0: u32) -> usize {
        const W: usize = 128;
        let mut changed = 0;
        for y in y0 as usize..y0 as usize + 64 {
            for x in x0 as usize..x0 as usize + 64 {
                let i = (y * W + x) * 4;
                if (0..3).any(|c| a[i + c].abs_diff(b[i + c]) > 8) {
                    changed += 1;
                }
            }
        }
        changed
    }

    /// What [`render_text_scene`] puts in the world besides the floor.
    #[derive(Clone, Copy)]
    enum TextCase {
        /// Nothing — the reference frame.
        None,
        /// Screen-space text anchored near the top-left corner.
        ScreenTopLeft,
        /// A world label at the origin, in the open.
        WorldLabel,
        /// The same label, with a wall between it and the camera.
        WorldLabelBehindWall,
        /// The wall alone, which is the reference the occluded case is compared against.
        WallOnly,
    }

    /// A lit floor, a camera, and whichever text case is asked for.
    async fn render_text_scene(case: TextCase) -> Vec<u8> {
        const W: u32 = 128;
        let mut renderer = crate::test_gpu::headless_renderer(W, W).await;
        renderer.taa = None;
        let mut asset_manager = AssetManager::new();
        let mut world = World::new();
        let tex = asset_manager.create_white_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
        );

        let floor = world.spawn();
        world.add_component(
            floor,
            Transform::new(Vec3::new(0.0, -2.0, 0.0)).with_scale(Vec3::new(12.0, 0.2, 12.0)),
        );
        world.add_component(floor, GlobalTransform::default());
        world.add_component(floor, AssetManager::create_cube(&renderer.device));
        world.add_component(
            floor,
            Material::new(tex.clone()).with_pbr(Vec4::new(0.2, 0.2, 0.25, 1.0), 0.9, 0.0),
        );
        world.add_component(floor, MeshRenderer::new());

        world.spawn_bundle(CameraBundle {
            position: Vec3::new(0.0, 0.0, 6.0),
            yaw: -std::f32::consts::FRAC_PI_2,
            pitch: 0.0,
            primary: true,
            ..Default::default()
        });
        world.spawn_bundle(DirectionalLightBundle::default());

        let wall = |world: &mut World, renderer: &Renderer| {
            let e = world.spawn();
            world.add_component(
                e,
                Transform::new(Vec3::new(0.0, 0.0, 3.0)).with_scale(Vec3::new(3.0, 3.0, 0.2)),
            );
            world.add_component(e, GlobalTransform::default());
            world.add_component(e, AssetManager::create_cube(&renderer.device));
            world.add_component(
                e,
                Material::new(tex.clone()).with_pbr(Vec4::new(0.8, 0.3, 0.3, 1.0), 0.7, 0.0),
            );
            world.add_component(e, MeshRenderer::new());
        };

        let font = renderer
            .load_font(gizmo_renderer::text::synthetic::synthetic_face())
            .expect("the synthetic face must parse");

        match case {
            TextCase::None => {}
            TextCase::ScreenTopLeft => {
                let e = world.spawn();
                world.add_component(
                    e,
                    gizmo_renderer::components::Text::screen(
                        "AAA",
                        font,
                        24.0,
                        gizmo_math::Vec2::new(8.0, 8.0),
                    ),
                );
            }
            TextCase::WorldLabel | TextCase::WorldLabelBehindWall => {
                if matches!(case, TextCase::WorldLabelBehindWall) {
                    wall(&mut world, &renderer);
                }
                let e = world.spawn();
                world.add_component(e, Transform::new(Vec3::ZERO));
                world.add_component(e, GlobalTransform::default());
                world.add_component(
                    e,
                    gizmo_renderer::components::Text::world("AAA", font, 48.0, 24.0),
                );
            }
            TextCase::WallOnly => wall(&mut world, &renderer),
        }

        render_world(&mut renderer, &mut world).await
    }

    /// `Gizmos::depth_test` selects between two pipelines, and they have to be two.
    ///
    /// They were not. Both were built with `CompareFunction::Always`, so the flag chose between
    /// identical copies and `Gizmos::depth_test`'s own doc — *"whether the lines are occluded by
    /// the scene"* — was false. Nothing caught it because nothing in the workspace sets the flag:
    /// `Gizmos` derives `Default` (false) and `gizmo-studio` writes `false` explicitly, so the
    /// broken half was never selected.
    ///
    /// The assertion is the pair, not either half. A depth-tested line hidden behind a cube proves
    /// nothing on its own — a line that is never drawn passes that too — so the same line has to
    /// appear when the flag is off.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn a_depth_tested_gizmo_is_hidden_by_geometry_and_an_untested_one_is_not() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        pollster::block_on(async {
            let none = render_gizmo_scene(None).await;
            let over = render_gizmo_scene(Some(false)).await;
            let behind = render_gizmo_scene(Some(true)).await;

            let (drawn, total) = changed_pixels(&none, &over, 128);
            assert!(
                drawn >= 60,
                "a gizmo line with the depth test off changed {drawn}/{total} pixels (measured \
                 115 — two one-pixel lines across a 128² frame) — it is not being drawn at all, \
                 so the other half proves nothing",
            );
            let (leaked, _) = changed_pixels(&none, &behind, 128);
            assert_eq!(
                leaked, 0,
                "a gizmo line BEHIND a cube changed {leaked} pixels with `depth_test = true` — \
                 the depth-tested pipeline is not depth-tested. It was built with \
                 `CompareFunction::Always`, identical to the other one, until 2026-08-24 — and \
                 with that restored this reads 115, i.e. the whole line.",
            );
        });
    }

    /// A cube filling the view, and a debug line drawn behind it.
    ///
    /// `None` draws no line at all — the reference. `Some(depth_test)` draws one and sets the flag.
    async fn render_gizmo_scene(depth_test: Option<bool>) -> Vec<u8> {
        const W: u32 = 128;
        let mut renderer = crate::test_gpu::headless_renderer(W, W).await;
        renderer.taa = None;
        let mut asset_manager = AssetManager::new();
        let mut world = World::new();
        let tex = asset_manager.create_white_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
        );

        // A wall at the origin, wide enough to cover the middle of the frame.
        let wall = world.spawn();
        world.add_component(
            wall,
            Transform::new(Vec3::ZERO).with_scale(Vec3::new(4.0, 4.0, 0.2)),
        );
        world.add_component(wall, GlobalTransform::default());
        world.add_component(wall, AssetManager::create_cube(&renderer.device));
        world.add_component(
            wall,
            Material::new(tex).with_pbr(Vec4::new(0.6, 0.6, 0.65, 1.0), 0.8, 0.0),
        );
        world.add_component(wall, MeshRenderer::new());

        world.spawn_bundle(CameraBundle {
            position: Vec3::new(0.0, 0.0, 8.0),
            yaw: -std::f32::consts::FRAC_PI_2,
            pitch: 0.0,
            primary: true,
            ..Default::default()
        });
        world.spawn_bundle(DirectionalLightBundle::default());

        if let Some(depth_test) = depth_test {
            let mut gizmos = crate::renderer::Gizmos { depth_test, ..Default::default() };
            // Behind the wall (the camera is at +8 looking down −Z), and across the middle of it,
            // so "hidden" and "drawn" differ in the same pixels.
            gizmos.draw_line(
                Vec3::new(-3.0, 0.0, -4.0),
                Vec3::new(3.0, 0.0, -4.0),
                [1.0, 0.0, 0.0, 1.0],
            );
            gizmos.draw_line(
                Vec3::new(0.0, -3.0, -4.0),
                Vec3::new(0.0, 3.0, -4.0),
                [1.0, 0.0, 0.0, 1.0],
            );
            world.insert_resource(gizmos);
        }

        render_world(&mut renderer, &mut world).await
    }

    /// A laid-out UI box paints, and a label on it follows the box rather than its own position.
    ///
    /// `gizmo-ui` computed boxes and drew nothing from the day it was written: `BackgroundColor`
    /// was attached by its own bundles and read by no crate in the workspace, and there was no text
    /// to put in a button. Both halves are checked here, and the second is the one with a control
    /// worth having — the `Text` is authored at (100, 100), far outside the box, so a bridge that
    /// did nothing would draw it there and the "nothing outside the box" assertion would fail.
    #[test]
    #[ignore = "requires a GPU adapter"]
    #[cfg(feature = "ui")]
    fn a_ui_box_paints_and_its_label_follows_it() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        // The box, in window pixels on a 128² target.
        const BOX_POS: [u32; 2] = [10, 10];
        const BOX_SIZE: [u32; 2] = [60, 30];

        pollster::block_on(async {
            let blank = render_ui_scene(false, false).await;
            let boxed = render_ui_scene(true, false).await;
            let labelled = render_ui_scene(true, true).await;

            // 1. The background paints, and it paints the box's own rectangle. A solid colour over
            //    60×30 is 1 800 pixels; the count is asserted loosely and the *placement* exactly,
            //    because "some pixels went red" is satisfied by a quad anywhere.
            let (painted, _) = changed_pixels(&blank, &boxed, 128);
            assert!(
                painted >= 1500,
                "a `BackgroundColor` on a laid-out node changed {painted} pixels (measured 1 978 — \
                 60×30 is 1 800 and the rest is bloom) — it is not being drawn",
            );
            // The halo, not the quad. UI is drawn into the HDR target before tone mapping, so a
            // bright box blooms: measured 178 pixels outside a 1 800-pixel rectangle, a one-to-two
            // pixel rim. A quad drawn at the wrong place would put ~1 800 out here, so the bound is
            // loose enough for the bloom and nowhere near loose enough for a misplacement.
            let halo = changed_outside_box(&blank, &boxed, BOX_POS, BOX_SIZE);
            assert!(
                halo <= 400,
                "the background changed {halo} pixels outside its own node rectangle (measured \
                 178, which is the bloom rim) — a quad that far out is drawn somewhere else",
            );

            // 2. The label lands inside the box, not at the position the component carries.
            let inside = changed_inside_box(&boxed, &labelled, BOX_POS, BOX_SIZE);
            assert!(
                inside >= 60,
                "the label changed {inside} pixels inside its box (measured 221) — it is not being \
                 drawn there",
            );
            assert_eq!(
                changed_outside_box(&boxed, &labelled, BOX_POS, BOX_SIZE),
                0,
                "the label drew outside its node — it is being placed at the position the `Text` \
                 carries (100, 100) rather than at the box, i.e. the UI bridge is not running",
            );
        });
    }

    /// Pixels that differ between two frames *inside* a window-pixel rectangle.
    #[cfg(feature = "ui")]
    fn changed_inside_box(a: &[u8], b: &[u8], pos: [u32; 2], size: [u32; 2]) -> usize {
        box_changes(a, b, pos, size, true)
    }

    /// …and outside it.
    #[cfg(feature = "ui")]
    fn changed_outside_box(a: &[u8], b: &[u8], pos: [u32; 2], size: [u32; 2]) -> usize {
        box_changes(a, b, pos, size, false)
    }

    #[cfg(feature = "ui")]
    fn box_changes(a: &[u8], b: &[u8], pos: [u32; 2], size: [u32; 2], inside: bool) -> usize {
        const W: usize = 128;
        let mut changed = 0;
        for y in 0..W {
            for x in 0..W {
                let in_box = x >= pos[0] as usize
                    && x < (pos[0] + size[0]) as usize
                    && y >= pos[1] as usize
                    && y < (pos[1] + size[1]) as usize;
                if in_box != inside {
                    continue;
                }
                let i = (y * W + x) * 4;
                if (0..3).any(|c| a[i + c].abs_diff(b[i + c]) > 8) {
                    changed += 1;
                }
            }
        }
        changed
    }

    /// A dark floor, and optionally a laid-out UI box with a background and a label.
    ///
    /// The `Node` is written directly rather than through `gizmo-ui`'s layout system: what is being
    /// tested is the *bridge*, and driving taffy here would make a layout regression look like a
    /// text one. `gizmo-ui`'s own 27 tests cover what fills a `Node`.
    #[cfg(feature = "ui")]
    async fn render_ui_scene(ui_box: bool, label: bool) -> Vec<u8> {
        const W: u32 = 128;
        let mut renderer = crate::test_gpu::headless_renderer(W, W).await;
        renderer.taa = None;
        let mut asset_manager = AssetManager::new();
        let mut world = World::new();
        let tex = asset_manager.create_white_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
        );

        let floor = world.spawn();
        world.add_component(
            floor,
            Transform::new(Vec3::new(0.0, -2.0, 0.0)).with_scale(Vec3::new(12.0, 0.2, 12.0)),
        );
        world.add_component(floor, GlobalTransform::default());
        world.add_component(floor, AssetManager::create_cube(&renderer.device));
        world.add_component(
            floor,
            Material::new(tex).with_pbr(Vec4::new(0.2, 0.2, 0.25, 1.0), 0.9, 0.0),
        );
        world.add_component(floor, MeshRenderer::new());

        world.spawn_bundle(CameraBundle {
            position: Vec3::new(0.0, 0.0, 6.0),
            yaw: -std::f32::consts::FRAC_PI_2,
            pitch: 0.0,
            primary: true,
            ..Default::default()
        });
        world.spawn_bundle(DirectionalLightBundle::default());

        let font = renderer
            .load_font(gizmo_renderer::text::synthetic::synthetic_face())
            .expect("the synthetic face must parse");

        if ui_box {
            let e = world.spawn();
            world.add_component(
                e,
                crate::ui::Node {
                    position: gizmo_math::Vec2::new(10.0, 10.0),
                    size: gizmo_math::Vec2::new(60.0, 30.0),
                },
            );
            world.add_component(
                e,
                crate::ui::BackgroundColor(Vec4::new(0.9, 0.2, 0.2, 1.0)),
            );
            if label {
                // Authored FAR from the box on purpose: with the bridge working this position is
                // ignored, and without it the text lands here and the test says so.
                world.add_component(
                    e,
                    gizmo_renderer::components::Text::screen(
                        "AAA",
                        font,
                        16.0,
                        gizmo_math::Vec2::new(100.0, 100.0),
                    )
                    .with_color(Vec4::new(0.05, 0.05, 0.05, 1.0)),
                );
            }
        }

        render_world(&mut renderer, &mut world).await
    }

    /// What hiding an object actually does — and what the docs said it did.
    ///
    /// `docs/CAPABILITY_GAPS.md` listed a missing `Visibility` component whose consequence was
    /// *"hiding means `ShadowCasting::Only`, which still casts a shadow"*, and `infinite_grid`
    /// said the same. Both were already false when written: `IsHidden` reached the game path on
    /// 2026-08-19 and it drops the entity before it becomes a draw item, so no shadow either.
    ///
    /// Three frames pin the difference the prose had backwards. `IsHidden` removes the object AND
    /// its shadow; `ShadowCasting::Only` removes the object and keeps the shadow. Asserting both
    /// is the point — either one alone is satisfied by a renderer that simply drew nothing.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn hiding_an_object_removes_its_shadow_and_shadow_casting_only_keeps_it() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available())
            || pollster::block_on(Renderer::headless_adapter_is_software())
        {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        pollster::block_on(async {
            let absent = render_hidden_scene(HiddenCase::NoCube).await;
            let shown = render_hidden_scene(HiddenCase::Shown).await;
            let hidden = render_hidden_scene(HiddenCase::IsHidden).await;
            let shadow_only = render_hidden_scene(HiddenCase::ShadowCastingOnly).await;

            let (drawn, total) = changed_pixels(&absent, &shown, 128);
            assert!(
                drawn >= 500,
                "the cube changed {drawn}/{total} pixels against an empty scene — the fixture is \
                 not drawing it, so nothing below means anything",
            );

            // `IsHidden` is a full hide: the frame is the one with no cube in the world at all.
            let (leftover, _) = changed_pixels(&absent, &hidden, 128);
            assert_eq!(
                leftover, 0,
                "an `IsHidden` cube left {leftover} pixels behind — it is not a full hide, which \
                 is what the capability list used to claim",
            );

            // `ShadowCasting::Only` is not: the cube goes, the shadow stays.
            let (residue, _) = changed_pixels(&absent, &shadow_only, 128);
            assert!(
                residue >= 100,
                "`ShadowCasting::Only` left {residue} pixels — it is supposed to keep casting, so \
                 either the shadow is gone or the fixture has no shadow to lose",
            );
            let (still_drawn, _) = changed_pixels(&shown, &shadow_only, 128);
            assert!(
                still_drawn >= 300,
                "`ShadowCasting::Only` changed only {still_drawn} pixels against the visible cube \
                 — the cube itself is still being drawn",
            );

            // And hiding is INHERITED: a hidden parent takes its children with it. It did not
            // until 2026-08-24 — both loops asked `contains(entity)` per entity, so 1 886 of a
            // pair's 2 946 pixels survived the parent being hidden.
            let pair = render_hidden_scene(HiddenCase::ParentWithChild).await;
            let hidden_parent = render_hidden_scene(HiddenCase::HiddenParentWithChild).await;
            let (pair_drawn, _) = changed_pixels(&absent, &pair, 128);
            assert!(
                pair_drawn >= 1000,
                "the parent/child pair changed {pair_drawn} pixels (measured 2 946) — the fixture \
                 is not drawing both, so the next assertion proves nothing",
            );
            let (child_left, _) = changed_pixels(&absent, &hidden_parent, 128);
            assert_eq!(
                child_left, 0,
                "hiding the parent left {child_left} pixels — its child is still drawn. Measured \
                 1 886 before `collect_hidden` walked the hierarchy.",
            );
        });
    }

    /// Which cube [`render_hidden_scene`] puts in front of the light.
    #[derive(Clone, Copy, PartialEq)]
    enum HiddenCase {
        /// No cube at all — the reference every other case is measured against.
        NoCube,
        Shown,
        IsHidden,
        ShadowCastingOnly,
        /// A parent cube marked hidden, with a visible child beside it.
        HiddenParentWithChild,
        /// The same pair with nothing hidden.
        ParentWithChild,
    }

    /// A lit floor and a cube above it, so the cube has a shadow to keep or lose.
    async fn render_hidden_scene(case: HiddenCase) -> Vec<u8> {
        const W: u32 = 128;
        let mut renderer = crate::test_gpu::headless_renderer(W, W).await;
        renderer.taa = None;
        let mut asset_manager = AssetManager::new();
        let mut world = World::new();
        let tex = asset_manager.create_white_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
        );

        let floor = world.spawn();
        world.add_component(
            floor,
            Transform::new(Vec3::new(0.0, -2.0, 0.0)).with_scale(Vec3::new(14.0, 0.2, 14.0)),
        );
        world.add_component(floor, GlobalTransform::default());
        world.add_component(floor, AssetManager::create_cube(&renderer.device));
        world.add_component(
            floor,
            Material::new(tex.clone()).with_pbr(Vec4::new(0.85, 0.85, 0.9, 1.0), 0.9, 0.0),
        );
        world.add_component(floor, MeshRenderer::new());

        let cube = |world: &mut World, x: f32| -> gizmo_core::entity::Entity {
            let e = world.spawn();
            world.add_component(e, Transform::new(Vec3::new(x, 0.0, 0.0)));
            world.add_component(e, GlobalTransform::default());
            world.add_component(e, AssetManager::create_cube(&renderer.device));
            world.add_component(
                e,
                Material::new(tex.clone()).with_pbr(Vec4::new(0.2, 0.5, 0.9, 1.0), 0.4, 0.0),
            );
            world.add_component(e, MeshRenderer::new());
            e
        };

        match case {
            HiddenCase::NoCube => {}
            HiddenCase::Shown => {
                cube(&mut world, 0.0);
            }
            HiddenCase::IsHidden => {
                let e = cube(&mut world, 0.0);
                world.add_component(e, crate::core::component::IsHidden);
            }
            HiddenCase::ShadowCastingOnly => {
                let e = cube(&mut world, 0.0);
                world.add_component(
                    e,
                    MeshRenderer::new()
                        .with_shadows(gizmo_renderer::components::ShadowCasting::Only),
                );
            }
            HiddenCase::ParentWithChild | HiddenCase::HiddenParentWithChild => {
                let parent = cube(&mut world, -1.6);
                let child = cube(&mut world, 1.6);
                use crate::core::hierarchy::HierarchyExt;
                world.add_child(parent, child);
                if case == HiddenCase::HiddenParentWithChild {
                    world.add_component(parent, crate::core::component::IsHidden);
                }
            }
        }

        // The sun is overhead and to one side, so the cube's shadow lands on the floor beside it
        // rather than under it where the cube would hide its own evidence.
        world.spawn_bundle(DirectionalLightBundle {
            rotation: gizmo_math::Quat::from_rotation_x(-0.9)
                * gizmo_math::Quat::from_rotation_y(0.6),
            intensity: 3.0,
            ..Default::default()
        });
        world.spawn_bundle(CameraBundle {
            position: Vec3::new(0.0, 2.5, 7.0),
            yaw: -std::f32::consts::FRAC_PI_2,
            pitch: -0.3,
            primary: true,
            ..Default::default()
        });

        render_world(&mut renderer, &mut world).await
    }
}
