use super::*;

impl<State: 'static> App<State> {
    /// The unified per-event handler — the old `EventLoop::run` closure body,
    /// reconstructed as a method so `ApplicationHandler` can drive it. Each event
    /// type reconstructs a `winit::Event<()>` and dispatches here, so the original
    /// control flow (and the `input_fn(&Event)` hook contract) is preserved.
    pub(super) fn handle_event(&mut self, event: Event<()>, current_window: &ActiveEventLoop) {
        // Reconstruct what the old `move` closure captured. `editor`/`state` are
        // taken out of `self` so the `editor.run(|ctx| … &mut self.world …)`
        // closure can borrow `self`'s other fields disjointly; they (and the
        // frame timers) are restored at every exit — including the two early
        // `return`s in the redraw path.
        let window = match self.window.clone() {
            Some(w) => w,
            None => return,
        };
        #[cfg(feature = "egui")]
        let mut editor = match self.editor.take() {
            Some(e) => e,
            None => return,
        };
        let mut state = match self.app_state.take() {
            Some(s) => s,
            None => return,
        };
        let mut last_frame_time = self
            .last_frame_time
            .unwrap_or_else(super::FrameInstant::now);
        let mut light_time = self.light_time;

        {
                current_window.set_control_flow(ControlFlow::Poll);

                // UI Entegrasyonu: Winit Olaylarını EGUI'ye Gönder
                #[cfg(feature = "egui")]
                let consumes_input = if let Event::WindowEvent {
                    ref event,
                    window_id,
                } = event
                {
                    if window_id == window.id() {
                        editor.handle_event(&window, event)
                    } else {
                        false
                    }
                } else {
                    false
                };
                #[cfg(not(feature = "egui"))]
                let consumes_input = false;

                // Eğer UI girdiyi yakalamadıysa Kullanıcı Input Hook'a Yolla
                if !consumes_input {
                    if let Some(input_hk) = self.input_fn.as_mut() {
                        let _ = input_hk(&mut self.world, &mut state, &event);
                    }
                }

                match event {
                    Event::WindowEvent {
                        ref event,
                        window_id,
                    } if window_id == window.id() => {
                        match event {
                            WindowEvent::CloseRequested => {
                                if let Some(record) = &self.record_data {
                                    let _ = record.save("gizmo_record.ron");
                                    tracing::info!(
                                        "Kayit basariyla 'gizmo_record.ron' dosyasina kaydedildi."
                                    );
                                }
                                // TLS'teki GPU kaynaklarını temizlemekle uğraşmak yerine direkt çık
                                #[cfg(not(target_arch = "wasm32"))]
                                std::process::exit(0);
                                #[cfg(target_arch = "wasm32")]
                                current_window.exit();
                            }
                            WindowEvent::Resized(physical_size) => {
                                {
                                    let mut r = self.world.get_resource_mut::<Renderer>().unwrap();
                                    r.resize(*physical_size);
                                }
                                let mut win_info = self
                                    .world
                                    .get_resource_mut_or_default::<gizmo_core::window::WindowInfo>(
                                    );
                                win_info.width = physical_size.width as f32;
                                win_info.height = physical_size.height as f32;
                            }
                            WindowEvent::KeyboardInput {
                                event: kb_event, ..
                            } => {
                                let mut codes_to_press = Vec::new();
                                // Fiziksel Tuş (PhysicalKey)
                                if let winit::keyboard::PhysicalKey::Code(keycode) =
                                    kb_event.physical_key
                                {
                                    codes_to_press.push(keycode as u32);
                                }
                                // Mantıksal Tuş (LogicalKey Fallback)
                                if codes_to_press.is_empty() {
                                    if let winit::keyboard::Key::Character(c) =
                                        kb_event.logical_key.as_ref()
                                    {
                                        match c.to_lowercase().as_str() {
                                            "w" => codes_to_press
                                                .push(winit::keyboard::KeyCode::KeyW as u32),
                                            "a" => codes_to_press
                                                .push(winit::keyboard::KeyCode::KeyA as u32),
                                            "s" => codes_to_press
                                                .push(winit::keyboard::KeyCode::KeyS as u32),
                                            "d" => codes_to_press
                                                .push(winit::keyboard::KeyCode::KeyD as u32),
                                            _ => {}
                                        }
                                    } else if let winit::keyboard::Key::Named(named) =
                                        kb_event.logical_key
                                    {
                                        match named {
                                            winit::keyboard::NamedKey::ArrowUp => codes_to_press
                                                .push(winit::keyboard::KeyCode::ArrowUp as u32),
                                            winit::keyboard::NamedKey::ArrowDown => codes_to_press
                                                .push(winit::keyboard::KeyCode::ArrowDown as u32),
                                            winit::keyboard::NamedKey::ArrowLeft => codes_to_press
                                                .push(winit::keyboard::KeyCode::ArrowLeft as u32),
                                            winit::keyboard::NamedKey::ArrowRight => codes_to_press
                                                .push(winit::keyboard::KeyCode::ArrowRight as u32),
                                            winit::keyboard::NamedKey::Space => codes_to_press
                                                .push(winit::keyboard::KeyCode::Space as u32),
                                            winit::keyboard::NamedKey::Escape => codes_to_press
                                                .push(winit::keyboard::KeyCode::Escape as u32),
                                            _ => {}
                                        }
                                    }
                                } // Ends the 'if codes_to_press.is_empty()' block

                                for code in codes_to_press {
                                    if kb_event.state == winit::event::ElementState::Pressed {
                                        self.input.on_key_pressed(code);
                                    } else {
                                        self.input.on_key_released(code);
                                    }
                                }
                            }
                            WindowEvent::MouseInput {
                                state: m_state,
                                button,
                                ..
                            } => {
                                let btn_code = match button {
                                    winit::event::MouseButton::Left => {
                                        gizmo_core::input::mouse::LEFT
                                    }
                                    winit::event::MouseButton::Right => {
                                        gizmo_core::input::mouse::RIGHT
                                    }
                                    winit::event::MouseButton::Middle => {
                                        gizmo_core::input::mouse::MIDDLE
                                    }
                                    _ => u32::MAX,
                                };
                                if btn_code != u32::MAX {
                                    if *m_state == winit::event::ElementState::Pressed {
                                        self.input.on_mouse_button_pressed(btn_code);
                                    } else {
                                        self.input.on_mouse_button_released(btn_code);
                                    }
                                }
                            }
                            WindowEvent::CursorMoved { position, .. } => {
                                self.input
                                    .on_mouse_moved(position.x as f32, position.y as f32);
                            }
                            _ => {}
                        }
                        if let WindowEvent::RedrawRequested = event {
                            let now = super::FrameInstant::now();
                            let mut dt = now.duration_since(last_frame_time).as_secs_f32();
                            dt = dt.min(0.05); // Güvenlik çemberi: Frame takılırsa 50ms'den fazla zıplamayacak, yerçekiminden düşme engellenecek.
                            last_frame_time = now;

                            // Playback / Record mantigi
                            if let Some(playback) = &self.playback_data {
                                if self.playback_frame_index < playback.frames.len() {
                                    let frame = &playback.frames[self.playback_frame_index];
                                    dt = frame.dt;
                                    self.input = frame.input.clone();
                                    self.playback_frame_index += 1;
                                } else {
                                    tracing::info!("Playback bitti. Uygulama kapaniyor...");
                                    current_window.exit();
                                }
                            } else if self.record_mode {
                                if let Some(record) = &mut self.record_data {
                                    record.frames.push(gizmo_core::input::FrameRecord {
                                        dt,
                                        input: self.input.clone(),
                                    });
                                }
                            }

                            light_time += dt;

                            // Update — run one egui frame (overlay UI + dev console).
                            #[cfg(feature = "egui")]
                            let full_output = editor.run(&window, |ctx| {
                                if let Some(ui_hk) = self.ui_fn.as_mut() {
                                    ui_hk(&mut self.world, &mut state, ctx);
                                }

                                // Render Global Dev Console on top of everything
                                dev_console::ui_dev_console(&mut self.world, ctx, &self.input);
                            });

                            // Editor viewport render targets + scene save/load
                            // requests (reads/mutates EditorState).
                            #[cfg(feature = "editor")]
                            {
                                crate::editor_runtime::sync_render_targets(
                                    &mut self.world,
                                    &mut editor,
                                );
                                crate::editor_runtime::process_scene_requests(&mut self.world);
                            }

                            // ECS Sistemlerini Çalıştırmadan önce DI için Core Resource'ları Güncelle
                            self.world.insert_resource(self.input.clone());
                            {
                                let has_time = self
                                    .world
                                    .get_resource::<gizmo_core::time::Time>()
                                    .is_some();
                                if has_time {
                                    let mut time = self
                                        .world
                                        .get_resource_mut::<gizmo_core::time::Time>()
                                        .unwrap();
                                    time.update(dt);
                                } else {
                                    let mut time = gizmo_core::time::Time::new();
                                    time.update(dt);
                                    self.world.insert_resource(time);
                                }
                            }

                            // ═══ Fixed Timestep Fizik Döngüsü ═══
                            if let Some(mut profiler) = self.world.get_resource_mut::<gizmo_core::profiler::FrameProfiler>() {
                                profiler.begin_scope("physics");
                            }
                            // PhysicsTime resource'u yoksa oluştur
                            if self
                                .world
                                .get_resource::<gizmo_core::time::PhysicsTime>()
                                .is_none()
                            {
                                self.world
                                    .insert_resource(gizmo_core::time::PhysicsTime::default());
                            }
                            {
                                let mut phys_time = self
                                    .world
                                    .get_resource_mut::<gizmo_core::time::PhysicsTime>()
                                    .unwrap();
                                phys_time.accumulate(dt);
                            }

                            #[cfg(feature = "network")]
                            {
                                let mut rollback_needed = false;
                                let mut rm = self.world.remove_resource::<gizmo_net::rollback::RollbackManager>();
                                
                                if let Some(ref mut manager) = rm {
                                    rollback_needed = manager.begin_frame(&mut self.world);
                                }
                                
                                if rollback_needed {
                                    let target_tick = rm.as_ref().unwrap().latest_tick;
                                    
                                    // Desync'i temizlemek için fiziği World'den zorla kopyalattır
                                    #[cfg(feature = "physics")]
                                    if let Some(mut pw) = self.world.get_resource_mut::<gizmo_physics_rigid::world::PhysicsWorld>() {
                                        pw.clear_bodies();
                                    }
                                    
                                    let fixed_dt = self
                                        .world
                                        .get_resource::<gizmo_core::time::PhysicsTime>()
                                        .map(|pt| pt.fixed_dt())
                                        .unwrap_or(1.0 / 60.0);

                                    loop {
                                        let curr = rm.as_ref().unwrap().current_tick;
                                        if curr >= target_tick {
                                            break;
                                        }

                                        // Simüle et (Geçmişten Günümüze Fast-Forward)
                                        // Dikkat: rm World'de olmadığı için içeride rollback.rs çalışmayabilir.
                                        // Ama physics_step_system RollbackManager'i kullanmıyor!
                                        self.schedule.run(&mut self.world, fixed_dt);

                                        // Yeni geçmiş karesini hafızaya al
                                        if let Some(ref mut manager) = rm {
                                            let snapshot = gizmo_net::rollback::PhysicsStateSnapshot::capture(&self.world, manager.current_tick);
                                            manager.state_buffer.save(snapshot);
                                            manager.current_tick += 1;
                                            // latest_tick güncellenmeyecek çünkü zaten eskiyi simüle ediyoruz
                                        }
                                    }
                                }

                                // Tekrar yerine koy
                                if let Some(manager) = rm {
                                    self.world.insert_resource(manager);
                                }
                            }

                            // Sabit dt'de normal fizik adımları — frame rate'ten bağımsız
                            loop {
                                let should = self
                                    .world
                                    .get_resource::<gizmo_core::time::PhysicsTime>()
                                    .map(|pt| pt.should_step())
                                    .unwrap_or(false);
                                if !should {
                                    break;
                                }

                                let fixed_dt = self
                                    .world
                                    .get_resource::<gizmo_core::time::PhysicsTime>()
                                    .map(|pt| pt.fixed_dt())
                                    .unwrap_or(1.0 / 60.0);

                                // ECS fizik sistemlerini sabit dt ile çalıştır
                                self.schedule.run(&mut self.world, fixed_dt);

                                #[cfg(feature = "network")]
                                {
                                    if let Some(mut rm) = self.world.get_resource_mut::<gizmo_net::rollback::RollbackManager>() {
                                        rm.end_frame(&self.world);
                                    }
                                }

                                let mut phys_time = self
                                    .world
                                    .get_resource_mut::<gizmo_core::time::PhysicsTime>()
                                    .unwrap();
                                phys_time.consume_step();
                            }

                            // İnterpolasyon alpha'sını hesapla (render için)
                            {
                                let mut phys_time = self
                                    .world
                                    .get_resource_mut::<gizmo_core::time::PhysicsTime>()
                                    .unwrap();
                                phys_time.compute_alpha();
                            }

                            if let Some(mut profiler) = self.world.get_resource_mut::<gizmo_core::profiler::FrameProfiler>() {
                                profiler.end_scope("physics");
                            }

                            if let Some(mut profiler) = self.world.get_resource_mut::<gizmo_core::profiler::FrameProfiler>() {
                                profiler.begin_scope("update");
                            }

                            // Kullanıcı update hook'u (render dt ile — kamera, UI, vb.)
                            if let Some(update_hk) = self.update_fn.as_mut() {
                                update_hk(&mut self.world, &mut state, dt, &self.input);
                            }

                            // Update sonrası olası ertelenmiş komutları (CommandQueue) hemen işle
                            self.world.apply_commands();

                            // Asset Loading System (Lazy Load)
                            if let Some(mut asset_manager) = self.world.remove_resource::<gizmo_renderer::asset::AssetManager>() {
                                let r = self.world.remove_resource::<Renderer>().unwrap();
                                gizmo_renderer::asset_loading::run_asset_loading_system(
                                    &mut self.world,
                                    &r.device,
                                    &r.queue,
                                    &r.scene.texture_bind_group_layout,
                                    &mut asset_manager,
                                );
                                self.world.insert_resource(r);
                                self.world.insert_resource(asset_manager);
                            }

                            if let Some(mut profiler) = self.world.get_resource_mut::<gizmo_core::profiler::FrameProfiler>() {
                                profiler.end_scope("update");
                            }

                            // --- DYNAMIC FRACTURE & PARTICLE INTEGRATION ---
                            #[cfg(feature = "physics")]
                            if let Some(physics_world) =
                                self.world
                                    .get_resource::<gizmo_physics_rigid::world::PhysicsWorld>()
                            {
                                if !physics_world.fracture_events.is_empty() {
                                    let renderer = self.world.get_resource::<Renderer>().unwrap();
                                    if let Some(gpu_particles) = &renderer.gpu_particles {
                                        for event in &physics_world.fracture_events {
                                            let center = [
                                                event.impact_point.x,
                                                event.impact_point.y,
                                                event.impact_point.z,
                                            ];
                                            let dust_color = [0.6, 0.55, 0.5, 0.8]; // Dust color
                                            let force =
                                                (event.impact_force * 0.01).clamp(2.0, 15.0);
                                            let particle_count = (event.impact_force * 0.1)
                                                .clamp(50.0, 500.0)
                                                as u32;
                                            gpu_particles.spawn_explosion(
                                                &renderer.queue,
                                                center,
                                                particle_count,
                                                dust_color,
                                                force,
                                            );
                                        }
                                    }
                                }
                            }

                            // Olayları Güncelle (Çift-buffer temizliği)
                            for updater in &mut self.event_updaters {
                                updater(&mut self.world);
                            }

                            // --- DRAW KISMI ---
                            if let Some(mut profiler) = self.world.get_resource_mut::<gizmo_core::profiler::FrameProfiler>() {
                                profiler.begin_scope("render");
                            }
                            let mut renderer = self.world.remove_resource::<Renderer>().unwrap();

                            let surface = renderer
                                .surface
                                .as_ref()
                                .expect("windowed render path requires a surface");
                            let output = match surface.get_current_texture() {
                                wgpu::CurrentSurfaceTexture::Success(texture)
                                | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
                                // Transient, non-error states (window resized/minimized/occluded
                                // or a timed-out acquire) — silently skip this frame.
                                wgpu::CurrentSurfaceTexture::Outdated
                                | wgpu::CurrentSurfaceTexture::Occluded
                                | wgpu::CurrentSurfaceTexture::Timeout => {
                                    self.world.insert_resource(renderer);
                                    if let Some(mut profiler) = self.world.get_resource_mut::<gizmo_core::profiler::FrameProfiler>() {
                                        profiler.end_scope("render");
                                    }
                                    #[cfg(feature = "egui")]
                                    {
                                        self.editor = Some(editor);
                                    }
                                    self.app_state = Some(state);
                                    self.last_frame_time = Some(last_frame_time);
                                    self.light_time = light_time;
                                    return;
                                }
                                other => {
                                    tracing::error!("Surface hatasi: {:?}", other);
                                    self.world.insert_resource(renderer);
                                    if let Some(mut profiler) = self.world.get_resource_mut::<gizmo_core::profiler::FrameProfiler>() {
                                        profiler.end_scope("render");
                                    }
                                    #[cfg(feature = "egui")]
                                    {
                                        self.editor = Some(editor);
                                    }
                                    self.app_state = Some(state);
                                    self.last_frame_time = Some(last_frame_time);
                                    self.light_time = light_time;
                                    return;
                                }
                            };

                            let view = output
                                .texture
                                .create_view(&wgpu::TextureViewDescriptor::default());

                            let mut encoder = renderer.device.create_command_encoder(
                                &wgpu::CommandEncoderDescriptor {
                                    label: Some("Render Encoder"),
                                },
                            );

                            // Kullaniciya CommandEncoder verip cizdiriyoruz!
                            if let Some(render_hk) = self.render_fn.as_mut() {
                                render_hk(
                                    &mut self.world,
                                    &state,
                                    &mut encoder,
                                    &view,
                                    &mut renderer,
                                    light_time,
                                );
                            } else if let Some(s_render) = self.simple_render_fn.as_mut() {
                                let mut ctx = RenderContext::new(
                                    &mut encoder,
                                    &view,
                                    &mut renderer,
                                    light_time,
                                );
                                s_render(&mut self.world, &state, &mut ctx);
                            }

                            #[cfg(feature = "egui")]
                            {
                                editor.render(
                                    &window,
                                    &renderer.device,
                                    &renderer.queue,
                                    &mut encoder,
                                    &view,
                                    full_output,
                                );
                            }

                            renderer.queue.submit(std::iter::once(encoder.finish()));
                            output.present();

                            self.world.insert_resource(renderer);

                            if let Some(mut profiler) = self.world.get_resource_mut::<gizmo_core::profiler::FrameProfiler>() {
                                profiler.end_scope("render");
                            }

                            // İşlemlerin bitiminde frame-özel input girdilerini (fare delta vs.) temizle
                            self.input.begin_frame();

                            if let Some(mut profiler) = self.world.get_resource_mut::<gizmo_core::profiler::FrameProfiler>() {
                                profiler.end_frame();
                            }
                        }
                    }
                    Event::AboutToWait => {
                        window.request_redraw();
                    }
                    Event::DeviceEvent {
                        event: winit::event::DeviceEvent::MouseMotion { delta },
                        ..
                    } => {
                        self.input.on_mouse_delta(delta.0 as f32, delta.1 as f32);
                    }
                    _ => {}
                }
            }

            // Put the runtime back for the next event.
            #[cfg(feature = "egui")]
            {
                self.editor = Some(editor);
            }
            self.app_state = Some(state);
            self.last_frame_time = Some(last_frame_time);
            self.light_time = light_time;
    }
}
