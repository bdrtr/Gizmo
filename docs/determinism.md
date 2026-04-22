# Cross-Platform Determinism in Gizmo Engine

Determinizm (Belirlenimcilik), aynı fiziksel durum (State) ile başlayan her simülasyonun adım adım çalıştırıldığında (aynı ΔT - dt süresinde) tamamen **Aynı Sonucu** üretmesidir.

Gizmo Engine, Multiplayer Rollback Netcode (Örn: GGPO) kullanan oyunlar veya Replay Sistemleri için katı (strict) determinizm koşullarına uygun tasarlanmıştır.

## Matematik Tasarımı: Strict IEEE-754

Tam (bit-exact) determinizm normalde I48F16 gibi Fixed-Point tamsayı matematiğini gerektirir çünkü FMA (Fused Multiply-Add) ve SIMD paralelizmi CPU mimarisine göre sonuçları saptırır. Gizmo Engine, bunu bir tamsayı kütüphanesine dönüştürmek yerine `glam`'ın float mantığını şu optimizasyonları ve engelleri aşarak IEEE-754 sınırlarında deterministik hale getirmiştir:

### 1. Fast-Math Optimizasyonlarının Kapatılması
Rust derleyicisi ve LLVM varsayılan olarak fast-math hedeflerine açık değildir. `gizmo-physics`, FMA talimatlarından kaynaklanan assosiyatif (associativity) farklılıklarını çözmek adına asit (strict) konfigürasyonlara uygun olarak çalışır. Aşağıdaki özellikleri `Cargo.toml` projelerinizde zorlamalısınız:

```toml
[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
# DİKKAT: overflow-checks kapalı olmalı, fma disable olmalı.
```

### 2. Standart LibM
Aşkın matematiksel fonksiyonları (`sin`, `cos`, `sqrt`) işletim sistemi ve standard C kütüphanelerinde (`libc`) CPU çekirdeğine göre (Intel vs ARM) nano-farklılıklar barındırır.
Gizmo platformlarında determinizmi aşmamak için donanımsal op kodlar yerine deterministik `libm` sandığının (crate) sağladığı float matematiğini izole edilmiş şekilde çalıştırabilme altyapısına sahibiz. Gizmo Math tamamen `core::f32` üzerindeki deterministic matematik uzantılarıyla desteklenir.

## Mimari Özellikleri

- **Paralel Kilitlenmemesi (Parallel Determinism):** Rayon iş parçacıkları (Threads) çalıştırma sırasını rastgele atasa da; Gizmo Engine'in "Island Generation" (Adacık Sınıflandırma) ve "Graph Coloring" algoritmaları, obje çözümlerinde hafızayı (memory) indeks id'leriyle izler. Hangi nesnenin hangi işlemcide çözüldüğünden bağımsız olarak sırasal olarak tek bir Gauss-Seidel sonucunda buluşurlar.
- **Sürtünme (Friction) ve Dürtü (Impulse):** Sürtünme katsayıları float epsilon hatalarını biriktirmemesi için "Warm-starting" algoritmasında 8-bit'lik offset toleransları ile eşiklenir. `system/solver.rs`'de `accumulated_j` değerleri bu tutarlılığı sağlar.

## Kullanım Testi
```rust
// İki farklı makinede oyun durumlarını Hash bazlı denetleyebilirsiniz
fn check_sync_state(world: &World) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for rb in world.query::<RigidBody>() {
        hasher.write_u32(rb.position.x.to_bits());
        hasher.write_u32(rb.position.y.to_bits());
        hasher.write_u32(rb.position.z.to_bits());
    }
    hasher.finish()
}
```

Bu belge doğrultusunda, motorun tüm AABB, DBVT ve Çözücü matematik döngüleri asenkron senaryolarda dahi çapraz platform uyumlu (Cross-Platform Parity) çalışmaktadır.
