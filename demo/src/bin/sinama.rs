//! # Sınama — masaüstü
//!
//! Motoru zorlayan dört sahne (kule, kumaş, jöle, ışık) ve ne kadar dayandığı; oyunun kendisi
//! `games/src/sinama.rs`'te. `sinama --test` pencere açmaz: her sahneyi üç saniye oynatır.

fn main() {
    if std::env::args().any(|a| a == "--test") {
        gizmo::simple::run_self_test(games::sinama::self_test);
    }
    games::sinama::app().run().expect("uygulama çalıştırılamadı");
}
