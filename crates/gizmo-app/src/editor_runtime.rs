//! Per-frame editor integration extracted out of the windowed event loop.
//!
//! The windowed [`App`](crate::App) loop is editor-agnostic: it only knows how
//! to drive the generic [`EguiContext`](crate::egui_ctx::EguiContext) overlay.
//! Everything that reads or mutates [`gizmo_editor::EditorState`] — the
//! scene/game viewport render-to-texture management and the scene save / load /
//! clear request handling — lives here, behind the `editor` feature, so the
//! event loop stays free of editor internals.
//!
//! This module is only compiled with the `editor` feature.

use crate::egui_ctx::EguiContext;
use gizmo_core::world::World;
use gizmo_renderer::renderer::Renderer;

/// The format egui must sample a viewport texture through, given the format the engine renders it
/// in.
///
/// One line, named, because it is the whole of the bug described on [`create_viewport_target`]: the
/// render view keeps the sRGB format so the hardware encodes, and egui gets the raw variant because
/// its shader wants bytes it can treat as gamma.
fn egui_sample_format(render_format: wgpu::TextureFormat) -> wgpu::TextureFormat {
    render_format.remove_srgb_suffix()
}

/// Creates one viewport render-to-texture and registers it with egui, returning the view the
/// engine renders into and the id the panel samples.
///
/// # The colour space, which was wrong for both viewports
///
/// egui's shader states its contract in a comment: *"We expect 'normal' textures that are NOT
/// sRGB-aware."* It samples a user texture, treats the value as **gamma-encoded**, and then applies
/// `linear_from_gamma_rgb` on the way out because the framebuffer re-encodes. Hand it an sRGB
/// texture and the hardware decodes on the way in as well — two decodes, one encode, and every
/// pixel of the 3D viewport lands one gamma step too dark. Measured through the post chain: the
/// composite writes linear 0.5, and 128 reaches the screen where 188 belongs. A dark scene turns
/// nearly black, which is what "the editor looks flat and washed out" actually was.
///
/// The fix is not to change the texture's format — `run_post_processing`'s pipeline is built for
/// `config.format`, and an attachment that disagrees is a validation error. The texture stays sRGB
/// so the hardware still encodes what the post chain writes; egui gets a **second view of the same
/// memory** in the non-sRGB variant, so its sample returns those encoded bytes verbatim, exactly
/// the "not sRGB-aware" texture it asked for.
///
/// On a surface format with no sRGB pair (`remove_srgb_suffix` is then a no-op) the two views are
/// the same and this is exactly the old behaviour — nothing to reinterpret, nothing gained.
fn create_viewport_target(
    device: &wgpu::Device,
    egui_renderer: &mut egui_wgpu::Renderer,
    label: &str,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> (std::sync::Arc<wgpu::TextureView>, egui::TextureId) {
    let sample_format = egui_sample_format(format);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[sample_format],
    });
    // What the engine renders into: the texture's own format, so the write is gamma-encoded.
    let render_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    // What egui samples: the same bytes, reinterpreted as raw.
    let sample_view = texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("egui sample view (gamma space)"),
        format: Some(sample_format),
        ..Default::default()
    });
    let id = egui_renderer.register_native_texture(device, &sample_view, wgpu::FilterMode::Linear);
    (std::sync::Arc::new(render_view), id)
}

/// Keeps the editor's scene/game viewport render targets sized to the panels.
///
/// When the editor's `EditorState` resource is present, this (re)creates the
/// offscreen textures the Scene View and Game View panels draw into whenever
/// their requested size changes, registers them with the egui renderer so the
/// panels can sample them, and publishes the matching `EditorRenderTarget` /
/// `GameRenderTarget` resources the engine renders into.
#[tracing::instrument(skip_all, name = "editor_sync_render_targets")]
pub fn sync_render_targets(world: &mut World, editor: &mut EguiContext) {
    // --- Scene View RTT (Render To Texture) YÖNETİMİ ---
    if world
        .get_resource::<gizmo_editor::EditorState>()
        .is_some()
    {
        let mut ed_state_ref = world
            .get_resource_mut::<gizmo_editor::EditorState>()
            .unwrap();
        let (rw, rh) = {
            let r = world.get_resource::<Renderer>().unwrap();
            (r.size.width, r.size.height)
        };
        let scene_w = ed_state_ref.scene_view_size.map(|s| s.x as u32).unwrap_or(rw);
        let scene_h = ed_state_ref.scene_view_size.map(|s| s.y as u32).unwrap_or(rh);
        let game_w = ed_state_ref.game_view_size.map(|s| s.x as u32).unwrap_or(rw);
        let game_h = ed_state_ref.game_view_size.map(|s| s.y as u32).unwrap_or(rh);

        let mut new_scene_target = None;
        let mut new_game_target = None;

        // Scene View RTT
        let mut needs_recreate_scene = false;
        if let Some(target) =
            world.get_resource::<gizmo_renderer::components::EditorRenderTarget>()
        {
            if target.0.width != scene_w || target.0.height != scene_h {
                needs_recreate_scene = true;
            }
        } else {
            needs_recreate_scene = true;
        }

        if needs_recreate_scene && scene_w > 0 && scene_h > 0 {
            if let Some(old_id) = ed_state_ref.scene_texture_id {
                editor.renderer.free_texture(&old_id);
            }
            let tex_id;
            {
                let r = world.get_resource::<Renderer>().unwrap();
                let (view, id) = create_viewport_target(
                    &r.device,
                    &mut editor.renderer,
                    "Editor RTT",
                    r.config.format,
                    scene_w,
                    scene_h,
                );
                tex_id = Some(id);
                new_scene_target = Some((view, scene_w, scene_h));
            }
            ed_state_ref.scene_texture_id = tex_id;
            tracing::debug!(
                width = scene_w,
                height = scene_h,
                "[Editor] scene-view RTT (re)created"
            );
        }

        // Game View RTT
        let mut needs_recreate_game = false;
        if let Some(target) =
            world.get_resource::<gizmo_renderer::components::GameRenderTarget>()
        {
            if target.0.width != game_w || target.0.height != game_h {
                needs_recreate_game = true;
            }
        } else {
            needs_recreate_game = true;
        }

        if needs_recreate_game && game_w > 0 && game_h > 0 {
            if let Some(old_id) = ed_state_ref.game_texture_id {
                editor.renderer.free_texture(&old_id);
            }
            let tex_id;
            {
                let r = world.get_resource::<Renderer>().unwrap();
                let (view, id) = create_viewport_target(
                    &r.device,
                    &mut editor.renderer,
                    "Game RTT",
                    r.config.format,
                    game_w,
                    game_h,
                );
                tex_id = Some(id);
                new_game_target = Some((view, game_w, game_h));
            }
            ed_state_ref.game_texture_id = tex_id;
            tracing::debug!(
                width = game_w,
                height = game_h,
                "[Editor] game-view RTT (re)created"
            );
        }

        drop(ed_state_ref);

        if let Some((view, w, h)) = new_scene_target {
            world.insert_resource(gizmo_renderer::components::EditorRenderTarget(
                gizmo_renderer::components::RenderTarget {
                    view,
                    width: w,
                    height: h,
                },
            ));
        }
        if let Some((view, w, h)) = new_game_target {
            world.insert_resource(gizmo_renderer::components::GameRenderTarget(
                gizmo_renderer::components::RenderTarget {
                    view,
                    width: w,
                    height: h,
                },
            ));
        }
    }
}

/// Services the editor's scene save / load / clear requests for this frame.
///
/// Polls the async file-dialog channel and promotes a chosen path into a
/// save/load request, then drains the `EditorState` scene requests, performing
/// the actual `SceneData` save/load (with the scripting components registered)
/// and despawning the previous scene's non-editor entities on clear/load.
#[tracing::instrument(skip_all, name = "editor_scene_requests")]
pub fn process_scene_requests(world: &mut World) {
    // --- EDITOR SCENE REQUESTS ---
    // 1. Poll the file-dialog channel and promote result to save/load request.
    let maybe_dialog_result = {
        let mut st = world.get_resource_mut::<gizmo_editor::EditorState>();
        if let Some(ref mut ed) = st {
            if let Some(rx_mutex) = ed.pending_dialog_rx.take() {
                match rx_mutex.into_inner() {
                    Ok(rx) => match rx.try_recv() {
                        Ok((is_save, Some(path))) => Some((is_save, Some(path))),
                        Ok((_, None)) => None, // dialog dismissed
                        Err(std::sync::mpsc::TryRecvError::Empty) => {
                            // still waiting — put it back
                            ed.pending_dialog_rx = Some(std::sync::Mutex::new(rx));
                            None
                        }
                        // Empty is handled above, so this is Disconnected: the file
                        // dialog thread dropped its sender without a result.
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            tracing::debug!(
                                "[Editor] file-dialog channel disconnected (no path chosen)"
                            );
                            None
                        }
                    },
                    // The dialog channel mutex was poisoned by a panicking thread.
                    Err(e) => {
                        tracing::warn!(error = %e, "[Editor] file-dialog channel mutex poisoned");
                        None
                    }
                }
            } else {
                None
            }
        } else {
            None
        }
    };
    if let Some((is_save, Some(path))) = maybe_dialog_result {
        if let Some(mut ed) = world.get_resource_mut::<gizmo_editor::EditorState>() {
            ed.scene_path = path.clone();
            if is_save {
                ed.scene.save_request = Some(path);
            } else {
                ed.scene.load_request = Some(path);
            }
        }
    }

    // 2. Extract requests before borrowing world mutably.
    let (save_req, load_req, clear_req) = {
        if let Some(mut ed) = world.get_resource_mut::<gizmo_editor::EditorState>() {
            (
                ed.scene.save_request.take(),
                ed.scene.load_request.take(),
                std::mem::replace(&mut ed.scene.clear_request, false),
            )
        } else {
            (None, None, false)
        }
    };

    // 3. Save
    if let Some(ref path) = save_req {
        // One assembled registry, not a hand-rolled one per call site — lights, cameras and
        // audio emitters were silently absent from every one of these before.
        let registry = crate::scene_registry::full_scene_registry();
        match gizmo_scene::scene::SceneData::save(world, path, &registry) {
            Ok(()) => {
                if let Some(mut ed) = world.get_resource_mut::<gizmo_editor::EditorState>() {
                    ed.has_unsaved_changes = false;
                    ed.status_message = format!("Kaydedildi: {}", path);
                }
                tracing::info!(scene = %path, "[Editor] scene saved");
            }
            Err(e) => tracing::error!(scene = %path, error = %e, "[Editor] scene save failed"),
        }
    }

    // 4. Clear + Load
    if clear_req || load_req.is_some() {
        let editor_entities: std::collections::HashSet<u32> = {
            let names = world.borrow::<gizmo_core::EntityName>();
            names
                .iter()
                .filter_map(|(id, _)| {
                    names.get(id).and_then(|n| {
                        if n.0.starts_with("Editor ") || n.0 == "Highlight Box" {
                            Some(id)
                        } else {
                            None
                        }
                    })
                })
                .collect()
        };
        let to_despawn: Vec<_> = world
            .iter_alive_entities()
            .into_iter()
            .filter(|e| !editor_entities.contains(&e.id()))
            .collect();
        tracing::debug!(
            despawn_count = to_despawn.len(),
            kept_editor_entities = editor_entities.len(),
            reason = if clear_req { "clear" } else { "load" },
            "[Editor] clearing scene (despawning non-editor entities)"
        );
        for e in to_despawn {
            world.despawn(e);
        }
    }
    if let Some(ref path) = load_req {
        if let Some(asset_manager) =
            world.remove_resource::<gizmo_renderer::asset::AssetManager>()
        {
            let r = world.remove_resource::<Renderer>().unwrap();
            let dummy_rgba = [255u8, 255, 255, 255];
            let _dummy_bg = r.create_texture(&dummy_rgba, 1, 1);
            let registry = crate::scene_registry::full_scene_registry();
            let load_result = gizmo_scene::scene::SceneData::load_into(path, world, &registry);
            let ok = load_result.is_ok();
            match &load_result {
                Ok(()) => tracing::info!(scene = %path, "[Editor] scene loaded"),
                Err(e) => {
                    tracing::warn!(scene = %path, error = %e, "[Editor] scene load failed")
                }
            }
            world.insert_resource(r);
            world.insert_resource(asset_manager);
            if let Some(mut ed) = world.get_resource_mut::<gizmo_editor::EditorState>() {
                ed.status_message = if ok {
                    format!("Yüklendi: {}", path)
                } else {
                    format!("Sahne yüklenemedi: {}", path)
                };
                ed.has_unsaved_changes = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pairing itself. Written as a table so that a surface format arriving later cannot
    /// quietly pick the wrong branch — and stated in both directions, because "sRGB in, raw out" is
    /// only half the contract; "raw in, raw out" is what makes the helper safe on a surface with no
    /// sRGB pair.
    #[test]
    fn egui_samples_the_raw_variant_of_whatever_the_engine_renders_into() {
        use wgpu::TextureFormat::*;
        assert_eq!(egui_sample_format(Bgra8UnormSrgb), Bgra8Unorm);
        assert_eq!(egui_sample_format(Rgba8UnormSrgb), Rgba8Unorm);
        // Already raw: nothing to reinterpret, and the two views coincide.
        assert_eq!(egui_sample_format(Bgra8Unorm), Bgra8Unorm);
        assert_eq!(egui_sample_format(Rgba8Unorm), Rgba8Unorm);
        // No sRGB pair at all — must be a no-op rather than a wrong guess.
        assert_eq!(egui_sample_format(Rgba16Float), Rgba16Float);

        assert_ne!(
            egui_sample_format(Bgra8UnormSrgb),
            Bgra8UnormSrgb,
            "on the format this engine actually gets, the sampled view MUST differ from the \
             rendered one — equal views are the bug: the viewport renders one gamma step too dark"
        );
    }

    /// The guard that outlives the fix: egui may only be handed a viewport texture through
    /// [`create_viewport_target`].
    ///
    /// Scans rather than listing files. The bug existed twice — the scene viewport and the game
    /// viewport, each with its own copy of the same twenty lines — so the thing worth policing is
    /// not those two call sites but the appearance of a third. A future panel that registers its
    /// own render target with a default (sRGB) view reintroduces exactly this defect, and it looks
    /// completely reasonable while doing it.
    #[test]
    fn nothing_registers_an_egui_texture_outside_the_viewport_helper() {
        let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .expect("crates/gizmo-app sits two levels below the workspace root")
            .to_path_buf();
        if !workspace.join("crates/gizmo-studio").is_dir() {
            return; // Packaged crate, not a workspace checkout.
        }

        let mut sources = Vec::new();
        collect_rs_files(&workspace.join("crates"), &mut sources);
        collect_rs_files(&workspace.join("demo"), &mut sources);
        assert!(sources.len() > 100, "source walk found only {} files", sources.len());

        let this_file = std::path::Path::new(file!()).file_name().unwrap();
        let mut offenders = Vec::new();
        for path in &sources {
            if path.file_name() == Some(this_file) {
                continue;
            }
            let text = std::fs::read_to_string(path).unwrap_or_default();
            for (i, line) in text.lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                if line.contains("register_native_texture(") {
                    offenders.push(format!("{}:{}", path.display(), i + 1));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "egui viewport textures must be created by `create_viewport_target` in \
             editor_runtime.rs, which hands egui the non-sRGB view. Registering a default view \
             renders that panel one gamma step too dark. Offenders:\n{}",
            offenders.join("\n")
        );

        // And the helper itself must still be handing over the reinterpreted view, not the
        // attachment — the one-word edit that would silently undo all of this.
        //
        // Only the code ABOVE `#[cfg(test)]` is searched. Scanning the whole file makes the check
        // vacuous, because the string being searched for also appears in this assertion: the first
        // version of this test passed with the defect reintroduced, which is how that was found.
        let me =
            std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/editor_runtime.rs"))
                .unwrap();
        let code = me
            .split_once("#[cfg(test)]")
            .expect("this file has a test module")
            .0;
        assert!(
            code.contains("register_native_texture(device, &sample_view"),
            "the helper must register the non-sRGB `sample_view`, not the render attachment"
        );
        assert!(
            !code.contains("register_native_texture(device, &render_view"),
            "registering the sRGB render view is the defect this whole module documents"
        );
    }

    fn collect_rs_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                collect_rs_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
}
