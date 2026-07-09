# Gizmo Engine — Production-Ready Yol Haritası

> Hedef: güvenilir, test edilmiş, (gerekirse) deterministik bir simülasyon motoru.
> Bu belge canlıdır — madde tamamlandıkça `[x]` işaretle, **Durum** bölümünü güncelle.

## Durum
- **Şu anki aşama:** Faz 0–4 TAMAMLANDI — solver kalite turu, CCD, joint, materyal combine, **TGS Soft**, **islands & sleeping**, **geniş-sahne perf**, **determinizm (state_hash + süreçler-arası test)**, ve **Faz 3 P2P rollback netcode (snapshot/restore + RollbackSession + Transport + gerçek-UDP client, lag/jitter/loss yakınsama testli)** kapandı. Sıradaki: Faz 5 (renderer-WASM/editor) / Faz 6 (API kararlılığı + 1.0; rb.friction API kararı burada).
- **İlerleme:** ECS+çekirdek fizik (9 bug), vehicle, soft-body, fracture, multibody/ABA, EPA, CCD, joints — denetlendi+düzeltildi. **TGS Soft (Box2D v3 Soft Step) uygulandı → n≥16 yüksek-enerji yığın çökmesi (SI'nin temel sınırı) çözüldü.** **538 test yeşil**, CI clippy temiz, determinizm 3/3 hash eşleşiyor (AAC365945335779E).
- **Faz 1 yeni kapsam (2026-06-15):** broad-phase DIFFERENTIAL test (BVH pairs = brute-force, margin=0 birebir + şişman-margin soundness), joint property (ball-socket yakınsama + zincir kararlılığı), gearbox index-güvenliği property (Faz 0 panik regresyonu), soft-body property (cloth/rope pinned + FEM sıkışma-geri-kazanımı/J-cutoff regresyonu), fracture property (voronoi determinizm + chunk geçerliliği), N-kutu SOAK (10sn kararlılık) + GOLDEN (zeminde dengelenen kutu) regresyon.
- **Sıradaki:** **Faz 0–5 + Faz 1 (benchmark CI dahil) TAMAM.** Grafik yükseltmesi
      (wgpu 23/winit 0.30/egui 0.34, Faz 6(c)) de yapıldı. Kalan: Faz 6 yayın mekaniği
      (kademeli sürüm, CHANGELOG, Stage A 1.0) + opsiyonel gizmo-net WASM.

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
- [x] **Benchmark regresyon takibi** — criterion sonuçlarını CI'da izle. **YAPILDI
      (2026-07-03):** CI'ye `benchmarks` job'ı eklendi — `cargo bench --workspace
      --benches -- --test` her benchmark'ı criterion'ın **test modunda BİR KEZ**
      koşar (yavaş zamanlama-ölçümü yok → CI-donanımı gürültüsüne dayanıklı;
      derleme-rot + çalışma-zamanı panik/assertion yakalar; `--all-targets` clippy
      zaten derliyordu, bu ÇALIŞTIRMAYI da ekler). **Gate ilk koşuda 3 gerçek sorun
      yakaladı:** (1) **motor bug'ı** — `spawn_batch` SparseSet bileşen içeren
      bundle'da 2. entity'de panikliyordu (`write_to_archetype`'ın arketip-sütun
      hızlı yolu sparse'ı yönlendirmez); fix: bundle'da sparse varsa entity-başına
      `spawn_bundle`'a düş (all-table'da O(1) hızlı yol korunur) + regresyon testi +
      eyleme-dönük panik mesajı. (2)+(3) **iki bench bug'ı** — `none_changed_detection`
      + `multiple_archetype_none_changed`, değişiklik-frame'ini `increment_tick()` ile
      ilerletmeye çalışıyordu ama o `change_ref_tick`'i taşımaz → `Changed<T>` tüm
      taze-spawn'ları raporlayıp `assert_eq!(0,count)`'u kırıyordu; `begin_change_frame`
      (gerçek frame-sınırı primitifi) ile düzeltildi. **Not:** tam zamanlama-regresyonu
      (critcmp/baseline) CI donanımında gürültülü → kasıtlı kapsam dışı; bu gate
      "bench'ler derlenir VE panik/assertion'sız koşar" garantisidir.

- [x] **Soundness/tutarlılık bakım turu (2026-07-06, adversarial subagent + Miri):**
      (1) **SparseSet `Mut<T>` fetch UB (HIGH)** — `query::fetch::fetch_raw` SparseSet yolu
      `&World`'ü `*mut World`'e cast edip `sparse_sets.get_mut()` çağırıyordu (SharedReadOnly→
      mutable retag = aliasing UB, `query_mut::<Mut<Sparse>>().iter_mut()` ile GÜVENLİ koddan
      ulaşılabilir; `par_for_each_mut`'ta gerçek veri yarışı). Fix: paylaşımlı `get()` + set'e
      `&self` erişim; ayrıca `ComponentSparseSet.ticks` `Vec<ComponentTicks>`→`Vec<UnsafeCell<
      ComponentTicks>>` (düz Vec `as_ptr(&self)` yalnız-okuma provenance verip `&mut *ticks_ptr`'i
      ikinci bir latent UB yapıyordu; `dense: BlobVec` zaten ham-pointer'lı içsel-değişebilir).
      `unsafe impl Send+Sync for ComponentSparseSet`. **Miri (Tree Borrows) ile temiz doğrulandı**
      (`sparse_set_change_detection_tracks_ticks` + `query::tests` yeşil). (2) **`Fp32::from_i32`
      wrap→saturate** — `val << SHIFT` |val|≥32768'de sessizce sarıyordu (diğer tüm op saturate
      ederken; `from_i32(40000)`→negatif); `saturating_mul(ONE_RAW)` + test. (3) broadphase
      `DynamicAabbTree.tight_aabbs` yazılıp-okunmayan dead-code SİLİNDİ (fat-margin no-rebuild
      mantığı korundu, broadphase differential proptest yeşil).
- [x] **Bakım turu 2. dalga (2026-07-06, 4 alan adversarial: gizmo-app/scene/net/renderer-resize):**
      (1) **Hiyerarşi cycle sağlamlığı (HIGH×2):** `save_prefab` BFS'i ve `hierarchy.rs
      despawn_recursive` visited-set'siz `Children` üzerinde dönüyordu → studio reparent (entity'yi
      kendi torununa sürükleme) veya scene-load'lu `Children` cycle'ında SONSUZ DÖNGÜ/OOM ve STACK
      OVERFLOW. İkisine de visited-set eklendi; save_prefab'ın aynı fix'i diamond hiyerarşide
      paylaşılan çocuğun İKİ KEZ serileştirilmesini (reload'da sızan boş entity) de önler. +3+1 test.
      (2) **Bare grup node öksüzleştirmesi (MED):** serialize skip-filter yalnız `Children` taşıyan
      yapısal grup node'unu düşürüyordu → alt-ağaç reload'da kopuyordu; non-empty Children taşıyan
      node artık korunuyor (tam-sahne + prefab). +test. (3) **Netcode ACK wraparound (HIGH):** demo
      server `input.tick > *entry` düz karşılaştırması tick `u32::MAX→0` sardığında ACK'i sonsuza dek
      donduruyordu (istemci `reconcile` signed-wraparound kullanırken) → istemci kuyruğu sınırsız
      büyür; ayrıca disconnect'te ACK girdisi silinmiyordu (client_id reuse'da stale ACK + sınırsız
      map). Paylaşılan `tick_is_newer` helper'ına çıkarıldı (kütüphane+server tek kaynak) + disconnect
      remove + wraparound testi. (4) **`Time::time_scale` ölü API (MED):** windowed loop fiziği ham
      dt ile besliyordu → dökümante `set_time_scale(0.0)` pause / `0.5` slow-mo simülasyonu HİÇ
      etkilemiyordu; `accumulate` artık scaled `time.dt()` alıyor (scale=1'de bit-aynı; update hook
      kamera/UI için ham dt kalır). (5) headless loop deferred `Commands` flush'u (0 sistemde
      uygulanmıyordu). **Renderer resize/kaynak-yaşam-döngüsü ADVERSARIAL TARANDI → 0 bug** (her
      ekran-boyutlu texture+bind-group resize'da yeniden oluşturuluyor; SSR/SSGI/fluid-SSFR temiz).
- [x] **Bakım turu 3. dalga (2026-07-06, 4 alan: gizmo-scripting/physics-soft/physics-dynamics/
      renderer-asset):** (1) **3. Children-cycle hang'i (HIGH):** `TransformPropagateSystem` (HER
      FRAME) BFS'i visited-set'siz → cycle'da queue sonsuz büyür=tüm uygulama hang (save_prefab'dan
      beter). Visited-set + test. **KÖK-NEDEN kapatıldı:** `HierarchyExt::is_ancestor` eklendi +
      `add_child` ve **studio reparent** artık kendi-torununa/kendine parent yapmayı REDDEDİYOR
      (cycle kaynakta önlenir + guard'lı traversal'lar zaten dayanıklı). +test. (2) **Karakter
      duvardan geçiyordu (HIGH):** KCC sweep `actual_d = d − radius >= 0` guard'ı tam temasta
      (FP'de ~−6e-8) hit'i düşürüp tam delta'yı uyguluyordu → duvarı deliyordu; `d >= 0` + `(d−radius)
      .max(0)` ile temas/penetrasyon bandı artık bloklar. +test (step_climb korundu). (3) **Soft-body
      node ≥2 collider'da tünelliyordu (HIGH):** `resolve_node_collision` menzildeki HER collider için
      position advance ediyordu (bitişik zemin/duvarda ikinci snap geçirtiyordu) → en-yakın-hit seçip
      tek snap. +test. (4) **Lua `on_update` hook'u hiç çalışmıyordu (MED):** script izole env'e yazar,
      `update()` globals'tan okuyordu → her yüklü script'in env'inden okunuyor. +test. (5) boş rope
      `len()−1` underflow guard (LOW). **RENDERER ASSET/MATERIAL/TEXTURE + VEHICLE ADVERSARIAL TARANDI**
      (alignment/format/mip/channel/index/fallback/vehicle-tire/suspension/engine hepsi temiz).
      **ATLANAN (dokümante):** scripting stale-id yeniden-kullanılan entity'yi mutasyona uğratır
      (bare u32 slot-id, generational-handle gerektirir=tasarım; repo 0 .lua); renderer async
      texture-streaming completion'ları install edilmiyor=`texture_streaming_system` no-op (özellik
      tamamlama + görsel A/B gerektirir, cache-key canonicalize↔normalize follow-on); degenerate
      sıfır-radius kapsül (geçersiz içerik).
- [x] **Bakım turu 4. dalga (2026-07-06, çok-ajanlı Workflow: 8 alan bul→adversarial-doğrula →
      14 onaylı bulgu, 10 düzeltildi/4 ertelendi; hepsi koda-karşı elle doğrulandı):** gizmo-ui,
      gizmo-audio, window+input, gizmo-editor, core-commands/hooks, gizmo-ai, renderer-gi, physics-
      core-shapes tarandı. **HIGH×5:** (1) **UI Node.position** taffy'nin PARENT-relative location'ını
      ABSOLUTE olarak yazıyordu → nested widget hit-test/render window köşesine kayıyordu; write-back
      top-down ata-offset accumulation'a çevrildi. (2) **Fare-bakış 2×** — masaüstünde HEM
      `CursorMoved→on_mouse_moved` (pos-diff delta) HEM `MouseMotion→on_mouse_delta` (raw) mouse_delta'yı
      besliyordu; `set_mouse_position` (pos-only) + cfg-gate (wasm hariç MouseMotion tek kaynak). (3)
      **Audio pitch panik** — `set_pitch(0/neg/NaN)` rodio SampleRateConverter `from>=1` assert'ini
      tetikleyip audio-thread'i öldürüyordu (scene-authored `AudioSource.pitch=0` erişilebilir);
      `sanitize_playback_speed` clamp + test. (4) **despawn reserved-entity panik** — `Commands::spawn`
      ile reserve edilmiş ama flush edilmemiş entity'de `is_alive` true ama `entity_locations` slot yok
      → 2 ham-index panik (236+304); bounds-safe (World::entity aynası) + test. (5) **Editor undo/redo**
      `get_entity(id)` bare slot okuyup kayıtlı generation'ı doğrulamıyordu → GC-recycled slot'ta yanlış
      entity; `is_alive(*entity)` (4 site). **MED×3:** input fast-tap (aynı-frame release+repress tuşu
      düşürüyordu; on_key/mouse_pressed pending-release'i iptal eder, +test), UI window_size (1280×720
      sabitti; `Res<WindowInfo>` gerçek boyut), GOAP heuristic (inadmissible count → suboptimal plan;
      Dijkstra h=0 optimallik + test). **LOW×2:** mouse-scroll wiring (MouseWheel handler yoktu),
      ProbeGrid empty-grid underflow guard. **ERTELENDİ (dokümante):** component_ops reserved-entity
      silent-drop (lazy-materialize Commands-flush ile double-flush riski + edge-case), UI z-order tek-
      kazanan (ZIndex tasarımı gerekir), hook re-entrant same-type (dar + restructure riski), coplanar
      convex-hull raycast phantom-AABB (degenerate collider, 2D-polygon testi gerekir). NOT: workflow
      verify-ajanı bir stray `reentrant_hook_probe.rs` bıraktı → SİLİNDİ.

- [x] **Bakım turu 5. dalga (2026-07-08, 4 az-denetlenmiş alan adversarial subagent + koda-karşı
      elle doğrulama):** gizmo-math, renderer culling/batching, gizmo-physics-dynamics, gizmo-animation
      tarandı; 4 CONFIRMED + 1 same-class düzeltildi, hepsi TDD. (1) **animation reverse-playback
      (HIGH):** isim-tabanlı `AnimationPlayer::advance` döngü zamanını `%=` ile sarıyordu (işareti
      korur) → ters oynatmada (`speed<0`) `elapsed_time` negatife düşüp sampler pozu sonsuza dek
      frame 0'a sabitliyordu; non-looping ters ise hiç durmuyordu (`playing` takılı). `rem_euclid` +
      `speed<0` non-looping-başta-dur; kardeş skeletal fix zaten doğruydu, bu yol atlanmıştı. (2)
      **render batch-key transparanlık/materyal-tipi çakışması (HIGH):** batch anahtarı materyalin
      *doku* bind-group'unu (paylaşımlı/cache'li — beyaz doku/aynı dosya) kullandığından yalnız
      transparanlık/materyal-tipinde ayrışan iki materyal tek batch'e düşüp routing bayraklarını ilk
      ECS-iterasyonundan alıyordu → şeffaf nesne opak (veya ters) render, PBR unlit yola gidiyor,
      hangisinin bozulduğu kareler-arası değişiyor (non-det). Oyun yolu `BatchKey`'e
      `is_transparent`/`unlit`/`is_skybox`; studio anahtarına `is_skybox`/`is_grid`/`is_unlit`
      (sonuncusu gölge-döküm kapısı → PBR nesne unlit-batch'te sessizce gölge dökmeyi bırakıyordu). (3)
      **anti-roll bar işareti TERS (MED):** `travel`=sıkışma, pozitif `suspension_force` şasiyi yukarı
      iter; sol köşe alçakken ARB'nin sola daha çok yukarı-kuvvet vermesi gerekirken kod tersini
      yapıyordu → `anti_roll_stiffness` artırmak yatmayı *artırıyordu* (pro-roll). `anti_roll_force`
      saf fonksiyona çıkarıldı (sol `+diff`, sağ `-diff`) + test. (4) **ray-AABB sınır-paralel sahte
      ıska (LOW ama platforma bağlı):** eksene paralel ışın kaynağı tam min yüzü üstündeyken
      `0*∞=NaN`, `Vec3A::min/max` (SIMD) yanlış operandı yayıyordu (max yüzü skaler indirgemeyle
      çalışıyordu → asimetrik); grid/tile/editor seçiminde erişilebilir. Deterministik skaler slab +
      simetrik test. **ERTELENDİ (dokümante):** animation `decompose_mat4` negatif-scale (ölü kod,
      `#[allow(dead_code)]`), renderer oyun-yolu instance-buffer büyütmüyor (8192 üstü sessiz drop;
      studio büyütüyor — bellek-güvenli sınırlama), vehicle tekerlek-spin damping fantom-fren
      (PLAUSIBLE, rolling-resistance kastı olabilir → kullanıcı kararı), vehicle COM kaldıraç-kolu
      (latent, `center_of_mass` default ZERO + motor-geneli konvansiyon belirsizliği), vehicle
      intra-step sıralı hız-mutasyonu (minor simetri kırılımı).

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
- [x] **Kinematik karakter denetleyici (KCC) + araç Ackermann denetimi** (2026-07-04,
      2 paralel subagent + elle doğrulama) — az test edilmiş `gizmo-physics-dynamics`
      (character.rs/vehicle.rs, 1386 LOC / yalnız 3 test) tarandı. **2 gerçek bug +
      ayırt edici regresyon testleri:** (1) **KCC basamak-tırmanma ileri-taşması**
      (`character.rs`): başarılı step'ten sonra `final_delta` tam uzunlukta kalıyordu →
      bir sonraki sweep iterasyonu (yükselen gövde artık duvarı temizlediğinden) tüm
      delta'yı YENİDEN uyguluyor, karakteri basamak başına ~2×'e kadar ileri fırlatıyordu;
      fix: step sonrası `final_delta`'yı kalan mesafeye (`move_dir·move_dist·(1−min_t)`)
      indir. Test `step_climb_does_not_overshoot_forward` (fix'siz taşma 0.045 > 0.025).
      (2) **Araç Ackermann iç/dış tekerlek TERS** (`vehicle.rs`): yarım-iz işareti tersti,
      iç tekerlek (dönüş merkezine yakın) DIŞ tekerlekten AZ dönüyordu (ters-Ackermann;
      +Y up/−Z forward/+X right konvansiyonu izlendi, sol=iç sol-dönüşte). Geometri saf
      `ackermann_steering_angle` yardımcısına çıkarıldı + işaret düzeltildi + test
      `ackermann_inner_wheel_steers_more_than_outer` (her iki dönüş yönü). **Bilinen sınır
      (kasıtlı DÜZELTİLMEDİ):** araç zemin-etkisi (`vehicle.rs` ~430) `height_above_ground`
      süspansiyon BAĞLANTI noktasından ölçüyor (şasi tabanı değil, dinlenmede ~0.85 m) →
      `ground_effect_height`(0.15)'in hep üstünde → ge_factor daima 1.0 = ölü özellik;
      doğru referans ayarlanmış downforce davranışını değiştireceğinden kod içi NOT'landı.

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
      bulgular **DÜZELTİLDİ** (`animation_system.rs`): (1) negatif speed (ters oynatma) artık
      `normalize_anim_time` (rem_euclid) ile sarıyor — birincil ve durum-makinesi yollarında;
      (2) crossfade'de fading-out klip artık player `speed`'iyle ilerliyor + tested `normalize_anim_time`
      kullanıyor (eski `prev_time += dt` 1× sabitliyor + `%=` negatif zamanı sarmıyordu) — pure
      `advance_and_sample_prev` yardımcısına çıkarıldı + 3 ayırt edici test. **(3, 2026-07-06)
      Animasyon durum-makinesi `find_transition` bug'ı DÜZELTİLDİ:** belirli-trigger sorgusu
      (`Some("jump")`) sıradaki ilk `trigger:None + has_exit_time` auto-geçişe takılıp klip bittiği
      karede yanlış duruma (idle) atlıyor + oyuncu girdisini yutuyordu → exit-time dalı artık
      `trigger.is_none()` ile kapılı (state_machine.rs, +4 test). **Asset/glTF/OBJ loader DENETLENDİ (subagent + elle):
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
      `core_shaders_compile` testinde naga ile doğrulanıyor. **COMPUTE/FLUID SHADER DENETİMİ
      TAMAM (2026-07-03, 4 paralel subagent + elle koda-karşı doğrulama):** 9 compute shader
      (fluid_compute/fluid_blur/spatial_hash, physics_compute/physics_culling/physics_debug,
      fem_compute, particle_compute, mesh_cull) + CPU-tarafı dispatch/bind-group/std430 layout'ları
      tarandı. Hepsi gpu_physics/gpu_fluid/gpu_particles olarak CANLI dispatch ediliyor. **Düzeltilen
      gerçek bug'lar (8, hepsi naga-valide; 4'ü ayırt edici regresyon testiyle kilitli):**
      **(B1, HIGH)** `box_contacts` storage buffer'ı eleman-başına 16 B eksik ayrılıyordu —
      WGSL std430'da `BoxContacts` stride'ı 352 B (yorum 336 hesaplamış; `count`→`_pad` 12 B ve
      `neighbors`→`normals` vec4-hizalama 4 B boşlukları atlanmış) → `max_boxes*336`'da yüksek-indeksli
      cisimler OOB indekslenip temas manifoldu alamıyor (içiçe geçme/tünelleme). Fix: `GpuBoxContacts`
      std430-sadık mirror struct + `size_of` ile boyutlandırma + size testi. **(D1, HIGH, canlı)**
      `spawn_explosion` parçacıkları `life == max_life` ile üretiyordu → compute/render shader
      `life >= max_life`'ı ölü saydığından her kırılma/çarpma toz patlaması GÖRÜNMEZ+hareketsiz;
      fix `life: 0.0` + GPU testi (bir adımda yaş ilerliyor). **(A1/A2, HIGH, LOD)** fluid
      `params.num_particles` çalışma zamanında hiç güncellenmiyordu → LOD<1.0'da hash-pass [active,N)
      parçacıklarını grid'e "gerçek" olarak sokuyor ama grid_offsets yalnız `active` yazıyor →
      aradakiler komşu taramasından sessizce düşüyor (yoğunluk/sıkışamazlık bozuluyor); fix
      `compute_pass` her kare `active`'i offset 28'e yazıyor + GPU dispatch testi. **(C4)** FEM
      ters-eleman (det F<0) işleme: `max(J,0.01)` J'yi pozitife zorluyordu → F^-T=cofactor/J'nin
      işaretini ters çeviriyordu; işaret-koruyan clamp + `ln|J|` + ters-eleman testi. **(A4)**
      spatial_hash negatif koordinat `i32()` sıfıra-truncate → `floor()` (sınır-altı parçacıklar
      hücre-0'a katlanmıyor). **(C1)** debug çizgi sayacı kapasiteyi aşınca indirect-draw vertex_count
      buffer'ı aşıyordu → atomicSub ile rezervasyon geri-alımı. **(C2/B2)** joint `body_b==u32::MAX`
      (dünya/statik sentinel) korumasız `boxes[MAX]` indeksliyordu (debug + solver; solver latent —
      joints hiç dispatch edilmiyor) → sentinel guard. **(C3)** FEM i32 fixed-point akümülatörü sert
      malzemede taşabiliyordu (`i32()` UB) → `enc()` cast-öncesi clamp (aralık-içi bit-aynı, savunmacı).
      **İkinci tur — A3+B3 de kapatıldı (2026-07-03):** (A3) fluid SSFR textureları (depth/
      thickness/blur/opaque-bg, 6 texture + 3 bind group) `create_ssfr_sized` yardımcısına çıkarıldı
      → `GpuFluidSystem::resize` + `Renderer::resize`'a bağlandı (eskiden bir kez kurulup yeniden
      oluşturulmuyordu → pencere büyüyünce fluid eski-boyut alt-dikdörtgene sıkışıyordu); resize
      testiyle kilitli. (B3) physics_culling `w<=0` (kamera-arkası) köşe → muhafazakâr "görünür say"
      guard'ı (clip-space düzlem testi yalnız w>0'da geçerli; görünür kutuyu yanlış cull etmeyi önler).
      **Kalan ertelenen:** (D2) parçacık ring-buffer'ı LOD-kırpılmış kuyruğa spawn edebilir (uzak
      kamerada birkaç parçacık görünmez) — LOD şemasında tasarım kararı (parçacık LOD'unu tümden
      kaldırmak vs. spawn'ı active-pencereye modlamak), temiz bir bug fix değil, ayrı iş.
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
- [x] **Prefab kaydet/yükle hiyerarşi sağlamlaştırma** (2026-07-04, subagent + elle) —
      `gizmo-scene` prefab yolunda 2 hiyerarşi-bozan bug + round-trip testi: (1) **çıplak
      grup kökü DÜŞÜYORDU** — yalnız `Children` taşıyan (isim/mesh/dinamik-bileşen yok)
      prefab kökü `serialize_entities` skip-filter'ine takılıp diske yazılmıyordu →
      `load_prefab` `root_id`'yi haritalayamıyor, kök hiç spawn edilmiyor, alt-ağaç
      koparak öksüz kalıyordu; fix: kökü `save_prefab`'te zorla dahil et. (2) **çözülemeyen
      ebeveyn ÖKSÜZ kalıyordu** — `instantiate_entities`'te `parent_id` kayıtlı sette
      yoksa `root_parent` fallback'i erişilemezdi (else dalı `if let Some(parent_id)`
      dışındaydı) → entity ne (kayıp) ebeveyne ne host'a bağlanıyordu; fix: çözülemeyince
      `root_parent`'a düş. Ayırt edici test `prefab_roundtrip_keeps_bare_group_root_and_children`.
- [x] **Studio gölge-geçişi caster filtresi** (2026-07-06) — studio (forward) gölge geçişi
      `flat_batches`'i FİLTRESİZ çiziyordu; game (deferred) yolu `unlit || is_transparent`
      atlar ve `classify_visibility` caster yordamı Unlit/Skybox/Grid/transparent'ı hariç tutar.
      Bu materyallerin KAMERA-görünür instance'ları `[start_instance, end_instance)`'te olduğundan
      gölge geçişi (`start..shadow_end` çiziyor) onları gölge haritalarına yazıyordu → her zaman
      var olan editör grid'i zemin-eş-düzlemli öz-gölge akne'si + eklenen skybox tüm sahneyi
      gölgeliyordu. Fix: shadow loop'ta `is_transparent||is_skybox||is_grid||is_unlit` atla (game
      yolunu birebir yansıtır); `is_unlit` bayrağı BatchData/FlatBatchData'ya eklendi.

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
- [x] **(c) `wgpu`/`winit`/`egui` güncel sürüme yükseltme** — *Stage B 1.0
      blokeri* — **YAPILDI.** Grafik yığını güncel sürümlere taşındı: **`wgpu`
      0.20→`23.0.1`**, **`winit` 0.29→`0.30.13`**, **`egui` 0.28→`0.34.3`**
      (+ `egui-wgpu`/`egui-winit`/`naga 23`). MSRV bunun için `1.89→1.92`'ye
      çıktı (egui 0.34 tabanı). Tüm grafik katmanı + `gizmo` facade + editör/studio
      derleniyor; workspace testleri + wasm grafik yığını + determinizm yeşil.
      (Bu madde eskiden "XL Stage B blokeri" olarak açıktı; taşıma tamamlanmış,
      yalnız kontrol listesi güncellenmemişti.)
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

## Faz 7 — Ürün Katmanı (şipilebilir oyun)

> Faz 0–6 çekirdeği (ECS + fizik + determinizm + test/CI) production seviyesine getirdi.
> Faz 7'nin teması **"doğru motoru → şipilebilir oyun motoruna" çevirmek**: yeni algoritma
> değil, mevcut derin parçaları görünür/kullanılabilir/ürün haline getirmek.
> Kaynak: 2026-07-08 8-alt-sistem kod-temelli değerlendirmesi (olgunluk: ECS 5, rigid-fizik 5,
> AI 5, render 4, editör 4, altyapı 4, gameplay 4, dinamik/soft-fizik 3, netcode 3).
> Sıra (etki/çaba + bağımlılık): **M0 → M1 ∥ M2 → M3 ∥ M5 → M4 → M6 → M7.**

### M7.0 — Hemen (ucuz, doğrulanmış açıklar) — çaba: düşük  ✅ TAMAM (2026-07-09)
- [x] SSGI apply UV yarı/tam-res uyumsuzluğu — çözüm zaten `7095ddf` (lighting/naga_oil turu)
      ile inmişti (vertex-emitted `[0,1]` UV); artık `ssgi_apply_uv_covers_full_frame`
      regresyon testiyle kilitli (eski `frag_coord/half_res` haritası ~2.0'a taşardı).
- [x] Transform undo/redo generation-safe — `history.rs` `TransformsChanged` kolu artık
      Transform borrow'undan ÖNCE `world.is_alive` ile filtreliyor (recycled slot atlanır;
      `EntityDespawned` kolunun aynası). 3 ayırt edici regresyon testi (eski bare-slot kodunda FAIL).
- [x] Kirli çalışma ağacı toplandı/commit'lendi (Faz 7 entegrasyon dalı); pre-existing
      `-D warnings` clippy blocker'ları (gizmo-analysis egui-deprecated + pipeline needless-borrow) temizlendi.

### M7.1 — Materyal & görsel sıçrama — etki: ÇOK YÜKSEK · çaba: orta-yüksek  🟢 KISMEN (2026-07-09)
En büyük görünür kazanç: gerçek dokulu PBR (şu an yalnız base-color + skaler PBR).
- [x] G-buffer geometry shader'ı çoklu doku örnekler: **normal map** (Gram-Schmidt TBN × normalScale),
      **metallic-roughness** (skaler × MR.gb), **emissive**, **AO**. NOT: 4 MRT dolu olduğundan emissive
      albedo'ya ADDITIVE, AO albedo'ya çarpım (LDR yaklaşımı — gerçek HDR unlit-glow emissive 5. MRT
      veya lighting-pass girişi ister = gizmo-studio dokunuşu, ertelendi).
- [x] Materyal-başına 7-binding doku bind-group'u (base/sampler/normal/MR/emissive/AO/params-uniform);
      `MaterialParams` GPU struct (32B); `MaterialDefaults` paylaşımlı 1×1 fallback (flat-normal + white-linear).
- [x] GLTF loader normal/MR/emissive/AO map'lerini + faktörleri bağlıyor (per-image sRGB↔linear sınıflandırma).
- [ ] (opsiyonel) spot-light gölgesi + ışık limiti — YAPILMADI.
- [~] Deliverable `material_demo`: shader/layout/loader hazır + naga+pipeline testli; GÖRSEL A/B insan
      gerektirir (normal+MR bir glTF'te gözle doğrulanmalı). KHR_materials_emissive_strength ve per-texture
      sampler ayarları henüz uygulanmadı.

### M7.2 — Gameplay sistemlerini bağla + car_demo çöz — etki: YÜKSEK · çaba: orta  🟢 KISMEN (2026-07-09)
Mevcut derin fiziği (Pacejka lastik, KCC) kullanılabilir kılar.
- [x] `vehicle_controller_system` + `character_controller_system` → `Phase::Physics`'te
      `physics_step_system` ÖNCESİNE kayıtlı ECS sistemleri (`gizmo-physics-dynamics/systems.rs` +
      `gizmo-app/gameplay.rs::GameplayPhysicsPlugin`). Query deseni `physics_step_system` ile birebir
      (exclusive-barrier `fn(&World,f32)` + `query_unchecked` + `iter_mut`); component'i olmayan
      entity'de no-op → determinizm oracle DEĞİŞMEDİ (57FA0A2E8313B7A2).
- [~] car_demo: main'in GELİŞMİŞ `VehicleController` kurulumu (Pacejka + Ackermann + anti-roll +
      tork eğrisi, COM/collider süspansiyon-ışını self-hit'e karşı ayarlı) KORUNDU. Sürüş/geometri
      son doğrulaması hâlâ EKRANDA GÖZLE yapılmalı (gated). *(Not: eski arcade-Vehicle geometri denemesi
      main'in daha gelişmiş sürümü lehine ELENDİ.)*
- [x] Ragdoll runtime: `spawn_ragdoll` / `RagdollBuilder::spawn` iskelet tanımından body+capsule+joint
      spawn edip `PhysicsWorld`'e bağlıyor (humanoid 11 body/10 joint; yerçekiminde NaN'sız düşer, testli).
- [ ] ABA multibody + GPU FEM kararı — YAPILMADI (deneysel-işaretleme/senkron ayrı iş).
- [~] Deliverable: yürüyen karakter + düşen ragdoll sistemleri + testleri hazır; sürülebilir araç
      demosu ekranda doğrulama bekliyor.

### M7.3 — Animasyon olgunlaşması — etki: ORTA-YÜKSEK · çaba: orta  🟢 KISMEN (2026-07-09)
- [x] Two-bone IK (analitik law-of-cosines, `solve_two_bone_ik` + `TwoBoneIkChain` component) +
      FABRIK (N-kemik iteratif). `register` artık `TwoBoneIkChain`'i de kaydediyor.
- [x] Scale track'leri korunuyor (`gizmo-animation` sampler'ı Scale'i birinci-sınıf kanal olarak
      örnekleyip `Transform::scale`'e yazıyor) + Step/Linear/CubicSpline modları + gerçek glTF in/out
      tangent'li cubic Hermite (bounds-checked, malformed'da linear'e düşer). Ayırt edici testler.
- [ ] İki `AnimationPlayer` tipini birleştir; skeletal sampling'i renderer'dan animation crate'ine
      taşı — ERTELENDİ. ⚠️ Bu yüzden `gizmo-animation`'daki scale/cubic iyileştirmeleri, renderer'ın
      KENDİ skeletal yolu (`gizmo-renderer/src/animation_system.rs:298` scale'i atar, `animation.rs:54`
      cubic'i linear'e düşürür) hâlâ eskisi olduğundan RENDER EDİLEN iskelete henüz ulaşmıyor
      (crate-arası birleştirme gerekir).

### M7.4 — Netcode ürünleştirme — etki: YÜKSEK · çaba: yüksek
Rollback güçlü; client-server "ürün" değil.
- [ ] Client-server uçtan uca: gerçek client binary; sunucu authoritative gameplay (entity spawn/replikasyon); `ClientPredictor` + `SnapshotInterpolator`'ı client loop'una bağla.
- [ ] İki rollback implementasyonunu birleştir (RollbackSession kanonik; ECS RollbackManager kaldır/köprüle).
- [ ] Cross-platform determinizm (Fp32/Q16.16 sim yolu, opsiyonel feature) → makineler-arası P2P. *Ayrı büyük alt-faz.*
- [ ] Basit lobby/connect-token + N-oyunculu rollback (input-delay/time-sync).

### M7.5 — Audio & UI cilası — etki: ORTA · çaba: orta-yüksek
- [ ] Audio: mixer/bus/submix, temel DSP (low-pass/reverb/occlusion), doppler, `AudioSource` ECS sistemi.
- [ ] UI: font/metin (glyph atlas — şu an metin yok), temel widget seti, z-index + kırpma/kaydırma, renderer entegrasyonu.

### M7.6 — Platform & araç — etki: ORTA · çaba: yüksek
- [ ] WASM özellik paritesi: tarayıcıda deferred/gölge/compute yolu (bind-group limiti).
- [ ] Editör: `gizmo-analysis` canlı panelini editöre bağla; `ComponentChanged` undo; script highlighting; WGSL hot-reload; TR/EN i18n.
- [ ] Birinci-sınıf AssetServer + hot-reload + scene-load'da GPU kaynak restore.

### M7.7 — 1.0 sürüm hijyeni — etki: ORTA · çaba: orta
- [ ] CI kapıları: rustfmt zorunlu, `missing_docs`+`cargo doc`, coverage (tarpaulin), cargo-deny/audit, physics/renderer benchmark'ları + regresyon takibi.
- [ ] Staged 1.0 (Stage A çekirdek→1.x, Stage B grafik→0.y) + publish pipeline.

### Denetim eki (2026-07-09) — kod-doğrulanmış atomik kalemler
> Kaynak: 5-ajanlık ROADMAP↔kod denetimi (M7.1–M7.3 "kısmi" maddelerinin GERÇEK kalan
> boşlukları + bug-avı turunun latent bulguları). Hepsi dosya:satır ile sabitlendi. Bunlar
> yukarıdaki M7.1/M7.2/M7.3 kalemlerinin somutlaştırılmış alt-görevleridir, yeni faz değil.

**Hızlı & kendi kendine yeten (düşük çaba, testle kapanır):**
- [x] **Audio tek-atış 3B sonsuz tekrar** — ✅ 2026-07-09: `AudioSource`'a `#[serde(skip)] has_played`
      mandalı eklendi; `audio.rs` guard'ı saf `should_autostart()` predicate'ine çıkarıldı
      (`is_3d && !has_played && sink.is_none()`), başlatma denemesinde (başarı VE hata) mandal
      kalkar. `audio_spatial_system` "opt-in" olarak dokümante edildi (DefaultPlugins'e girmez;
      AudioManager+çıkış cihazı ister). +2 cihaz-bağımsız regresyon testi.
- [x] **gizmo-analysis metrik kind-collision** — ✅ 2026-07-09: `entry()` artık `Option<&mut>` döndürüyor;
      ilk kayıtlı kind KAZANIR, uyumsuz kind yazımı DÜŞÜRÜLÜR (seriyi bozmaz) + `trace` feature'ında
      isim-başına-bir-kez `tracing::warn!`. Steady-state alloc-free hızlı yol korundu. +2 test.
- [x] **Scripting negatif/NaN collider boyutu** — ✅ 2026-07-09: `sanitize_dim()` (sonlu-değil/≤0 →
      `MIN_COLLIDER_DIM=1e-4`) box/sphere collider boyutlarına uygulandı. +3 test (Lua'dan negatif/NaN).
- [x] **car_demo bayat yorum** — ✅ 2026-07-09: yorum gerçeğe güncellendi (`vehicle_controller_system`
      M7.2'de kayıtlı → ölü-kod DEĞİL). Demo'yu motor sistemine bağlama sürüş-EKRAN-doğrulamasına
      bağlı olduğundan ayrı iş olarak bırakıldı.

**M7.1 tamamlama (dokulu PBR — görünür kazanç):**
- [ ] **Dokulu glTF `material_demo` sahnesi/asset'i ekle** — şu an hiçbir demo dokulu glTF
      yüklemiyor (`demo/src/bin/bevy_material_demo.rs` yalnız base-color küpleri renklendiriyor);
      normal/MR/emissive/AO GÖRSEL A/B'si yapılabilecek sahne YOK (deliverable asset eksik).
- [ ] Spot-light gölgesi + ışık limiti — `deferred_lighting.wgsl` yalnız spot koni-atenüasyonu
      (`:559-560`) yapıyor, gölge örneklemesi yok.
- [ ] Gerçek HDR unlit-glow emissive — `gbuffer.wgsl:187-193` şu an additive LDR yaklaşımı
      (4 MRT dolu); 5. MRT veya lighting-pass girişi ister.
- [ ] `KHR_materials_emissive_strength` (`loaders.rs:686-687` not: uygulanmıyor) + per-texture
      sampler (tek paylaşımlı `gltf_material_sampler`; glTF wrap/filter ayarları yok sayılıyor).

**M7.3 tamamlama (EN YÜKSEK ETKİ — iyileştirmeler render'a ulaşsın):**
- [x] **Render skeletal sampler'ında scale-track + cubic-Hermite** — ✅ 2026-07-09: iki gerçek boşluk
      RENDER yolunda (`skeletal::sample::evaluate_clip`, renderer `animation_system.rs`'in çağırdığı)
      kapatıldı: (a) scale izleri artık `changes[joint].2`'ye UYGULANIYOR (squash/stretch/nefes
      render iskelete ulaşır); (b) gerçek cubic-Hermite (glTF Ek C) — `Keyframe`'e opsiyonel
      `in_tangent`/`out_tangent` eklendi, loader tangentleri artık SAKLIYOR (eskiden atıyordu),
      `Track::sample_cubic` + Vec3/Quat Hermite kombinatörleri (tangent yoksa lerp'e düşer).
      +7 test. NOT: İki AnimationPlayer (clip.rs/system.rs zaten scale'i işliyordu) tam birleştirme
      HÂLÂ ayrı iş; ama render yolu artık scale+cubic'i doğru örnekliyor. GÖRSEL A/B insan-gated.

**M7.2 kalan karar:**
- [ ] **ABA multibody + GPU-FEM kararı** — KOD MEVCUT (`crates/gizmo-physics-rigid/src/multibody/aba.rs`
      471 LOC Featherstone + property test; `gpu_physics` feature FEM yolu) ama ana pipeline'a bağlı
      değil (yalnız kendi testleri çağırıyor) → "motora bağla ya da deneysel işaretle" kararı; kod yazımı değil.
- [ ] car_demo sürüş/geometri EKRAN doğrulaması (gated — insan gözü gerekir).

---

## Çalışma Yöntemi
- Her madde: **düzelt → regresyon testi yaz → derle/test/clippy → işaretle.**
- Davranış değiştiren fizik düzeltmelerini `headless_stress_test` + odaklı senaryolarla doğrula.
- Bug-avı turlarında subagent fan-out kullan, sonra her bulguyu elle doğrula (false-positive'leri ele).
</content>
