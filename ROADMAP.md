# Gizmo Engine — Production-Ready Yol Haritası

> Hedef: güvenilir, test edilmiş, (gerekirse) deterministik bir simülasyon motoru.
> Bu belge canlıdır — madde tamamlandıkça `[x]` işaretle, **Durum** bölümünü güncelle.

## Durum
- **Şu anki aşama:** Faz 0–4 TAMAMLANDI — solver kalite turu, CCD, joint, materyal combine, **TGS Soft**, **islands & sleeping**, **geniş-sahne perf**, **determinizm (state_hash + süreçler-arası test)**, ve **Faz 3 P2P rollback netcode (snapshot/restore + RollbackSession + Transport + gerçek-UDP client, lag/jitter/loss yakınsama testli)** kapandı. Sıradaki: Faz 5 (renderer-WASM/editor) / Faz 6 (API kararlılığı + 1.0; rb.friction API kararı burada).
- **İlerleme:** ECS+çekirdek fizik (9 bug), vehicle, soft-body, fracture, multibody/ABA, EPA, CCD, joints — denetlendi+düzeltildi. **TGS Soft (Box2D v3 Soft Step) uygulandı → n≥16 yüksek-enerji yığın çökmesi (SI'nin temel sınırı) çözüldü.** **538 test yeşil**, CI clippy temiz, determinizm 3/3 hash eşleşiyor (AAC365945335779E).
- **Faz 1 yeni kapsam (2026-06-15):** broad-phase DIFFERENTIAL test (BVH pairs = brute-force, margin=0 birebir + şişman-margin soundness), joint property (ball-socket yakınsama + zincir kararlılığı), gearbox index-güvenliği property (Faz 0 panik regresyonu), soft-body property (cloth/rope pinned + FEM sıkışma-geri-kazanımı/J-cutoff regresyonu), fracture property (voronoi determinizm + chunk geçerliliği), N-kutu SOAK (10sn kararlılık) + GOLDEN (zeminde dengelenen kutu) regresyon.
- **Sıradaki:** Faz 1 kalanı (benchmark regresyon takibi) VEYA Faz 2 (determinizm kararı) / Faz 3 (netcode client).

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
- [x] **EPA yüz yönelimi** — DÜZELTİLDİ: `compute_face_normal` artık normali saklanan
      winding'den (sağ-el kuralı) alıyor, origin-heuristiği (`normal_raw·v_a < 0`) kaldırıldı.
      Polytope dışa-sarımı yapıca korunuyor: başlangıç tetrahedron'u her yüzü karşı (iç) köşeden
      uzağa sarıyor; genişlemede yeni yüzler yatay-kenar (horizon) yönünü miras alıyor. Origin sığ/
      değiyor temasta bir yüzün dışında kalabildiğinden eski işaret testi normali içe çevirip
      teması yanlış yöne sokabiliyordu. Ayırt edici test (`test_compute_face_normal_follows_winding_not_origin`)
      eski kodda FAIL, yenide PASS; + sığ-temas davranış muhafızı. (gjk.rs)
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

- [x] Her çekirdek algoritmaya birim/property test (GJK/EPA, SAT manifold, solver, integrator, joints,
      ECS, raycast, broad-phase, fracture, soft-body, gearbox, multibody/ABA kapsandı).
- [x] **Property-based testler** — proptest 1.x. İlk 9 (2026-06-12):
      `gizmo-physics-core/tests/proptest_collision.rs` (4) + `gizmo-physics-rigid/tests/proptest_dynamics.rs` (5).
      **+9 (2026-06-15):** `proptest_broadphase.rs` (3) — DIFFERENTIAL: margin=0'da BVH
      `query_pairs` kaba-kuvvetle BİREBİR + şişman-margin soundness (kaçırılan çift yok) +
      self/duplicate-yok; `proptest_joints.rs` (2) — ball-socket anchor yakınsama + yerçekiminde
      eklem-zinciri kararlı/patlamaz; `proptest_gearbox.rs` (1) — tutarsız dizilerle bile
      otomatik vites panik etmez, indeks sınırda, oran sonlu (Faz 0 panik regresyonu);
      `gizmo-physics-soft/tests/proptest_soft.rs` (4) — cloth/rope pinned düğüm sabit + sonlu,
      FEM tet yerçekiminde sonlu, **sıkışmış FEM tet hacmini geri kazanır (J-cutoff 0.1→1e-4 regresyonu)**;
      `proptest_fracture.rs` (2) — voronoi_shatter determinist (BTree fix #7 regresyonu) + chunk
      geçerli (pozitif/sonlu hacim, sonlu köşe). **+5 (2026-06-15, ikinci tur):**
      `gizmo-core/tests/proptest_ecs.rs` (1) — MODEL-TABANLI oracle: rastgele spawn/add/remove/
      despawn dizileri arketip ECS'i referans modelle (canlı sayısı, component değerleri,
      `&A`/`&B`/`(&A,&B)` query kümeleri, stale-handle) BİREBİR eşliyor — arketip göçü + generation
      doğrulaması sağlam çıktı; `gizmo-physics-core/tests/proptest_raycast.rs` (4) — küre analitik
      mesafe+ters-yön-ıska, **OBB rigid-transform DEĞİŞMEZLİĞİ** (ışın+kutu aynı katı dönüşümle → t
      sabit, normal Q ile taşınır), ray_box↔ray_aabb identity tutarlılığı. **+6 (2026-06-15, üçüncü
      tur):** `gizmo-physics-rigid/tests/proptest_multibody.rs` (3) — ABA analitik: tek revolute sarkaç
      `q̈=−m·g·l·sin(q)/(1+m·l²)` HER açıda, tek prismatic `q̈=gravity·axis`, rastgele N-link zincir sonlu;
      `gizmo-physics-core/tests/proptest_sat.rs` (3) — eksen-hizalı penetrasyon = MTV (analitik),
      rastgele-dönmüş örtüşmede normal birim+sonlu & pen>0, bounding-sphere ayrık → boş. Bulgu (eski):
      box_box per-contact penetrasyon face-clip-asimetrik (MTV simetrik — bug değil, dokümante).
      **Üç bug-avı turunun (ECS, raycast, ABA, SAT) hiçbiri yeni bug bulmadı → çekirdek sağlam.**
- [x] **Stres + soak** — `gizmo-physics-rigid/tests/soak_and_golden.rs::soak_box_stack_stays_stable`:
      3-kutu yığını 10 sn (600 kare) — kalıntı hız ~1e-9, yanal sürüklenme ~1e-5, tünelleme/NaN yok.
      (NOT: 6-kutu DÜŞEN yığın 10 sn'de çöküp yana kayıyor → tall-stack warm-starting Faz 4'e yazıldı.)
- [x] **Golden/regresyon** — `soak_and_golden.rs::golden_box_settles_on_ground`: y=5'ten düşen kutu
      zeminde y≈0.5'e, |v|<0.1, yanal<0.05 ile dengelenir (toleranslar cross-platform f32 sapmasını
      soğurur). + `headless_stress_test` (2000-kutu kule) 3-koşu hash eşleşmesiyle determinizmi kilitliyor.
- [x] **CI matrisi** — `.github/workflows/ci.yml`: test (ubuntu/macos/windows × `cargo test --workspace` + gizmo-net feature'lı), lint (rustfmt report-only + `clippy -D warnings` RATCHET — mevcut 17 lint `-A` ile grandfather'lı, yenisi kırar), determinism (headless tower stress). clippy backlog'u TEMİZLENDİ (2026-06-12): `-A` muafiyet listesi 17→2 (kalan `too_many_arguments`/`type_complexity` mimari); 2 gerçek bug yakalandı (lines_filter_map_ok, ölü recursion param). rustfmt tam uyum sonra blocking yapılacak.
- [ ] **Benchmark regresyon takibi** — criterion sonuçlarını CI'da izle.

**Çıkış kriteri:** yeşil CI, anlamlı kapsam, regresyonlar otomatik yakalanıyor.

---

## Faz 2 — Determinizm Kararı  ✅ TAMAM

- [x] **Hedef KARARI:** **aynı-platform replay/rollback** (cross-platform bit-exact KAPSAM DIŞI).
      Çoğu oyun için yeterli; cross-platform bit-exact ayrı/devasa iş (Fp32 göçü) ve gerekmiyor.
- [x] **Test harness (hash eşleşmesi) + docs:** `PhysicsWorld::state_hash()` sync-hash API'si
      eklendi (entity-id sıralı, `to_bits`, sabit-anahtarlı DefaultHasher → süreçler-arası tutarlı;
      rollback desync tespiti + replay için). `crates/gizmo-physics-rigid/tests/determinism.rs`:
      iki özdeş dünya → aynı hash (hash-iterasyon-sırası bağımsızlığı), hash adımla değişir,
      perturbasyon ayrışır (desync). `docs/determinism.md` karara göre kesinleştirildi.
- [~] Cross-platform (Fp32/softfloat) — BİLİNÇLİ ERTELENDİ (hedef aynı-platform; gerekmiyor).
- [x] **Çok-makineli (süreçler-arası) test pipeline'ı:** `demo/src/bin/determinism_oracle.rs`
      (kanonik küçük sahne → `state_hash`) + `demo/tests/cross_process_determinism.rs` oracle'ı
      İKİ/ÜÇ AYRI SÜREÇTE koşup hash'leri karşılaştırır → farklı süreç HashMap taban-seed'ine
      rağmen EŞİT (aynı-binary farklı-makine determinizmi için ön koşul). + mevcut headless 3-koşu.

**Çıkış kriteri:** determinizm vaadi belgeyle uyuşuyor ve testle kanıtlı. → KARŞILANDI.

---

## Faz 3 — Netcode Olgunlaştırma  ✅ (P2P rollback tam; renet client-server opsiyonel)

- [x] **Rollback'i deterministik fizikle ENTEGRE ET** (Faz 2'ye bağlıydı) — `PhysicsWorld::
      snapshot()/restore_snapshot()` (`world.rs`, `WorldSnapshot`): rollback için TAM iç durum
      (transforms+velocities+rigid_bodies/uyku + **contact_cache/warm-start** + accumulator).
      Eski `PhysicsStateSnapshot` (ECS, yalnız transform+velocity+sleep) deterministik
      re-simülasyona YETMİYORDU (warm-start kaybı → sapma). `tests/rollback.rs::
      rollback_resimulation_matches_continuous`: rollback(20)+resim(20→40) BİT-BİT == kesintisiz
      sim (state_hash eşit) → tam durum geri yükleme doğru.
- [x] **Lag/jitter/packet-loss simülasyonuyla otomatik test** — `tests/rollback.rs::
      rollback_netcode_converges_under_lag_jitter_loss`: kontrollü cisim + per-tick girdi;
      her girdi LAG=5 tick geç öğrenilir (en kötü hal → her tick rollback), yanlış-tahmin
      düzeltilir, döngü sonu kalan girdiler toplu teslim + son rollback. Peer "ground truth"
      peer'e YAKINSAR (state_hash eşit) = senkron. GGPO rollback döngüsü deterministik çalışıyor.
- [x] **Uçtan uca P2P rollback client** (transport katmanı) — `Transport` trait (`rollback/
      session.rs`): gerçek `UdpTransport` + test `LoopbackTransport` (lag + paket-kaybı sim) aynı
      kodu paylaşır. **`RollbackSession<T>`**: PhysicsWorld-native GGPO döngüsü (resend penceresiyle
      paket-kaybına dayanıklı; yanlış-tahminde deterministik snapshot ile rollback+resim).
      `examples/p2p_rollback_test.rs` ESKİ ECS manuel döngüden (warm-start kayıplı) yeni
      RollbackSession + gerçek UDP'ye yeniden yazıldı (çalışan istemci; iki süreç localhost'ta
      onaylı geçmişte senkron — doğrulandı). NOT (ileri/opsiyonel): renet client-server backend
      (NetworkClient/ClientPredictor/SnapshotInterpolator) ayrı bir mimari olarak duruyor;
      P2P-rollback yolu bu projenin determinizm temeliyle tam entegre.

**Çıkış kriteri:** iki client gerçekçi ağ koşullarında senkron kalıyor → KARŞILANDI
(`two_peers_converge_under_lag_and_packet_loss`: lag+kayıp altında state_hash-eşit yakınsama;
gerçek-UDP örnek onaylı geçmişte senkron).

---

## Faz 4 — Fizik Derinliği & Kalite

- [~] **Solver kalite turu: warm-starting doğrulama, sub-stepping ayarı, manifold kararlılığı**
      — DENETLENDİ + KISMEN DÜZELTİLDİ (2026-06-17). Çöküşün kök nedeni bulundu: mükemmel
      hizalı kutu yığını **metastable**; ileri-tek-yönlü PGS manifoldun 4 temas noktasını sabit
      sırada işleyip her çarpmada küçük bir merkez-dışı (dönme) yanlılığı bırakıyor → çarpma
      anında açısal hız tohumlanıp yığını deviriyor (kanıt: çarpmada `maxang` 0→3.1 sıçrıyor).
      **Düzeltmeler:** (1) **simetrik Gauss-Seidel** — solver iterasyonda tarama yönünü değiştirir
      (`solver.rs`), yön-yanlılığını iptal eder; (2) **kalıcı temaslarda restitution bastırma**
      — yerleşmiş/yığın teması (lifetime>0) sekme enerjisini her substep yeniden enjekte etmesin
      (`pipeline.rs`; sıçrayan top hâlâ sekiyor çünkü temas kopunca lifetime sıfırlanır). Sonuç:
      orta-boşluklu yığınlar (n≤~12, gap≤~0.2) artık varsayılan 20 iterasyonda çarpmayı atlatıp
      dik kalıyor (eskiden çöküyordu). Regresyon: `soak_and_golden.rs::soak_falling_stack_survives_impact`
      (n=8, gap=0.1 düşen yığın) — AYIRT EDİCİ: SGS kapatılınca DÜŞÜYOR. **Yan bulgu + düzeltme:**
      `test_coulomb_friction_and_sleeping` kusurluymuş — `RigidBody::friction` alanını değiştiriyordu
      ama temas sürtünmesi collider MATERYALİNDEN gelir (`sqrt(mat_a.dyn·mat_b.dyn)`), rb.friction
      temas çözücüye HİÇ ulaşmaz; iki kutu da aynı mesafeyi gidip test sub-mm gürültüyle geçiyordu.
      Test gerçek materyal sürtünmesini kullanacak + anlamlı marj (≈23 m'ye karşı ≈5 m) test edecek
      şekilde düzeltildi. Doğrulama: workspace 528 test yeşil, clippy ratchet exit 0, determinizm
      3/3 tutarlı (yeni hash **4F4A5BE6569A6ED4** — solver sırası değişti).
- [x] **TGS Soft çözücü (uzun-yığın çözümü)** — UYGULANDI (Box2D v3 "Soft Step" uyarlaması,
      `solver.rs`). SI'nin temel sınırı (n≥16 yüksek-enerji çarpan yığınların kaotik çökmesi)
      kapatıldı. Her substep'te: warm-start → BIASED soft solve + **iterasyonlar-arası
      pozisyon-delta entegrasyonu** (gerçek TGS: dp güncel hızla ilerler, bir sonraki
      iterasyonun bias'ı GÜNCEL penetrasyonu görür → düzeltme yığın boyunca yayılır) → RELAX
      (bias=0) + sönümlü restitution → pozisyon düzeltmesi `dp − relaxed·dt` olarak dışarı.
      Soft katsayılar `contact_hertz=30`/`damping_ratio=10`'dan. `use_tgs_soft` (varsayılan
      açık) ile gate'li; eski split-impulse yolu fallback. **CCD-etkin island'lar eski yolu
      kullanır** (speculative temaslar ince ayarlı; TGS dp/relax akışı yüksek-hızlı açılı
      çarpmada speculative clamp'le çatışıp tünelletiyordu). Ayırt edici regresyon:
      `soak_and_golden.rs::soak_tall_stack_n16_stays_upright` (n=16 düşen yığın, restitution-0
      materyal — propagation'ı izole eder; eski SI'de ÇÖKER, TGS'te dik kalır). Doğrulama:
      workspace **538 test yeşil**, CI clippy exit 0, determinizm **3/3 tutarlı (yeni hash
      AAC365945335779E)**. **KALAN (ileri iş):** yüksek-restitution (≥0.3) çok uzun yığınların
      çarpma sekmesi hâlâ kaotik (her motorda fiziksel olarak öyle; ayrı konu) — restitution-0
      / düşük materyalde kararlı.
- [x] **Materyal combine modları** — DÜZELTİLDİ (2026-06-17). Pipeline manifold sürtünme/sekme'yi
      `sqrt(dyn·dyn)` + `restitution.max` ile HARDCODE ediyordu → her materyalin
      `friction_combine`/`restitution_combine` modu (ICE=Min, RUBBER=Max, …) YOK SAYILIYORDU.
      Artık `PhysicsMaterial::combine()` kullanılıyor. Varsayılan materyal (GeometricMean sürtünme,
      Max sekme) için BİREBİR aynı → determinizm hash `4F4A5BE6569A6ED4` DEĞİŞMEDİ (default-nötr);
      yalnız özel modlu materyaller düzeldi. `CombineMode` crate kökünden export edildi. Ayırt edici
      test (`world.rs::test_material_combine_modes_respected`): Max-combine kutu düşük-sürtünme
      zeminde ~5-6 m'de durur (eski hardcode ~17 m kaydırırdı).
- [ ] **(Faz 6 / API) `RigidBody::friction`+`restitution` alanları temas çözücüde YOK SAYILIYOR**
      — kaynak doğruluğu collider `material`'dir; rb alanları yalnız editor UI + fracture
      propagation'da okunuyor. Ya materyale köprülenmeli ya kaldırılmalı (köprüleme varsayılanları
      kaydırır: rb default 0.5 vs material default static 0.6/dyn 0.5 → davranış değişir; bu yüzden
      şimdi yapılmadı, API kararı Faz 6'ya bırakıldı).
- [x] **CCD (sürekli çarpışma) sağlamlık testleri (tünelleme yok)** — DENETLENDİ+DÜZELTİLDİ.
      Speculative-contact CCD vardı ama 3 bug'ı kapatıldı: (1) `Gjk::speculative_contact`
      temas noktasını `penetration = 0` ile üretiyordu → solver merminin hızını BAŞLANGIÇ
      konumunda sıfırlıyor, mermi duvardan metrelerce ÖNCE donuyordu ("hayalet duvar").
      Düzeltme: temas, ayrılma boşluğunu **negatif penetration** (`-allowed_close`) olarak
      taşıyor; solver'da zaten var olan `penetration < 0 ⇒ bias = gap/dt` yolu cismin o adımda
      yüzeye TAM kadar (SKIN=1cm payla) ilerleyip durmasını sağlıyor — ne tünel ne donma.
      (2) Temas noktası A'nın (statik duvar) merkezine demirleniyordu → uzaktaki dinamik
      mermi için devasa kaldıraç kolu (`k_n≈596`), impuls cılız, durdurmuyordu → **ters-kütle
      ağırlıklı merkeze** demirlendi (dinamik-vs-statik'te dinamik cismin COM'una düşer,
      `r×n=0`, saf doğrusal durdurma); bunun için `speculative_contact` imzasına `inv_mass_a/b`
      eklendi. (3) Restitution speculative boşluk-kapatmaya uygulanıyordu (varsayılan materyal
      e=0.3) → bias bozuluyor, çok-substep'te (240Hz, kare başına 4 substep) mermi son substep'te
      yüzeyi aşıp giriyordu → solver'da `penetration < 0` iken `e=0` (sekme gerçek temasta,
      penetration≥0'da uygulanır). **8 yeni test** (`tests/ccd.rs`): çok-hızlı head-on tünel-yok
      + ön-yüzde durma, açılı/offset impact, temiz-yol/ayrılan cisim yanlış-durdurulmaz, iki
      CCD cismi iç-içe geçmez, CCD küre zeminde normal yerleşir + **proptest** (rastgele
      hız/duvar-kalınlığı/yarıçap → asla tünel/penetrasyon). Ayırt edici: eski ghost-freeze
      bug'ı testleri DÜŞÜRÜYOR. Determinizm hash D96110593C3394F7 değişmedi (CCD yolu izole).
- [x] **Joint kütüphanesi + her tür için test (fixed/hinge/slider/ballsocket/spring + motor/limit)**
      — DENETLENDİ + 2 BUG DÜZELTİLDİ (2026-06-17). 5 tür de davranışsal olarak test edildi
      (`tests/joints_behavior.rs`, 8 test). **Bug 1 — Fixed joint dönüşü KİLİTLEMİYORDU:**
      `solve_fixed_joint` yalnız 3 doğrusal (anchor-pin) kısıt uyguluyordu → "Fixed" aslında
      ball-socket gibi serbest dönüyordu (B spin'i 3 rad/4.5 rad·s⁻¹ ile devam ediyordu). Düzeltme:
      bağıl açısal hızı 3 eksende sıfırlayan velocity-lock eklendi (yalnız `JointData::Fixed` iken;
      solver hinge/ballsocket pozisyon aşamasıyla paylaşıldığından gate şart). AYRICA erken-`return`
      (anchor çakışıksa) açısal kilidi atlıyordu → doğrusal kısım `if err_len>=ε` ile sarıldı, kilit
      her zaman çalışıyor. Offset yerçekimi yükü altında 5 sn weld testi geçiyor (drift yok). **Bug 2
      — Slider LİMİTİ tutmuyordu:** limit impulse-clamp sınırları çalışan hinge'in TERSİNE takılmıştı
      (alt-limit err>0 için `(−∞,0)` ama `(0,+∞)` olmalı; üst tersi) → 5 m/s'lik cisim 1 m'lik üst
      limiti delip 19.6 m'ye gidiyordu; clamp'ler swap'lendi → 1.002 m'de duruyor. Temiz çıkanlar:
      hinge limit+motor, slider motor, ballsocket cone limit, spring (Hooke + damping + min/max).
      Doğrulama: workspace 537 test yeşil, clippy exit 0, determinizm hash `4F4A5BE6569A6ED4` DEĞİŞMEDİ
      (stress testi joint kullanmıyor). NOT (ileri): Fixed weld velocity-lock'tur (pozisyon-bias yok);
      sürekli ağır yükte mikro-drift olabilir — gerekirse initial-relative-rotation saklayıp Baumgarte
      eklenir (Faz 6 polish).
- [x] **Islands & sleeping sağlamlaştırma** — DENETLENDİ (3 paralel subagent) + 5 gerçek bug
      DÜZELTİLDİ (her biri regresyon testiyle, `tests/sleeping.rs`). **ASIL bug — ada-uyumsuz
      uyku:** per-body uyku + island-wake birlikte ping-pong KİLİDİ yapıyordu → dinlenen bir
      yığın (|v|=0 olsa bile) ASLA uyumuyordu (bir kutu uyur uymaz ada hâlâ "uyanık" komşu
      içerdiğinden `wake_updates` onu geri uyandırıyordu). Çözüm (`pipeline.rs`): "çöz" kapısı
      (`island_active`: uyanık dinamik/hareketli kinematik) ile "uyandır" kapısı (`island_has_mover`:
      yalnız eşik ÜSTÜ hızlı/`!can_sleep`) AYRILDI → ada topluca uyuyabilir; gerçek hareketli
      (düşen kutu vb.) yine uyandırır. **+ apply_impulse/apply_force** `&mut RigidBody` alıp
      `wake_up()` çağırır (eskiden `&` → uyuyan cisme impuls SESSİZCE yutuluyordu). **+ joint-coupled
      wake** (pipeline joints öncesi: bir ucu hareketli eklemin uyuyan dinamik ucu uyandırılır;
      joint_solver `&[RigidBody]` ile uyandıramıyordu). **+ island inşa SIRASI deterministik**
      (`island.rs`: HashMap `into_values` süreç-bağlı → min-indise göre sıralanır). Doğrulama:
      workspace **543 test yeşil**, CI clippy exit 0, determinizm 3/3 (yeni hash 9ED99A65E026DD68 —
      cisimler artık uyuduğundan). NOT: `should_sleep`/`Island.sleeping` hâlâ ölü kod (zararsız,
      ileride island-seviye uyku için bırakıldı).
- [x] **Geniş sahne performans profili** (mimalloc/archetype cache locality doğrulama) —
      YAPILDI. (1) **PhysicsMetrics zamanlaması BAĞLANDI** (`world.rs step_internal`): aşama
      başına `Instant` (broadphase/narrowphase/solver/integration ms) + body/sleeping/contact
      sayımları artık dolduruluyor (eskiden struct vardı ama HİÇ ölçülmüyordu — ölü profilleme);
      simülasyon sonucunu etkilemez (determinizm korunur). (2) **Profil binary'si** eklendi
      (`demo/src/bin/wide_scene_profile.rs`, mimalloc global allocator) — geniş yerleşmiş sahne
      (ayrık sütunlar) profili. (3) **mimalloc fizik yüküne BAĞLANDI** (eskiden yalnız
      gizmo-studio; profil binary'si artık kullanıyor). (4) Profilin ortaya çıkardığı DARBOĞAZ:
      narrowphase (kaotik düşen sahnede zamanın ~%82'si) → **dormant-çift narrowphase ATLAMA**
      (`pipeline.rs`): iki cisim de dormant (statik/uyuyan/hareketsiz-kinematik) ise GJK/SAT
      atlanır, contact-cache KORUNUR (ended-collision sahte wake yok); en az biri aktifse normal
      narrowphase → düşen/itilen cisim uyuyan komşuyu uyandırır. **Sonuç (1281 cisim, 400 frame):**
      uyku %0→%50-66 monoton artar, erken→geç frame **1.7× hızlanma**, ~13.6 ms/frame (73 FPS),
      yerleşmiş sahnede aşama dağılımı solver %41 / narrowphase %30 / broadphase %24. Archetype
      bitişik-kolon ECS + mimalloc ile geniş sahne ms-ölçekli. Doğrulama: workspace 543 test
      yeşil, CI clippy exit 0, determinizm 3/3 (yeni hash 598E315D0E7499FF). **Faz 4 TAMAM.**

---

## Faz 5 — Renderer & Araçlar

- [~] Renderer denetimi (`gizmo-renderer`) — CPU-tarafı bug-avı BAŞLADI (2026-06-15, 3 paralel
      subagent + elle doğrulama). **BÜYÜK BUG bulundu + düzeltildi:** prosedürel mesh üreticilerinin
      ÇOĞU üçgenleri declared outward normalin TERSİNE sarıyordu → varsayılan `Ccw + Back-cull`
      pipeline'ında (deferred.rs:336-337, pipeline.rs:579-580) yüzler back-face sayılıp culllanıyor,
      yani şekiller "içi-dışına"/görünmez render oluyordu. cube/torus/arrow/terrain doğruydu;
      sphere/cylinder/cone/capsule/plane/circle/tetrahedron/conical_frustum TERSTİ. 8 fonksiyon saf
      `*_data()` fonksiyonlarına ayrılıp winding düzeltildi; sphere+capsule kutuplarındaki dejenere
      üçgenler de kaldırıldı. **9 winding-tutarlılık testi** eklendi (geo-normal·declared-normal>0,
      birim-normal, dejenere-yok); discriminating (eski winding'de FAIL, düzeltmede PASS) kanıtlandı.
      **Kamera/view-projection + frustum culling DENETLENDİ → temiz** (Gribb-Hartmann plane çıkarımı
      Z∈[0,1] formunda doğru, view matrisi RH look_at, p/n-vertex AABB testi doğru). Animasyon minör
      bulgular (düzeltilmedi, düşük öncelik): (1) `animation_system.rs` blend sırasında `prev_time += dt`
      hız çarpanı uygulamıyor (speed≠1'de crossfade yanlış hızda); (2) negatif speed (ters oynatma)
      `%=` ile sarmıyor (`rem_euclid` gerek). **Asset/glTF/OBJ loader DENETLENDİ (subagent + elle):
      3 bug düzeltildi** — (HIGH) skin joint weight'leri normalize edilmiyordu; shader `Σwᵢ·Mᵢ`'yi
      renormalize etmediğinden 1'e toplanmayan weight'ler (quantization/export) mesh'i ölçekleyip
      bozuyordu → `normalize_skin_weights` (sum>0 iken normalize, `[0,0,0,0]` skinless dokunulmaz;
      3 birim test); (MED) indexed glTF'te OOB indeks tek tek atlanınca sonraki üçgenlerin gruplaması
      kayıyordu → 3'erli işlenip OOB üçgen komple atlanıyor; (LOW) R8G8 doku luminance+alpha gibi
      açılıyordu → gerçek (R,G,0,255). TRS/joint-remap/IBM/handedness/OBJ/normal-tangent-recalc/
      animasyon-parse temiz çıktı. **GPU pipeline/shader DENETLENDİ (2 subagent + elle):**
      CPU↔GPU arayüzü (SceneUniforms/LightData/InstanceRaw/Vertex std140 layout, vertex-attribute
      location/format, array stride'ları) byte-byte TEMİZ — `Vertex` offset_of! assertion'larıyla
      kilitlendi + `core_shaders_compile` testi (shader.wgsl/gbuffer/deferred_lighting'i headless
      device'ta naga ile doğruluyor). Shader mantığında **1 bug düzeltildi:** skinned normal,
      skin matrisinin inverse-transpose'u yerine ham matrisle çarpılıyordu (model kısmı doğruydu)
      → non-uniform bone scale/shear'da normal kayıyordu; `inverse_transpose_3x3(skin3)` uygulandı
      (rigid/uniform'da no-op — fragment'ta normalize edilir). PBR BRDF (D_GGX/V_SmithJoint/F_Schlick),
      CSM/point shadow bias, sRGB/tonemap, TBN, attenuation temiz çıktı. (Not: normal-mapping TBN
      döşeli ama normal-map dokusu hiç örneklenmez — eksik ÖZELLİK, bug değil.) **İleri
      post-process shader'ları DENETLENDİ (subagent + elle): 2 bug düzeltildi** — (1) DoF derinlik
      linearizasyonu OpenGL `[-1,1]` formülü kullanıyordu (`(2n)/(f+n-d(f-n))`) ama wgpu `[0,1]`
      yazıyor → `n·f/(f-d(f-n))` (post_process.wgsl; DoF varsayılan kapalı ama matematik artık doğru);
      (2) SSGI hemisphere taban `up`-vektör seçimi `abs(normal.z)<0.999` ile yanlıştı → ±X'e bakan
      normallerde `up`=(1,0,0) paralel olup `cross=0` → NaN tangent → kara SSGI; `abs(normal.y)>0.999`
      ile düzeltildi (ssgi.wgsl). SSAO/SSR/TAA/FXAA/volumetric/blur/apply pasları temiz (depth/pozisyon
      reconstruction, reprojection, tonemap-tek-sRGB doğru). 25 standalone render/post shader artık
      `core_shaders_compile` testinde naga ile doğrulanıyor. KALAN: compute/fluid shader'ları.
- [~] WASM hedefi — **SİMÜLASYON ÇEKİRDEĞİ ✅ (2026-07-01), renderer/pencere/net ERTELENDİ.**
      Deterministik sim çekirdeği artık `wasm32-unknown-unknown`'a derleniyor + CI'da doğrulanabilir
      (`cargo build --target wasm32-unknown-unknown -p <crate>`): gizmo-math/core/physics-core/
      physics-rigid/physics-soft/ai/animation/scene (8 crate, 0 uyarı). Çözülen bloker'lar (hepsi
      **native'i BİT-AYNI** tutuyor — determinizm hash 57FA0A2E8313B7A2 değişmedi, tüm testler yeşil):
      (1) `rayon` non-wasm'e target-gate'lendi + wasm'de `parallel_compat` sıralı shim (sıra korunur →
      davranış/determinizm nötr); physics-core'un kullanılmayan rayon dep'i silindi. (2) `uuid` js +
      `rand 0.10→getrandom 0.4` wasm_js feature + `.cargo/config.toml`'da `getrandom_backend="wasm_js"`
      cfg (wasm32-scoped). (3) physics `step.rs` `std::time::Instant`→`web_time::Instant` (wasm); diğer
      time siteleri zaten cfg-ayrık. (4) `gizmo-ai` pathfinding `std::thread::scope`→wasm'de tek-thread
      fallback (native threaded yol dokunulmadı). **ERTELENEN (gerçek web backend gerekir, cfg değil):**
      audio (web-audio), gizmo-net (`std::net` UDP → WebSocket/WebTransport), scripting (mlua/C Lua).
      **RENDERER + PENCERE ✅ (2026-07-02) — MOTOR TARAYICIDA ÇALIŞIYOR.** `gizmo-renderer` +
      `gizmo-app` (render,physics,scene) + facade (`gizmo-engine` web feature alt-kümesi:
      window,render,physics,physics-dynamics,physics-soft,scene,animation,ui) wasm32'ye derleniyor;
      **`demo-web/`** (wasm-bindgen cdylib + index.html) headless Chrome'da **uçtan uca doğrulandı**:
      BrowserWebGpu adapter, 90 kare render, canvas piksel analizi 69 farklı renk (canlı sahne),
      sıfır validation hatası, düşen fizik küpleri ekran görüntüsünde havada. Yapılanlar:
      (1) `gizmo-app` wasm `resumed` async init — `Renderer::new` (async WebGPU adapter/device)
      `spawn_local`'da, sonuç `PendingWebInit` slotuyla ilk uyanışta `finish_initialize`'a teslim
      (native `pollster` yolu aynı `finish_initialize`'ı paylaşır). (2) `gizmo-scripting` (mlua)
      non-wasm'e target-gate'lendi — `scene` feature web'de Script bileşeni kaydı olmadan çalışır.
      (3) Facade forward geçidi web 4-grup şemasına uyarlandı (tarayıcı WebGPU maxBindGroups=4,
      ampirik doğrulandı): BG_SKELETON/BG_INSTANCE cfg sabitleri, gölge geçitleri + gölge bind'i
      web'de atlanır (`load_shader_web` shadow örneklemesini shader'dan zaten söküyor, eski web
      hazırlığı). (4) Küçük düzeltmeler: async_assets wasm OBJ yolu `AssetError` döndürür, tüm
      wasm-cfg uyarıları hedefli allow/cfg ile sıfırlandı. CI `wasm` job'ı artık grafik yığınını +
      `demo-web`'i de derliyor. Native BİT-AYNI: 552+ test yeşil, determinizm hash değişmedi.
- [x] **Editor/studio sahne kaydet/yükle GÜVENİLİRLİĞİ** — round-trip regresyon testi
      (`scene.rs::scene_save_load_roundtrip_preserves_components_and_hierarchy`): isimli ebeveyn+çocuk
      + Transform değerleriyle dünya RON'a KAYDEDİLİP TAZE dünyaya YÜKLENİNCE bileşen değerleri
      (reflect serialize↔deserialize) + ebeveyn-çocuk hiyerarşisi (id remap) KORUNUYOR. Save/load
      sistemi (registry + bevy_reflect) sağlam çıktı. (Prefab join serileştirme zaten test'liydi.)
      KALAN (ileri): inspector UI güvenilirliği (gizmo-studio, GUI — otomatik test zor).

---

## Faz 6 — API Kararlılığı & 1.0

> **Strateji:** Tüm 1.0 yayın stratejisi (kademeli/STAGED 1.0, crate-bazlı
> hazırlık tablosu, dış-tip kontratı, kalan-iş kontrol listesi, yayın sırası)
> [`RELEASING.md`](RELEASING.md) dosyasında. Motor 1.0'a **HENÜZ HAZIR DEĞİL**;
> net bir kademeli yol var (önce dış-bağımlılığı hafif çekirdek = Stage A,
> sonra grafik katmanı = Stage B).

### Tamamlanan 1.0-hazırlık denetim turları (205 bulgu / 26 bloker)
- [x] **Tur 1 — Güvenli/non-breaking sertleştirme** (`b39e082`): paketleme
      (repository URL, LICENSE×2, `publish=false`, keywords/categories) +
      temizlik (88 `Debug`, 12 bağımlılık, 7 ölü-kod) + doc.
- [x] **Tur 2 — `#[non_exhaustive]`** (`2fdbe6c`): 96 açık public tipe semver
      koruması.
- [x] **Tur 3a — İmza-değişmeyen hata sertleştirme + trait sealing** (`f15d262`):
      `Error`+`Display` impl'leri, 38 panik/NaN guard, `Fp32` tutarlılık,
      `WorldQuery`/`SystemParam`/`FetchComponent` sealing.
- [x] **Tur 3b — Breaking hata kontratı** (`613d140`): 13 somut `Error` enum'u,
      46 `fn → Result` + tüm çağrı yerleri.
- [x] Hepsi build + `--all-features` + 552 test + clippy(`-D warnings`) yeşil,
      `main`'de.

### Stage-A-sealing turu (2026-06-25) — (a)+(b)+(e)+(f) TAMAM
- [x] **(a) `bevy_reflect` 'reflect' feature-gating** — *Stage A 1.0 blokeri,
      denetim blokeri #1* — **L**. `Transform`/`RigidBody`/`Velocity` derive'ları
      (core/physics-core/physics-rigid/scene) `#[cfg_attr]` ile **off-by-default**
      workspace `reflect` feature'ı arkasında; scene `serde_bridge` ile reflect↔
      serde fallback (her iki yol round-trip testli). Default API'de artık
      `bevy_reflect` yok.
- [x] **(b) `arrayvec` opak newtype** — *Stage A 1.0 blokeri, #2* — **M**.
      `CollisionEvent.contact_points` artık opak `ContactPoints`; `arrayvec`
      public API'den çıktı, physics-rigid direkt dep'ten kaldırıldı.
- [x] **(e) `glam`'i resmi public dep olarak belgele** — **S**. `gizmo-math`
      artık doğrudan `pub use glam::{…}` (bevy_math yerine) + crate doc'unda
      public-dep notu.
- [x] **(f) MSRV belirle** — **S**. `rust-version = "1.89"` (ampirik doğrulandı:
      1.82/1.85 başarısız, 1.89 yeşil) + CI `msrv` job. Ek: CI `features` job
      (reflect-ON Stage A testleri).
- [x] Doğrulama: build (default + `--all-features`) + 552 test + gizmo-net
      feature testi + reflect-feature testleri + CI clippy (stable, default+reflect)
      + determinizm 3/3 (`598E315D0E7499FF`, fizik DEĞİŞMEDİ) + MSRV 1.89 build.

### (d) get_ rename TAMAM (2026-06-25, Bevy-konvansiyonuyla kapsamlandı)
- [x] **(d) `get_` rename (C-GETTER)** — **M**. KRİTİK nüans: motor Bevy'i model
      alıyor, Bevy `get_`'i FALLIBLE erişimciler için tutar (`get_resource`→Option
      vs `resource()`→panik). `get_*`'ların çoğu Option/Result dönüyor veya
      collection `get`/`get_mut` → **kasıtlı, korundu** (get_resource ×173,
      get_entity ×41 dahil). Yalnız gerçek infallible düz-değer getter'ları
      yeniden adlandırıldı: get_neighbors/get_entity_component_types/get_log_version/
      get_engine_torque/get_entity_names → get_'siz. Saf rename, 552 test+clippy+
      determinizm (hash değişmedi) doğruladı.
- [~] **Görünürlük daraltma** — gizmo-animation→gizmo-app daraltması YAPILAMADI:
      `gizmo_app::Plugin`/`App`'i implement ediyor (load-bearing); Plugin/App'i
      çekirdek crate'e çıkarmak ayrı mimari iş → gizmo-animation Stage B'de kaldı.
      Geniş `pub` daraltması = ince denetim takibi (ertelendi).

### Kalan 1.0 blokerleri (bkz. `RELEASING.md` §4 kontrol listesi)
- [ ] **(c) `wgpu`/`winit`/`egui` güncel sürüme yükseltme** — *Stage B 1.0
      blokeri* — **XL**. `wgpu 0.20→güncel`, `winit 0.29→güncel`,
      `egui 0.28→güncel` (+ egui ekosistemi). Tüm grafik katmanı + `gizmo`
      facade buna bağlı.
- [ ] **Not (gizmo-math bağımlılık hijyeni, opsiyonel):** gizmo-math `bevy_math`/
      `bevy_picking`/`bevy_mesh` dep'leri `bevy_reflect`'i transitif çekiyor;
      public tip glam olduğundan API sızıntısı yok ama dep ağacından çıkarmak
      ayrı iş (bkz. RELEASING.md §3 notu).

### Yayın mekaniği
- [ ] Kademeli sürümler tek-workspace-versiyon varsayımını kırar: Stage A `1.x`,
      Stage B `0.y` (`RELEASING.md` §5). `publish_all.sh` / version inheritance
      güncellenmeli.
- [ ] Genel API'yi dondur; `unsafe` kontratlarını belgele (Stage A için).
- [ ] CHANGELOG + migration kılavuzu.
- [ ] **Stage A `1.0.0`** ((a)+(b)+(d)+(e)+(f) tek breaking turda).
- [ ] **Stage B `1.0.0`** ((c) sonrası, ayrı/sonraki yayın).

---

## Çalışma Yöntemi
- Her madde: **düzelt → regresyon testi yaz → derle/test/clippy → işaretle.**
- Davranış değiştiren fizik düzeltmelerini `headless_stress_test` + odaklı senaryolarla doğrula.
- Bug-avı turlarında subagent fan-out kullan, sonra her bulguyu elle doğrula (false-positive'leri ele).
</content>
