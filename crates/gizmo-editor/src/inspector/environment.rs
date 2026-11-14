
use crate::editor_state::EditorState;
use egui;
use gizmo_core::World;
use gizmo_renderer::components::{active_camera, Camera, DofBlur, PostProcess};

/// One labelled slider, sized to the panel it is in.
///
/// Every row here used to be `ui.horizontal(|ui| { ui.label(..); ui.add(Slider::new(..).text(..)) })`.
/// A horizontal layout neither wraps nor shrinks, so the row's width was the sum of a long Turkish
/// label, the theme's fixed 96 px slider, a value box and a second suffix label — a flat 422.7 px
/// that did not move when the panel got narrower. The Inspector is 25% of the window
/// (`create_default_dock_state`), i.e. 400 px at the default 1600 px studio window, so the panel
/// clipped its own contents out of the box, and worse: the over-wide rows widened the enclosing
/// `ScrollArea`, so the closing hint wrapped past the panel edge and lost the end of both lines.
///
/// The label goes above the slider instead of beside it — a vertical layout wraps, and the slider
/// is then given the whole remaining width. The suffix text is gone with it: it was a second name
/// for a control that already had one ("Bloom Yoğunluğu:" … "Glow").
/// The shape of depth of field's blur: a lens's bokeh (round, or an iris of so many blades and its turn)
/// or a gaussian.
fn dof_blur_row(ui: &mut egui::Ui, blur: &mut DofBlur) {
    ui.label("Bulanıklık Biçimi");
    let gaussian = matches!(blur, DofBlur::Gaussian);
    egui::ComboBox::from_id_salt("dof_blur_kind")
        .selected_text(if gaussian { "Gauss (yumuşak)" } else { "Bokeh (diyafram)" })
        .show_ui(ui, |ui| {
            if ui.selectable_label(!gaussian, "Bokeh (diyafram)").clicked() && gaussian {
                *blur = DofBlur::round();
            }
            if ui.selectable_label(gaussian, "Gauss (yumuşak)").clicked() {
                *blur = DofBlur::Gaussian;
            }
        });
    ui.add_space(6.0);
    if let DofBlur::Bokeh { blades, rotation } = blur {
        ui.label("Diyafram Kanatları (0 = yuvarlak)");
        ui.add(egui::Slider::new(blades, 0..=9));
        if *blades > 0 && *blades < 3 {
            *blades = 3;
        }
        ui.add_space(6.0);
        if *blades >= 3 {
            slider_row(ui, "Kanat Dönüşü (radyan)", rotation, 0.0..=std::f32::consts::TAU);
        }
    }
}

fn slider_row(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
) {
    ui.label(label);
    // The value box and the frame's own padding come out of the panel; the rest is the track.
    // Floored so a panel dragged very narrow degrades into a small slider rather than a negative
    // width, which egui would round up to something wider than the panel — the defect again.
    const VALUE_BOX: f32 = 68.0;
    const MIN_TRACK: f32 = 40.0;
    ui.spacing_mut().slider_width = (ui.available_width() - VALUE_BOX).max(MIN_TRACK);
    ui.add(egui::Slider::new(value, range));
    ui.add_space(6.0);
}

/// The scene's look — bloom, lens artefacts and depth of field — edited on the camera that
/// renders it.
///
/// # Why this panel changed shape
///
/// It used to edit `EditorState::post_process`, and that was the defect rather than a detail: the
/// struct is editor state, nothing wrote it to a file, and the engine's frame read a second,
/// unrelated copy off the `Renderer`. So the look an author tuned here was gone on the next
/// reopen and absent from every exported build — while the sentence at the top of this panel said,
/// in as many words, that these were the *scene's* settings. It was live and it was a lie.
///
/// The look is now [`PostProcess`], a component on the camera, so it round-trips through the scene
/// file like everything else a scene consists of and the shipped game reads exactly what the
/// viewport showed. Exposure is edited here too but is written to [`Camera::exposure`], which is
/// where it has always lived and the one post-process value an exported build has always honoured.
///
/// With no camera there is nothing to grade, and with no grade on the camera the panel offers to
/// add one rather than editing a copy nobody reads — the whole failure mode, in one button.
pub fn draw_environment_settings(ui: &mut egui::Ui, world: &World, state: &mut EditorState) {
    ui.heading("🌍 World & Environment Settings");
    ui.label(
        egui::RichText::new(
            "Sahnenin look'u — aktif kameranın PostProcess bileşeninde yaşar, sahne dosyasıyla \
             birlikte kaydedilir ve ihraç edilen oyuna geçer.",
        )
        .weak()
        .small(),
    );
    ui.separator();

    let Some(cam_id) = active_camera(world) else {
        ui.label(
            egui::RichText::new(
                "Sahnede kamera yok. Look bir kameranın özelliği — önce bir Camera ekleyin.",
            )
            .color(egui::Color32::from_rgb(180, 180, 180)),
        );
        return;
    };

    if world.query::<&PostProcess>().get(cam_id).is_none() {
        ui.label(
            egui::RichText::new(
                "Aktif kamera henüz derecelendirilmemiş: motorun nötr varsayılanlarıyla çiziliyor.",
            )
            .weak()
            .small(),
        );
        ui.add_space(6.0);
        if ui.button("🎨 Bu kameraya look ekle").clicked() {
            if let Some(entity) = world.live(cam_id) {
                state.add_component_request = Some((entity, "PostProcess".to_string()));
            }
        }
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new(
                "Eklenen look, eklendiği anda hiçbir şeyi değiştirmez: varsayılanı motorun \
                 bileşensiz davranışının aynısıdır.",
            )
            .weak()
            .small(),
        );
        return;
    }

    egui::ScrollArea::vertical().show(ui, |ui| {
        // SAFETY: editor UI runs single-threaded in the egui draw; no concurrent World access.
        // The same pattern every other inspector section uses.
        let mut grades = unsafe { world.query_unchecked::<&mut PostProcess>() };
        if let Some(mut grade) = grades.get_mut(cam_id) {
            let before = *grade;

            egui::CollapsingHeader::new(crate::theme::section_title("Post-Processing / Bloom"))
                .default_open(true)
                .show(ui, |ui| {
                    slider_row(ui, "Bloom Yoğunluğu", &mut grade.bloom_intensity, 0.0..=5.0);
                    slider_row(ui, "Bloom Eşiği (Threshold)", &mut grade.bloom_threshold, 0.0..=2.0);
                    slider_row(ui, "Film Greni (Grain)", &mut grade.film_grain, 0.0..=1.0);
                });

            egui::CollapsingHeader::new(crate::theme::section_title("Camera Lens Effects"))
                .default_open(true)
                .show(ui, |ui| {
                    slider_row(ui, "Köşe Karartması (Vignette)", &mut grade.vignette, 0.0..=1.0);
                    slider_row(
                        ui,
                        "Kromatik Sapma (Aberration)",
                        &mut grade.chromatic_aberration,
                        0.0..=0.05,
                    );
                });

            egui::CollapsingHeader::new(crate::theme::section_title("Depth of Field (Odak)"))
                .default_open(true)
                .show(ui, |ui| {
                    slider_row(ui, "Odak Uzaklığı (metre)", &mut grade.dof_focus_dist, 0.1..=100.0);
                    slider_row(ui, "Odak Aralığı (Net Alan)", &mut grade.dof_focus_range, 0.1..=50.0);
                    slider_row(ui, "Arka Plan Bulanıklığı (0 = kapalı)", &mut grade.dof_blur_size, 0.0..=10.0);
                    // A lens: the f-number takes the focus band's place (0 keeps the band), and what
                    // lies past the farthest distance counts as that far (0 = no limit).
                    slider_row(ui, "Diyafram f/ (0 = odak aralığı)", &mut grade.dof_f_stops, 0.0..=22.0);
                    slider_row(ui, "En Uzak Mesafe (0 = sınırsız)", &mut grade.dof_max_depth, 0.0..=1000.0);
                    dof_blur_row(ui, &mut grade.dof_blur);
                });

            egui::CollapsingHeader::new(crate::theme::section_title("Motion Blur (Hareket)"))
                .default_open(true)
                .show(ui, |ui| {
                    slider_row(ui, "Deklanşör (0 = kapalı, 0.5 = 180°)", &mut grade.motion_blur, 0.0..=1.0);
                });

            // Clamped only when something moved, and only then: a file can carry values no slider
            // can reach, and correcting them on the frame the user first touches the panel is
            // honest, whereas rewriting them on a frame nobody edited is an edit the user did not
            // make.
            if *grade != before {
                *grade = grade.clamped();
            }
        }
        drop(grades);

        // Exposure is the camera's own, and always was — it is the one post-process value an
        // exported build has read all along. Editing it here rather than duplicating it into the
        // grade is what keeps the viewport and the shipped frame from disagreeing about it, which
        // they did for as long as this panel wrote an editor-only copy.
        egui::CollapsingHeader::new(crate::theme::section_title("Pozlama (Camera)"))
            .default_open(true)
            .show(ui, |ui| {
                // SAFETY: as above.
                let mut cameras = unsafe { world.query_unchecked::<&mut Camera>() };
                if let Some(mut cam) = cameras.get_mut(cam_id) {
                    slider_row(ui, "Kamera Pozlaması (Exposure)", &mut cam.exposure, 0.1..=5.0);
                    cam.exposure = cam.exposure.clamp(0.01, 20.0);
                }
            });

        ui.add_space(20.0);
        ui.label(
            egui::RichText::new(
                "💡 İpucu: Güneşin (Directional Light) yönünü ve rengini ayarlamak için Hierarchy panelinden 'Directional Light' objesini seçin.",
            )
            .color(egui::Color32::from_rgb(180, 180, 180)),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
        fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let mut out = Vec::new();
        shapes.iter().for_each(|c| walk(&c.shape, &mut out));
        out
    }

    /// What the panel shows of a camera graded with `blur`.
    fn panel(blur: DofBlur) -> Vec<String> {
        let mut world = World::new();
        let camera = world.spawn();
        world.add_component(camera, Camera::new(1.0, 0.1, 100.0, 0.0, 0.0, true));
        world.add_component(camera, PostProcess::new().with_dof_blur(blur));
        let mut state = EditorState::default();
        let ctx = egui::Context::default();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| draw_environment_settings(ui, &world, &mut state));
        let shown = texts(&output.shapes);
        output.drop_without_applying_deltas();
        shown
    }

    /// The lens's dials are on the panel — the aperture, the farthest distance, the blur's shape — and an
    /// iris's blades and turn show for a bokeh of blades, the turn not for a round one, neither for a
    /// gaussian.
    #[test]
    fn the_panel_sets_the_lens_and_the_blurs_shape() {
        let hexagon = panel(DofBlur::blades(6, 0.5));
        for label in ["Diyafram f/ (0 = odak aralığı)", "En Uzak Mesafe (0 = sınırsız)", "Bulanıklık Biçimi", "Bokeh (diyafram)", "Diyafram Kanatları (0 = yuvarlak)", "Kanat Dönüşü (radyan)"] {
            assert!(hexagon.iter().any(|t| t == label), "{label} is not on the panel: {hexagon:?}");
        }
        let round = panel(DofBlur::round());
        assert!(round.iter().any(|t| t == "Diyafram Kanatları (0 = yuvarlak)"));
        assert!(!round.iter().any(|t| t == "Kanat Dönüşü (radyan)"), "a round iris shows a turn");
        let gaussian = panel(DofBlur::Gaussian);
        assert!(gaussian.iter().any(|t| t == "Gauss (yumuşak)"));
        assert!(!gaussian.iter().any(|t| t.starts_with("Diyafram Kanat")), "a gaussian shows blades");
    }
}
