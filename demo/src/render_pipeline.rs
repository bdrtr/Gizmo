use crate::state::GameState;
use gizmo::prelude::*;

pub fn execute_render_pipeline(
    world: &mut World,
    state: &GameState,
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    renderer: &mut gizmo::renderer::Renderer,
    _light_time: f32,
) {
    let aspect = if renderer.size.height > 0 {
        renderer.size.width as f32 / renderer.size.height as f32
    } else {
        1.0
    };

    let mut proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, aspect, 0.1, 2000.0);
    let mut view_mat = Mat4::from_translation(Vec3::ZERO);
    let mut cam_pos = Vec3::ZERO;
    let _is_hidden_guard = world.borrow::<gizmo::core::component::IsHidden>();

    if let (Some(cameras), Some(mut transforms)) =
        (world.borrow::<Camera>(), world.borrow_mut::<Transform>())
    {
        if let (Some(cam), Some(trans)) = (
            cameras.get(state.player_id),
            transforms.get(state.player_id),
        ) {
            proj = cam.get_projection(aspect);
            view_mat = cam.get_view(trans.position);
            cam_pos = trans.position;
        }
        // Skybox her zaman Kamerayla aynı yerde durarak sonsuzluk hissi yaratır.
        if let Some(sky_t) = transforms.get_mut(state.skybox_id) {
            sky_t.position = cam_pos;
        }
    }

    let view_proj = proj * view_mat;

    // Event: Spawning moved to spawner_update_system.
    // Event: Texture Loading moved to main render loop pass before execute_render_pipeline.

    // --- SKELETAL ANIMATION UPDATE ---
    let delta_time = 1.0 / (state.current_fps.max(1.0));

    if let Some(mut q) = world.query_mut_mut::<gizmo::renderer::components::AnimationPlayer, gizmo::renderer::components::Skeleton>() {
            for (_e, anim_player, skeleton) in q.iter_mut() {
                if anim_player.animations.is_empty() { continue; }
                
                let active_idx = anim_player.active_animation.min(anim_player.animations.len() - 1);
                let anim = &anim_player.animations[active_idx];
                
                // Zamanı ilerlet
                anim_player.current_time += delta_time;
                if anim_player.current_time > anim.duration {
                    if anim_player.loop_anim {
                        anim_player.current_time %= anim.duration.max(0.001); // 0 div fix
                    } else {
                        anim_player.current_time = anim.duration;
                    }
                }
                
                let time = anim_player.current_time;
                
                // 1) Local Poses hesapla (Sadece animasyondan gelenleri ez, geri kalanı orijinal local_bind kalsın)
                let hierarchy = &skeleton.hierarchy;
                let mut local_poses = vec![Mat4::IDENTITY; hierarchy.joints.len()];
                
                for (b_idx, joint) in hierarchy.joints.iter().enumerate() {
                    let (mut s, mut r, mut t) = joint.local_bind_transform.to_scale_rotation_translation();
                    
                    if let Some(track) = anim.translations.iter().find(|tr| tr.target_node == joint.node_index) {
                        if let Some(val) = track.get_interpolated(time, |a, b, lerp_t| a.lerp(b, lerp_t)) {
                            t = val;
                        }
                    }
                    
                    if let Some(track) = anim.rotations.iter().find(|tr| tr.target_node == joint.node_index) {
                        if let Some(val) = track.get_interpolated(time, |a, b, lerp_t| a.slerp(b, lerp_t)) {
                            r = Quat::from_xyzw(val.x, val.y, val.z, val.w);
                        }
                    }
                    
                    if let Some(track) = anim.scales.iter().find(|tr| tr.target_node == joint.node_index) {
                        if let Some(val) = track.get_interpolated(time, |a, b, lerp_t| a.lerp(b, lerp_t)) {
                            s = val;
                        }
                    }
                    
                    local_poses[b_idx] = Mat4::from_scale_rotation_translation(s, r, t);
                }

                // 2) Global matrisleri hesapla (Hierarchy)
                let globals = hierarchy.calculate_global_matrices(&local_poses);
                
                // 3) Inverse Bind Matrices ile çarpıp Skeleton'un local_poses alanına yaz (ki shader bilsin)
                skeleton.local_poses.clear();
                for (i, global_mat) in globals.iter().enumerate() {
                    let final_mat = *global_mat * hierarchy.joints[i].inverse_bind_matrix;
                    skeleton.local_poses.push(final_mat);
                }
                
                // 4) GPU'ya gönder! (En faza 64 kemik)
                let mut gpu_data = [[[0.0f32; 4]; 4]; 64];
                for i in 0..skeleton.local_poses.len().min(64) {
                    gpu_data[i] = skeleton.local_poses[i].to_cols_array_2d();
                }
                renderer.queue.write_buffer(&skeleton.buffer, 0, bytemuck::cast_slice(&gpu_data));
            }
        }

    // Işık kaynaklarını topla (Maksimum 10)
    let mut lights_data = [gizmo::renderer::renderer::LightData {
        position: [0.0; 4],
        color: [0.0; 4],
    }; 10];
    let mut num_lights = 0;

    if let Some(q) = world.query_ref_ref::<PointLight, Transform>() {
        for (_e, l, t) in q.iter() {
            if num_lights >= 10 {
                break;
            }
            lights_data[num_lights as usize] = gizmo::renderer::renderer::LightData {
                position: [t.position.x, t.position.y, t.position.z, l.intensity],
                color: [l.color.x, l.color.y, l.color.z, 0.0],
            };
            num_lights += 1;
        }
    }

    // --- Directional Light (Güneş) Taraması ---
    let mut sun_dir = [0.0, -1.0, 0.0, 0.0];
    let mut sun_col = [0.0, 0.0, 0.0, 0.0];

    if let Some(q) =
        world.query_ref_ref::<gizmo::renderer::components::DirectionalLight, Transform>()
    {
        for (_e, dl, t) in q.iter() {
            if dl.is_sun {
                // Transform'un rotasyonundan ileri vektörü hesapla (Güneşin baktığı yön)
                // Standartlara göre ışık '-Z' ye bakar
                let forward = t.rotation.mul_vec3(Vec3::new(0.0, 0.0, -1.0)).normalize();
                sun_dir = [forward.x, forward.y, forward.z, 1.0]; // w=1.0: güneş tanımlı
                sun_col = [dl.color.x, dl.color.y, dl.color.z, dl.intensity];
                break; // Sadece ilk ana güneşi al
            }
        }
    }

    // Shadow Mapping İçin Dinamik Ana Işık Kamerası Hazırla
    let mut light_view_proj = Mat4::IDENTITY;
    if sun_dir[3] > 0.5 {
        // Dinamik Frustum: Gölge kamerasını oyuncunun (cam_pos) tam üstüne/arkasına kilitleriz.
        let light_direction = Vec3::new(sun_dir[0], sun_dir[1], sun_dir[2]).normalize();
        // Güneşi kameranın çok uzağına yerleştirip, devasa şehri tamamen kapsamasını sağla
        let light_pos = cam_pos - light_direction * 150.0;

        let light_view = Mat4::look_at_rh(light_pos, cam_pos, Vec3::new(0.0, 1.0, 0.0));
        // Devasa şehir haritaları için gölge projeksiyon kutusunu 100 metre yarıçapına çıkarıyoruz
        let light_proj = Mat4::orthographic_rh(-100.0, 100.0, -100.0, 100.0, 0.1, 300.0);
        light_view_proj = light_proj * light_view;
    } else if num_lights > 0 {
        // Fallback: PointLight taklidi
        let l_pos = Vec3::new(
            lights_data[0].position[0],
            lights_data[0].position[1],
            lights_data[0].position[2],
        );
        let light_view = Mat4::look_at_rh(l_pos, Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0));
        let light_proj = Mat4::orthographic_rh(-10.0, 10.0, -10.0, 10.0, 0.1, 100.0);
        light_view_proj = light_proj * light_view;
    }

    // Global Uniforms (Her frame sadece 1 kere gönderilir)
    let scene_uniform_data = gizmo::renderer::renderer::SceneUniforms {
        view_proj: view_proj.to_cols_array_2d(),
        camera_pos: [cam_pos.x, cam_pos.y, cam_pos.z, 1.0],
        sun_direction: sun_dir,
        sun_color: sun_col,
        lights: lights_data,
        light_view_proj: light_view_proj.to_cols_array_2d(),
        num_lights,
        _padding: [0; 3],
    };
    renderer.queue.write_buffer(
        &renderer.scene.global_uniform_buffer,
        0,
        bytemuck::cast_slice(&[scene_uniform_data]),
    );

    // --- BATCHING (INSTANCING) HAZIRLIĞI VE FRUSTUM CULLING ---
    use gizmo::renderer::renderer::InstanceRaw;

    let frustum = gizmo::math::frustum::Frustum::from_matrix(&view_proj);

    struct BatchData {
        vbuf: std::sync::Arc<wgpu::Buffer>,
        vertex_count: u32,
        bind_group: std::sync::Arc<wgpu::BindGroup>,
        skeleton_bg: std::sync::Arc<wgpu::BindGroup>,
        instances: Vec<InstanceRaw>,
        is_skybox: bool,
    }

    // Anahtarlar aynı, fakat şeffaflığa ve çift taraflılığa göre ayrı tablolar tutuyoruz
    let mut opaque_batches: std::collections::HashMap<
        (
            *const wgpu::Buffer,
            *const wgpu::BindGroup,
            *const wgpu::BindGroup,
        ),
        BatchData,
    > = std::collections::HashMap::new();
    let mut opaque_double_sided_batches: std::collections::HashMap<
        (
            *const wgpu::Buffer,
            *const wgpu::BindGroup,
            *const wgpu::BindGroup,
        ),
        BatchData,
    > = std::collections::HashMap::new();
    let mut transparent_batches: std::collections::HashMap<
        (
            *const wgpu::Buffer,
            *const wgpu::BindGroup,
            *const wgpu::BindGroup,
        ),
        BatchData,
    > = std::collections::HashMap::new();

    let renderers = world.borrow::<gizmo::renderer::components::MeshRenderer>();
    let skeletons = world.borrow::<gizmo::renderer::components::Skeleton>();
    let lod_groups = world.borrow::<gizmo::renderer::components::LodGroup>();

    if let Some(q) = world.query_ref_ref_ref::<Mesh, Transform, Material>() {
        for (e, mesh, trans, mat) in q.iter() {
            // Sadece MeshRenderer tagli olanları çiz:
            if let Some(r) = &renderers {
                if r.get(e).is_none() {
                    continue;
                }
            } else {
                continue;
            }

            // Gizli olarak işaretlenmiş objeleri atla!
            if let Some(hidden) = world.borrow::<gizmo::core::component::IsHidden>() {
                if hidden.contains(e) {
                    continue;
                }
            }

            // --- GLOBAL TRANSFORM HESAPLAMA ---
            // transform_hierarchy_system() daha önce tüm hiyerarşiyi t.global_matrix'te çözdüğü için
            // doğrudan global_matrix'i kullanmamız yeterlidir. Çift çarpım yapmıyoruz!
            let global_model = trans.global_matrix;

            let center_mat = Mat4::from_translation(mesh.center_offset);
            let model = global_model * center_mat;

            // Frustum Culling (Görüş açısı dışındakileri atla)
            if e != state.skybox_id
                && e != state.gizmo_x
                && e != state.gizmo_y
                && e != state.gizmo_z
            {
                let world_aabb = mesh.bounds.transform(&model);
                if !frustum.contains_aabb(&world_aabb) {
                    continue;
                }
            }

            // --- LOD (Level of Detail) SEÇİMİ ---
            // Eğer entity'de LodGroup varsa, kameraya mesafeye göre düşük/yüksek detay mesh seç
            let active_mesh = if let Some(lods) = &lod_groups {
                if let Some(lod) = lods.get(e) {
                    let world_pos = Vec3::new(model.w_axis.x, model.w_axis.y, model.w_axis.z);
                    let dist = cam_pos.distance(world_pos);
                    lod.select_mesh(dist).unwrap_or(mesh)
                } else {
                    mesh
                }
            } else {
                mesh
            };

            let instance_data = InstanceRaw {
                model: model.to_cols_array_2d(),
                albedo_color: [mat.albedo.x, mat.albedo.y, mat.albedo.z, mat.albedo.w],
                roughness: mat.roughness,
                metallic: mat.metallic,
                unlit: mat.unlit,
                _padding: 0.0,
            };

            // --- SKELETON (KEMİK) ARAMASI ---
            // Yalnızca child meshin değil, atalarından (Root) herhangi birisinde Skeleton var mı diye tırman:
            let mut skel_bg = renderer.scene.dummy_skeleton_bind_group.clone();
            if let Some(skels) = &skeletons {
                if let Some(s) = skels.get(e) {
                    skel_bg = s.bind_group.clone();
                } else if let Some(parents) = world.borrow::<Parent>() {
                    let mut curr = e;
                    while let Some(p) = parents.get(curr) {
                        if let Some(s) = skels.get(p.0) {
                            skel_bg = s.bind_group.clone();
                            break;
                        }
                        curr = p.0;
                    }
                }
            }

            let vbuf_ptr = std::sync::Arc::as_ptr(&active_mesh.vbuf);
            let bg_ptr = std::sync::Arc::as_ptr(&mat.bind_group);
            let skel_ptr = std::sync::Arc::as_ptr(&skel_bg);

            let batches = if mat.is_transparent {
                &mut transparent_batches
            } else if mat.is_double_sided {
                &mut opaque_double_sided_batches
            } else {
                &mut opaque_batches
            };

            let batch = batches
                .entry((vbuf_ptr, bg_ptr, skel_ptr))
                .or_insert_with(|| BatchData {
                    vbuf: active_mesh.vbuf.clone(),
                    vertex_count: active_mesh.vertex_count,
                    bind_group: mat.bind_group.clone(),
                    skeleton_bg: skel_bg,
                    instances: Vec::new(),
                    is_skybox: mat.unlit == 2.0,
                });

            batch.instances.push(instance_data);
        }
    }

    struct FlatBatchData {
        vbuf: std::sync::Arc<wgpu::Buffer>,
        vertex_count: u32,
        bind_group: std::sync::Arc<wgpu::BindGroup>,
        skeleton_bg: std::sync::Arc<wgpu::BindGroup>,
        start_instance: u32,
        end_instance: u32,
        is_transparent: bool,
        is_double_sided: bool,
        is_skybox: bool,
    }

    let mut all_instances = Vec::new();
    let mut flat_batches = Vec::new();

    let mut process_batches = |batches: std::collections::HashMap<_, BatchData>,
                               is_transparent: bool,
                               is_double_sided: bool| {
        for (_, mut batch) in batches {
            // Şeffaf objelerin arka plandan öne doğru sıralanması (Z-Sorting)
            // Instance'ın model matrisinden world pozisyonunu çekip kameraya uzaklığına göre sıralıyoruz
            if is_transparent {
                batch.instances.sort_by(|a, b| {
                    let pos_a = Vec3::new(a.model[3][0], a.model[3][1], a.model[3][2]);
                    let pos_b = Vec3::new(b.model[3][0], b.model[3][1], b.model[3][2]);
                    let dist_a = cam_pos.distance_squared(pos_a);
                    let dist_b = cam_pos.distance_squared(pos_b);
                    // Uzak olanlar ÖNCE çizilmeli (Azalan sıralama)
                    dist_b
                        .partial_cmp(&dist_a)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }

            let start = all_instances.len() as u32;
            let count = batch.instances.len() as u32;
            all_instances.append(&mut batch.instances);

            flat_batches.push(FlatBatchData {
                vbuf: batch.vbuf,
                vertex_count: batch.vertex_count,
                bind_group: batch.bind_group,
                skeleton_bg: batch.skeleton_bg,
                start_instance: start,
                end_instance: start + count,
                is_transparent,
                is_double_sided,
                is_skybox: batch.is_skybox,
            });
        }
    };

    process_batches(opaque_batches, false, false);
    process_batches(opaque_double_sided_batches, false, true);
    process_batches(transparent_batches, true, false);

    if !all_instances.is_empty() {
        let limit = 100_000;
        let safe_len = std::cmp::min(all_instances.len(), limit);
        renderer.queue.write_buffer(
            &renderer.scene.instance_buffer,
            0,
            bytemuck::cast_slice(&all_instances[0..safe_len]),
        );
    }

    // --- 0. COMPUTE PASSES ---
    if let Some(gpu_particles) = &renderer.gpu_particles {
        gpu_particles.update_params(&renderer.queue, delta_time);

        // --- YENİ PARTİCÜL SPAWNLAMA (CPU -> GPU) ---
        if let Some(mut emitters) =
            world.borrow_mut::<gizmo::renderer::components::ParticleEmitter>()
        {
            if let Some(transforms) = world.borrow::<Transform>() {
                use rand::Rng;
                let mut rng = rand::rng();
                let mut all_new_particles = Vec::new();

                let emitter_entities = emitters.dense.iter().map(|e| e.entity).collect::<Vec<_>>();
                for e_id in emitter_entities {
                    if let Some(emitter) = emitters.get_mut(e_id) {
                        if !emitter.is_active || emitter.spawn_rate <= 0.0 {
                            continue;
                        }

                        let base_pos = if let Some(t) = transforms.get(e_id) {
                            t.position + t.rotation.mul_vec3(emitter.local_offset)
                        } else {
                            emitter.local_offset
                        };

                        emitter.accumulator += delta_time;
                        // Güvenlik limiti: Frame drop olursa bir frame'de 100'den fazla spawnlamasın
                        // Aksi takdirde 1 saniye donup binlerce üreterek FPS'i çökertir
                        let spawn_interval = 1.0 / emitter.spawn_rate;
                        let mut spawned_this_frame = 0;

                        while emitter.accumulator >= spawn_interval && spawned_this_frame < 100 {
                            emitter.accumulator -= spawn_interval;
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
                                gizmo::renderer::particle_renderer::GpuParticle {
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

        gpu_particles.compute_pass(encoder);
    }

    if let Some(physics) = &renderer.gpu_physics {
        physics.compute_pass(encoder);
    }

    // --- 1. GÖLGE PASS (Shadow Pass) ---
    {
        let mut shadow_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Shadow Pass"),
            color_attachments: &[], // Shadow pass sadece Depth'e çizer
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &renderer.scene.shadow_texture_view,
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

        // Tıpkı main render gibi gruplanmış nesneleri tek draw çağrısıyla bas
        for batch in &flat_batches {
            if batch.start_instance >= 100_000 {
                continue;
            }
            let safe_end = std::cmp::min(batch.end_instance, 100_000);

            shadow_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
            shadow_pass.set_bind_group(1, &batch.skeleton_bg, &[]);
            shadow_pass.set_bind_group(2, &renderer.scene.instance_bind_group, &[]);
            shadow_pass.set_vertex_buffer(0, batch.vbuf.slice(..));
            shadow_pass.draw(0..batch.vertex_count, batch.start_instance..safe_end);
        }
    }

    // --- 2. ANA RENDER PASS (HDR Offscreen Texture'a çiz) ---
    {
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Main Render Pass (HDR)"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &renderer.post.hdr_texture_view, // Artık ekran yerine HDR texture'a çiziyoruz!
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.1,
                        g: 0.15,
                        b: 0.20,
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

        // 1. OPAQUE OBJELERİ ÇİZ (Sırtı Cull edilen normal objeler)
        render_pass.set_pipeline(&renderer.scene.render_pipeline);
        for batch in &flat_batches {
            if batch.is_transparent || batch.is_double_sided || batch.is_skybox {
                continue;
            } // Şeffafları, Skybox'ı ve çift yönlüleri atla
            if batch.start_instance >= 100_000 {
                continue;
            }
            let safe_end = std::cmp::min(batch.end_instance, 100_000);

            render_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
            render_pass.set_bind_group(1, &batch.bind_group, &[]);
            render_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
            render_pass.set_bind_group(3, &batch.skeleton_bg, &[]);
            render_pass.set_bind_group(4, &renderer.scene.instance_bind_group, &[]);
            render_pass.set_vertex_buffer(0, batch.vbuf.slice(..));
            render_pass.draw(0..batch.vertex_count, batch.start_instance..safe_end);
        }

        // 2. ÇİFT YÖNLÜ OPAQUE OBJELER (Kumaşlar, cull_mode = None)
        render_pass.set_pipeline(&renderer.scene.render_double_sided_pipeline);
        for batch in &flat_batches {
            if batch.is_transparent || !batch.is_double_sided || batch.is_skybox {
                continue;
            }
            if batch.start_instance >= 100_000 {
                continue;
            }
            let safe_end = std::cmp::min(batch.end_instance, 100_000);

            render_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
            render_pass.set_bind_group(1, &batch.bind_group, &[]);
            render_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
            render_pass.set_bind_group(3, &batch.skeleton_bg, &[]);
            render_pass.set_bind_group(4, &renderer.scene.instance_bind_group, &[]);
            render_pass.set_vertex_buffer(0, batch.vbuf.slice(..));
            render_pass.draw(0..batch.vertex_count, batch.start_instance..safe_end);
        }

        // --- DRAW GPU PHYSICS SPHERES (Katı Obje olarak farz ediliyor) ---
        if let Some(physics) = &renderer.gpu_physics {
            physics.render_pass(&mut render_pass, &renderer.scene.global_bind_group);
        }

        // 3. SKYBOX YAKALAMA VE ÖZEL PIPELINE İLE ÇİZİM
        render_pass.set_pipeline(&renderer.scene.sky_pipeline);
        for batch in &flat_batches {
            if !batch.is_skybox {
                continue;
            } // Sadece Skybox'u çiz
            if batch.start_instance >= 100_000 {
                continue;
            }
            let safe_end = std::cmp::min(batch.end_instance, 100_000);

            render_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
            render_pass.set_bind_group(1, &batch.bind_group, &[]);
            render_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]); // sky.wgsl içinde boş da olsa bağlı kalması gerek
            render_pass.set_bind_group(3, &batch.skeleton_bg, &[]);
            render_pass.set_bind_group(4, &renderer.scene.instance_bind_group, &[]);
            render_pass.set_vertex_buffer(0, batch.vbuf.slice(..));
            render_pass.draw(0..batch.vertex_count, batch.start_instance..safe_end);
        }

        // 4. TRANSPARENT OBJELERİ ÇİZ (Depth yazması kapalı, Opaque'nin üstüne blend olur)
        render_pass.set_pipeline(&renderer.scene.transparent_pipeline);
        for batch in &flat_batches {
            if !batch.is_transparent {
                continue;
            } // Sadece şeffafları çiz
            if batch.start_instance >= 100_000 {
                continue;
            }
            let safe_end = std::cmp::min(batch.end_instance, 100_000);

            render_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
            render_pass.set_bind_group(1, &batch.bind_group, &[]);
            render_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
            render_pass.set_bind_group(3, &batch.skeleton_bg, &[]);
            render_pass.set_bind_group(4, &renderer.scene.instance_bind_group, &[]);
            render_pass.set_vertex_buffer(0, batch.vbuf.slice(..));
            render_pass.draw(0..batch.vertex_count, batch.start_instance..safe_end);
        }

        // --- 4. DRAW GPU PARTICLES (Billboard & Şeffaf) ---
        if let Some(gpu_particles) = &renderer.gpu_particles {
            render_pass.set_pipeline(&gpu_particles.render_pipeline);
            render_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
            render_pass.set_vertex_buffer(0, gpu_particles.quad_vertex_buffer.slice(..));
            render_pass.set_vertex_buffer(1, gpu_particles.particles_buffer.slice(..));
            render_pass.draw(0..4, 0..gpu_particles.active_particles);
        }
    }

    // --- 3. POST-PROCESSING (Bloom + Tone Mapping → Ekrana Yaz) ---
    renderer.run_post_processing(encoder, view);
}
