use gizmo::prelude::*;
use gizmo::editor::{EditorState, GizmoMode, DragAxis};
use gizmo::physics::ColliderShape;
use gizmo::math::{Vec3, Ray};

pub fn handle_studio_input(
    world: &mut World,
    state: &mut EditorState,
    ray: Ray,
    player_id: u32,
    do_raycast: bool,
) {
    if do_raycast {
        perform_raycast(world, state, ray, player_id);
    } else if let Some(axis) = state.dragging_axis {
        perform_drag(world, state, ray, axis);
    }
}

fn perform_raycast(world: &mut World, state: &mut EditorState, ray: Ray, player_id: u32) {
    state.do_raycast = false;

    let mut closest_t = std::f32::MAX;
    let mut hit_entity = None;
    let mut hit_entity_details = None;

    if let (Some(colliders), Some(transforms)) = (world.borrow::<Collider>(), world.borrow::<Transform>()) {
        for i in 0..colliders.dense.len() {
            let id = colliders.entity_dense[i];
            
            // Editör objelerini veya highlight box'ı es geç
            if id == player_id || id == state.highlight_box {
                continue;
            }
            
            if let Some(t) = (*transforms).get(id) {
                let extents = colliders.dense[i].shape.bounding_box_half_extents();
                
                let inv_rot = t.rotation.inverse();
                let local_ray_dir = inv_rot * ray.direction;
                let local_ray_origin = inv_rot * (ray.origin - t.position);
                let local_ray = Ray::new(local_ray_origin, local_ray_dir);
                
                let scaled_half = Vec3::new(
                    extents.x * t.scale.x,
                    extents.y * t.scale.y,
                    extents.z * t.scale.z,
                );
                let min = -scaled_half;
                let max = scaled_half;
                
                // Işın testi (Yerel Düzlemde OBB/AABB Testi)
                if let Some(hitt) = local_ray.intersect_aabb(min, max) {
                    if hitt > 0.0 && hitt < closest_t {
                        closest_t = hitt;
                        hit_entity = Some(id);
                        hit_entity_details = Some((t.clone(), scaled_half));
                    }
                }
            }
        }
    }

    // Bounding Box debug çizimi kaldırıldı, sadece lazerin değdiği noktaya minik küp çiziyoruz.
        let color = if hit_entity.is_some() { gizmo::math::Vec4::new(0.0, 1.0, 0.0, 1.0) } else { gizmo::math::Vec4::new(1.0, 0.0, 0.0, 1.0) };
        let reach = if closest_t < 1000.0 && closest_t > 0.0 { closest_t } else { 20.0 };
        
        let hit_point = ray.origin + ray.direction * reach;
        let point_scale = Vec3::new(0.05, 0.05, 0.05); 
        
        state.debug_draw_requests.push((hit_point, gizmo::math::Quat::default(), point_scale, color));

        if hit_entity.is_none() {
            println!("[CLICKED BUT MISSED] RAY Origin: {:?}, Dir: {:?}, NDC: {:?}", 
                ray.origin, ray.direction, state.mouse_ndc);
        } else {
            println!("[CLICKED AND HIT] Entity: {:?}", hit_entity);
        }

        if let Some(hit) = hit_entity {
            if hit == state.gizmo_x || hit == state.gizmo_y || hit == state.gizmo_z {
                // Gizmo tıklandı — sürüklemeyi başlat
                if let Some(sel) = state.selected_entity {
                    if let Some(transforms) = world.borrow::<Transform>() {
                        if let Some(t) = transforms.get(sel) {
                            state.drag_original_pos   = t.position;
                            state.drag_original_scale = t.scale;
                            state.drag_original_rot   = t.rotation;

                            let axis_dir = gizmo_axis_dir(hit, state);

                            if state.gizmo_mode == GizmoMode::Rotate {
                                let denom = ray.direction.dot(axis_dir);
                                if denom.abs() > 0.0001 {
                                    let plane_t = (t.position - ray.origin).dot(axis_dir) / denom;
                                    if plane_t >= 0.0 {
                                        let intersection = ray.origin + ray.direction * plane_t;
                                        let local_hit = intersection - t.position;
                                        let (u, v) = tangent_basis(axis_dir);
                                        state.drag_start_t = local_hit.dot(v).atan2(local_hit.dot(u));
                                        state.dragging_axis = axis_from_hit(hit, state);
                                    }
                                }
                            } else {
                                let w0    = ray.origin - t.position;
                                let b     = ray.direction.dot(axis_dir);
                                let d     = ray.direction.dot(w0);
                                let e     = axis_dir.dot(w0);
                                let denom = 1.0 - b * b;
                                if denom.abs() > 0.0001 {
                                    state.drag_start_t  = (e - b * d) / denom;
                                    state.dragging_axis = axis_from_hit(hit, state);
                                }
                            }
                        }
                    }
                }
            } else {
                // Normal obje tıklandı
                state.selected_entity = Some(hit);
            }
        } else {
            // Boşa tıklandı
            state.selected_entity = None;
        }
    }

fn perform_drag(world: &mut World, state: &mut EditorState, ray: Ray, axis: DragAxis) {
    if let Some(sel) = state.selected_entity {
        let axis_dir = match axis {
            DragAxis::X => Vec3::new(1.0, 0.0, 0.0),
            DragAxis::Y => Vec3::new(0.0, 1.0, 0.0),
            DragAxis::Z => Vec3::new(0.0, 0.0, 1.0),
        };

        if state.gizmo_mode == GizmoMode::Rotate {
            let denom = ray.direction.dot(axis_dir);
            if denom.abs() > 0.0001 {
                let plane_t_val = (state.drag_original_pos - ray.origin).dot(axis_dir) / denom;
                if plane_t_val >= 0.0 {
                    let intersection = ray.origin + ray.direction * plane_t_val;
                    let local_hit    = intersection - state.drag_original_pos;
                    let (u, v) = tangent_basis(axis_dir);
                    let current_angle = local_hit.dot(v).atan2(local_hit.dot(u));
                    let delta_angle   = current_angle - state.drag_start_t;
                    let rot_delta = gizmo::math::Quat::from_axis_angle(axis_dir, delta_angle);
                    if let Some(mut trans) = world.borrow_mut::<Transform>() {
                        if let Some(t) = (*trans).get_mut(sel) {
                            t.rotation = rot_delta * state.drag_original_rot;
                        }
                    }
                }
            }
        } else {
            let w0    = ray.origin - state.drag_original_pos;
            let b     = ray.direction.dot(axis_dir);
            let d     = ray.direction.dot(w0);
            let e     = axis_dir.dot(w0);
            let denom = 1.0 - b * b;
            if denom.abs() > 0.0001 {
                let current_t = (e - b * d) / denom;
                let delta_t   = current_t - state.drag_start_t;
                if let Some(mut trans) = world.borrow_mut::<Transform>() {
                    if let Some(t) = (*trans).get_mut(sel) {
                        if state.gizmo_mode == GizmoMode::Translate {
                            t.position = state.drag_original_pos + axis_dir * delta_t;
                        } else if state.gizmo_mode == GizmoMode::Scale {
                            let mut new_scale = state.drag_original_scale + axis_dir * delta_t;
                            new_scale.x = new_scale.x.max(0.01);
                            new_scale.y = new_scale.y.max(0.01);
                            new_scale.z = new_scale.z.max(0.01);
                            t.scale = new_scale;
                        }
                    }
                }
            }
        }
    }
}

// ---- Yardımcılar ----

fn gizmo_axis_dir(hit: u32, state: &EditorState) -> Vec3 {
    if hit == state.gizmo_x      { Vec3::new(1.0, 0.0, 0.0) }
    else if hit == state.gizmo_y { Vec3::new(0.0, 1.0, 0.0) }
    else                         { Vec3::new(0.0, 0.0, 1.0) }
}

fn axis_from_hit(hit: u32, state: &EditorState) -> Option<DragAxis> {
    if hit == state.gizmo_x      { Some(DragAxis::X) }
    else if hit == state.gizmo_y { Some(DragAxis::Y) }
    else if hit == state.gizmo_z { Some(DragAxis::Z) }
    else                         { None }
}

fn tangent_basis(axis: Vec3) -> (Vec3, Vec3) {
    let u = if axis.x.abs() < 0.9 {
        Vec3::new(1.0, 0.0, 0.0).cross(axis).normalize()
    } else {
        Vec3::new(0.0, 1.0, 0.0).cross(axis).normalize()
    };
    let v = axis.cross(u);
    (u, v)
}

pub fn sync_gizmos(world: &mut World, state: &EditorState) {
    let target_trans = if let Some(selected) = state.selected_entity {
        world.borrow::<Transform>().and_then(|t| t.get(selected).cloned())
    } else { None };

    let mode = state.gizmo_mode;

    // 1. GIZMO Oklari Guncelle
    if let Some(mut trans) = world.borrow_mut::<Transform>() {
        if let Some(t) = target_trans {
            if let Some(tx) = (*trans).get_mut(state.gizmo_x) {
                let s_len = 1.5;
                tx.scale = match mode {
                    GizmoMode::Translate | GizmoMode::Scale => Vec3::new(s_len, s_len, s_len),
                    GizmoMode::Rotate => Vec3::new(0.05, s_len, s_len),
                };
                tx.position = t.position;
            }
            if let Some(ty) = (*trans).get_mut(state.gizmo_y) {
                let s_len = 1.5;
                ty.scale = match mode {
                    GizmoMode::Translate | GizmoMode::Scale => Vec3::new(s_len, s_len, s_len),
                    GizmoMode::Rotate => Vec3::new(s_len, 0.05, s_len),
                };
                ty.position = t.position;
            }
            if let Some(tz) = (*trans).get_mut(state.gizmo_z) {
                let s_len = 1.5;
                tz.scale = match mode {
                    GizmoMode::Translate | GizmoMode::Scale => Vec3::new(s_len, s_len, s_len),
                    GizmoMode::Rotate => Vec3::new(s_len, s_len, 0.05),
                };
                tz.position = t.position;
            }
            // 2. Highlight Box Güncellemesi (Tıklanan objenin etrafındaki transparan çerçeve)
            if let Some(hb) = (*trans).get_mut(state.highlight_box) {
                hb.position = t.position;
                hb.rotation = t.rotation;

                // Hit details kullanarak extents'i alalım veya kendimiz hesaplayalım.
                let mut base_extents = Vec3::ONE;
                if let Some(details) = state.hit_entity_details.clone() {
                    let (_, e) = details;
                    base_extents = e;
                } else {
                    // CUBUN boyutu yoksa UI'dan seçilmiş olabilir. Collider'dan Extents'i çekelim!
                    if let Some(colls) = world.borrow::<gizmo::physics::components::Collider>() {
                        if let Some(c) = colls.get(selected) {
                            base_extents = c.shape.bounding_box_half_extents() * t.scale;
                        }
                    }
                }
                
                hb.scale = base_extents * 1.05; // Çerçeveyi tam objenin collision AABB bounds'una sığdır
            }
        } else {
            // Hiçbir şey seçili değilse uzağa sakla
            for id in [state.gizmo_x, state.gizmo_y, state.gizmo_z, state.highlight_box] {
                if let Some(t) = (*trans).get_mut(id) { t.position = Vec3::new(0.0, -10000.0, 0.0); }
            }
        }
    }

    // 3. Gizmo eksenlerinin taranabilir boyutlarını (Collider) ayarla
    if target_trans.is_some() {
        if let Some(mut colls) = world.borrow_mut::<Collider>() {
            let updates = [
                (state.gizmo_x, match mode {
                    GizmoMode::Translate | GizmoMode::Scale => Vec3::new(0.5, 3.0, 3.0),
                    GizmoMode::Rotate => Vec3::new(0.1, 1.5, 1.5),
                }),
                (state.gizmo_y, match mode {
                    GizmoMode::Translate | GizmoMode::Scale => Vec3::new(3.0, 0.5, 3.0),
                    GizmoMode::Rotate => Vec3::new(1.5, 0.1, 1.5),
                }),
                (state.gizmo_z, match mode {
                    GizmoMode::Translate | GizmoMode::Scale => Vec3::new(3.0, 3.0, 0.5),
                    GizmoMode::Rotate => Vec3::new(1.5, 1.5, 0.1),
                }),
            ];
            for (id, half) in updates {
                if let Some(c) = colls.get_mut(id) {
                    if let ColliderShape::Aabb(ref mut aabb) = c.shape {
                        aabb.half_extents = half;
                    }
                }
            }
        }
    }
}

pub fn build_ray(world: &World, player_id: gizmo::core::ecs::EntityId, ndc_x: f32, ndc_y: f32, aspect: f32, _wh: f32) -> Option<Ray> {
    if let (Some(transforms), Some(cameras)) = (world.borrow::<Transform>(), world.borrow::<gizmo::renderer::components::Camera>()) {
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
