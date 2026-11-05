//! # Nişancı — masaüstü
//!
//! Oyunun kendisi `games/src/nisanci.rs`'te: tarayıcı yapısı (`demo-web`, `?oyun=nisanci`) da aynı
//! oyunu çalıştırıyor. `nisanci --test` pencere açmaz: iki dakika kendi kendine oynar, sonra ilk
//! dokunuşu dener.

fn main() {
    if std::env::args().any(|a| a == "--test") {
        std::process::exit(if games::nisanci::self_test() { 0 } else { 1 });
    }
    games::nisanci::app().run().expect("uygulama çalıştırılamadı");
}
