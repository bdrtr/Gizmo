//! # Nişancı — masaüstü
//!
//! Oyunun kendisi `games/src/nisanci.rs`'te: tarayıcı yapısı (`demo-web`, `?oyun=nisanci`) da aynı
//! oyunu çalıştırıyor. `nisanci --test` pencere açmaz: iki dakika kendi kendine oynar, sonra ilk
//! dokunuşu dener.

fn main() {
    if std::env::args().any(|a| a == "--test") {
        gizmo::simple::run_self_test(games::nisanci::self_test);
    }
    games::nisanci::app().run().expect("uygulama çalıştırılamadı");
}
