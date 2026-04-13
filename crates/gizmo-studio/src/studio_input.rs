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
}

fn perform_raycast(world: &mut World, state: &mut EditorState, ray: Ray, player_id: u32, ctrl_pressed: bool) {
    state.do_raycast = false;

    let mut closest_t = std::f32::MAX;
    let mut hit_entity = None;

    if let (Some(colliders), Some(transforms)) =
        (world.borrow::<Collider>(), world.borrow::<Transform>())
    {
        let is_hidden = world.borrow::<gizmo::core::component::IsHidden>();

        for i in 0..colliders.dense.len() {
            let id = colliders.dense[i].entity;

            // Editör objelerini, highlight box'ı es geç
            if id == player_id || id == state.highlight_box {
                continue;
            }
            // Gizli component'i olan objeleri tıklanabilir yapma.
            // Seçili objemiz bittiğinde Gizmo okları IsHidden alır, o yüzden tıklanmazlar.
            if let Some(hidden) = &is_hidden {
                if hidden.contains(id) {
                    continue;
                }
            }

            if let Some(t) = (*transforms).get(id) {
                let extents = colliders.dense[i]
                    .data
                    .shape
                    .bounding_box_half_extents(t.rotation);
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
            state.toggle_selection(hit);
        } else {
            state.select_exclusive(hit);
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

    if let Some(&selected) = state.selected_entities.iter().next() {
        if let Some(transforms) = world.borrow::<Transform>() {
            if let Some(t) = transforms.get(selected) {
                any_selected = true;
                selected_pos = t.position;
                selected_rot = t.rotation;
                selected_scale = t.scale;

                if let Some(colls) = world.borrow::<Collider>() {
                    if let Some(c) = colls.get(selected) {
                        selected_col = Some(c.clone());
                    }
                }
            }
        }
    }

    if any_selected {
        // Obje seçiliyse Highlight Box pozisyonunu ve boyutunu güncelle
        if let Some(mut trans) = world.borrow_mut::<Transform>() {
            if let Some(hb) = (*trans).get_mut(state.highlight_box) {
                hb.position = selected_pos;
                hb.rotation = selected_rot;

                let mut base_extents = Vec3::ONE;
                if let Some(c) = &selected_col {
                    base_extents = c.shape.bounding_box_half_extents(selected_rot) * selected_scale;
                }

                hb.scale = base_extents * 1.05; // Çerçeveyi tam objenin collision AABB bounds'una sığdır
            }
        }

        // ECS üzerinden görünür yap
        if let Some(entity_hb) = world.get_entity(state.highlight_box) {
            world.remove_component::<gizmo::core::component::IsHidden>(entity_hb);
        }
    } else {
        // Hiçbir şey seçili değilse 't.position = -10000' hack'i yerine ECS üzerinden render'ı atla
        if let Some(entity_hb) = world.get_entity(state.highlight_box) {
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
    if let (Some(transforms), Some(cameras)) = (
        world.borrow::<Transform>(),
        world.borrow::<gizmo::renderer::components::Camera>(),
    ) {
        if let (Some(cam_t), Some(cam)) = (transforms.get(player_id), cameras.get(player_id)) {
            let view = cam.get_view(cam_t.position);
            let proj = cam.get_projection(aspect);

            let inv_vp = (proj * view).inverse();

            // WGPU'da Z=0 yakın düzlem, Z=1 uzak düzlemdir. Lazerin ekrandan çıktığı nokta Z=0'dır.
            let near_vec = inv_vp.project_point3(gizmo::math::Vec3::new(ndc_x, ndc_y, 0.0));
            let far_vec = inv_vp.project_point3(gizmo::math::Vec3::new(ndc_x, ndc_y, 1.0));

            let world_dir = (far_vec - near_vec).normalize();

            return Some(Ray {
                origin: near_vec,
                direction: world_dir,
            });
        }
    }
    None
}
