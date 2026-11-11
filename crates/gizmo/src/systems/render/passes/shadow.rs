use super::super::*;

/// What a draw casts with: a game's surface its own pipeline, anything else the engine's, by whether
/// its mesh closes and whether it cuts (a cut-out, a LOD crossfade). `None`: nothing it can cast
/// with, which is a surface where there is no shadow pass.
#[cfg(not(gizmo_forward_only))]
fn caster_pipeline<'a>(renderer: &'a Renderer, item: &DrawItem) -> Option<&'a wgpu::RenderPipeline> {
    match item.surface {
        Some(id) => renderer.custom_materials.surface(id)?.shadow_pipeline(item.closed),
        None => Some(renderer.scene.shadow_pipeline_for(item.closed, item.discards)),
    }
}

/// Binds what every shadow pipeline reads per draw — the material for a cut-out's fragment stage,
/// the skeleton — and the pipeline itself when it changes.
#[cfg(not(gizmo_forward_only))]
fn bind_caster(
    pass: &mut wgpu::RenderPass<'_>,
    renderer: &Renderer,
    item: &DrawItem,
    pipeline: &wgpu::RenderPipeline,
    bound: &mut Option<*const wgpu::RenderPipeline>,
) {
    if *bound != Some(std::ptr::from_ref(pipeline)) {
        pass.set_pipeline(pipeline);
        *bound = Some(std::ptr::from_ref(pipeline));
    }
    let skeleton = item.skeleton_bind_group.as_ref().unwrap_or(&renderer.scene.dummy_skeleton_bind_group);
    pass.set_bind_group(1, &*item.bind_group, &[]);
    pass.set_bind_group(2, skeleton.as_ref(), &[]);
}

/// Whether `item` casts into a shadow map at all: its own flag, then its material's say — the same
/// answer for the cascades and every local light's layer.
#[cfg(not(gizmo_forward_only))]
fn casts_into_shadow_maps(renderer: &Renderer, item: &DrawItem) -> bool {
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
        // is a solid world and a world that does not occlude the sun has no shadows in it at all.
        None => !((item.unlit && !item.baked_lit) || item.is_transparent),
    }
}

/// One depth pass into `target`, drawing every caster through the shadow pass group `light`.
#[cfg(not(gizmo_forward_only))]
fn record_caster_pass(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &Renderer,
    draw_items: &[DrawItem],
    uploaded_instances: u32,
    label: &str,
    target: &wgpu::TextureView,
    light: &wgpu::BindGroup,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: target,
            depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    // Every shadow pipeline has the same groups: the light's matrix, the material (read by a
    // cut-out), the skeleton and the instances.
    pass.set_bind_group(0, light, &[]);
    pass.set_bind_group(3, &renderer.scene.instance_bind_group, &[]);
    let mut bound = None;
    for item in draw_items {
        if !casts_into_shadow_maps(renderer, item) {
            continue;
        }
        let Some(pipeline) = caster_pipeline(renderer, item) else { continue };
        bind_caster(&mut pass, renderer, item, pipeline, &mut bound);
        // Two-region instance layout (see collect_draw_items): the camera-visible instances
        // (region A) and the off-screen shadow-only casters (region B) are not contiguous, so cast
        // from BOTH ranges with a draw each. Empty ranges draw 0 instances = no-op. Through
        // `record_draw`, which binds the index buffer: the point light's cube drew `0..vertex_count`
        // unindexed until 2026-11-11, so an indexed mesh — every primitive and import — cast its
        // deduplicated corners taken three at a time, a plane its first triangle and nothing of the
        // second.
        item.record_draw(&mut pass, item.camera_instance_range(uploaded_instances));
        item.record_draw(&mut pass, item.shadow_instance_range(uploaded_instances));
    }
}

/// The frame's shadow maps: one depth pass per cascade, then one per local shadow layer the frame
/// planned (`local_layers`, `SceneSetup::local_shadows`), each layer's matrix already written to its
/// buffer. A frame where no light casts records no local pass at all.
#[cfg(not(gizmo_forward_only))]
pub fn record_shadow_passes(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &Renderer,
    draw_items: &[DrawItem],
    uploaded_instances: u32,
    local_layers: usize,
) {
    for i in 0..crate::renderer::CASCADE_COUNT {
        record_caster_pass(
            encoder,
            renderer,
            draw_items,
            uploaded_instances,
            "Shadow Pass",
            &renderer.scene.shadow_cascade_layer_views[i],
            renderer.scene.view_shadow_pass_bind_group(i),
        );
    }
    let local_layers = local_layers.min(renderer.scene.local_shadow_maps.layers());
    for i in 0..local_layers {
        record_caster_pass(
            encoder,
            renderer,
            draw_items,
            uploaded_instances,
            "Local Shadow Pass",
            &renderer.scene.local_shadow_maps.layer_views[i],
            renderer.scene.view_local_shadow_pass_bind_group(i),
        );
    }
}
