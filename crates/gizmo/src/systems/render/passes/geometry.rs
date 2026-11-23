use super::super::*;

pub fn record_deferred_geometry(
    encoder: &mut wgpu::CommandEncoder,
    // `mut` for the GPU timer's marks between the passes (`gpu.prepass`, `gpu.gbuffer`).
    renderer: &mut Renderer,
    world: &World,
    draw_items: &[DrawItem],
    uploaded_instances: u32,
    cam_pos: gizmo_math::Vec3,
    // Where the opaque baked-lit surfaces are drawn this view (`DrawItem::in_gbuffer`).
    baked: BakedRoute,
) {
    // A view drawn as lines only (`Renderer::wire_only`) puts nothing into the G-buffer: its passes
    // still clear their targets, so the lighting shades only the sky, and the forward pass draws the
    // lines (`passes::forward`).
    let draw_items: &[DrawItem] = if renderer.wire_only { &[] } else { draw_items };

    // ── Z-Prepass (Depth Only) ────────────────────────────────────────────────
    if let Some(ref def) = renderer.deferred {
        z_prepass(encoder, renderer, def, draw_items, uploaded_instances, baked);
    }
    if renderer.deferred.is_some() {
        renderer.gpu_mark(encoder, "gpu.prepass");
    }

    // ── G-buffer pass (PBR geometry → albedo / normal / world-position) ─────
    if let Some(ref def) = renderer.deferred {
        gbuffer_pass(encoder, renderer, def, draw_items, uploaded_instances, baked);
    }
    if renderer.deferred.is_some() {
        renderer.gpu_mark(encoder, "gpu.gbuffer");
    }

    decal_pass(encoder, renderer, world, cam_pos);

    // ── Deferred lighting pass (G-buffers → HDR) ──────────────────────────
    if let Some(ref def) = renderer.deferred {
        lighting_pass(encoder, renderer, def);
    }
}

/// The depth-only prepass: the opaque G-buffer surfaces, and the solid baked-lit ones the forward
/// pass draws, lay their depth.
fn z_prepass(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &Renderer,
    def: &gizmo_renderer::deferred::DeferredState,
    draw_items: &[DrawItem],
    uploaded_instances: u32,
    baked: BakedRoute,
) {
let mut z_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
    label: Some("Z-Prepass"),
    color_attachments: &[], // No color targets
    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
        view: &renderer.depth_texture_view,
        depth_ops: Some(wgpu::Operations {
            load: wgpu::LoadOp::Clear(1.0),
            store: wgpu::StoreOp::Store,
        }),
        stencil_ops: None,
    }),
    timestamp_writes: None,
    occlusion_query_set: None,
    multiview_mask: None,
});
z_pass.set_bind_group(0, renderer.scene.view_bind_group(), &[]);
z_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
z_pass.set_bind_group(4, renderer.scene.view_instance_bind_group(), &[]);
z_pass.set_bind_group(5, &def.motion.bind_group, &[]);
// The cull mode is baked into the pipeline, so a double-sided material needs its own.
// Tracked and set only on change rather than per item: the batches are not sorted by this
// flag and a scene is overwhelmingly one-sided, so this is a handful of switches at most.
// Through a plan: it binds what changes only, and draws a run sharing a pipeline, a material
// and a buffer as one multi-draw.
let mut plan = super::plan::PassPlan::default();
let mut bound: Option<(Option<gizmo_renderer::custom_material::SurfaceId>, bool, bool, bool)> = None;
for item in draw_items {
    // `ShadowCasting::Only`: casts, is not drawn. Checked in the z-prepass as well as the
    // G-buffer below, or the object would still write depth and punch a hole in whatever
    // stands behind it.
    if !item.visible_in_camera {
        continue;
    }
    // A solid baked-lit surface the forward pass draws (`BakedRoute::Forward`, `Reflective`) lays its
    // depth here too, through its own vertex stage (`CorePipelines::baked_lit_depth`), so
    // that the forward pass shades only what is in front of it. It used not to, on the reasoning
    // that shading an overlap again costs less than drawing everything twice; but a city's streets
    // are layer on layer, and a wet one's shading is dear. `nfsu2-gizmo`'s city, 1080p, AMD 780M,
    // 2026-11-21: a street view's forward pass 11.9 → 6.3 ms wet and 8.4 → 4.7 ms dry, for 1.6 ms
    // of prepass; a wet drive's 5.1 → 3.2 ms, for 0.6 ms.
    //
    // A cut-out stays out: laying its depth takes a fragment stage that samples and discards as
    // the forward draw will, and the leaves and fences that cut hide little — the same views
    // spent 0.2 ms more on the prepass and not a hundredth less on the forward pass. One on the
    // G-buffer route writes its depth in the G-buffer pass (`fs_baked`'s pipelines), where the
    // lighting pass shades each pixel once whatever overlaps.
    let baked_depth = item.baked_lit
        && !item.in_gbuffer(baked)
        && !item.is_transparent
        && !item.is_skybox
        && !item.discards
        && item.surface.is_none();
    if !baked_depth && (!item.in_gbuffer(baked) || item.baked_lit || item.is_skybox || item.is_transparent) {
        continue;
    }
    let key = (item.surface, item.is_double_sided, item.discards, baked_depth);
    if bound != Some(key) {
        let pipeline = if baked_depth {
            &renderer.scene.core.baked_lit_depth[usize::from(item.is_double_sided)]
        } else {
            match item.surface {
                // A game's surface cuts where its own code says, so its prepass always has the
                // fragment stage; an id with nothing behind it draws nothing, here as everywhere.
                Some(id) => match renderer.custom_materials.surface(id) {
                    // A surface that never cuts leaves the depth a plain solid does.
                    Some(s) if s.is_solid() => plain_prepass(def, item.discards, item.is_double_sided),
                    Some(s) => match s.prepass_pipeline(item.is_double_sided) {
                        Some(pipeline) => pipeline,
                        None => continue,
                    },
                    None => continue,
                },
                None => plain_prepass(def, item.discards, item.is_double_sided),
            }
        };
        plan.set_pipeline(pipeline);
        bound = Some(key);
    }
    let skel_bg = item
        .skeleton_bind_group
        .as_ref()
        .unwrap_or(&renderer.scene.dummy_skeleton_bind_group);
    plan.set_bind_group(3, skel_bg.as_ref());
    plan.set_bind_group(1, &item.bind_group);
    // Main pass: camera-visible instances only.
    item.plan_draw(&mut plan, item.camera_instance_range(uploaded_instances));
}
plan.replay(&mut z_pass, &renderer.device, renderer.multi_draw);
}

/// The prepass pipeline of a surface with no fragment stage of its own: by whether it cuts and
/// whether it shows both faces. Written out twice — for the engine's surfaces and for a game's that
/// never cuts.
fn plain_prepass(def: &gizmo_renderer::deferred::DeferredState, discards: bool, double_sided: bool) -> &wgpu::RenderPipeline {
    match (discards, double_sided) {
        (true, two_sided) => &def.z_prepass_discard_pipelines[usize::from(two_sided)],
        (false, true) => &def.z_prepass_double_sided_pipeline,
        (false, false) => &def.z_prepass_pipeline,
    }
}

/// The G-buffer pass: the opaque surfaces into albedo, normal, world position, tangent, velocity
/// and emissive.
fn gbuffer_pass(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &Renderer,
    def: &gizmo_renderer::deferred::DeferredState,
    draw_items: &[DrawItem],
    uploaded_instances: u32,
    baked: BakedRoute,
) {
let mut gbuf_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
    label: Some("G-Buffer Pass"),
    color_attachments: &[
        Some(wgpu::RenderPassColorAttachment {
            view: &def.albedo_metallic_view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        }),
        Some(wgpu::RenderPassColorAttachment {
            view: &def.normal_roughness_view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        }),
        Some(wgpu::RenderPassColorAttachment {
            view: &def.world_position_view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                // TRANSPARENT, not BLACK: `.w` is the written flag every reader tests
                // (`pos_sample.w < 0.5` = no geometry). BLACK's alpha is 1, so every empty
                // pixel read as a surface at the camera — lit as one, and counted as an
                // occluder at distance 0 by SSAO.
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        }),
        Some(wgpu::RenderPassColorAttachment {
            view: &def.world_tangent_view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        }),
        // RT4: velocity — zero wherever no G-buffer geometry lands (the sky), so the
        // post pass falls back to the camera's own motion there.
        Some(wgpu::RenderPassColorAttachment {
            view: &def.velocity_view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        }),
        // RT5: emissive — black where nothing glows.
        Some(wgpu::RenderPassColorAttachment {
            view: &def.emissive_view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        }),
    ],
    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
        view: &renderer.depth_texture_view,
        depth_ops: Some(wgpu::Operations {
            load: wgpu::LoadOp::Load, // Z-Prepass populated this!
            store: wgpu::StoreOp::Store,
        }),
        stencil_ops: None,
    }),
    timestamp_writes: None,
    occlusion_query_set: None,
    multiview_mask: None,
});
gbuf_pass.set_bind_group(0, renderer.scene.view_bind_group(), &[]);
gbuf_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
gbuf_pass.set_bind_group(4, renderer.scene.view_instance_bind_group(), &[]);
gbuf_pass.set_bind_group(5, &def.motion.bind_group, &[]);
let mut plan = super::plan::PassPlan::default();
let mut bound: Option<(Option<gizmo_renderer::custom_material::SurfaceId>, bool, bool)> = None;
for item in draw_items {
    if !item.visible_in_camera {
        continue; // ShadowCasting::Only
    }
    if !item.in_gbuffer(baked) || item.is_transparent {
        continue;
    }
    let key = (item.surface, item.is_double_sided, item.baked_lit);
    if bound != Some(key) {
        let pipeline = match item.surface {
            // The engine's G-buffer shader around the game's surface: written as `Pbr` is.
            Some(id) => match renderer.custom_materials.surface(id).and_then(|s| s.gbuffer_pipeline(item.is_double_sided)) {
                Some(pipeline) => pipeline,
                None => continue,
            },
            // A baked-lit surface, finished by the lighting pass as `baked_lit.wgsl` draws it.
            None if item.baked_lit => &def.gbuffer_baked_pipelines[usize::from(item.is_double_sided)],
            None if item.is_double_sided => &def.gbuffer_double_sided_pipeline,
            None => &def.gbuffer_pipeline,
        };
        plan.set_pipeline(pipeline);
        bound = Some(key);
    }
    let skel_bg = item
        .skeleton_bind_group
        .as_ref()
        .unwrap_or(&renderer.scene.dummy_skeleton_bind_group);
    plan.set_bind_group(3, skel_bg.as_ref());
    plan.set_bind_group(1, &item.bind_group);
    // Main pass: camera-visible instances only.
    item.plan_draw(&mut plan, item.camera_instance_range(uploaded_instances));
}
plan.replay(&mut gbuf_pass, &renderer.device, renderer.multi_draw);
}

/// The decal pass: the frame's decals, blended into the G-buffer's albedo.
fn decal_pass(encoder: &mut wgpu::CommandEncoder, renderer: &Renderer, world: &World, cam_pos: gizmo_math::Vec3) {
    // ── Decal Pass (Blend into G-buffer) ──────────────────────────
    // What to draw comes from the shared collector, so this pass and the editor's forward one
    // cannot disagree about where a decal is — only about which target it lands in.
    let (decal_uniforms, decal_draws) = super::super::collect_decals(world, cam_pos);
    if let Some(ref decal_state) = renderer.decal {
        if !decal_uniforms.is_empty() {
            renderer.queue.write_buffer(
                &decal_state.uniform_buffer,
                0,
                bytemuck::cast_slice(&decal_uniforms),
            );
        }
    }

    if !decal_draws.is_empty() {
        if let (Some(decal_state), Some(ref def)) =
            (renderer.decal.as_ref().filter(|d| d.enabled), &renderer.deferred)
        {
            let mut decal_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Decal Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &def.albedo_metallic_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None, // No depth testing needed
                timestamp_writes: None,
                occlusion_query_set: None,
            multiview_mask: None,
            });

            decal_pass.set_pipeline(&decal_state.pipeline);
            decal_pass.set_bind_group(0, renderer.scene.view_bind_group(), &[]);
            decal_pass.set_bind_group(1, &decal_state.world_pos_bg, &[]);
            decal_pass.set_vertex_buffer(0, decal_state.vertex_buffer.slice(..));

            for (i, bind_group) in decal_draws.iter().enumerate() {
                let offset = (i as wgpu::BufferAddress * gizmo_renderer::decal::DECAL_STRIDE) as u32;
                decal_pass.set_bind_group(2, bind_group.as_ref(), &[]);
                decal_pass.set_bind_group(3, &decal_state.decal_uniform_bg, &[offset]);
                decal_pass.draw(0..gizmo_renderer::decal::CUBE_VERTICES, 0..1);
            }
        }
    }
}

/// The deferred lighting pass: the G-buffer, lit, into the HDR target.
fn lighting_pass(encoder: &mut wgpu::CommandEncoder, renderer: &Renderer, def: &gizmo_renderer::deferred::DeferredState) {
let mut light_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
    label: Some("Deferred Lighting Pass"),
    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
        view: &renderer.post.hdr_texture_view,
        depth_slice: None,
        resolve_target: None,
        ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.4, g: 0.6, b: 0.9, a: 1.0 }),
            store: wgpu::StoreOp::Store,
        },
    })],
    depth_stencil_attachment: None,
    timestamp_writes: None,
    occlusion_query_set: None,
    multiview_mask: None,
});
light_pass.set_pipeline(&def.lighting_pipeline);
light_pass.set_bind_group(0, renderer.scene.view_bind_group(), &[]);
light_pass.set_bind_group(1, &renderer.scene.shadow_bind_group, &[]);
light_pass.set_bind_group(2, &def.gbuffer_bind_group, &[]);
// Group 3 is always bound because the pipeline layout always declares it; an empty grid
// is a one-probe buffer of zeros and `gi_sample` returns black for it.
light_pass.set_bind_group(3, &def.irradiance.bind_group, &[]);
light_pass.draw(0..3, 0..1);
}
