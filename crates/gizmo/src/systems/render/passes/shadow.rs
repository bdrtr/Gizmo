use super::super::*;

/// What a draw casts with: a game's surface its own pipeline, anything else the engine's, by whether
/// it is drawn by its front faces alone (`front_faces`: its mesh closes, and the pass is the sun's) and
/// whether it cuts (a cut-out, a LOD crossfade). `None`: nothing it can cast with, which is a surface
/// where there is no shadow pass.
fn caster_pipeline<'a>(renderer: &'a Renderer, item: &DrawItem, front_faces: bool) -> Option<&'a wgpu::RenderPipeline> {
    match item.surface {
        // A surface that never cuts casts as a plain solid does: no fragment stage to run.
        Some(id) => {
            let surface = renderer.custom_materials.surface(id)?;
            if surface.is_solid() {
                Some(renderer.scene.shadow_pipeline_for(front_faces, item.discards))
            } else {
                surface.shadow_pipeline(front_faces)
            }
        }
        None => Some(renderer.scene.shadow_pipeline_for(front_faces, item.discards)),
    }
}

/// Binds what every shadow pipeline reads per draw — the material for a cut-out's fragment stage,
/// the skeleton — and the pipeline, through `plan`, which records each only when it changes.
///
/// A solid caster's pipeline has no fragment stage, and its vertex stage reads no material: its group
/// 1 is there for the layout alone, so every solid caster of the pass binds the same one, `solid` (the
/// first solid caster's). Bound each its own, a car's two dozen parts — one material each — were two
/// dozen draws in every cascade, which runs of casters sharing everything else would draw as one
/// (`PassPlan`'s multi-draws).
fn bind_caster<'a>(
    plan: &mut super::plan::PassPlan<'a>,
    renderer: &'a Renderer,
    item: &'a DrawItem,
    pipeline: &'a wgpu::RenderPipeline,
    solid: &mut Option<&'a wgpu::BindGroup>,
) {
    plan.set_pipeline(pipeline);
    let skeleton = item.skeleton_bind_group.as_ref().unwrap_or(&renderer.scene.dummy_skeleton_bind_group);
    let cuts = item.discards || item.surface.is_some_and(|id| renderer.custom_materials.surface(id).is_some_and(|s| !s.is_solid()));
    let material = if cuts { item.bind_group.as_ref() } else { *solid.get_or_insert(item.bind_group.as_ref()) };
    plan.set_bind_group(1, material);
    plan.set_bind_group(2, skeleton.as_ref());
}

/// What a depth pass stands for: the light, in a shadow map, or the rain, in the rain map. Glass is the
/// difference — it lets the light through and stops the rain. The rain map drew none of it until
/// 2026-11-20, and a car's cabin under its windows took the rain as if it had no roof: its parcel shelf
/// soaked to a mirror and beaded, behind the rear window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stops {
    Light,
    Rain,
}

/// Whether `item` is drawn into a depth pass that `stops` it: its own flag, then its material's say — the
/// same answer for the cascades and every local light's layer, and for the rain map but for glass.
fn casts_into_shadow_maps(renderer: &Renderer, item: &DrawItem, stops: Stops) -> bool {
    // Per-object opt-out, on top of the material rules: `MeshRenderer::shadows`. A flag the editor
    // obeys and the game ignores would make the viewport lie about the build.
    if !item.casts_shadows {
        return false;
    }
    // A game's surface casts as `Pbr` does — the branches below — unless its id has nothing behind
    // it, which draws nothing, its shadow included.
    if item.surface.is_some_and(|id| renderer.custom_materials.surface(id).is_none()) {
        return false;
    }
    // A custom material routes as `unlit` — that is what keeps it out of the deferred path — but
    // "skips the G-buffer" is not "is not solid". The shadow pass writes depth only and does not
    // care how a fragment would have shaded, so the material itself decides, through
    // `CustomMaterial::casts_shadows`. Without this arm the flag would be a field that reads `true`
    // and does nothing, which is the exact defect `CAPABILITY_GAPS.md` §D keeps a list of.
    match item.custom.map(|id| renderer.custom_materials.get(id).is_some_and(|c| c.casts_shadows)) {
        Some(cast) => cast,
        // Baked-lit geometry casts. It is `unlit` in the sense of skipping the deferred path, but it
        // is a solid world and a world that does not occlude the sun has no shadows in it at all. What
        // is transparent lets the light through, and not the rain.
        None => !((item.unlit && !item.baked_lit) || (item.is_transparent && stops == Stops::Light)),
    }
}

/// One depth pass into `target`, through the shadow pass group `light`, drawing the casters whose
/// chunks meet `view_proj`'s frustum (`gizmo_renderer::shadow_culling`). Returns the instances it
/// drew.
///
/// `local`: a local light's layer, where every caster is drawn by both faces — a light can be inside
/// a solid, which its front faces alone would let it shine out of. The sun's cascades draw a closed
/// mesh by its front faces, what the light meets first (`pipelines::build_shadow_pipelines`).
#[allow(clippy::too_many_arguments)]
fn record_caster_pass(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &Renderer,
    draw_items: &[DrawItem],
    uploaded_instances: u32,
    label: &str,
    target: &wgpu::TextureView,
    light: &wgpu::BindGroup,
    view_proj: Mat4,
    local: bool,
    stops: Stops,
    clear: bool,
) -> u32 {
    let frustum = crate::math::Frustum::from_matrix(&view_proj);
    let mut drawn = 0;
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: target,
            depth_ops: Some(wgpu::Operations {
                load: if clear { wgpu::LoadOp::Clear(1.0) } else { wgpu::LoadOp::Load },
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    // Every shadow pipeline has the same groups: the light's matrix, the material (read by a
    // cut-out), the skeleton and the instances.
    pass.set_bind_group(0, light, &[]);
    pass.set_bind_group(3, renderer.scene.view_instance_bind_group(), &[]);
    // Through a plan: a run of casters sharing a pipeline, a material and a buffer is one multi-draw.
    let mut plan = super::plan::PassPlan::default();
    let mut solid = None;
    for item in draw_items {
        if !casts_into_shadow_maps(renderer, item, stops) {
            continue;
        }
        let Some(pipeline) = caster_pipeline(renderer, item, item.closed && !local) else { continue };
        // The runs of this batch's instances — camera-visible ones (region A) and off-screen
        // casters (region B) alike — that this pass's frustum meets, each a draw. Through
        // `record_draw`, which binds the index buffer: the point light's cube drew `0..vertex_count`
        // unindexed until 2026-11-11, so an indexed mesh — every primitive and import — cast its
        // deduplicated corners taken three at a time, a plane its first triangle and nothing of the
        // second.
        let mut ranges = gizmo_renderer::shadow_culling::visible_ranges(&item.shadow_chunks, &frustum, uploaded_instances).peekable();
        if ranges.peek().is_some() {
            bind_caster(&mut plan, renderer, item, pipeline, &mut solid);
        }
        for range in ranges {
            drawn += range.end - range.start;
            item.plan_draw(&mut plan, range);
        }
    }
    plan.replay(&mut pass, &renderer.device, renderer.multi_draw);
    drawn
}

/// The rain map (`gizmo_renderer::rain_map`): the scene's depth from straight above, `view_proj`'s, into
/// the sun array's layer past the cascades, through its pass group (`SceneState::rain_map`, written
/// before). `cleared`: whether [`record_shadow_passes`] cleared the whole array this frame, the rain's
/// layer with it; if not, the pass clears its layer itself. Every caster by both faces, as a local light's layer: a bridge's deck is often a sheet, and
/// one face of it would let the rain through. And glass, which casts no shadow but keeps the rain off what
/// is under it ([`Stops::Rain`]). Returns the instances it drew: none while the array has no
/// layer for it (`SceneState::shadow_rain_layer_view`).
pub fn record_rain_map(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &Renderer,
    draw_items: &[DrawItem],
    uploaded_instances: u32,
    view_proj: Mat4,
    cleared: bool,
) -> u32 {
    let Some(target) = renderer.scene.shadow_rain_layer_view.as_ref() else {
        return 0;
    };
    record_caster_pass(
        encoder,
        renderer,
        draw_items,
        uploaded_instances,
        "Rain Map Pass",
        target,
        &renderer.scene.rain_map.bind_group,
        view_proj,
        true,
        Stops::Rain,
        !cleared,
    )
}

/// Clears every layer of the sun's shadow array — the cascades and the rain map's — in one pass, where
/// the device can (`wgpu::Features::MULTIVIEW`), and says whether it did. A pass of a cascade clearing
/// its own layer clears a part of the texture, which a driver need not do as a fast clear: Mesa's
/// RADV does not, and wrote every texel of the four layers instead — 0.65 ms of a wet drive's 0.9 of
/// shadow maps in `nfsu2-gizmo` (1080p, AMD 780M, 2026-11-21), where the cars drawn into them took 0.25.
/// One pass over all of them is a clear of the whole texture.
fn clear_sun_layers(encoder: &mut wgpu::CommandEncoder, renderer: &Renderer) -> bool {
    let layers = renderer.scene.shadow_depth_texture.depth_or_array_layers();
    let Some(mask) = std::num::NonZeroU32::new(1u32.checked_shl(layers).map_or(u32::MAX, |b| b - 1))
        .filter(|_| {
            renderer.device.features().contains(wgpu::Features::MULTIVIEW)
                && layers > 1
                && layers <= renderer.device.limits().max_multiview_view_count
        })
    else {
        return false;
    };
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Shadow Array Clear"),
        color_attachments: &[],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: &renderer.scene.shadow_texture_view,
            depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: Some(mask),
    });
    true
}

/// The frame's shadow maps: one depth pass per cascade when `cascades` holds their matrices
/// (`SceneSetup::draws_cascades` — the frame has a sun), then one per local shadow layer the frame
/// planned (`SceneSetup::local_shadows`), each matrix already written to its pass's buffer. A frame
/// with no sun and no light that casts records none. Returns the passes recorded and the instances
/// they drew between them.
pub fn record_shadow_passes(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &Renderer,
    draw_items: &[DrawItem],
    uploaded_instances: u32,
    cascades: Option<&[Mat4; crate::renderer::CASCADE_COUNT]>,
    local_layers: &[gizmo_renderer::local_shadows::LocalShadowLayer],
) -> (u32, u32, bool) {
    let (mut passes, mut drawn) = (0, 0);
    // The whole array at once where it can be, before any cascade draws into it (`clear_sun_layers`).
    let cleared = cascades.is_some() && clear_sun_layers(encoder, renderer);
    // As many as this path draws (`CASCADES_DRAWN`: one on the forward-only path).
    for (i, view_proj) in cascades.into_iter().flatten().take(crate::renderer::csm::CASCADES_DRAWN).enumerate() {
        drawn += record_caster_pass(
            encoder,
            renderer,
            draw_items,
            uploaded_instances,
            "Shadow Pass",
            &renderer.scene.shadow_cascade_layer_views[i],
            renderer.scene.view_shadow_pass_bind_group(i),
            *view_proj,
            false,
            Stops::Light,
            !cleared,
        );
        passes += 1;
    }
    let local_layers = &local_layers[..local_layers.len().min(renderer.scene.local_shadow_maps.layers())];
    for (i, layer) in local_layers.iter().enumerate() {
        drawn += record_caster_pass(
            encoder,
            renderer,
            draw_items,
            uploaded_instances,
            "Local Shadow Pass",
            &renderer.scene.local_shadow_maps.layer_views[i],
            renderer.scene.view_local_shadow_pass_bind_group(i),
            layer.view_proj,
            true,
            Stops::Light,
            true,
        );
        passes += 1;
    }
    (passes, drawn, cleared)
}
