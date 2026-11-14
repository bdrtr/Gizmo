//! # Dünya — masaüstü
//!
//! Açık dünyada keşif: yürü, arabaya bin, kristal topla. Oyunun kendisi `games/src/dunya.rs`'te.
//! `dunya --test` pencere açmaz: otomatik pilot yürür, kristal toplar, arabayla hücre sınırlarını geçer.

fn main() {
    if std::env::args().any(|a| a == "--test") {
        gizmo::simple::run_self_test(games::dunya::self_test);
    }
    games::dunya::app().run().expect("uygulama çalıştırılamadı");
}
