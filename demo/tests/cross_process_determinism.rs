//! Cross-process determinizm pipeline'ı (Faz 2 — "çok-makineli (iki binary, hash diff)").
//!
//! `determinism_oracle` binary'sini İKİ AYRI SÜREÇTE çalıştırır ve `state_hash`'leri
//! karşılaştırır. Her süreç kendi HashMap taban-seed'ini OS'ten ayrı alır; hash'ler eşitse
//! engine çıktısı hash-iterasyon-sırasından bağımsızdır → aynı-platform determinizm SÜREÇLER
//! ARASI da geçerli (replay/rollback'in farklı makinelerde aynı binary'yle çalışmasının ön
//! koşulu). Bit-exact cross-PLATFORM (x86↔ARM) bu testin KAPSAMINDA DEĞİL.

use std::process::Command;

fn run_oracle() -> String {
    let exe = env!("CARGO_BIN_EXE_determinism_oracle");
    let output = Command::new(exe)
        .output()
        .expect("determinism_oracle binary'si çalıştırılamadı");
    assert!(
        output.status.success(),
        "oracle hata kodu döndü: {:?}",
        output.status
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .find_map(|l| l.strip_prefix("STATE_HASH="))
        .map(|h| h.trim().to_string())
        .expect("oracle çıktısında STATE_HASH satırı yok")
}

#[test]
fn determinism_holds_across_separate_processes() {
    let h1 = run_oracle();
    let h2 = run_oracle();
    let h3 = run_oracle();
    assert_eq!(
        h1, h2,
        "iki ayrı SÜREÇ farklı state_hash üretti → cross-process determinizm bozuk\n  run1={h1}\n  run2={h2}"
    );
    assert_eq!(h2, h3, "3. süreç ayrıştı: {h2} vs {h3}");
}
