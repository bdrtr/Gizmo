//! Profiler Panel — Görsel performans izleme paneli
//!
//! FrameProfiler'daki verileri egui ile görselleştirir:
//! - Frame time grafiği (son 300 frame)
//! - FPS sayacı
//! - Scope bazlı zamanlama tablosu (mini flamegraph)
//! - Bütçe çubukları (16.6ms = 60fps hedef)

use crate::editor_state::EditorState;
use egui;
use gizmo_core::World;

/// Profiler panelinin renk paleti
const COLOR_BG_BAR: egui::Color32 = egui::Color32::from_rgb(40, 40, 45);
const COLOR_GOOD: egui::Color32 = egui::Color32::from_rgb(80, 200, 120);
const COLOR_WARN: egui::Color32 = egui::Color32::from_rgb(240, 180, 50);
const COLOR_BAD: egui::Color32 = egui::Color32::from_rgb(220, 60, 60);

/// Frame süresine göre renk döndürür
fn frame_color(ms: f64) -> egui::Color32 {
    if ms < 16.67 {
        COLOR_GOOD
    } else if ms < 33.33 {
        COLOR_WARN
    } else {
        COLOR_BAD
    }
}

/// Scope derinliğine göre renk paleti
fn scope_color(depth: u32, idx: usize) -> egui::Color32 {
    const PALETTE: &[egui::Color32] = &[
        egui::Color32::from_rgb(86, 156, 214),  // Mavi
        egui::Color32::from_rgb(78, 201, 176),  // Turkuaz
        egui::Color32::from_rgb(220, 220, 170), // Sarı
        egui::Color32::from_rgb(206, 145, 120), // Turuncu
        egui::Color32::from_rgb(181, 137, 214), // Mor
        egui::Color32::from_rgb(215, 186, 125), // Altın
        egui::Color32::from_rgb(156, 220, 254), // Açık mavi
        egui::Color32::from_rgb(244, 135, 113), // Mercan
    ];
    let i = (depth as usize * 3 + idx) % PALETTE.len();
    PALETTE[i]
}

/// Profiler panelini çizer
pub fn ui_profiler(ui: &mut egui::Ui, world: &World, _state: &mut EditorState) {
    let profiler = match world.get_resource::<gizmo_core::FrameProfiler>() {
        Some(p) => p,
        None => {
            ui.colored_label(COLOR_WARN, "⚠ FrameProfiler resource bulunamadı.");
            return;
        }
    };

    let fps = profiler.estimated_fps();
    let avg_ms = profiler.avg_frame_ms(60);
    let history = profiler.history();

    // ──────────────── HEADER ────────────────
    ui.horizontal(|ui| {
        let fps_color = frame_color(avg_ms);
        ui.label(
            egui::RichText::new(format!("⚡ {:.0} FPS", fps))
                .strong()
                .size(18.0)
                .color(fps_color),
        );
        ui.separator();
        ui.label(
            egui::RichText::new(format!("{:.2}ms", avg_ms))
                .size(14.0)
                .color(fps_color),
        );
        ui.separator();
        ui.label(
            egui::RichText::new(format!("Frame #{}", profiler.frame_count()))
                .weak()
                .small(),
        );
    });

    ui.add_space(4.0);

    // ──────────────── FRAME TIME GRAFİĞİ ────────────────
    ui.label(egui::RichText::new("Frame Süresi").strong());

    let available_width = ui.available_width();
    let graph_height = 80.0;

    let (rect, _response) = ui.allocate_exact_size(
        egui::vec2(available_width, graph_height),
        egui::Sense::hover(),
    );

    if !history.is_empty() {
        let painter = ui.painter_at(rect);

        // Arka plan
        painter.rect_filled(rect, 4.0, COLOR_BG_BAR);

        // Hedef çizgileri
        let max_ms = 33.33f64; // Y ekseni max
        let y_16ms = rect.top() + (1.0 - 16.67 / max_ms) as f32 * rect.height();
        let y_33ms = rect.top();

        // 60fps hedef çizgisi
        painter.line_segment(
            [
                egui::pos2(rect.left(), y_16ms),
                egui::pos2(rect.right(), y_16ms),
            ],
            egui::Stroke::new(
                1.0,
                egui::Color32::from_rgba_premultiplied(80, 200, 120, 60),
            ),
        );

        // Çubuklar
        let bar_count = history.len().min(available_width as usize);
        let bar_width = rect.width() / bar_count as f32;

        for (i, profile) in history.iter().rev().take(bar_count).enumerate() {
            let x = rect.right() - (i as f32 + 1.0) * bar_width;
            let h = (profile.total_ms / max_ms).min(1.0) as f32 * rect.height();
            let bar_rect = egui::Rect::from_min_size(
                egui::pos2(x, rect.bottom() - h),
                egui::vec2(bar_width - 1.0, h),
            );
            painter.rect_filled(bar_rect, 0.0, frame_color(profile.total_ms));
        }

        // Etiketler
        painter.text(
            egui::pos2(rect.left() + 4.0, y_16ms - 12.0),
            egui::Align2::LEFT_BOTTOM,
            "60fps",
            egui::FontId::proportional(10.0),
            egui::Color32::from_rgb(80, 200, 120),
        );
        let _ = y_33ms; // suppress unused
    }

    ui.add_space(6.0);

    // ──────────────── BÜTÇE ÇUBUKLARI ────────────────
    ui.label(egui::RichText::new("Frame Bütçesi").strong());

    let budget_16 = (avg_ms / 16.67).min(2.0) as f32;
    let budget_rect_width = available_width * 0.7;

    ui.horizontal(|ui| {
        ui.label("60fps:");
        let (bar_rect, _) =
            ui.allocate_exact_size(egui::vec2(budget_rect_width, 16.0), egui::Sense::hover());
        let painter = ui.painter_at(bar_rect);
        painter.rect_filled(bar_rect, 3.0, COLOR_BG_BAR);
        let fill_w = (budget_16 * 0.5 * bar_rect.width()).min(bar_rect.width());
        let fill_rect =
            egui::Rect::from_min_size(bar_rect.left_top(), egui::vec2(fill_w, bar_rect.height()));
        painter.rect_filled(fill_rect, 3.0, frame_color(avg_ms));
        painter.text(
            bar_rect.center(),
            egui::Align2::CENTER_CENTER,
            format!("{:.0}%", budget_16 * 50.0),
            egui::FontId::proportional(11.0),
            egui::Color32::WHITE,
        );
    });

    ui.add_space(6.0);

    // ──────────────── SCOPE TABLOSU (Mini Flamegraph) ────────────────
    if let Some(last) = profiler.last_frame() {
        if !last.scopes.is_empty() {
            ui.label(egui::RichText::new("Scope Zamanlamaları").strong());

            egui::ScrollArea::vertical()
                .max_height(200.0)
                .show(ui, |ui| {
                    egui::Grid::new("profiler_scope_grid")
                        .striped(true)
                        .min_col_width(60.0)
                        .show(ui, |ui| {
                            // Başlık
                            ui.label(egui::RichText::new("Scope").strong().small());
                            ui.label(egui::RichText::new("Süre").strong().small());
                            ui.label(egui::RichText::new("Grafik").strong().small());
                            ui.end_row();

                            // Her scope'u depth'e göre indent ederek göster
                            for (idx, scope) in last.scopes.iter().enumerate() {
                                let indent = "  ".repeat(scope.depth as usize);
                                let color = scope_color(scope.depth, idx);

                                ui.label(
                                    egui::RichText::new(format!("{}▪ {}", indent, scope.name))
                                        .color(color)
                                        .small(),
                                );

                                let ms = scope.duration_ms();
                                ui.label(
                                    egui::RichText::new(format!("{:.3}ms", ms))
                                        .color(if ms > 5.0 { COLOR_BAD } else { color })
                                        .small()
                                        .monospace(),
                                );

                                // Mini çubuk
                                let bar_frac = (ms / last.total_ms.max(0.1)).min(1.0) as f32;
                                let (bar_r, _) = ui.allocate_exact_size(
                                    egui::vec2(120.0, 12.0),
                                    egui::Sense::hover(),
                                );
                                let p = ui.painter_at(bar_r);
                                p.rect_filled(bar_r, 2.0, COLOR_BG_BAR);
                                p.rect_filled(
                                    egui::Rect::from_min_size(
                                        bar_r.left_top(),
                                        egui::vec2(bar_r.width() * bar_frac, bar_r.height()),
                                    ),
                                    2.0,
                                    color,
                                );

                                ui.end_row();
                            }
                        });
                });
        }
    }

    // ──────────────── SCOPE ORTALAMALARI ────────────────
    ui.add_space(6.0);
    ui.label(egui::RichText::new("Ortalama Zamanlamalar (60 frame)").strong());

    let known_scopes = [
        "ecs_update",
        "pre_update",
        "update",
        "physics",
        "post_update",
        "render",
        "broadphase",
        "narrowphase",
        "solver",
        "integrate",
    ];

    egui::Grid::new("profiler_avg_grid")
        .striped(true)
        .show(ui, |ui| {
            for &scope_name in &known_scopes {
                let avg = profiler.avg_scope_ms(scope_name, 60);
                if avg > 0.001 {
                    ui.label(egui::RichText::new(scope_name).small());
                    ui.label(
                        egui::RichText::new(format!("{:.3}ms", avg))
                            .small()
                            .monospace()
                            .color(if avg > 5.0 { COLOR_BAD } else { COLOR_GOOD }),
                    );
                    ui.end_row();
                }
            }
        });
}
