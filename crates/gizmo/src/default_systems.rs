use crate::core::World;
use crate::math::{Mat4, Vec3};
use crate::physics::{Collider, ColliderShape, GpuPhysicsLink, RigidBody, Transform};
use crate::renderer::{
    components::{Camera, Material, Mesh, MeshRenderer},
    Renderer,
};
use bytemuck;
use wgpu;

pub struct DrawItem {
    vbuf: std::sync::Arc<wgpu::Buffer>,
    vertex_count: u32,
    bind_group: std::sync::Arc<wgpu::BindGroup>,
    unlit: bool,
    is_skybox: bool,
    world_center: [f32; 3],
    radius: f32,
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
    let cameras = world.borrow::<Camera>();
    let transforms = world.borrow::<Transform>();
    {
        // TODO: Aktif kamera için `ActiveCamera` tarzı bir marker bileşeni kullanılmalı.
        // ECS array sırası stabil değildir. Şimdilik geçici çözüm olarak ilki alınıyor.
        if let Some((active_cam, _)) = cameras.iter().next() {
            if let (Some(cam), Some(trans)) = (cameras.get(active_cam), transforms.get(active_cam))
            {
                proj = cam.get_projection(aspect);
                view_mat = cam.get_view(trans.position);
                cam_pos = trans.position;
                cam_forward = trans.rotation * Vec3::new(0.0, 0.0, -1.0);
            }
        }
    }

    // Save unjittered projection before applying TAA offset (needed for reprojection next frame).
    let unjittered_proj = proj;

    // ── TAA Halton jitter: subpixel offset applied via z-column of projection ──
    if let Some(ref taa) = renderer.taa {
        let jp = crate::renderer::taa::TaaState::get_jitter(taa.frame_index);
        // Convert pixel jitter [−0.5, 0.5] to NDC offset (2 / viewport_size per axis)
        let jx = jp[0] * 2.0 / renderer.size.width  as f32;
        let jy = jp[1] * 2.0 / renderer.size.height as f32;
        // Adding jitter to NDC.x requires: new_clip.x = clip.x - jx*vz
        // ↔ subtract jx from proj.z_axis.x (the M[0][2] element, row0·col2)
        proj.z_axis.x -= jx;
        proj.z_axis.y -= jy;
    }

    let view_proj            = proj            * view_mat;  // jittered — used for SceneUniforms
    let unjittered_view_proj = unjittered_proj * view_mat;  // clean    — stored in TaaState for next frame

    // Güneş Işığını Bul
    let mut sun_dir = gizmo_math::Vec3::new(0.0, -1.0, 0.0);
    let mut sun_col = gizmo_math::Vec4::new(1.0, 1.0, 1.0, 1.0);
    if let Some(q) = world.query::<(&crate::renderer::components::DirectionalLight, &crate::physics::Transform)>() {
        for (_id, (light, transform)) in q.iter() {
            if light.role == crate::renderer::components::LightRole::Sun {
                sun_dir = transform.rotation.mul_vec3(gizmo_math::Vec3::new(0.0, 0.0, -1.0)).normalize();
                sun_col = gizmo_math::Vec4::new(light.color.x, light.color.y, light.color.z, light.intensity);
                break;
            }
        }
    }

    let cascade_splits = [10.0f32, 50.0, 200.0, 2000.0];
    let cascade_vp = crate::renderer::directional_cascade_view_projs(
        cam_pos,
        cam_forward,
        aspect,
        std::f32::consts::FRAC_PI_4,
        0.1,
        &cascade_splits,
        sun_dir,
        crate::renderer::SHADOW_MAP_RES,
    );
    let light_view_projs: [[[f32; 4]; 4]; 4] = cascade_vp.map(|m| m.to_cols_array_2d());

    // Dinamik Işıkları Bul
    let mut lights_data = [crate::renderer::gpu_types::LightData {
        position: [0.0; 4],
        color: [0.0; 4],
        direction: [0.0, -1.0, 0.0, 0.0],
        params: [0.0; 4],
    }; 10];
    let mut num_lights = 0;

    if let Some(q) = world.query::<(&crate::renderer::components::PointLight, &crate::physics::Transform)>() {
        for (_id, (light, transform)) in q.iter() {
            if num_lights >= 10 { break; }
            lights_data[num_lights as usize] = crate::renderer::gpu_types::LightData {
                position: [transform.position.x, transform.position.y, transform.position.z, light.intensity],
                color: [light.color.x, light.color.y, light.color.z, light.radius],
                direction: [0.0, -1.0, 0.0, 0.0],
                params: [0.0, 0.0, 0.0, 0.0], // y = 0 means PointLight
            };
            num_lights += 1;
        }
    }

    if let Some(q) = world.query::<(&crate::renderer::components::SpotLight, &crate::physics::Transform)>() {
        for (_id, (light, transform)) in q.iter() {
            if num_lights >= 10 { break; }
            let dir = transform.rotation.mul_vec3(gizmo_math::Vec3::new(0.0, 0.0, -1.0)).normalize();
            lights_data[num_lights as usize] = crate::renderer::gpu_types::LightData {
                position: [transform.position.x, transform.position.y, transform.position.z, light.intensity],
                color: [light.color.x, light.color.y, light.color.z, light.radius],
                direction: [dir.x, dir.y, dir.z, light.inner_angle],
                params: [light.outer_angle, 1.0, 0.0, 0.0], // y = 1 means SpotLight
            };
            num_lights += 1;
        }
    }

    let scene_uniform_data = crate::renderer::gpu_types::SceneUniforms {
        view_proj: view_proj.to_cols_array_2d(),
        camera_pos: [cam_pos.x, cam_pos.y, cam_pos.z, 1.0],
        sun_direction: [sun_dir.x, sun_dir.y, sun_dir.z, 1.0],
        sun_color: [sun_col.x, sun_col.y, sun_col.z, sun_col.w],
        lights: lights_data,
        light_view_proj: light_view_projs,
        cascade_splits,
        camera_forward: [cam_forward.x, cam_forward.y, cam_forward.z, 0.0],
        cascade_params: [0.1, 1.0 / crate::renderer::SHADOW_MAP_RES as f32, 0.0, 0.0],
        num_lights,
        // WGSL padding: vec3<u32> alignment 16 gerektirir
        _pre_align_pad: [0; 3],
        _align_pad: [0; 3],
        _post_align_pad: 0,
        _pad_scene: [0; 3],
        _end_pad: 0,
    };
    renderer.queue.write_buffer(
        &renderer.scene.global_uniform_buffer,
        0,
        bytemuck::cast_slice(&[scene_uniform_data]),
    );
    for i in 0..crate::renderer::CASCADE_COUNT {
        renderer.queue.write_buffer(
            &renderer.scene.shadow_cascade_uniform_buffers[i],
            0,
            bytemuck::bytes_of(&crate::renderer::gpu_types::ShadowVsUniform {
                light_view_proj: light_view_projs[i],
            }),
        );
    }

    // Upload TAA params (prev_vp from last frame, current jitter, blend alpha)
    if let Some(ref mut taa) = renderer.taa {
        let jp = crate::renderer::taa::TaaState::get_jitter(taa.frame_index);
        let jx = jp[0] * 2.0 / renderer.size.width  as f32;
        let jy = jp[1] * 2.0 / renderer.size.height as f32;
        let alpha = if taa.frame_index == 0 { 1.0f32 } else { 0.1f32 };
        taa.update_params(&renderer.queue, [jx, jy], alpha);
        taa.store_prev_vp(unjittered_view_proj.to_cols_array_2d());
    }

    let renderers = world.borrow::<MeshRenderer>();
    let mut instances = Vec::new();
    let mut draw_items = Vec::new();
    if let Some(mut q) = world.query::<(&Mesh, &Transform, &Material)>() {
        for (e, (mesh, trans, mat)) in q.iter_mut() {
            if renderers.get(e).is_none() {
                continue;
            }

            let center_mat = Mat4::from_translation(mesh.center_offset);
            let model = trans.local_matrix * center_mat;

            // Compute world-space bounding sphere for GPU frustum cull pass
            let local_cx = (mesh.bounds.min.x + mesh.bounds.max.x) * 0.5;
            let local_cy = (mesh.bounds.min.y + mesh.bounds.max.y) * 0.5;
            let local_cz = (mesh.bounds.min.z + mesh.bounds.max.z) * 0.5;
            let world_c = model.transform_point3(Vec3::new(local_cx, local_cy, local_cz));
            let hx = (mesh.bounds.max.x - mesh.bounds.min.x) * 0.5;
            let hy = (mesh.bounds.max.y - mesh.bounds.min.y) * 0.5;
            let hz = (mesh.bounds.max.z - mesh.bounds.min.z) * 0.5;
            let local_r = (hx * hx + hy * hy + hz * hz).sqrt();
            let sx = model.x_axis.truncate().length();
            let sy = model.y_axis.truncate().length();
            let sz = model.z_axis.truncate().length();
            let world_r = local_r * sx.max(sy).max(sz);

            let instance_data = crate::renderer::gpu_types::InstanceRaw {
                model: model.to_cols_array_2d(),
                albedo_color: [mat.albedo.x, mat.albedo.y, mat.albedo.z, mat.albedo.w],
                roughness: mat.roughness,
                metallic: mat.metallic,
                unlit: match mat.material_type {
                    crate::renderer::components::MaterialType::Skybox => 2.0,
                    crate::renderer::components::MaterialType::Unlit => 1.0,
                    _ => 0.0,
                },
                _padding: 0.0,
            };
            instances.push(instance_data);
            draw_items.push(DrawItem {
                vbuf: mesh.vbuf.clone(),
                vertex_count: mesh.vertex_count,
                bind_group: mat.bind_group.clone(),
                unlit: mat.material_type == crate::renderer::components::MaterialType::Unlit
                    || mat.material_type == crate::renderer::components::MaterialType::Skybox,
                is_skybox: mat.material_type == crate::renderer::components::MaterialType::Skybox,
                world_center: [world_c.x, world_c.y, world_c.z],
                radius: world_r,
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

    // GPU cull prepare: upload per-instance bounding spheres and initial draw args (instance_count=0)
    if let Some(ref cull) = renderer.gpu_cull {
        let bounds_data: Vec<crate::renderer::MeshBoundsRaw> = draw_items
            .iter()
            .map(|item| crate::renderer::MeshBoundsRaw {
                world_center: item.world_center,
                radius: item.radius,
            })
            .collect();
        let draw_args_data: Vec<crate::renderer::DrawIndirectArgs> = draw_items
            .iter()
            .enumerate()
            .map(|(i, item)| crate::renderer::DrawIndirectArgs {
                vertex_count: item.vertex_count,
                instance_count: 0, // GPU cull pass sets this to 1 if visible
                first_vertex: 0,
                first_instance: i as u32,
            })
            .collect();
        cull.prepare(&renderer.queue, &bounds_data, &draw_args_data);
    }

    if let Some(physics) = &renderer.gpu_physics {
        // Her frame başında sıradaki state'i çekmek için WGPU CommandEncoder'a asenkron mapping iste.
        physics.request_readback(encoder);

        physics.compute_pass(encoder);
        physics.debug_compute_pass(encoder);
        physics.cull_pass(encoder, &renderer.scene.global_bind_group);
    }

    // Gpu Fluid Processing
    if let Some(fluid) = &renderer.gpu_fluid {
        fluid.compute_pass(encoder, &renderer.queue, true, fluid.num_particles);
    }

    // Gpu Particles Processing
    if let Some(particles) = &renderer.gpu_particles {
        let dt = world.get_resource::<gizmo_core::time::Time>().map(|t| t.time_scale() * 0.016).unwrap_or(0.016);
        particles.update_params(&renderer.queue, dt); // Scale based on time_scale
        particles.compute_pass(encoder);
    }

    // GPU mesh frustum culling — writes instance_count into indirect_buffer
    if let Some(ref cull) = renderer.gpu_cull {
        cull.cull_pass(encoder, &renderer.scene.global_bind_group, draw_items.len() as u32);
    }

    // Resize deferred G-buffers if window changed; resize SSAO + TAA to match
    if let Some(ref mut def) = renderer.deferred {
        def.resize(&renderer.device, renderer.size.width, renderer.size.height);
    }
    {
        let w = renderer.size.width;
        let h = renderer.size.height;
        if let (Some(ssao), Some(def)) = (&mut renderer.ssao, &renderer.deferred) {
            if ssao.width != w || ssao.height != h {
                ssao.resize(&renderer.device, def, w, h);
            }
        }
        if let (Some(ssr), Some(def)) = (&mut renderer.ssr, &renderer.deferred) {
            if ssr.width != w || ssr.height != h {
                ssr.resize(&renderer.device, def, &renderer.post.hdr_texture_view, w, h);
            }
        }
        if let (Some(volumetric), Some(def)) = (&mut renderer.volumetric, &renderer.deferred) {
            if volumetric.width != w || volumetric.height != h {
                volumetric.resize(&renderer.device, def, w, h);
            }
        }
    }
    {
        let w = renderer.size.width;
        let h = renderer.size.height;
        if let (Some(taa), Some(def)) = (&mut renderer.taa, &renderer.deferred) {
            if taa.width != w || taa.height != h {
                taa.resize(
                    &renderer.device,
                    &renderer.post.hdr_texture_view,
                    &def.world_position_view,
                    w, h,
                );
            }
        }
    }

    // CSM shadow passes — one depth-only pass per cascade.
    for i in 0..crate::renderer::CASCADE_COUNT {
        let mut shadow_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Shadow Pass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &renderer.scene.shadow_cascade_layer_views[i],
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        shadow_pass.set_pipeline(&renderer.scene.shadow_pipeline);
        shadow_pass.set_bind_group(0, &renderer.scene.shadow_pass_bind_groups[i], &[]);
        shadow_pass.set_bind_group(1, &renderer.scene.dummy_skeleton_bind_group, &[]);
        shadow_pass.set_bind_group(2, &renderer.scene.instance_bind_group, &[]);
        for (j, item) in draw_items.iter().enumerate() {
            if item.unlit {
                continue;
            }
            shadow_pass.set_vertex_buffer(0, item.vbuf.slice(..));
            shadow_pass.draw(0..item.vertex_count, (j as u32)..((j as u32) + 1));
        }
    }

    // ── G-buffer pass (PBR geometry → albedo / normal / world-position) ─────
    if let Some(ref def) = renderer.deferred {
        let mut gbuf_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("G-Buffer Pass"),
            color_attachments: &[
                Some(wgpu::RenderPassColorAttachment {
                    view: &def.albedo_metallic_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: &def.normal_roughness_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: &def.world_position_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                }),
            ],
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
        gbuf_pass.set_pipeline(&def.gbuffer_pipeline);
        gbuf_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
        gbuf_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
        gbuf_pass.set_bind_group(3, &renderer.scene.dummy_skeleton_bind_group, &[]);
        gbuf_pass.set_bind_group(4, &renderer.scene.instance_bind_group, &[]);
        for (i, item) in draw_items.iter().enumerate() {
            if item.unlit {
                continue;
            }
            gbuf_pass.set_bind_group(1, &item.bind_group, &[]);
            gbuf_pass.set_vertex_buffer(0, item.vbuf.slice(..));
            if let Some(ref cull) = renderer.gpu_cull {
                gbuf_pass.draw_indirect(
                    &cull.indirect_buffer,
                    crate::renderer::GpuCullState::indirect_offset(i),
                );
            } else {
                gbuf_pass.draw(0..item.vertex_count, (i as u32)..((i as u32) + 1));
            }
        }
    }

    // ── Decal Pass (Blend into G-buffer) ──────────────────────────
    let mut decal_draws = Vec::new();
    if let Some(ref decal_state) = renderer.decal {
        let decals = world.borrow::<crate::renderer::components::Decal>();
        let transforms = world.borrow::<crate::physics::Transform>();
        let mut uniform_data = Vec::new();
        
        for (id, decal) in decals.iter() {
            if let Some(trans) = transforms.get(id) {
                let model = trans.local_matrix;
                let inv_model = model.inverse();
                
                uniform_data.push(crate::renderer::decal::DecalUniforms {
                    inv_model: inv_model.to_cols_array(),
                    model: model.to_cols_array(),
                    albedo_color: [decal.color.x, decal.color.y, decal.color.z, decal.color.w],
                    _pad: [0.0; 28],
                });
                
                decal_draws.push(decal.bind_group.clone());
                if uniform_data.len() >= 1024 { break; } // Max 1024 decals limit
            }
        }
        
        if !uniform_data.is_empty() {
            renderer.queue.write_buffer(
                &decal_state.uniform_buffer,
                0,
                bytemuck::cast_slice(&uniform_data),
            );
        }
    }

    if !decal_draws.is_empty() {
        if let (Some(ref decal_state), Some(ref def)) = (&renderer.decal, &renderer.deferred) {
            let mut decal_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Decal Pass"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: &def.albedo_metallic_view,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                    }),
                ],
                depth_stencil_attachment: None, // No depth testing needed
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            
            decal_pass.set_pipeline(&decal_state.pipeline);
            decal_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
            decal_pass.set_bind_group(1, &decal_state.world_pos_bg, &[]);
            decal_pass.set_vertex_buffer(0, decal_state.vertex_buffer.slice(..));
            
            for (i, bind_group) in decal_draws.iter().enumerate() {
                let offset = (i * 256) as u32;
                decal_pass.set_bind_group(2, bind_group, &[]);
                decal_pass.set_bind_group(3, &decal_state.decal_uniform_bg, &[offset]);
                decal_pass.draw(0..36, 0..1);
            }
        }
    }

    // ── Deferred lighting pass (G-buffers → HDR) ──────────────────────────
    if let Some(ref def) = renderer.deferred {
        let mut light_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Deferred Lighting Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &renderer.post.hdr_texture_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        light_pass.set_pipeline(&def.lighting_pipeline);
        light_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
        light_pass.set_bind_group(1, &renderer.scene.shadow_bind_group, &[]);
        light_pass.set_bind_group(2, &def.gbuffer_bind_group, &[]);
        light_pass.draw(0..3, 0..1);
    }

    // ── SSAO: hemisphere sampling → raw AO texture ────────────────────────────
    if let Some(ref ssao) = renderer.ssao {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("SSAO Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &ssao.ao_view,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::WHITE), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&ssao.ssao_pipeline);
        pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
        pass.set_bind_group(1, &ssao.ssao_gbuf_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    // ── SSAO blur: 5×5 box filter → blurred AO texture ───────────────────────
    if let Some(ref ssao) = renderer.ssao {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("SSAO Blur Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &ssao.ao_blurred_view,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::WHITE), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&ssao.blur_pipeline);
        pass.set_bind_group(0, &ssao.blur_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    // ── SSAO apply: multiply AO into HDR (multiply blend) ─────────────────────
    if let Some(ref ssao) = renderer.ssao {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("SSAO Apply Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &renderer.post.hdr_texture_view,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&ssao.apply_pipeline);
        pass.set_bind_group(0, &ssao.apply_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    // ── Forward pass (unlit / skybox / GPU subsystems; PBR skipped if deferred) ──
    {
        let hdr_load = if renderer.deferred.is_some() {
            wgpu::LoadOp::Load
        } else {
            wgpu::LoadOp::Clear(wgpu::Color { r: 0.1, g: 0.1, b: 0.15, a: 1.0 })
        };
        let depth_load = if renderer.deferred.is_some() {
            wgpu::LoadOp::Load
        } else {
            wgpu::LoadOp::Clear(1.0)
        };
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Default Engine Render Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &renderer.post.hdr_texture_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: hdr_load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &renderer.depth_texture_view,
                depth_ops: Some(wgpu::Operations {
                    load: depth_load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        render_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
        render_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
        render_pass.set_bind_group(3, &renderer.scene.dummy_skeleton_bind_group, &[]);
        render_pass.set_bind_group(4, &renderer.scene.instance_bind_group, &[]);

        for (i, item) in draw_items.iter().enumerate() {
            let pipeline = if item.is_skybox {
                &renderer.scene.sky_pipeline
            } else if item.unlit {
                &renderer.scene.unlit_pipeline
            } else if renderer.deferred.is_none() {
                &renderer.scene.render_pipeline
            } else {
                continue; // PBR already rendered in deferred G-buffer + lighting pass
            };
            render_pass.set_pipeline(pipeline);
            render_pass.set_bind_group(1, &item.bind_group, &[]);
            render_pass.set_vertex_buffer(0, item.vbuf.slice(..));
            if let Some(ref cull) = renderer.gpu_cull {
                render_pass.draw_indirect(
                    &cull.indirect_buffer,
                    crate::renderer::GpuCullState::indirect_offset(i),
                );
            } else {
                render_pass.draw(0..item.vertex_count, (i as u32)..((i as u32) + 1));
            }
        }

        // Draw GPU Physics Spheres!
        if let Some(physics) = &renderer.gpu_physics {
            physics.render_pass(&mut render_pass, &renderer.scene.global_bind_group);
            physics.debug_render_pass(&mut render_pass, &renderer.scene.global_bind_group);
        }

        // Draw SPH fluid
        if let Some(fluid) = &renderer.gpu_fluid {
            fluid.render_pass(&mut render_pass, &renderer.scene.global_bind_group);
        }

        // Draw GPU Particles
        if let Some(particles) = &renderer.gpu_particles {
            particles.render_pass(&mut render_pass, &renderer.scene.global_bind_group);
        }

        if let Some(gizmos) = world.get_resource::<crate::renderer::Gizmos>() {
            if let Some(debug_renderer) = &mut renderer.debug_renderer {
                debug_renderer.update(&renderer.queue, &gizmos);
                debug_renderer.render(
                    &mut render_pass,
                    &renderer.scene.global_bind_group,
                    gizmos.depth_test,
                );
            }
        }
    }

    if let Some(fluid) = &renderer.gpu_fluid {
        fluid.render_ssfr(
            encoder,
            &renderer.post.hdr_texture,
            &renderer.post.hdr_texture_view,
            &renderer.depth_texture_view,
            &renderer.scene.global_bind_group,
            fluid.num_particles,
        );
    }

    // Auto-clear gizmos for the next frame
    if let Some(mut gizmos) = world.get_resource_mut::<crate::renderer::Gizmos>() {
        gizmos.clear();
    }

    // ── SSR: Screen Space Reflections ───────────────────────────────────────────
    if let Some(ref ssr) = renderer.ssr {
        // Pass 1: SSR Raymarch
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("SSR Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &ssr.ssr_view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&ssr.ssr_pipeline);
            pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
            pass.set_bind_group(1, &ssr.ssr_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        // Pass 2: SSR Apply (Additive blend into HDR texture)
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("SSR Apply Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &renderer.post.hdr_texture_view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&ssr.apply_pipeline);
            pass.set_bind_group(0, &ssr.apply_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    // ── Volumetric Lighting (God Rays) ──────────────────────────────────────────
    if let Some(ref vol) = renderer.volumetric {
        // Pass 1: Volumetric Raymarch
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Volumetric Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &vol.volumetric_view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&vol.volumetric_pipeline);
            pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
            pass.set_bind_group(1, &renderer.scene.shadow_bind_group, &[]);
            pass.set_bind_group(2, &vol.volumetric_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        // Pass 2: Volumetric Apply (Additive blend into HDR texture)
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Volumetric Apply Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &renderer.post.hdr_texture_view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&vol.apply_pipeline);
            pass.set_bind_group(0, &vol.apply_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    // ── TAA resolve: blend jittered HDR with clamped history ─────────────────
    if let Some(ref taa) = renderer.taa {
        let (resolve_bg, output_view) = taa.current_resolve_inputs_output();
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("TAA Resolve Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view:           output_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load:  wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&taa.resolve_pipeline);
        pass.set_bind_group(0, resolve_bg, &[]);
        pass.draw(0..3, 0..1);
    }

    // ── TAA blit: copy stabilized history output back into HDR texture ───────
    if let Some(ref taa) = renderer.taa {
        let blit_bg = taa.current_blit_bg();
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("TAA Blit Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view:           &renderer.post.hdr_texture_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load:  wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&taa.blit_pipeline);
        pass.set_bind_group(0, &taa.empty_bg, &[]);
        pass.set_bind_group(1, blit_bg, &[]);
        pass.draw(0..3, 0..1);
    }

    // Advance TAA ping-pong and frame counter
    if let Some(ref mut taa) = renderer.taa {
        taa.advance_frame();
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
                // To support proper rotation, we should draw the 8 corners of the box.
                match &col.shape {
                    gizmo_physics::ColliderShape::Box(b) => {
                        let h = b.half_extents;
                        let p0 = trans.local_matrix.transform_point3(Vec3::new(-h.x, -h.y, -h.z));
                        let p1 = trans.local_matrix.transform_point3(Vec3::new( h.x, -h.y, -h.z));
                        let p2 = trans.local_matrix.transform_point3(Vec3::new( h.x,  h.y, -h.z));
                        let p3 = trans.local_matrix.transform_point3(Vec3::new(-h.x,  h.y, -h.z));
                        let p4 = trans.local_matrix.transform_point3(Vec3::new(-h.x, -h.y,  h.z));
                        let p5 = trans.local_matrix.transform_point3(Vec3::new( h.x, -h.y,  h.z));
                        let p6 = trans.local_matrix.transform_point3(Vec3::new( h.x,  h.y,  h.z));
                        let p7 = trans.local_matrix.transform_point3(Vec3::new(-h.x,  h.y,  h.z));
                        
                        gizmos.draw_line(p0, p1, color); gizmos.draw_line(p1, p2, color);
                        gizmos.draw_line(p2, p3, color); gizmos.draw_line(p3, p0, color);
                        gizmos.draw_line(p4, p5, color); gizmos.draw_line(p5, p6, color);
                        gizmos.draw_line(p6, p7, color); gizmos.draw_line(p7, p4, color);
                        gizmos.draw_line(p0, p4, color); gizmos.draw_line(p1, p5, color);
                        gizmos.draw_line(p2, p6, color); gizmos.draw_line(p3, p7, color);
                    }
                    gizmo_physics::ColliderShape::Sphere(s) => {
                        let r = s.radius;
                        let min = trans.position - Vec3::new(r, r, r);
                        let max = trans.position + Vec3::new(r, r, r);
                        gizmos.draw_box(min, max, color);
                    }
                    _ => {
                        let min = trans.position - Vec3::new(1.0, 1.0, 1.0);
                        let max = trans.position + Vec3::new(1.0, 1.0, 1.0);
                        gizmos.draw_box(min, max, color);
                    }
                }
            }
        }
        
        let soft_color = [1.0, 0.4, 0.8, 1.0]; // Pinkish for soft body
        if let Some(q) = world.query::<&gizmo_physics::soft_body::SoftBodyMesh>() {
            for (_, sm) in q.iter() {
                for elem in &sm.elements {
                    let p0 = sm.nodes[elem.node_indices[0] as usize].position;
                    let p1 = sm.nodes[elem.node_indices[1] as usize].position;
                    let p2 = sm.nodes[elem.node_indices[2] as usize].position;
                    let p3 = sm.nodes[elem.node_indices[3] as usize].position;
                    
                    // 6 edges of a tetrahedron
                    gizmos.draw_line(p0, p1, soft_color);
                    gizmos.draw_line(p0, p2, soft_color);
                    gizmos.draw_line(p0, p3, soft_color);
                    gizmos.draw_line(p1, p2, soft_color);
                    gizmos.draw_line(p1, p3, soft_color);
                    gizmos.draw_line(p2, p3, soft_color);
                }
            }
        }
    }
}

/// ECS'deki yeni yaratılmış Fiziksel Objeleri (RigidBody + Transform + Collider)
/// GPU Physics çekirdeğinin otoyoluna (GpuPhysicsSystem::spheres_buffer) kaydeder.
/// Statik collider'lar için ayrı sayaç. İlk 3 slot başlangıç collider'larına ayrılmıştır.
static NEXT_STATIC_COLLIDER_SLOT: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(3);

pub fn gpu_physics_submit_system(world: &mut crate::core::World, renderer: &Renderer) {
    use crate::physics::Velocity;

    if let Some(physics) = &renderer.gpu_physics {
        let mut unlinked_entities = Vec::new();
        if let Some(q) = world.query::<(&RigidBody, &Transform, &Collider)>() {
            let links = world.borrow::<GpuPhysicsLink>();
            let velocities = world.borrow::<Velocity>();
            for (e, (rb, trans, col)) in q.iter() {
                if links.get(e).is_none() {
                    let vel = velocities.get(e).map(|v| *v).unwrap_or_default();
                    unlinked_entities.push((e, *rb, *trans, col.clone(), vel));
                }
            }
        }

        let mut next_dynamic_id = world
            .query::<&GpuPhysicsLink>()
            .map(|q| q.iter().count() as u32)
            .unwrap_or(0);

        for (e, rb, trans, col, vel) in unlinked_entities {
            if matches!(col.shape, ColliderShape::Plane(_)) {
                // Statik engel — ayrı slot sayacı kullan
                let slot =
                    NEXT_STATIC_COLLIDER_SLOT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if slot >= 100 {
                    eprintln!("[GpuPhysics] Statik collider slot limiti (100) aşıldı, collider atlanıyor.");
                    NEXT_STATIC_COLLIDER_SLOT.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                    continue;
                }

                let gpu_col = gizmo_renderer::gpu_physics::GpuCollider {
                    shape_type: match col.shape {
                        ColliderShape::Plane(_) => 1,
                        _ => 0, // Varsayılan Box (AABB)
                    },
                    _pad1: [0; 3],
                    data1: match &col.shape {
                        ColliderShape::Plane(p) => [p.normal.x, p.normal.y, p.normal.z, 0.0],
                        ColliderShape::Box(b) => {
                            let min = trans.position - b.half_extents;
                            [min.x, min.y, min.z, 0.0]
                        }
                        _ => [0.0; 4],
                    },
                    data2: match &col.shape {
                        ColliderShape::Plane(p) => [p.distance, 0.0, 0.0, 0.0],
                        ColliderShape::Box(b) => {
                            let max = trans.position + b.half_extents;
                            [max.x, max.y, max.z, 0.0]
                        }
                        _ => [0.0; 4],
                    },
                };
                physics.update_collider(&renderer.queue, slot, &gpu_col);
            } else {
                // Dinamik Kutu (AABB)
                let id = next_dynamic_id;
                next_dynamic_id += 1;

                let extents = match &col.shape {
                    ColliderShape::Box(b) => {
                        [b.half_extents.x, b.half_extents.y, b.half_extents.z]
                    }
                    _ => [0.5, 0.5, 0.5],
                };

                let gpu_box = gizmo_renderer::gpu_physics::GpuBox {
                    position: [trans.position.x, trans.position.y, trans.position.z],
                    mass: rb.mass,
                    velocity: [vel.linear.x, vel.linear.y, vel.linear.z],
                    state: 0,
                    rotation: [
                        trans.rotation.x,
                        trans.rotation.y,
                        trans.rotation.z,
                        trans.rotation.w,
                    ],
                    angular_velocity: [vel.angular.x, vel.angular.y, vel.angular.z],
                    sleep_counter: if rb.is_sleeping { 60 } else { 0 },
                    color: [0.3, 0.8, 1.0, 1.0],
                    half_extents: extents,
                    _pad: 0,
                };
                physics.update_box(&renderer.queue, id, &gpu_box);

                world.add_component(world.get_entity(e).unwrap(), GpuPhysicsLink { id });
            }
        }
    }
}

/// GPU'dan Asenkron (0ms) çekilen devasa Fizik lokasyon durumlarını,
/// Ekrandaki objelerin render edilmesi için ECS'deki Transform'larına kopyalar.
pub fn gpu_physics_readback_system(world: &mut crate::core::World, renderer: &Renderer) {
    if let Some(physics) = &renderer.gpu_physics {
        if let Some(gpu_data) = physics.poll_readback_data(&renderer.device) {
            if let Some(mut q) =
                world.query::<(gizmo_core::prelude::Mut<Transform>, &GpuPhysicsLink)>()
            {
                for (_, (mut trans, link)) in q.iter_mut() {
                    let idx = link.id as usize;
                    if idx < gpu_data.len() {
                        let box_data = &gpu_data[idx];
                        trans.position = gizmo_math::Vec3::new(
                            box_data.position[0],
                            box_data.position[1],
                            box_data.position[2],
                        );
                        trans.rotation = gizmo_math::Quat::from_xyzw(
                            box_data.rotation[0],
                            box_data.rotation[1],
                            box_data.rotation[2],
                            box_data.rotation[3],
                        );
                        trans.update_local_matrix();
                    }
                }
            }
        }
    }
}

/// Phase 7.1: Fluid-Rigid Coupling
/// Senkronize eder: GpuPhysicsLink sahibi objeleri FluidCollider buffer'ına yazar.
pub fn gpu_fluid_coupling_system(world: &crate::core::World, renderer: &mut Renderer) {
    use gizmo_renderer::gpu_fluid::types::FluidCollider;
    use gizmo_renderer::gpu_fluid::types::MAX_FLUID_COLLIDERS;

    if let Some(fluid) = &mut renderer.gpu_fluid {
        let mut colliders = vec![FluidCollider {
            position: [0.0; 3],
            radius: 0.0,
            velocity: [0.0; 3],
            shape_type: 0,
            half_extents: [0.0; 3],
            _pad: 0.0,
        }; MAX_FLUID_COLLIDERS];
        
        let mut count = 0;

        if let Some(q) = world.query::<(&Transform, &crate::physics::Velocity, &Collider)>() {
            for (_, (trans, vel, col)) in q.iter() {
                if count >= MAX_FLUID_COLLIDERS {
                    break;
                }
                
                // Sadece belli y altındaki veya dinamik olanları eklemek isteyebiliriz, ama şimdilik hepsini ekleyelim
                let shape_type;
                let mut radius = 0.0;
                let mut half_extents = [0.0; 3];

                match &col.shape {
                    ColliderShape::Sphere(s) => {
                        shape_type = 0;
                        radius = s.radius;
                    }
                    ColliderShape::Box(b) => {
                        shape_type = 1;
                        half_extents = [b.half_extents.x, b.half_extents.y, b.half_extents.z];
                    }
                    _ => continue, // Sadece Sphere ve Box destekliyoruz
                }

                colliders[count] = FluidCollider {
                    position: [trans.position.x, trans.position.y, trans.position.z],
                    radius,
                    velocity: [vel.linear.x, vel.linear.y, vel.linear.z],
                    shape_type,
                    half_extents,
                    _pad: 0.0,
                };
                count += 1;
            }
        }
        
        // GPU'ya yaz
        renderer.queue.write_buffer(
            &fluid.colliders_buffer,
            0,
            bytemuck::cast_slice(&colliders),
        );
        
        // Fluid Params num_colliders güncelle
        fluid.update_colliders_count(&renderer.queue, count as u32);
    }
}

/// Gelişmiş CPU Fizik motoru entegrasyonu (Menteşeler, Raycast, Joint sistemleri)
/// gizmo-physics içerisindeki PhysicsWorld çalıştırılır ve ECS içerisindeki
/// RigidBody, Transform, Velocity ve Collider verileri senkronize edilir.
pub fn cpu_physics_step_system(world: &mut crate::core::World, dt: f32) {
    if world.get_resource::<gizmo_physics::world::PhysicsWorld>().is_none() {
        return; // Physics plugin is not active.
    }

    // World üzerinden sahipliği geçici olarak al
    let mut phys_world = world
        .remove_resource::<gizmo_physics::world::PhysicsWorld>()
        .unwrap();

    let mut bodies = Vec::new();
    if let Some(q) = world.query::<(
        &crate::physics::RigidBody,
        &crate::physics::Transform,
        &crate::physics::Velocity,
        &crate::physics::Collider,
    )>() {
        for (e, (rb, transform, vel, col)) in q.iter() {
            bodies.push((gizmo_core::entity::Entity::new(e, 0), *rb, *transform, *vel, col.clone()));
        }
    }

    let mut soft_bodies = Vec::new();
    if let Some(q) = world.query::<(
        &gizmo_physics::soft_body::SoftBodyMesh,
        &crate::physics::Transform,
    )>() {
        for (e, (sm, transform)) in q.iter() {
            soft_bodies.push((gizmo_core::entity::Entity::new(e, 0), sm.clone(), *transform));
        }
    }

    // Fizik adımını çalıştır
    phys_world.step(&mut bodies, &mut soft_bodies, dt);

    // Güncellenmiş değerleri geri ECS'e yaz
    if let Some(q) = world.query::<(
        gizmo_core::prelude::Mut<crate::physics::Transform>,
        gizmo_core::prelude::Mut<crate::physics::Velocity>,
        gizmo_core::prelude::Mut<crate::physics::RigidBody>,
    )>() {
        for (e, rb, trans, vel, _) in bodies {
            if let Some((mut t, mut v, mut r)) = q.get(e.id()) {
                *t = trans;
                t.update_local_matrix(); // Görselin güncellenmesi için matrisin yenilenmesi ŞART!
                *v = vel;
                *r = rb;
            }
        }
    }
    
    if let Some(q) = world.query::<(
        gizmo_core::prelude::Mut<crate::physics::Transform>,
        gizmo_core::prelude::Mut<gizmo_physics::soft_body::SoftBodyMesh>,
    )>() {
        for (e, sm, trans) in soft_bodies {
            if let Some((mut t, mut s)) = q.get(e.id()) {
                *t = trans;
                t.update_local_matrix();
                *s = sm;
            }
        }
    }

    // Kaynağı geri koy
    world.insert_resource(phys_world);
}
