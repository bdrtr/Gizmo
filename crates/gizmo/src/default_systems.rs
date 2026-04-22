use crate::core::World;
use crate::math::{Mat4, Vec3};
use crate::renderer::{
    components::{Camera, Material, Mesh, MeshRenderer},
    Renderer,
};
use crate::physics::{Collider, ColliderShape, GpuPhysicsLink, RigidBody, Transform};
use bytemuck;
use wgpu;

pub struct DrawItem {
    vbuf: std::sync::Arc<wgpu::Buffer>,
    vertex_count: u32,
    bind_group: std::sync::Arc<wgpu::BindGroup>,
    unlit: bool,
}

/// Bevy'nin DefaultPlugins davranisini taklit eden, sadece modelleri
/// isiklandirip hizlica ekrana basmaya yarayan kutudan cikmis Render Motoru.
/// Yeni acilan `tut` gibi bos projelerde yuzlerce satir kod yazmamak icin kullanilir.
pub fn default_render_pass(
    world: &mut World,
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    renderer: &mut Renderer,
) {
    let aspect = if renderer.size.height > 0 {
        renderer.size.width as f32 / renderer.size.height as f32
    } else {
        1.0
    };
    let mut proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, aspect, 0.1, 2000.0);
    let mut view_mat = Mat4::from_translation(Vec3::ZERO);
    let mut cam_pos = Vec3::ZERO;
    let mut cam_forward = Vec3::new(0.0, 0.0, -1.0);

    // TODO: Bütün nesnelerin (özellikle kamera ve çizilecek objelerin) global matrix'leri
    // bu pass çağrılmadan hemen önce bir `update_transforms(world)` sistemiyle güncellenmiş olmalıdır.
    
    // ECS veri GPU'ya basılır ve GPU verisi ECS'ye alınır
    gpu_physics_submit_system(world, renderer);
    gpu_physics_readback_system(world, renderer);

    // KAMERALARI BUL VE MATRIX YARAT
    let cameras = world.borrow::<Camera>(); let transforms = world.borrow::<Transform>();
    {
        // TODO: Aktif kamera için `ActiveCamera` tarzı bir marker bileşeni kullanılmalı.
        // ECS array sırası stabil değildir. Şimdilik geçici çözüm olarak ilki alınıyor.
        if let Some((active_cam, _)) = cameras.iter().next() {
            if let (Some(cam), Some(trans)) =
                (cameras.get(active_cam), transforms.get(active_cam))
            {
                proj = cam.get_projection(aspect);
                view_mat = cam.get_view(trans.position);
                cam_pos = trans.position;
                cam_forward = trans.rotation * Vec3::new(0.0, 0.0, -1.0);
            }
        }
    }

    let view_proj = proj * view_mat;
    let frustum = crate::renderer::Frustum::from_matrix(&view_proj);

    let id = Mat4::IDENTITY.to_cols_array_2d();
    let scene_uniform_data = crate::renderer::gpu_types::SceneUniforms {
        view_proj: view_proj.to_cols_array_2d(),
        camera_pos: [cam_pos.x, cam_pos.y, cam_pos.z, 1.0],
        sun_direction: [0.0, -1.0, 0.0, 1.0],
        sun_color: [1.0, 1.0, 1.0, 1.0],
        lights: [crate::renderer::gpu_types::LightData {
            position: [0.0; 4],
            color: [0.0; 4],
            direction: [0.0, -1.0, 0.0, 0.0],
            params: [0.0; 4],
        }; 10],
        light_view_proj: [id; 4],
        cascade_splits: [10.0, 50.0, 200.0, 2000.0],
        camera_forward: [cam_forward.x, cam_forward.y, cam_forward.z, 0.0],
        cascade_params: [0.1, 1.0 / crate::renderer::SHADOW_MAP_RES as f32, 0.0, 0.0],
        num_lights: 0,
        // _align_pad ve _pad_scene: wgpu Uniform alignment kuralları gereği son paddingler
        _align_pad: [0; 3],
        _pad_scene: [0; 3],
        _end_pad: 0,
    };
    renderer.queue.write_buffer(
        &renderer.scene.global_uniform_buffer,
        0,
        bytemuck::cast_slice(&[scene_uniform_data]),
    );

    let renderers = world.borrow::<MeshRenderer>();
    let mut instances = Vec::new();
    let mut draw_items = Vec::new();
    if let Some(mut q) = world.query::<(&Mesh, &Transform, &Material)>() {
        for (e, (mesh, trans, mat)) in q.iter_mut() {
            if renderers.get(e).is_none() {
                continue;
            }

            let center_mat = Mat4::from_translation(mesh.center_offset);
            let model = trans.global_matrix * center_mat;
            if !crate::renderer::visible_in_frustum(&frustum, &model, mesh.bounds) {
                continue;
            }
            let instance_data = crate::renderer::gpu_types::InstanceRaw {
                model: model.to_cols_array_2d(),
                albedo_color: [mat.albedo.x, mat.albedo.y, mat.albedo.z, mat.albedo.w],
                roughness: mat.roughness,
                metallic: mat.metallic,
                unlit: mat.unlit,
                _padding: 0.0,
            };
            instances.push(instance_data);
            draw_items.push(DrawItem {
                vbuf: mesh.vbuf.clone(),
                vertex_count: mesh.vertex_count,
                bind_group: mat.bind_group.clone(),
                unlit: mat.unlit > 0.5,
            });
        }
    }

    // Instance limiti kontrolü (Taşmaları önlemek için capaciteyi zorla)
    // TODO: Eğer needed > capacity ise çalışma zamanı pipeline re-allocation yapılmalı.
    let max_instances = renderer.scene.instance_capacity as usize;
    let instances: Vec<_> = instances.into_iter().take(max_instances).collect();

    if !instances.is_empty() {
        renderer.queue.write_buffer(
            &renderer.scene.instance_buffer,
            0,
            bytemuck::cast_slice(&instances),
        );
    }

    if let Some(physics) = &renderer.gpu_physics {
        // Her frame başında sıradaki state'i çekmek için WGPU CommandEncoder'a asenkron mapping iste.
        physics.request_readback(encoder);
        
        physics.compute_pass(encoder);
        physics.cull_pass(encoder, &renderer.scene.global_bind_group);
    }
    
    // Gpu Fluid Processing
    if let Some(fluid) = &renderer.gpu_fluid {
        fluid.compute_pass(encoder);
    }

    {
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Default Engine Render Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &renderer.post.hdr_texture_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.1,
                        g: 0.1,
                        b: 0.15,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &renderer.depth_texture_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        // TODO: Shadow map pass eksik, render_pass öncesinde directional shadow mapping calistirilmali!

        // Draw loop disina sabit BindGroupLayout lari aliyoruz (Performans optimizasyonu)
        render_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
        render_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
        render_pass.set_bind_group(3, &renderer.scene.dummy_skeleton_bind_group, &[]);
        render_pass.set_bind_group(4, &renderer.scene.instance_bind_group, &[]);

        for (i, item) in draw_items.iter().enumerate() {
            let pipeline = if item.unlit {
                &renderer.scene.unlit_pipeline
            } else {
                &renderer.scene.render_pipeline
            };
            render_pass.set_pipeline(pipeline);
            render_pass.set_bind_group(1, &item.bind_group, &[]);
            render_pass.set_vertex_buffer(0, item.vbuf.slice(..));
            render_pass.draw(0..item.vertex_count, (i as u32)..((i as u32) + 1));
        }

        // Draw GPU Physics Spheres!
        if let Some(physics) = &renderer.gpu_physics {
            physics.render_pass(&mut render_pass, &renderer.scene.global_bind_group);
        }

        // Draw SPH fluid
        if let Some(fluid) = &renderer.gpu_fluid {
            fluid.render_pass(&mut render_pass, &renderer.scene.global_bind_group);
        }

        if let Some(gizmos) = world.get_resource::<crate::renderer::Gizmos>() {
            if let Some(debug_renderer) = &mut renderer.debug_renderer {
                debug_renderer.update(&renderer.queue, &gizmos);
                debug_renderer.render(&mut render_pass, &renderer.scene.global_bind_group, gizmos.depth_test);
            }
        }
    }

    // Auto-clear gizmos for the next frame
    if let Some(mut gizmos) = world.get_resource_mut::<crate::renderer::Gizmos>() {
        gizmos.clear();
    }

    renderer.run_post_processing(encoder, view);
}

/// Basit bir sistem: Sahnede bulunan tüm fizik Collider'larının etrafına
/// yeşil bir Gizmo AABB kutusu çizer. 
/// Bu sayede geliştirici 물리 objelerinin nerede olduğunu görsel olarak test edebilir.
pub fn physics_debug_system(world: &crate::core::World) {
    if let Some(mut gizmos) = world.get_resource_mut::<crate::renderer::Gizmos>() {
        // Renk: Parlak Yeşil (R, G, B, A)
        let color = [0.1, 0.9, 0.1, 1.0];
        
        if let Some(q) = world.query::<(&crate::physics::Transform, &gizmo_physics::Collider)>() {
            for (_, (trans, col)) in q.iter() {
                // Not: tam bir döndürme (rotation) desteği istenirse draw_obb şeklinde
                // daha gelişmiş bir gizmo methodu eklenebilir. Şimdilik AABB (Eksen Hizalı) çiziyoruz.
                // Col.bounds aslında aabb_min ve aabb_max'ını barındırıyorsa ona göre de alınabilir.
                // Biz burada tahmini (veya objenin etrafındaki) bir kutu çizdirebiliriz.
                // GizmoMotor collider şeklini tam çizdirsin diye:
                match &col.shape {
                    gizmo_physics::shape::ColliderShape::Aabb(a) => {
                        let h = a.half_extents;
                        let min = trans.position - h;
                        let max = trans.position + h;
                        gizmos.draw_box(min, max, color);
                    }
                    gizmo_physics::shape::ColliderShape::Sphere(s) => {
                        let r = s.radius;
                        let min = trans.position - Vec3::new(r, r, r);
                        let max = trans.position + Vec3::new(r, r, r);
                        gizmos.draw_box(min, max, color);
                    }
                    _ => {
                        // Kapsül, Konveks, Mesh vs için genel bir min-max kutusu uyduralım
                        let min = trans.position - Vec3::new(1.0, 1.0, 1.0);
                        let max = trans.position + Vec3::new(1.0, 1.0, 1.0);
                        gizmos.draw_box(min, max, color);
                    }
                }
            }
        }
    }
}

/// ECS'deki yeni yaratılmış Fiziksel Objeleri (RigidBody + Transform + Collider)
/// GPU Physics çekirdeğinin otoyoluna (GpuPhysicsSystem::spheres_buffer) kaydeder.
pub fn gpu_physics_submit_system(world: &mut crate::core::World, renderer: &Renderer) {
    if let Some(physics) = &renderer.gpu_physics {
        let mut unlinked_entities = Vec::new();
        if let Some(q) = world.query::<(&RigidBody, &Transform, &Collider)>() {
            let links = world.borrow::<GpuPhysicsLink>();
            for (e, (rb, trans, col)) in q.iter() {
                if links.get(e).is_none() {
                    unlinked_entities.push((e, *rb, *trans, col.clone()));
                }
            }
        }
        
        let mut next_id = world.query::<&GpuPhysicsLink>().map(|q| q.iter().count() as u32).unwrap_or(0);
        
        for (e, rb, trans, col) in unlinked_entities {
            let id = next_id;
            next_id += 1;
            
            if rb.mass == 0.0 || rb.is_sleeping {
                // Statik engel
                let gpu_col = gizmo_renderer::physics_renderer::GpuCollider {
                    shape_type: match col.shape {
                        ColliderShape::Plane { .. } => 1,
                        _ => 0, // Varsayılan Box (AABB)
                    },
                    _pad1: [0; 3],
                    data1: match &col.shape {
                        ColliderShape::Plane { normal, .. } => [normal.x, normal.y, normal.z, 0.0],
                        ColliderShape::Aabb(aabb) => {
                            let min = trans.position - aabb.half_extents;
                            [min.x, min.y, min.z, 0.0]
                        },
                        _ => [0.0; 4],
                    },
                    data2: match &col.shape {
                        ColliderShape::Plane { constant, .. } => [*constant, 0.0, 0.0, 0.0],
                        ColliderShape::Aabb(aabb) => {
                            let max = trans.position + aabb.half_extents;
                            [max.x, max.y, max.z, 0.0]
                        },
                        _ => [0.0; 4],
                    },
                };
                // ID eşleştirmesi farklı olabilir ama basitlik adına collider sonuna ekle:
                // Şu an id uyuşmazlığı olabilir, demo için statik slotlara (0-50 arası) atıldığını varsayıyoruz.
                // Gerçek mimaride GpuColliderLink ayrı tutulmalı.
                physics.update_collider(&renderer.queue, id % 50, &gpu_col);
            } else {
                // Dinamik Kutu (AABB)
                let extents = match &col.shape {
                    ColliderShape::Aabb(s) => [s.half_extents.x, s.half_extents.y, s.half_extents.z],
                    _ => [0.5, 0.5, 0.5], 
                };
                
                let gpu_box = gizmo_renderer::physics_renderer::GpuBox {
                    position: [trans.position.x, trans.position.y, trans.position.z],
                    mass: rb.mass,
                    velocity: [0.0, 0.0, 0.0],
                    state: 0,
                    rotation: [trans.rotation.x, trans.rotation.y, trans.rotation.z, trans.rotation.w],
                    angular_velocity: [0.0, 0.0, 0.0],
                    sleep_counter: 0,
                    color: [0.3, 0.8, 1.0, 1.0], // Default color for ECS spawned
                    half_extents: extents,
                    _pad: 0,
                };
                physics.update_box(&renderer.queue, id, &gpu_box);
                
                world.add_component(
                    world.get_entity(e).unwrap(),
                    GpuPhysicsLink { id },
                );
            }
        }
    }
}

/// GPU'dan Asenkron (0ms) çekilen devasa Fizik lokasyon durumlarını,
/// Ekrandaki objelerin render edilmesi için ECS'deki Transform'larına kopyalar.
pub fn gpu_physics_readback_system(world: &mut crate::core::World, renderer: &Renderer) {
    if let Some(physics) = &renderer.gpu_physics {
        if let Some(gpu_data) = physics.poll_readback_data(&renderer.device) {
            if let Some(mut q) = world.query::<(gizmo_core::prelude::Mut<Transform>, &GpuPhysicsLink)>() {
                for (_, (mut trans, link)) in q.iter_mut() {
                    let idx = link.id as usize;
                    if idx < gpu_data.len() {
                        let box_data = &gpu_data[idx];
                        trans.position = gizmo_math::Vec3::new(box_data.position[0], box_data.position[1], box_data.position[2]);
                        trans.rotation = gizmo_math::Quat::from_xyzw(box_data.rotation[0], box_data.rotation[1], box_data.rotation[2], box_data.rotation[3]);
                        trans.update_local_matrix();
                    }
                }
            }
        }
    }
}
