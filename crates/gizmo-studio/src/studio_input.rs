use gizmo::editor::EditorState;
use gizmo::math::{Ray, Vec3};
use gizmo::prelude::*;

pub fn handle_studio_input(
    world: &mut World,
    state: &mut EditorState,
    ray: Ray,
    player_id: u32,
    do_raycast: bool,
    ctrl_pressed: bool,
) {
    if do_raycast {
        perform_raycast(world, state, ray, player_id, ctrl_pressed);
    }

    if let Some((start, end)) = state.selection.rubber_band_request.take() {
        perform_rubber_band_selection(world, state, start, end, player_id, ctrl_pressed);
    }
}

fn perform_rubber_band_selection(
    world: &mut World,
    state: &mut EditorState,
    start: gizmo::math::Vec2,
    end: gizmo::math::Vec2,
    player_id: u32,
    ctrl_pressed: bool,
) {
    if state.camera.view.is_none() || state.camera.proj.is_none() || state.scene_view_rect.is_none()
    {
        return;
    }

    let view_mat = state.camera.view.unwrap();
    let proj_mat = state.camera.proj.unwrap();
    let vp_mat = proj_mat * view_mat;

    // Egui tiplerine dokunmadan koordinatlari aliyoruz. scene_view_rect bir egui::Rect
    let rect_left = state.scene_view_rect.unwrap().min.x;
    let rect_top = state.scene_view_rect.unwrap().min.y;
    let rect_width = state.scene_view_rect.unwrap().max.x - rect_left;
    let rect_height = state.scene_view_rect.unwrap().max.y - rect_top;

    let min_x = start.x.min(end.x);
    let max_x = start.x.max(end.x);
    let min_y = start.y.min(end.y);
    let max_y = start.y.max(end.y);

    if !ctrl_pressed {
        state.selection.entities.clear();
    }

    let transforms = world.borrow::<Transform>();
    {
        let is_hidden = world.borrow::<gizmo::core::component::IsHidden>();

        for (id, t) in transforms.iter() {
            if id == player_id || state.selection.highlight_box.map(|h| h.id()) == Some(id) {
                continue;
            }

            let hidden = &is_hidden;
            if hidden.contains(id) {
                continue;
            }

            let clip_pos =
                vp_mat * gizmo::math::Vec4::new(t.position.x, t.position.y, t.position.z, 1.0);

            // Kamera arkasindaysa atla
            if clip_pos.w <= 0.0 {
                continue;
            }

            let ndc = gizmo::math::Vec3::new(clip_pos.x, clip_pos.y, clip_pos.z) / clip_pos.w;

            // NDC'yi Screen koordinatina cevir
            let screen_x = ((ndc.x + 1.0) / 2.0) * rect_width + rect_left;
            let screen_y = ((1.0 - ndc.y) / 2.0) * rect_height + rect_top;

            // Dörtgen icinde kalip kalmadigini kontrol et
            if screen_x >= min_x && screen_x <= max_x && screen_y >= min_y && screen_y <= max_y {
                state
                    .selection
                    .entities
                    .insert(gizmo::prelude::Entity::new(id, 0));
            }
        }
    }
}

fn perform_raycast(
    world: &mut World,
    state: &mut EditorState,
    ray: Ray,
    player_id: u32,
    ctrl_pressed: bool,
) {
    state.do_raycast = false;

    let mut closest_t = std::f32::MAX;
    let mut hit_entity = None;

    let colliders = world.borrow::<Collider>();
    let transforms = world.borrow::<Transform>();
    {
        let is_hidden = world.borrow::<gizmo::core::component::IsHidden>();

        for (id, col) in colliders.iter() {
            // Editör objelerini, highlight box'ı es geç
            if id == player_id || state.selection.highlight_box.map(|h| h.id()) == Some(id) {
                continue;
            }
            // Gizli component'i olan objeleri tıklanabilir yapma.
            // Seçili objemiz bittiğinde Gizmo okları IsHidden alır, o yüzden tıklanmazlar.
            let hidden = &is_hidden;
            if hidden.contains(id) {
                continue;
            }

            if let Some(t) = transforms.get(id) {
                let extents = col.compute_aabb(t.position, t.rotation).half_extents();
                let scaled_half = Vec3::new(
                    extents.x * t.scale.x,
                    extents.y * t.scale.y,
                    extents.z * t.scale.z,
                );

                // Işın testi (OBB Testi)
                if let Some(hitt) = ray.intersect_obb(t.position, scaled_half, t.rotation) {
                    if hitt > 0.0 && hitt < closest_t {
                        closest_t = hitt;
                        hit_entity = Some(id);
                    }
                }
            }
        }
    }

    if let Some(hit) = hit_entity {
        if ctrl_pressed {
            state.toggle_selection(gizmo::prelude::Entity::new(hit, 0));
        } else {
            state.select_exclusive(gizmo::prelude::Entity::new(hit, 0));
        }
    } else {
        state.clear_selection();
    }
}

pub fn sync_gizmos(world: &mut World, state: &EditorState) {
    let mut any_selected = false;
    let mut selected_pos = gizmo::math::Vec3::ZERO;
    let mut selected_rot = gizmo::math::Quat::IDENTITY;
    let mut selected_scale = gizmo::math::Vec3::ONE;
    let mut selected_col = None;

    if let Some(&selected) = state.selection.entities.iter().next() {
        let transforms = world.borrow::<Transform>();
        {
            if let Some(t) = transforms.get(selected.id()) {
                any_selected = true;
                selected_pos = t.position;
                selected_rot = t.rotation;
                selected_scale = t.scale;

                let colls = world.borrow::<Collider>();
                {
                    if let Some(c) = colls.get(selected.id()) {
                        selected_col = Some(c.clone());
                    }
                }
            }
        }
    }

    if any_selected {
        // Obje seçiliyse Highlight Box pozisyonunu ve boyutunu güncelle
        {
            let mut trans = world.borrow_mut::<Transform>();
            if let Some(hb) = trans.get_mut(
                state
                    .selection
                    .highlight_box
                    .unwrap_or(gizmo::prelude::Entity::new(0, 0))
                    .id(),
            ) {
                hb.position = selected_pos;
                hb.rotation = selected_rot;

                let mut base_extents = Vec3::ONE;
                if let Some(c) = &selected_col {
                    base_extents = c.compute_aabb(selected_pos, selected_rot).half_extents() * selected_scale;
                }

                hb.scale = base_extents * 1.05; // Çerçeveyi tam objenin collision AABB bounds'una sığdır
            }
        }

        // ECS üzerinden görünür yap
        if let Some(entity_hb) = world.get_entity(
            state
                .selection
                .highlight_box
                .unwrap_or(gizmo::prelude::Entity::new(0, 0))
                .id(),
        ) {
            world.remove_component::<gizmo::core::component::IsHidden>(entity_hb);
        }
    } else {
        // Hiçbir şey seçili değilse 't.position = -10000' hack'i yerine ECS üzerinden render'ı atla
        if let Some(entity_hb) = world.get_entity(
            state
                .selection
                .highlight_box
                .unwrap_or(gizmo::prelude::Entity::new(0, 0))
                .id(),
        ) {
            world.add_component(entity_hb, gizmo::core::component::IsHidden);
        }
    }
}

pub fn build_ray(
    world: &World,
    player_id: u32,
    ndc_x: f32,
    ndc_y: f32,
    aspect: f32,
    _wh: f32,
) -> Option<Ray> {
    let transforms = world.borrow::<Transform>();
    let cameras = world.borrow::<gizmo::renderer::components::Camera>();
    {
        if let (Some(cam_t), Some(cam)) = (transforms.get(player_id), cameras.get(player_id)) {
            let view = cam.get_view(cam_t.position);
            let proj = cam.get_projection(aspect);

            let inv_vp = (proj * view).inverse();

            // WGPU'da Z=0 yakın düzlem, Z=1 uzak düzlemdir. Lazerin ekrandan çıktığı nokta Z=0'dır.
            let near_vec = inv_vp.project_point3(gizmo::math::Vec3::new(ndc_x, ndc_y, 0.0));
            let far_vec = inv_vp.project_point3(gizmo::math::Vec3::new(ndc_x, ndc_y, 1.0));

            let world_dir = (far_vec - near_vec).normalize();

            return Some(Ray {
                origin: near_vec.into(),
                direction: world_dir.into(),
            });
        }
    }
    None
}
