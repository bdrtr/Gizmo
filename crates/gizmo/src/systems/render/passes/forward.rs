use super::super::*;

// Forward pipeline bind-group indeksleri hedefe göre değişir: tarayıcı WebGPU
// maxBindGroups=4 olduğundan web şemasında shadow grubu atılır ve skeleton /
// instance bir kayar (bkz. gizmo_renderer::pipeline shaders.rs `load_shader_web`).
//   Native: 0=global 1=texture 2=shadow 3=skeleton 4=instance
//   WASM:   0=global 1=texture 2=skeleton 3=instance
#[cfg(not(gizmo_forward_only))]
const BG_SKELETON: u32 = 3;
#[cfg(not(gizmo_forward_only))]
const BG_INSTANCE: u32 = 4;
#[cfg(gizmo_forward_only)]
const BG_SKELETON: u32 = 2;
#[cfg(gizmo_forward_only)]
const BG_INSTANCE: u32 = 3;

pub fn record_forward_and_fluid(
    encoder: &mut wgpu::CommandEncoder,
    // `mut` for the GPU timer's marks between the parts (`gpu.forward`, `gpu.surface_effects`).
    renderer: &mut Renderer,
    world: &World,
    draw_items: &[DrawItem],
    uploaded_instances: u32,
    // Whether this view draws the frame's GPU simulations — the particles and the fluid, as many as
    // the frame's step simulated. A reflection probe's face does not: it is one still frame, captured
    // before the step.
    simulations: bool,
    // Inverse of the (unjittered) camera view-projection — the volumetric smoke raymarch
    // reconstructs rays with this instead of the buggy WGSL inverse_mat4.
    inv_view_proj: [[f32; 4]; 4],
    // `Camera::clear_color`, or `DEFAULT_BACKGROUND`. Only the forward-only pipeline clears here; with
    // a G-buffer the lighting pass already wrote the background, and this pass loads it.
    clear_color: Option<[f32; 3]>,
    // Whether anything reads the G-buffer's velocity this frame (TAA, motion blur, SSR). Without a
    // reader the forward-drawn surfaces are not drawn into it a second time.
    velocity_read: bool,
    // Where the opaque baked-lit surfaces are drawn this view: not here when they went into the
    // G-buffer (`DrawItem::in_gbuffer`) or are drawn with their reflection inputs (`drawn_reflective`).
    baked: BakedRoute,
    // The view's camera and the frame's clock (the scene block's): what the rain's streaks are measured by.
    (camera, clock): (gizmo_math::Vec3, f32),
) {
    // ── Forward pass (unlit / skybox / GPU subsystems; PBR skipped if deferred) ──
    {
        let hdr_load = if renderer.deferred.is_some() {
            wgpu::LoadOp::Load
        } else {
            let [r, g, b] = clear_color.unwrap_or(crate::renderer::frame_uniforms::DEFAULT_BACKGROUND);
            wgpu::LoadOp::Clear(wgpu::Color { r: r as f64, g: g as f64, b: b as f64, a: 1.0 })
        };
        let depth_load = if renderer.deferred.is_some() {
            wgpu::LoadOp::Load
        } else {
            wgpu::LoadOp::Clear(1.0)
        };

        // The opaque world first, then what blends over it — glass, transparent surfaces — in a pass of
        // its own. Between the two, SSR and SSGI are added to the opaque frame, so what blends shows
        // them as much as it lets through; and glass reads the frame behind it, which a target cannot
        // be while it is drawn to, so with glass in view the frame is copied there too. `draw_items` is
        // sorted by layer.
        let has_glass = renderer.behind.is_some()
            && draw_items.iter().any(|item| item.is_transmissive && item.camera_count > 0);
        // Split only when something stands between the two: a split costs a tiled GPU — a phone's —
        // a store and a load of the whole frame.
        let volumetric = super::screen_space::volumetric_on(renderer);
        let world_end = if has_glass || volumetric || super::screen_space::surface_effects_on(renderer) {
            draw_items.partition_point(|item| item.layer < super::super::batching::DrawLayer::Transmissive)
        } else {
            draw_items.len()
        };
        record_baked_reflective(encoder, renderer, &draw_items[..world_end], uploaded_instances, baked);
        let mut render_pass = begin_forward_pass(encoder, renderer, hdr_load, depth_load);
        draw_environment_sky(&mut render_pass, renderer);
        draw_forward_items(&mut render_pass, renderer, &draw_items[..world_end], uploaded_instances, baked);
        #[cfg(gizmo_forward_only)]
        redraw_under_decals(&mut render_pass, renderer, &draw_items[..world_end], uploaded_instances, baked, inv_view_proj);

        // Draw GPU Physics Spheres!
        if let Some(physics) = &renderer.gpu_physics {
            physics.render_pass(&mut render_pass, renderer.scene.view_bind_group());
            physics.debug_render_pass(&mut render_pass, renderer.scene.view_bind_group());
        }

        // No fluid here. `GpuFluidSystem::render_pass` was called on this line and its body was
        // empty — "Fallback for compatibility, not used directly by SSFR loop" — so the forward
        // pass has never drawn a drop. The fluid is drawn by `record_fluid_surface` below, which
        // reconstructs its surface in screen space. Deleted with the method.

        // (GPU Particles artık AYRI bir pass'te — aşağıda — soft-particle derinlik örneklemesi için.)
        drop(render_pass);
        renderer.gpu_mark(encoder, "gpu.forward");
        // Marked inside, stage by stage (`gpu.seen`, `gpu.ssr_prepare`, `gpu.ssr_march`, `gpu.ssr_blur`,
        // `gpu.ssr_apply`); `gpu.surface_effects` is what is left, the SSGI.
        super::screen_space::record_surface_effects(encoder, renderer);
        renderer.gpu_mark(encoder, "gpu.surface_effects");
        // The parts of what blends, each to a GPU mark of its own: `gpu.blended`, which the frame's
        // draw marks after this, held them all — the air's march, the glass, the motion, the particles,
        // the rain — and a drive's 7 ms of it could not be told apart.
        if volumetric {
            // The veil's pass only while something blends, or to clear what an earlier frame left in it.
            let blended = &draw_items[world_end..];
            let blends = blended.iter().any(|item| veils(item, uploaded_instances)) || particles_veil(renderer, simulations);
            if blends || renderer.volumetric.as_ref().is_some_and(|v| v.veil_stale()) {
                record_veil_depth(encoder, renderer, blended, uploaded_instances, simulations);
                if let Some(vol) = renderer.volumetric.as_mut() {
                    vol.veil_drawn(blends);
                }
            }
            renderer.gpu_mark(encoder, "gpu.veil");
            super::screen_space::record_volumetric_march(encoder, renderer, blends);
            renderer.gpu_mark(encoder, "gpu.haze");
        }
        if let Some(behind) = renderer.behind.as_ref().filter(|_| has_glass) {
            behind.capture(encoder, &renderer.post.hdr_texture, renderer.depth_texture_view.texture());
        }
        if world_end < draw_items.len() {
            let mut render_pass = begin_forward_pass(encoder, renderer, wgpu::LoadOp::Load, wgpu::LoadOp::Load);
            draw_forward_items(&mut render_pass, renderer, &draw_items[world_end..], uploaded_instances, baked);
        }
        renderer.gpu_mark(encoder, "gpu.transparent");
    }

    // How what was drawn forward and opaque moved, into the G-buffer's velocity, for TAA and motion blur
    // — when either, or SSR, reads it. It is every forward-drawn surface drawn again, but for the baked-lit
    // ones the reflective pass drew (`BakedRoute::Reflective`), which wrote theirs as they were shaded.
    if velocity_read {
        record_forward_motion(encoder, renderer, draw_items, uploaded_instances, baked);
        renderer.gpu_mark(encoder, "gpu.motion");
    }

    // ── GPU particles (soft: a pass of their own, see `record_gpu_particles`) and the fluid's
    //    surface — both as many as the frame's step simulated. ──
    if simulations {
        crate::systems::render::record_gpu_particles(encoder, renderer);
        crate::systems::render::record_fluid_surface(encoder, renderer);
        renderer.gpu_mark(encoder, "gpu.particles");
        // Rain: a probe's face is one still frame, and rain caught in it would hang in every reflection.
        renderer.record_rain(encoder, camera, clock);
        renderer.gpu_mark(encoder, "gpu.rain");
    }

    // ── Volumetrik duman (T6): sahnenin üstüne HDR'ye raymarch (post-process ÖNCESİ). The grid was
    //    stepped once for the frame, ahead of every view (`step_gpu_simulations`); a view only draws. ──
    if let Some(smoke) = &renderer.smoke {
        let (time, dt) = world
            .get_resource::<gizmo_core::time::Time>()
            .map(|t| (t.elapsed() as f32, t.dt()))
            .unwrap_or((0.0, 1.0 / 60.0));
        smoke.raymarch(
            encoder,
            &renderer.device,
            &renderer.queue,
            renderer.scene.view_bind_group(),
            &renderer.post.hdr_texture_view,
            &renderer.depth_texture_view,
            time,
            dt,
            inv_view_proj,
        );
    }
}

/// How each opaque surface drawn forward moved — unlit; a baked-lit one is in the G-buffer where there is
/// one (`DrawItem::in_gbuffer`) and writes its own, and so does one the reflective pass draws
/// (`record_baked_reflective`, `fs_reflective`'s velocity), where the city of `nfsu2-gizmo` drawn again
/// for this pass alone took 0.9 ms of a wet drive (1080p, AMD 780M, 2026-11-21) — into the G-buffer's velocity, where
/// the forward pass left the frame's depth (`fs_motion` in `gbuffer.wgsl`). They write depth and no
/// G-buffer, so their pixels kept the motion of what stood behind them: TAA reprojected a moving unlit
/// object as if it stood still, and motion blur left it sharp. Not a sky, a backdrop, the grid, water
/// (its waves move its vertices, which this pass would not), a mirror, or a game's own pipeline (its own
/// vertex stage); nothing that blends, whose pixels are what stands behind as much as they are its own.
fn record_forward_motion(encoder: &mut wgpu::CommandEncoder, renderer: &Renderer, draw_items: &[DrawItem], uploaded_instances: u32, baked: BakedRoute) {
    let Some(def) = renderer.deferred.as_ref() else {
        return;
    };
    let moving = |item: &&DrawItem| {
        item.visible_in_camera
            && !item.in_gbuffer(baked)
            && !item.is_transparent
            && !item.is_skybox
            && !item.is_backdrop
            && !item.is_grid
            && !item.is_water
            && item.mirror.is_none()
            && item.custom.is_none()
            && !item.drawn_reflective(baked)
            && !item.camera_instance_range(uploaded_instances).is_empty()
    };
    if !draw_items.iter().any(|item| moving(&item)) {
        return;
    }
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Forward Motion"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &def.velocity_view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: &renderer.depth_texture_view,
            depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_bind_group(0, renderer.scene.view_bind_group(), &[]);
    pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
    pass.set_bind_group(BG_INSTANCE, renderer.scene.view_instance_bind_group(), &[]);
    pass.set_bind_group(5, &def.motion.bind_group, &[]);
    let mut plan = super::plan::PassPlan::default();
    for item in draw_items.iter().filter(moving) {
        plan.set_pipeline(&def.motion_pipelines[usize::from(item.is_double_sided)]);
        let skeleton = item.skeleton_bind_group.as_ref().unwrap_or(&renderer.scene.dummy_skeleton_bind_group);
        plan.set_bind_group(3, skeleton.as_ref());
        plan.set_bind_group(1, &item.bind_group);
        item.plan_draw(&mut plan, item.camera_instance_range(uploaded_instances));
    }
    plan.replay(&mut pass, &renderer.device, renderer.multi_draw);
}

/// The depth of the nearest surface that blends, depth only, into the volumetric march's
/// `veil_depth_view`: where it splits the air between what is added ahead of what blends and what is
/// added over it. `blended` is what is drawn after the opaque world. An additive draw is left out: it
/// only adds, so the air behind it is no less seen than the air in front, and putting it behind would
/// dim it by whatever pane stands further back. Drawn through the shadow pipelines from the camera
/// (`VolumetricState::veil_pass`) — both faces, as a blended draw has no culling, and cutting what a
/// cut-out or a game's surface cuts — which every path has: the G-buffer's depth prepass it was drawn
/// through until 2026-11-23 exists only on the desktop. So a water surface splits the air where its
/// mesh lies, not where its waves are. With `simulations`, the GPU particles too, where they show at
/// all: a smoke shows the air behind it as much as it lets it through.
fn record_veil_depth(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &Renderer,
    blended: &[DrawItem],
    uploaded_instances: u32,
    simulations: bool,
) {
    let Some(vol) = renderer.volumetric.as_ref() else {
        return;
    };
    // The particles' soft-fade group reads the frame's depth: built before the pass, which borrows it.
    let particles = renderer
        .gpu_particles
        .as_ref()
        .filter(|_| particles_veil(renderer, simulations))
        .map(|p| (p, p.create_depth_bind_group(&renderer.device, &renderer.depth_texture_view, renderer.scene.view_cluster_buffers(), &renderer.scene.cookies)));
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Volumetric Veil Depth"),
        color_attachments: &[],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: &vol.veil_depth_view,
            depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    // The shadow pipelines' groups: the pass's matrix, the material (read by a cut), the skeleton and
    // the instances.
    pass.set_bind_group(0, &vol.veil_pass, &[]);
    pass.set_bind_group(3, renderer.scene.view_instance_bind_group(), &[]);
    let mut plan = super::plan::PassPlan::default();
    let mut solid = None;
    for item in blended {
        if !veils(item, uploaded_instances) {
            continue;
        }
        let instances = item.camera_instance_range(uploaded_instances);
        let Some(pipeline) = super::shadow::caster_pipeline(renderer, item, false) else { continue };
        super::shadow::bind_caster(&mut plan, renderer, item, pipeline, &mut solid);
        item.plan_draw(&mut plan, instances);
    }
    plan.replay(&mut pass, &renderer.device, renderer.multi_draw);
    if let Some((particles, depth)) = particles.as_ref() {
        particles.record_veil(&mut pass, renderer.scene.view_bind_group(), depth);
    }
}

/// Whether `item`, drawn after the opaque world, is drawn into the volumetric veil: seen, some of it in
/// view, and not additive (see [`record_veil_depth`]).
fn veils(item: &DrawItem, uploaded_instances: u32) -> bool {
    item.visible_in_camera
        && item.blend_mode != crate::renderer::components::BlendMode::Additive
        && !item.camera_instance_range(uploaded_instances).is_empty()
}

/// Whether the GPU particles are drawn into the volumetric veil: this view draws the simulations, and
/// some are alive.
fn particles_veil(renderer: &Renderer, simulations: bool) -> bool {
    simulations && renderer.gpu_particles.as_ref().is_some_and(|p| p.active_particles > 0)
}

/// The forward-only path's decals: every opaque lit draw a decal's box meets drawn again through the lit
/// shader with `DECALS` (`DecalRedraw`), inside the rectangle on the screen that the decals it meets
/// cover, where the depth is the one its first draw left. What it costs is the decals' pixels: every
/// lit pixel paid for the decals' code while a frame had one until 2026-11-23, about 5 ms on an Adreno
/// 618. Opaque surfaces alone, as the desktop's decals paint the G-buffer's. `inv_view_proj` is the
/// view's (unjittered: this path has no TAA).
#[cfg(gizmo_forward_only)]
fn redraw_under_decals(
    render_pass: &mut wgpu::RenderPass<'_>,
    renderer: &Renderer,
    items: &[DrawItem],
    uploaded_instances: u32,
    baked: BakedRoute,
    inv_view_proj: [[f32; 4]; 4],
) {
    let Some(redraw) = renderer.scene.core.decal_redraw.as_ref() else {
        return;
    };
    let decals = &renderer.scene.forward_decal_spheres;
    if decals.is_empty() || renderer.wire_only {
        return;
    }
    let (width, height) = (renderer.post.hdr_texture.width(), renderer.post.hdr_texture.height());
    let view_proj = gizmo_math::Mat4::from_cols_array_2d(&inv_view_proj).inverse();
    let boxes: Vec<(crate::math::Aabb, [u32; 4])> = decals
        .iter()
        .filter_map(|d| {
            let r = gizmo_math::Vec3::splat(d.radius);
            let b = crate::math::Aabb::new(d.center - r, d.center + r);
            gizmo_renderer::decal::screen_rect(view_proj, b, width, height).map(|rect| (b, rect))
        })
        .collect();
    if boxes.is_empty() {
        return;
    }
    for item in items {
        let receives = item.visible_in_camera
            && !(item.is_skybox || item.is_backdrop || item.is_water || item.is_grid || item.baked_lit || item.unlit)
            && !(item.is_transparent || item.is_transmissive)
            && item.custom.is_none()
            && item.surface.is_none()
            && item.mirror.is_none()
            && !item.drawn_reflective(baked);
        let instances = item.camera_instance_range(uploaded_instances);
        if !receives || instances.is_empty() || item.bounds.is_empty() {
            continue;
        }
        let Some([x0, y0, x1, y1]) = boxes
            .iter()
            .filter(|(b, _)| item.bounds.intersects(*b))
            .map(|(_, rect)| *rect)
            .reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])])
        else {
            continue;
        };
        render_pass.set_scissor_rect(x0, y0, x1 - x0, y1 - y0);
        let mut plan = super::plan::PassPlan::default();
        plan.set_pipeline(redraw.pipeline(item.lobes, item.details, item.is_double_sided));
        plan.set_bind_group(1, &item.bind_group);
        plan.set_bind_group(BG_SKELETON, item.skeleton_bind_group.as_deref().unwrap_or(&*renderer.scene.dummy_skeleton_bind_group));
        item.plan_draw(&mut plan, instances);
        plan.replay(render_pass, &renderer.device, renderer.multi_draw);
    }
    render_pass.set_scissor_rect(0, 0, width, height);
}

/// The main pass over the HDR target and the depth buffer, with what every draw in it shares bound.
fn begin_forward_pass<'e>(
    encoder: &'e mut wgpu::CommandEncoder,
    renderer: &Renderer,
    hdr_load: wgpu::LoadOp<wgpu::Color>,
    depth_load: wgpu::LoadOp<f32>,
) -> wgpu::RenderPass<'e> {
    let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Default Engine Render Pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &renderer.post.hdr_texture_view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: hdr_load,
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: &renderer.depth_texture_view,
            depth_ops: Some(wgpu::Operations {
                load: depth_load,
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    render_pass.set_bind_group(0, renderer.scene.view_bind_group(), &[]);
    // Web şemasında shadow grubu pipeline layout'unda yok (4-grup limiti).
    #[cfg(not(gizmo_forward_only))]
    render_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
    render_pass.set_bind_group(BG_INSTANCE, renderer.scene.view_instance_bind_group(), &[]);
    // Group 5 of the lit pipelines: the irradiance volume's group, for reflection probes — an empty
    // one without the deferred path. Bound once; it stays bound across the pipeline switches below.
    #[cfg(not(gizmo_forward_only))]
    render_pass.set_bind_group(5, renderer.irradiance_group(), &[]);
    render_pass
}

/// Draws these items, in order, with the forward pipeline each one asks for — through a
/// [`PassPlan`](super::plan::PassPlan), which binds what changes only and draws a run of items
/// sharing a pipeline, a material and a buffer as one multi-draw.
fn draw_forward_items(
    render_pass: &mut wgpu::RenderPass<'_>,
    renderer: &Renderer,
    draw_items: &[DrawItem],
    uploaded_instances: u32,
    baked: BakedRoute,
) {
    let mut plan = super::plan::PassPlan::default();
    for item in draw_items {
        // `ShadowCasting::Only`: casts, is not drawn — by this pass either, which until 2026-11-11 did
        // not ask, so a shadow-only draw stood in the picture whenever it was drawn forward: blended,
        // unlit, baked-lit, a game's own, and everything on the forward-only path. Its wireframe goes
        // with it, as the editor's does.
        if !item.visible_in_camera {
            continue;
        }
        let mut draw_solid = false;
        // Neither the sky nor a backdrop is world geometry a wireframe overlay is about,
        // and the wireframe pipeline is `shader.wgsl`: it neither camera-locks nor pins
        // depth, so a backdrop's wireframe would be drawn somewhere the backdrop is not.
        let wire = item.wire.as_ref().filter(|_| !item.is_skybox && !item.is_backdrop);
        // A view drawn as lines only (`Renderer::wire_only`): its world geometry is its wireframe
        // alone. The sky, a backdrop and the editor's grid are not geometry, and keep their draw.
        let lines_only = renderer.wire_only && !item.is_skybox && !item.is_backdrop && !item.is_grid;
        let wire = wire.filter(|_| !item.is_grid);

        // Forward: what the G-buffer does not hold (`DrawItem::in_gbuffer` — a baked-lit surface is in it
        // where there is one), and everything where there is none.
        if (item.is_skybox || !item.in_gbuffer(baked) || renderer.deferred.is_none() || item.is_transparent)
            && !item.drawn_reflective(baked)
            && !lines_only
        {
            draw_solid = true;
        }

        // Main pass: yalnız kamera-görünür instance'lar. camera_count==0 ise
        // (tüm batch shadow-only / kamera-dışı) aralık BOŞ olur — 0-instance
        // draw'ı hem boşa komut, hem tarayıcıda "instance count of 0 is
        // unusual" uyarısı üretir → tamamen atla.
        let inst_start = item.first_instance;
        let inst_end = (item.first_instance + item.camera_count)
            .min(uploaded_instances)
            .max(item.first_instance);
        if inst_end <= inst_start {
            continue;
        }

        let skel_bg = item
            .skeleton_bind_group
            .as_ref()
            .unwrap_or(&renderer.scene.dummy_skeleton_bind_group);

        if draw_solid {
            // A registered material is checked BEFORE every engine branch below, because it is
            // the one case where the engine has no opinion: the game named a pipeline and the
            // chain of `else if`s would otherwise route it by flags it never set. An id with no
            // entry in the registry draws nothing at all — see `Renderer::custom_materials` for
            // why that is deliberate.
            //
            // Not `continue`: the wireframe overlay below is a debugging view and a custom
            // material has no business being exempt from it.
            let custom_pipeline = item
                .custom
                .map(|id| renderer.custom_materials.get(id).map(|c| &c.pipeline));

            // A mirror with this frame's capture shows it; without one (off screen last frame,
            // seen from behind, split screen) it falls through and draws as its plain colour.
            let mirror = item.mirror.and_then(|owner| renderer.planar.captured(owner.id()));

            let engine_pipeline = if mirror.is_some() {
                if item.is_water {
                    &renderer.scene.core.water_mirror
                } else {
                    &renderer.scene.core.mirror
                }
            } else if item.is_backdrop {
                // FIRST, ahead of the `unlit` and `is_transparent` branches below: a
                // backdrop sets `unlit` (that is what keeps it out of the deferred and
                // shadow passes) and may well be transparent, and either of those
                // pipelines would draw it as ordinary world geometry — writing depth, not
                // camera-locked, in front of the world. The draw items reach this loop
                // already sorted with `DrawLayer::Backdrop` first.
                &renderer.scene.core.backdrop
            } else if item.is_skybox {
                &renderer.scene.core.sky
            } else if item.is_water {
                // Ahead of the `unlit` and `is_transparent` branches below, for the same
                // reason the backdrop branch is first: water sets `unlit` — that is what keeps
                // it out of the z-prepass, the G-buffer and both shadow passes — and a water
                // material with albedo alpha < 1 declares itself transparent too, so either of
                // those branches would draw the ocean with a vertex shader that does not
                // displace it. `water.wgsl` is the only pipeline in this chain whose *geometry*
                // differs from everyone else's, which is why it cannot fall through.
                //
                // Unlike the transparent pipeline, this one culls back faces, so the
                // double-sided flag is load-bearing here rather than redundant: a swimmer
                // looking up at the surface is the ordinary case.
                if item.is_double_sided {
                    &renderer.scene.core.water_double_sided
                } else {
                    &renderer.scene.core.water
                }
            } else if item.is_grid {
                // Ahead of `unlit`, which a grid also sets (`skips_deferred`): the grid
                // has its own shader, as in the editor.
                &renderer.scene.core.grid
            } else if item.baked_lit {
                // A baked-lit material that declared itself transparent gets the blended
                // variant. Every other stage already routes it as transparent (sorted
                // back-to-front here, skipped by the z-prepass and the shadow pass); the
                // opaque pipeline was the one place that ignored the declaration, and it
                // discarded the fragment alpha with it.
                if item.is_transparent {
                    &renderer.scene.core.baked_lit_transparent
                } else if item.is_double_sided {
                    &renderer.scene.core.baked_lit_double_sided
                } else {
                    &renderer.scene.core.baked_lit
                }
            } else if item.unlit {
                // A transparent unlit draw blending other than by alpha — a glow, a spark —
                // takes that mode's variant, which also does not write depth.
                match item.is_transparent.then(|| renderer.scene.blend_variant(true, item.blend_mode)).flatten() {
                    Some(p) => p,
                    None => renderer.scene.unlit_for(item.discards, item.is_transparent),
                }
            } else {
                // There is no two-sided *variant* of the blended pipeline because the blended
                // pipeline already is one: `transparent` is built with `cull_mode: None`
                // (`pipelines.rs`, and `baked_lit_state(true)` says the same for the baked-lit
                // half). So transparent geometry is never back-face culled, and
                // `Material::double_sided` is a no-op here — redundant rather than ignored.
                //
                // This comment previously claimed the opposite ("single-sided in both paths").
                // Corrected 2026-08-23 against a measurement in `demo/src/bin/blend_modes.rs`:
                // a back-facing transparent plane renders, and toggling `with_double_sided`
                // produces a bit-identical frame.
                if item.is_transmissive {
                    renderer.scene.transmissive_pipeline(item.is_double_sided)
                } else {
                    renderer.scene.lit_pipeline(item.lobes, item.details, item.is_transparent, item.is_double_sided, item.blend_mode)
                }
            };

            // A game's surface takes the place of the engine's lit pipelines, which are the only ones
            // above it can reach (it routes as `Pbr`), except a mirror's: a mirror shows its capture,
            // whatever its material is.
            let surface_pipeline = item.surface.filter(|_| mirror.is_none()).map(|id| {
                renderer.custom_materials.surface(id).map(|surface| {
                    if item.is_transmissive {
                        surface.transmissive_pipeline(item.is_double_sided)
                    } else {
                        surface.lit_pipeline(item.lobes, item.is_transparent, item.is_double_sided, item.blend_mode)
                    }
                })
            });

            // `Some(None)` is a registered id with nothing behind it: draw nothing rather than
            // fall through to `engine_pipeline`, which would shade it as ordinary PBR. An item is
            // a custom material or a surface, never both.
            let pipeline = match custom_pipeline.or(surface_pipeline) {
                Some(Some(p)) => Some(p),
                Some(None) => None,
                None => Some(engine_pipeline),
            };

            if let Some(pipeline) = pipeline {
                plan.set_pipeline(pipeline);
                plan.set_bind_group(1, mirror.unwrap_or(&*item.bind_group));
                plan.set_bind_group(BG_SKELETON, skel_bg.as_ref());
                item.plan_draw(&mut plan, inst_start..inst_end);
            }
        }

        if let Some(wire) = wire {
            plan.set_pipeline(&renderer.scene.core.wireframe);
            plan.set_bind_group(1, &item.bind_group);
            plan.set_bind_group(BG_SKELETON, skel_bg.as_ref());
            // Group 5 is this colour for the lines, and the probes again for whatever comes next.
            #[cfg(not(gizmo_forward_only))]
            plan.set_bind_group(5, wire);
            #[cfg(gizmo_forward_only)]
            let _ = wire;
            item.plan_draw(&mut plan, inst_start..inst_end);
            #[cfg(not(gizmo_forward_only))]
            plan.set_bind_group(5, renderer.irradiance_group());
        }
    }
    plan.replay(render_pass, &renderer.device, renderer.multi_draw);
}

/// The opaque baked-lit surfaces on [`BakedRoute::Reflective`]: drawn forward (`baked_lit.wgsl`'s
/// `fs_reflective`) into the frame and, beside it, into the G-buffer's RT1, RT2 and RT5, which is what
/// screen-space reflections read of them — rather than through the G-buffer's six targets and the
/// lighting pass, which cost a baked-lit city twice this draw — and into its velocity (RT4), which the
/// forward motion pass would otherwise draw them all again for. After the lighting pass, which wrote the
/// background where they stand: their depth decides what shows, as in the forward pass.
fn record_baked_reflective(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &Renderer,
    draw_items: &[DrawItem],
    uploaded_instances: u32,
    baked: BakedRoute,
) {
    let Some(def) = renderer.deferred.as_ref() else {
        return;
    };
    let drawn = |item: &&DrawItem| item.drawn_reflective(baked) && item.visible_in_camera && item.camera_count > 0;
    if !draw_items.iter().any(|item| drawn(&item)) {
        return;
    }
    let load = |view| {
        Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
        })
    };
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Baked-Lit Reflective Pass"),
        color_attachments: &[
            load(&renderer.post.hdr_texture_view),
            load(&def.normal_roughness_view),
            load(&def.world_position_view),
            load(&def.emissive_view),
            load(&def.velocity_view),
        ],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: &renderer.depth_texture_view,
            depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_bind_group(0, renderer.scene.view_bind_group(), &[]);
    #[cfg(not(gizmo_forward_only))]
    pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
    pass.set_bind_group(BG_INSTANCE, renderer.scene.view_instance_bind_group(), &[]);
    pass.set_bind_group(5, &def.motion.bind_group, &[]);
    let mut plan = super::plan::PassPlan::default();
    for item in draw_items.iter().filter(drawn) {
        let inst_start = item.first_instance;
        let inst_end = (item.first_instance + item.camera_count).min(uploaded_instances).max(item.first_instance);
        if inst_end <= inst_start {
            continue;
        }
        plan.set_pipeline(&def.baked_reflective_pipelines[usize::from(item.is_double_sided)]);
        plan.set_bind_group(1, &item.bind_group);
        plan.set_bind_group(
            BG_SKELETON,
            item.skeleton_bind_group.as_ref().unwrap_or(&renderer.scene.dummy_skeleton_bind_group),
        );
        item.plan_draw(&mut plan, inst_start..inst_end);
    }
    plan.replay(&mut pass, &renderer.device, renderer.multi_draw);
}

/// An [`EnvironmentMap`](crate::renderer::environment::EnvironmentMap) behind the scene: at the far
/// plane, so only where nothing was drawn, ahead of everything that blends over it.
fn draw_environment_sky(render_pass: &mut wgpu::RenderPass<'_>, renderer: &Renderer) {
    if renderer.environment_map.is_none() || renderer.skip_sky {
        return;
    }
    // The forward-only path keeps it in the cube its lit shader reads the sky from, on group 0.
    if let Some(pipeline) = renderer.scene.environment_background() {
        render_pass.set_pipeline(pipeline);
        render_pass.draw(0..3, 0..1);
        return;
    }
    let Some(def) = renderer.deferred.as_ref() else { return };
    render_pass.set_pipeline(&def.environment_sky_pipeline);
    render_pass.set_bind_group(1, &def.irradiance.bind_group, &[]);
    render_pass.draw(0..3, 0..1);
}
