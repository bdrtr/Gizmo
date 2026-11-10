//! # Pinbol — masaüstü
//!
//! Oyunun kendisi `games/src/pinbol.rs`'te: tarayıcı yapısı (`demo-web`, `?oyun=pinbol`) da aynı
//! oyunu çalıştırıyor. `pinbol --test` pencere açmaz: otomatik palet bir dakika oynar.

fn main() {
    if std::env::args().any(|a| a == "--test") {
        gizmo::simple::run_self_test(games::pinbol::self_test);
    }
    games::pinbol::app().run().expect("uygulama çalıştırılamadı");
}
