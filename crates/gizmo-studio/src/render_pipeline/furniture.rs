//! The editor's furniture drawn as lines into the Scene panel (`EditorLines`): the box round each
//! selected entity, and the icons of the scene's cameras and lights.
//!
//! They were written into the game's `Gizmos` by the studio's update, with the game's
//! `depth_test` switched off every frame so the box stayed on top. The game path draws the game's
//! lines into the Game panel too, so the editor's selection box and icons showed in the game's own
//! picture — in play mode as well, for the box — and a game's debug lines lost their depth test in
//! the studio.

use crate::StudioState;
use gizmo::core::HierarchyExt;
use gizmo::editor::EditorState;
use gizmo::prelude::*;

/// Everything this module draws, into `lines`: the selection boxes always, the navigation grid's obstacle
/// cells while the editor is open, the icons only while editing.
pub(super) fn furniture_lines(world: &World, state: &StudioState, editor_state: &EditorState, lines: &mut gizmo::renderer::Gizmos) {
    selection_boxes(world, editor_state, lines);
    if editor_state.open {
        nav_grid_obstacles(world, lines);
    }
    if !editor_state.is_playing() {
        camera_icons(world, state, editor_state, lines);
        light_icons(world, editor_state, lines);
    }
}

/// An orange box round each selected entity and its descendants, turning with it: the meshes' bounds
/// merged in the first mesh's frame, and its eight corners taken to the world (an oriented box).
fn selection_boxes(world: &World, editor_state: &EditorState, lines: &mut gizmo::renderer::Gizmos) {
    // A bundle's mesh is in `Assets`, not on the entity: its box comes from there.
    let meshes = crate::drawn_mesh::DrawnMeshes::new(world);
    let global_transforms = world.query::<&gizmo::physics::components::GlobalTransform>();

    const ORANGE: [f32; 4] = [1.0, 0.5, 0.0, 1.0];
    const EDGES: [(usize, usize); 12] = [
        (0,1),(1,2),(2,3),(3,0),
        (4,5),(5,6),(6,7),(7,4),
        (0,4),(1,5),(2,6),(3,7),
    ];

    for &selected_entity in editor_state.selection.entities.iter() {
        // BFS: Bu entity + tüm torunları — cycle-safe ve tekrarsız.
        // Unguarded until 2026-08-30, and this one runs EVERY FRAME per selected entity, so
        // a cycle under a selected entity froze the studio on selection rather than on an
        // action the user took deliberately.
        let descendants: Vec<_> = world
            .descendants_inclusive(selected_entity)
            .into_iter()
            .filter_map(|id| world.live(id))
            .collect();

        // İlk mesh'li entity'nin GT matrisini "referans frame" olarak al.
        // Diğer çocukların bounds'larını bu frame'e dönüştürüp local-space
        // merged AABB oluştur. Sonra bu AABB'yi referans GT ile world'e taşı.
        let mut ref_gt = gizmo::math::Mat4::IDENTITY;
        let mut ref_gt_inv = gizmo::math::Mat4::IDENTITY;
        let mut local_min = gizmo::math::Vec3A::splat(f32::MAX);
        let mut local_max = gizmo::math::Vec3A::splat(f32::NEG_INFINITY);
        let mut has_mesh = false;

        for ent in &descendants {
            let (mesh, gt) = match (meshes.get(*ent), global_transforms.get(*ent)) {
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
            lines.draw_line(world_corners[a], world_corners[b], ORANGE);
        }
    }
}

/// A wire camera — body, lens and up arrow — at every camera but the editor's own; yellow when
/// selected.
fn camera_icons(world: &World, state: &StudioState, editor_state: &EditorState, lines: &mut gizmo::renderer::Gizmos) {
    let cameras = world.query::<&gizmo::renderer::components::Camera>();
    let global_transforms = world.query::<&gizmo::physics::components::GlobalTransform>();

    for (entity_id, _) in world.query::<(Entity, &gizmo::renderer::components::Camera)>().iter() {
        // Editor kamerasını göstermeye gerek yok
        if entity_id == state.editor_camera {
            continue;
        }

        if let (Some(cam), Some(gt)) = (cameras.get(entity_id), global_transforms.get(entity_id)) {
            let (pos, rot, _scale) = gizmo::renderer::decompose_mat4(gt.matrix);

            let forward = rot * gizmo::math::Vec3::new(0.0, 0.0, -1.0);
            let up = rot * gizmo::math::Vec3::new(0.0, 1.0, 0.0);
            let right = rot * gizmo::math::Vec3::new(1.0, 0.0, 0.0);

            let is_selected = editor_state.selection.entities.iter().any(|e| *e == entity_id);
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
            lines.draw_line(top_left, top_right, color);
            lines.draw_line(top_right, bot_right, color);
            lines.draw_line(bot_right, bot_left, color);
            lines.draw_line(bot_left, top_left, color);

            // Ön Yüzey
            lines.draw_line(f_top_left, f_top_right, color);
            lines.draw_line(f_top_right, f_bot_right, color);
            lines.draw_line(f_bot_right, f_bot_left, color);
            lines.draw_line(f_bot_left, f_top_left, color);

            // Yan Bağlantılar
            lines.draw_line(top_left, f_top_left, color);
            lines.draw_line(top_right, f_top_right, color);
            lines.draw_line(bot_left, f_bot_left, color);
            lines.draw_line(bot_right, f_bot_right, color);

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
            lines.draw_line(l_top_left, l_top_right, color);
            lines.draw_line(l_top_right, l_bot_right, color);
            lines.draw_line(l_bot_right, l_bot_left, color);
            lines.draw_line(l_bot_left, l_top_left, color);

            // Lens gövde bağlantıları
            lines.draw_line(f_top_left, l_top_left, color);
            lines.draw_line(f_top_right, l_top_right, color);
            lines.draw_line(f_bot_left, l_bot_left, color);
            lines.draw_line(f_bot_right, l_bot_right, color);

            // Yukarı Yön Oku (Up vector)
            let up_base = body_back + up * (body_size + 0.05);
            let up_tip = up_base + up * 0.3;
            lines.draw_line(up_base - right * 0.05, up_tip, [0.0, 1.0, 0.0, 1.0]);
            lines.draw_line(up_base + right * 0.05, up_tip, [0.0, 1.0, 0.0, 1.0]);
            lines.draw_line(up_base - right * 0.05, up_base + right * 0.05, [0.0, 1.0, 0.0, 1.0]);
        }
    }
}

/// A sun for each directional light (with its direction) and a cross for each point light (with its
/// reach when selected), in the light's colour; yellow when selected.
fn light_icons(world: &World, editor_state: &EditorState, lines: &mut gizmo::renderer::Gizmos) {
    let global_transforms = world.query::<&gizmo::physics::components::GlobalTransform>();
    let dir_lights = world.query::<&gizmo::renderer::components::DirectionalLight>();
    let point_lights = world.query::<&gizmo::renderer::components::PointLight>();

    // Directional Light Gizmos
    for (entity_id, _) in world.query::<(Entity, &gizmo::renderer::components::DirectionalLight)>().iter() {
        if let (Some(light), Some(gt)) = (dir_lights.get(entity_id), global_transforms.get(entity_id)) {
            let (pos, rot, _scale) = gizmo::renderer::decompose_mat4(gt.matrix);
            let forward = rot * gizmo::math::Vec3::new(0.0, 0.0, -1.0);
            let right = rot * gizmo::math::Vec3::new(1.0, 0.0, 0.0);
            let up = rot * gizmo::math::Vec3::new(0.0, 1.0, 0.0);

            let is_selected = editor_state.selection.entities.iter().any(|e| *e == entity_id);
            let color = if is_selected { [1.0, 1.0, 0.0, 1.0] } else { [light.color.x, light.color.y, light.color.z, 1.0] };

            // Güneş ikonu (Merkezden çıkan ışınlar)
            let radius = 0.4;
            for i in 0..8 {
                let angle = (i as f32) * std::f32::consts::PI / 4.0;
                let dir = right * angle.cos() + up * angle.sin();
                lines.draw_line(pos + dir * (radius * 0.5), pos + dir * radius, color);
            }

            // Yön çizgisi
            lines.draw_line(pos, pos + forward * 1.5, color);
        }
    }

    // Point Light Gizmos
    for (entity_id, _) in world.query::<(Entity, &gizmo::renderer::components::PointLight)>().iter() {
        if let (Some(light), Some(gt)) = (point_lights.get(entity_id), global_transforms.get(entity_id)) {
            let (pos, _rot, _scale) = gizmo::renderer::decompose_mat4(gt.matrix);

            let is_selected = editor_state.selection.entities.iter().any(|e| *e == entity_id);
            let color = if is_selected { [1.0, 1.0, 0.0, 1.0] } else { [light.color.x, light.color.y, light.color.z, 1.0] };

            // Ampul ikonu (küçük küre / çapraz çizgiler)
            let r = 0.2;
            let p1 = pos + gizmo::math::Vec3::new(r, 0.0, 0.0);
            let p2 = pos - gizmo::math::Vec3::new(r, 0.0, 0.0);
            let p3 = pos + gizmo::math::Vec3::new(0.0, r, 0.0);
            let p4 = pos - gizmo::math::Vec3::new(0.0, r, 0.0);
            let p5 = pos + gizmo::math::Vec3::new(0.0, 0.0, r);
            let p6 = pos - gizmo::math::Vec3::new(0.0, 0.0, r);

            lines.draw_line(p1, p2, color);
            lines.draw_line(p3, p4, color);
            lines.draw_line(p5, p6, color);

            // Etki alanı çemberi (seçiliyse)
            if is_selected {
                let steps = 32;
                let mut prev_point = pos + gizmo::math::Vec3::new(light.radius, 0.0, 0.0);
                for i in 1..=steps {
                    let angle = (i as f32) * std::f32::consts::TAU / (steps as f32);
                    let current_point = pos + gizmo::math::Vec3::new(angle.cos() * light.radius, 0.0, angle.sin() * light.radius);
                    lines.draw_line(prev_point, current_point, [color[0]*0.5, color[1]*0.5, color[2]*0.5, 0.5]);
                    prev_point = current_point;
                }
            }
        }
    }
}

/// A red box at every obstacle cell of the navigation grid.
fn nav_grid_obstacles(world: &World, lines: &mut gizmo::renderer::Gizmos) {
    if let Some(grid) = world.get_resource::<gizmo::ai::pathfinding::NavGrid>() {
        for &cell in &grid.obstacles {
            let center = grid.grid_to_world(cell);
            let half_size = gizmo::math::Vec3::new(
                grid.cell_size * 0.5,
                grid.cell_size * 0.5,
                grid.cell_size * 0.5,
            );
            let min = center - half_size;
            let max = center + half_size;
            lines.draw_box(min, max, [1.0, 0.0, 0.0, 0.5]); // Red boxes for obstacles
        }
    }
}
