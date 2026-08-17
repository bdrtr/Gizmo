use crate::state::{DebugAssets, StudioState};
use crate::studio_input;
use gizmo::editor::EditorState;
use gizmo::physics::components::Transform;
use gizmo::prelude::*;

/// Turns this frame's viewport input into picking and scene-view interactions — the click that
/// selects, the drag that rubber-bands.
pub fn handle_input_and_scene_view(
    world: &mut World,
    editor_state: &mut EditorState,
    state: &mut StudioState,
    dt: f32,
    input: &Input,
    window: &gizmo::prelude::WindowInfo,
) {
    let (ww, wh) = window.size();
    let aspect = if let Some(rect) = editor_state.scene_view_rect {
        rect.width() / rect.height()
    } else {
        ww / wh
    };

    // Kameranın görüntü/izdüşüm matrisleri HER KARE yayınlanıyor.
    //
    // Bunlar `if let Some(ndc) = mouse_ndc` bloğunun içindeydi — yani yalnız fare sahne
    // viewport'unun ÜSTÜNDEYKEN. Işın atmak için doğru (fare yoksa ışın da yok), ama matrisler bir
    // fare olayı değil, kameranın o kareki hâli: onları okuyan başka herkes (viewport'un eksen
    // gizmo'su, kutuyla seçim) fare içeri girene kadar `None` görüyordu. Eksen gizmo'su bu yüzden
    // ilk karelerde hiç çizilmiyordu ve kusur ekran görüntüsüyle ölçüldü.
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

    // Editör Scene View üzerinden gelen NDC ve raycast tetiğini okuyalım
    if let Some(ndc) = editor_state.mouse_ndc {
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

#[cfg(test)]
mod tests {
    /// The editor camera's view/projection matrices must be published **every frame**, not only on
    /// the frames where the pointer happens to be over the scene viewport.
    ///
    /// They used to be written inside `if let Some(ndc) = editor_state.mouse_ndc`, which is the
    /// right gate for casting a picking ray and the wrong one for the matrices: the viewport's axis
    /// gizmo reads them to know which way the camera is facing, and it simply did not draw until
    /// the mouse first entered the viewport. That is how the defect was found — a screenshot of a
    /// freshly launched studio has no pointer in it.
    ///
    /// This reads the source rather than running the system: `handle_input_and_scene_view` wants a
    /// `World`, a fully built `StudioState` (asset watcher, camera entities, debug meshes) and a
    /// `WindowInfo`, and standing all of that up would test the fixture, not the ordering. What can
    /// break here is precisely the ordering, so that is what is pinned.
    #[test]
    fn the_camera_matrices_are_published_outside_the_pointer_branch() {
        let src = include_str!("input.rs");
        let code = src.split("#[cfg(test)]").next().unwrap_or("");
        // Comments talk about the branch by name; only real code counts.
        let code: String = code
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");

        let assign = code
            .find("editor_state.camera.view = Some")
            .expect("the studio must publish the editor camera's view matrix somewhere");
        let branch = code
            .find("if let Some(ndc) = editor_state.mouse_ndc")
            .expect("the picking ray is still gated on the pointer");
        assert!(
            assign < branch,
            "the camera matrices are back inside the pointer branch — everything that reads them \
             without a mouse (the axis gizmo, box select) sees None again"
        );
    }
}
