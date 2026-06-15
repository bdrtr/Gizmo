//! Property-based tests for the vehicle `Gearbox` (index-safety).
//!
//! Faz 1.2 — Faz 0'da düzeltilen bir panik regresyonunu kilitler: otomatik vites
//! `gears` ile `shift_up/down_speeds` dizilerini KÖR indekslemekten panik
//! ediyordu (`shift_up_speeds[cg]` taşması). Düzeltme `.get()` kullanıyor. Bu
//! test KASTEN tutarsız-uzunlukta diziler + rastgele (negatif/dev/sıfır) hız
//! dizileri besler: çözüm asla panik etmemeli, vites daima sınırda kalmalı ve
//! vites oranı daima sonlu olmalı.

use gizmo_physics_rigid::components::vehicle::Gearbox;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn gearbox_never_panics_and_stays_in_bounds(
        gears in prop::collection::vec(0.2f32..5.0, 1..8),
        ups in prop::collection::vec(0.0f32..120.0, 0..10),
        downs in prop::collection::vec(0.0f32..120.0, 0..10),
        speeds in prop::collection::vec(-50.0f32..200.0, 1..40),
        reversing in any::<bool>(),
        reverse_ratio in 0.5f32..5.0,
    ) {
        let mut gb = Gearbox {
            gears: gears.clone(),
            reverse_ratio,
            final_drive: 3.5,
            current_gear: 0,
            is_automatic: true,
            // KASTEN tutarsız uzunluklar — eski kod burada taşıp panik ediyordu.
            shift_up_speeds: ups,
            shift_down_speeds: downs,
            is_reversing: reversing,
        };

        for &s in &speeds {
            gb.update_gear(s); // panik = test başarısız
            prop_assert!(
                gb.current_gear < gb.gears.len(),
                "vites indeksi sınır dışı: {} / {}",
                gb.current_gear, gb.gears.len()
            );
            let r = gb.current_ratio();
            prop_assert!(r.is_finite(), "vites oranı sonlu değil: {r}");
            // Oran ya bir ileri vites ya da geri oranı olmalı.
            let valid = if reversing {
                (r - reverse_ratio).abs() < 1e-6
            } else {
                gears.iter().any(|&g| (g - r).abs() < 1e-6)
            };
            prop_assert!(valid, "vites oranı bilinen bir orana eşlenmiyor: {r}");
        }
    }
}
