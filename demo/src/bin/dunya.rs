//! # Dünya — masaüstü
//!
//! Açık dünyada keşif: yürü, arabaya bin, kristal topla. Oyunun kendisi `games/src/dunya.rs`'te.
//! `dunya --test` pencere açmaz: otomatik pilot yürür, kristal toplar, arabayla hücre sınırlarını geçer.
//! `dunya --bench [kare]` pencere açmadan GPU'da çizer ve karenin nereye gittiğini döker.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--test") {
        gizmo::simple::run_self_test(games::dunya::self_test);
    }
    if let Some(i) = args.iter().position(|a| a == "--bench") {
        let frames = args.get(i + 1).and_then(|n| n.parse().ok()).unwrap_or(1200);
        std::process::exit(if games::dunya::bench::bench(frames) { 0 } else { 1 });
    }
    games::dunya::app().run().expect("uygulama çalıştırılamadı");
}
