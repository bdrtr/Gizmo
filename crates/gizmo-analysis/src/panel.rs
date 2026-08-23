//! The live analysis panel (the `egui` feature).
//!
//! Visualises the `Analyzer` with egui: FPS plus frame-time percentiles and a graph, the ECS
//! archetype/component/memory table, this frame's nested span bars, and a table of every metric's
//! statistics with a mini sparkline. It complements the editor's `profiler_panel` (which only
//! looks at the FrameProfiler; this shows all of the Analyzer's data).
//!
//! How to call it (inside the root `Ui` you build in the app's `set_ui` closure):
//! ```
//! # let ctx = egui::Context::default();
//! # let mut world = gizmo_core::world::World::new();
//! # world.insert_resource(gizmo_analysis::Analyzer::new());
//! # let out = ctx.run_ui(egui::RawInput::default(), |ui| {
//! egui::Panel::right("analysis").show(ui, |ui| {
//!     gizmo_analysis::panel::analysis_ui_world(ui, &world);
//! });
//! # });
//! # assert!(!out.shapes.is_empty(), "the panel always draws something");
//! # // REQUIRED: `TexturesDelta` panics if it is merely dropped — egui hands back deltas it
//! # // expects the caller to apply. Without this the doctest fails with
//! # // "Dropped TexturesDelta with 1 unapplied deltas".
//! # out.drop_without_applying_deltas();
//! ```

use crate::analyzer::Analyzer;
use crate::util::human_bytes;
use gizmo_core::world::short_type_name;

const COLOR_BG_BAR: egui::Color32 = egui::Color32::from_rgb(40, 40, 45);
const COLOR_GOOD: egui::Color32 = egui::Color32::from_rgb(80, 200, 120);
const COLOR_WARN: egui::Color32 = egui::Color32::from_rgb(240, 180, 50);
const COLOR_BAD: egui::Color32 = egui::Color32::from_rgb(220, 60, 60);
const COLOR_ACCENT: egui::Color32 = egui::Color32::from_rgb(120, 170, 255);

fn frame_color(ms: f64) -> egui::Color32 {
    if ms < 16.67 {
        COLOR_GOOD
    } else if ms < 33.33 {
        COLOR_WARN
    } else {
        COLOR_BAD
    }
}

fn scope_color(depth: u32, idx: usize) -> egui::Color32 {
    const PALETTE: &[egui::Color32] = &[
        egui::Color32::from_rgb(86, 156, 214),
        egui::Color32::from_rgb(78, 201, 176),
        egui::Color32::from_rgb(220, 220, 170),
        egui::Color32::from_rgb(206, 145, 120),
        egui::Color32::from_rgb(181, 137, 214),
        egui::Color32::from_rgb(215, 186, 125),
        egui::Color32::from_rgb(156, 220, 254),
        egui::Color32::from_rgb(244, 135, 113),
    ];
    PALETTE[(depth as usize * 3 + idx) % PALETTE.len()]
}

/// Pulls the `Analyzer` resource out of the World and draws the panel (a convenience).
pub fn analysis_ui_world(ui: &mut egui::Ui, world: &gizmo_core::world::World) {
    match world.get_resource::<Analyzer>() {
        Some(a) => analysis_ui(ui, &a),
        None => {
            ui.colored_label(COLOR_WARN, "⚠ Analyzer resource bulunamadı.");
        }
    }
}

/// Opens the panel as an egui window (closable).
pub fn analysis_window(ctx: &egui::Context, open: &mut bool, analyzer: &Analyzer) {
    egui::Window::new("🔬 Gizmo Analysis")
        .open(open)
        .default_width(440.0)
        .show(ctx, |ui| analysis_ui(ui, analyzer));
}

/// Draws the analysis panel into the given `Ui`.
pub fn analysis_ui(ui: &mut egui::Ui, analyzer: &Analyzer) {
    header(ui, analyzer);
    ui.add_space(4.0);
    frame_graph(ui, analyzer);
    ui.add_space(6.0);
    ecs_section(ui, analyzer);
    ui.add_space(6.0);
    spans_section(ui, analyzer);
    ui.add_space(6.0);
    metrics_section(ui, analyzer);
}

fn header(ui: &mut egui::Ui, analyzer: &Analyzer) {
    let fps = analyzer.estimated_fps();
    let fs = analyzer.stats("frame_ms");
    let mean = fs.map(|s| s.mean).unwrap_or(0.0);
    let color = frame_color(mean);

    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("⚡ {fps:.0} FPS"))
                .strong()
                .size(18.0)
                .color(color),
        );
        ui.separator();
        ui.label(
            egui::RichText::new(format!("{mean:.2} ms"))
                .size(14.0)
                .color(color),
        );
        ui.separator();
        ui.label(
            egui::RichText::new(format!("frame #{}", analyzer.frame()))
                .weak()
                .small(),
        );
    });

    if let Some(s) = fs {
        ui.label(
            egui::RichText::new(format!(
                "min {:.2} · p50 {:.2} · p95 {:.2} · p99 {:.2} · max {:.2}  (n={})",
                s.min, s.p50, s.p95, s.p99, s.max, s.count
            ))
            .small()
            .monospace()
            .weak(),
        );
    }
}

fn frame_graph(ui: &mut egui::Ui, analyzer: &Analyzer) {
    ui.label(egui::RichText::new("Frame Süresi (ms)").strong());
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 70.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, COLOR_BG_BAR);

    let frames: Vec<f64> = analyzer.history().map(|f| f.frame_ms).collect();
    if frames.is_empty() {
        return;
    }
    let max_ms = 33.33f64;
    // 60fps hedef çizgisi.
    let y_16 = rect.top() + (1.0 - 16.67 / max_ms) as f32 * rect.height();
    painter.line_segment(
        [egui::pos2(rect.left(), y_16), egui::pos2(rect.right(), y_16)],
        egui::Stroke::new(1.0f32, egui::Color32::from_rgba_premultiplied(80, 200, 120, 70)),
    );

    let bar_count = frames.len().min(width.max(1.0) as usize);
    let bar_w = rect.width() / bar_count as f32;
    for (i, ms) in frames.iter().rev().take(bar_count).enumerate() {
        let x = rect.right() - (i as f32 + 1.0) * bar_w;
        let h = (ms / max_ms).min(1.0) as f32 * rect.height();
        painter.rect_filled(
            egui::Rect::from_min_size(egui::pos2(x, rect.bottom() - h), egui::vec2(bar_w - 1.0, h)),
            0.0,
            frame_color(*ms),
        );
    }
    painter.text(
        egui::pos2(rect.left() + 4.0, y_16 - 2.0),
        egui::Align2::LEFT_BOTTOM,
        "60fps",
        egui::FontId::proportional(10.0),
        COLOR_GOOD,
    );
}

fn ecs_section(ui: &mut egui::Ui, analyzer: &Analyzer) {
    let Some(last) = analyzer.last() else {
        return;
    };
    let e = &last.ecs;
    ui.label(egui::RichText::new("ECS").strong());
    ui.label(
        egui::RichText::new(format!(
            "{} entity · {} archetype ({} dolu) · {} component tipi · {} resource · {}",
            e.entities,
            e.archetypes,
            e.non_empty_archetypes,
            e.registered_components,
            e.resources,
            human_bytes(e.component_bytes),
        ))
        .small(),
    );

    if last.archetypes.is_empty() {
        return;
    }
    egui::CollapsingHeader::new(format!("Archetype tablosu ({})", last.archetypes.len()))
        .default_open(false)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .max_height(180.0)
                .id_salt("gizmo_analysis_arch")
                .show(ui, |ui| {
                    egui::Grid::new("gizmo_analysis_arch_grid")
                        .striped(true)
                        .min_col_width(48.0)
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new("#").strong().small());
                            ui.label(egui::RichText::new("entity").strong().small());
                            ui.label(egui::RichText::new("bellek").strong().small());
                            ui.label(egui::RichText::new("component'ler").strong().small());
                            ui.end_row();
                            for a in &last.archetypes {
                                ui.label(egui::RichText::new(format!("{}", a.id)).small().monospace());
                                ui.label(
                                    egui::RichText::new(format!("{}", a.entity_count))
                                        .small()
                                        .monospace()
                                        .color(COLOR_ACCENT),
                                );
                                ui.label(
                                    egui::RichText::new(human_bytes(a.bytes)).small().monospace(),
                                );
                                let names: Vec<&str> =
                                    a.components.iter().map(|c| short_type_name(c.name)).collect();
                                ui.label(egui::RichText::new(names.join(", ")).small());
                                ui.end_row();
                            }
                        });
                });
        });
}

fn spans_section(ui: &mut egui::Ui, analyzer: &Analyzer) {
    let Some(last) = analyzer.last() else {
        return;
    };
    if last.spans.is_empty() {
        return;
    }
    ui.label(egui::RichText::new("Span'ler (bu frame)").strong());
    let total = last.frame_ms.max(0.001);
    egui::ScrollArea::vertical()
        .max_height(160.0)
        .id_salt("gizmo_analysis_spans")
        .show(ui, |ui| {
            egui::Grid::new("gizmo_analysis_span_grid")
                .striped(true)
                .min_col_width(50.0)
                .show(ui, |ui| {
                    for (idx, sp) in last.spans.iter().enumerate() {
                        let color = scope_color(sp.depth, idx);
                        let indent = "  ".repeat(sp.depth as usize);
                        ui.label(
                            egui::RichText::new(format!("{indent}▪ {}", short_type_name(sp.name)))
                                .color(color)
                                .small(),
                        );
                        ui.label(
                            egui::RichText::new(format!("{:.3} ms", sp.ms))
                                .small()
                                .monospace()
                                .color(if sp.ms > 5.0 { COLOR_BAD } else { color }),
                        );
                        let frac = (sp.ms / total).min(1.0) as f32;
                        let (r, _) =
                            ui.allocate_exact_size(egui::vec2(120.0, 12.0), egui::Sense::hover());
                        let p = ui.painter_at(r);
                        p.rect_filled(r, 2.0, COLOR_BG_BAR);
                        p.rect_filled(
                            egui::Rect::from_min_size(
                                r.left_top(),
                                egui::vec2(r.width() * frac, r.height()),
                            ),
                            2.0,
                            color,
                        );
                        ui.end_row();
                    }
                });
        });
}

fn metrics_section(ui: &mut egui::Ui, analyzer: &Analyzer) {
    let store = analyzer.metrics();
    if store.is_empty() {
        return;
    }
    egui::CollapsingHeader::new(format!("Metrikler ({})", store.len()))
        .default_open(true)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .max_height(240.0)
                .id_salt("gizmo_analysis_metrics")
                .show(ui, |ui| {
                    egui::Grid::new("gizmo_analysis_metric_grid")
                        .striped(true)
                        .min_col_width(44.0)
                        .show(ui, |ui| {
                            for header in ["metrik", "son", "ort", "p95", "max", "trend"] {
                                ui.label(egui::RichText::new(header).strong().small());
                            }
                            ui.end_row();

                            for (name, series) in store.iter() {
                                let s = series.stats();
                                ui.label(egui::RichText::new(name).small());
                                ui.label(
                                    egui::RichText::new(format!("{:.2}", s.last))
                                        .small()
                                        .monospace()
                                        .color(COLOR_ACCENT),
                                );
                                ui.label(egui::RichText::new(format!("{:.2}", s.mean)).small().monospace());
                                ui.label(egui::RichText::new(format!("{:.2}", s.p95)).small().monospace());
                                ui.label(egui::RichText::new(format!("{:.2}", s.max)).small().monospace());
                                let values: Vec<f64> = series.values().collect();
                                sparkline(ui, &values, 90.0, 14.0, COLOR_ACCENT);
                                ui.end_row();
                            }
                        });
                });
        });
}

/// A small line chart — draws a metric's ring history.
fn sparkline(ui: &mut egui::Ui, values: &[f64], w: f32, h: f32, color: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 2.0, COLOR_BG_BAR);
    if values.len() < 2 {
        return;
    }
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for &v in values {
        if v.is_finite() {
            lo = lo.min(v);
            hi = hi.max(v);
        }
    }
    if !lo.is_finite() || !hi.is_finite() {
        return;
    }
    let span = (hi - lo).max(1e-9);
    let n = values.len();
    let pts: Vec<egui::Pos2> = values
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let x = rect.left() + rect.width() * (i as f32 / (n - 1) as f32);
            let norm = ((v - lo) / span) as f32;
            egui::pos2(x, rect.bottom() - norm * rect.height())
        })
        .collect();
    p.add(egui::Shape::line(pts, egui::Stroke::new(1.2f32, color)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Analyzer;

    // This used to call `Context::run` behind an `#[allow(deprecated)]`, with a note saying the
    // egui API migration was "a separate maintenance pass". egui 0.36 removed the method, so the
    // pass happened here. `run_ui` hands the closure a `&mut Ui` directly — the central panel the
    // old form needed was only there to turn a `&Context` into one, so it goes with it.
    fn run_panel(analyzer: &Analyzer) {
        let ctx = egui::Context::default();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| analysis_ui(ui, analyzer));
        // `FullOutput` is `#[must_use]` and carries texture deltas a real app would upload;
        // a headless test has no GPU to give them to, and dropping them silently would leak
        // the atlas allocation the context just made.
        output.drop_without_applying_deltas();
    }

    #[test]
    fn panel_builds_when_empty() {
        // Hiç frame toplanmadan da panik atmamalı.
        run_panel(&Analyzer::new());
    }

    #[test]
    fn panel_builds_with_data() {
        let mut world = gizmo_core::world::World::new();
        world.insert_resource(crate::FrameProfiler::new());
        let mut analyzer = Analyzer::new();
        analyzer.gauge("demo", 1.0);
        if let Some(mut p) = world.get_resource_mut::<crate::FrameProfiler>() {
            p.begin_scope("frame");
            p.end_scope("frame");
            p.end_frame();
        }
        analyzer.collect(&world);
        run_panel(&analyzer);
    }
}
