# Gizmo Engine — Production-Ready Yol Haritası

> Hedef: güvenilir, test edilmiş, (gerekirse) deterministik bir simülasyon motoru.
> Bu belge canlıdır — madde tamamlandıkça `[x]` işaretle, **Durum** bölümünü güncelle.

## Durum
- **Şu anki aşama:** Faz 0 BİTMEK ÜZERE → Faz 1 (Test & CI altyapısı)
- **İlerleme:** ECS+çekirdek fizik (9 bug), vehicle, soft-body, fracture, multibody/ABA denetlendi+düzeltildi. ~469 test yeşil, hepsi push'lu.
- **Sıradaki:** Faz 1 (CI matrisi, property/stres testleri) VEYA Faz 0 kalan küçük notlar (raycast kenar durumları, floating-base ABA).

İlke: **önce doğruluk, sonra kapsam.** Bir fizik motoru ancak çekirdeği güvenilirse
production-ready olur. Önce bilinen/şüpheli bug'lar ve test altyapısı; özellik eklemek sonra.

---

## Faz 0 — Stabilizasyon & Güven  ⬅️ ŞİMDİ

Derin incelemede işaretlenen ama henüz **kapatılmamış** orta-güvenli sorunlar:

- [x] **Eklem efektif-kütle `k`** — `apply_linear_constraint` + slider motor artık
      `k_ang = (r×n)·I⁻¹·(r×n)` kullanıyor (temas çözücüyle aynı, doğru form). Ayırt edici
      test: 1-DOF kısıt tek uygulamada bağıl hızı sıfırlıyor (yanlış k'de kalan=0.0236).
- [x] **Sürtünme birikimi** — normalden türetilen SABİT iki ortonormal tangent; her eksende
      skaler birikim + dairesel Coulomb koni clamp. (Eski tek-tangent yöntemi dik bileşeni
      kaybediyordu.) Test: diyagonal kayma simetrik yavaşlayıp duruyor.
- [x] **Stale-handle okuma** — generation-doğrulamalı `Query::get_entity`/`get_mut_entity`
      eklendi; ham `get(u32)`/`query_entity*` "unchecked" olarak belgelendi; çarpışma-olayı
      caller'ları (fracture) checked sürüme geçirildi; regresyon testi eklendi. (e674424 sonrası)
- [x] **`spawn_batch` değişmezi** — İNCELENDİ: `spawn_batch` homojen (`I::Item` tek tip →
      arketip tüm sütunları kapsar), bu yüzden Rust tip sistemi gereği desync ERİŞİLEBİLİR
      DEĞİL (teorik değişmez, gerçek bug değil). Yine de `Archetype::debug_assert_consistent`
      (debug-only) eklenip spawn_batch'te çağrıldı + 100-entity tutarlılık testi eklendi —
      iç değişiklikler değişmezi sessizce bozarsa yakalanır.
- [ ] **EPA yüz yönelimi** — `compute_face_normal` normali winding yerine origin'den zorluyor;
      sığ/dejenere temaslarda yanlış yüz seçilebilir. (Witness refactor'ı bunu değiştirmedi.)
- [x] **Uyku/kinematik etkileşimi** — ada "uyanık" sayımı artık hareket eden kinematik
      gövdeyi de "mover" kabul ediyor; üstündeki uyuyan dinamik cisim uyandırılıp çözülüyor.
      Test: hareket eden platform uyuyan kutuyu uyandırıp sürüklüyor. (Not: tam ada-uyumu /
      `should_sleep` entegrasyonu ayrı bir iyileştirme; per-body uyku zaten çalışıyor.)
- [x] **`iter_chunks_mut` aşırı işaretleme** — İNCELENDİ: ham `&mut [T]` dilimde hangi
      elemanın yazıldığı izlenemez; "hepsini işaretle" TEMKİNLİ-DOĞRU (gerçek yazmayı
      kaçırmaz; false negative yok). API zaten doğru aracı sunuyor: oku→`iter_chunks`,
      hassas yaz→`iter_mut`, toplu yaz→`iter_chunks_mut`. Davranış belgelendi + chunked
      yazmanın change detection'ı tetiklediğini doğrulayan test eklendi.
- [x] **SparseSet change tracking** — `Changed<T>`/`Added<T>` artık SparseSet bileşenlerinde
      entity'nin saklanan tick'ini okuyor (`ComponentSparseSet::ticks_for`); tablo
      bileşenleriyle aynı kareler-arası semantik. Test: Added/Changed + mutasyon + "değişiklik
      yok" durumları doğru.

Denetlenmemiş alt-sistemleri aynı derinlikte tara (her biri ayrı bug-avı turu):
- [x] FEM soft-body (`gizmo-physics-soft`) — DENETLENDİ+DÜZELTİLDİ: FEM J-cutoff 0.1→1e-4 (geçerli sıkışmış elemanlar artık direniyor, eskiden çöküyordu) + NaN guard; FEM kuvvet toplaması deterministik (paralel hesap, sıralı topla); cloth zemin sürtünmesi aşağı hızı biriktirmiyor (enerji enjeksiyonu); rope zemin çarpışması sabit düğümleri taşımıyor; rope damping sırası; system.rs dt clamp (1/30) — kare sıçramasında patlama yok. Testler: rope pinned-altta, soft-body sıkışma-direnci.
- [x] Multibody / ABA (`gizmo-physics-rigid/multibody`) — DENETLENDİ: spatial cebir (cross_motion/force, inertia, transformlar), 3-pass ABA, gravity injection ve pass-3 q̈ formülleri SAĞLAM. Düzeltilen High bug: Fixed/tekil eklemde (D≈0) çocuğun atalet+bias'ı ebeveyne HİÇ taşınmıyordu (zinciri koparıyordu) → artık tam I^A/p^A taşınıyor, yalnız U·D⁻¹·Uᵀ projeksiyonu D'ye göre düşülüyor. Pendulum testi analitik değere bağlandı (q̈=-4.905, eskiden yalnız |q̈|>0.1). Eksen normalize guard. Testler: analitik pendulum + Fixed-eklem-zincir-koparmıyor (eski kodla FAIL). Bilinen TODO: serbest-yüzen taban (base) entegrasyonu implemente değil; S frame'i rotation_to_parent≠I'de latent.
- [x] Fracture & destruction — DENETLENDİ: çekirdek Voronoi/Cramer/bisector matematiği SAĞLAM. Düzeltilenler: ince kıymık parçalarda `force/mass` patlaması (MAX_EXPLOSION_DV=50 clamp, her iki yol), ConvexHull/fracture parça ataleti artık AABB'den türetiliyor (eskiden sabit 1×1×1 → tüm parçalar aynı atalet). Testler: hull-AABB ataleti, kıymık-hız clamp. Bilinen sınırlamalar (ileri): ince hücrelerde açık-mesh/hacim (quickhull downstream onarıyor), yüz winding tutarlılığı (render), PreFracturedCache extents/seed doğrulamasız (Entity generation reuse'u kısmen koruyor).
- [x] Vehicle modeli (`vehicle.rs`) — DENETLENDİ+DÜZELTİLDİ: tahrik kuvveti friction-circle'a clamp'lendi (eskiden sonsuz çekiş), lastik kuvvetleri temas-yamasında uygulanıyor (eskiden bağlantı noktası), gearbox indeks-güvenli (`update_gear`/`current_ratio`, panik yok), steer `max_steer_angle`'a clamp, tekerlek başına kütle `mass/wheel_count` (eskiden sabit ×0.25). Gearbox birim testleri eklendi. NOT: tam entegrasyon test harness'i (ECS+PhysicsWorld) Faz 1'e bırakıldı.
- [x] Raycast kenar durumları — DÜZELTİLDİ: `ray_aabb` içeriden başlamada artık çıkış yüzeyini döndürüyor (eskiden t=0 → sahte +Y normal); ConvexHull raycast'i AABB yaklaşımı yerine GERÇEK hull üçgenlerine Möller-Trumbore (yüz yoksa AABB fallback). Testler: içeriden-çıkış-normali, hull-tam-test (AABB köşesi ıskalıyor).

**Bilinçli ertelenenler (bug DEĞİL, özellik/ihmal edilebilir):**
- Floating-base ABA entegrasyonu — implemente değil (özellik; Faz 4'e ait, hızlı temizlik değil).
- ABA motion-subspace `S`, rotation_to_parent≠I'de pre-rotation frame'de — büyük olasılıkla non-issue (eksen kendi etrafında dönüşte değişmez); kanıtlanana dek dokunulmadı.
- Fracture ince-hücre açık-mesh (quickhull downstream onarıyor) ve yüz-winding (yalnız render).
- PreFracturedCache extents/seed doğrulamasız (Entity generation, id-reuse'u büyük ölçüde koruyor).

**Çıkış kriteri:** bilinen High/Medium bug yok; her düzeltme bir regresyon testiyle kilitli. → KARŞILANDI.

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
