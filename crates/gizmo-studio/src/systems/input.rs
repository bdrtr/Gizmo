use crate::state::{DebugAssets, StudioState};
use crate::studio_input;
use gizmo::editor::EditorState;
use gizmo::physics::components::Transform;
use gizmo::prelude::*;

pub fn handle_input_and_scene_view(
    world: &mut World,
    editor_state: &mut EditorState,
    state: &mut StudioState,
    dt: f32,
    input: &Input,
    window: &gizmo::prelude::WindowInfo,
) {
    // Editör Scene View üzerinden gelen NDC ve raycast tetiğini okuyalım
    if let Some(ndc) = editor_state.mouse_ndc {
        let (ww, wh) = window.size();
        let aspect = if let Some(rect) = editor_state.scene_view_rect {
            rect.width() / rect.height()
        } else {
            ww / wh
        };

        {
            let transforms = world.borrow::<Transform>();
            let cameras = world.borrow::<gizmo::renderer::components::Camera>();
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

            let ctrl_pressed = input
                .is_key_pressed(gizmo::winit::keyboard::KeyCode::ControlLeft as u32)
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
        if let Some(debug_assets) = world.get_resource::<DebugAssets>() {
            pending_debug_assets =
                Some((debug_assets.cube.clone(), debug_assets.white_tex.clone()));
        }

        if let Some((cube, white_tex)) = pending_debug_assets {
            let reqs = std::mem::take(&mut editor_state.debug_draw_requests);
            for (pos, rot, scale, color) in reqs {
                let e = world.spawn();
                world.add_component(e, Transform::new(pos).with_rotation(rot).with_scale(scale));
                world.add_component(e, cube.clone());
                let mut mat = gizmo::prelude::Material::new(white_tex.clone()).with_unlit(color);
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
        let final_pos = editor_state.spawn_asset_position;

        let lower_path = asset_path.to_lowercase();
        if lower_path.ends_with(".prefab") {
            editor_state.prefab_load_request = Some((asset_path, None, final_pos));
        } else if lower_path.ends_with(".gizmo") {
            editor_state.scene.load_request = Some(asset_path);
        } else if lower_path.ends_with(".glb")
            || lower_path.ends_with(".gltf")
            || lower_path.ends_with(".obj")
        {
            editor_state.gltf_load_request = Some((asset_path, final_pos));
        } else {
            editor_state.log_error(&format!(
                "Desteklenmeyen dosya türü. Sadece Prefab, Sahne veya 3D Modeller eklenebilir: {}",
                asset_path
            ));
        }
    }
}
