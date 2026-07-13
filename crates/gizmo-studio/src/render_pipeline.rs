use crate::StudioState;
use gizmo::prelude::*;
use std::cell::RefCell;

// The pointer triple identifies mesh + texture bind group + skeleton, but the
// texture bind group is cached and SHARED across distinct materials (default
// white texture, same file), so two materials differing only in material type
// would collide into one batch and inherit the first-iterated material's
// `is_skybox`/`is_grid`/`is_unlit`. Those flags gate shadow casting and pass
// routing (e.g. line ~759 skips unlit/skybox/grid in the shadow pass), so a real
// PBR object batched under an unlit-first batch would silently stop casting
// shadows. Keying on the routing flags too keeps them apart.
type BatchKey = (
    *const wgpu::Buffer,
    *const wgpu::BindGroup,
    *const wgpu::BindGroup,
    bool, // is_skybox
    bool, // is_grid
    bool, // is_unlit
);

struct BatchData {
    vbuf: std::sync::Arc<wgpu::Buffer>,
    vertex_count: u32,
    bind_group: std::sync::Arc<wgpu::BindGroup>,
    skeleton_bg: std::sync::Arc<wgpu::BindGroup>,
    instances: Vec<gizmo::renderer::InstanceRaw>,
    /// Casters outside the camera frustum but inside a shadow cascade's light frustum —
    /// drawn into the shadow maps only so off-screen objects still cast visible shadows.
    shadow_instances: Vec<gizmo::renderer::InstanceRaw>,
    is_skybox: bool,
    is_grid: bool,
    is_unlit: bool,
}

struct FlatBatchData {
    vbuf: std::sync::Arc<wgpu::Buffer>,
    vertex_count: u32,
    bind_group: std::sync::Arc<wgpu::BindGroup>,
    skeleton_bg: std::sync::Arc<wgpu::BindGroup>,
    start_instance: u32,
    /// End of the CAMERA-visible range (main passes draw `start..end_instance`).
    end_instance: u32,
    /// End of the full range incl. off-screen shadow casters (shadow pass draws
    /// `start..shadow_end_instance`). Equals `end_instance` when there are none.
    shadow_end_instance: u32,
    is_transparent: bool,
    is_double_sided: bool,
    is_skybox: bool,
    is_grid: bool,
    is_unlit: bool,
}

struct PipelineCache {
    opaque_batches: std::collections::HashMap<BatchKey, BatchData>,
    opaque_double_sided_batches: std::collections::HashMap<BatchKey, BatchData>,
    transparent_batches: std::collections::HashMap<BatchKey, BatchData>,
    all_instances: Vec<gizmo::renderer::InstanceRaw>,
    flat_batches: Vec<FlatBatchData>,
    vec_pool: Vec<Vec<gizmo::renderer::InstanceRaw>>,
}

impl Default for PipelineCache {
    fn default() -> Self {
        Self {
            opaque_batches: std::collections::HashMap::with_capacity(256),
            opaque_double_sided_batches: std::collections::HashMap::with_capacity(256),
            transparent_batches: std::collections::HashMap::with_capacity(256),
            all_instances: Vec::with_capacity(10000),
            flat_batches: Vec::with_capacity(256),
            vec_pool: Vec::with_capacity(256),
        }
    }
}

thread_local! {
    static CACHE: RefCell<PipelineCache> = RefCell::new(PipelineCache::default());
}

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

    let (aspect, ed_shading_mode, show_colliders) = sync_editor_settings(world, renderer);

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

    // Işık kaynakları (point + spot + sun) — game renderer ile ORTAK setup
    // helper'ından. Eskiden burada elle-yazılmış üç ışık döngüsü vardı (ham
    // Transform okuyordu → parented ışıklar yanlış yerleşiyordu, mesh'ler ise
    // GlobalTransform kullanıyordu = tutarsız). Artık iki renderer tek koddan
    // besleniyor; ışık mantığı bir daha ayrışamaz. sun'ı studio'nun `[f32;4]`
    // (w = güneş-var-flag) temsiline çeviriyoruz.
    let scene_lights = gizmo::systems::render::collect_scene_lights(world);
    let lights_data = scene_lights.lights;
    let num_lights = scene_lights.num_lights;
    let sun_dir = [
        scene_lights.sun_dir.x,
        scene_lights.sun_dir.y,
        scene_lights.sun_dir.z,
        if scene_lights.has_sun { 1.0 } else { 0.0 }, // w=1.0: güneş tanımlı
    ];
    let sun_col = [
        scene_lights.sun_col.x,
        scene_lights.sun_col.y,
        scene_lights.sun_col.z,
        scene_lights.sun_col.w,
    ];

    // Pick the shadow-casting direction: the sun if the scene has one, else fall
    // back to the first point light aimed at the origin (studio-only fallback — the
    // game always casts from the sun). Cascade orchestration itself is the shared
    // helper, so game and studio can't drift on the SHADOW_DISTANCE cap / lambda.
    let shadow_dir = if sun_dir[3] > 0.5 {
        Some(Vec3::new(sun_dir[0], sun_dir[1], sun_dir[2]).normalize())
    } else if num_lights > 0 {
        let l_pos = Vec3::new(
            lights_data[0].position[0],
            lights_data[0].position[1],
            lights_data[0].position[2],
        );
        Some((Vec3::ZERO - l_pos).normalize())
    } else {
        None
    };

    let cascades = gizmo::renderer::compute_directional_cascades(
        cam_pos,
        cam_forward,
        aspect,
        cam_fov,
        cam_near,
        cam_far,
        shadow_dir.unwrap_or(Vec3::new(0.0, -1.0, 0.0)),
    );
    let cascade_splits = cascades.splits;
    let identity_m = Mat4::IDENTITY.to_cols_array_2d();
    let mut light_view_proj_cascades = [identity_m; 4];
    // No light → leave cascades at identity (the shared helper still gives correct
    // splits for the SceneUniforms, but the matrices are unused this frame).
    if shadow_dir.is_some() {
        for (dst, src) in light_view_proj_cascades
            .iter_mut()
            .zip(cascades.view_projs.iter())
        {
            *dst = src.to_cols_array_2d();
        }
    }

    // z = elapsed time for fluid caustics/wave animation (fluid_composite.wgsl reads it);
    // was hardcoded 0.0 → frozen water (same bug as the gizmo runtime path).
    let elapsed_time = world
        .get_resource::<gizmo::core::time::Time>()
        .map(|t| t.elapsed() as f32)
        .unwrap_or(0.0);
    let cascade_params = [
        cam_near,
        1.0 / gizmo::renderer::SHADOW_MAP_RES as f32,
        elapsed_time,
        0.0,
    ];
    let camera_forward_u = [cam_forward.x, cam_forward.y, cam_forward.z, 0.0];

    // Global Uniforms (Her frame sadece 1 kere gönderilir)
    let scene_uniform_data = gizmo::renderer::renderer::SceneUniforms {
        view_proj: view_proj.to_cols_array_2d(),
        camera_pos: [cam_pos.x, cam_pos.y, cam_pos.z, 1.0],
        sun_direction: sun_dir,
        sun_color: sun_col,
        lights: lights_data,
        light_view_proj: light_view_proj_cascades,
        cascade_splits,
        camera_forward: camera_forward_u,
        cascade_params,
        num_lights,
        exposure: 1.0,
        _pre_align_pad: [0; 2],
        _align_pad: [0; 3],
        environment_blend_t: 0.0,
        environment_preset: 0,
        point_shadows_enabled: 0,
        environment_preset_2: 0,
        shading_mode: ed_shading_mode,
    };
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
                let camera_visible = match gizmo::renderer::classify_visibility(
                    &culling_frustum,
                    &cascade_frusta,
                    &model,
                    mesh.bounds,
                    mat.material_type,
                    mat.is_transparent,
                    mat.albedo.w,
                ) {
                    gizmo::renderer::Visibility::Culled => continue,
                    gizmo::renderer::Visibility::Camera => true,
                    gizmo::renderer::Visibility::ShadowOnly => false,
                };

                // Culling'i geçen objelerin Bounding Box'larını debug çizimi için kaydet.
                // Skybox ve Grid'i hariç tut: bounds'ları tüm sahneyi sardığı için
                // kırmızı AABB'leri ekranı baştan başa kesen dev çizgiler olarak görünüyordu.
                if !is_playing_mode
                    && !matches!(
                        mat.material_type,
                        gizmo::renderer::components::MaterialType::Skybox
                            | gizmo::renderer::components::MaterialType::Grid
                    )
                {
                    debug_aabbs.push(mesh.bounds.transform(&model));
                }

                // --- LOD (Level of Detail) SEÇİMİ ---
                // Eğer entity'de LodGroup varsa, kameraya mesafeye göre düşük/yüksek detay mesh seç
                let lods = &lod_groups;
                let active_mesh_opt = if let Some(lod) = lods.get(e) {
                    let world_pos = Vec3::new(model.w_axis.x, model.w_axis.y, model.w_axis.z);
                    let dist = cam_pos.distance(world_pos);
                    lod.select_mesh(dist)
                } else {
                    Some(mesh)
                };

                let active_mesh = match active_mesh_opt {
                    Some(m) => m,
                    None => continue, // CULL edildi!
                };

                let packed_params = (mat.anisotropy * 1000.0).floor() + 1000.0 * (mat.clear_coat * 1000.0).floor() + 1000000.0 * (mat.subsurface * 100.0).floor() ;

                let instance_data = InstanceRaw {
                    model: model.to_cols_array_2d(),
                    albedo_color: [mat.albedo.x, mat.albedo.y, mat.albedo.z, mat.albedo.w],
                    roughness: mat.roughness,
                    metallic: mat.metallic,
                    unlit: match mat.material_type {
                        gizmo::renderer::components::MaterialType::Skybox => 2.0,
                        gizmo::renderer::components::MaterialType::Unlit => 1.0,
                        _ => 0.0,
                    },
                    _padding: packed_params,
                };

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
                let is_skybox =
                    mat.material_type == gizmo::renderer::components::MaterialType::Skybox;
                let is_grid = mat.material_type == gizmo::renderer::components::MaterialType::Grid;
                let is_unlit =
                    mat.material_type == gizmo::renderer::components::MaterialType::Unlit;

                let batches = if mat.is_transparent {
                    &mut *transparent_batches
                } else if mat.is_double_sided {
                    &mut *opaque_double_sided_batches
                } else {
                    &mut *opaque_batches
                };

                let batch = batches
                    .entry((vbuf_ptr, bg_ptr, skel_ptr, is_skybox, is_grid, is_unlit))
                    .or_insert_with(|| BatchData {
                        vbuf: active_mesh.vbuf.clone(),
                        vertex_count: active_mesh.vertex_count,
                        bind_group: mat.bind_group.clone(),
                        skeleton_bg: skel_bg,
                        instances: vec_pool.pop().unwrap_or_else(|| Vec::with_capacity(32)),
                        shadow_instances: Vec::new(),
                        is_skybox,
                        is_grid,
                        is_unlit,
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
                for (_, mut batch) in batches.drain() {
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



fn sync_editor_settings(world: &gizmo::core::World, renderer: &mut gizmo::renderer::Renderer) -> (f32, u32, bool) {
    let mut aspect = if renderer.size.height > 0 {
        renderer.size.width as f32 / renderer.size.height as f32
    } else {
        1.0
    };

    let mut ed_shading_mode = 0;
    let mut ed_fxaa_enabled = true;
    let mut ed_ssao_enabled = true;
    let mut ed_ssao_strength = 0.8;
    let mut show_colliders = false;
    
    let mut post_params = gizmo::renderer::renderer::PostProcessUniforms {
        bloom_intensity: 0.8,
        bloom_threshold: 0.85,
        exposure: 1.0,
        vignette_intensity: 0.2,
        chromatic_aberration: 0.005,
        film_grain_intensity: 0.0,
        dof_focus_dist: 10.0,
        dof_focus_range: 20.0,
        dof_blur_size: 2.0,
        cam_near: 0.1,
        cam_far: 2000.0,
        underwater: 0.0,
        fog_r: 0.0,
        fog_g: 0.0,
        fog_b: 0.0,
        fog_density: 0.0,
    };

    if let Some(ed_state) = world.get_resource::<gizmo::editor::EditorState>() {
        ed_shading_mode = ed_state.shading_mode;
        ed_fxaa_enabled = ed_state.post_process.fxaa_enabled;
        ed_ssao_enabled = ed_state.post_process.ssao_enabled;
        ed_ssao_strength = ed_state.post_process.ssao_strength;
        
        show_colliders = ed_state.show_colliders;
        post_params.bloom_intensity = ed_state.post_process.bloom_intensity;
        post_params.bloom_threshold = ed_state.post_process.bloom_threshold;
        post_params.exposure = ed_state.post_process.exposure;
        post_params.vignette_intensity = ed_state.post_process.vignette;
        post_params.chromatic_aberration = ed_state.post_process.chromatic_aberration;
        post_params.dof_focus_dist = ed_state.post_process.dof_focus_dist;
        post_params.dof_focus_range = ed_state.post_process.dof_focus_range;
        post_params.dof_blur_size = ed_state.post_process.dof_blur_size;
        post_params.film_grain_intensity = ed_state.post_process.film_grain;

        if let Some(rect) = ed_state.scene_view_rect {
            if rect.height() > 0.0 {
                aspect = rect.width() / rect.height();
            }
        }
    }

    renderer.update_post_process(&renderer.queue, post_params);

    if let Some(ref mut fxaa) = renderer.fxaa {
        if fxaa.enabled != ed_fxaa_enabled {
            fxaa.enabled = ed_fxaa_enabled;
            fxaa.set_enabled(&renderer.queue, ed_fxaa_enabled);
        }
    }

    if let Some(ref mut ssao) = renderer.ssao {
        let actual_strength = if ed_ssao_enabled { ed_ssao_strength } else { 0.0 };
        ssao.set_strength(&renderer.queue, actual_strength);
    }

    (aspect, ed_shading_mode, show_colliders)
}

// execute_render_pipeline'ten çıkarılan render geçişleri (Tier 3: mega-fn bölmesi).
// Yan-etki-only: encoder'a komut kaydeder, çıktı yok.
fn record_studio_shadow_passes(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &gizmo::renderer::Renderer,
    flat_batches: &[FlatBatchData],
    light_view_proj_cascades: &[[[f32; 4]; 4]; 4],
) {
        for (cascade_i, &cascade_view_proj) in light_view_proj_cascades.iter().enumerate() {
            renderer.queue.write_buffer(
                &renderer.scene.shadow_cascade_uniform_buffers[cascade_i],
                0,
                gizmo::bytemuck::bytes_of(&gizmo::renderer::ShadowVsUniform {
                    light_view_proj: cascade_view_proj,
                }),
            );

            let mut shadow_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(&format!("Shadow Pass cascade {cascade_i}")),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &renderer.scene.shadow_cascade_layer_views[cascade_i],
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            multiview_mask: None,
            });

            shadow_pass.set_pipeline(&renderer.scene.shadow_pipeline);

            for batch in flat_batches {
                // Non-casters must not write the shadow maps — matches the game path
                // (`passes.rs`: skip unlit/transparent) and the `classify_visibility`
                // caster predicate (excludes Unlit/Skybox/Grid/transparent). Their
                // CAMERA-VISIBLE instances still live in `[start_instance, end_instance)`
                // here, so without this filter the editor grid / a skybox / transparent
                // objects would cast shadows (grid → ground-coplanar self-shadow acne,
                // skybox → shadows the whole scene).
                if batch.is_transparent || batch.is_skybox || batch.is_grid || batch.is_unlit {
                    continue;
                }
                if batch.start_instance >= renderer.scene.instance_capacity as u32 {
                    continue;
                }
                // Shadow pass draws the FULL range (camera-visible + off-screen casters).
                let safe_end = std::cmp::min(
                    batch.shadow_end_instance,
                    renderer.scene.instance_capacity as u32,
                );

                shadow_pass.set_bind_group(
                    0,
                    &renderer.scene.shadow_pass_bind_groups[cascade_i],
                    &[],
                );
                shadow_pass.set_bind_group(1, &*batch.skeleton_bg, &[]);
                shadow_pass.set_bind_group(2, &renderer.scene.instance_bind_group, &[]);
                shadow_pass.set_vertex_buffer(0, batch.vbuf.slice(..));
                shadow_pass.draw(0..batch.vertex_count, batch.start_instance..safe_end);
            }
        }
}

fn record_studio_main_pass(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &mut gizmo::renderer::Renderer,
    world: &gizmo::core::World,
    flat_batches: &[FlatBatchData],
    game_view_proj: Option<Mat4>,
    debug_aabbs: &[Aabb],
    show_colliders: bool,
) {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Main Render Pass (HDR)"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &renderer.post.hdr_texture_view, // Artık ekran yerine HDR texture'a çiziyoruz!
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Linear space 0.035 ~= sRGB 0.22 (Blender dark grey) after Gamma Correction / HDR
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.035,
                            g: 0.035,
                            b: 0.035,
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
            multiview_mask: None,
            });

            render_pass.set_pipeline(&renderer.scene.render_pipeline);
            for batch in flat_batches {
                if batch.is_transparent || batch.is_double_sided || batch.is_skybox || batch.is_grid
                {
                    continue;
                } // Şeffafları, Skybox'ı, Çift Yönlüleri ve Grid'i atla
                if batch.start_instance >= renderer.scene.instance_capacity as u32 {
                    continue;
                }
                let safe_end =
                    std::cmp::min(batch.end_instance, renderer.scene.instance_capacity as u32);

                render_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
                render_pass.set_bind_group(1, &*batch.bind_group, &[]);
                render_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
                render_pass.set_bind_group(3, &*batch.skeleton_bg, &[]);
                render_pass.set_bind_group(4, &renderer.scene.instance_bind_group, &[]);
                render_pass.set_vertex_buffer(0, batch.vbuf.slice(..));
                render_pass.draw(0..batch.vertex_count, batch.start_instance..safe_end);
            }

            // 2. ÇİFT YÖNLÜ OPAQUE OBJELER (Kumaşlar, cull_mode = None)
            render_pass.set_pipeline(&renderer.scene.render_double_sided_pipeline);
            for batch in flat_batches {
                if batch.is_transparent
                    || !batch.is_double_sided
                    || batch.is_skybox
                    || batch.is_grid
                {
                    continue;
                }
                if batch.start_instance >= renderer.scene.instance_capacity as u32 {
                    continue;
                }
                let safe_end =
                    std::cmp::min(batch.end_instance, renderer.scene.instance_capacity as u32);

                render_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
                render_pass.set_bind_group(1, &*batch.bind_group, &[]);
                render_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
                render_pass.set_bind_group(3, &*batch.skeleton_bg, &[]);
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
            for batch in flat_batches {
                if !batch.is_skybox {
                    continue;
                } // Sadece Skybox'u çiz
                if batch.start_instance >= renderer.scene.instance_capacity as u32 {
                    continue;
                }
                let safe_end =
                    std::cmp::min(batch.end_instance, renderer.scene.instance_capacity as u32);

                render_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
                render_pass.set_bind_group(1, &*batch.bind_group, &[]);
                render_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]); // sky.wgsl içinde boş da olsa bağlı kalması gerek
                render_pass.set_bind_group(3, &*batch.skeleton_bg, &[]);
                render_pass.set_bind_group(4, &renderer.scene.instance_bind_group, &[]);
                render_pass.set_vertex_buffer(0, batch.vbuf.slice(..));
                render_pass.draw(0..batch.vertex_count, batch.start_instance..safe_end);
            }

            // 4. TRANSPARENT OBJELERİ ÇİZ (Depth yazması kapalı, Opaque'nin üstüne blend olur)
            render_pass.set_pipeline(&renderer.scene.transparent_pipeline);
            for batch in flat_batches {
                if !batch.is_transparent || batch.is_grid {
                    continue;
                } // Sadece saydamları çiz
                if batch.start_instance >= renderer.scene.instance_capacity as u32 {
                    continue;
                }
                let safe_end =
                    std::cmp::min(batch.end_instance, renderer.scene.instance_capacity as u32);

                render_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
                render_pass.set_bind_group(1, &*batch.bind_group, &[]);
                render_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
                render_pass.set_bind_group(3, &*batch.skeleton_bg, &[]);
                render_pass.set_bind_group(4, &renderer.scene.instance_bind_group, &[]);
                render_pass.set_vertex_buffer(0, batch.vbuf.slice(..));
                render_pass.draw(0..batch.vertex_count, batch.start_instance..safe_end);
            }

            // 5. GRID ÇİZİMİ (Play modunda gizle — Game View temiz görünsün)
            let is_playing_mode = world.get_resource::<gizmo::editor::EditorState>()
                .map(|ed| ed.is_playing() || ed.mode == gizmo::editor::EditorMode::Paused)
                .unwrap_or(false);
            if !is_playing_mode {
                render_pass.set_pipeline(&renderer.scene.grid_pipeline);
                for batch in flat_batches {
                    if !batch.is_grid {
                        continue;
                    }
                    if batch.start_instance >= renderer.scene.instance_capacity as u32 {
                        continue;
                    }
                    let safe_end =
                        std::cmp::min(batch.end_instance, renderer.scene.instance_capacity as u32);

                    render_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
                    render_pass.set_bind_group(1, &*batch.bind_group, &[]);
                    render_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
                    render_pass.set_bind_group(3, &*batch.skeleton_bg, &[]);
                    render_pass.set_bind_group(4, &renderer.scene.instance_bind_group, &[]);
                    render_pass.set_vertex_buffer(0, batch.vbuf.slice(..));
                    render_pass.draw(0..batch.vertex_count, batch.start_instance..safe_end);
                }
            }

            // --- 4. DRAW GPU PARTICLES (Billboard & Şeffaf) ---
            if let Some(gpu_particles) = &renderer.gpu_particles {
                render_pass.set_pipeline(&gpu_particles.pipelines.render_pipeline);
                render_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
                render_pass.set_vertex_buffer(0, gpu_particles.quad_vertex_buffer.slice(..));
                render_pass.set_vertex_buffer(1, gpu_particles.particles_buffer.slice(..));
                render_pass.draw(0..4, 0..gpu_particles.active_particles);
            }
            // --- 5. GIZMOS VE DEBUG LINES ÇİZİMİ (Play modunda gizle) ---
            if !is_playing_mode {
                if let Some(mut gizmos) = world.get_resource_mut::<gizmo::renderer::Gizmos>() {
                    // Game Camera Frustum'unu Yeşil çiz
                    if let Some(vp) = game_view_proj {
                        gizmos.draw_frustum(vp, [0.0, 1.0, 0.0, 1.0]); // Yeşil
                    }

                    // Ekranda kalan (Cull edilmeyen) objelerin AABB'lerini Kırmızı çiz
                    for aabb in debug_aabbs {
                        gizmos.draw_aabb(*aabb, [1.0, 0.0, 0.0, 1.0]); // Kırmızı
                    }

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

            if show_colliders {
                if let Some(physics) = &renderer.gpu_physics {
                    physics.debug_render_pass(&mut render_pass, &renderer.scene.global_bind_group);
                }
            }
}
