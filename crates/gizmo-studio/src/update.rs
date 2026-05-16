use crate::state::StudioState;
use gizmo::editor::EditorState;
use gizmo::prelude::*;

pub fn update_studio(world: &mut World, state: &mut StudioState, dt: f32, input: &Input) {
    state.current_fps = 1.0 / dt;
    state.actual_dt = dt;

    let mut look_delta = None;
    let mut pan_delta = None;
    let mut orbit_delta = None;
    let mut scroll_delta = None;
    world.resource_scope(|world, editor_state: &mut EditorState| {
        look_delta = editor_state.camera.look_delta;
        pan_delta = editor_state.camera.pan_delta;
        orbit_delta = editor_state.camera.orbit_delta;
        scroll_delta = editor_state.camera.scroll_delta;

        let win_info = world
            .get_resource::<WindowInfo>()
            .map(|w| *w)
            .unwrap_or_default();
        crate::systems::input::handle_input_and_scene_view(
            world,
            editor_state,
            state,
            dt,
            input,
            &win_info,
        );
        crate::systems::build::handle_build_requests(editor_state);
        crate::systems::shortcuts::handle_editor_shortcuts(world, editor_state, state, input);
        crate::systems::simulation::handle_simulation(world, editor_state, state, dt, input);
        crate::systems::scene_ops::handle_scene_operations(world, editor_state, state);

        // Garbage Collection & Auto-Save (her frame kontrol, belirli aralıklarla çalışır)
        crate::systems::gc::garbage_collection_system(world, state, editor_state, dt);
    });
    // İşletim sistemleri (Async Asset Server ve Transform senkronizasyonları)
    crate::systems::gizmos::update_editor_gizmos(world, state);

    // Resolve all Transform hierarchy
    let mut transform_sync = gizmo::systems::transform::TransformSyncSystem;
    let mut transform_propagate = gizmo::systems::transform::TransformPropagateSystem;
    gizmo::core::system::System::run(&mut transform_sync, world, dt);
    gizmo::core::system::System::run(&mut transform_propagate, world, dt);

    // ─── OBB HIGHLIGHT: Seçili entity'lerin turuncu kutu çizimi ───
    // Strateji: Tüm çocuk mesh'lerin bounds'larını LOCAL SPACE'de birleştir,
    // sonra birleşik AABB'nin 8 köşesini ortak GT matrisiyle world'e dönüştür.
    // Bu sayede kutu modelle birlikte döner (OBB - Oriented Bounding Box).
    if let Some(mut gizmos) = world.get_resource_mut::<gizmo::renderer::Gizmos>() {
        gizmos.depth_test = false;

        let meshes = world.borrow::<gizmo::renderer::components::Mesh>();
        let global_transforms = world.borrow::<gizmo::physics::components::GlobalTransform>();
        let children_comp = world.borrow::<gizmo::core::component::Children>();
        let Some(editor_state) = world.get_resource::<gizmo::editor::EditorState>() else { return; };

        const ORANGE: [f32; 4] = [1.0, 0.5, 0.0, 1.0];
        const EDGES: [(usize, usize); 12] = [
            (0,1),(1,2),(2,3),(3,0),
            (4,5),(5,6),(6,7),(7,4),
            (0,4),(1,5),(2,6),(3,7),
        ];

        for &selected_entity in editor_state.selection.entities.iter() {
            // BFS: Bu entity + tüm torunları
            let mut descendants = vec![selected_entity];
            let mut i = 0;
            while i < descendants.len() {
                if let Some(children) = children_comp.get(descendants[i].id()) {
                    for &child_id in &children.0 {
                        if let Some(child_ent) = world.get_entity(child_id) {
                            descendants.push(child_ent);
                        }
                    }
                }
                i += 1;
            }

            // İlk mesh'li entity'nin GT matrisini "referans frame" olarak al.
            // Diğer çocukların bounds'larını bu frame'e dönüştürüp local-space
            // merged AABB oluştur. Sonra bu AABB'yi referans GT ile world'e taşı.
            let mut ref_gt = gizmo::math::Mat4::IDENTITY;
            let mut ref_gt_inv = gizmo::math::Mat4::IDENTITY;
            let mut local_min = gizmo::math::Vec3A::splat(f32::MAX);
            let mut local_max = gizmo::math::Vec3A::splat(f32::NEG_INFINITY);
            let mut has_mesh = false;

            for ent in &descendants {
                let (mesh, gt) = match (meshes.get(ent.id()), global_transforms.get(ent.id())) {
                    (Some(m), Some(g)) => (m, g),
                    _ => continue,
                };

                if !has_mesh {
                    // İlk mesh entity → referans frame
                    ref_gt = gt.matrix;
                    ref_gt_inv = gt.matrix.inverse();
                    has_mesh = true;
                }

                // Bu entity'nin çocuk-GT'sinin referans GT'ye göre farkı
                let relative = ref_gt_inv * gt.matrix;

                let (bmin, bmax) = (mesh.bounds.min, mesh.bounds.max);
                // 8 köşeyi referans frame'e dönüştür
                let corners = [
                    gizmo::math::Vec3::new(bmin.x, bmin.y, bmin.z),
                    gizmo::math::Vec3::new(bmax.x, bmin.y, bmin.z),
                    gizmo::math::Vec3::new(bmax.x, bmax.y, bmin.z),
                    gizmo::math::Vec3::new(bmin.x, bmax.y, bmin.z),
                    gizmo::math::Vec3::new(bmin.x, bmin.y, bmax.z),
                    gizmo::math::Vec3::new(bmax.x, bmin.y, bmax.z),
                    gizmo::math::Vec3::new(bmax.x, bmax.y, bmax.z),
                    gizmo::math::Vec3::new(bmin.x, bmax.y, bmax.z),
                ];
                for c in &corners {
                    let v = relative * gizmo::math::Vec4::new(c.x, c.y, c.z, 1.0);
                    let lp = gizmo::math::Vec3A::new(v.x, v.y, v.z);
                    local_min = local_min.min(lp);
                    local_max = local_max.max(lp);
                }
            }



            if !has_mesh {
                continue;
            }

            // Merged local AABB'nin 8 köşesini referans GT ile world'e taşı → OBB!
            let lmin = gizmo::math::Vec3::new(local_min.x, local_min.y, local_min.z);
            let lmax = gizmo::math::Vec3::new(local_max.x, local_max.y, local_max.z);
            let local_corners = [
                gizmo::math::Vec3::new(lmin.x, lmin.y, lmin.z),
                gizmo::math::Vec3::new(lmax.x, lmin.y, lmin.z),
                gizmo::math::Vec3::new(lmax.x, lmax.y, lmin.z),
                gizmo::math::Vec3::new(lmin.x, lmax.y, lmin.z),
                gizmo::math::Vec3::new(lmin.x, lmin.y, lmax.z),
                gizmo::math::Vec3::new(lmax.x, lmin.y, lmax.z),
                gizmo::math::Vec3::new(lmax.x, lmax.y, lmax.z),
                gizmo::math::Vec3::new(lmin.x, lmax.y, lmax.z),
            ];

            // Köşeleri world-space'e dönüştür (rotasyon korunur!)
            let world_corners: [gizmo::math::Vec3; 8] = std::array::from_fn(|j| {
                let v = ref_gt * gizmo::math::Vec4::new(
                    local_corners[j].x, local_corners[j].y, local_corners[j].z, 1.0,
                );
                gizmo::math::Vec3::new(v.x, v.y, v.z)
            });

            for &(a, b) in &EDGES {
                gizmos.draw_line(world_corners[a], world_corners[b], ORANGE);
            }
        }

        // --- KAMERA İKONLARI (GİZMO) ---
        let cameras = world.borrow::<gizmo::renderer::components::Camera>();
        let global_transforms = world.borrow::<gizmo::physics::components::GlobalTransform>();
        let is_playing = editor_state.is_playing();

        if !is_playing {
            for (entity_id, _) in cameras.iter() {
                // Editor kamerasını göstermeye gerek yok
                if entity_id == state.editor_camera {
                    continue;
                }

                if let (Some(cam), Some(gt)) = (cameras.get(entity_id), global_transforms.get(entity_id)) {
                    let (pos, rot, _scale) = gizmo::renderer::decompose_mat4(gt.matrix);
                    
                    let forward = rot * gizmo::math::Vec3::new(0.0, 0.0, -1.0);
                    let up = rot * gizmo::math::Vec3::new(0.0, 1.0, 0.0);
                    let right = rot * gizmo::math::Vec3::new(1.0, 0.0, 0.0);
                    
                    let is_selected = editor_state.selection.entities.contains(&gizmo::core::entity::Entity::new(entity_id, 0));
                    let color = if is_selected { [1.0, 1.0, 0.0, 1.0] } else { [0.8, 0.8, 0.8, 1.0] }; // Seçiliyse sarı, değilse gri
                    
                    // Kamera Gövdesi (Küçük bir kutu)
                    let body_size = 0.2;
                    let body_front = pos + forward * body_size;
                    let body_back = pos - forward * body_size;
                    
                    let top_left = body_back + up * body_size - right * body_size;
                    let top_right = body_back + up * body_size + right * body_size;
                    let bot_left = body_back - up * body_size - right * body_size;
                    let bot_right = body_back - up * body_size + right * body_size;
                    
                    let f_top_left = body_front + up * body_size - right * body_size;
                    let f_top_right = body_front + up * body_size + right * body_size;
                    let f_bot_left = body_front - up * body_size - right * body_size;
                    let f_bot_right = body_front - up * body_size + right * body_size;
                    
                    // Arka Yüzey
                    gizmos.draw_line(top_left, top_right, color);
                    gizmos.draw_line(top_right, bot_right, color);
                    gizmos.draw_line(bot_right, bot_left, color);
                    gizmos.draw_line(bot_left, top_left, color);
                    
                    // Ön Yüzey
                    gizmos.draw_line(f_top_left, f_top_right, color);
                    gizmos.draw_line(f_top_right, f_bot_right, color);
                    gizmos.draw_line(f_bot_right, f_bot_left, color);
                    gizmos.draw_line(f_bot_left, f_top_left, color);
                    
                    // Yan Bağlantılar
                    gizmos.draw_line(top_left, f_top_left, color);
                    gizmos.draw_line(top_right, f_top_right, color);
                    gizmos.draw_line(bot_left, f_bot_left, color);
                    gizmos.draw_line(bot_right, f_bot_right, color);
                    
                    // Lens / Frustum (Piramit)
                    let lens_length = 0.6;
                    let fov_factor = (cam.fov / 2.0).tan() * lens_length;
                    
                    let lens_end = body_front + forward * lens_length;
                    let aspect = 1.77; // 16:9
                    let l_top_left = lens_end + up * fov_factor - right * (fov_factor * aspect);
                    let l_top_right = lens_end + up * fov_factor + right * (fov_factor * aspect);
                    let l_bot_left = lens_end - up * fov_factor - right * (fov_factor * aspect);
                    let l_bot_right = lens_end - up * fov_factor + right * (fov_factor * aspect);
                    
                    // Lens ucu çerçevesi
                    gizmos.draw_line(l_top_left, l_top_right, color);
                    gizmos.draw_line(l_top_right, l_bot_right, color);
                    gizmos.draw_line(l_bot_right, l_bot_left, color);
                    gizmos.draw_line(l_bot_left, l_top_left, color);
                    
                    // Lens gövde bağlantıları
                    gizmos.draw_line(f_top_left, l_top_left, color);
                    gizmos.draw_line(f_top_right, l_top_right, color);
                    gizmos.draw_line(f_bot_left, l_bot_left, color);
                    gizmos.draw_line(f_bot_right, l_bot_right, color);
                    
                    // Yukarı Yön Oku (Up vector)
                    let up_base = body_back + up * (body_size + 0.05);
                    let up_tip = up_base + up * 0.3;
                    gizmos.draw_line(up_base - right * 0.05, up_tip, [0.0, 1.0, 0.0, 1.0]);
                    gizmos.draw_line(up_base + right * 0.05, up_tip, [0.0, 1.0, 0.0, 1.0]);
                    gizmos.draw_line(up_base - right * 0.05, up_base + right * 0.05, [0.0, 1.0, 0.0, 1.0]);
                }
            }
        }
        // --- IŞIK İKONLARI (GİZMO) ---
        let dir_lights = world.borrow::<gizmo::renderer::components::DirectionalLight>();
        let point_lights = world.borrow::<gizmo::renderer::components::PointLight>();

        if !is_playing {
            // Directional Light Gizmos
            for (entity_id, _) in dir_lights.iter() {
                if let (Some(light), Some(gt)) = (dir_lights.get(entity_id), global_transforms.get(entity_id)) {
                    let (pos, rot, _scale) = gizmo::renderer::decompose_mat4(gt.matrix);
                    let forward = rot * gizmo::math::Vec3::new(0.0, 0.0, -1.0);
                    let right = rot * gizmo::math::Vec3::new(1.0, 0.0, 0.0);
                    let up = rot * gizmo::math::Vec3::new(0.0, 1.0, 0.0);
                    
                    let is_selected = editor_state.selection.entities.contains(&gizmo::core::entity::Entity::new(entity_id, 0));
                    let color = if is_selected { [1.0, 1.0, 0.0, 1.0] } else { [light.color.x, light.color.y, light.color.z, 1.0] };
                    
                    // Güneş ikonu (Merkezden çıkan ışınlar)
                    let radius = 0.4;
                    for i in 0..8 {
                        let angle = (i as f32) * std::f32::consts::PI / 4.0;
                        let dir = right * angle.cos() + up * angle.sin();
                        gizmos.draw_line(pos + dir * (radius * 0.5), pos + dir * radius, color);
                    }
                    
                    // Yön çizgisi
                    gizmos.draw_line(pos, pos + forward * 1.5, color);
                }
            }

            // Point Light Gizmos
            for (entity_id, _) in point_lights.iter() {
                if let (Some(light), Some(gt)) = (point_lights.get(entity_id), global_transforms.get(entity_id)) {
                    let (pos, _rot, _scale) = gizmo::renderer::decompose_mat4(gt.matrix);
                    
                    let is_selected = editor_state.selection.entities.contains(&gizmo::core::entity::Entity::new(entity_id, 0));
                    let color = if is_selected { [1.0, 1.0, 0.0, 1.0] } else { [light.color.x, light.color.y, light.color.z, 1.0] };
                    
                    // Ampul ikonu (küçük küre / çapraz çizgiler)
                    let r = 0.2;
                    let p1 = pos + gizmo::math::Vec3::new(r, 0.0, 0.0);
                    let p2 = pos - gizmo::math::Vec3::new(r, 0.0, 0.0);
                    let p3 = pos + gizmo::math::Vec3::new(0.0, r, 0.0);
                    let p4 = pos - gizmo::math::Vec3::new(0.0, r, 0.0);
                    let p5 = pos + gizmo::math::Vec3::new(0.0, 0.0, r);
                    let p6 = pos - gizmo::math::Vec3::new(0.0, 0.0, r);
                    
                    gizmos.draw_line(p1, p2, color);
                    gizmos.draw_line(p3, p4, color);
                    gizmos.draw_line(p5, p6, color);
                    
                    // Etki alanı çemberi (seçiliyse)
                    if is_selected {
                        let steps = 32;
                        let mut prev_point = pos + gizmo::math::Vec3::new(light.radius, 0.0, 0.0);
                        for i in 1..=steps {
                            let angle = (i as f32) * std::f32::consts::TAU / (steps as f32);
                            let current_point = pos + gizmo::math::Vec3::new(angle.cos() * light.radius, 0.0, angle.sin() * light.radius);
                            gizmos.draw_line(prev_point, current_point, [color[0]*0.5, color[1]*0.5, color[2]*0.5, 0.5]);
                            prev_point = current_point;
                        }
                    }
                }
            }
        }
    }

    let show_colliders = world.get_resource::<gizmo::editor::EditorState>().map(|ed| ed.show_colliders).unwrap_or(false);
    if show_colliders {
        gizmo::systems::physics::physics_debug_system(world);
    }

    // Kamera sistemine editor state'e geri dönmüş delta'yı gönder
    crate::systems::camera::handle_camera(
        world,
        state,
        dt,
        input,
        look_delta,
        pan_delta,
        orbit_delta,
        scroll_delta.unwrap_or(0.0),
    );
}

/// Dizin kopyalama yardımcı fonksiyonu
pub fn copy_dir_all(
    src: impl AsRef<std::path::Path>,
    dst: impl AsRef<std::path::Path>,
    log: &dyn Fn(&str),
) -> std::io::Result<()> {
    std::fs::create_dir_all(&dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(entry.path(), dst.as_ref().join(entry.file_name()), log)?;
        } else {
            std::fs::copy(entry.path(), dst.as_ref().join(entry.file_name()))?;
        }
    }
    Ok(())
}
