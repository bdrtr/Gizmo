use gizmo::prelude::*;
use crate::state::DebugAssets;

pub fn update_editor_gizmos(world: &mut World, state: &crate::state::StudioState) {
    let mut to_spawn = Vec::new();
    let mut existing_gizmos = Vec::new();
    
    let is_playing = if let Some(ed) = world.get_resource::<gizmo::editor::EditorState>() {
        ed.is_playing()
    } else {
        false
    };

    {
        let cameras = world.borrow::<gizmo::renderer::components::Camera>();
        let point_lights = world.borrow::<gizmo::renderer::components::PointLight>();
        let spot_lights = world.borrow::<gizmo::renderer::SpotLight>();
        let dir_lights = world.borrow::<gizmo::renderer::components::DirectionalLight>();
        let names = world.borrow::<gizmo::core::component::EntityName>();
        let children_storage = world.borrow::<gizmo::core::component::Children>();
        
        let mut check_entity = |id: u32, gizmo_name: &str| {
            let mut has_gizmo = false;
            if let Some(children) = children_storage.get(id) {
                for &child_id in &children.0 {
                    if let Some(c_name) = names.get(child_id) {
                        if c_name.0 == gizmo_name {
                            has_gizmo = true;
                            existing_gizmos.push(child_id);
                            break;
                        }
                    }
                }
            }
            if !has_gizmo {
                to_spawn.push((id, gizmo_name.to_string()));
            }
        };

        for e in world.iter_alive_entities() {
            let id = e.id();
            
            // Skip the Editor Camera
            if id == state.editor_camera {
                continue;
            }

            if cameras.get(id).is_some() {
                check_entity(id, "Editor Gizmo - Camera");
            } else if point_lights.get(id).is_some() {
                check_entity(id, "Editor Gizmo - PointLight");
            } else if spot_lights.get(id).is_some() {
                check_entity(id, "Editor Gizmo - SpotLight");
            } else if dir_lights.get(id).is_some() {
                // If it's the main directional light, it usually has its own representation, but let's add one if not
                check_entity(id, "Editor Gizmo - DirectionalLight");
            }
        }
    }

    // Toggle visibility based on play mode
    for gizmo_id in existing_gizmos {
        if let Some(ent) = world.get_entity(gizmo_id) {
            let is_hidden = world.borrow::<gizmo::core::component::IsHidden>().contains(gizmo_id);
            if is_playing && !is_hidden {
                world.add_component(ent, gizmo::core::component::IsHidden);
            } else if !is_playing && is_hidden {
                world.remove_component::<gizmo::core::component::IsHidden>(ent);
            }
        }
    }

    // Play modunda "Editor " ile başlayan TÜM entity'leri gizle (Light Icon'lar, Grid vb.)
    {
        let names = world.borrow::<gizmo::core::component::EntityName>();
        let hidden = world.borrow::<gizmo::core::component::IsHidden>();
        let editor_entities: Vec<u32> = names.iter()
            .filter(|(id, name)| {
                name.0.starts_with("Editor ") && *id != state.editor_camera
            })
            .map(|(id, _)| id)
            .collect();
        drop(names);
        drop(hidden);

        for eid in editor_entities {
            if let Some(ent) = world.get_entity(eid) {
                let is_hidden = world.borrow::<gizmo::core::component::IsHidden>().contains(eid);
                if is_playing && !is_hidden {
                    world.add_component(ent, gizmo::core::component::IsHidden);
                } else if !is_playing && is_hidden {
                    world.remove_component::<gizmo::core::component::IsHidden>(ent);
                }
            }
        }
    }

    if to_spawn.is_empty() {
        return;
    }

let pending_assets = world.get_resource::<DebugAssets>().map(|a| (a.cube.clone(), a.sphere.clone(), a.white_tex.clone()));
    if let Some((cube_mesh, sphere_mesh, white_tex)) = pending_assets {
        for (parent_id, name) in to_spawn {
            let gizmo_ent = world.spawn();
            world.add_component(gizmo_ent, gizmo::core::component::EntityName(name.clone()));
            
            // Choose mesh based on gizmo name
            let is_light = name.contains("Light");
            let is_camera = name.contains("Camera");
            
            let mesh = if is_light { sphere_mesh.clone() } else { cube_mesh.clone() };
            world.add_component(gizmo_ent, mesh);
            
            world.add_component(gizmo_ent, gizmo::renderer::components::MeshRenderer::new());
            
            let color = if is_light {
                gizmo::math::Vec4::new(1.0, 0.9, 0.2, 1.0) // Yellow-ish
            } else if is_camera {
                gizmo::math::Vec4::new(0.2, 0.2, 0.25, 1.0) // Koyu gri/mavimsi
            } else {
                gizmo::math::Vec4::new(1.0, 1.0, 1.0, 1.0)
            };
            world.add_component(gizmo_ent, gizmo::prelude::Material::new(white_tex.clone()).with_unlit(color));
            
            // Create a small box for the gizmo
            let mut trans = gizmo::physics::components::Transform::new(gizmo::math::Vec3::ZERO);
            if is_camera {
                trans.scale = gizmo::math::Vec3::new(0.3, 0.2, 0.5); // Kamera kasası (dikdörtgen prizma)
            } else {
                trans.scale = gizmo::math::Vec3::new(0.4, 0.4, 0.4);
            }
            world.add_component(gizmo_ent, trans);
            world.add_component(gizmo_ent, gizmo::physics::components::GlobalTransform::default());
            world.add_component(gizmo_ent, gizmo::core::component::Parent(parent_id));
            
            // Ensure collider for raycast picking
            world.add_component(gizmo_ent, gizmo::physics::Collider::box_collider(gizmo::math::Vec3::new(0.4, 0.4, 0.4)));

            // If we are playing, hide it immediately
            if is_playing {
                world.add_component(gizmo_ent, gizmo::core::component::IsHidden);
            }

            
            let has_children_comp = {
                world.borrow::<gizmo::core::component::Children>().contains(parent_id)
            };

            if has_children_comp {
                let mut children_storage = world.borrow_mut::<gizmo::core::component::Children>();
                if let Some(ch) = children_storage.get_mut(parent_id) {
                    ch.0.push(gizmo_ent.id());
                }
            } else if let Some(ent) = world.get_entity(parent_id) {
                world.add_component(
                    ent, 
                    gizmo::core::component::Children(vec![gizmo_ent.id()])
                );
            }
        }
    }
}
