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
            if id == player_id {
                continue;
            }

            let hidden = &is_hidden;
            if hidden.contains(id) {
                continue;
            }

            // Editör donanımlarını (Grid, Işık vb) seçilebilir objelerden çıkar
            if let Some(name) = world.borrow::<gizmo::core::component::EntityName>().get(id) {
                if name.0 == "Editor Grid" 
                    || name.0 == "Editor Guidelines" 
                    || name.0 == "Directional Light" 
                    || name.0.starts_with("Editor Light Icon") 
                {
                    continue;
                }
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

    let transforms = world.borrow::<Transform>();
    {
        let colliders = world.borrow::<Collider>();
        let is_hidden = world.borrow::<gizmo::core::component::IsHidden>();

        for (id, t) in transforms.iter() {
            // Editör objelerini es geç
            if id == player_id {
                continue;
            }
            let hidden = &is_hidden;
            if hidden.contains(id) {
                continue;
            }

            // Editör donanımlarını (Grid, Işık vb) seçilebilir objelerden çıkar
            let mut name_str = String::new();
            if let Some(name) = world.borrow::<gizmo::core::component::EntityName>().get(id) {
                if name.0 == "Editor Grid" 
                    || name.0 == "Editor Guidelines" 
                    || name.0 == "Directional Light" 
                    || name.0.starts_with("Editor Light Icon") 
                {
                    continue;
                }
                name_str = name.0.clone();
            }

            // Objenin collider'ı varsa onun boyutunu al, yoksa standart 1x1x1 (çarpı scale) kutu farz et.
            let mut extents = Vec3::ONE;
            if let Some(col) = colliders.get(id) {
                extents = col.compute_aabb(gizmo::math::Vec3::ZERO, gizmo::math::Quat::IDENTITY).half_extents().into();
            }

            // Burada t.scale kullanımı local. Eğer obje child ise raycast yanlış yeri test edebilir!
            // Ama default cube bir child değil.
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

    if let Some(hit) = hit_entity {
        let mut name_str = format!("Entity {}", hit);
        if let Some(name) = world.borrow::<gizmo::core::component::EntityName>().get(hit) {
            name_str = name.0.clone();
        }
        state.log_info(&format!("Seçildi: {}", name_str));
        if ctrl_pressed {
            state.toggle_selection(gizmo::prelude::Entity::new(hit, 0));
        } else {
            state.select_exclusive(gizmo::prelude::Entity::new(hit, 0));
        }
    } else {
        state.log_info("Boşluğa tıklandı, seçim temizlendi.");
        state.clear_selection();
    }
}

// Removed sync_gizmos
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
