use gizmo::ai::components::NavAgent;
use gizmo::audio::AudioSource;
use gizmo::physics::components::{CharacterController, Collider, RigidBody, Transform, Velocity};
use gizmo::physics::world::PhysicsWorld;
use gizmo::prelude::*;
use gizmo::renderer::asset::AssetManager;
use gizmo::renderer::async_assets::AsyncAssetLoader;
use gizmo::renderer::components::{Camera, LodGroup, LodLevel, Material, MeshRenderer};
use gizmo::winit::keyboard::KeyCode;
use std::f32::consts::PI;

struct RpgState {
    character_entity: gizmo::core::Entity,
    camera_yaw: f32,
    camera_pitch: f32,
    asset_watcher: std::sync::Mutex<Option<gizmo::renderer::AssetWatcher>>,
}

struct ChunkAssets {
    high_res_tree: gizmo::renderer::components::Mesh,
    low_res_tree: gizmo::renderer::components::Mesh,
    tree_mat: gizmo::renderer::components::Material,
    ground_mesh: gizmo::renderer::components::Mesh,
    ground_mat: gizmo::renderer::components::Material,
}

fn setup(world: &mut World, renderer: &Renderer) -> RpgState {
    println!("⚔️ GIZMO ENGINE RPG TEST BAŞLIYOR ⚔️");
    let mut asset_manager = AssetManager::new();
    let phys_world = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0));
    world.insert_resource(phys_world);
    world.insert_resource(AsyncAssetLoader::new());

    // --- SKYBOX ---
    let skybox_mesh = AssetManager::create_inverted_cube(&renderer.device);
    let sky_path = if std::path::Path::new("tut/assets/sky.jpg").exists() {
        "tut/assets/sky.jpg"
    } else {
        "assets/sky.jpg"
    };
    let sky_tex = asset_manager
        .load_material_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
            sky_path,
        )
        .expect("Failed to load skybox texture");
    let sky_mat = Material::new(sky_tex).with_skybox();

    let sky_ent = world.spawn();
    world.add_component(
        sky_ent,
        Transform::new(Vec3::ZERO).with_scale(Vec3::splat(2000.0)),
    );
    world.add_component(sky_ent, skybox_mesh);
    world.add_component(sky_ent, sky_mat);
    world.add_component(sky_ent, MeshRenderer::new());

    // --- GROUND (DEVASA AÇIK DÜNYA ZEMİNİ) ---
    let ground_mesh = AssetManager::create_cube(&renderer.device);
    let ground_tex = asset_manager.create_checkerboard_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );
    // Asenkron Doku Akışı test edilmesi için Ground texture kaynağı bırakıyoruz
    let ground_mat = Material::new(ground_tex.clone())
        .with_pbr(Vec4::new(0.4, 0.6, 0.3, 1.0), 0.9, 0.0)
        .with_texture_source("assets/textures/grass_high_res.png".to_string());

    // --- AÇIK DÜNYA CHUNK ASSETLERİ ---
    println!("LOD TESTİ: Orman Assetleri hazırlanıyor...");
    let high_res_tree = AssetManager::create_sphere(&renderer.device, 1.0, 32, 32);
    let low_res_tree = AssetManager::create_sphere(&renderer.device, 1.0, 8, 8);
    let tree_mat =
        Material::new(ground_tex.clone()).with_pbr(Vec4::new(0.1, 0.8, 0.1, 1.0), 0.8, 0.0);

    world.insert_resource(ChunkAssets {
        high_res_tree,
        low_res_tree,
        tree_mat,
        ground_mesh: ground_mesh.clone(),
        ground_mat: ground_mat.clone(),
    });

    let empty_prefab = world.spawn();
    let mut pool_manager = gizmo::core::PoolManager::new();
    pool_manager.register_pool("chunk_obj", empty_prefab);
    world.insert_resource(pool_manager);

    // --- YAPAY ZEKA NPCLER ---
    println!("YAPAY ZEKA: Köylüler spawn oluyor...");
    for i in 0..5 {
        let npc = world.spawn();
        let npc_mesh = AssetManager::create_sphere(&renderer.device, 0.5, 16, 16);
        let npc_mat =
            Material::new(ground_tex.clone()).with_pbr(Vec4::new(1.0, 0.2, 0.2, 1.0), 0.5, 0.0);

        world.add_component(
            npc,
            Transform::new(Vec3::new(10.0 + i as f32 * 5.0, 1.0, 10.0)),
        );
        world.add_component(npc, npc_mesh);
        world.add_component(npc, npc_mat);
        world.add_component(npc, MeshRenderer::new());
        world.add_component(npc, Collider::capsule(0.5, 0.5));
        world.add_component(npc, RigidBody::new_kinematic());
        world.add_component(npc, Velocity::default());
        world.add_component(npc, CharacterController::default());
        world.add_component(npc, NavAgent::default()); // AI sistemi tarafından yürütülecek

        // 3D Spatial Ses Denemesi
        let mut audio = AudioSource::new("assets/sounds/villager_hum.wav");
        audio.is_3d = true;
        audio.max_distance = 20.0;
        world.add_component(npc, audio);
    }

    // --- ANA KARAKTER (PLAYER) ---
    println!("KARAKTER: Oyuncu yaratılıyor...");
    let char_ent = world.spawn();
    let char_mesh = AssetManager::create_sphere(&renderer.device, 0.5, 16, 16);
    let char_mat =
        Material::new(ground_tex.clone()).with_pbr(Vec4::new(0.2, 0.2, 1.0, 1.0), 0.5, 0.5);

    world.add_component(char_ent, Transform::new(Vec3::new(0.0, 2.0, 0.0)));
    world.add_component(char_ent, char_mesh);
    world.add_component(char_ent, char_mat);
    world.add_component(char_ent, MeshRenderer::new());

    let mut kcc = CharacterController::default();
    kcc.speed = 10.0;
    kcc.jump_speed = 8.0;
    kcc.step_height = 0.5;

    world.add_component(char_ent, kcc);
    world.add_component(char_ent, Collider::capsule(0.5, 0.5));
    world.add_component(char_ent, RigidBody::new_kinematic());
    world.add_component(char_ent, Velocity::default());

    // --- KAMERA ---
    let camera_ent = world.spawn();
    world.add_component(camera_ent, Transform::new(Vec3::new(0.0, 5.0, 10.0)));
    world.add_component(
        camera_ent,
        Camera::new(
            std::f32::consts::FRAC_PI_3,
            0.1,
            1500.0,
            0.0,
            -PI / 8.0,
            true,
        ),
    );

    // --- GÜNEŞ (DIRECTIONAL LIGHT) ---
    let sun = world.spawn();
    world.add_component(
        sun,
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_x(-PI / 4.0)),
    );
    world.add_component(
        sun,
        gizmo::renderer::components::DirectionalLight::new(
            Vec3::new(1.0, 0.95, 0.9),
            5.0,
            gizmo::renderer::components::LightRole::Sun,
        ),
    );

    println!("✅ SETUP: Tamamlandı!");
    let watcher =
        gizmo::renderer::AssetWatcher::new(&["assets/shaders", "assets/textures", "assets/models"]);
    RpgState {
        character_entity: char_ent,
        camera_yaw: 0.0,
        camera_pitch: -PI / 8.0,
        asset_watcher: std::sync::Mutex::new(watcher),
    }
}

fn update(world: &mut World, state: &mut RpgState, dt: f32, input: &gizmo::core::input::Input) {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex;

    static FRAME_COUNT: AtomicU32 = AtomicU32::new(0);
    static REAL_TIME_ACCUM: Mutex<f32> = Mutex::new(0.0);
    static LAST_T: Mutex<Option<std::time::Instant>> = Mutex::new(None);

    let now = std::time::Instant::now();
    if let Ok(mut last) = LAST_T.lock() {
        if let Some(prev) = *last {
            let real_dt = now.duration_since(prev).as_secs_f32();
            if let Ok(mut accum) = REAL_TIME_ACCUM.lock() {
                *accum += real_dt;
                let count = FRAME_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
                if *accum >= 1.0 {
                    println!(
                        "REAL FPS: {} (real avg dt: {:.2}ms) | Entities: {}",
                        count,
                        (*accum / count as f32) * 1000.0,
                        world.iter_alive_entities().len()
                    );
                    FRAME_COUNT.store(0, Ordering::Relaxed);
                    *accum = 0.0;
                }
            }
        }
        *last = Some(now);
    }

    // --- KAMERA KONTROLLERİ ---
    if input.is_mouse_button_pressed(1) {
        let delta = input.mouse_delta();
        state.camera_yaw -= delta.0 * 0.005;
        state.camera_pitch -= delta.1 * 0.005;
        state.camera_pitch = state.camera_pitch.clamp(-PI / 2.0 + 0.1, PI / 2.0 - 0.1);
    }

    let fx = state.camera_yaw.cos() * state.camera_pitch.cos();
    let fy = state.camera_pitch.sin();
    let fz = state.camera_yaw.sin() * state.camera_pitch.cos();
    let forward = Vec3::new(fx, fy, fz).normalize_or_zero();
    let right = Vec3::new(-state.camera_yaw.sin(), 0.0, state.camera_yaw.cos()).normalize_or_zero();

    let mut move_forward = forward;
    move_forward.y = 0.0;
    move_forward = move_forward.normalize_or_zero();
    let mut move_right = right;
    move_right.y = 0.0;
    move_right = move_right.normalize_or_zero();
    let cam_rot = Quat::from_rotation_y(-state.camera_yaw);

    // --- KARAKTER HAREKETİ ---
    let mut char_pos = Vec3::ZERO;
    if let Some(kcc) = world
        .borrow_mut::<CharacterController>()
        .get_mut(state.character_entity.id())
    {
        let mut move_dir = Vec3::ZERO;
        if input.is_key_pressed(KeyCode::KeyW as u32) {
            move_dir += move_forward;
        }
        if input.is_key_pressed(KeyCode::KeyS as u32) {
            move_dir -= move_forward;
        }
        if input.is_key_pressed(KeyCode::KeyD as u32) {
            move_dir += move_right;
        }
        if input.is_key_pressed(KeyCode::KeyA as u32) {
            move_dir -= move_right;
        }

        move_dir = move_dir.normalize_or_zero();
        kcc.target_velocity = move_dir * kcc.speed;

        if input.is_key_pressed(KeyCode::Space as u32) {
            kcc.jump_buffer_timer = kcc.jump_buffer_time;
        }
    }

    // --- FİZİK MOTORU VE ASENKRON DOKU AKIŞI (STREAMING) ADIMI ---
    // Ölüm sarmalını önlemek için fizikte max 2 adıma izin veriyoruz
    let mut physics_dt = dt.min(0.032);
    while physics_dt > 0.0 {
        let step = physics_dt.min(0.016);
        gizmo::physics::system::physics_step_system(world, step);
        physics_dt -= step;
    }

    // Kamera pozisyonuna göre Doku Akış (Texture Streaming) Sistemi Çalıştırılır
    {
        if let Some(trans) = world.borrow::<Transform>().get(state.character_entity.id()) {
            char_pos = trans.position;
        }
    }
    // Texture Streaming Sistemi (Lod Level'e göre uzaklaştıkça memory'den silinebilir veya eklenebilir)    // Açık Dünya (Open World) Chunk Sistemini çalıştır
    gizmo::systems::open_world_chunk_system(
        world,
        char_pos,
        // --- LOAD CHUNK ---
        |w, coord| {
            let assets = {
                let res = w.get_resource::<ChunkAssets>().unwrap();
                (
                    res.high_res_tree.clone(),
                    res.low_res_tree.clone(),
                    res.tree_mat.clone(),
                    res.ground_mesh.clone(),
                    res.ground_mat.clone(),
                )
            };

            use rand::rngs::StdRng;
            use rand::{Rng, SeedableRng};
            let seed = (coord.0 as u64)
                .wrapping_mul(31)
                .wrapping_add(coord.1 as u64);
            let mut rng = StdRng::seed_from_u64(seed);

            let chunk_center =
                gizmo::systems::chunk_system::ChunkManager::chunk_to_world_pos(coord);

            // --- YARDIMCI FONKSİYON: Havuzdan Obje Al ---
            let get_pooled_entity = |w: &mut World| -> gizmo::core::Entity {
                w.resource_scope(|w, pool: &mut gizmo::core::PoolManager| {
                    pool.instantiate(w, "chunk_obj").unwrap()
                })
                .unwrap()
            };

            // --- CHUNK ZEMİNİ ---
            let ground = get_pooled_entity(w);
            let ground_id = ground.to_bits();
            let ground_scale = Vec3::new(50.0, 1.0, 50.0);
            w.add_component(
                ground,
                Transform::new(Vec3::new(chunk_center.x, -1.0, chunk_center.z))
                    .with_scale(ground_scale),
            );
            w.add_component(ground, assets.3.clone()); // ground_mesh
            w.add_component(ground, assets.4.clone()); // ground_mat
            w.add_component(ground, MeshRenderer::new());

            // Otomatik Bounding Box hesaplama
            let ground_he = assets.3.bounds.half_extents();
            let scaled_he = Vec3::new(
                ground_he.x * ground_scale.x,
                ground_he.y * ground_scale.y,
                ground_he.z * ground_scale.z,
            );
            w.add_component(ground, Collider::box_collider(scaled_he));

            w.add_component(ground, RigidBody::new_static());
            w.add_component(ground, Velocity::default());
            w.get_resource_mut::<gizmo::systems::chunk_system::ChunkManager>()
                .unwrap()
                .register_entity(coord, ground_id);

            // Her chunk'a 5 ağaç dikelim
            for _ in 0..5 {
                let x = chunk_center.x + rng.gen_range(-40.0..40.0);
                let z = chunk_center.z + rng.gen_range(-40.0..40.0);

                if coord.0 == 0 && coord.1 == 0 && x.abs() < 20.0 && z.abs() < 20.0 {
                    continue;
                }

                let tree = get_pooled_entity(w);
                let tree_id = tree.to_bits();
                let tree_scale = Vec3::new(2.0, 5.0, 2.0);
                w.add_component(
                    tree,
                    Transform::new(Vec3::new(x, 0.0, z)).with_scale(tree_scale),
                );
                w.add_component(tree, assets.1.clone()); // Low res base
                w.add_component(tree, assets.2.clone()); // Mat
                w.add_component(tree, MeshRenderer::new());

                // Ağacın otomatik capsule boyutu hesaplaması
                let tree_he = assets.1.bounds.half_extents();
                let radius = tree_he.x.max(tree_he.z) * tree_scale.x;
                let half_height = tree_he.y * tree_scale.y;
                w.add_component(tree, Collider::capsule(radius, half_height));

                w.add_component(tree, RigidBody::new_static());
                w.add_component(tree, Velocity::default());
                w.get_resource_mut::<gizmo::systems::chunk_system::ChunkManager>()
                    .unwrap()
                    .register_entity(coord, tree_id);

                let lod_group = LodGroup::new(vec![
                    LodLevel::new(assets.0.clone(), 30.0),
                    LodLevel::new(assets.1.clone(), 150.0),
                ]);
                w.add_component(tree, lod_group);
            }
        },
        // --- UNLOAD CHUNK (Object Pooling) ---
        |w, _coord, entities| {
            w.resource_scope(|w, pool: &mut gizmo::core::PoolManager| {
                for entity_bits in &entities {
                    let e = gizmo::core::Entity::from_bits(*entity_bits);
                    pool.destroy(w, "chunk_obj", e);
                }
            });
        },
    );

    if let Some(trans) = world
        .borrow_mut::<Transform>()
        .get_mut(state.character_entity.id())
    {
        trans.rotation = Quat::from_rotation_y(state.camera_yaw);
    }

    // TPS/FPS Kamera Takibi
    let cam_pos = char_pos + Vec3::new(0.0, 1.5, 0.0);
    if let Some(mut q) = world.query::<(
        gizmo::core::query::Mut<Transform>,
        gizmo::core::query::Mut<Camera>,
    )>() {
        for (_, (mut trans, mut cam)) in q.iter_mut() {
            trans.position = cam_pos;
            trans.rotation = cam_rot;
            cam.yaw = state.camera_yaw;
            cam.pitch = state.camera_pitch;
        }
    }
}

fn render(
    world: &mut World,
    state: &RpgState,
    encoder: &mut gizmo::wgpu::CommandEncoder,
    view: &gizmo::wgpu::TextureView,
    renderer: &mut Renderer,
    _light_time: f32,
) {
    if let Ok(mut watcher_guard) = state.asset_watcher.lock() {
        if let Some(watcher) = watcher_guard.as_mut() {
            let changed_files = watcher.poll_changes();
            if !changed_files.is_empty() {
                let mut rebuild_shaders = false;
                for path in changed_files {
                    let ext = path
                        .extension()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_lowercase();
                    if ext == "wgsl" {
                        rebuild_shaders = true;
                    }
                }
                if rebuild_shaders {
                    println!("♻️ Hot-Reload: Shaderlar yeniden derleniyor...");
                    renderer.rebuild_shaders();
                    println!("✅ Hot-Reload: Shader derleme tamamlandı.");
                }
            }
        }
    }

    // Gökyüzünden düşen su damlalarını kapatıyoruz (performans için)
    renderer.gpu_fluid = None;
    renderer.gpu_particles = None;
    renderer.gpu_physics = None;

    // Default render pass (Kamera, Işıklar, Model, Skybox, SSGI, SSR, Bloom vs. her şeyi yapar)
    gizmo::systems::render::default_render_pass(world, encoder, view, renderer);
}

fn main() {
    App::<RpgState>::new("Gizmo Engine - Open World RPG Demo", 1600, 900)
        .set_setup(setup)
        .set_update(update)
        .set_render(render)
        .run();
}
