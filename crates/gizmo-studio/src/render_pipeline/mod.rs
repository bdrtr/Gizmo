use crate::StudioState;
use gizmo::prelude::*;
use std::cell::RefCell;

mod batching;
mod passes;
use batching::*;
use passes::*;

pub fn execute_render_pipeline(
    world: &mut World,
    state: &StudioState,
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    renderer: &mut gizmo::renderer::Renderer,
    _light_time: f32,
) {
    // --- SKELETAL ANIMATION UPDATE (Done before any ECS borrows!) ---
    let delta_time = state.actual_dt;
    gizmo::renderer::animation_update_system(world, delta_time, &renderer.queue);
    
    let mut bone_att = gizmo::systems::transform::BoneAttachmentSystem;
    gizmo::core::system::System::run(&mut bone_att, world, delta_time);

    let (aspect, ed_shading_mode, show_colliders, post_params) =
        sync_editor_settings(world, renderer);

    let mut proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, aspect, 0.1, 2000.0);
    let mut view_mat = Mat4::from_translation(Vec3::ZERO);
    let mut cam_pos = Vec3::ZERO;
    let mut cam_near = 0.1f32;
    let mut cam_far = 2000.0f32;
    let mut cam_fov = std::f32::consts::FRAC_PI_4;
    let mut cam_forward = Vec3::new(0.0, 0.0, -1.0);
    let _is_hidden_guard = world.borrow::<gizmo::core::component::IsHidden>();

    let cameras = world.borrow::<Camera>();
    let transforms = world.borrow::<Transform>();

    // Play modunda Game Camera, Edit modunda Editor Camera kullan
    let is_playing_mode = world.get_resource::<gizmo::editor::EditorState>()
        .map(|ed| ed.is_playing() || ed.mode == gizmo::editor::EditorMode::Paused)
        .unwrap_or(false);

    let active_camera_id = if is_playing_mode && cameras.get(state.game_camera).is_some() {
        state.game_camera
    } else {
        state.editor_camera
    };

    {
        if let (Some(cam), Some(trans)) = (
            cameras.get(active_camera_id),
            transforms.get(active_camera_id),
        ) {
            proj = cam.get_projection(aspect);
            view_mat = cam.get_view(trans.position);
            cam_pos = trans.position;
            cam_near = cam.near;
            cam_far = cam.far;
            cam_fov = cam.fov;
            cam_forward = cam.get_front();
        }
    }

    let view_proj = proj * view_mat;

    // Event: Spawning moved to spawner_update_system.
    // Event: Texture Loading moved to main render loop pass before execute_render_pipeline.

    // z = elapsed time for fluid caustics/wave animation (fluid_composite.wgsl reads it);
    // was hardcoded 0.0 → frozen water (same bug as the gizmo runtime path).
    let elapsed_time = world
        .get_resource::<gizmo::core::time::Time>()
        .map(|t| t.elapsed() as f32)
        .unwrap_or(0.0);

    // The active camera, in the form both uniform blocks are built from. The post block gets it
    // too — its `cam_near`/`cam_far` were hardcoded 0.1/2000 here, which mis-linearised depth for
    // DoF on any editor or game camera with a different range.
    let camera = gizmo::renderer::CameraFrame {
        view_proj,
        position: cam_pos,
        forward: cam_forward,
        near: cam_near,
        far: cam_far,
        // Exposure reaches the picture through the post block, which the editor drives from its
        // own slider; the scene block's copy is unread (see `SceneUniforms::new`).
        exposure: post_params.exposure,
    };
    renderer.update_post_process(&renderer.queue, post_params.with_camera(&camera));

    // Lights, cascades and the scene block — the SAME helper the game path calls. Everything the
    // editor does differently is an argument here, with its reason, and `tests/render_parity.rs`
    // holds the two argument sets side by side. This block used to be forty lines of light
    // conversion, shadow-direction choice and cascade fallback written out a second time.
    let setup = gizmo::systems::render::collect_scene_setup(
        world,
        &gizmo::systems::render::SceneSetupInputs {
            camera,
            aspect,
            cam_fov,
            // A scene being lit by hand often has no sun yet, and an editor viewport with no
            // shadows at all reads as broken rather than as unlit.
            shadow_caster: gizmo::systems::render::ShadowCaster::SunOrFirstLight,
            // The editor renders the scene as authored: no environment preset blend, and the
            // shading mode comes from the viewport's debug dropdown, not from the renderer.
            environment: gizmo::renderer::EnvironmentFrame {
                shading_mode: ed_shading_mode,
                ..Default::default()
            },
            // The editor records no point-shadow cube pass, so there is no cube to sample;
            // enabling the lookup would read whatever the game path left in it.
            point_shadows_enabled: false,
            elapsed_time,
        },
    );
    let cascade_mats = setup.cascade_view_projs;
    let light_view_proj_cascades = cascade_mats.map(|m| m.to_cols_array_2d());

    // Global Uniforms (Her frame sadece 1 kere gönderilir).
    let scene_uniform_data = gizmo::renderer::SceneUniforms::new(&setup.frame);
    renderer.queue.write_buffer(
        &renderer.scene.global_uniform_buffer,
        0,
        gizmo::bytemuck::cast_slice(&[scene_uniform_data]),
    );

    // --- BATCHING (INSTANCING) HAZIRLIĞI VE FRUSTUM CULLING ---
    use gizmo::renderer::renderer::InstanceRaw;

    // --- GAME CAMERA FRUSTUM HESAPLAMA (Görselleştirme için) ---
    let mut game_view_proj = None;
    if !is_playing_mode {
        if let (Some(cam), Some(trans)) = (
            cameras.get(state.game_camera),
            transforms.get(state.game_camera),
        ) {
            let p = cam.get_projection(aspect);
            let v = cam.get_view(trans.position);
            game_view_proj = Some(p * v);
        }
    }

    let frustum = gizmo::renderer::Frustum::from_matrix(&view_proj);
    let game_frustum = game_view_proj.map(|vp| gizmo::renderer::Frustum::from_matrix(&vp));

    // Frustum Culling için her zaman Game Camera'yı kullanalım (Edit modunda da culling test edebilmek için)
    let culling_frustum = game_frustum.unwrap_or(frustum);

    // Per-cascade LIGHT frusta — shadow casters are culled against these (not the camera
    // frustum), so off-screen objects that cast shadows INTO view aren't dropped.
    let cascade_frusta: [gizmo::renderer::Frustum; 4] = std::array::from_fn(|i| {
        gizmo::renderer::Frustum::from_matrix(&Mat4::from_cols_array_2d(&light_view_proj_cascades[i]))
    });

    let mut debug_aabbs = Vec::new();

    CACHE.with(|cache_ref| {
        let mut cache = cache_ref.borrow_mut();
        let PipelineCache {
            opaque_batches,
            opaque_double_sided_batches,
            transparent_batches,
            all_instances,
            flat_batches,
            vec_pool,
        } = &mut *cache;

        for (_, mut b) in opaque_batches.drain() {
            b.instances.clear();
            vec_pool.push(b.instances);
        }
        for (_, mut b) in opaque_double_sided_batches.drain() {
            b.instances.clear();
            vec_pool.push(b.instances);
        }
        for (_, mut b) in transparent_batches.drain() {
            b.instances.clear();
            vec_pool.push(b.instances);
        }
        all_instances.clear();
        flat_batches.clear();

        let renderers = world.borrow::<gizmo::renderer::components::MeshRenderer>();
        let skeletons = world.borrow::<gizmo::renderer::components::Skeleton>();
        let lod_groups = world.borrow::<gizmo::renderer::components::LodGroup>();

        if let Some(mut q) = world.query::<(&Mesh, &gizmo::physics::components::GlobalTransform, &Material)>() {
            for (e, (mesh, global_trans, mat)) in q.iter_mut() {
                // Sadece MeshRenderer tagli olanları çiz
                if renderers.get(e).is_none() {
                    continue;
                }

                // Gizli olarak işaretlenmiş objeleri atla
                if _is_hidden_guard.contains(e) {
                    continue;
                }

                // --- GLOBAL TRANSFORM HESAPLAMA ---
                // ECS transform senkronizasyonu GlobalTransform'u güncellediği için doğrudan onu kullanıyoruz.
                let global_model = global_trans.matrix;

                let center_mat = Mat4::from_translation(mesh.center_offset);
                let model = global_model * center_mat;

                // Frustum Culling (AABB vs view-projection frustum). Camera visibility
                // drives the MAIN passes (unchanged). A shadow CASTER outside the camera
                // frustum is still kept if it falls in any cascade's LIGHT frustum, so it
                // casts a shadow into view (drawn into shadow maps only — see below).
                // Shared with the game path so the cull test + caster predicate stay in
                // lockstep (camera-visible → main passes; off-screen caster inside a
                // cascade → shadow maps only; else skip). Culls against the game camera
                // in edit mode (culling_frustum).
                // A camera-locked backdrop is exempt from the frustum test. It is drawn AROUND
                // the viewer by construction (`backdrop.wgsl` adds the camera position), so
                // its authored matrix says nothing about where it lands — and unlike the game
                // path, this one cannot simply cull against the locked transform instead:
                // `culling_frustum` is deliberately the GAME camera's while the shader locks
                // to the ACTIVE (editor) camera, and in edit mode those are different places.
                // There is no matrix that makes the test meaningful here, so it is skipped.
                // One `Aabb::transform` for the camera test AND every cascade test (and the
                // debug-AABB push below reuses it) — `classify_visibility` redid it per frustum.
                let world_aabb = mesh.bounds.transform(&model);
                let routing = gizmo::renderer::routing::route(mat.material_type);
                let camera_visible = if gizmo::renderer::is_camera_locked(mat.material_type) {
                    true
                } else {
                    match gizmo::renderer::classify_visibility_world(
                        &culling_frustum,
                        &cascade_frusta,
                        world_aabb,
                        mat.material_type,
                        mat.is_transparent,
                        mat.albedo.w,
                    ) {
                        gizmo::renderer::Visibility::Culled => continue,
                        gizmo::renderer::Visibility::Camera => true,
                        gizmo::renderer::Visibility::ShadowOnly => false,
                    }
                };

                // Culling'i geçen objelerin Bounding Box'larını debug çizimi için kaydet.
                // Skybox ve Grid'i hariç tut: bounds'ları tüm sahneyi sardığı için
                // kırmızı AABB'leri ekranı baştan başa kesen dev çizgiler olarak görünüyordu.
                if !is_playing_mode
                    && !matches!(
                        mat.material_type,
                        gizmo::renderer::components::MaterialType::Skybox
                            // Same reason as Skybox: a backdrop wraps the whole view, and its
                            // AABB is drawn at the authored position the GPU never uses. A
                            // PLACED backdrop is at its authored position — but it is still a
                            // painting pinned to the far plane, so it is skipped here for the
                            // same reason and not for that one.
                            | gizmo::renderer::components::MaterialType::Backdrop
                            | gizmo::renderer::components::MaterialType::BackdropPlaced
                            | gizmo::renderer::components::MaterialType::Grid
                    )
                {
                    debug_aabbs.push(world_aabb);
                }

                // --- LOD (Level of Detail) SEÇİMİ ---
                // Hangi mesh çizilecek sorusunun cevabı ortak (`LodGroup::pick`); bize ait olan
                // yalnız mesafenin nereden ölçüldüğü — burada birleştirilmiş model matrisinden.
                let dist = cam_pos.distance(Vec3::new(model.w_axis.x, model.w_axis.y, model.w_axis.z));
                let active_mesh = match gizmo::renderer::components::LodGroup::pick(
                    lod_groups.get(e),
                    mesh,
                    dist,
                ) {
                    Some(m) => m,
                    None => continue, // CULL edildi!
                };

                let instance_data = InstanceRaw::new(
                    model.to_cols_array_2d(),
                    [mat.albedo.x, mat.albedo.y, mat.albedo.z, mat.albedo.w],
                    mat.roughness,
                    mat.metallic,
                    routing.instance_flag,
                    mat.anisotropy,
                    mat.clear_coat,
                    mat.subsurface,
                    mat.ambient.to_array(),
                    mat.emissive.to_array(),
                );

                // --- SKELETON (KEMİK) ARAMASI ---
                // Skeleton bind group, skinned mesh'ler spawn edilirken doğrudan entity'ye önbelleklenmelidir.
                // Bu nedenle her frame parent zincirini tırmanıp Skeleton aramak yerine doğrudan kendi üzerindekini kullanıyoruz.
                let mut skel_bg = renderer.scene.dummy_skeleton_bind_group.clone();
                if let Some(s) = skeletons.get(e) {
                    skel_bg = s.bind_group.clone();
                }

                let vbuf_ptr = std::sync::Arc::as_ptr(&active_mesh.vbuf);
                let bg_ptr = std::sync::Arc::as_ptr(&mat.bind_group);
                let skel_ptr = std::sync::Arc::as_ptr(&skel_bg);

                // Routing flags — part of the batch key (see BatchKey docs) so a
                // shared cached texture bind group can't merge materials that route
                // or cast shadows differently.
                // One decision, in `gizmo-renderer::routing`, shared with the engine's own draw
                // loop. This match used to be written out here with a `_ => 0.0` arm, and the
                // engine's copy had a different set of arms: `BakedLit` was routed there and fell
                // through the wildcard here, so a baked-lit level shaded one way in the game and
                // another in this editor. Reading the shared answer is what fixes that — and it is
                // a **visible change to this viewport**, which nothing here can test.
                let is_skybox = routing.is_skybox;
                let is_grid = routing.is_grid;
                let is_unlit = routing.unlit_material;
                let is_backdrop = routing.is_backdrop;

                let batches = if mat.is_transparent {
                    &mut *transparent_batches
                } else if mat.is_double_sided {
                    &mut *opaque_double_sided_batches
                } else {
                    &mut *opaque_batches
                };

                let batch = batches
                    .entry((vbuf_ptr, bg_ptr, skel_ptr, is_skybox, is_grid, is_unlit, is_backdrop))
                    .or_insert_with(|| BatchData {
                        vbuf: active_mesh.vbuf.clone(),
                        vertex_count: active_mesh.vertex_count,
                        // Studio'nun LOD'u ayrı bir `Mesh` seçiyor (`lods` bileşeni), motorun
                        // `lod_vbufs` düzleştirilmiş tamponları değil — dolayısıyla seçilen
                        // mesh'in indeksleri kendi vertex dizisine göre geçerli ve olduğu gibi
                        // taşınabilir. Ana boru hattındaki düşürme kuralı buraya UYMAZ.
                        ibuf: active_mesh.ibuf.clone(),
                        index_count: active_mesh.index_count,
                        index_format: active_mesh.index_format,
                        bind_group: mat.bind_group.clone(),
                        skeleton_bg: skel_bg,
                        instances: vec_pool.pop().unwrap_or_else(|| Vec::with_capacity(32)),
                        shadow_instances: Vec::new(),
                        is_skybox,
                        is_grid,
                        is_unlit,
                        is_backdrop,
                    });

                if camera_visible {
                    batch.instances.push(instance_data);
                } else {
                    // Off-screen caster kept above for shadow maps only.
                    batch.shadow_instances.push(instance_data);
                }
            }
        }

        let process_batches =
            |batches: &mut std::collections::HashMap<BatchKey, BatchData>,
             is_transparent: bool,
             is_double_sided: bool,
             all_inst: &mut Vec<gizmo::renderer::InstanceRaw>,
             flat_b: &mut Vec<FlatBatchData>,
             vec_pool: &mut Vec<Vec<gizmo::renderer::InstanceRaw>>| {
                // Drain the batches. Transparent batches must be ordered back-to-front
                // RELATIVE TO EACH OTHER (inter-batch) too: they drain from a HashMap in
                // arbitrary order, so overlapping transparents of DIFFERENT materials
                // (= different batches) composited wrongly. Instances WITHIN a transparent
                // batch are also sorted back-to-front (intra-batch, as before).
                let mut drained: Vec<BatchData> = batches.drain().map(|(_, b)| b).collect();
                if is_transparent {
                    for batch in &mut drained {
                        batch.instances.sort_by(|a, b| {
                            let pos_a = Vec3::new(a.model[3][0], a.model[3][1], a.model[3][2]);
                            let pos_b = Vec3::new(b.model[3][0], b.model[3][1], b.model[3][2]);
                            // Uzak olanlar ÖNCE çizilmeli (Azalan sıralama).
                            cam_pos
                                .distance_squared(pos_b)
                                .partial_cmp(&cam_pos.distance_squared(pos_a))
                                .unwrap_or(std::cmp::Ordering::Equal)
                        });
                    }
                    // Inter-batch: compute each batch's centroid depth once, farthest first.
                    let mut keyed: Vec<(f32, BatchData)> = drained
                        .into_iter()
                        .map(|b| (batch_centroid_depth(&b.instances, cam_pos), b))
                        .collect();
                    keyed.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
                    drained = keyed.into_iter().map(|(_, b)| b).collect();
                }

                for mut batch in drained {
                    let start = all_inst.len() as u32;
                    // Camera-visible instances FIRST (main passes draw up to end_instance),
                    // then off-screen shadow casters (shadow pass draws up to shadow_end_instance).
                    let count = batch.instances.len() as u32;
                    all_inst.append(&mut batch.instances);
                    vec_pool.push(batch.instances); // Empty vec with capacity is pushed back!
                    let shadow_count = batch.shadow_instances.len() as u32;
                    all_inst.append(&mut batch.shadow_instances);

                    flat_b.push(FlatBatchData {
                        vbuf: batch.vbuf,
                        vertex_count: batch.vertex_count,
                        ibuf: batch.ibuf,
                        index_count: batch.index_count,
                        index_format: batch.index_format,
                        bind_group: batch.bind_group,
                        skeleton_bg: batch.skeleton_bg,
                        start_instance: start,
                        end_instance: start + count,
                        shadow_end_instance: start + count + shadow_count,
                        is_transparent,
                        is_double_sided,
                        is_skybox: batch.is_skybox,
                        is_grid: batch.is_grid,
                        is_unlit: batch.is_unlit,
                        is_backdrop: batch.is_backdrop,
                    });
                }
            };

        // Process
        process_batches(
            opaque_batches,
            false,
            false,
            all_instances,
            flat_batches,
            vec_pool,
        );
        process_batches(
            opaque_double_sided_batches,
            false,
            true,
            all_instances,
            flat_batches,
            vec_pool,
        );
        process_batches(
            transparent_batches,
            true,
            false,
            all_instances,
            flat_batches,
            vec_pool,
        );


        if !all_instances.is_empty() {
            renderer.ensure_instance_capacity(all_instances.len());
            renderer.queue.write_buffer(
                &renderer.scene.instance_buffer,
                0,
                gizmo::bytemuck::cast_slice(all_instances),
            );
        }

        // --- 0. COMPUTE PASSES ---
        if let Some(gpu_particles) = &renderer.gpu_particles {
            gpu_particles.update_params(&renderer.queue, delta_time, 0.0); // time (curl-noise) studio'da kullanılmıyor

            // --- YENİ PARTİCÜL SPAWNLAMA (CPU -> GPU) ---
            //
            // Bu blok yıllarca YALNIZCA burada durdu, yani motorun kendi geçidini kullanan bir
            // oyunda `ParticleEmitter` hiçbir şey yaymıyordu. Motor tarafı artık kendi kopyasını
            // taşıyor: `gizmo::systems::render::spawn_from_emitters` (2026-08-14).
            //
            // Buradan ONA çağrı yapılamıyor ve sebebi teknik: o fonksiyon `&mut World` alıyor —
            // ödünçleme önkoşulu olmayan, sağlam bir imza — oysa bu boru hattı blok boyunca canlı
            // bir okuma ödünçlemesi (`_is_hidden_guard`, satır ~34) tutuyor. `&World` alan bir
            // varyant ikisini de idare ederdi ama çağıranın hangi ödünçlemeleri tuttuğuna bağlı,
            // koşullu güvenli bir genel API olurdu; o takas bu tekrardan kötü. Tekrar burada
            // yazılı duruyor ki gizli kalmasın: ikisi birlikte değişmeli.
            // Collect emitter entities up front through a read borrow that is dropped at the
            // end of this statement, so the mutable ParticleEmitter query below never coexists
            // with a same-type read borrow.
            let emitter_entities: Vec<u32> = world
                .borrow::<gizmo::renderer::components::ParticleEmitter>()
                .entities()
                .collect();
            // SAFETY: exclusive `&mut World`; ParticleEmitter is a distinct component type from
            // the read-only Transform query below, and the read borrow above is already dropped,
            // so this mutable query never aliases another live access to the same storage.
            let mut emitters =
                unsafe { world.borrow_mut_unchecked::<gizmo::renderer::components::ParticleEmitter>() };
            {
                let transforms = world.borrow::<Transform>();
                {
                    use rand::Rng;
                    let mut rng = rand::rng();
                    let mut all_new_particles = Vec::new();

                    for e_id in emitter_entities {
                        if let Some(mut emitter) = emitters.get_mut(e_id) {
                            if !emitter.is_active || emitter.spawn_rate <= 0.0 {
                                continue;
                            }

                            let base_pos = if let Some(t) = transforms.get(e_id) {
                                t.position + t.rotation.mul_vec3(emitter.local_offset)
                            } else {
                                emitter.local_offset
                            };

                            emitter.add_time(delta_time);
                            // Güvenlik limiti: Frame drop olursa bir frame'de 100'den fazla spawnlamasın
                            // Aksi takdirde 1 saniye donup binlerce üreterek FPS'i çökertir
                            let spawn_interval = 1.0 / emitter.spawn_rate;
                            let mut spawned_this_frame = 0;

                            while emitter.get_accumulator() >= spawn_interval
                                && spawned_this_frame < 100
                            {
                                emitter.consume_time(spawn_interval);
                                spawned_this_frame += 1;

                                let rand_v_x =
                                    rng.random_range(-1.0..=1.0) * emitter.velocity_randomness;
                                let rand_v_y =
                                    rng.random_range(-1.0..=1.0) * emitter.velocity_randomness;
                                let rand_v_z =
                                    rng.random_range(-1.0..=1.0) * emitter.velocity_randomness;

                                let out_dir = Vec3::new(rand_v_x, rand_v_y, rand_v_z);
                                let vel = emitter.initial_velocity + out_dir;

                                let rand_life =
                                    rng.random_range(-1.0..=1.0) * emitter.lifespan_randomness;
                                let max_life = (emitter.lifespan + rand_life).max(0.1);

                                all_new_particles.push(
                                    gizmo::renderer::gpu_particles::GpuParticle {
                                        position: [base_pos.x, base_pos.y, base_pos.z],
                                        life: 0.0,
                                        velocity: [vel.x, vel.y, vel.z],
                                        max_life,
                                        color: emitter.color_start.into(),
                                        size_start: emitter.size_start,
                                        size_end: emitter.size_end,
                                        _padding: [0.0; 2],
                                    },
                                );
                            }
                        }
                    }

                    gpu_particles.spawn_particles(&renderer.queue, &all_new_particles);
                }
            }

            gpu_particles.compute_pass(encoder, gpu_particles.active_particles);
        }

        if let Some(physics) = &renderer.gpu_physics {
            physics.set_debug_flags(&renderer.queue, if show_colliders { 1 } else { 0 });
            physics.compute_pass(encoder);
            if show_colliders {
                physics.debug_compute_pass(encoder);
            }
        }

            // --- 1. CSM GÖLGE PASS + 2. ANA RENDER PASS (Tier 3: geçişler ayrı fn) ---
            record_studio_shadow_passes(encoder, renderer, flat_batches.as_slice(), &light_view_proj_cascades);
            record_studio_main_pass(
                encoder, renderer, world, flat_batches.as_slice(), game_view_proj, &debug_aabbs, show_colliders,
            );
    }); // Cikis: CACHE.with bloğu

    // Çizilen Gizmo'ları sonraki frame için temizle
    if let Some(mut gizmos) = world.get_resource_mut::<gizmo::renderer::Gizmos>() {
        gizmos.clear();
    }

    // --- 3. POST-PROCESSING (Bloom + Tone Mapping → Ekrana Yaz) ---
    let render_target = world.get_resource::<gizmo::renderer::components::EditorRenderTarget>();
    let output_view = if let Some(target) = &render_target {
        // Ana ekranı siyah ile mecburi temizleyelim (Swapchain error önleme)
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Clear Swapchain Background Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        &target.0.view
    } else {
        view
    };

    renderer.run_post_processing(encoder, output_view);

    // Game View RTT: Post-processing çıktısını GameRenderTarget'a da yaz
    let game_target = world.get_resource::<gizmo::renderer::components::GameRenderTarget>();
    if let Some(target) = &game_target {
        renderer.run_post_processing(encoder, &target.0.view);
    }
}



