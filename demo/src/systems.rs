use crate::GameState;
use gizmo::core::input::Input;
use gizmo::math::{Quat, Vec3, Vec4};
use gizmo::prelude::*;

pub fn free_camera_system(world: &mut World, dt: f32) {
    let state_opt = world.get_resource::<crate::state::EngineConfig>();
    let input_opt = world.get_resource::<gizmo::core::input::Input>();

    if state_opt.is_none() || input_opt.is_none() {
        // EngineConfig henüz eklenmediyse çık (Crash'i önler)
        return;
    }
    let state = state_opt.unwrap();
    let input = input_opt.unwrap();

    if !state.free_cam {
        return;
    }

    let active_camera_entity = state.active_camera_entity;

    let wants_pointer = world
        .get_resource::<egui::Context>()
        .map(|c| c.wants_pointer_input())
        .unwrap_or(false);
    let wants_keyboard = world
        .get_resource::<egui::Context>()
        .map(|c| c.wants_keyboard_input())
        .unwrap_or(false);

    let speed = 25.0 * dt;
    let mut move_delta = Vec3::ZERO;
    let mut do_rotation = false;

    if input.is_mouse_button_pressed(gizmo::core::input::mouse::RIGHT) && !wants_pointer {
        do_rotation = true;
    }

    // WASD kontrol
    if !wants_keyboard {
        use gizmo::winit::keyboard::KeyCode;

        // Shift ile hızlandır
        let mut current_speed = speed;
        if input.is_key_pressed(KeyCode::ShiftLeft as u32) {
            current_speed *= 10.0;
        }

        if input.is_key_pressed(KeyCode::KeyW as u32) {
            move_delta += Vec3::new(0.0, 0.0, 1.0) * current_speed;
        } // W
        if input.is_key_pressed(KeyCode::KeyS as u32) {
            move_delta -= Vec3::new(0.0, 0.0, 1.0) * current_speed;
        } // S
        if input.is_key_pressed(KeyCode::KeyA as u32) {
            move_delta -= Vec3::new(1.0, 0.0, 0.0) * current_speed;
        } // A
        if input.is_key_pressed(KeyCode::KeyD as u32) {
            move_delta += Vec3::new(1.0, 0.0, 0.0) * current_speed;
        } // D
    }

    if let Some(mut query) = world.query_mut_mut::<Camera, Transform>() {
        for (id, cam, trans) in query.iter_mut() {
            if id == active_camera_entity {
                if do_rotation {
                    let delta = input.mouse_delta();
                    cam.yaw += delta.0 * 0.002;
                    cam.pitch -= delta.1 * 0.002;
                    cam.pitch = cam.pitch.clamp(-1.5, 1.5);
                }

                if move_delta.length_squared() > 0.0 {
                    let f = cam.get_front();
                    let r = cam.get_right();
                    let actual_move = f * move_delta.z + r * move_delta.x;
                    trans.position += actual_move;
                }
            }
        }
    }
}

pub fn transform_hierarchy_system(world: &mut World) {
    // 1. Önce herkesin local matrix'ini güncelle (PARALEL!)
    if let Some(mut transforms) = world.borrow_mut::<Transform>() {
        use rayon::prelude::*;
        transforms.dense.par_iter_mut().for_each(|t| {
            t.data.update_local_matrix();
        });
    }

    // 2. ROOT (Kök) Objelerini bul (Üstünde Parent olmayanlar)
    let mut to_update = Vec::new();
    if let Some(transforms) = world.borrow::<Transform>() {
        let parents = world.borrow::<gizmo::core::component::Parent>();
        for entity_id in transforms.dense.iter().map(|e| e.entity) {
            let has_parent = if let Some(p) = &parents {
                p.contains(entity_id)
            } else {
                false
            };
            if !has_parent {
                to_update.push((entity_id, Mat4::IDENTITY));
            }
        }
    }

    // 3. BFS ile ağacı aşağıya doğru düzleştirerek Global Matrix hesapla
    let mut head = 0;
    if let (Some(mut transforms), Some(children_comp)) = (
        world.borrow_mut::<Transform>(),
        world.borrow::<gizmo::core::component::Children>(),
    ) {
        while head < to_update.len() {
            let (entity_id, parent_global) = to_update[head];
            head += 1;

            let mut current_global = Mat4::IDENTITY;

            // Bu child'ın global_matrix hesaplaması: Parent Global * Local
            if let Some(t) = transforms.get_mut(entity_id) {
                t.global_matrix = parent_global * t.local_matrix();
                current_global = t.global_matrix;
            }

            // Child node'ları kuyruğa ekle
            if let Some(children) = children_comp.get(entity_id) {
                for &child_id in &children.0 {
                    to_update.push((child_id, current_global));
                }
            }
        }
    }
}

#[allow(dead_code)]
pub(crate) fn audio_update_system(world: &mut World, state: &mut GameState) {
    let mut cam_pos = Vec3::ZERO;
    let mut cam_right = Vec3::new(1.0, 0.0, 0.0);

    if let (Some(cameras), Some(transforms)) =
        (world.borrow::<Camera>(), world.borrow::<Transform>())
    {
        if let Some(t) = transforms.get(state.player_id) {
            cam_pos = t.position;
        }
        if let Some(cam) = cameras.get(state.player_id) {
            cam_right = cam.get_right();
        }
    }

    let left_ear = cam_pos - cam_right * 0.5;
    let right_ear = cam_pos + cam_right * 0.5;

    if let Some(ref mut am) = state.audio {
        am.clean_dead_sinks();

        let audio_entities =
            if let Some(audio_sources) = world.borrow::<gizmo::audio::AudioSource>() {
                audio_sources
                    .dense
                    .iter()
                    .map(|e| e.entity)
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };

        if let (Some(mut audio_sources), Some(transforms)) = (
            world.borrow_mut::<gizmo::audio::AudioSource>(),
            world.borrow::<Transform>(),
        ) {
            for e in audio_entities {
                if let Some(audio_src) = audio_sources.get_mut(e) {
                    let curr_pos = transforms.get(e).map_or(Vec3::ZERO, |t| t.position);

                    // Fake pos to isolate Panning (fixed distance = 1.0)
                    let dir = if curr_pos == cam_pos {
                        Vec3::new(0.0, 0.0, 1.0)
                    } else {
                        (curr_pos - cam_pos).normalize()
                    };
                    let fake_pos = cam_pos + dir * 1.0;

                    // Henüz ses başlamamışsa (ilk kare veya yeniden tetikleme)
                    if audio_src._internal_sink_id.is_none() {
                        let emitter = [fake_pos.x, fake_pos.y, fake_pos.z];
                        let r_ear = [right_ear.x, right_ear.y, right_ear.z];
                        let l_ear = [left_ear.x, left_ear.y, left_ear.z];

                        let id = if audio_src.is_3d {
                            if audio_src.loop_sound {
                                am.play_3d_looped(&audio_src.sound_name, emitter, r_ear, l_ear)
                            } else {
                                am.play_3d(&audio_src.sound_name, emitter, r_ear, l_ear)
                            }
                        } else {
                            if audio_src.loop_sound {
                                am.play_looped(&audio_src.sound_name)
                            } else {
                                am.play(&audio_src.sound_name)
                            }
                        };
                        audio_src._internal_sink_id = id;
                    }

                    // Aktif sink varsa parametrelerini güncelle
                    if let Some(sid) = audio_src._internal_sink_id {
                        if !am.is_playing(sid, audio_src.is_3d) {
                            // Ses bitti — tek seferlik ise temizle, döngüsel ise yeniden başlat
                            audio_src._internal_sink_id = None;
                        } else {
                            if audio_src.is_3d {
                                am.update_spatial_sink(
                                    sid,
                                    [fake_pos.x, fake_pos.y, fake_pos.z],
                                    [right_ear.x, right_ear.y, right_ear.z],
                                    [left_ear.x, left_ear.y, left_ear.z],
                                );
                            }

                            // --- MESAFE ZAYIFLAMASI (Distance Attenuation) HESAPLAMASI ---
                            let mut final_volume = audio_src.volume;
                            if audio_src.is_3d {
                                let dist = cam_pos.distance(curr_pos);
                                if dist > audio_src.max_distance {
                                    final_volume = 0.0;
                                } else {
                                    // Linear uzaklık yerine karesel (Inverse-Square tarzı) bir yumuşatma
                                    let dist_ratio = 1.0 - (dist / audio_src.max_distance);
                                    final_volume = audio_src.volume * dist_ratio * dist_ratio;
                                }
                            }

                            am.set_sink_volume(sid, final_volume, audio_src.is_3d);
                            am.set_sink_pitch(sid, audio_src.pitch, audio_src.is_3d);
                        }
                    }
                }
            }
        }

        // --- YENİ EKLENEN ÇARPIŞMA (COLLISION) OLAYLARI DİNLEME ---
        if let Some(mut collision_events) =
            world.get_resource_mut::<gizmo::core::event::Events<gizmo::physics::CollisionEvent>>()
        {
            let mut top_events = collision_events.drain().collect::<Vec<_>>();

            // Performans için sadece en sert 15 çarpışmayı oynat! (Yüzlerce sesin aynı karede patlamasını engeller)
            top_events.sort_by(|a, b| {
                b.impulse
                    .partial_cmp(&a.impulse)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            for ev in top_events.into_iter().take(15) {
                let em = [ev.position.x, ev.position.y, ev.position.z];
                let r_e = [right_ear.x, right_ear.y, right_ear.z];
                let l_e = [left_ear.x, left_ear.y, left_ear.z];

                if let Some(sid) = am.play_3d("bounce", em, r_e, l_e) {
                    // Impulse değerine göre gerçekçi ses simülasyonu
                    // Çok sert vuran çınlar ve yüksek ses çıkarır, yavaş vuran "tık" eder.
                    let volume = (ev.impulse * 0.15).clamp(0.02, 1.0);
                    let pitch = 0.8 + (ev.impulse * 0.05).clamp(0.0, 0.6);

                    am.set_sink_volume(sid, volume, true);
                    am.set_sink_pitch(sid, pitch, true);
                }
            }
        }
    }
}

pub fn vehicle_controller_system(world: &mut World, dt: f32) {
    let state_opt = world.get_resource::<crate::state::EngineConfig>();
    let input_opt = world.get_resource::<Input>();
    let actions_opt = world.get_resource::<gizmo::core::input::ActionMap>();

    if state_opt.is_none() || input_opt.is_none() || actions_opt.is_none() {
        println!(
            "MISSING RESOURCE! state: {}, input: {}, actions: {}",
            state_opt.is_none(),
            input_opt.is_none(),
            actions_opt.is_none()
        );
        return;
    }

    let _state = state_opt.unwrap();
    let input = input_opt.unwrap();
    let actions = actions_opt.unwrap();
    if let Some(_am_ref) =
        gizmo::core::world::World::default().get_resource::<crate::state::AppMode>()
    {
        // AppMode kontrolünü global yapabiliriz ancak şimdilik manuel State içerisinden bakıyoruz.
    }

    let engine_power = 12000.0;
    let max_steer = 1.3; // 0.8'den 1.3'e çıkarıldı: Artık tekerlekler daha fazla dönecek!
    let mut current_engine = 0.0;
    let mut current_steer = 0.0;
    let mut current_brake = 0.0;

    if actions.is_action_pressed(&input, "Accelerate") {
        current_engine = engine_power;
    }
    if actions.is_action_pressed(&input, "Reverse") {
        current_engine = -engine_power * 0.4;
    }
    if actions.is_action_pressed(&input, "SteerLeft") {
        current_steer = max_steer;
    }
    if actions.is_action_pressed(&input, "SteerRight") {
        current_steer = -max_steer;
    }
    if actions.is_action_pressed(&input, "Brake") {
        current_brake = 15000.0;
    }

    if let Some(mut query) =
        world.query_mut_ref::<gizmo::physics::vehicle::VehicleController, crate::Player>()
    {
        for (_id, v, _player) in query.iter_mut() {
            v.steering_angle += (current_steer - v.steering_angle) * 20.0 * dt; // Direksiyonun hızı da 15'ten 20'ye artırıldı
            v.engine_force = current_engine;
            v.brake_force = current_brake;
            break; // Yalnızca tek oyuncu olduğu varsayılıyor
        }
    }
}

pub fn character_update_system(world: &mut World, dt: f32) {
    let input_opt = world.get_resource::<gizmo::core::input::Input>();
    let state_opt = world.get_resource::<crate::state::EngineConfig>();

    if input_opt.is_none() || state_opt.is_none() {
        return;
    }

    let input = input_opt.unwrap();
    let state = state_opt.unwrap();

    // Serbest kamera çalışıyorsa karakter hareket etmesin
    if state.free_cam {
        return;
    }

    let mut move_dir = Vec3::ZERO;
    // Arrow keys & WASD for character motion (winit keycodes typically arrow up=73, we check ASCII as well if we want)
    // 17=W, 31=S, 30=A, 32=D (from winit physical keys approximately or we use action bindings)
    let actions_opt = world.get_resource::<gizmo::core::input::ActionMap>();
    if let Some(ref actions) = actions_opt {
        if actions.is_action_pressed(&input, "Accelerate") {
            move_dir.z -= 1.0;
        }
        if actions.is_action_pressed(&input, "Reverse") {
            move_dir.z += 1.0;
        }
        if actions.is_action_pressed(&input, "SteerLeft") {
            move_dir.x -= 1.0;
        }
        if actions.is_action_pressed(&input, "SteerRight") {
            move_dir.x += 1.0;
        }
    }

    let is_moving = move_dir.length_squared() > 0.001;
    if is_moving {
        move_dir = move_dir.normalize();
    }

    // Fetch multiple components avoiding overlapping mut borrows manually where needed
    // The easiest is to use isolated queries or multiple get_mut.
    if let Some(mut chars) = world.borrow_mut::<gizmo::physics::character::CharacterController>() {
        for &e in &chars.dense.iter().map(|e| e.entity).collect::<Vec<_>>() {
            if let Some(cc) = chars.get_mut(e) {
                // Apply input velocity
                cc.desired_velocity = move_dir * 10.0;

                // Allow jump if grounded
                if let Some(ref actions) = actions_opt {
                    if actions.is_action_pressed(&input, "Brake") && cc.is_grounded {
                        cc.jump(8.0);
                    }
                }
            }
        }
    }

    // Now update transforms for rotation and animation players for playing
    if let (Some(mut transforms), Some(mut aniopts), Some(chars)) = (
        world.borrow_mut::<Transform>(),
        world.borrow_mut::<gizmo::renderer::components::AnimationPlayer>(),
        world.borrow::<gizmo::physics::character::CharacterController>(),
    ) {
        for e in chars.dense.iter().map(|e| e.entity) {
            if let Some(cc) = chars.get(e) {
                let actual_speed = cc.desired_velocity.length();
                if let Some(t) = transforms.get_mut(e) {
                    if is_moving {
                        // Kapsül hareketinin karakter için yönü hesapla ve yönünü o yöne döndür.
                        // Model default `-Z` bakar. Biz karakteri hareket yönüne döndürelim.
                        let _look_target = t.position + move_dir;
                        let angle = move_dir.x.atan2(move_dir.z);
                        // CesiumMan by default faces +Z, and atan2(x, z) gives 0 when (0, 1), pi/2 when (1, 0)
                        let target_rot = Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), angle);
                        t.rotation = t.rotation.slerp(target_rot, 15.0 * dt);
                    }
                }

                if let Some(anim) = aniopts.get_mut(e) {
                    anim.active_animation = 0;
                    if actual_speed > 0.1 || !cc.is_grounded {
                        anim.loop_anim = true;
                        // Time should progress. We let render_pipeline progress it, but wait!
                        // render_pipeline only progresses what it iterates.
                        // We could speed it up by modifying current_time here, but render_pipeline multiplies by dt.
                        // Let's just create a playback_speed inside AnimationPlayer or simply block time advancing in render_pipeline if not walking.
                        // Since we don't have playback_speed, we can just reset or pause the anim.
                        // Actually, render_pipeline always increases current_time!
                        // To pause, we can just subtract the dt here so render_pipeline adds it back to zero change.
                        // Better: We add `is_playing: bool` to the component? We can't change it here without refactoring.
                        // For now, if not moving, we reset `current_time` to force IDLE pose (start of walk animation).
                    } else {
                        // Keep IDLE stand by resetting or locking time to 0.0
                        anim.current_time = 0.0;
                    }
                }
            }
        }
    }
}

pub fn spawner_update_system(world: &mut World, state: &crate::state::GameState, _dt: f32) {
    let mut cam_pos = Vec3::ZERO;
    let mut cam_front = Vec3::new(0.0, 0.0, -1.0);
    if let (Some(cameras), Some(transforms)) =
        (world.borrow::<Camera>(), world.borrow::<Transform>())
    {
        if let (Some(cam), Some(cam_t)) = (
            cameras.get(state.player_id),
            transforms.get(state.player_id),
        ) {
            cam_pos = cam_t.position;
            cam_front = cam.get_front();
        }
    }

    // --- DOMINO SPAWNER ---
    let mut spawn_domino_count = 0;
    if let Some(mut events) =
        world.get_resource_mut::<gizmo::core::event::Events<crate::state::SpawnDominoEvent>>()
    {
        for ev in events.drain() {
            spawn_domino_count += ev.count;
        }
    }

    for _ in 0..spawn_domino_count {
        let entity = world.spawn();
        let spawn_pos = cam_pos + cam_front * 5.0;

        world.add_component(entity, Transform::new(spawn_pos));
        world.add_component(entity, Velocity::new(Vec3::ZERO));
        world.add_component(entity, gizmo::physics::shape::Collider::new_sphere(1.0));
        world.add_component(entity, RigidBody::new(1.0, 0.5, 0.2, true));
        world.add_component(entity, EntityName("Yeni Küre".into()));

        let mut mesh_clone = None;
        if let Some(meshes) = world.borrow::<Mesh>() {
            if let Some(m) = meshes.get(state.bouncing_box_id) {
                mesh_clone = Some(m.clone());
            }
        }
        if let Some(mesh) = mesh_clone {
            world.add_component(entity, mesh);
        }

        let r = ((entity.id() * 73 + 17) % 255) as f32 / 255.0;
        let g = ((entity.id() * 137 + 43) % 255) as f32 / 255.0;
        let b = ((entity.id() * 199 + 7) % 255) as f32 / 255.0;

        let mut bind_group_clone = None;
        if let Some(mats) = world.borrow::<gizmo::renderer::components::Material>() {
            if let Some(mat) = mats.get(state.bouncing_box_id) {
                bind_group_clone = Some(mat.bind_group.clone());
            }
        }
        if let Some(bg) = bind_group_clone {
            let new_mat = gizmo::renderer::components::Material::new(bg).with_pbr(
                Vec4::new(r, g, b, 1.0),
                0.4,
                0.1,
            );
            world.add_component(entity, new_mat);
        }

        world.add_component(entity, gizmo::renderer::components::MeshRenderer::new());
        if let Some(mut sel_events) =
            world.get_resource_mut::<gizmo::core::event::Events<crate::state::SelectionEvent>>()
        {
            sel_events.push(crate::state::SelectionEvent {
                entity_id: entity.id(),
            });
        }
    }

    // --- PACHINKO SPAWNER - TEST İÇİN DEVRE DIŞI BIRAKILDI ---
    /*
    let mut should_spawn = false;
    let mut current_count = 0;

    if let Some(mut pachinko_res) = world.get_resource_mut::<crate::state::PachinkoSpawnerState>() {
        // ... (spawner kodları)
    }
    */
}

pub fn chase_camera_system(world: &mut World, dt: f32) {
    let state_opt = world.get_resource::<crate::state::EngineConfig>();
    if state_opt.is_none() {
        return;
    }

    let state = state_opt.unwrap();
    // Eğer serbest kamera modundaysak takip kamerasını iptal et
    if state.free_cam {
        return;
    }

    let active_camera_entity = state.active_camera_entity;

    // 1. Arabanın (Player) Transform'unu bul
    let mut target_transform = None;
    if let Some(mut q1) =
        world.query_mut_ref::<gizmo::physics::vehicle::VehicleController, crate::Player>()
    {
        for (id, _vc, _player) in q1.iter_mut() {
            if let Some(transforms) = world.borrow::<Transform>() {
                if let Some(transform) = transforms.get(id) {
                    target_transform = Some(*transform);
                }
            }
            break;
        }
    }

    // 2. Kamerayı Arabaya göre hizala ve bakış açısını kilitle
    if let Some(tt) = target_transform {
        if let Some(mut q2) = world.query_mut_mut::<Camera, Transform>() {
            for (id, cam, trans) in q2.iter_mut() {
                if id == active_camera_entity {
                    // Arabanın yönüne göre arka konumu (Araçta +Z ön tarafa bakar, bu yüzden fwd +Z)
                    let fwd = tt.rotation * Vec3::new(0.0, 0.0, 1.0);
                    // Arabanın arkasında ve biraz yukarısında olması gereken ideal pozisyon
                    let desired_pos = tt.position - fwd * 9.0 + Vec3::new(0.0, 3.5, 0.0);

                    // Pozisyonu oldukça sıkı takip et (Lerp) - Titremeyi azaltmak için hız artırıldı
                    trans.position += (desired_pos - trans.position) * 15.0 * dt;

                    // Kameranın tam olarak arabanın biraz üstüne bakmasını sağla
                    let look_target = tt.position + Vec3::new(0.0, 1.0, 0.0);
                    let dir = (look_target - trans.position).normalize();

                    // dir vektörüne göre Yaw ve Pitch açılarını hesapla
                    cam.pitch = dir.y.asin();
                    cam.yaw = dir.z.atan2(dir.x);
                }
            }
        }
    }
}

pub fn ccd_test_system(world: &mut World, _dt: f32) {
    let mut shoot_req = None; // None, Some(false) for NoCCD, Some(true) for CCD
    let mut cam_pos = Vec3::ZERO;
    let mut cam_front = Vec3::new(0.0, 0.0, 1.0); // +Z

    {
        let input_opt = world.get_resource::<Input>();
        let actions_opt = world.get_resource::<gizmo::core::input::ActionMap>();
        let state_opt = world.get_resource::<crate::state::EngineConfig>();

        if input_opt.is_none() || actions_opt.is_none() || state_opt.is_none() {
            return;
        }

        let input = input_opt.unwrap();
        let actions = actions_opt.unwrap();
        let state = state_opt.unwrap();

        if actions.is_action_pressed(&input, "ShootNoCCD") {
            shoot_req = Some(false);
        }

        // RELOAD MANTIĞI
        if actions.is_action_pressed(&input, "Reload") {
            if let Some(mut stats) = world.get_resource_mut::<crate::state::PlayerStats>() {
                stats.ammo = stats.max_ammo;
                println!("Silah Sarjörü Dolduruldu (Reload)! Ammo: {}", stats.ammo);
            }
        }

        if shoot_req.is_some() {
            // MERMİ KONTROLÜ
            let can_shoot =
                if let Some(mut stats) = world.get_resource_mut::<crate::state::PlayerStats>() {
                    if stats.ammo > 0 {
                        stats.ammo -= 1;
                        true
                    } else {
                        println!("Mermi Bitti! R tuşuna basarak doldur.");
                        false
                    }
                } else {
                    true // Stats yoksa sınırsız sık
                };

            if !can_shoot {
                return;
            }

            if let (Some(cameras), Some(transforms)) =
                (world.borrow::<Camera>(), world.borrow::<Transform>())
            {
                if let (Some(cam), Some(cam_t)) = (
                    cameras.get(state.active_camera_entity),
                    transforms.get(state.active_camera_entity),
                ) {
                    cam_pos = cam_t.position;
                    cam_front = cam.get_front();
                } else if let Some(first) = cameras.dense.first().map(|e| &e.entity) {
                    if let (Some(cam), Some(cam_t)) = (cameras.get(*first), transforms.get(*first))
                    {
                        cam_pos = cam_t.position;
                        cam_front = cam.get_front();
                    }
                }
            }
        }
    }

    if let Some(use_ccd) = shoot_req {
        let entity = world.spawn();
        // Karakterin biraz önünden fırlat
        let spawn_pos = cam_pos + cam_front * 2.0;

        world.add_component(
            entity,
            Transform::new(spawn_pos).with_scale(Vec3::new(0.2, 0.2, 0.2)),
        );
        // SES HIZINDAN BİLE HIZLI: 500 m/s
        world.add_component(entity, Velocity::new(cam_front * 500.0));
        world.add_component(entity, gizmo::physics::shape::Collider::new_sphere(0.2));

        let mut rb = RigidBody::new(0.5, 0.9, 0.1, false); // Yerçekimi kapalı (Bullet physics)
        rb.ccd_enabled = use_ccd;
        world.add_component(entity, rb);
        world.add_component(
            entity,
            gizmo::prelude::EntityName(if use_ccd {
                "CCD Bullet".into()
            } else {
                "Ghost Bullet".into()
            }),
        );

        // Extract mesh and material first to drop the World immutable borrow
        let prefab_data = if let Some(prefab) = world.get_resource::<crate::state::BulletPrefab>() {
            Some((prefab.mesh.clone(), prefab.material.clone()))
        } else {
            None
        };

        if let Some((mesh, material)) = prefab_data {
            world.add_component(entity, mesh);
            world.add_component(entity, material);
            world.add_component(entity, gizmo::renderer::components::MeshRenderer::new());
        }
    }
}

pub fn gizmo_city_dash_system(world: &mut World, state: &mut GameState, dt: f32) {
    let mut player_pos = None;

    if let Some(transforms) = world.borrow::<Transform>() {
        // The car is the only VehicleController normally, or we just use player_id!
        // Wait, `basic_scene.rs` set `state.player_entity`? We don't have `state.player_entity`, but we have `GameState::player_id` because basic scene returns it but it maps to `state.player_id`.
        if let Some(t) = transforms.get(state.player_id) {
            player_pos = Some(t.position);
        }
    }

    let mut coins_to_despawn = Vec::new();
    let ppos = player_pos.unwrap_or(Vec3::ZERO);

    if let (Some(mut transforms), Some(coins)) = (
        world.borrow_mut::<Transform>(),
        world.borrow::<crate::state::Coin>(),
    ) {
        for entity in coins.dense.iter().map(|e| &e.entity) {
            if let Some(t) = transforms.get_mut(*entity) {
                t.rotation = t.rotation * Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), dt * 3.0);

                if player_pos.is_some() {
                    let dist = (t.position - ppos).length();
                    if dist < 4.0 {
                        coins_to_despawn.push(*entity);
                    }
                }
            }
        }
    }

    for id in coins_to_despawn {
        if let Some(ent) = world.get_entity(id) {
            world.despawn(ent);
        }

        state.game_score += 1;
        if state.game_score > state.game_max_score {
            state.game_score = state.game_max_score;
        }

        if let Some(audio) = state.audio.as_mut() {
            let sound = if state.game_score == state.game_max_score {
                "checkpoint" // Ya da music
            } else {
                "bounce"
            };
            audio.play(sound);
        }
    }
}
