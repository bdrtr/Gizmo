//! The profiler panel — visual performance monitoring.
//!
//! Visualises FrameProfiler's data with egui:
//! - A frame-time graph (the last 300 frames)
//! - An FPS counter
//! - A per-scope timing table (a mini flamegraph)
//! - Budget bars (16.6 ms = the 60 fps target)

use crate::editor_state::EditorState;
use egui;
use gizmo_core::World;

/// Profiler panelinin renk paleti
use crate::theme::palette;

/// The graph's well — the deepest surface, so the bars read as sitting in a trough.
const COLOR_BG_BAR: egui::Color32 = palette::VOID;

/// Under budget: neutral. A frame that meets its budget is unremarkable, and painting it green
/// spends the loudest colour in the panel on the case you never need to look at.
const COLOR_GOOD: egui::Color32 = palette::BORDER_HOT;
/// Over 60 fps budget.
const COLOR_WARN: egui::Color32 = palette::ACCENT_LIGHT;
/// Over 30 fps budget.
const COLOR_BAD: egui::Color32 = palette::ACCENT;

/// The colour for a frame time.
fn frame_color(ms: f64) -> egui::Color32 {
    if ms < 16.67 {
        COLOR_GOOD
    } else if ms < 33.33 {
        COLOR_WARN
    } else {
        COLOR_BAD
    }
}

/// The colour palette, by scope depth.
///
/// Deliberately NOT folded into the mono accent scheme. The design system says one accent and no
/// second, which is right for chrome — but a flamegraph's colours are data, not decoration: eight
/// shades of one hue would make adjacent scopes indistinguishable, which is the only thing this
/// table is for. The rule is about the interface, and this is a chart.
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

/// Draws the profiler panel.
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
        painter.rect_filled(rect, 0.0, COLOR_BG_BAR);

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
            // The budget line, in the accent at low alpha: it is a threshold, not a value, so it
            // has to be legible without competing with the bars that cross it.
            egui::Stroke::new(1.0_f32, palette::ACCENT.gamma_multiply(0.45)),
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
            "16.6 ms",
            egui::FontId::proportional(9.0),
            palette::TEXT_DIM,
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

#[cfg(test)]
mod tests {
    use super::*;

    // ─── frame_color: eşik/sınır davranışı (60fps=16.67ms, 30fps=33.33ms) ───

    #[test]
    fn frame_color_below_60fps_budget_is_good() {
        assert_eq!(frame_color(0.0), COLOR_GOOD);
        assert_eq!(frame_color(8.0), COLOR_GOOD);
        assert_eq!(frame_color(16.66), COLOR_GOOD);
    }

    /// The boundary: exactly 16.67 ms is no longer "good" (`< 16.67` is false) → warning.
    #[test]
    fn frame_color_at_60fps_boundary_is_warn() {
        assert_eq!(frame_color(16.67), COLOR_WARN);
    }

    #[test]
    fn frame_color_between_budgets_is_warn() {
        assert_eq!(frame_color(20.0), COLOR_WARN);
        assert_eq!(frame_color(33.32), COLOR_WARN);
    }

    /// The boundary: exactly 33.33 ms is no longer "warning" (`< 33.33` is false) → bad.
    #[test]
    fn frame_color_at_30fps_boundary_is_bad() {
        assert_eq!(frame_color(33.33), COLOR_BAD);
        assert_eq!(frame_color(100.0), COLOR_BAD);
    }

    /// A negative duration (meaningless, but handled defensively) still lands on the "good"
    /// side.
    #[test]
    fn frame_color_negative_is_good() {
        assert_eq!(frame_color(-5.0), COLOR_GOOD);
    }

    /// NaN and ∞ are false in both `<` comparisons → bad (this documents that behaviour).
    #[test]
    fn frame_color_nan_and_inf_fall_through_to_bad() {
        assert_eq!(frame_color(f64::NAN), COLOR_BAD);
        assert_eq!(frame_color(f64::INFINITY), COLOR_BAD);
    }

    // ─── scope_color: (depth*3 + idx) % 8 palet indeksleme ───

    /// The palette has 8 entries → the index wraps with a period of 8.
    #[test]
    fn scope_color_wraps_modulo_palette_len() {
        assert_eq!(scope_color(0, 0), scope_color(0, 8));
        assert_eq!(scope_color(0, 0), scope_color(0, 16));
    }

    /// Depth adds an offset of 3 to the index: scope_color(d, i) == scope_color(0, d*3 + i).
    #[test]
    fn scope_color_depth_offsets_index_by_three() {
        assert_eq!(scope_color(1, 0), scope_color(0, 3));
        assert_eq!(scope_color(2, 1), scope_color(0, 7));
        // 2*3 + 2 = 8 ≡ 0 (mod 8)
        assert_eq!(scope_color(2, 2), scope_color(0, 0));
    }

    /// The first 8 indices (i=0..8, depth=0) must give colours that differ from EACH OTHER.
    #[test]
    fn scope_color_first_eight_indices_are_distinct() {
        let colors: Vec<_> = (0..8).map(|i| scope_color(0, i)).collect();
        for i in 0..colors.len() {
            for j in (i + 1)..colors.len() {
                assert_ne!(
                    colors[i], colors[j],
                    "indeks {} ve {} aynı renge çözümlendi",
                    i, j
                );
            }
        }
    }

    /// Determinism: the same input always gives the same colour.
    #[test]
    fn scope_color_is_deterministic() {
        assert_eq!(scope_color(3, 5), scope_color(3, 5));
    }
}
