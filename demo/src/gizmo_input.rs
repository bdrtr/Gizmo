/// Gizmo seçim, tıklama-raycast ve sürükleme (drag) mantığı.
use gizmo::prelude::*;
use crate::{GameState, DragAxis, GizmoMode};

pub fn handle_gizmo_input(
    world: &mut World,
    state: &mut GameState,
    ray: gizmo::math::Ray,
    do_raycast: bool,
) {
    if do_raycast {
        perform_raycast(world, state, ray);
    } else if let Some(axis) = state.dragging_axis {
        perform_drag(world, state, ray, axis);
    }
}

fn perform_raycast(world: &mut World, state: &mut GameState, ray: gizmo::math::Ray) {
    state.do_raycast = false;

    let mut closest_t = std::f32::MAX;
    let mut hit_entity = None;

    if let (Some(colliders), Some(transforms)) = (world.borrow::<Collider>(), world.borrow::<Transform>()) {
        for i in 0..colliders.dense.len() {
            let id = colliders.entity_dense[i];
            if id == state.player_id || id == state.skybox_id {
                continue;
            }
            if let Some(t) = transforms.get(id) {
                if let gizmo::physics::ColliderShape::Aabb(aabb) = &colliders.dense[i].shape {
                    let scaled_half = Vec3::new(
                        aabb.half_extents.x * t.scale.x,
                        aabb.half_extents.y * t.scale.y,
                        aabb.half_extents.z * t.scale.z,
                    );
                    let min = t.position - scaled_half;
                    let max = t.position + scaled_half;
                    if let Some(hitt) = ray.intersect_aabb(min, max) {
                        if hitt > 0.0 && hitt < closest_t {
                            closest_t = hitt;
                            hit_entity = Some(id);
                        }
                    }
                }
            }
        }

        if let Some(hit) = hit_entity {
            if hit == state.gizmo_x || hit == state.gizmo_y || hit == state.gizmo_z {
                // Gizmo oku tıklandı — sürüklemeyi başlat
                if let Some(sel) = state.inspector_selected_entity {
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
            } else {
                // Normal obje tıklandı
                state.inspector_selected_entity = Some(hit);
                let mut name_str = format!("Model {}", hit);
                if let Some(names) = world.borrow::<crate::EntityName>() {
                    if let Some(n) = names.get(hit) {
                        name_str = n.0.clone();
                    }
                }
                println!("Raycast: {} seçildi!", name_str);
            }
        } else {
            state.inspector_selected_entity = None;
        }
    }
}

fn perform_drag(world: &mut World, state: &mut GameState, ray: gizmo::math::Ray, axis: DragAxis) {
    if let Some(sel) = state.inspector_selected_entity {
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
                        if let Some(t) = trans.get_mut(sel) {
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
                    if let Some(t) = trans.get_mut(sel) {
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

fn gizmo_axis_dir(hit: u32, state: &GameState) -> Vec3 {
    if hit == state.gizmo_x      { Vec3::new(1.0, 0.0, 0.0) }
    else if hit == state.gizmo_y { Vec3::new(0.0, 1.0, 0.0) }
    else                         { Vec3::new(0.0, 0.0, 1.0) }
}

fn axis_from_hit(hit: u32, state: &GameState) -> Option<DragAxis> {
    if hit == state.gizmo_x      { Some(DragAxis::X) }
    else if hit == state.gizmo_y { Some(DragAxis::Y) }
    else                         { Some(DragAxis::Z) }
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

/// Gizmo okları seçilen objeye taşır; seçilmemişse gizler.
pub fn sync_gizmos(world: &mut World, state: &GameState) {
    let target_pos = if let Some(selected) = state.inspector_selected_entity {
        world.borrow::<Transform>().and_then(|t| t.get(selected).map(|t| t.position))
    } else { None };

    let mode = state.gizmo_mode;

    if let Some(mut trans) = world.borrow_mut::<Transform>() {
        if let Some(pos) = target_pos {
            if let Some(tx) = trans.get_mut(state.gizmo_x) {
                tx.position = pos;
                tx.scale = match mode {
                    GizmoMode::Translate | GizmoMode::Scale => Vec3::new(1.5, 0.08, 0.08),
                    GizmoMode::Rotate => Vec3::new(0.05, 1.5, 1.5),
                };
            }
            if let Some(ty) = trans.get_mut(state.gizmo_y) {
                ty.position = pos;
                ty.scale = match mode {
                    GizmoMode::Translate | GizmoMode::Scale => Vec3::new(0.08, 1.5, 0.08),
                    GizmoMode::Rotate => Vec3::new(1.5, 0.05, 1.5),
                };
            }
            if let Some(tz) = trans.get_mut(state.gizmo_z) {
                tz.position = pos;
                tz.scale = match mode {
                    GizmoMode::Translate | GizmoMode::Scale => Vec3::new(0.08, 0.08, 1.5),
                    GizmoMode::Rotate => Vec3::new(1.5, 1.5, 0.05),
                };
            }
        } else {
            for id in [state.gizmo_x, state.gizmo_y, state.gizmo_z] {
                if let Some(t) = trans.get_mut(id) { t.position = Vec3::new(0.0, -1000.0, 0.0); }
            }
        }
    }

    // Collider boyutlarını da güncelle
    if target_pos.is_some() {
        if let Some(mut colls) = world.borrow_mut::<Collider>() {
            let updates = [
                (state.gizmo_x, match mode {
                    GizmoMode::Translate | GizmoMode::Scale => Vec3::new(1.5, 0.3, 0.3),
                    GizmoMode::Rotate => Vec3::new(0.1, 1.5, 1.5),
                }),
                (state.gizmo_y, match mode {
                    GizmoMode::Translate | GizmoMode::Scale => Vec3::new(0.3, 1.5, 0.3),
                    GizmoMode::Rotate => Vec3::new(1.5, 0.1, 1.5),
                }),
                (state.gizmo_z, match mode {
                    GizmoMode::Translate | GizmoMode::Scale => Vec3::new(0.3, 0.3, 1.5),
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
