# Gizmo Engine — Production-Ready Yol Haritası

> Hedef: güvenilir, test edilmiş, (gerekirse) deterministik bir simülasyon motoru.
> Bu belge canlıdır — madde tamamlandıkça `[x]` işaretle, **Durum** bölümünü güncelle.

## Durum
- **Şu anki aşama:** Faz 0 (Stabilizasyon & Güven)
- **Son büyük iş (2026-06-11):** netcode birleştirme + 9 çekirdek bug düzeltmesi (commit `ba323fc`).
- **Sıradaki:** Faz 0 — kalan şüpheli bug'ları kapatmak.

İlke: **önce doğruluk, sonra kapsam.** Bir fizik motoru ancak çekirdeği güvenilirse
production-ready olur. Önce bilinen/şüpheli bug'lar ve test altyapısı; özellik eklemek sonra.

---

## Faz 0 — Stabilizasyon & Güven  ⬅️ ŞİMDİ

Derin incelemede işaretlenen ama henüz **kapatılmamış** orta-güvenli sorunlar:

- [ ] **Eklem efektif-kütle `k`** — `joints/solver.rs:~204` çapraz-çarpım sırası: doğrusu
      `n·[(I⁻¹(r×n))×r]`, kodda `((I⁻¹ r)×n)×r`. Merkez-dışı ankor + anizotropik atalette
      yanlış impulse büyüklüğü → sarkma/salınım.
- [ ] **Sürtünme birikimi** — `solver.rs` 2D sürtünmede tangent yönü her iterasyonda yeniden
      hesaplanıp eski birikmiş vektör yeni tangente projekte ediliyor → kayıplı/yön kayması.
      Sabit iki tangent bazında skaler birikim yap.
- [x] **Stale-handle okuma** — generation-doğrulamalı `Query::get_entity`/`get_mut_entity`
      eklendi; ham `get(u32)`/`query_entity*` "unchecked" olarak belgelendi; çarpışma-olayı
      caller'ları (fracture) checked sürüme geçirildi; regresyon testi eklendi. (e674424 sonrası)
- [ ] **`spawn_batch` değişmezi** — bundle archetype'ın tüm sütunlarını kapsamazsa
      `entities.len() != column.len()` desync'i mümkün; değişmezi enforce et veya kontrol ekle.
- [ ] **EPA yüz yönelimi** — `compute_face_normal` normali winding yerine origin'den zorluyor;
      sığ/dejenere temaslarda yanlış yüz seçilebilir. (Witness refactor'ı bunu değiştirmedi.)
- [ ] **Uyku/kinematik etkileşimi** — hareket eden kinematik platform üstündeki uyuyan dinamik
      cismi uyandırmıyor; `IslandManager::should_sleep` pipeline'dan hiç çağrılmıyor.
- [ ] **`iter_chunks_mut` aşırı işaretleme** — `get_slice` tüm archetype satırlarını "changed"
      işaretliyor (yazılmayanlar dahil) → change detection'da false positive.
- [ ] **SparseSet change tracking** — `Changed<T>`/`Added<T>` SparseSet bileşenlerinde her zaman
      `true` (TODO); gerçek tick takibi ekle.

Denetlenmemiş alt-sistemleri aynı derinlikte tara (her biri ayrı bug-avı turu):
- [ ] FEM soft-body (`gizmo-physics-soft`)
- [ ] Multibody / ABA (`gizmo-physics-rigid/multibody`)
- [ ] Fracture & destruction
- [ ] Vehicle modeli (`vehicle.rs`)
- [ ] Raycast kalan kenar durumları (içeriden başlama, hull-AABB yaklaşımı)

**Çıkış kriteri:** bilinen High/Medium bug yok; her düzeltme bir regresyon testiyle kilitli.

---

## Faz 1 — Test & CI Altyapısı

- [ ] Her çekirdek algoritmaya birim test (GJK/EPA, SAT, solver, integrator, joints, ECS).
- [ ] **Property-based / differential testler** — fizik invariantları: penetrasyon ⟶ 0,
      momentum/enerji sınırları, broad-phase pairs = brute-force.
- [ ] **Stres + soak** — N-kutu yığını M dakika stabil mi; enerji patlaması/sürüklenme yok.
- [ ] **Golden/regresyon** — referans senaryoların hash/snapshot'ı (zaten `headless_stress_test` var).
- [ ] **CI matrisi** — Linux/macOS/Windows × stable; `clippy -D warnings`; `cargo test --workspace`.
- [ ] **Benchmark regresyon takibi** — criterion sonuçlarını CI'da izle.

**Çıkış kriteri:** yeşil CI, anlamlı kapsam, regresyonlar otomatik yakalanıyor.

---

## Faz 2 — Determinizm Kararı

- [ ] **Hedefi netleştir:** cross-platform bit-exact mı, yoksa aynı-platform replay/rollback mı?
- [ ] Aynı-platform yeterliyse: mevcut durumu test harness'iyle garanti et (hash eşleşmesi) +
      `docs/determinism.md`'i bununla sınırla (kısmen yapıldı).
- [ ] Cross-platform isteniyorsa (lockstep netcode için): simülasyon durumunu `f32`/glam'dan
      `Fp32` sabit-noktaya taşı **veya** softfloat (`libm`) + SIMD/FMA kapatma stratejisi kur.
      (Büyük iş — kapsamı netleştirmeden başlama.)
- [ ] Çok-makineli determinizm test pipeline'ı (iki binary, hash diff).

**Çıkış kriteri:** determinizm vaadi belgeyle uyuşuyor ve testle kanıtlı.

---

## Faz 3 — Netcode Olgunlaştırma

- [ ] Gerçek **istemci binary'si**: `NetworkClient` + `ClientPredictor` + `SnapshotInterpolator`
      + `InputAck`/`WorldStateUpdate` döngüsü (şu an kütüphane parçaları bağlanmaya hazır ama
      uçtan uca çalışan bir client yok).
- [ ] Rollback'i deterministik fizikle entegre et (Faz 2'ye bağlı).
- [ ] Lag/jitter/packet-loss simülasyonuyla otomatik test.

**Çıkış kriteri:** iki client gerçekçi ağ koşullarında senkron kalıyor.

---

## Faz 4 — Fizik Derinliği & Kalite

- [ ] Solver kalite turu: warm-starting doğrulama, sub-stepping ayarı, manifold kararlılığı.
- [ ] CCD (sürekli çarpışma) sağlamlık testleri (tünelleme yok).
- [ ] Joint kütüphanesini tamamla + her tür için test (fixed/hinge/slider/ballsocket/spring + motor/limit).
- [ ] Islands & sleeping sağlamlaştırma (Faz 0 uyku bug'ı sonrası).
- [ ] Geniş sahne performans profili (mimalloc/archetype cache locality doğrulama).

---

## Faz 5 — Renderer & Araçlar

- [ ] Renderer denetimi (`gizmo-renderer` — bu turda hiç denetlenmedi).
- [ ] WASM hedefini uçtan uca doğrula (async asset loader dahil).
- [ ] Editor/studio iş akışı: sahne kaydet/yükle, prefab, inspector güvenilirliği.

---

## Faz 6 — API Kararlılığı & 1.0

- [ ] Genel API'yi gözden geçir/dondur; `unsafe` kontratlarını belgele.
- [ ] Crates.io yayın hattını doğrula (`publish_all.sh` topolojik sıra güncel).
- [ ] Sürümleme + CHANGELOG + migration kılavuzu.
- [ ] `1.0.0`.

---

## Çalışma Yöntemi
- Her madde: **düzelt → regresyon testi yaz → derle/test/clippy → işaretle.**
- Davranış değiştiren fizik düzeltmelerini `headless_stress_test` + odaklı senaryolarla doğrula.
- Bug-avı turlarında subagent fan-out kullan, sonra her bulguyu elle doğrula (false-positive'leri ele).
</content>
