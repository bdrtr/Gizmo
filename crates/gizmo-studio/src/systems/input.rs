use crate::state::{DebugAssets, StudioState};
use crate::studio_input;
use gizmo::editor::EditorState;
use gizmo::physics::components::Transform;
use gizmo::physics::shape::Collider;
use gizmo::prelude::*;

pub fn handle_input_and_scene_view(world: &mut World, editor_state: &mut EditorState, state: &mut StudioState, dt: f32, input: &Input, window: &gizmo::prelude::WindowInfo) {
        // Editör Scene View üzerinden gelen NDC ve raycast tetiğini okuyalım
        if let Some(ndc) = editor_state.mouse_ndc {
            let (ww, wh) = window.size();
            let aspect = if let Some(rect) = editor_state.scene_view_rect {
                rect.width() / rect.height()
            } else {
                ww / wh
            };

            if let (Ok(transforms), Ok(cameras)) = (
                world.borrow::<Transform>(),
                world.borrow::<gizmo::renderer::components::Camera>(),
            ) {
                if let (Some(t), Some(cam)) = (
                    transforms.get(state.editor_camera),
                    cameras.get(state.editor_camera),
                ) {
                    editor_state.camera.view = Some(cam.get_view(t.position));
                    editor_state.camera.proj = Some(cam.get_projection(aspect));
                }
            }

            let current_ray =
                studio_input::build_ray(world, state.editor_camera, ndc.x, ndc.y, aspect, 1.0);
            if let Some(ray) = current_ray {
                let do_rc = editor_state.do_raycast;
                if do_rc {
                    editor_state.do_raycast = false;
                    state.do_raycast = false;
                }
                
                let ctrl_pressed = input.is_key_pressed(gizmo::winit::keyboard::KeyCode::ControlLeft as u32) 
                                || input.is_key_pressed(gizmo::winit::keyboard::KeyCode::ControlRight as u32);
                                
                studio_input::handle_studio_input(
                    world,
                    editor_state,
                    ray,
                    state.editor_camera,
                    do_rc,
                    ctrl_pressed,
                );
            }
        }

        studio_input::sync_gizmos(world, &editor_state);

        // GIZMO DEBUG RENDERER: Spawn and Despawn logic
        // Zamanlayıcısı dolanları sil
        let mut surviving_entities = Vec::new();
        for (timer, ent) in editor_state.debug_spawned_entities.drain(..) {
            if timer - dt > 0.0 {
                surviving_entities.push((timer - dt, ent));
            } else {
                world.despawn_by_id(ent);
            }
        }
        editor_state.debug_spawned_entities = surviving_entities;

        // Yeni debug istekleri spawnla
        if !editor_state.debug_draw_requests.is_empty() {
            let mut pending_debug_assets = None;
            if let Ok(Some(debug_assets)) = world.get_resource::<DebugAssets>() {
                pending_debug_assets =
                    Some((debug_assets.cube.clone(), debug_assets.white_tex.clone()));
            }

            if let Some((cube, white_tex)) = pending_debug_assets {
                let reqs = std::mem::take(&mut editor_state.debug_draw_requests);
                for (pos, rot, scale, color) in reqs {
                    let e = world.spawn();
                    world
                        .add_component(e, Transform::new(pos).with_rotation(rot).with_scale(scale));
                    world.add_component(e, cube.clone());
                    let mut mat =
                        gizmo::prelude::Material::new(white_tex.clone()).with_unlit(color);
                    if color.w < 0.99 {
                        mat = mat.with_transparent(true);
                    }
                    world.add_component(e, mat);
                    world.add_component(e, gizmo::renderer::components::MeshRenderer::new());
                    editor_state.debug_spawned_entities.push((2.0, e.id())); // 2 saniye kalsın
                }
            } else {
                editor_state.debug_draw_requests.clear();
            }
        }

        // Asset browser sürükle bırak spawn işlemi
        if let Some(asset_path) = editor_state.spawn_asset_request.take() {
            let mut final_pos = None;
            if let Some(ndc) = editor_state.spawn_asset_position {
                let (ww, wh) = window.size();
                let aspect = if let Some(rect) = editor_state.scene_view_rect {
                    rect.width() / rect.height()
                } else {
                    ww / wh
                };

                if let Some(ray) =
                    studio_input::build_ray(world, state.editor_camera, ndc.x, ndc.y, aspect, 1.0)
                {
                    // Raycast yap (Gizmo'ları yoksayarak)
                    let mut closest_t = std::f32::MAX;
                    if let (Ok(colliders), Ok(transforms)) =
                        (world.borrow::<Collider>(), world.borrow::<Transform>())
                    {
                        for (id, col) in colliders.iter() {
                            if id == state.editor_camera || Some(gizmo::prelude::Entity::new(id, 0)) == editor_state.selection.highlight_box {
                                continue;
                            }

                            if let Some(t) = transforms.get(id) {
                                let extents = col
                                    .shape
                                    .bounding_box_half_extents(t.rotation);
                                let scaled_half = gizmo::math::Vec3::new(
                                    extents.x * t.scale.x,
                                    extents.y * t.scale.y,
                                    extents.z * t.scale.z,
                                );

                                if let Some(hitt) =
                                    ray.intersect_obb(t.position, scaled_half, t.rotation)
                                {
                                    if hitt > 0.0 && hitt < closest_t {
                                        closest_t = hitt;
                                    }
                                }
                            }
                        }
                    }

                    if closest_t < std::f32::MAX {
                        final_pos = Some((ray.origin + ray.direction * closest_t).into());
                    } else {
                        // Basit bir Z=0 / Y=0 zemin kesişimi yapalım
                        if ray.direction.y < -0.0001 {
                            let t = -ray.origin.y / ray.direction.y;
                            final_pos = Some((ray.origin + ray.direction * t).into());
                        } else {
                            // Işık yukarı bakıyorsa 15 birim öteye atalım
                            final_pos = Some((ray.origin + ray.direction * 15.0).into());
                        }
                    }
                }
            }

            if asset_path.ends_with(".prefab") {
                editor_state.prefab_load_request = Some((asset_path, None, final_pos));
            } else if asset_path.ends_with(".gizmo") {
                editor_state.scene.load_request = Some(asset_path);
            } else if asset_path.ends_with(".glb") || asset_path.ends_with(".gltf") || asset_path.ends_with(".obj") {
                editor_state.gltf_load_request = Some((asset_path, final_pos));
            } else {
                editor_state.log_error(&format!(
                    "Desteklenmeyen dosya türü. Sadece Prefab, Sahne veya 3D Modeller eklenebilir: {}",
                    asset_path
                ));
            }
        }
}
