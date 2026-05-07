use crate::core::World;
use crate::math::{Mat4, Vec3};
use crate::physics::Transform;
use crate::renderer::{
    components::{Camera, Material, Mesh, MeshRenderer},
    Renderer,
};
use bytemuck;
use wgpu;
use super::physics::*;


#[derive(Clone)]
pub struct DrawItem {
    vbuf: std::sync::Arc<wgpu::Buffer>,
    vertex_count: u32,
    bind_group: std::sync::Arc<wgpu::BindGroup>,
    unlit: bool,
    is_skybox: bool,
    first_instance: u32,
    instance_count: u32,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct BatchKey {
    vbuf_id: usize,
    mat_id: usize,
}

struct BatchData {
    vbuf: std::sync::Arc<wgpu::Buffer>,
    bind_group: std::sync::Arc<wgpu::BindGroup>,
    vertex_count: u32,
    unlit: bool,
    is_skybox: bool,
    instances: Vec<crate::renderer::gpu_types::InstanceRaw>,
}

/// Bevy'nin DefaultPlugins davranisini taklit eden, sadece modelleri
/// isiklandirip hizlica ekrana basmaya yarayan kutudan cikmis Render Motoru.
/// Yeni acilan `tut` gibi bos projelerde yuzlerce satir kod yazmamak icin kullanilir.
#[tracing::instrument(skip_all, name = "render_system")]
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
        shading_mode: 0,
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

#[derive(Default)]
pub struct RenderCache {
    pub batches: std::collections::HashMap<BatchKey, BatchData>,
    pub instances: Vec<crate::renderer::gpu_types::InstanceRaw>,
    pub draw_items: Vec<DrawItem>,
}

// ... inside default_render_pass ...
    // ... before line 205 ...
    let renderers = world.borrow::<MeshRenderer>();
    
    // Get or create RenderCache
    let frustum = crate::math::Frustum::from_matrix(&unjittered_view_proj);
    
    thread_local! {
        static RENDER_CACHE: std::cell::RefCell<RenderCache> = std::cell::RefCell::new(RenderCache::default());
    }

    let draw_items = RENDER_CACHE.with(|rc| {
        let mut cache = rc.borrow_mut();
        
        // Clear instances but keep allocations
        for batch in cache.batches.values_mut() {
            batch.instances.clear();
        }
        cache.instances.clear();
        cache.draw_items.clear();

        let pooled_storage = world.borrow::<gizmo_core::pool::Pooled>();
        if let Some(mut q) = world.query::<(&Mesh, &Transform, &Material)>() {
            for (e, (mesh, trans, mat)) in q.iter_mut() {
                if renderers.get(e).is_none() {
                    continue;
                }
                
                // Pooled (havuzda pasif) nesneleri render etme
                if pooled_storage.get(e).is_some() {
                    continue;
                }

                let center_mat = Mat4::from_translation(mesh.center_offset);
                let model = trans.local_matrix * center_mat;

                // CPU Frustum Culling
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

                if !frustum.intersects_sphere(world_c, world_r) {
                    continue;
                }

                // Auto-LOD (Level of Detail) Seçimi
                let dist_to_cam = (world_c - cam_pos).length();
                let use_lod1 = if !mesh.lod_vbufs.is_empty() {
                    dist_to_cam > world_r * 15.0 // Nesne boyutuna göre uzaklaştıkça LOD1'e geç (örneğin 2m çapında bir nesne 30m uzaktayken geç)
                } else {
                    false
                };

                let active_vbuf = if use_lod1 {
                    mesh.lod_vbufs[0].clone()
                } else {
                    mesh.vbuf.clone()
                };
                let active_vertex_count = if use_lod1 {
                    mesh.lod_vertex_counts[0]
                } else {
                    mesh.vertex_count
                };

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
                
                let key = BatchKey {
                    vbuf_id: std::sync::Arc::as_ptr(&active_vbuf) as usize,
                    mat_id: std::sync::Arc::as_ptr(&mat.bind_group) as usize,
                };

                let batch = cache.batches.entry(key).or_insert_with(|| BatchData {
                    vbuf: active_vbuf.clone(),
                    bind_group: mat.bind_group.clone(),
                    vertex_count: active_vertex_count,
                    unlit: mat.material_type == crate::renderer::components::MaterialType::Unlit
                        || mat.material_type == crate::renderer::components::MaterialType::Skybox,
                    is_skybox: mat.material_type == crate::renderer::components::MaterialType::Skybox,
                    instances: Vec::new(),
                });
                batch.instances.push(instance_data);
            }
        }
        
        let mut local_instances = std::mem::take(&mut cache.instances);
        let mut local_draw_items = std::mem::take(&mut cache.draw_items);

        for (_, batch) in cache.batches.iter() {
            if batch.instances.is_empty() { continue; }
            let first_instance = local_instances.len() as u32;
            let instance_count = batch.instances.len() as u32;
            local_instances.extend(&batch.instances);
            
            local_draw_items.push(DrawItem {
                vbuf: batch.vbuf.clone(),
                vertex_count: batch.vertex_count,
                bind_group: batch.bind_group.clone(),
                unlit: batch.unlit,
                is_skybox: batch.is_skybox,
                first_instance,
                instance_count,
            });
        }
        
        cache.instances = local_instances;
        cache.draw_items = local_draw_items;

        // Instance limiti kontrolü (Taşmaları önlemek için capaciteyi zorla)
        let max_instances = renderer.scene.instance_capacity as usize;
        let instances_slice = if cache.instances.len() > max_instances {
            &cache.instances[..max_instances]
        } else {
            &cache.instances
        };

        if !instances_slice.is_empty() {
            renderer.queue.write_buffer(
                &renderer.scene.instance_buffer,
                0,
                bytemuck::cast_slice(instances_slice),
            );
        }
        
        // Pass draw_items to rendering logic by cloning the small struct (Arc clones are cheap)
        cache.draw_items.clone()
    });
    // CPU Batched Instancing replaces GPU cull for draw_items

    if let Some(physics) = &renderer.gpu_physics {
        // Her frame başında sıradaki state'i çekmek için WGPU CommandEncoder'a asenkron mapping iste.
        physics.request_readback(encoder);

        physics.compute_pass(encoder);
        physics.debug_compute_pass(encoder);
        physics.cull_pass(encoder, &renderer.scene.global_bind_group);
    }

    // Compute LOD (Level of Detail) Scaling
    let fluid_pos = Vec3::new(0.0, 5.0, 0.0);
    let dist_to_fluid = (cam_pos - fluid_pos).length();
    let fluid_lod = if dist_to_fluid < 40.0 { 1.0 } else if dist_to_fluid < 80.0 { 0.5 } else if dist_to_fluid < 150.0 { 0.1 } else { 0.0 };

    let dist_to_origin = cam_pos.length();
    let particle_lod = if dist_to_origin < 50.0 { 1.0 } else if dist_to_origin < 100.0 { 0.5 } else if dist_to_origin < 200.0 { 0.1 } else { 0.0 };

    // Gpu Fluid Processing
    if let Some(fluid) = &renderer.gpu_fluid {
        let active_fluid = (fluid.num_particles as f32 * fluid_lod) as u32;
        fluid.compute_pass(encoder, &renderer.queue, true, active_fluid);
    }

    // Gpu Particles Processing
    if let Some(particles) = &renderer.gpu_particles {
        let active_parts = (particles.max_particles as f32 * particle_lod) as u32;
        let dt = world.get_resource::<gizmo_core::time::Time>().map(|t| t.time_scale() * 0.016).unwrap_or(0.016);
        particles.update_params(&renderer.queue, dt); // Scale based on time_scale
        particles.compute_pass(encoder, active_parts);
    }

    // GPU cull pass removed since we use CPU instancing

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
        for item in &draw_items {
            if item.unlit {
                continue;
            }
            shadow_pass.set_vertex_buffer(0, item.vbuf.slice(..));
            shadow_pass.draw(0..item.vertex_count, item.first_instance..(item.first_instance + item.instance_count));
        }
    }

    // ── Z-Prepass (Depth Only) ────────────────────────────────────────────────
    if let Some(ref def) = renderer.deferred {
        let mut z_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Z-Prepass"),
            color_attachments: &[], // No color targets
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
        z_pass.set_pipeline(&def.z_prepass_pipeline);
        z_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
        z_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
        z_pass.set_bind_group(3, &renderer.scene.dummy_skeleton_bind_group, &[]);
        z_pass.set_bind_group(4, &renderer.scene.instance_bind_group, &[]);
        for item in &draw_items {
            if item.unlit || item.is_skybox {
                continue;
            }
            z_pass.set_bind_group(1, &item.bind_group, &[]);
            z_pass.set_vertex_buffer(0, item.vbuf.slice(..));
            z_pass.draw(0..item.vertex_count, item.first_instance..(item.first_instance + item.instance_count));
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
                    load: wgpu::LoadOp::Load, // Z-Prepass populated this!
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
        for item in &draw_items {
            if item.unlit {
                continue;
            }
            gbuf_pass.set_bind_group(1, &item.bind_group, &[]);
            gbuf_pass.set_vertex_buffer(0, item.vbuf.slice(..));
            gbuf_pass.draw(0..item.vertex_count, item.first_instance..(item.first_instance + item.instance_count));
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

        for item in &draw_items {
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
            render_pass.draw(0..item.vertex_count, item.first_instance..(item.first_instance + item.instance_count));
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
            let active_parts = (particles.max_particles as f32 * particle_lod) as u32;
            particles.render_pass(&mut render_pass, &renderer.scene.global_bind_group, active_parts);
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
        let active_fluid = (fluid.num_particles as f32 * fluid_lod) as u32;
        fluid.render_ssfr(
            encoder,
            &renderer.post.hdr_texture,
            &renderer.post.hdr_texture_view,
            &renderer.depth_texture_view,
            &renderer.scene.global_bind_group,
            active_fluid,
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

    // ── SSGI: Screen Space Global Illumination ────────────────────────────────
    if let Some(ref ssgi) = renderer.ssgi {
        // Pass 1: SSGI Raymarch
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("SSGI Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &ssgi.ssgi_view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&ssgi.ssgi_pipeline);
            pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
            pass.set_bind_group(1, &ssgi.ssgi_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        // Pass 2: SSGI Blur
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("SSGI Blur Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &ssgi.ssgi_blurred_view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&ssgi.blur_pipeline);
            pass.set_bind_group(0, &ssgi.blur_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        // Pass 3: SSGI Apply (Additive blend into HDR texture)
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("SSGI Apply Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &renderer.post.hdr_texture_view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&ssgi.apply_pipeline);
            pass.set_bind_group(0, &ssgi.apply_bind_group, &[]);
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

// ============================================================
//  RenderContext Kolaylık Metodu
//  `ctx.default_render(world)` ile varsayılan pipeline çalışır.
// ============================================================

/// `RenderContext` üzerine eklenen kolaylık metodları.
/// `use gizmo::prelude::*;` ile otomatik olarak dahil edilir.
pub trait RenderContextExt {
    /// Motorun varsayılan render pipeline'ını çalıştırır.
    /// Deferred rendering, gölgeler, SSAO, SSR, TAA ve post-processing dahildir.
    ///
    /// ```ignore
    /// fn render(world: &mut World, _state: &GameState, ctx: &mut RenderContext) {
    ///     ctx.disable_gpu_compute();
    ///     ctx.default_render(world);
    /// }
    /// ```
    fn default_render(&mut self, world: &mut crate::core::World);
}

impl<'a, 'r> RenderContextExt for crate::renderer::RenderContext<'a, 'r> {
    fn default_render(&mut self, world: &mut crate::core::World) {
        let (encoder, view, renderer) = self.parts_mut();
        default_render_pass(world, encoder, view, renderer);
    }
}

