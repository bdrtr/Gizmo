use super::*;

/// Consecutive `get_current_texture()` failures (Outdated/Lost/Validation) since the last good
/// frame — drives rate-limited logging + backoff so a persistently unrecoverable surface (a true
/// device loss the renderer can't yet recreate) degrades to a throttled retry with one clear
/// warning, instead of a 100%-CPU busy-spin and a per-frame log flood.
static SURFACE_FAIL_STREAK: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

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
                        // The hook's `bool` (event consumed) is not acted on by the
                        // loop, but surface it at trace so a debugging session can see
                        // when the user input handler claims an event.
                        if input_hk(&mut self.world, &mut state, &event) {
                            tracing::trace!("[App] user input hook consumed event");
                        }
                    }
                }

                match event {
                    Event::WindowEvent {
                        ref event,
                        window_id,
                    } if window_id == window.id() => {
                        match event {
                            WindowEvent::CloseRequested => {
                                tracing::info!("[App] close requested; shutting down");
                                if let Some(record) = &self.record_data {
                                    match record.save("gizmo_record.ron") {
                                        Ok(()) => tracing::info!(
                                            path = "gizmo_record.ron",
                                            frames = record.frames.len(),
                                            "[App] input kaydı diske yazıldı"
                                        ),
                                        Err(e) => tracing::warn!(
                                            path = "gizmo_record.ron",
                                            error = %e,
                                            "[App] input kaydı yazılamadı"
                                        ),
                                    }
                                }
                                // DÜZGÜN kapanış: event loop'a çıkışı SİNYALLE (init-hata
                                // yollarının kullandığı `event_loop.exit()` sözleşmesi) →
                                // `run_app` döner, App+World+Renderer normal DROP olur → wgpu
                                // device/surface temiz kapanır (buffer flush, kaynak serbest).
                                // Eski `process::exit(0)` hiçbir destructor çalıştırmadan aniden
                                // kesiyordu; graceful exit her iki hedefte de (wasm dahil) aynı yol.
                                current_window.exit();
                            }
                            WindowEvent::Resized(physical_size) => {
                                // resize() web'de boyutu 640x360'a caplayabilir; WindowInfo'yu
                                // ham fiziksel boyuttan DEĞİL, renderer'ın gerçek (caplenmiş)
                                // boyutundan besle ki kamera aspect'i / picking math'i
                                // surface'le tutarlı kalsın.
                                let effective = {
                                    let mut r = self.world.get_resource_mut::<Renderer>().unwrap();
                                    r.resize(*physical_size);
                                    r.size
                                };
                                let mut win_info = self
                                    .world
                                    .get_resource_mut_or_default::<gizmo_core::window::WindowInfo>(
                                    );
                                win_info.width = effective.width as f32;
                                win_info.height = effective.height as f32;
                                tracing::debug!(
                                    physical_width = physical_size.width,
                                    physical_height = physical_size.height,
                                    effective_width = effective.width,
                                    effective_height = effective.height,
                                    "[App] window resized"
                                );
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
                                        // egui bir metin alanına odaklıyken (consumes_input),
                                        // tuş BASIŞINI motor Input'una besleme — yoksa editör
                                        // kısayolları (Delete/Ctrl+*/W-E-R) metin yazarken
                                        // tetiklenir; Delete seçili entity'leri siler (GC sonrası
                                        // kalıcı veri kaybı). BIRAKMA her zaman işlenir: odak
                                        // metin kutusuna geçtikten sonra bırakılan bir tuş aksi
                                        // halde "sonsuza dek basılı" kalırdı.
                                        if !consumes_input {
                                            self.input.on_key_pressed(code);
                                        }
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
                                // Desktop: the raw look delta comes from
                                // DeviceEvent::MouseMotion, so CursorMoved only tracks
                                // absolute position — accumulating delta here too
                                // DOUBLE-counted mouse-look (2× sensitivity). wasm has
                                // no MouseMotion, so there CursorMoved is also the delta
                                // source (`on_mouse_moved` accumulates it).
                                #[cfg(not(target_arch = "wasm32"))]
                                self.input
                                    .set_mouse_position(position.x as f32, position.y as f32);
                                #[cfg(target_arch = "wasm32")]
                                self.input
                                    .on_mouse_moved(position.x as f32, position.y as f32);
                            }
                            WindowEvent::MouseWheel { delta, .. } => {
                                // Scroll was documented public API (Input::mouse_scroll)
                                // but never wired — no MouseWheel arm existed, so it
                                // always read 0.0.
                                let scroll = match delta {
                                    winit::event::MouseScrollDelta::LineDelta(_, y) => *y,
                                    winit::event::MouseScrollDelta::PixelDelta(pos) => pos.y as f32,
                                };
                                self.input.on_mouse_scroll(scroll);
                            }
                            // Odak kaybında (Alt-Tab / tarayıcı sekme değişimi) basılı
                            // tuşları bırak — yoksa OS key-up göndermez ve tuşlar
                            // sonsuza dek "basılı" kalıp kamerayı kaydırır.
                            WindowEvent::Focused(false) => {
                                self.input.release_all();
                            }
                            _ => {}
                        }
                        if let WindowEvent::RedrawRequested = event {
                            // Per-frame root span; the physics/update/render sub-spans
                            // (from the ECS schedule and physics pipeline) nest under it.
                            let _frame_span = tracing::trace_span!("frame").entered();
                            let now = super::FrameInstant::now();
                            let mut dt = now.duration_since(last_frame_time).as_secs_f32();
                            dt = dt.min(0.05); // Güvenlik çemberi: Frame takılırsa 50ms'den fazla zıplamayacak, yerçekiminden düşme engellenecek.
                            last_frame_time = now;
                            tracing::trace!(dt, "[frame] begin");

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
                            // `sim_dt` = Time'ın `time_scale` ile ölçeklenmiş (ve clamp'lenmiş)
                            // dt'si → fiziği/ECS'i besler ki `set_time_scale(0.0)` gerçekten
                            // DURDURSUN, `0.5` ağır çekim yapsın. `time_scale == 1.0` (varsayılan)
                            // iken `min(dt, max_dt) == dt` olduğundan davranış bit-aynı. Kullanıcı
                            // update hook'u ham `dt` alır (kamera/UI duraklamada bile akıcı kalsın).
                            let sim_dt = {
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
                                    time.dt()
                                } else {
                                    let mut time = gizmo_core::time::Time::new();
                                    time.update(dt);
                                    let sim_dt = time.dt();
                                    self.world.insert_resource(time);
                                    sim_dt
                                }
                            };

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
                                phys_time.accumulate(sim_dt);
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

                                    let mut catchup_steps = 0u32;
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
                                        catchup_steps += 1;
                                    }
                                    if catchup_steps > 0 {
                                        tracing::debug!(
                                            catchup_steps,
                                            target_tick,
                                            "[frame] rollback fast-forward re-simulated ticks"
                                        );
                                    }
                                }

                                // Tekrar yerine koy
                                if let Some(manager) = rm {
                                    self.world.insert_resource(manager);
                                }
                            }

                            // Sabit dt'de normal fizik adımları — frame rate'ten bağımsız
                            let mut phys_steps = 0u32;
                            let mut last_fixed_dt = 0.0f32;
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
                                phys_steps += 1;
                                last_fixed_dt = fixed_dt;
                            }
                            // Accumulator sonucu: bu karede kaç sabit fizik adımı koştu.
                            // 0-adımlı kareler (henüz birikmedi) loglanmaz; çok yüksek bir
                            // sayı frame-spike/spiral-of-death işaretidir.
                            if phys_steps > 0 {
                                tracing::debug!(
                                    steps = phys_steps,
                                    fixed_dt = last_fixed_dt,
                                    "[frame] fixed-timestep physics steps"
                                );
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
                                    tracing::debug!(
                                        fracture_count = physics_world.fracture_events.len(),
                                        "[frame] fracture events -> dust particles"
                                    );
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
                                | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
                                    // A good frame: clear the failure streak.
                                    SURFACE_FAIL_STREAK.store(0, std::sync::atomic::Ordering::Relaxed);
                                    texture
                                }
                                // Transient, non-error states — the window is minimized/behind
                                // another window, or the acquire timed out. The surface config is
                                // still valid; just skip this frame and try again.
                                wgpu::CurrentSurfaceTexture::Occluded
                                | wgpu::CurrentSurfaceTexture::Timeout => {
                                    tracing::trace!(
                                        "[frame] surface transient (occluded/timeout); frame skipped"
                                    );
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
                                // RECOVERABLE surface loss. `Outdated` = the surface config is
                                // stale (display/DPI/format change); `Lost` = the surface (or the
                                // whole GPU context) dropped, e.g. suspend↔resume, a driver TDR,
                                // an external-monitor unplug, or GPU stress. wgpu's first-line fix
                                // for both is to `configure()` again — so reconfigure the swapchain
                                // and retry next frame instead of FREEZING on a black screen (the
                                // old code lumped `Outdated` into the transient skip and never
                                // reconfigured, and treated `Lost` as a dead-end error). This is
                                // the "demos don't crash under stress" hardening.
                                wgpu::CurrentSurfaceTexture::Outdated
                                | wgpu::CurrentSurfaceTexture::Lost => {
                                    use std::sync::atomic::Ordering;
                                    let streak = SURFACE_FAIL_STREAK.fetch_add(1, Ordering::Relaxed) + 1;
                                    // `Outdated` (stale DPI/format/resize) genuinely recovers via
                                    // configure(). `Lost` is BEST-EFFORT here: a true surface/device
                                    // loss needs surface (or device+resource) RECREATION, which the
                                    // renderer can't yet do (it retains no Instance/Window) — a known
                                    // limitation. configure() still recovers the common recoverable
                                    // cases (suspend/resume, transient context loss).
                                    renderer.reconfigure_surface();
                                    // Rate-limit: warn once up front, then ~once/5s, so a persistent
                                    // loss never floods the log.
                                    if streak == 1 || streak.is_multiple_of(300) {
                                        tracing::warn!(
                                            streak,
                                            "[frame] surface Outdated/Lost — reconfigured swapchain; if this persists the surface/device may be unrecoverable"
                                        );
                                    }
                                    // Backoff: after a short grace, throttle the retry so an
                                    // unrecoverable loss degrades to a quiet ~60Hz retry instead of a
                                    // 100%-CPU busy-spin. (Skipped on wasm — the browser throttles rAF.)
                                    #[cfg(not(target_arch = "wasm32"))]
                                    if streak > 4 {
                                        std::thread::sleep(std::time::Duration::from_millis(16));
                                    }
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
                                // A validation error was raised inside `get_current_texture` and
                                // caught by an error scope — a real bug, not recoverable by
                                // reconfiguring. Log it and skip the frame so we neither crash nor
                                // busy-spin on a reconfigure that can't help.
                                wgpu::CurrentSurfaceTexture::Validation => {
                                    use std::sync::atomic::Ordering;
                                    let streak = SURFACE_FAIL_STREAK.fetch_add(1, Ordering::Relaxed) + 1;
                                    if streak == 1 || streak.is_multiple_of(300) {
                                        tracing::error!(
                                            streak,
                                            "[frame] surface acquire raised a validation error; frame skipped"
                                        );
                                    }
                                    #[cfg(not(target_arch = "wasm32"))]
                                    if streak > 4 {
                                        std::thread::sleep(std::time::Duration::from_millis(16));
                                    }
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
                            } else {
                                // Hiçbir render hook yok → 3B sahne çizilmez (klasik "siyah
                                // ekran" tuzağı: egui HUD görünür ama sahne siyah). Bir kez uyar.
                                use std::sync::atomic::{AtomicBool, Ordering};
                                static WARNED: AtomicBool = AtomicBool::new(false);
                                if !WARNED.swap(true, Ordering::Relaxed) {
                                    tracing::warn!(
                                        "Render hook ayarlı DEĞİL — 3B sahne çizilmeyecek (ekran \
                                         siyah kalır). `.with_scene_render()` (manuel App), \
                                         `.set_render(..)` veya `.with_simple_scene(..)` kullan."
                                    );
                                }
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
