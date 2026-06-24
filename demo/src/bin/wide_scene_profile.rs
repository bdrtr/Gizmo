//! Geniş-sahne performans profili (Faz 4 — mimalloc / archetype cache locality doğrulama).
//!
//! Çok sayıda kutu (varsayılan ~2000) bir zemine düşüp yerleşir; her frame'in aşama
//! zamanlaması (PhysicsMetrics) + uyku ilerlemesi raporlanır. Doğrular:
//!   * mimalloc global allocator FİZİK yükünde kullanılıyor (eskiden yalnız gizmo-studio),
//!   * uyku optimizasyonu: yerleştikçe uyuyan cisim sayısı artar → çözücü işi DÜŞER
//!     (erken vs geç frame süresi farkı),
//!   * aşama dağılımı (broadphase / narrowphase / solver / integration) — darboğaz görünür,
//!   * archetype ECS + bitişik kolon düzeni geniş sahnede ms-ölçekli kalır.
//!
//! Çalıştır: `cargo run --release -p demo --bin wide_scene_profile [yan] [frame]`
//!   yan   = grid kenar uzunluğu (yan×yan×kat kutu); varsayılan 20 (→ 20×20×5=2000)
//!   frame = simüle edilecek frame sayısı; varsayılan 300

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use gizmo::core::entity::Entity;
use gizmo::math::Vec3;
use gizmo::physics::components::{Collider, RigidBody, Transform, Velocity};
use gizmo::physics::world::PhysicsWorld;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let side: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(20);
    let layers: usize = 5;
    let frames: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(300);

    let mut world = PhysicsWorld::new();

    // Zemin (geniş statik kutu).
    let mut ground = RigidBody::new_static();
    ground.wake_up();
    world.add_body(
        Entity::new(0, 0),
        ground,
        Transform::new(Vec3::new(0.0, -1.0, 0.0)),
        Velocity::default(),
        Collider::box_collider(Vec3::new(side as f32 * 1.5, 1.0, side as f32 * 1.5)),
    );

    // Geniş yerleşmiş sahne: yan×yan AYRIK sütun (yatay boşluk → her sütun kendi adası),
    // her sütun `layers` kutuluk TAM TEMASLI yığın. Hızla yerleşip uyur → gerçek "büyük
    // sahnede çoğu cisim dinlenmede" durumunu temsil eder (dormant-skip burada kazandırır).
    let half = 0.5f32;
    let spacing = 1.3f32; // > kutu genişliği (1.0) → sütunlar birbirine değmez (ayrı ada)
    let mut id = 1u32;
    for ly in 0..layers {
        for x in 0..side {
            for z in 0..side {
                let px = (x as f32 - side as f32 / 2.0) * spacing;
                let pz = (z as f32 - side as f32 / 2.0) * spacing;
                let py = half + ly as f32 * (2.0 * half); // tam temas, düşüş yok
                let mut rb = RigidBody::new(1.0, 0.0, 0.6, true);
                rb.wake_up();
                let col = Collider::box_collider(Vec3::splat(half));
                rb.update_inertia_from_collider(&col);
                world.add_body(
                    Entity::new(id, 0),
                    rb,
                    Transform::new(Vec3::new(px, py, pz)),
                    Velocity::default(),
                    col,
                );
                id += 1;
            }
        }
    }

    let body_count = world.transforms.len();
    println!("=== GENİŞ-SAHNE PROFİLİ (mimalloc) ===");
    println!(
        "Cisimler: {body_count} (zemin + {}×{}×{} kutu), frame: {frames}, allocator: mimalloc",
        side, side, layers
    );

    // Toplam aşama-zamanlama birikimi + frame süreleri.
    let (mut t_broad, mut t_narrow, mut t_solver, mut t_integ) = (0.0f32, 0.0, 0.0, 0.0);
    let mut frame_ms: Vec<f32> = Vec::with_capacity(frames);
    let mut sleep_at: Vec<(usize, usize, usize)> = Vec::new(); // (frame, sleeping, contacts)

    let total_start = Instant::now();
    for f in 0..frames {
        let fs = Instant::now();
        world.step(1.0 / 60.0).ok();
        frame_ms.push(fs.elapsed().as_secs_f32() * 1000.0);

        let m = &world.metrics;
        t_broad += m.broadphase_ms;
        t_narrow += m.narrowphase_ms;
        t_solver += m.solver_ms;
        t_integ += m.integration_ms;

        if f == 5 || f == frames / 4 || f == frames / 2 || f == frames - 1 {
            sleep_at.push((f + 1, world.metrics.sleeping_count, world.metrics.contact_count));
        }
    }
    let total_ms = total_start.elapsed().as_secs_f32() * 1000.0;

    let avg_frame = total_ms / frames as f32;
    let stage_sum = t_broad + t_narrow + t_solver + t_integ;
    let pct = |x: f32| if stage_sum > 0.0 { 100.0 * x / stage_sum } else { 0.0 };

    // İlk 10 ve son 10 frame ortalaması (uyku optimizasyonunun işi düşürdüğünü gösterir).
    let early: f32 = frame_ms.iter().take(10).sum::<f32>() / 10.0;
    let late: f32 = frame_ms.iter().rev().take(10).sum::<f32>() / 10.0;

    println!("\n--- Toplam ---");
    println!("Toplam: {total_ms:.1} ms | frame ort: {avg_frame:.3} ms ({:.1} FPS bütçesi)", 1000.0 / avg_frame);
    println!("İlk 10 frame ort: {early:.3} ms  →  Son 10 frame ort: {late:.3} ms  (uyku ile {:.1}× hızlanma)", if late > 0.0 { early / late } else { 0.0 });

    println!("\n--- Aşama dağılımı (tüm frame'ler toplamı) ---");
    println!("  broadphase : {t_broad:8.1} ms ({:4.1}%)", pct(t_broad));
    println!("  narrowphase: {t_narrow:8.1} ms ({:4.1}%)", pct(t_narrow));
    println!("  solver     : {t_solver:8.1} ms ({:4.1}%)", pct(t_solver));
    println!("  integration: {t_integ:8.1} ms ({:4.1}%)", pct(t_integ));

    println!("\n--- Uyku ilerlemesi (uyku optimizasyonu doğrulaması) ---");
    for (f, s, c) in &sleep_at {
        println!(
            "  frame {f:4}: uyuyan {s:5}/{} dinamik ({:4.1}%) | temas {c}",
            body_count - 1,
            100.0 * *s as f32 / (body_count - 1) as f32
        );
    }
    println!("\nNOT: Yerleştikçe uyuyan oranı artmalı; solver/narrowphase işi düşmeli (uyuyan");
    println!("adalar çözücüde atlanır). Archetype ECS bitişik kolonları + mimalloc ile geniş");
    println!("sahnede frame süresi ms-ölçekli kalır.");
}
