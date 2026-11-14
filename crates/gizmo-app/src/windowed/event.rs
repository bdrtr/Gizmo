use super::*;
use super::pipeline::DrawReport;

/// Consecutive `get_current_texture()` failures (Outdated/Lost/Validation) since the last good
/// frame — drives rate-limited logging + backoff so a persistently unrecoverable surface (a true
/// device loss the renderer can't yet recreate) degrades to a throttled retry with one clear
/// warning, instead of a 100%-CPU busy-spin and a per-frame log flood.
static SURFACE_FAIL_STREAK: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

impl<State: 'static> App<State> {
    /// Rewind to a mispredicted tick and silently re-simulate up to the present.
    ///
    /// Sixty lines at depth nine in the middle of the frame, and the only part of the loop that
    /// runs the schedule more than once. `begin_frame` restores both halves of the snapshot (the
    /// ECS components and the physics world — see `gizmo_net`), then this catches up tick by tick,
    /// recording each re-simulated frame so a second rollback into the same window still has
    /// something to rewind to.
    #[cfg(feature = "network")]
    fn service_rollback(&mut self) {
        let mut rollback_needed = false;
        let mut rm = self.world.remove_resource::<gizmo_net::rollback::RollbackManager>();
        
        if let Some(ref mut manager) = rm {
            rollback_needed = manager.begin_frame(&mut self.world);
        }
        
        if rollback_needed {
            let target_tick = rm.as_ref().unwrap().latest_tick;

            // The `clear_bodies()` that used to be here is gone. It threw
            // the physics world away so `sync_bodies` would rebuild it
            // from the ECS — a resync that made sense while the ECS was
            // the only thing being restored. `begin_frame` now restores
            // the physics world itself (joints, accumulator, contact
            // cache), and clearing it afterwards would discard exactly
            // that.
            
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

                // Yeni geçmiş karesini hafızaya al. `record_resimulated_tick`
                // captures BOTH halves — this loop used to inline the wire
                // half only, which is how the physics snapshot came to be
                // missing from the catch-up path as well as the rewind.
                if let Some(ref mut manager) = rm {
                    manager.record_resimulated_tick(&self.world);
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

    /// Which engine key codes one winit key event stands for.
    ///
    /// The physical key is the answer whenever winit has one — `PhysicalKey::Code(k) => k as u32`
    /// is the convention the whole engine reads (`gizmo_core::input::keys` documents it, and
    /// `tests/key_convention.rs` proves the name table against it). The logical fallback below
    /// covers layouts that report no physical code, and it is deliberately a short list rather
    /// than a general name lookup: widening it would start forwarding keys this path has never
    /// forwarded, which is a behaviour change and not a refactor.
    ///
    /// Pure: no `self`, no side effects. It was thirty lines at depth nine inside the event
    /// match, which is the only reason it looked complicated.
    fn key_codes_for(kb_event: &winit::event::KeyEvent) -> Vec<u32> {
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
        }
        codes_to_press
    }

    /// Feed `dt` and `self.input` from a recorded run, or append this frame to one.
    ///
    /// Takes `dt` by reference because playback OVERRIDES it: a replayed frame must advance by the
    /// delta it was captured with, not by how long this machine took to draw it. That is the whole
    /// point of the recording, and it is easy to lose in a refactor — hence the `&mut f32` rather
    /// than a return value nobody has to use.
    fn apply_playback_or_record(&mut self, dt: &mut f32, current_window: &ActiveEventLoop) {
        if let Some(playback) = &self.playback_data {
            if self.playback_frame_index < playback.frames.len() {
                let frame = &playback.frames[self.playback_frame_index];
                *dt = frame.dt;
                self.input = frame.input.clone();
                self.playback_frame_index += 1;
            } else {
                tracing::info!("Playback bitti. Uygulama kapaniyor...");
                current_window.exit();
            }
        } else if self.record_mode {
            if let Some(record) = &mut self.record_data {
                record.frames.push(gizmo_core::input::FrameRecord {
                    dt: *dt,
                    input: self.input.clone(),
                });
            }
        }
    }

    /// Leaves the application the one graceful way: write the input recording if one is being
    /// kept, then signal the event loop to exit, so `run_app` returns and the app, world and
    /// renderer drop in order (the wgpu device and surface close cleanly). Closing the window and
    /// [`AppExit`](gizmo_core::window::AppExit) both come here. (A `process::exit(0)` used to cut
    /// it short without running a destructor.)
    fn shut_down(&self, event_loop: &ActiveEventLoop) {
        if let Some(record) = &self.record_data {
            match record.save("gizmo_record.ron") {
                Ok(()) => tracing::info!(
                    path = "gizmo_record.ron",
                    frames = record.frames.len(),
                    "[App] input kaydı diske yazıldı"
                ),
                Err(e) => tracing::warn!(path = "gizmo_record.ron", error = %e, "[App] input kaydı yazılamadı"),
            }
        }
        event_loop.exit();
    }

    /// Applies the world's [`CursorOptions`](gizmo_core::window::CursorOptions) to the window when
    /// it differs from what was last applied. No resource reads as the default (free, visible).
    ///
    /// `Locked` falls back to `Confined` where the platform refuses to lock (X11 does); mouse-look
    /// works with either, because `Input::mouse_delta` comes from raw device motion.
    fn apply_cursor_options(&mut self, window: &winit::window::Window) {
        use gizmo_core::window::{CursorGrab, CursorOptions};
        use winit::window::CursorGrabMode;
        let wanted = self.world.get_resource::<CursorOptions>().map(|c| *c).unwrap_or_default();
        if wanted == self.applied_cursor {
            return;
        }
        let grab = match wanted.grab {
            CursorGrab::Locked => window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined)),
            CursorGrab::Confined => window.set_cursor_grab(CursorGrabMode::Confined),
            _ => window.set_cursor_grab(CursorGrabMode::None),
        };
        if let Err(e) = grab {
            tracing::warn!(error = %e, "[App] the window refused the cursor grab");
        }
        window.set_cursor_visible(wanted.visible);
        self.applied_cursor = wanted;
    }

    /// Applies the world's [`WindowMode`](gizmo_core::window::WindowMode) when it differs from what
    /// was last applied. Fullscreen is borderless, on the window's current monitor.
    fn apply_window_mode(&mut self, window: &winit::window::Window) {
        use gizmo_core::window::WindowMode;
        let wanted = self.world.get_resource::<WindowMode>().map(|m| *m).unwrap_or_default();
        if wanted == self.applied_mode {
            return;
        }
        window.set_fullscreen(match wanted {
            WindowMode::Fullscreen => Some(winit::window::Fullscreen::Borderless(None)),
            _ => None,
        });
        self.applied_mode = wanted;
    }

    /// Run one pass of the lazy asset loader.
    ///
    /// Takes the asset manager out of the world and puts it back — the loader takes the world
    /// mutably beside it, so the borrow checker, not the logic, chose that shape. Every app has one
    /// (`AssetPlugin`, or the game's own in its place); the renderer is not in the world, and the
    /// loader goes through the game side's `GpuContext`.
    fn run_asset_loading(&mut self) {
        // Through the game side's `GpuContext`: the loader makes meshes and textures, which is
        // what that context is for, and runs with the game's update, not the draw.
        let Some(gpu) = self.world.get_resource::<gizmo_renderer::GpuContext>().map(|g| g.clone()) else {
            return;
        };
        if let Some(mut asset_manager) = self.world.remove_resource::<gizmo_renderer::asset::AssetManager>() {
            gpu.run_asset_loading(&mut self.world, &mut asset_manager);
            self.world.insert_resource(asset_manager);
        }
    }

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
                                self.shut_down(current_window);
                            }
                            WindowEvent::Resized(physical_size) => {
                                // resize() web'de boyutu 640x360'a caplayabilir; WindowInfo'yu
                                // ham fiziksel boyuttan DEĞİL, renderer'ın gerçek (caplenmiş)
                                // boyutundan besle ki kamera aspect'i / picking math'i
                                // surface'le tutarlı kalsın.
                                let effective = {
                                    let r = self.renderer_mut().expect("a window event comes after initialization");
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
                                let codes_to_press = Self::key_codes_for(kb_event);
                                // The text the press wrote, as the layout produced it — for name
                                // fields and the like (`Input::typed_text`). Not while egui owns
                                // the keyboard, for the same reason as the key presses below.
                                if kb_event.state == winit::event::ElementState::Pressed && !consumes_input {
                                    if let Some(text) = kb_event.text.as_ref() {
                                        self.input.on_text(text);
                                    }
                                }

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
                                let (sx, sy) = pointer_scale(&self.world, &window);
                                let position = winit::dpi::PhysicalPosition::new(position.x * sx as f64, position.y * sy as f64);
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
                            // Fingers. Dropped until 2026-11-02, so on a phone (the web build
                            // runs on one) or a touchscreen nothing could be pressed: the first
                            // finger now drives the mouse (`Input::on_touch`), every finger is in
                            // `Input::touches`.
                            WindowEvent::Touch(touch) => {
                                use gizmo_core::input::TouchPhase;
                                let phase = match touch.phase {
                                    winit::event::TouchPhase::Started => TouchPhase::Started,
                                    winit::event::TouchPhase::Moved => TouchPhase::Moved,
                                    winit::event::TouchPhase::Ended => TouchPhase::Ended,
                                    winit::event::TouchPhase::Cancelled => TouchPhase::Cancelled,
                                };
                                let (sx, sy) = pointer_scale(&self.world, &window);
                                self.input.on_touch(
                                    touch.id,
                                    phase,
                                    touch.location.x as f32 * sx,
                                    touch.location.y as f32 * sy,
                                );
                            }
                            // Odak kaybında (Alt-Tab / tarayıcı sekme değişimi) basılı
                            // tuşları bırak — yoksa OS key-up göndermez ve tuşlar
                            // sonsuza dek "basılı" kalıp kamerayı kaydırır.
                            WindowEvent::Focused(false) => {
                                self.input.release_all();
                            }
                            // Gamepads are read straight from the device, so unlike the
                            // keyboard they never stopped sending while focus was away and
                            // never announce what is still held on the way back. `release_all`
                            // dropped that state; this is what restores it, and without it a
                            // player holding the throttle across an Alt-Tab comes back to a
                            // dead pad until they let go.
                            #[cfg(feature = "gamepad")]
                            WindowEvent::Focused(true) => {
                                if let Some(backend) = self.gamepad_backend.as_mut() {
                                    backend.resync(&mut self.input);
                                }
                            }
                            _ => {}
                        }
                        if let WindowEvent::RedrawRequested = event {
                            // When this frame began, and how long the loop left before it since the
                            // last one ended — the pacing delay, and whatever events ran in it.
                            let frame_began = super::FrameInstant::now();
                            let gap = self
                                .frame_ended_at
                                .map(|ended| frame_began.duration_since(ended))
                                .unwrap_or_default();
                            sync_display(&mut self.world, &window);
                            // Per-frame root span; the physics/update/render sub-spans
                            // (from the ECS schedule and physics pipeline) nest under it.
                            let _frame_span = tracing::trace_span!("frame").entered();
                            let now = super::FrameInstant::now();
                            let mut dt = now.duration_since(last_frame_time).as_secs_f32();
                            dt = dt.min(0.05); // Güvenlik çemberi: Frame takılırsa 50ms'den fazla zıplamayacak, yerçekiminden düşme engellenecek.
                            last_frame_time = now;
                            tracing::trace!(dt, "[frame] begin");

                            // Physical controllers, before the replay logic below: during a
                            // recording this is what gets recorded, and during playback the
                            // recorded input overwrites it — a replay must not be steerable.
                            #[cfg(feature = "gamepad")]
                            {
                                if self.gamepad_backend.is_none() {
                                    self.gamepad_backend =
                                        Some(crate::gamepad::GamepadBackend::new(&mut self.input));
                                }
                                if let Some(backend) = self.gamepad_backend.as_mut() {
                                    backend.pump(&mut self.input);
                                }
                            }

                            // Playback / Record mantigi
                            self.apply_playback_or_record(&mut dt, current_window);

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

                            // Editor component and scene save/load requests (reads/mutates
                            // EditorState). The viewport render targets are the sync point's: they
                            // need the renderer and the painter.
                            #[cfg(feature = "editor")]
                            {
                                crate::editor_runtime::process_component_requests(&mut self.world);
                                crate::editor_runtime::process_scene_requests(&mut self.world);
                            }

                            // ECS Sistemlerini Çalıştırmadan önce DI için Core Resource'ları Güncelle
                            self.world.insert_resource(self.input.clone());
                            // `sim_dt` = Time'ın `time_scale` ile ölçeklenmiş (ve clamp'lenmiş)
                            // dt'si → fiziği/ECS'i besler ki `set_time_scale(0.0)` gerçekten
                            // DURDURSUN, `0.5` ağır çekim yapsın. `time_scale == 1.0` (varsayılan)
                            // iken `min(dt, max_dt) == dt` olduğundan davranış bit-aynı. Kullanıcı
                            // update hook'u ham `dt` alır (kamera/UI duraklamada bile akıcı kalsın).
                            let sim_dt = crate::frame::advance_time(&mut self.world, dt);

                            // ═══ Fixed Timestep Fizik Döngüsü ═══
                            if let Some(mut profiler) = self.world.get_resource_mut::<gizmo_core::profiler::FrameProfiler>() {
                                profiler.begin_scope("physics");
                            }
                            // `PhysicsTime` is ensured and fed by `frame::run_fixed_and_update`
                            // below; it is created here only so the rollback block that follows
                            // can read `fixed_dt` before the first frame has stepped.
                            if self
                                .world
                                .get_resource::<gizmo_core::time::PhysicsTime>()
                                .is_none()
                            {
                                self.world
                                    .insert_resource(gizmo_core::time::PhysicsTime::default());
                            }

                            #[cfg(feature = "network")]
                            self.service_rollback();

                            // Fixed-timestep simulation, then the once-per-frame update.
                            //
                            // The accumulator drains into `self.schedule` at a constant dt
                            // (0..N times, frame-rate independent), and `self.update_schedule`
                            // then runs EXACTLY ONCE with the real frame delta. That second
                            // part is the fix for the defect this loop used to have: with only
                            // the fixed schedule, a frame that did not fill the accumulator ran
                            // no systems at all — and since `Input` is captured just above and
                            // cleared at the bottom of this same frame, key edges and mouse
                            // deltas on those frames were written and discarded unobserved.
                            // With vsync off by default, that was most frames.
                            //
                            // See `crate::frame` for the sequencing and its regression tests.
                            let steps = crate::frame::run_fixed_and_update(
                                &mut self.world,
                                &mut self.schedule,
                                &mut self.update_schedule,
                                sim_dt,
                                dt,
                                |_world| {
                                    #[cfg(feature = "network")]
                                    if let Some(mut rm) = _world
                                        .get_resource_mut::<gizmo_net::rollback::RollbackManager>()
                                    {
                                        rm.end_frame(_world);
                                    }
                                },
                            );
                            let _ = steps;

                            if let Some(mut profiler) = self.world.get_resource_mut::<gizmo_core::profiler::FrameProfiler>() {
                                profiler.end_scope("physics");
                            }

                            if let Some(mut profiler) = self.world.get_resource_mut::<gizmo_core::profiler::FrameProfiler>() {
                                profiler.begin_scope("update");
                            }

                            // Kullanıcı update hook'u (render dt ile — kamera, UI, vb.)
                            for update_hk in self.update_fns.iter_mut() {
                                update_hk(&mut self.world, &mut state, dt, &self.input);
                            }

                            // Update sonrası olası ertelenmiş komutları (CommandQueue) hemen işle
                            self.world.apply_commands();

                            // Asset Loading System (Lazy Load)
                            self.run_asset_loading();

                            // The game asked to quit: leave the way closing the window does.
                            if self.world.get_resource::<gizmo_core::window::AppExit>().is_some() {
                                tracing::info!("[App] AppExit requested; shutting down");
                                self.shut_down(current_window);
                            }

                            // The cursor behaviour the game asked for this frame.
                            self.apply_cursor_options(&window);
                            self.apply_window_mode(&window);


                            if let Some(mut profiler) = self.world.get_resource_mut::<gizmo_core::profiler::FrameProfiler>() {
                                profiler.end_scope("update");
                            }

                            // Olayları Güncelle (Çift-buffer temizliği)
                            for updater in &mut self.event_updaters {
                                updater(&mut self.world);
                            }

                            if let Some(mut profiler) = self.world.get_resource_mut::<gizmo_core::profiler::FrameProfiler>() {
                                profiler.begin_scope("render");
                            }

                            // ═══ The sync point ═══
                            // The game's update is done, and the draw's side comes back — at once
                            // on the loop's thread, or when the render thread has finished the last
                            // frame. Everything the frame does with the world and the renderer
                            // happens here, before the swapchain is asked for an image, so it
                            // happens on a frame that turns out not to be drawn, too.
                            // What the last frame's draw said — a screenshot asking to exit after
                            // it — is acted on here, where the loop is.
                            let synced_at = super::FrameInstant::now();
                            let (report, threaded) = {
                                let gpu = self.gpu.as_mut().expect("a frame comes after initialization");
                                gpu.sync();
                                (gpu.take_report(), gpu.is_threaded())
                            };
                            if report.exit {
                                self.shut_down(current_window);
                            }
                            // The last frame's draw, timed on whichever thread ran it: with a render
                            // thread it ran outside every scope of this one, and the `render` scope
                            // is only this sync point.
                            if !report.busy.is_zero() {
                                if let Some(mut profiler) = self.world.get_resource_mut::<gizmo_core::profiler::FrameProfiler>() {
                                    profiler.record_scope("draw", report.busy);
                                }
                            }
                            // Pacing (see `pacing`): the render thread was busy `report.busy` with
                            // the last frame; the loop was busy for the time since it handed that
                            // frame over, less the gap it left on purpose. What the draw has over
                            // the loop is how much later the next update can start.
                            let pace = match self.handed_at.take() {
                                Some(handed) if threaded => {
                                    let loop_busy = synced_at.duration_since(handed).saturating_sub(gap);
                                    self.pacing.record(report.busy, loop_busy)
                                }
                                _ => std::time::Duration::ZERO,
                            };
                            let gpu = self.gpu.as_mut().expect("a frame comes after initialization");
                            let side = gpu.sync();
                            // A stale swapchain the last draw found: configured here, where nothing
                            // else submits — see `acquire_backbuffer`.
                            if report.reconfigure {
                                side.renderer.reconfigure_surface();
                            }
                            #[cfg(feature = "editor")]
                            crate::editor_runtime::sync_render_targets(&mut self.world, &side.renderer, &mut side.painter);
                            if let Some(stage) = side.stage.as_mut() {
                                stage.prepare(&mut self.world, &mut side.renderer);
                            }
                            #[cfg(feature = "egui")]
                            let ui_frame = editor.end_frame(&window, full_output);

                            // ═══ The draw ═══
                            if side.stage.is_some() {
                                // From what the stage kept, with no world: on the render thread
                                // when there is one, while the loop goes on to the next update.
                                gpu.draw(Box::new(move |side: &mut DrawSide| {
                                    let DrawSide {
                                        renderer,
                                        stage,
                                        #[cfg(feature = "egui")]
                                        painter,
                                    } = side;
                                    let stage = stage.as_mut().expect("a stage frame is drawn by the stage");
                                    draw_and_present(
                                        renderer,
                                        #[cfg(feature = "egui")]
                                        painter,
                                        #[cfg(feature = "egui")]
                                        ui_frame,
                                        |encoder, view, renderer| stage.draw(encoder, view, renderer),
                                    )
                                }));
                                self.handed_at = Some(super::FrameInstant::now());
                            } else {
                                // A hook handed the game world in the middle of the draw: drawn
                                // here, at the sync point.
                                let render = &mut self.render;
                                let world = &mut self.world;
                                let report = draw_and_present(
                                    &mut side.renderer,
                                    #[cfg(feature = "egui")]
                                    &mut side.painter,
                                    #[cfg(feature = "egui")]
                                    ui_frame,
                                    |encoder, view, renderer| match render {
                                        // Kullaniciya CommandEncoder verip cizdiriyoruz!
                                        RenderHook::Raw(render_hk) => {
                                            render_hk(world, &state, encoder, view, renderer, light_time)
                                        }
                                        RenderHook::Simple(s_render) => {
                                            let mut ctx = RenderContext::new(encoder, view, renderer, light_time);
                                            s_render(world, &state, &mut ctx);
                                        }
                                        RenderHook::Stage(_) => unreachable!("a stage moves to the draw's side at start"),
                                        RenderHook::None => {
                                            // Hiçbir render hook yok → 3B sahne çizilmez (klasik
                                            // "siyah ekran" tuzağı: egui HUD görünür ama sahne
                                            // siyah). Bir kez uyar.
                                            use std::sync::atomic::{AtomicBool, Ordering};
                                            static WARNED: AtomicBool = AtomicBool::new(false);
                                            if !WARNED.swap(true, Ordering::Relaxed) {
                                                tracing::warn!(
                                                    "Render hook ayarlı DEĞİL — 3B sahne çizilmeyecek \
                                                     (ekran siyah kalır). `.with_scene_render()` (manuel \
                                                     App), `.set_render(..)` veya `.with_simple_scene(..)` \
                                                     kullan."
                                                );
                                            }
                                        }
                                    },
                                );
                                if report.exit {
                                    self.shut_down(current_window);
                                }
                            }

                            if let Some(mut profiler) = self.world.get_resource_mut::<gizmo_core::profiler::FrameProfiler>() {
                                profiler.end_scope("render");
                            }

                            // Rumble the frame's systems asked for, handed to the driver here —
                            // AFTER they have run, because that is when the requests exist, and
                            // before `begin_frame`, which is where a request made this frame would
                            // otherwise sit unseen until the next one.
                            #[cfg(feature = "gamepad")]
                            if let Some(backend) = self.gamepad_backend.as_mut() {
                                backend.apply_rumble(&mut self.input);
                            }

                            // İşlemlerin bitiminde frame-özel input girdilerini (fare delta vs.) temizle
                            self.input.begin_frame();

                            if let Some(mut profiler) = self.world.get_resource_mut::<gizmo_core::profiler::FrameProfiler>() {
                                profiler.end_frame();
                            }

                            let ended = super::FrameInstant::now();
                            self.frame_ended_at = Some(ended);
                            self.next_frame_at = (!pace.is_zero()).then(|| ended + pace);
                        }
                    }
                    Event::AboutToWait => {
                        // The next frame now — or, while a render thread draws the last one and
                        // the loop would only wait for it at the sync point, once the pacing delay
                        // is up. Window events keep arriving meanwhile, and the update that follows
                        // takes them.
                        match self.next_frame_at.filter(|at| super::FrameInstant::now() < *at) {
                            #[cfg(not(target_arch = "wasm32"))]
                            Some(at) => current_window.set_control_flow(ControlFlow::WaitUntil(at)),
                            _ => {
                                self.next_frame_at = None;
                                window.request_redraw();
                            }
                        }
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

/// One frame's draw, from the swapchain image to the present: `draw` records the scene, the
/// overlay UI is painted over it, and the frame is submitted, captured if a screenshot asked for
/// it, and presented. Runs on the loop's thread or on the render thread, and needs nothing from
/// either but what it is given.
///
/// A frame whose image cannot be acquired is not drawn — and the UI frame is still absorbed: egui
/// hands its texture deltas over once.
fn draw_and_present(
    renderer: &mut Renderer,
    #[cfg(feature = "egui")] painter: &mut crate::egui_ctx::EguiPainter,
    #[cfg(feature = "egui")] ui_frame: crate::egui_ctx::EguiFrame,
    draw: impl FnOnce(&mut wgpu::CommandEncoder, &wgpu::TextureView, &mut Renderer),
) -> DrawReport {
    let output = match acquire_backbuffer(renderer) {
        Backbuffer::Ready(output) => output,
        skipped => {
            // The one epilogue: every skip path in the acquire lands here. The egui frame already
            // ran, and its texture deltas are handed over exactly once. Skipping the *pixels* is
            // what this path is for; skipping the *uploads* dropped the font atlas for the rest of
            // the run — and killed every debug build on epaint's assert, which is why no windowed
            // demo could start.
            #[cfg(feature = "egui")]
            painter.absorb(&renderer.device, &renderer.queue, ui_frame);
            return DrawReport { reconfigure: matches!(skipped, Backbuffer::Reconfigure), ..DrawReport::default() };
        }
    };

    let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder =
        renderer.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Render Encoder") });

    draw(&mut encoder, &view, renderer);

    #[cfg(feature = "egui")]
    painter.paint(
        &renderer.device,
        &renderer.queue,
        &mut encoder,
        &view,
        // The acquired backbuffer, not the window: the two disagree for one frame on every resize.
        [output.texture.width(), output.texture.height()],
        ui_frame,
    );

    renderer.queue.submit(std::iter::once(encoder.finish()));

    // Between submit and present: the surface texture is still alive and now holds the finished
    // frame, egui included. This is the only moment in the frame where the engine can see what the
    // user sees.
    let report = service_screenshot(renderer, &output);

    // wgpu 30 moved presentation off the texture and onto the queue:
    // `SurfaceTexture::present(self)` became `Queue::present(texture)`.
    renderer.queue.present(output);
    report
}

/// Acquire this frame's backbuffer, or `None` when the frame must be skipped.
///
/// A free function of the renderer, because it runs in the draw — on a render thread, when there is
/// one — where the app is not.
///
/// The recovery behaviour is unchanged: transient states skip quietly, `Outdated`/`Lost`
/// reconfigure the swapchain and back off, a validation error is logged and skipped, and the
/// failure streak drives the rate-limited logging.
fn acquire_backbuffer(renderer: &Renderer) -> Backbuffer {
    use std::sync::atomic::Ordering;
    // No surface: the app is off the screen (`suspended`) and there is nowhere to draw.
    let Some(surface) = renderer.surface.as_ref() else { return Backbuffer::Skip };
    match surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(texture) | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
            // A good frame: clear the failure streak.
            SURFACE_FAIL_STREAK.store(0, Ordering::Relaxed);
            Backbuffer::Ready(texture)
        }
        // Transient, non-error states — the window is minimized/behind another window, or the
        // acquire timed out. The surface config is still valid; just skip this frame and try again.
        wgpu::CurrentSurfaceTexture::Occluded | wgpu::CurrentSurfaceTexture::Timeout => {
            tracing::trace!("[frame] surface transient (occluded/timeout); frame skipped");
            Backbuffer::Skip
        }
        // RECOVERABLE surface loss. `Outdated` = the surface config is stale (display/DPI/format
        // change); `Lost` = the surface (or the whole GPU context) dropped, e.g. suspend↔resume, a
        // driver TDR, an external-monitor unplug, or GPU stress. wgpu's first-line fix for both is
        // to `configure()` again — so the swapchain is reconfigured and the frame retried instead
        // of FREEZING on a black screen. `Outdated` genuinely recovers that way; `Lost` is
        // best-effort (a true device loss needs the device recreated, which the renderer cannot
        // yet do).
        //
        // **Not here, though.** This may be the render thread, and configuring waits for the GPU
        // to go idle: a texture the game submits during that wait (streaming, a mesh that finished
        // loading) fails the wait, and wgpu raises that as an error. The loop configures at the
        // sync point instead, where it is the only submitter (`DrawReport::reconfigure`).
        wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
            let streak = SURFACE_FAIL_STREAK.fetch_add(1, Ordering::Relaxed) + 1;
            // Rate-limit: warn once up front, then ~once/5s, so a persistent loss never floods the
            // log.
            if streak == 1 || streak.is_multiple_of(300) {
                tracing::warn!(
                    streak,
                    "[frame] surface Outdated/Lost — reconfiguring the swapchain; if this persists the surface/device may be unrecoverable"
                );
            }
            // Backoff: after a short grace, throttle the retry so an unrecoverable loss degrades to
            // a quiet ~60Hz retry instead of a 100%-CPU busy-spin. (Skipped on wasm — the browser
            // throttles rAF.)
            #[cfg(not(target_arch = "wasm32"))]
            if streak > 4 {
                std::thread::sleep(std::time::Duration::from_millis(16));
            }
            Backbuffer::Reconfigure
        }
        // A validation error was raised inside `get_current_texture` and caught by an error scope
        // — a real bug, not recoverable by reconfiguring. Log it and skip the frame so we neither
        // crash nor busy-spin on a reconfigure that can't help.
        wgpu::CurrentSurfaceTexture::Validation => {
            let streak = SURFACE_FAIL_STREAK.fetch_add(1, Ordering::Relaxed) + 1;
            if streak == 1 || streak.is_multiple_of(300) {
                tracing::error!(streak, "[frame] surface acquire raised a validation error; frame skipped");
            }
            #[cfg(not(target_arch = "wasm32"))]
            if streak > 4 {
                std::thread::sleep(std::time::Duration::from_millis(16));
            }
            Backbuffer::Skip
        }
    }
}

/// What the acquire got.
enum Backbuffer {
    /// An image to draw into.
    Ready(wgpu::SurfaceTexture),
    /// No image this frame; the next one tries again.
    Skip,
    /// No image, and the swapchain is stale: the loop configures it again at the next sync point.
    Reconfigure,
}

/// Write the presented frame to a PNG when `GIZMO_SCREENSHOT` asks for it.
///
/// Called from the one point in the draw where the finished frame is still readable — after
/// `submit`, before `present`. Does nothing at all without the env var, which is why it can sit
/// unconditionally in the hot path.
///
/// The request lives in statics rather than in `App`: it comes from the process environment, so
/// it is process-wide by nature, and threading a diagnostic through the builder, the struct,
/// its `Debug` impl and every constructor would cost more than it explains.
fn service_screenshot(renderer: &Renderer, output: &wgpu::SurfaceTexture) -> DrawReport {
    use gizmo_renderer::capture::CaptureRequest;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::OnceLock;

    static REQUEST: OnceLock<Option<CaptureRequest>> = OnceLock::new();
    static FRAME: AtomicU64 = AtomicU64::new(0);

    let Some(req) = REQUEST.get_or_init(CaptureRequest::from_env) else {
        return DrawReport::default();
    };
    // `fetch_add` returns the previous value, so the first presented frame is 0.
    let frame = FRAME.fetch_add(1, Ordering::Relaxed);
    let Some(path) = req.path_for(frame) else {
        return DrawReport::default();
    };

    match gizmo_renderer::capture::texture_to_png(&renderer.device, &renderer.queue, &output.texture, &path) {
        Ok(()) => tracing::info!(path = %path.display(), frame, "screenshot written"),
        Err(e) => tracing::error!("{e}"),
    }
    // The exit is the loop's to make, at its next sync point: the draw may be on a render thread.
    DrawReport { exit: req.exit_after && req.is_last(frame), ..DrawReport::default() }
}

/// Keeps `DisplayScale` and `SafeArea` true to the window, the frame and the platform. Called every
/// frame and writes only on a change: Android sends no event when its content rectangle moves, and
/// a resize, a scale change and a frame budget all feed the same number.
fn sync_display(world: &mut gizmo_core::World, window: &winit::window::Window) {
    let physical = window.inner_size();
    let frame = world.get_resource::<gizmo_core::window::WindowInfo>().map(|i| (i.width, i.height));
    let Some((frame_w, frame_h)) = frame else { return };
    let scale =
        gizmo_core::window::DisplayScale::from_window(frame_w, physical.width as f32, window.scale_factor() as f32);
    if world.get_resource::<gizmo_core::window::DisplayScale>().map(|s| *s) != Some(scale) {
        world.insert_resource(scale);
    }
    #[cfg(target_os = "android")]
    if let Some(app) = crate::android::app() {
        let r = app.content_rect();
        let area = safe_area(
            [r.left, r.top, r.right, r.bottom],
            (physical.width as f32, physical.height as f32),
            (frame_w, frame_h),
        );
        if world.get_resource::<gizmo_core::window::SafeArea>().map(|a| *a) != Some(area) {
            tracing::debug!(?area, ?scale, "[App] display");
            world.insert_resource(area);
        }
    }
    #[cfg(not(target_os = "android"))]
    let _ = frame_h;
}

/// The window edges outside a content rectangle (`[left, top, right, bottom]`, physical pixels), in
/// frame pixels. An empty rectangle is no information, not a window wholly covered.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn safe_area(content: [i32; 4], physical: (f32, f32), frame: (f32, f32)) -> gizmo_core::window::SafeArea {
    let [left, top, right, bottom] = content;
    if right <= left || bottom <= top || physical.0 <= 0.0 || physical.1 <= 0.0 {
        return gizmo_core::window::SafeArea::default();
    }
    let (sx, sy) = (frame.0 / physical.0, frame.1 / physical.1);
    gizmo_core::window::SafeArea {
        left: left.max(0) as f32 * sx,
        top: top.max(0) as f32 * sy,
        right: (physical.0 - right as f32).max(0.0) * sx,
        bottom: (physical.1 - bottom as f32).max(0.0) * sy,
    }
}

/// What a pointer position in window pixels is multiplied by to land in the frame's pixels — the
/// space `WindowInfo`, the UI layout and the camera all work in.
///
/// On the desktop the two are the same, and this is exactly (1, 1). In the browser the frame is
/// rendered below the canvas's size and stretched up by CSS (`cap_web_render_size`), and pointer
/// events still arrive in the canvas's own pixels: on a phone a finger at x = 300 of 1080 was
/// taken for x = 300 of a 350-wide frame, so every touch control and UI click past the top-left
/// corner landed off to the right of where it was meant, or off the frame altogether.
fn pointer_scale(world: &gizmo_core::World, window: &winit::window::Window) -> (f32, f32) {
    let physical = window.inner_size();
    let Some(info) = world.get_resource::<gizmo_core::window::WindowInfo>() else { return (1.0, 1.0) };
    if physical.width == 0 || physical.height == 0 || info.width <= 0.0 || info.height <= 0.0 {
        return (1.0, 1.0);
    }
    (info.width / physical.width as f32, info.height / physical.height as f32)
}

#[cfg(test)]
mod safe_area_tests {
    use super::safe_area;

    /// The phone it was measured on: a 1080 x 2307 window whose bottom 130 pixels are under the
    /// navigation bar, drawn in a frame of 656 x 1401.
    #[test]
    fn the_covered_edge_is_in_frame_pixels() {
        let area = safe_area([0, 0, 1080, 2177], (1080.0, 2307.0), (656.0, 1401.0));
        assert!((area.bottom - 78.95).abs() < 0.1, "{area:?}");
        assert_eq!((area.left, area.top, area.right), (0.0, 0.0, 0.0));
    }

    #[test]
    fn an_empty_rectangle_covers_nothing() {
        assert_eq!(safe_area([0, 0, 0, 0], (1080.0, 2307.0), (656.0, 1401.0)), Default::default());
    }
}
