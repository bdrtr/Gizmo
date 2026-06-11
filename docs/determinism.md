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

## Senkron kontrolü (sync check)

İki tarafın durumunu hash'leyerek desync tespiti yapılabilir. **Doğru API ile**:

```rust
use std::hash::Hasher;
use gizmo_physics_core::components::transform::Transform;

fn check_sync_state(world: &gizmo_core::World) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let transforms = world.borrow::<Transform>();
    // NOT: entity'leri SABİT bir sırada gez (id'ye göre) — aksi halde hash sırası değişir.
    for ent in world.iter_alive_entities() {
        if let Some(t) = transforms.get(ent.id()) {
            for c in [t.position.x, t.position.y, t.position.z] {
                hasher.write_u32(c.to_bits());
            }
        }
    }
    hasher.finish()
}
```

Aynı makinede/binary'de bu hash adım adım eşleşmelidir. Farklı mimariler arasında
eşleşme şu an **beklenmemelidir**.
