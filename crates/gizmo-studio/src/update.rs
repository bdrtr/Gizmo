use crate::state::{DebugAssets, StudioState};
use crate::studio_input;
use gizmo::editor::{BuildTarget, EditorState};
use gizmo::physics::components::Transform;
use gizmo::prelude::*;

pub fn update_studio(world: &mut World, state: &mut StudioState, dt: f32, input: &Input) {
    state.current_fps = 1.0 / dt;

    let mut look_delta = None;
    let mut pan_delta = None;
    let mut orbit_delta = None;
    let mut scroll_delta = None;
    if let Some(mut editor_state) = world.remove_resource::<EditorState>() {
        look_delta = editor_state.camera_look_delta;
        pan_delta = editor_state.camera_pan_delta;
        orbit_delta = editor_state.camera_orbit_delta;
        scroll_delta = editor_state.camera_scroll_delta;
        // Editör Scene View üzerinden gelen NDC ve raycast tetiğini okuyalım
        if let Some(ndc) = editor_state.mouse_ndc {
            let (ww, wh) = input.window_size();
            let aspect = if let Some(rect) = editor_state.scene_view_rect {
                rect.width() / rect.height()
            } else {
                ww / wh
            };

            if let (Some(transforms), Some(cameras)) = (
                world.borrow::<Transform>(),
                world.borrow::<gizmo::renderer::components::Camera>(),
            ) {
                if let (Some(t), Some(cam)) = (
                    transforms.get(state.editor_camera),
                    cameras.get(state.editor_camera),
                ) {
                    editor_state.camera_view = Some(cam.get_view(t.position));
                    editor_state.camera_proj = Some(cam.get_projection(aspect));
                }
            }

            let current_ray =
                studio_input::build_ray(world, state.editor_camera, ndc.x, ndc.y, aspect, 1.0);
            if let Some(ray) = current_ray {
                let do_rc = editor_state.do_raycast;
                if do_rc {
                    editor_state.do_raycast = false;
                    state.do_raycast = false;
                }
                
                let ctrl_pressed = input.is_key_pressed(gizmo::winit::keyboard::KeyCode::ControlLeft as u32) 
                                || input.is_key_pressed(gizmo::winit::keyboard::KeyCode::ControlRight as u32);
                                
                studio_input::handle_studio_input(
                    world,
                    &mut editor_state,
                    ray,
                    state.editor_camera,
                    do_rc,
                    ctrl_pressed,
                );
            }
        }

        studio_input::sync_gizmos(world, &editor_state);

        // GIZMO DEBUG RENDERER: Spawn and Despawn logic
        // Zamanlayıcısı dolanları sil
        let mut surviving_entities = Vec::new();
        for (timer, ent) in editor_state.debug_spawned_entities.drain(..) {
            if timer - dt > 0.0 {
                surviving_entities.push((timer - dt, ent));
            } else {
                world.despawn_by_id(ent);
            }
        }
        editor_state.debug_spawned_entities = surviving_entities;

        // Yeni debug istekleri spawnla
        if !editor_state.debug_draw_requests.is_empty() {
            let mut pending_debug_assets = None;
            if let Some(debug_assets) = world.get_resource::<DebugAssets>() {
                pending_debug_assets =
                    Some((debug_assets.cube.clone(), debug_assets.white_tex.clone()));
            }

            if let Some((cube, white_tex)) = pending_debug_assets {
                let reqs = std::mem::take(&mut editor_state.debug_draw_requests);
                for (pos, rot, scale, color) in reqs {
                    let e = world.spawn();
                    world
                        .add_component(e, Transform::new(pos).with_rotation(rot).with_scale(scale));
                    world.add_component(e, cube.clone());
                    let mut mat =
                        gizmo::prelude::Material::new(white_tex.clone()).with_unlit(color);
                    if color.w < 0.99 {
                        mat = mat.with_transparent(true);
                    }
                    world.add_component(e, mat);
                    world.add_component(e, gizmo::renderer::components::MeshRenderer::new());
                    editor_state.debug_spawned_entities.push((2.0, e.id())); // 2 saniye kalsın
                }
            } else {
                editor_state.debug_draw_requests.clear();
            }
        }

        // Asset browser sürükle bırak spawn işlemi
        if let Some(asset_path) = editor_state.spawn_asset_request.take() {
            let mut final_pos = None;
            if let Some(ndc) = editor_state.spawn_asset_position {
                let (ww, wh) = input.window_size();
                let aspect = if let Some(rect) = editor_state.scene_view_rect {
                    rect.width() / rect.height()
                } else {
                    ww / wh
                };

                if let Some(ray) =
                    studio_input::build_ray(world, state.editor_camera, ndc.x, ndc.y, aspect, 1.0)
                {
                    // Raycast yap (Gizmo'ları yoksayarak)
                    let mut closest_t = std::f32::MAX;
                    if let (Some(colliders), Some(transforms)) =
                        (world.borrow::<Collider>(), world.borrow::<Transform>())
                    {
                        for i in 0..colliders.dense.len() {
                            let id = colliders.dense[i].entity;
                            if id == state.editor_camera || id == editor_state.highlight_box {
                                continue;
                            }

                            if let Some(t) = (*transforms).get(id) {
                                let extents = colliders.dense[i]
                                    .data
                                    .shape
                                    .bounding_box_half_extents(t.rotation);
                                let scaled_half = gizmo::math::Vec3::new(
                                    extents.x * t.scale.x,
                                    extents.y * t.scale.y,
                                    extents.z * t.scale.z,
                                );

                                if let Some(hitt) =
                                    ray.intersect_obb(t.position, scaled_half, t.rotation)
                                {
                                    if hitt > 0.0 && hitt < closest_t {
                                        closest_t = hitt;
                                    }
                                }
                            }
                        }
                    }

                    if closest_t < std::f32::MAX {
                        final_pos = Some(ray.origin + ray.direction * closest_t);
                    } else {
                        // Basit bir Z=0 / Y=0 zemin kesişimi yapalım
                        if ray.direction.y < -0.0001 {
                            let t = -ray.origin.y / ray.direction.y;
                            final_pos = Some(ray.origin + ray.direction * t);
                        } else {
                            // Işık yukarı bakıyorsa 15 birim öteye atalım
                            final_pos = Some(ray.origin + ray.direction * 15.0);
                        }
                    }
                }
            }

            if asset_path.ends_with(".prefab") {
                editor_state.prefab_load_request = Some((asset_path, None, final_pos));
            } else if asset_path.ends_with(".gizmo") {
                editor_state.scene_load_request = Some(asset_path);
            } else {
                editor_state.log_error(&format!(
                    "Sadece prefab dosyaları sahnede spawn edilebilir: {}",
                    asset_path
                ));
            }
        }

        // --- BUILD SİSTEMİ (STANDALONE EXPORTER) ---
        if editor_state.build_request {
            editor_state.build_request = false;
            editor_state
                .is_building
                .store(true, std::sync::atomic::Ordering::SeqCst);
            editor_state.build_logs.lock().unwrap().clear();

            let is_building_flag = editor_state.is_building.clone();
            let logs_queue = editor_state.build_logs.clone();
            let build_target = editor_state.build_target;

            std::thread::spawn(move || {
                let log = |msg: &str| {
                    if let Ok(mut l) = logs_queue.lock() {
                        l.push(msg.to_string());
                    }
                };

                // Hedefe göre cargo args belirle
                let (target_triple, exe_name, target_label) = match build_target {
                    BuildTarget::Native => (
                        None,
                        if cfg!(windows) { "demo.exe" } else { "demo" },
                        "Native",
                    ),
                    BuildTarget::Linux => (Some("x86_64-unknown-linux-gnu"), "demo", "Linux (ELF)"),
                    BuildTarget::Windows => {
                        (Some("x86_64-pc-windows-gnu"), "demo.exe", "Windows (.exe)")
                    }
                    BuildTarget::MacOs => (Some("x86_64-apple-darwin"), "demo", "macOS"),
                };

                log(&format!(
                    "== [Adım 1/3] Gizmo Build Başlıyor — Hedef: {} ==",
                    target_label
                ));

                let mut args = vec!["build", "--release", "-p", "demo"];
                let target_str;
                if let Some(triple) = target_triple {
                    target_str = format!("--target={}", triple);
                    args.push(&target_str);
                    log(&format!("> cargo {}", args.join(" ")));
                } else {
                    log("> cargo build --release -p demo");
                }

                let mut command = std::process::Command::new("cargo");
                command
                    .args(&args)
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped());

                match command.spawn() {
                    Ok(mut child) => {
                        let stderr = child.stderr.take().unwrap();
                        let logs_queue_clone = logs_queue.clone();
                        let stderr_thread = std::thread::spawn(move || {
                            use std::io::{BufRead, BufReader};
                            let reader = BufReader::new(stderr);
                            for line in reader.lines() {
                                if let Ok(l) = line {
                                    if let Ok(mut l_lock) = logs_queue_clone.lock() {
                                        l_lock.push(l);
                                    }
                                }
                            }
                        });

                        let status = child.wait().unwrap();
                        let _ = stderr_thread.join();

                        if status.success() {
                            log("\n== [Adım 2/3] Derleme Başarılı! Dosyalar Kopyalanıyor ==");
                            let export_dir = std::path::Path::new("export/gizmo_game");
                            let _ = std::fs::remove_dir_all(export_dir);
                            let _ = std::fs::create_dir_all(export_dir);

                            // Hedef triple varsa output target/TRIPLE/release/ altında olur
                            let src_base = if let Some(triple) = target_triple {
                                std::path::PathBuf::from("target")
                                    .join(triple)
                                    .join("release")
                            } else {
                                std::path::PathBuf::from("target/release")
                            };
                            let src_exe = src_base.join(exe_name);
                            let dst_exe = export_dir.join(exe_name);

                            if let Err(e) = std::fs::copy(&src_exe, &dst_exe) {
                                log(&format!(
                                    "HATA: Executable kopyalanamadı ({:?}): {}",
                                    src_exe, e
                                ));
                            } else {
                                log(&format!("Kopyalandı -> {:?}", dst_exe));
                                #[cfg(unix)]
                                {
                                    use std::os::unix::fs::PermissionsExt;
                                    if let Ok(metadata) = std::fs::metadata(&dst_exe) {
                                        let mut perms = metadata.permissions();
                                        perms.set_mode(0o755);
                                        let _ = std::fs::set_permissions(&dst_exe, perms);
                                    }
                                }
                            }

                            log("\n== [Adım 3/3] Assetler Taşınıyor ==");
                            fn copy_dir_all(
                                src: impl AsRef<std::path::Path>,
                                dst: impl AsRef<std::path::Path>,
                                log: &dyn Fn(&str),
                            ) -> std::io::Result<()> {
                                std::fs::create_dir_all(&dst)?;
                                for entry in std::fs::read_dir(src)? {
                                    let entry = entry?;
                                    let ty = entry.file_type()?;
                                    if ty.is_dir() {
                                        copy_dir_all(
                                            entry.path(),
                                            dst.as_ref().join(entry.file_name()),
                                            log,
                                        )?;
                                    } else {
                                        std::fs::copy(
                                            entry.path(),
                                            dst.as_ref().join(entry.file_name()),
                                        )?;
                                    }
                                }
                                Ok(())
                            }

                            let _ = copy_dir_all("demo/assets", export_dir.join("assets"), &log);
                            log("Kopyalandı -> assets/");
                            let _ = copy_dir_all("demo/scenes", export_dir.join("scenes"), &log);
                            log("Kopyalandı -> scenes/");
                            let _ = copy_dir_all("demo/scripts", export_dir.join("scripts"), &log);
                            log("Kopyalandı -> scripts/");
                            let _ = copy_dir_all("media", export_dir.join("media"), &log);
                            log("Kopyalandı -> media/");

                            log("\n🎉 BUILD TAMAMLANDI! 🎉");
                            log("Oyununuz 'export/gizmo_game/' dizininde hazır.");
                        } else {
                            log("\n❌ HATA: Cargo derlemesi başarısız oldu.");
                        }
                    }
                    Err(e) => {
                        log(&format!("HATA: Cargo işlemi başlatılamadı: {}", e));
                    }
                }

                is_building_flag.store(false, std::sync::atomic::Ordering::SeqCst);
            });
        }

        // --- EDITOR KISAYOLLARI (SHORTCUTS) ---
        let ctrl_pressed = input.is_key_pressed(gizmo::winit::keyboard::KeyCode::ControlLeft as u32)
            || input.is_key_pressed(gizmo::winit::keyboard::KeyCode::ControlRight as u32);
        let shift_pressed = input.is_key_pressed(gizmo::winit::keyboard::KeyCode::ShiftLeft as u32)
            || input.is_key_pressed(gizmo::winit::keyboard::KeyCode::ShiftRight as u32);

        // Kısayol: Undo / Redo
        if ctrl_pressed {
            if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyZ as u32) {
                if shift_pressed {
                    editor_state.history.redo(world);
                } else {
                    editor_state.history.undo(world);
                }
            } else if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyY as u32) {
                editor_state.history.redo(world);
            }

            // Kısayol: Ctrl + D (Çoğalt)
            if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyD as u32) {
                for &entity in editor_state.selected_entities.iter() {
                    editor_state.duplicate_requests.push(entity);
                }
            }
        }

        // Kısayol: Delete (Sil) (Ctrl durumundan bağımsız tetiklenmeli)
        if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::Delete as u32) {
            for &entity in editor_state.selected_entities.iter() {
                editor_state.despawn_requests.push(entity);
            }
            editor_state.clear_selection();
        }

        // Kısayol: F (Seçili Objeye Odaklan) (Yine Ctrl'den bağımsız tetiklenmeli)
        if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyF as u32) {
            if !editor_state.selected_entities.is_empty() {
                    if let Some(transforms) = world.borrow::<Transform>() {
                        let mut center_pos = gizmo::math::Vec3::ZERO;
                        let mut count = 0.0;
                        for &target_id in editor_state.selected_entities.iter() {
                            if let Some(target) = transforms.get(target_id) {
                                center_pos += target.position;
                                count += 1.0;
                            }
                        }

                        if count > 0.0 {
                            let target_pos = center_pos / count;
                            drop(transforms); // Ödünç almayı bırak

                            if let (Some(mut t_mut), Some(mut cam_mut)) = (
                                world.borrow_mut::<Transform>(),
                                world.borrow_mut::<gizmo::renderer::components::Camera>(),
                            ) {
                                if let (Some(cam_t), Some(cam)) = (
                                    t_mut.get_mut(state.editor_camera),
                                    cam_mut.get_mut(state.editor_camera),
                                ) {
                                    let offset = cam.get_front() * -10.0;
                                    cam_t.position = target_pos + offset;
                                    cam_t.update_local_matrix();
                                }
                            }
                        }
                    }
                }
            }
        }

        // --- HOT RELOAD POLLING SİSTEMİ ---
        if let Some(watcher) = &mut state.asset_watcher {
            let changes = watcher.poll_changes();
            if !changes.is_empty() {
                for changed_path in changes {
                    let path_str = changed_path.to_string_lossy().to_string();
                    let is_script = path_str.ends_with(".lua");

                    if is_script {
                        editor_state.log_info(&format!("🔥 Script Hot-Reload: {}", path_str));
                        if let Some(mut engine) =
                            world.get_resource_mut::<gizmo::scripting::ScriptEngine>()
                        {
                            if let Err(e) = engine.load_script(&path_str) {
                                editor_state.log_error(&format!("❌ Script yüklenemedi: {}", e));
                            }
                        }
                    } else if path_str.ends_with(".wgsl") {
                        editor_state.log_warning(&format!(
                            "🔥 Shader Hot-Reload iskitleniyor: {}",
                            path_str
                        ));
                        // WGPU Shader hot reload events can be integrated here similarly
                        let has_events = world.get_resource::<gizmo::core::event::Events<crate::state::ShaderReloadEvent>>().is_some();
                        if !has_events {
                            world.insert_resource(gizmo::core::event::Events::<
                                crate::state::ShaderReloadEvent,
                            >::new());
                        }
                        if let Some(mut events) = world.get_resource_mut::<gizmo::core::event::Events<crate::state::ShaderReloadEvent>>() {
                            events.push(crate::state::ShaderReloadEvent);
                        }
                    }
                }
            }
        }

        // --- OYUN / SİMÜLASYON DÖNGÜSÜ ---
        if editor_state.is_playing() {
            // SCRIPT ENGINE UPDATE: Sadece "Play" modundayken oyun mantığını işlet
            if let Some(mut engine) = world.remove_resource::<gizmo::scripting::ScriptEngine>() {
                if let Err(e) = engine.update(world, input, dt) {
                    editor_state.log_error(&format!("Script Error: {}", e));
                }

                // Flush commands directly
                let unhandled_commands = engine.flush_commands(world, dt);
                for _cmd in unhandled_commands {
                    // For now, audio/scene commands can be skipped or warned inside the editor
                    // as the editor shouldn't suddenly switch scenes due to a script.
                }

                // Call per-entity updates
                if let Some(scripts) = world.borrow::<gizmo::scripting::Script>() {
                    let mut entity_calls = Vec::new();
                    for entity_id in scripts.dense.iter().map(|e| e.entity) {
                        if let Some(script) = scripts.get(entity_id) {
                            entity_calls.push((entity_id, script.file_path.clone()));
                        }
                    }
                    drop(scripts);

                    for (entity_id, path) in entity_calls {
                        let _ = engine.reload_if_changed(&path);
                        if let Err(e) = engine.update_entity(entity_id, &path, dt) {
                            editor_state.log_warning(&format!("Entity script error: {}", e));
                        }
                    }
                }

                world.insert_resource(engine);
            }

            state.physics_accumulator += dt;
            let fixed_dt = 1.0 / 60.0;
            // Death spiral önleme
            state.physics_accumulator = state.physics_accumulator.min(fixed_dt * 16.0);

            let mut steps = 0;
            while state.physics_accumulator >= fixed_dt && steps < 16 {
                gizmo::physics::integration::physics_apply_forces_system(world, fixed_dt);
                gizmo::physics::vehicle::physics_vehicle_system(world, fixed_dt);
                gizmo::physics::system::physics_collision_system(world, fixed_dt);
                gizmo::physics::character::physics_character_system(world, fixed_dt);
                gizmo::physics::integration::physics_movement_system(world, fixed_dt);

                state.physics_accumulator -= fixed_dt;
                steps += 1;
            }
        } else {
            state.physics_accumulator = 0.0;
        }
        // --- DİNAMİK COMPONENT EKLEME İŞLEMİ ---
        if let Some((ent_id, comp_name)) = editor_state.add_component_request.take() {
            if let Some(ent) = world.get_entity(ent_id) {
                match comp_name.as_str() {
                    "Transform" => world.add_component(ent, Transform::new(Vec3::ZERO)),
                    "Velocity" => {
                        world.add_component(ent, gizmo::physics::Velocity::new(Vec3::ZERO))
                    }
                    "RigidBody" => world
                        .add_component(ent, gizmo::physics::RigidBody::new(1.0, 0.5, 0.5, true)),
                    "Collider" => {
                        world.add_component(ent, gizmo::physics::Collider::new_aabb(1.0, 1.0, 1.0))
                    }
                    "Camera" => world.add_component(
                        ent,
                        gizmo::renderer::components::Camera::new(
                            60.0_f32.to_radians(),
                            0.1,
                            1000.0,
                            0.0,
                            0.0,
                            false,
                        ),
                    ),
                    "PointLight" => world.add_component(
                        ent,
                        gizmo::renderer::components::PointLight::new(Vec3::new(1., 1., 1.), 1.0),
                    ),
                    "Material" => {
                        let white_tex = world
                            .get_resource::<DebugAssets>()
                            .map(|a| a.white_tex.clone());
                        if let Some(tex) = white_tex {
                            world.add_component(ent, gizmo::prelude::Material::new(tex));
                        }
                    }
                    "Script" => world.add_component(
                        ent,
                        gizmo::scripting::Script::new("scripts/new_script.lua"),
                    ),
                    "ParticleEmitter" => world
                        .add_component(ent, gizmo::renderer::components::ParticleEmitter::new()),
                    "AudioSource" => world.add_component(
                        ent,
                        gizmo_audio::AudioSource {
                            sound_name: "".to_string(),
                            is_3d: true,
                            max_distance: 100.0,
                            volume: 1.0,
                            pitch: 1.0,
                            loop_sound: false,
                            _internal_sink_id: None,
                        },
                    ),
                    "Terrain" => {
                        world.add_component(
                            ent,
                            gizmo::renderer::components::Terrain {
                                heightmap_path: "demo/assets/textures/heightmap.png".to_string(),
                                width: 100.0,
                                depth: 100.0,
                                max_height: 20.0,
                            },
                        );
                        // Request rendering mesh creation
                        editor_state.generate_terrain_requests.push(ent_id);
                    }
                    _ => editor_state.log_warning(&format!("Bilinmeyen component: {}", comp_name)),
                }
            }
        }

        if !editor_state.despawn_requests.is_empty() {
            let mut history_backup = Vec::new();
            let despawn_reqs: Vec<u32> = editor_state.despawn_requests.drain(..).collect();
            for ent_id in despawn_reqs {
                editor_state.selected_entities.remove(&ent_id);
                if let Some(ent) = world.get_entity(ent_id) {
                    let backup = gizmo::scene::SceneData::serialize_entities(world, vec![ent_id]);
                    if let Some(data) = backup.into_iter().next() {
                        if let Ok(bytes) = bincode::serialize(&data) {
                            history_backup.push(bytes);
                        }
                    }
                    world.despawn(ent);
                    editor_state.log_info(&format!("Entity {} silindi.", ent_id));
                }
            }
            if !history_backup.is_empty() {
                editor_state
                    .history
                    .push(gizmo::editor::history::EditorAction::EntityDespawned {
                        data: history_backup,
                    });
            }
        }

        // --- YENİ ENTITY OLUŞTURMA (Küp / Küre / Boş) ---
        if let Some(kind) = editor_state.spawn_request.take() {
            let pending_assets = world
                .get_resource::<DebugAssets>()
                .map(|a| (a.cube.clone(), a.white_tex.clone()));

            if let Some((cube_mesh, white_tex)) = pending_assets {
                let e = world.spawn();
                world.add_component(e, Transform::new(Vec3::ZERO));
                world.add_component(e, gizmo::renderer::components::MeshRenderer::new());

                match kind.as_str() {
                    "Cube" => {
                        world.add_component(
                            e,
                            gizmo::core::component::EntityName("Küp".to_string()),
                        );
                        world.add_component(e, cube_mesh);
                        world.add_component(
                            e,
                            gizmo::prelude::Material::new(white_tex).with_pbr(
                                gizmo::math::Vec4::new(0.8, 0.8, 0.8, 1.0),
                                0.5,
                                0.0,
                            ),
                        );
                        world.add_component(e, gizmo::physics::Collider::new_aabb(1.0, 1.0, 1.0));
                        editor_state.log_info("Yeni küp oluşturuldu.");
                    }
                    "Sphere" => {
                        world.add_component(
                            e,
                            gizmo::core::component::EntityName("Küre".to_string()),
                        );
                        world.add_component(e, cube_mesh);
                        world.add_component(
                            e,
                            gizmo::prelude::Material::new(white_tex).with_pbr(
                                gizmo::math::Vec4::new(0.4, 0.6, 1.0, 1.0),
                                0.2,
                                0.0,
                            ),
                        );
                        world.add_component(e, gizmo::physics::Collider::new_sphere(1.0));
                        editor_state.log_info("Yeni küre oluşturuldu.");
                    }
                    _ => {
                        world.add_component(
                            e,
                            gizmo::core::component::EntityName("Boş Entity".to_string()),
                        );
                        editor_state.log_info("Boş entity oluşturuldu.");
                    }
                }

                editor_state.select_exclusive(e.id());
                editor_state
                    .history
                    .push(gizmo::editor::history::EditorAction::EntitySpawned {
                        entity_ids: vec![e.id()],
                    });
            }
        }

        // --- GÖRÜNÜRLÜK AÇMA / KAPATMA ---
        if let Some(ent_id) = editor_state.toggle_visibility_request.take() {
            if let Some(ent) = world.get_entity(ent_id) {
                let currently_hidden = world
                    .borrow::<gizmo::core::component::IsHidden>()
                    .map_or(false, |h| h.contains(ent_id));
                if currently_hidden {
                    world.remove_component::<gizmo::core::component::IsHidden>(ent);
                    editor_state.log_info(&format!("Entity {} görünür yapıldı.", ent_id));
                } else {
                    world.add_component(ent, gizmo::core::component::IsHidden);
                    editor_state.log_info(&format!("Entity {} gizlendi.", ent_id));
                }
            }
        }

        // --- PARENT DEĞİŞTİRME (Reparent) ---
        if let Some((child_id, new_parent_id)) = editor_state.reparent_request.take() {
            // Eski parent'ın children listesinden çıkar
            if let Some(mut children_comp) = world.borrow_mut::<gizmo::core::component::Children>()
            {
                let alive_ids: Vec<u32> = world.iter_alive_entities().map(|e| e.id()).collect();
                for id in alive_ids {
                    if let Some(ch) = children_comp.get_mut(id) {
                        ch.0.retain(|&cid| cid != child_id);
                    }
                }
                // Yeni parent'a ekle
                if let Some(ch) = children_comp.get_mut(new_parent_id) {
                    if !ch.0.contains(&child_id) {
                        ch.0.push(child_id);
                    }
                }
            }
            if let Some(child_ent) = world.get_entity(child_id) {
                world.add_component(child_ent, gizmo::core::component::Parent(new_parent_id));
                editor_state.log_info(&format!(
                    "Entity {} parent {} olarak ayarlandı.",
                    child_id, new_parent_id
                ));
            }
        }

        // --- PARENT KALDIR (Root Yap) ---
        if let Some(child_id) = editor_state.unparent_request.take() {
            if let Some(mut children_comp) = world.borrow_mut::<gizmo::core::component::Children>()
            {
                let alive_ids: Vec<u32> = world.iter_alive_entities().map(|e| e.id()).collect();
                for id in alive_ids {
                    if let Some(ch) = children_comp.get_mut(id) {
                        ch.0.retain(|&cid| cid != child_id);
                    }
                }
            }
            if let Some(child_ent) = world.get_entity(child_id) {
                world.remove_component::<gizmo::core::component::Parent>(child_ent);
                editor_state.log_info(&format!("Entity {} kök (root) yapıldı.", child_id));
            }
        }

        world.insert_resource(editor_state);
    }

    // Editör kamera hızını world'dan oku
    let camera_speed = world
        .get_resource::<EditorState>()
        .map(|es| es.camera_speed)
        .unwrap_or(8.0);

    // Editor Camera WASD Controller
    if let (Some(mut transforms), Some(mut cameras)) = (
        world.borrow_mut::<Transform>(),
        world.borrow_mut::<gizmo::renderer::components::Camera>(),
    ) {
        if let (Some(t), Some(cam)) = (
            transforms.get_mut(state.editor_camera),
            cameras.get_mut(state.editor_camera),
        ) {
            // 1. Mouse Look (Egui üzerinden gelen delta okuması)
            if let Some(delta) = look_delta {
                let sensitivity = 0.003;

                cam.yaw += delta.x * sensitivity;
                cam.pitch -= delta.y * sensitivity;

                // Gimbal Lock'u (tepetaklak olmayı) önle
                let max_pitch = 89.0_f32.to_radians();
                if cam.pitch > max_pitch {
                    cam.pitch = max_pitch;
                }
                if cam.pitch < -max_pitch {
                    cam.pitch = -max_pitch;
                }

                // Transform rotasyonunu kameraya uydur (motor içi tutarlılık için)
                let q_yaw = gizmo::math::Quat::from_axis_angle(
                    gizmo::math::Vec3::new(0.0, 1.0, 0.0),
                    cam.yaw,
                );
                let q_pitch = gizmo::math::Quat::from_axis_angle(
                    gizmo::math::Vec3::new(1.0, 0.0, 0.0),
                    cam.pitch,
                );
                t.rotation = q_yaw * q_pitch;
            }

            // 2. Serbest Uçuş (WASD + Q/E)
            let speed = if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::ShiftLeft as u32) {
                camera_speed * 2.5
            } else {
                camera_speed
            };

            let forward = cam.get_front();
            let right = forward
                .cross(gizmo::math::Vec3::new(0.0, 1.0, 0.0))
                .normalize();
            let up = gizmo::math::Vec3::new(0.0, 1.0, 0.0);

            let mut move_dir = gizmo::math::Vec3::ZERO;
            // Kamera nereye bakıyorsa ORAYA ileri git
            if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyW as u32) {
                move_dir += forward;
            }
            if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyS as u32) {
                move_dir -= forward;
            }
            if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyA as u32) {
                move_dir -= right;
            }
            if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyD as u32) {
                move_dir += right;
            }
            // Dünyaya göre yukarı/aşağı tırmanış
            if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyE as u32) {
                move_dir += up;
            }
            if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyQ as u32) {
                move_dir -= up;
            }

            t.position += move_dir.normalize_or_zero() * (speed * dt);

            // 3. Orta Tık Pan (Kaydırma)
            if let Some(pan) = pan_delta {
                let pan_speed = 0.01;
                t.position += right * (-pan.x * pan_speed);
                t.position += up * (pan.y * pan_speed);
            }

            // 4. Alt + Sol Tık Orbit (Etrafında Dönme)
            if let Some(orbit) = orbit_delta {
                let orbit_speed = 0.005;

                // Pivot noktasını bul (Kameranın 10 birim önü)
                let pivot = t.position + forward * 10.0;

                cam.yaw += orbit.x * orbit_speed;
                cam.pitch -= orbit.y * orbit_speed;

                let max_pitch = 89.0_f32.to_radians();
                if cam.pitch > max_pitch {
                    cam.pitch = max_pitch;
                }
                if cam.pitch < -max_pitch {
                    cam.pitch = -max_pitch;
                }

                let q_yaw = gizmo::math::Quat::from_axis_angle(
                    gizmo::math::Vec3::new(0.0, 1.0, 0.0),
                    cam.yaw,
                );
                let q_pitch = gizmo::math::Quat::from_axis_angle(
                    gizmo::math::Vec3::new(1.0, 0.0, 0.0),
                    cam.pitch,
                );
                t.rotation = q_yaw * q_pitch;

                // Yeni pozisyonu pivota göre konumlandır
                t.position = pivot - (t.rotation * gizmo::math::Vec3::new(0.0, 0.0, 1.0)) * 10.0;
            }

            // 5. Scroll Zoom (İleri / Geri)
            if let Some(scroll) = scroll_delta {
                let zoom_speed = 0.5;
                t.position += forward * (scroll * zoom_speed);
            }
        }
    }
}
