//! # Dünya — stüdyoda
//!
//! Gizmo Studio, Dünya'nın üzerinde açık: sahne düzenlenebiliyor, ▶ oyunu kendi dünyasında
//! (`gizmo_studio::pie`) kendi kurulumu ve karesiyle oynatıyor, ⏹ düzenlenen sahneye dokunulmamış
//! hâliyle dönüyor. Oyunun kendisi `games/src/dunya`; burası yalnızca ikisini bağlıyor.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    gizmo::core::logger::init_tracing();
    let game = gizmo_studio::StudioGame::new("Dünya", games::dunya::studio::setup, games::dunya::studio::update)
        .with_render_settings(games::dunya::studio::render_settings)
        .with_app(|app| {
            games::dunya::studio::embedded()
                .iter()
                .fold(app.add_plugin(gizmo::ui::UiPlugin), |app, (path, bytes)| app.add_embedded_asset(path, std::borrow::Cow::Borrowed(*bytes)))
        });
    gizmo_studio::run(Some(game)).expect("stüdyo açılamadı");
}
