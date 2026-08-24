use crate::StudioState;
use gizmo::prelude::*;
use std::cell::RefCell;

mod batching;
mod passes;
mod viewpoint;
use batching::*;
use passes::*;

/// How much of the fluid the viewport simulates and draws.
///
/// **Zero unless the scene opted in.** `Renderer::gpu_fluid` is `Some` on every native renderer —
/// it is allocated up front, 100 000 particles' worth — and what decides whether a scene *has*
/// fluid is `Renderer::fluid_enabled`, which is `false` by default and set by the two demos that
/// want it. Reading only `num_particles` here would have run an SPH solve and a screen-space
/// surface pass over 100 000 particles in every editor frame of every scene, which is how a
/// capability port turns into a performance defect.
///
/// Past that gate the viewport runs the **whole** fluid, where the game path scales it by camera
/// distance: an editor shows what is there, the same reasoning the render-parity exceptions
/// already record for mesh-internal LOD.
fn studio_fluid_particles(renderer: &gizmo::renderer::Renderer) -> u32 {
    if !renderer.fluid_enabled {
        return 0;
    }
    renderer.gpu_fluid.as_ref().map(|f| f.num_particles).unwrap_or(0)
}

/// Records the studio's render passes for one viewport — shadows, opaque, transparent, grid and
/// skybox — into the given encoder.
pub fn execute_render_pipeline(
    world: &mut World,
    state: &StudioState,
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    renderer: &mut gizmo::renderer::Renderer,
    _light_time: f32,
) {
    // Meshes with a `Transform` but no `GlobalTransform` get one, and every world matrix is
    // refreshed — the same call the game path makes, for the same reason. Studio's update loop
    // runs the sync and propagate systems, but neither *adds* the component, so an entity spawned
    // as `Transform + Mesh + Material` drew in the game and silently vanished in the editor.
    gizmo::systems::render::ensure_global_transforms(world);

    // `MaterialDesc` → `Material` and back, the same pair the game path runs and for the same
    // reason: a just-loaded scene carries descriptions and no materials, and a scene about to be
    // saved needs its descriptions written back. The editor is where materials are authored, so it
    // is the path that loses most when this is missing.
    gizmo::renderer::material_sync::resolve_material_descriptions(world, renderer);
    gizmo::renderer::material_sync::sync_material_descriptions(world);

    // --- SKELETAL ANIMATION UPDATE (Done before any ECS borrows!) ---
    //
    // BOTH drivers, because there are two: `AnimationPlayer` (a clip and a playhead) and
    // `AnimationStateMachine` (states and transitions). The editor ran only the first, so an
    // entity animated by a state machine played in an exported game and stood perfectly still in
    // the viewport — the editor's oldest failure mode, one draw path knowing a capability the
    // other does not. The parity inventory could not see it either: it scans components *defined*
    // in `gizmo-renderer/src/components`, and the whole skeletal family is re-exported there from
    // `gizmo-animation`. Both halves are fixed together.
    // The delta is the CLOCK's, not this frame's — see `animation_delta`. Passing the raw frame
    // delta let `set_time_scale(0.5)` halve a game's animation while the viewport ran at full
    // speed, and left ⏸ with walking skeletons.
    let animation_dt = crate::systems::simulation::animation_delta(world);
    gizmo::renderer::animation_update_system(world, animation_dt, &renderer.queue);
    gizmo::renderer::animation_state_machine_update_system(world, animation_dt, &renderer.queue);
    let delta_time = state.actual_dt;
    
    let mut bone_att = gizmo::systems::transform::BoneAttachmentSystem;
    gizmo::core::system::System::run(&mut bone_att, world, delta_time);

    // GPU physics, the other way round from the bone attachment above: the game path ran these
    // two and the editor did not, so a scene using GPU rigid bodies simulated in an exported game
    // and sat still in the viewport — while the viewport's own passes were already drawing from
    // `renderer.gpu_physics`. Both are no-ops unless the renderer has a GPU physics world, so a
    // scene that never asked for one pays nothing.
    gizmo::systems::physics::gpu_physics_submit_system(world, renderer);
    gizmo::systems::physics::gpu_physics_readback_system(world, renderer);

    // GPU fluid: stepped here, drawn twice below (particles inside the main pass, surface after
    // it). The editor did not step or draw it at all until 2026-08-19 — a fluid scene flowed in a
    // shipped game and showed nothing in the viewport, while this pipeline was already drawing
    // `renderer.gpu_physics` beside it.
    gizmo::systems::render::step_gpu_fluid(encoder, renderer, studio_fluid_particles(renderer));

    let (aspect, ed_shading_mode, show_colliders, post_params, game_view_visible) =
        sync_editor_settings(world, renderer);

    // Hidden entities, INCLUDING everything under one — `collect_hidden` is the engine's own
    // walk, shared with the game path, because two answers to "is this drawn" is what the
    // 2026-08-19 fix was about. The `IsHidden` borrow is still taken here rather than inside the
    // helper so this loop keeps naming the marker in code, which `render_parity` asserts.
    let _is_hidden_guard = world.borrow::<gizmo::core::component::IsHidden>();
    let hidden = gizmo::systems::render::collect_hidden(
        &_is_hidden_guard,
        &world.borrow::<gizmo::core::component::Children>(),
    );

    // Which camera this frame is drawn from — and, separately, which one it culls against. Both
    // decisions, and the reasons for them, live in `viewpoint` where they are tested.
    let vp = viewpoint::resolve(
        world,
        state.editor_camera,
        state.game_camera,
        aspect,
        post_params.exposure,
    );
    let is_playing_mode = vp.is_playing_mode;
    let cam_fov = vp.fov;
    let cam_pos = vp.camera.position;

    // Event: Spawning moved to spawner_update_system.
    // Event: Texture Loading moved to main render loop pass before execute_render_pipeline.

    // z = elapsed time for fluid caustics/wave animation (fluid_composite.wgsl reads it);
    // was hardcoded 0.0 → frozen water (same bug as the gizmo runtime path).
    let elapsed_time = world
        .get_resource::<gizmo::core::time::Time>()
        .map(|t| t.elapsed() as f32)
        .unwrap_or(0.0);

    // Built by `viewpoint::resolve` above, exposure included — the post block carries the ACTIVE
    // CAMERA's exposure into it (see `passes.rs`), which is the value the exported build reads
    // too; the scene block's copy is unread (see `SceneUniforms::new`).
    let camera = vp.camera;
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
    // Same as the game path: the forward shader's light loop reads these, so skipping this upload
    // means an unlit viewport rather than a slower one.
    renderer.scene.upload_clusters(&renderer.queue, &setup.clusters);

    // --- BATCHING (INSTANCING) HAZIRLIĞI VE FRUSTUM CULLING ---
    use gizmo::renderer::renderer::InstanceRaw;

    // Culls against the GAME camera even in edit mode — deliberately, so the viewport can show
    // what the game would and would not draw. See `viewpoint::culling_frustum`, where that is
    // tested rather than asserted in a comment.
    let culling_frustum = viewpoint::culling_frustum(world, state.game_camera, aspect, &vp);
    // The gizmo pass draws the game camera's frustum as a wire box in edit mode.
    let game_view_proj = viewpoint::game_view_proj(world, state.game_camera, aspect, &vp);

    // Per-cascade LIGHT frusta — shadow casters are culled against these (not the camera
    // frustum), so off-screen objects that cast shadows INTO view aren't dropped.
    let cascade_frusta: [gizmo::renderer::Frustum; 4] = std::array::from_fn(|i| {
        gizmo::renderer::Frustum::from_matrix(&Mat4::from_cols_array_2d(&light_view_proj_cascades[i]))
    });

    let mut debug_aabbs = Vec::new();

    // Set inside the block below: false means the Game panel still needs the fallback copy.
    let mut game_view_rendered = false;
    // Filled inside the block below, published to the world after it.
    let mut render_stats = gizmo::renderer::components::RenderStats::default();

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
        let editor_only = world.borrow::<gizmo::core::component::EditorOnly>();

        if let Some(mut q) = world.query::<(&Mesh, &gizmo::physics::components::GlobalTransform, &Material)>() {
            for (e, (mesh, global_trans, mat)) in q.iter_mut() {
                // Sadece MeshRenderer tagli olanları çiz
                if renderers.get(e).is_none() {
                    continue;
                }

                // Gizli olarak işaretlenmiş objeleri (ve gizli bir ebeveynin altındakileri) atla
                if hidden.contains(&e) {
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
                let dist = gizmo::renderer::components::effective_lod_distance(
                    cam_pos.distance(Vec3::new(model.w_axis.x, model.w_axis.y, model.w_axis.z)),
                    renderers.get(e).map(|r| r.lod_bias).unwrap_or(1.0),
                );
                let active_mesh = match gizmo::renderer::components::LodGroup::pick(
                    lod_groups.get(e),
                    mesh,
                    dist,
                ) {
                    Some(m) => m,
                    None => continue, // CULL edildi!
                };

                // What the shader is HANDED, which is not always what was authored: the backdrop
                // pipeline's vertex shader adds the camera position, so a PLACED backdrop needs it
                // taken back out here or the two do not cancel and it rides the camera like a
                // locked one. The engine's path has always done this; this one uploaded the raw
                // matrix, so a backdrop the level placed drifted with the editor camera and sat
                // still in the game. Identity for every other material type.
                let upload_model =
                    gizmo::renderer::instance_model(mat.material_type, &model, cam_pos);
                let instance_data = InstanceRaw::new(
                    upload_model.to_cols_array_2d(),
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
                // Wired 2026-08-24 alongside the game path. Before it, `route(Water)` returned
                // `route(Pbr)`'s answer and this viewport shaded water as PBR by accident; once
                // the game path started routing it forward, the same material's instance flag
                // became 1.0 — which `shader.wgsl` reads as "skip the lights" — so leaving this
                // loop alone would have turned editor water FLAT rather than merely different.
                let is_water = routing.is_water;
                // Editor furniture, so the game view can leave it out. Not derivable from the
                // material: a light icon is an ordinary unlit cube.
                let is_editor_only = editor_only.get(e).is_some();
                // Per-object shadow casting. `MeshRenderer` is guaranteed present — the loop above
                // skips entities without one.
                let shadows = renderers.get(e).map(|r| r.shadows).unwrap_or_default();
                let casts_shadows = shadows.casts();
                let visible_in_camera = shadows.visible();

                // Water goes in the opaque map whatever its alpha: it has its own pass with its
                // own pipeline, and the transparent pass would draw it with `shader.wgsl`.
                let batches = if mat.is_transparent && !is_water {
                    &mut *transparent_batches
                } else if mat.is_double_sided {
                    &mut *opaque_double_sided_batches
                } else {
                    &mut *opaque_batches
                };

                let batch = batches
                    .entry((
                        vbuf_ptr,
                        bg_ptr,
                        skel_ptr,
                        is_skybox,
                        is_grid,
                        is_unlit,
                        is_backdrop,
                        is_water,
                        is_editor_only,
                        casts_shadows,
                        visible_in_camera,
                    ))
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
                        is_water,
                        is_editor_only,
                        casts_shadows,
                        visible_in_camera,
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
                        gizmo::renderer::sort_back_to_front(&mut batch.instances, cam_pos);
                    }
                    // Inter-batch: compute each batch's centroid depth once, farthest first.
                    let mut keyed: Vec<(f32, BatchData)> = drained
                        .into_iter()
                        .map(|b| (gizmo::renderer::batch_depth(&b.instances, cam_pos), b))
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
                        is_water: batch.is_water,
                        is_editor_only: batch.is_editor_only,
                        casts_shadows: batch.casts_shadows,
                        visible_in_camera: batch.visible_in_camera,
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
                    // rand 0.10 split the trait: `Rng` is the core one (re-exported from
                    // rand_core) and the convenience methods — `random_range` here — moved to
                    // `RngExt`. Importing the old one leaves them out of scope with an
                    // "unused import" warning as the only clue.
                    use rand::RngExt;
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

            // What this frame cost, measured from the list that is about to be drawn rather than
            // guessed: one draw call per batch, and triangles counted with their instances. The
            // editor's RENDER STATS overlay reads this; before it, the field it displayed
            // (`StudioState::draw_call_count`) was never written by anything.
            {
                let mut stats = gizmo::renderer::components::RenderStats::default();
                for b in flat_batches.iter() {
                    let instances = b.end_instance.saturating_sub(b.start_instance);
                    if instances == 0 {
                        continue;
                    }
                    stats.draw_calls += 1;
                    stats.instances += instances;
                    let indices = if b.index_count > 0 { b.index_count } else { b.vertex_count };
                    stats.triangles += (indices / 3) * instances;
                }
                // GPU memory, sampled about once a second. `generate_allocator_report` builds a
                // Vec of every live allocation, so calling it per frame would cost more than the
                // number is worth — and it does not move at frame rate anyway.
                let previous = world
                    .get_resource::<gizmo::renderer::components::RenderStats>()
                    .map(|r| (r.gpu_allocated_bytes, r.gpu_sampled_at))
                    .unwrap_or((None, f32::NEG_INFINITY));
                if elapsed_time - previous.1 >= 1.0 {
                    stats.gpu_allocated_bytes = renderer
                        .device
                        .generate_allocator_report()
                        .map(|r| r.total_allocated_bytes);
                    stats.gpu_sampled_at = elapsed_time;
                } else {
                    stats.gpu_allocated_bytes = previous.0;
                    stats.gpu_sampled_at = previous.1;
                }

                render_stats = stats;
            }

            // --- GAME VIEW: bu karenin İLK çizimi, kendi encoder'ında ---
            //
            // Sırası kasıtlı ve düzeltilmesi gereken kusurun ta kendisi: `Queue::write_buffer`
            // encoder komutlarıyla araya girmez. Bir submit'ten önce yapılan bütün yazımlar o
            // submit'teki BÜTÜN geçişler için geçerlidir. Uniform'u kare ortasında ikinci kez
            // yazıp "artık oyun kamerası" demek bu yüzden çalışmıyor — ilk çizim de o değeri
            // görüyor ve iki panel yine aynı görüntüyü veriyor (ölçüldü: 65536 baytın 0'ı farklı).
            //
            // Doğrusu, oyun görünümünü kendi encoder'ına çizip HEMEN submit etmek: o submit'e
            // kadar yazılanlar ona, sonrasında yazılanlar çağıranın encoder'ına uygulanır. Böylece
            // ikinci bir uniform tamponu ya da bind group çoğaltmasına gerek kalmıyor.
            // Only for a panel someone can see. In the default layout Scene and Game are tabs of
            // ONE dock leaf, so at most one of them is on screen — and this ran unconditionally,
            // paying for a second full scene pass (batches, uniforms, its own submit) every frame
            // to fill a texture behind another tab. Gating it took the studio's frame from
            // 2.63/3.02 ms to 1.65/1.69 ms, about 40%.
            //
            // The flag is this frame's answer, not a stale one: the app loop runs the egui hook
            // (`event.rs`, `ui_fn`) before the render hook, so the panels have already said whether
            // they drew. The one cost is the frame you switch tabs on — the Game panel announces
            // itself and paints in the same frame, so it shows the previous contents once (black,
            // if it has never rendered) and is correct from the next frame. Rendering a frame early
            // for a panel that is not there yet is the trade this whole gate exists to refuse.
            if !is_playing_mode && game_view_visible {
                let game_batches: Vec<batching::FlatBatchData> = flat_batches
                    .iter()
                    .filter(|b| !b.is_editor_only)
                    .cloned()
                    .collect();
                game_view_rendered = record_game_view(
                    world,
                    renderer,
                    &game_batches,
                    state.game_camera,
                    aspect,
                    ed_shading_mode,
                    elapsed_time,
                    post_params,
                );
                if game_view_rendered {
                    // Aşağıdaki geçişler için editörün uniform'larını geri yaz. Bu yazımlar
                    // çağıranın submit'inden önce olduğu için ona uygulanırlar.
                    renderer.update_post_process(&renderer.queue, post_params.with_camera(&camera));
                    renderer.queue.write_buffer(
                        &renderer.scene.global_uniform_buffer,
                        0,
                        gizmo::bytemuck::cast_slice(&[scene_uniform_data]),
                    );
                }
            }

            // --- 1. CSM GÖLGE PASS + 2. ANA RENDER PASS (Tier 3: geçişler ayrı fn) ---
            record_studio_shadow_passes(encoder, renderer, flat_batches.as_slice(), &light_view_proj_cascades);
            record_studio_main_pass(
                encoder, renderer, world, flat_batches.as_slice(), game_view_proj, &debug_aabbs,
                show_colliders, true, ed_shading_mode,
            );
            // A `Decal` before the particles: a decal paints a surface, a particle floats in
            // front of one. Both after the main pass, because both sample the depth this frame
            // just wrote and a texture cannot be attachment and sampler in one pass.
            //
            // This is the engine's own pass (`gizmo::systems::render::record_forward_decals`),
            // not a studio copy — the editor drawing forward is exactly why decals were invisible
            // here until the game ran, and a second implementation is how that would come back.
            gizmo::systems::render::record_forward_decals(encoder, renderer, world, cam_pos);
            record_studio_particle_pass(encoder, renderer);
            // The fluid's surface, the engine's own pass again. The viewport runs the WHOLE fluid
            // rather than the game's distance-scaled slice: an editor shows what is there, which
            // is the same reasoning the render-parity exceptions already record for `lod_vbufs`.
            gizmo::systems::render::record_fluid_surface(encoder, renderer, studio_fluid_particles(renderer));
            // `Text`, the engine's own pass for the third time and for the same reason: a label's
            // place in the world is one decision, and the version of it the editor shows has to be
            // the version the game draws. Both `TextSpace` variants come with it — a world label
            // depth-tested against this viewport's own depth, screen text over all of it — because
            // the placement lives in `gizmo_renderer::text`, not in either loop. Last of the world
            // passes for exactly that reason.
            gizmo::systems::render::record_text(encoder, renderer, world);

    }); // Cikis: CACHE.with bloğu

    world.insert_resource(render_stats);

    // Çizilen Gizmo'ları sonraki frame için temizle
    if let Some(mut gizmos) = world.get_resource_mut::<gizmo::renderer::Gizmos>() {
        gizmos.clear();
    }

    // --- 3. POST-PROCESSING (Bloom + Tone Mapping → Ekrana Yaz) ---
    // Scoped so the resource borrow ends before the game view needs the world again.
    {
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
    }

    // --- 4. GAME VIEW: the same world, from the other camera, without the furniture ---
    //
    // This used to be `run_post_processing` a second time on the SAME hdr texture, which made the
    // Game panel a byte-identical copy of the Scene panel — measured, with two cameras pointed in
    // opposite directions. One render, two outputs.
    //
    // In play mode both panels legitimately show the game camera (`viewpoint::resolve` already
    // switched to it), so the cheap path is still the correct one there.
    if !game_view_rendered {
        // Play mode (both panels legitimately show the game camera), or a scene with no game
        // camera at all: the HDR texture already holds the only picture there is.
        let game_view = world
            .get_resource::<gizmo::renderer::components::GameRenderTarget>()
            .map(|t| t.0.view.clone());
        if let Some(game_view) = game_view {
            renderer.run_post_processing(encoder, &game_view);
        }
    }
}

/// Draws the game camera's view of the scene into `output`, returning false if there is no game
/// camera to draw from.
///
/// Runs the same steps as the main frame — scene setup, cascades, main pass, post — with three
/// differences, each of which is the point:
///
/// - the camera is the **game** camera, resolved by name rather than by mode;
/// - the batch list has already had `EditorOnly` filtered out by the caller, and `draw_chrome` is
///   false, so neither the furniture in the batches nor the grid/gizmo/collider draws appear;
/// - the cascades are refitted to this camera. Reusing the editor camera's would be cheaper and
///   wrong in a specific way: the shader picks a cascade from view depth, so the splits have to
///   belong to the camera doing the viewing or the wrong cascade gets sampled.
///
/// # Cost
///
/// A second full scene render plus four cascades, every frame the Game panel exists — measured on
/// the default studio scene at 481 → 453 FPS, about 6%. Culling and the instance buffer are shared
/// with the editor's render, so what is paid twice is pass recording and rasterisation, not the
/// CPU-side batching. There is deliberately no "is the panel visible" gate: nothing in the editor
/// state reports that today, and inventing a visibility protocol to save 6% on a preview users
/// expect to be live is the wrong trade until a heavy scene says otherwise.
#[allow(clippy::too_many_arguments)]
fn record_game_view(
    world: &World,
    renderer: &mut gizmo::renderer::Renderer,
    batches: &[batching::FlatBatchData],
    game_camera: u32,
    aspect: f32,
    ed_shading_mode: u32,
    elapsed_time: f32,
    post_params: gizmo::renderer::PostProcessUniforms,
) -> bool {
    let Some(output) = world
        .get_resource::<gizmo::renderer::components::GameRenderTarget>()
        .map(|t| t.0.view.clone())
    else {
        return false;
    };
    let Some(camera) = viewpoint::camera_frame(world, game_camera, aspect, post_params.exposure)
    else {
        return false;
    };
    let cam_fov = {
        let cameras = world.borrow::<gizmo::renderer::components::Camera>();
        match cameras.get(game_camera) {
            Some(c) => c.fov,
            None => return false,
        }
    };

    let setup = gizmo::systems::render::collect_scene_setup(
        world,
        &gizmo::systems::render::SceneSetupInputs {
            camera,
            aspect,
            cam_fov,
            shadow_caster: gizmo::systems::render::ShadowCaster::SunOrFirstLight,
            environment: gizmo::renderer::EnvironmentFrame {
                shading_mode: ed_shading_mode,
                ..Default::default()
            },
            point_shadows_enabled: false,
            elapsed_time,
        },
    );
    let cascades = setup.cascade_view_projs.map(|m| m.to_cols_array_2d());

    renderer.update_post_process(&renderer.queue, post_params.with_camera(&camera));
    let scene_uniform = gizmo::renderer::SceneUniforms::new(&setup.frame);
    renderer.queue.write_buffer(
        &renderer.scene.global_uniform_buffer,
        0,
        gizmo::bytemuck::cast_slice(&[scene_uniform]),
    );

    // Its own encoder, submitted before returning — that submission boundary is what makes the
    // uniform writes above belong to this render and not to the editor's.
    let mut enc = renderer
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Game View") });
    record_studio_shadow_passes(&mut enc, renderer, batches, &cascades);
    // The game view never takes the editor's debug shading: it is the picture the game shows.
    record_studio_main_pass(&mut enc, renderer, world, batches, None, &[], false, false, 0);
    // No particle pass: the GPU particle compute for this frame is recorded into the caller's
    // encoder, which runs after this one, so drawing them here would show the previous frame's
    // positions. A preview panel is not worth a second compute dispatch.
    renderer.run_post_processing(&mut enc, &output);
    renderer.queue.submit(std::iter::once(enc.finish()));
    true
}



