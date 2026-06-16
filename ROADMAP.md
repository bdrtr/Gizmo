# Gizmo Engine — Production-Ready Yol Haritası

> Hedef: güvenilir, test edilmiş, (gerekirse) deterministik bir simülasyon motoru.
> Bu belge canlıdır — madde tamamlandıkça `[x]` işaretle, **Durum** bölümünü güncelle.

## Durum
- **Şu anki aşama:** Faz 0 TAMAMLANDI (son açık bug EPA yüz yönelimi de kapandı) → Faz 1 (Test & CI) ilerliyor.
- **İlerleme:** ECS+çekirdek fizik (9 bug), vehicle, soft-body, fracture, multibody/ABA, EPA — denetlendi+düzeltildi. **496 test yeşil**, clippy ratchet temiz, determinizm 3/3 hash eşleşiyor.
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
      (Bulgu 2026-06-15: TAM-TEMASLA başlatılan 3-kutu yığını 10 sn kaya gibi sabit; ancak
      boşluklu/düşen 6-kutu yığını çarpma jitter'ıyla 10 sn'de yana kayıp çöküyor — uzun yığın
      kararlılığı warm-starting/sürtünme-birikimi iyileştirmesi gerektiriyor.)
- [ ] CCD (sürekli çarpışma) sağlamlık testleri (tünelleme yok).
- [ ] Joint kütüphanesini tamamla + her tür için test (fixed/hinge/slider/ballsocket/spring + motor/limit).
- [ ] Islands & sleeping sağlamlaştırma (Faz 0 uyku bug'ı sonrası).
- [ ] Geniş sahne performans profili (mimalloc/archetype cache locality doğrulama).

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
