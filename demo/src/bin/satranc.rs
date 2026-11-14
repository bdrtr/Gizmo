//! # Satranç — masaüstü
//!
//! Oyunun kendisi `games/src/satranc.rs`'te; tablet ve telefon da (`demo-android-satranc`) aynı
//! oyunu çalıştırıyor. `satranc --test` pencere açmaz: perft ile kuralları, bir hamlede matı ve
//! motorun kendine karşı bir maçını sınar.

fn main() {
    if std::env::args().any(|a| a == "--test") {
        gizmo::simple::run_self_test(games::satranc::self_test);
    }
    games::satranc::app().run().expect("uygulama çalıştırılamadı");
}
