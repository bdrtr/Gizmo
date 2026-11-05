//! # Pinbol — masaüstü
//!
//! Oyunun kendisi `games/src/pinbol.rs`'te: tarayıcı yapısı (`demo-web`, `?oyun=pinbol`) da aynı
//! oyunu çalıştırıyor. `pinbol --test` pencere açmaz: otomatik palet bir dakika oynar.

fn main() {
    if std::env::args().any(|a| a == "--test") {
        std::process::exit(if games::pinbol::self_test() { 0 } else { 1 });
    }
    games::pinbol::app().run().expect("uygulama çalıştırılamadı");
}
