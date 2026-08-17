//! The top toolbar panel (save/load, play/pause, gizmo mode).

use crate::editor_state::{BuildTarget, EditorMode, EditorState, GizmoMode};
use egui;
use crate::theme::palette::{ACCENT, ACCENT_LIGHT, ACCENT_PALE, BORDER, TEXT_BRIGHT, TEXT_DIM};

#[cfg(target_arch = "wasm32")]
use web_time::Instant;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

/// Draws the toolbar panel.
pub fn draw_toolbar(ui: &mut egui::Ui, state: &mut EditorState) {
    // The prototype's top chrome is TWO rows, and that split is most of what makes it read as an
    // engine editor rather than a strip of buttons: a 26 px menu bar (mark, menus, breadcrumb) and
    // a tool row (mode buttons, then labelled chips for the modal settings).
    //
    // Everything that used to live in the single 36 px row is still reachable — the file actions,
    // the layout commands, the profiler, the settings, the navmesh build and the build target all
    // moved into the menus rather than disappearing.
    draw_menu_bar(ui, state);
    draw_tool_row(ui, state);
}

/// Row one: the mark, the menus, and the breadcrumb.
fn draw_menu_bar(ui: &mut egui::Ui, state: &mut EditorState) {
    egui::Panel::top("toolbar_menu")
        .exact_size(26.0)
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                draw_wordmark(ui);
                ui.add_space(6.0);

                let is_dialog_open = state.pending_dialog_rx.is_some();

                ui.menu_button("File", |ui| {
                    if ui.button("Yeni / Temizle").clicked() {
                        state.scene.clear_request = true;
                        ui.close();
                    }
                    ui.separator();
                    if ui.add_enabled(!is_dialog_open, egui::Button::new("Kaydet…")).clicked() {
                        spawn_scene_dialog(state, true);
                        ui.close();
                    }
                    if ui.add_enabled(!is_dialog_open, egui::Button::new("Yükle…")).clicked() {
                        spawn_scene_dialog(state, false);
                        ui.close();
                    }
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label("Sahne:");
                        ui.add(egui::TextEdit::singleline(&mut state.scene_path).desired_width(180.0));
                    });
                });

                ui.menu_button("Edit", |ui| {
                    if ui.button("Ayarlar").clicked() {
                        state.open_tab(crate::editor_state::EditorTab::Settings);
                        ui.close();
                    }
                });

                ui.menu_button("Entity", |ui| {
                    if ui
                        .button("NavMesh Kur")
                        .on_hover_text(
                            "Fiziksel dünyadaki statik objelere göre Yapay Zeka navigasyon \
                             ızgarasını (NavMesh) yeniden oluşturur.",
                        )
                        .clicked()
                    {
                        state.scene.rebuild_navmesh_request = true;
                        ui.close();
                    }
                });

                ui.menu_button("Assets", |ui| {
                    if ui.button("Asset Browser").clicked() {
                        state.open_tab(crate::editor_state::EditorTab::AssetBrowser);
                        ui.close();
                    }
                });

                draw_build_menu(ui, state);

                ui.menu_button("Window", |ui| {
                    if ui.button("Profiler").clicked() {
                        state.toggle_tab(crate::editor_state::EditorTab::Profiler);
                        ui.close();
                    }
                    if ui.button("Console").clicked() {
                        state.toggle_tab(crate::editor_state::EditorTab::Console);
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Düzeni Kaydet").clicked() {
                        if let Err(e) = state.save_layout() {
                            state.log_error(&format!("Layout kaydedilemedi: {}", e));
                        }
                        ui.close();
                    }
                    if ui.button("Varsayılan Düzene Dön").clicked() {
                        state.reset_layout();
                        ui.close();
                    }
                });

                ui.menu_button("Help", |ui| {
                    ui.label(format!("Gizmo Engine {}", env!("CARGO_PKG_VERSION")));
                    ui.label("wgpu / vulkan");
                });

                // The breadcrumb, right-aligned as in the prototype: project · scene · branch.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let scene = std::path::Path::new(&state.scene_path)
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| "untitled".into());
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new("main").color(TEXT_DIM).size(11.0));
                    ui.label(egui::RichText::new("·").color(BORDER));
                    ui.label(egui::RichText::new(scene).color(TEXT_BRIGHT).size(11.0));
                    ui.label(egui::RichText::new("·").color(BORDER));
                    ui.label(egui::RichText::new("project").color(TEXT_DIM).size(11.0));
                });
            });
        });
}

/// Row two: the transform tools, the modal settings as chips, and the transport.
fn draw_tool_row(ui: &mut egui::Ui, state: &mut EditorState) {
    egui::Panel::top("toolbar_tools")
        .exact_size(30.0)
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;

                // Q-W-E-R (only while nothing is taking text).
                if !ui.ctx().egui_wants_keyboard_input() {
                    if ui.input(|i| i.key_pressed(egui::Key::Q)) { state.gizmo_mode = GizmoMode::Select; }
                    if ui.input(|i| i.key_pressed(egui::Key::W)) { state.gizmo_mode = GizmoMode::Translate; }
                    if ui.input(|i| i.key_pressed(egui::Key::E)) { state.gizmo_mode = GizmoMode::Rotate; }
                    if ui.input(|i| i.key_pressed(egui::Key::R)) { state.gizmo_mode = GizmoMode::Scale; }
                }

                // Icon-only, like the prototype. The shortcut lives in the tooltip instead of the
                // label, which is what lets four tools fit in the width of one old button.
                // Short words, not icons. The prototype uses glyphs, but egui's bundled font has
                // no ✥/⟳/⤢ and renders each as an empty box — which the old toolbar was already
                // doing with 🖐/🔀/🔄/📏. A missing glyph is worse than a legible word, and
                // shipping an icon set means vendoring a font.
                for (mode, glyph, tip) in [
                    (GizmoMode::Select, "Seç", "Seç (Q)"),
                    (GizmoMode::Translate, "Taşı", "Taşı (W)"),
                    (GizmoMode::Rotate, "Döndür", "Döndür (E)"),
                    (GizmoMode::Scale, "Ölçek", "Ölçekle (R)"),
                ] {
                    let on = state.gizmo_mode == mode;
                    if ui
                        .add(egui::Button::selectable(on, egui::RichText::new(glyph).size(11.0)))
                        .on_hover_text(tip)
                        .clicked()
                    {
                        state.gizmo_mode = mode;
                    }
                }

                ui.add_space(6.0);

                // `Space | Global` — a chip, not a button: the label says which setting it is and
                // the value says where it stands, which is how every one of these reads in the
                // prototype.
                let space_value = if state.gizmo_local_space { "Local" } else { "Global" };
                if chip(ui, "Space", space_value, false).clicked() {
                    state.gizmo_local_space = !state.gizmo_local_space;
                }

                let snap_on = state.prefs.snap_enabled;
                if chip(ui, "Grid snap", &format!("{:.2}", state.prefs.snap_translate), snap_on)
                    .on_hover_text("Grid'e yapışma (Ctrl basılıyken tersine döner)")
                    .clicked()
                {
                    state.prefs.snap_enabled = !snap_on;
                    state.prefs.mark_dirty();
                }

                if chip(ui, "Angle", &format!("{:.0}°", state.prefs.snap_rotate_deg), snap_on)
                    .on_hover_text("Döndürme adımı")
                    .clicked()
                {
                    state.prefs.snap_enabled = !snap_on;
                    state.prefs.mark_dirty();
                }

                ui.add_space(6.0);

                // A segmented control, not a combo box: four states you switch between constantly,
                // and the prototype shows the current one without making you open anything.
                crate::theme::segmented(
                    ui,
                    &mut state.shading_mode,
                    &[(0u32, "Lit"), (1, "Normals"), (2, "Albedo"), (3, "Wire")],
                );

                ui.add_space(crate::theme::SPACE_1);
                crate::theme::toggle(ui, &mut state.show_colliders, "Colliders");

                // Transport on the right, where the prototype keeps it.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(4.0);
                    if state.mode == EditorMode::Edit {
                        if ui
                            .button(egui::RichText::new("▶ Başlat").color(ACCENT))
                            .clicked()
                        {
                            state.toggle_play();
                        }
                    } else {
                        if ui
                            .button(egui::RichText::new("⏹ Durdur").color(ACCENT))
                            .clicked()
                        {
                            state.toggle_play();
                        }
                        let pause_text = if state.mode == EditorMode::Play { "⏸" } else { "▶" };
                        if ui
                            .button(egui::RichText::new(pause_text).color(ACCENT_LIGHT))
                            .clicked()
                        {
                            state.toggle_pause();
                        }
                    }

                    if state.build.is_building.load(std::sync::atomic::Ordering::Acquire) {
                        let elapsed = state
                            .build
                            .start_time
                            .map(|st| format!(" ({}s)", st.elapsed().as_secs()))
                            .unwrap_or_default();
                        ui.label(egui::RichText::new(format!("Derleniyor…{elapsed}")).color(ACCENT_LIGHT));
                        ui.add(egui::Spinner::new().size(12.0));
                    }
                });
            });
        });
}

/// A labelled setting: a dim name and a bright value inside one bordered box.
///
/// The prototype's `Space Global` / `Grid snap 0.25` / `Angle 15°` controls. Written as one helper
/// because the three differ only in their two strings, and because the accent state — a filled
/// square and an accented border when the setting is engaged — is the kind of detail that drifts
/// apart when it is spelled out three times.
fn chip(ui: &mut egui::Ui, label: &str, value: &str, engaged: bool) -> egui::Response {
    let mut text = egui::text::LayoutJob::default();
    text.append(
        label,
        0.0,
        egui::TextFormat {
            font_id: egui::FontId::proportional(11.0),
            color: if engaged { ACCENT_PALE } else { TEXT_DIM },
            ..Default::default()
        },
    );
    text.append(
        value,
        6.0,
        egui::TextFormat {
            font_id: egui::FontId::proportional(11.0),
            color: TEXT_BRIGHT,
            ..Default::default()
        },
    );
    let button = egui::Button::new(text)
        .stroke(egui::Stroke::new(1.0_f32, if engaged { ACCENT } else { BORDER }))
        .corner_radius(0);
    ui.add(button)
}

/// The Build menu: target, then the action.
fn draw_build_menu(ui: &mut egui::Ui, state: &mut EditorState) {
    ui.menu_button("Build", |ui| {
        let target_label = match state.build.target {
            BuildTarget::Native => "Native (Mevcut OS)",
            BuildTarget::Linux => "Linux",
            BuildTarget::Windows => "Windows",
            BuildTarget::MacOs => "macOS",
        };
        ui.label(egui::RichText::new(format!("Hedef: {target_label}")).color(TEXT_DIM).size(11.0));
        ui.selectable_value(&mut state.build.target, BuildTarget::Native, "Native (Mevcut OS)");
        ui.selectable_value(&mut state.build.target, BuildTarget::Linux, "Linux (ELF)");
        ui.selectable_value(&mut state.build.target, BuildTarget::Windows, "Windows (.exe)");
        ui.selectable_value(&mut state.build.target, BuildTarget::MacOs, "macOS");
        ui.separator();

        let building = state.build.is_building.load(std::sync::atomic::Ordering::Acquire);
        if ui
            .add_enabled(!building, egui::Button::new("Build Et"))
            .clicked()
        {
            state.build.request = true;
            state.build.start_time = Some(Instant::now());
            state.open_tab(crate::editor_state::EditorTab::Console);
            state.console.mode = crate::editor_state::ConsoleMode::BuildOutput;
            ui.close();
        }
    });
}

/// Opens the native save/load dialog on a worker thread and parks the receiver on the state.
///
/// One function for both directions: the two used to be forty near-identical lines apart, agreeing
/// by hand on the filter, the initial directory and the `\\?\` prefix strip that Windows adds.
fn spawn_scene_dialog(state: &mut EditorState, saving: bool) {
    let (tx, rx) = std::sync::mpsc::channel();
    state.pending_dialog_rx = Some(std::sync::Mutex::new(rx));

    // The browser has no native file dialog, and `std::thread::spawn` is not supported on
    // `wasm32-unknown-unknown` either — it does not run the closure, it panics. So the wasm arm
    // answers "cancelled" on the spot rather than spawning a thread that cannot exist. It used to
    // share the native path's thread, which meant Save/Load in a browser build panicked on click.
    #[cfg(target_arch = "wasm32")]
    let _ = tx.send((saving, None));

    #[cfg(not(target_arch = "wasm32"))]
    {
        let scene_path = state.scene_path.clone();
        std::thread::spawn(move || {
            // Derived inside the native arm because it exists only for `set_directory`. Outside,
            // it was computed on wasm too and thrown away — dead work the native lint could not
            // see, since it reads as used on the target it compiles for.
            let initial_dir = std::path::Path::new(&scene_path)
                .parent()
                .filter(|p| p.is_dir())
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| std::path::PathBuf::from("."));

            let dialog = rfd::FileDialog::new()
                .add_filter("Gizmo Scene", &["scene"])
                .set_directory(&initial_dir);
            let res = if saving { dialog.save_file() } else { dialog.pick_file() };
            let _ = tx.send((
                saving,
                res.map(|p: std::path::PathBuf| {
                    let s = p.to_string_lossy().to_string();
                    s.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(s)
                }),
            ));
        });
    }
}

/// The prototype's mark: an accent square with a crosshair knocked out of it, then GIZMO in
/// letterspaced caps.
///
/// Drawn rather than assembled from widgets because it is a 11x11 px glyph with 1 px cuts — the
/// prototype builds it from three absolutely-positioned divs, and the painter is the direct
/// translation of that. Sized off the row so it tracks the theme instead of pinning a magic
/// height next to one.
fn draw_wordmark(ui: &mut egui::Ui) {
    const MARK: f32 = 11.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(MARK, MARK), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 0.0, ACCENT);
    // The two 1 px cuts, in the chrome colour, exactly as the prototype knocks them out.
    let cut = ui.visuals().panel_fill;
    p.rect_filled(
        egui::Rect::from_min_size(rect.min + egui::vec2(5.0, 1.0), egui::vec2(1.0, 9.0)),
        0.0,
        cut,
    );
    p.rect_filled(
        egui::Rect::from_min_size(rect.min + egui::vec2(1.0, 5.0), egui::vec2(9.0, 1.0)),
        0.0,
        cut,
    );
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new("G I Z M O")
            .size(13.0)
            .strong()
            .color(TEXT_BRIGHT),
    );
}
