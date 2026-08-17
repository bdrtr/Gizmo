#![deny(clippy::undocumented_unsafe_blocks)]
//! (`undocumented_unsafe_blocks` is a RATCHET: this crate carries no `unsafe` block without a
//! `// SAFETY:` line stating why it is sound, and the lint keeps it that way. Every crate in the
//! workspace except `gizmo-core` is at zero and denies it; `gizmo-core`'s ECS internals are the
//! measured remainder — see docs/ENGINE.md.)
//! Gizmo Editor — an `egui`-based scene editor for the Gizmo engine.
//!
//! This crate provides the editor UI built on top of `egui`/`egui_dock`,
//! rendered with `wgpu`. Editor-wide state lives in
//! [`EditorState`](editor_state::EditorState). The whole UI is drawn each
//! frame by [`draw_editor`]. The window/egui runtime that hosts the editor
//! overlay (`EguiContext`) lives in the `gizmo-app` crate behind its `egui`
//! feature, so it can also back non-editor in-game HUDs.
//!
//! ## Panels
//! - **Toolbar** — top bar: Save/Load, Play/Pause, gizmo mode
//! - **Hierarchy** — left panel: entity tree
//! - **Inspector** — right panel: component editor
//! - **Asset Browser** — bottom panel: file browser
//! - **Scene View** — center panel: 3D scene viewport
//! - **Game View** — runtime/play viewport

/// The editor's visual design (palette, geometry, type scale).
/// The ANIMATION timeline panel.
pub mod animation_panel;
pub mod theme;
pub mod asset_browser;
pub mod console;
pub mod editor_state;
pub mod error;
pub mod game_view;
pub mod hierarchy;
pub mod history;
pub mod inspector;
pub mod prefs;
pub mod profiler_panel;
pub mod scene_view;
pub mod toolbar;
pub mod windows;

pub use editor_state::{BuildTarget, EditorMode, EditorState, EditorTab, GizmoMode, SpawnKind};
pub use error::EditorError;

use egui_dock::{DockArea, TabViewer};
use gizmo_core::World;

#[cfg(target_arch = "wasm32")]
use web_time::Instant;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

/// Bridges the dockable [`EditorTab`]s to their per-panel UI drawing code.
///
/// Implements [`egui_dock::TabViewer`] so [`egui_dock::DockArea`] can render
/// each tab using the shared [`World`] and mutable [`EditorState`].
pub struct EditorTabViewer<'a> {
    /// The ECS world the panels read entity/component data from.
    pub world: &'a World,
    /// Mutable editor-wide state shared across all panels.
    pub state: &'a mut EditorState,
}

impl<'a> TabViewer for EditorTabViewer<'a> {
    type Tab = EditorTab;

    /// A stable identity per tab, required by `egui_dock` 0.21.
    ///
    /// `EditorTab` is a fieldless enum that derives `Hash`, and the dock holds at most one of
    /// each variant, so hashing the variant is both unique and stable across frames — which is
    /// what the id is for. The alternative the crate suggests, hashing `title()`, would tie tab
    /// identity to a display string: renaming "Ayarlar" would silently reset that tab's state.
    fn id(&mut self, tab: &mut Self::Tab) -> egui::Id {
        egui::Id::new(tab)
    }

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        match tab {
            EditorTab::Hierarchy => "Hierarchy".into(),
            EditorTab::Inspector => "Inspector".into(),
            EditorTab::AssetBrowser => "Assets".into(),
            EditorTab::SceneView => "Scene".into(),
            EditorTab::GameView => "Game".into(),
            EditorTab::Console => "Console".into(),
            EditorTab::Settings => "Ayarlar".into(),
            EditorTab::ScriptEditor => "Script Editor".into(),
            EditorTab::Profiler => "Profiler".into(),
            EditorTab::Animation => "Animation".into(),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        match tab {
            EditorTab::Hierarchy => hierarchy::ui_hierarchy(ui, self.world, self.state),
            EditorTab::Inspector => inspector::ui_inspector(ui, self.world, self.state),
            EditorTab::AssetBrowser => asset_browser::ui_asset_browser(ui, self.state),
            EditorTab::SceneView => scene_view::ui_scene_view(ui, self.world, self.state),
            EditorTab::GameView => game_view::ui_game_view(ui, self.state),
            EditorTab::Console => console::ui_console(ui, self.state),
            EditorTab::Settings => windows::ui_settings_window(ui, self.state),
            EditorTab::ScriptEditor => windows::ui_script_editor(ui, self.state),
            EditorTab::Profiler => profiler_panel::ui_profiler(ui, self.world, self.state),
            EditorTab::Animation => animation_panel::ui_animation(ui, self.world, self.state),
        }
    }
}

/// Draws the whole editor (all panels and global shortcuts) for one frame.
///
/// Call this once per frame, passing a full-viewport root [`egui::Ui`] (built by
/// the host on the background layer — see `gizmo-studio`), the ECS [`World`], and
/// the mutable [`EditorState`]. The editor composes its panels into the root `Ui`
/// via `show_inside` (egui 0.34's root-`Ui` composition model).
pub fn draw_editor(ui: &mut egui::Ui, world: &World, state: &mut EditorState) {
    // Panel visibility is a per-FRAME fact, so it is cleared here and re-asserted by whichever
    // panels the dock actually draws below. Both flags existed before this and were only ever set
    // to `true` — never cleared, never read — so they said "this panel has been visible at least
    // once", which is not a question anyone was asking.
    state.scene_view_visible = false;
    state.game_view_visible = false;

    let ctx = ui.ctx().clone();
    // ==== Global Klavye Kısayolları (Sadece text alanları odakta değilken) ====
    if !ctx.egui_wants_keyboard_input() {
        ctx.input(|i| {
            if i.key_pressed(egui::Key::Q) { state.gizmo_mode = GizmoMode::Select; }
            if i.key_pressed(egui::Key::W) { state.gizmo_mode = GizmoMode::Translate; }
            if i.key_pressed(egui::Key::E) { state.gizmo_mode = GizmoMode::Rotate; }
            if i.key_pressed(egui::Key::R) { state.gizmo_mode = GizmoMode::Scale; }
            // Delete kısayolu shortcuts.rs'de işleniyor (BUG-11 düzeltmesi: çift tetikleme önlendi)
        });
    }

    // ==== Asenkron İletişim (Dialog vb.) Olay Döngüsü ====
    let msg = if let Some(rx) = &state.pending_dialog_rx {
        // Poison-recovery: mutex zehirlenmişse panik yerine iç değeri kurtar.
        match rx.lock().unwrap_or_else(|e| e.into_inner()).try_recv() {
            Ok(v) => Some(v),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some((false, None)),
            Err(_) => None,
        }
    } else {
        None
    };

    if let Some((is_save, opt_path)) = msg {
        if let Some(path_str) = opt_path {
            state.scene_path = path_str.clone();
            if is_save {
                state.status_message = format!("Sahne kaydediliyor → {}", path_str);
                state.scene.save_request = Some(path_str);
            } else {
                state.status_message = format!("Sahne yüklendi ← {}", path_str);
                state.scene.load_request = Some(path_str);
            }
        }
        state.pending_dialog_rx = None;
    }

    // ==== Ctrl+S ile tetiklenen kaydetme dialog isteği (shortcuts.rs'den gelir) ====
    if state.scene.request_save_dialog {
        state.scene.request_save_dialog = false;
        if state.pending_dialog_rx.is_none() {
            let (tx, rx) = std::sync::mpsc::channel();
            state.pending_dialog_rx = Some(std::sync::Mutex::new(rx));
            std::thread::spawn(move || {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let res = rfd::FileDialog::new()
                        .add_filter("Gizmo Scene", &["scene"])
                        .set_directory(".")
                        .save_file();
                    let _ = tx.send((
                        true,
                        res.map(|p: std::path::PathBuf| {
                            let s = p.to_string_lossy().to_string();
                            if let Some(stripped) = s.strip_prefix(r"\\?\") {
                                stripped.to_string()
                            } else {
                                s
                            }
                        }),
                    ));
                }
                #[cfg(target_arch = "wasm32")]
                let _ = tx.send((true, None));
            });
        }
    }

    // 1. Status Bar (En altta) — composed into the root `Ui`; `show_inside`
    // shrinks the root cursor from the bottom so the dock fills the remainder.
    egui::Panel::bottom("status_bar")
        .exact_size(23.0) // the prototype's bar
        .show(ui, |ui| {
            draw_status_bar(ui, world, state);
        });

    // 2. Toolbar (en üstte kalmaya devam etmeli, dock'un dışında)
    if state.show_toolbar {
        toolbar::draw_toolbar(ui, state);
    }

    // Kamera çizim durumları dock içerisinde güncellenecek, frame sonunda/başında başka yerde sıfırlanmalıdır veya flag kilitlenmelidir.

    // 2. Docking Alanı (Geri kalan tüm alanı kaplar)
    let mut dock_state =
        std::mem::replace(&mut state.dock_state, egui_dock::DockState::new(vec![]));

    let mut viewer = EditorTabViewer { world, state };

    // The dock frames every panel, so it was the loudest thing still off-palette: six hardcoded
    // cool greys and two blue separators in the middle of a warm scheme, predating `theme.rs`.
    let mut dock_style = egui_dock::Style::from_egui(ctx.global_style().as_ref());
    {
        use crate::theme::palette::*;
        dock_style.separator.width = 2.0; // the system's "strong 2px rules between sections"
        dock_style.separator.color_idle = BORDER;
        dock_style.separator.color_hovered = BORDER_HOT;
        dock_style.separator.color_dragged = ACCENT;

        dock_style.tab_bar.bg_fill = CHROME;
        dock_style.tab_bar.height = 25.0; // the prototype's tab bar
        dock_style.tab.active.bg_fill = SURFACE;
        dock_style.tab.inactive.bg_fill = CHROME;
        dock_style.tab.focused.bg_fill = SURFACE;
        dock_style.tab.hovered.bg_fill = SURFACE;
        dock_style.tab.active.text_color = TEXT_BRIGHT;
        dock_style.tab.inactive.text_color = TEXT_MUTED;
        dock_style.tab.focused.text_color = TEXT_BRIGHT;
        dock_style.tab.hovered.text_color = TEXT_BRIGHT;

        // Square, like everything else. `Style::from_egui` copies the global corner radius, which
        // this theme already zeroes — but the tab styles carry their own and would keep whatever
        // egui_dock's default is.
        for t in [
            &mut dock_style.tab.active,
            &mut dock_style.tab.inactive,
            &mut dock_style.tab.focused,
            &mut dock_style.tab.hovered,
        ] {
            t.corner_radius = egui::CornerRadius::ZERO;
        }
    }

    // Dock fills the central region left by the panels above (egui_dock's `show_inside`,
    // composed into the same root `Ui`).
    //
    // NOTE: this is `egui_dock`'s method, NOT egui's. egui 0.36 renamed `Panel::show_inside` to
    // `show`; `DockArea::show_inside` kept its name, and a blanket rename across the crate broke
    // exactly here.
    DockArea::new(&mut dock_state)
        .style(dock_style)
        .show_inside(ui, &mut viewer);

viewer.state.dock_state = dock_state;

    // Handle delayed tab opening safely outside the dock tree loop
    if state.script.open {
        state.open_tab(EditorTab::ScriptEditor);
        state.script.open = false;
    }

    // Her çerçevenin sonunda I/O optimizasyonu olarak prefs kirlendiyse dosyaya yaz
    if let Some(e) = state.prefs.flush_if_dirty() {
        state.log_error(&format!(
            "❌ Editör tercihleri kaydedilemedi: {} — ayarlarınız bu oturumdan sonra kalmayacak.",
            e
        ));
    }
}

/// The prototype's global status bar: a state dot and message on the left, live counts, memory,
/// and the frame cost on the right.
///
/// # What is on it, and what is still not
///
/// The prototype shows `RAM 1.8 GB · VRAM 742 MB`, `41 systems`, `wgpu / vulkan` and
/// `egui 0.29 · rustc 1.83`.
///
/// `RAM` is here now: it is this process's resident set size, sampled once a second. `VRAM` is
/// measured too — but it lives in the viewport's RENDER STATS as `gpu mem` and is deliberately not
/// repeated here, because the same number in two places is an invitation to work out why they
/// differ. (It is also *not* VRAM: it is what wgpu has sub-allocated for this process.)
///
/// The other two remain absent rather than plausible: the renderer fetches its adapter info at
/// startup, logs it and keeps nothing, and the schedule exposes no system count. A status bar that
/// prints a number it did not measure is worse than a shorter one. Listed in `docs/ENGINE.md` §3.
fn draw_status_bar(ui: &mut egui::Ui, world: &World, state: &mut EditorState) {
    use crate::theme::palette::*;

    let sep = |ui: &mut egui::Ui| {
        ui.label(egui::RichText::new("|").color(BORDER).size(11.0));
    };

    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = crate::theme::SPACE_2;

        // The state dot: accent while playing, neutral while editing. The prototype leads with it
        // because "am I in play mode" is the one piece of state that changes what every other
        // control does.
        let playing = state.is_playing() || state.mode == crate::editor_state::EditorMode::Paused;
        let (dot, _) = ui.allocate_exact_size(egui::vec2(7.0, 7.0), egui::Sense::hover());
        ui.painter()
            .rect_filled(dot, 0.0, if playing { ACCENT } else { BORDER_HOT });
        ui.label(egui::RichText::new(&state.status_message).color(TEXT_BRIGHT).size(11.0));

        sep(ui);
        let selected = state.selection.entities.len();
        let sel_label = match selected {
            0 => "nothing selected".to_string(),
            1 => {
                let names = world.borrow::<gizmo_core::EntityName>();
                let id = state.selection.entities.iter().next().map(|e| e.id());
                let name = id
                    .and_then(|id| names.get(id).map(|n| n.0.clone()))
                    .unwrap_or_else(|| format!("Entity {}", id.unwrap_or(0)));
                format!("{name} selected")
            }
            n => format!("{n} selected"),
        };
        ui.label(egui::RichText::new(sel_label).color(TEXT_BODY).size(11.0));

        sep(ui);
        ui.label(
            egui::RichText::new(format!("{} entities", world.iter_alive_entities().len()))
                .color(TEXT_BODY)
                .size(11.0),
        );

        // Resident set size, sampled at most once a second. It is a file read and a parse, and a
        // status bar does not need either of those sixty times a second — the same reasoning as
        // the `gpu mem` row, which walks every live GPU allocation.
        let now = Instant::now();
        let due = state
            .rss_sampled_at
            .is_none_or(|t| now.duration_since(t).as_secs_f32() >= RSS_SAMPLE_SECS);
        if due {
            state.rss_sampled_at = Some(now);
            state.rss_bytes = process_rss_bytes();
        }
        if let Some(bytes) = state.rss_bytes {
            sep(ui);
            ui.label(
                egui::RichText::new(format!("RAM {}", crate::theme::format_memory(bytes)))
                    .color(TEXT_BODY)
                    .size(11.0),
            );
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(p) = world.get_resource::<gizmo_core::FrameProfiler>() {
                ui.label(
                    egui::RichText::new(format!("{:.2} ms", p.avg_frame_ms(30)))
                        .color(TEXT_BODY)
                        .size(11.0),
                );
            }
        });
    });
}

/// How often the RAM figure is re-read.
const RSS_SAMPLE_SECS: f32 = 1.0;

/// This process's resident set size in bytes, or `None` where it cannot be measured.
///
/// Linux reads it out of `/proc/self/status`; everywhere else the row is **absent** rather than
/// zero, because this project's rule is that a panel must not print a number it did not measure
/// and `0 MB` is a number.
///
/// `VmRSS` rather than `/proc/self/statm`: statm reports a page *count*, which needs the page size
/// to become bytes, and that means either `libc` — a dependency for one `sysconf` call — or
/// assuming 4 KiB, which arm64 kernels are free to disagree with. `VmRSS` is already in kB.
///
/// No `sysinfo`-style crate either: those pull in a whole process enumerator to answer a question
/// about the one process we are already inside.
fn process_rss_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        parse_vm_rss(&std::fs::read_to_string("/proc/self/status").ok()?)
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// Pull `VmRSS` out of `/proc/self/status` and return it in bytes.
///
/// Split from the file read because the read cannot be tested and this can: the format is a kernel
/// detail, and a parser that silently returns the wrong unit is a status bar confidently reporting
/// a thousandth of the real figure.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_vm_rss(status: &str) -> Option<u64> {
    let rest = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))?;
    let mut fields = rest.split_whitespace();
    let value: u64 = fields.next()?.parse().ok()?;
    // The kernel always writes kB here. If that ever stops being true, report nothing rather than
    // guessing a scale — a status bar off by 1024 looks perfectly reasonable.
    match fields.next() {
        Some("kB") => Some(value * 1024),
        _ => None,
    }
}

#[cfg(test)]
mod status_bar_tests {
    use super::parse_vm_rss;

    /// The real shape of the line, tabs and all.
    #[test]
    fn vm_rss_is_read_in_kilobytes_and_returned_in_bytes() {
        let status = "Name:\tgizmo-studio\nVmPeak:\t 9999999 kB\nVmRSS:\t  1843200 kB\nThreads:\t9\n";
        assert_eq!(parse_vm_rss(status), Some(1_843_200 * 1024));
    }

    /// The real file surrounds `VmRSS` with lines a looser match would take instead — `VmHWM` is
    /// the *peak* and `RssAnon` / `RssFile` / `RssShmem` are its components. Any of them reported
    /// as the current figure looks entirely plausible on a status bar.
    ///
    /// The fixture is the real neighbourhood, in the real order, and `VmRSS` is deliberately NOT
    /// first: a parser that took whichever Rss-ish line it met first would pass a tidier fixture.
    #[test]
    fn only_the_vm_rss_line_counts() {
        let status = "\
RssAnon:\t   111111 kB
RssFile:\t   222222 kB
RssShmem:\t     3333 kB
VmHWM:\t   9000000 kB
VmRSS:\t    512000 kB
VmData:\t   777777 kB
";
        assert_eq!(
            parse_vm_rss(status),
            Some(512_000 * 1024),
            "neither the peak nor one of the Rss components — the VmRSS line"
        );
        // ...and a file without it at all reports nothing rather than zero.
        assert_eq!(parse_vm_rss("RssAnon:\t 111111 kB\nVmHWM:\t 9000000 kB\n"), None);
    }

    /// A unit the kernel does not currently write means the format changed under us. Report
    /// nothing: a status bar off by a factor of 1024 looks perfectly reasonable, which is exactly
    /// what makes it dangerous.
    #[test]
    fn an_unexpected_unit_reports_nothing_rather_than_guessing() {
        assert_eq!(parse_vm_rss("VmRSS:\t  1800 MB\n"), None);
        assert_eq!(parse_vm_rss("VmRSS:\t  1800\n"), None, "no unit at all");
        assert_eq!(parse_vm_rss("VmRSS:\thuge kB\n"), None, "not a number");
        assert_eq!(parse_vm_rss(""), None);
    }

    /// On the machine this runs on, the reading must actually work — a parser that is only ever
    /// exercised against hand-written fixtures is a parser that has never met the kernel.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_real_proc_status_parses_to_something_plausible() {
        let bytes = super::process_rss_bytes().expect("Linux must be able to read its own RSS");
        // A test binary that has loaded this crate is somewhere between a megabyte and a terabyte.
        assert!(
            bytes > 1024 * 1024 && bytes < 1024_u64.pow(4),
            "implausible RSS: {bytes} bytes"
        );
    }
}

#[cfg(test)]
mod panel_visibility_tests {
    use super::*;

    /// Panel visibility must describe *this* frame, not any frame since startup.
    ///
    /// The studio skips an entire extra scene render when `game_view_visible` is false, so the flag
    /// has to be recomputed per frame: cleared at the top of `draw_editor`, set again by whichever
    /// viewport actually drew. Drop either half and the flag degrades into "has been visible once",
    /// which latches true and quietly hands the saving back.
    ///
    /// Scene and Game are tabs of one dock leaf, so exactly one of them is on top at a time. The
    /// test brings each to the front in turn and demands the pair of flags follow — and it starts
    /// each frame with both flags at the *opposite* of the expected answer, so no assertion can
    /// pass on leftover state. Every one of the four has to be written during its frame.
    ///
    /// `Context::run_ui` drives a real frame with no window and no GPU. The viewports paint nothing
    /// without a texture, but the dock still decides who is on top, which is the part under test.
    #[test]
    fn a_frame_recomputes_which_viewport_is_visible() {
        let world = World::new();
        let ctx = egui::Context::default();

        for (front, tab) in [("Scene", EditorTab::SceneView), ("Game", EditorTab::GameView)] {
            let want_scene = tab == EditorTab::SceneView;
            let mut state = EditorState {
                // Not whatever `editor_layout.json` happens to be in the working directory.
                dock_state: editor_state::create_default_dock_state(),
                scene_view_visible: !want_scene,
                game_view_visible: want_scene,
                ..Default::default()
            };
            let found = state.dock_state.find_tab(&tab).expect("tab is in the default layout");
            state
                .dock_state
                .set_active_tab(found)
                .expect("the tab the layout just handed back is still addressable");

            let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                draw_editor(ui, &world, &mut state);
            });
            output.drop_without_applying_deltas();

            assert_eq!(
                (state.scene_view_visible, state.game_view_visible),
                (want_scene, !want_scene),
                "with the {front} tab in front, the visibility flags read \
                 (scene={}, game={}) — the studio spends a full extra scene render on \
                 game_view_visible and skips the render a visible viewport needs, so a flag that \
                 describes some earlier frame costs real frame time either way",
                state.scene_view_visible,
                state.game_view_visible,
            );
        }
    }

    /// The Game panel shows the texture it was handed, in every mode that has one.
    ///
    /// This is the other half of the same bargain as
    /// [`a_frame_recomputes_which_viewport_is_visible`], and it was broken in the opposite
    /// direction: the studio renders the game camera into its own target on every frame it is *not*
    /// playing, and the panel used to display that target only *while* playing. Exact opposites, so
    /// a whole live-preview feature rendered every frame into a texture nobody ever saw.
    ///
    /// The frame runs in Edit mode — the mode the old condition rejected — and the assertion
    /// looks for the texture in the shapes the frame actually emitted, not for the state that was
    /// supposed to lead there.
    #[test]
    fn the_game_panel_paints_the_texture_it_was_given() {
        /// Shapes nest: a `Shape::Vec` holds more shapes, so a flat scan would miss the mesh.
        fn paints(shape: &egui::Shape, id: egui::TextureId) -> bool {
            match shape {
                egui::Shape::Mesh(mesh) => mesh.texture_id == id,
                egui::Shape::Vec(shapes) => shapes.iter().any(|s| paints(s, id)),
                _ => false,
            }
        }

        let target = egui::TextureId::User(0x6112_0000);
        let world = World::new();
        let mut state = EditorState {
            dock_state: editor_state::create_default_dock_state(),
            mode: EditorMode::Edit,
            game_texture_id: Some(target),
            ..Default::default()
        };
        let found = state
            .dock_state
            .find_tab(&EditorTab::GameView)
            .expect("the Game tab is in the default layout");
        state
            .dock_state
            .set_active_tab(found)
            .expect("the tab the layout just handed back is still addressable");

        let ctx = egui::Context::default();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            draw_editor(ui, &world, &mut state);
        });
        let painted = output.shapes.iter().any(|s| paints(&s.shape, target));
        output.drop_without_applying_deltas();

        assert!(
            painted,
            "the Game panel had a texture and did not paint it — while not playing, the studio \
             renders the game camera into exactly this target every frame, so a panel that refuses \
             to show it outside play mode throws the whole preview away and charges for it too"
        );
    }
}


