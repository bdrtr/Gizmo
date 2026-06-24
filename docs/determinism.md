# Gizmo Engine'de Determinizm — Mevcut Durum

Determinizm: aynı başlangıç durumundan (state) aynı `dt` adımlarıyla çalıştırılan
simülasyonun adım adım **aynı sonucu** üretmesidir. Rollback netcode (GGPO tarzı)
ve replay sistemleri için gereklidir.

> **Bu belge motorun GERÇEK durumunu anlatır.** Önceki sürüm bit-exact cross-platform
> determinizm ve fixed-point matematik vaat ediyordu; bunların ikisi de şu an
> **uygulanmış değil**. Aşağıda neyin garanti edildiği, neyin edilmediği açıkça yazılıdır.

## Şu an ne durumda?

Simülasyon durumu (`Transform`, `Velocity`, çözücü) tamamen **`glam` / `f32`** üzerinde
çalışır. `gizmo-math` içindeki `Fp32` (Q16.16 sabit noktalı) tip **mevcut ama simülasyon
tarafından kullanılmıyor** — bağımsız, deneysel bir yardımcıdır.

| Senaryo | Durum |
|---|---|
| Aynı makine + aynı binary'de replay / rollback | ✅ Pratikte deterministik (aşağıdaki koşullarla) |
| Aynı mimari, farklı derleme bayrakları | ⚠️ fast-math/FMA farklılıkları sapma yaratabilir |
| Farklı CPU mimarileri (x86 ↔ ARM) bit-exact | ❌ **Garanti edilmiyor** (glam SIMD + transandantal fonksiyonlar) |

## Aynı-makine determinizmi için sağlanan koşullar

- **Sıralama (ordering):** Çözücü, adaları (islands) ve gövdeleri indeks/entity-id'ye
  göre işler; Rayon paralelliği sonucu değiştirmez (Gauss-Seidel toplamı sıradan bağımsız
  birleşir). Hash tabanlı yapıların iterasyon sırasının çıktıyı etkilediği yerler
  ayıklandı (ör. `quickhull` artık `BTreeMap`/`BTreeSet` kullanır — eskiden `HashMap`/`HashSet`
  rastgele seed yüzünden convex hull çıktısını çalıştırmadan çalıştırmaya değiştiriyordu).
- **Sabit zaman adımı:** `PhysicsWorld::step` bir accumulator ile sabit `dt` alt-adımları
  (substep) çalıştırır; değişken kare süresi simülasyona sızmaz.
- **Warm-starting:** Çözücü birikmiş impulse'ları sakladığından aynı giriş aynı çıkışı verir.

## Cross-platform bit-exact için ne gerekirdi (henüz YOK)

Tam (bit-exact) cross-platform determinizm için tüm simülasyonun `f32` yerine `Fp32`
sabit-noktalı matematikten geçmesi gerekir; çünkü:

- **SIMD & FMA:** `glam` SIMD yolları ve FMA (Fused Multiply-Add) komutları mimariye göre
  farklı yuvarlama verir; result associativity CPU'ya bağlıdır.
- **Transandantal fonksiyonlar:** `sin`/`cos`/`sqrt` donanım uygulamaları (Intel vs ARM)
  nano-farklılıklar barındırır; bit-exact için `libm` gibi yazılımsal, sabit bir uygulama
  şarttır.

`Fp32` bu işin temelini sağlar ama `Transform`/`Velocity`/çözücü ona taşınmadıkça vaat
edilemez. (Not: `Fp32::to_i32` artık sıfıra doğru kesiyor ve `+`/`-`/`*` doygunlukla
sınırlanıyor — eskiden negatiflerde asimetrik yuvarlama ve sessiz i32 taşması vardı.)

## Senkron kontrolü (sync check) — `PhysicsWorld::state_hash()`

Durumu hash'leyerek desync tespiti (rollback) ve replay doğrulaması için motorda hazır
API vardır:

```rust
let h: u64 = world.state_hash(); // deterministik durum özeti
```

`state_hash()` cisimleri **entity id'sine göre SABİT sırada** gezer (ekleme/dizi/HashMap
sırasından bağımsız), her `f32`'yi `to_bits()` ile ve uyku durumunu karıştırır; **sabit
anahtarlı** `DefaultHasher` kullanır (RandomState DEĞİL) → çıktı **süreçler arası** tutarlıdır.
Aynı makinede/binary'de adım adım eşleşir; farklı CPU mimarileri arası bit-exact beklenmez.

## Faz 2 kararı: aynı-platform replay/rollback (test ile KİLİTLİ)

Determinizm hedefi **aynı-platform replay/rollback** olarak belirlendi (cross-platform
bit-exact KAPSAM DIŞI; gerekirse Fp32 göçü ayrı bir büyük iştir). Bu garanti artık otomatik
testlerle sabitlenmiştir:

- `crates/gizmo-physics-rigid/tests/determinism.rs` — iki ÖZDEŞ dünya aynı adımlarla AYNI
  `state_hash` üretir (her dünya HashMap'lerini ayrı seed'le kurar → hash-iterasyon-sırası
  bağımsızlığı kanıtı); hash adımla değişir; perturbasyon ayrıştırılır (desync tespiti).
- `demo/tests/cross_process_determinism.rs` — `determinism_oracle` binary'si İKİ/ÜÇ AYRI
  SÜREÇTE koşar, `state_hash`'ler eşittir (farklı süreç HashMap taban-seed'ine rağmen) →
  **süreçler-arası** determinizm (aynı-binary farklı-makine için ön koşul).
- `demo/src/bin/headless_stress_test` — 2000-kutu kule, 3 koşu hash eşleşmesi (CI determinism job).
