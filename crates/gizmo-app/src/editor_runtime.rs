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

/// Keeps the editor's scene/game viewport render targets sized to the panels.
///
/// When the editor's `EditorState` resource is present, this (re)creates the
/// offscreen textures the Scene View and Game View panels draw into whenever
/// their requested size changes, registers them with the egui renderer so the
/// panels can sample them, and publishes the matching `EditorRenderTarget` /
/// `GameRenderTarget` resources the engine renders into.
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
                let texture = r.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Editor RTT"),
                    size: wgpu::Extent3d {
                        width: scene_w,
                        height: scene_h,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: r.config.format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                tex_id = Some(editor.renderer.register_native_texture(
                    &r.device,
                    &view,
                    wgpu::FilterMode::Linear,
                ));
                new_scene_target = Some((std::sync::Arc::new(view), scene_w, scene_h));
            }
            ed_state_ref.scene_texture_id = tex_id;
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
                let texture = r.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Game RTT"),
                    size: wgpu::Extent3d {
                        width: game_w,
                        height: game_h,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: r.config.format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                tex_id = Some(editor.renderer.register_native_texture(
                    &r.device,
                    &view,
                    wgpu::FilterMode::Linear,
                ));
                new_game_target = Some((std::sync::Arc::new(view), game_w, game_h));
            }
            ed_state_ref.game_texture_id = tex_id;
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
                        Err(_) => None,
                    },
                    Err(_) => None,
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
        let mut registry = gizmo_scene::registry::default_scene_registry();
        #[cfg(not(target_arch = "wasm32"))]
        gizmo_scripting::register_script_components(&mut registry);
        match gizmo_scene::scene::SceneData::save(world, path, &registry) {
            Ok(()) => {
                if let Some(mut ed) = world.get_resource_mut::<gizmo_editor::EditorState>() {
                    ed.has_unsaved_changes = false;
                    ed.status_message = format!("Kaydedildi: {}", path);
                }
            }
            Err(e) => tracing::error!("[App] Sahne kayıt hatası: {}", e),
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
            let mut registry = gizmo_scene::registry::default_scene_registry();
            #[cfg(not(target_arch = "wasm32"))]
            gizmo_scripting::register_script_components(&mut registry);
            let ok = gizmo_scene::scene::SceneData::load_into(path, world, &registry).is_ok();
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
