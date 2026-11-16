//! Gizmo Studio's entry point: the editor on its own ([`gizmo_studio::run`] with no game). Run it
//! with `cargo run -p gizmo-studio`; the `editor`, `scene`, `audio` and `scripting` engine features
//! are enabled by default. A game opens the same editor on itself from a binary of its own — see
//! `gizmo_studio::app`.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    gizmo::core::logger::init_tracing();
    gizmo_studio::run(None).expect("uygulama çalıştırılamadı");
}
