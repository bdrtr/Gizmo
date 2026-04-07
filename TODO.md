# 🛠️ Gizmo Engine — Kapsamlı Hata & İyileştirme Listesi

> Bu liste motorun tüm alt sistemlerinin satır satır incelenmesiyle oluşturulmuştur.
> **Tarih:** 2026-04-07

---

## 📦 FİZİK MOTORU (gizmo-physics)

### Çarpışma Algılama (collision.rs, system.rs)

- [x] **Capsule-AABB sadece 5 nokta örnekliyor** — `collision.rs:231-247`
  Kapsül-AABB en yakın nokta hesabı, kapsülün merkez segmentinden sadece 5 noktayı test ediyor. Çapraz açıdan gelen kapsüller için gerçek en yakın noktayı kaçırabilir → yanlış normal → NPC duvara kayar.
  **Çözüm:** Analitik segment-AABB en yakın nokta hesaplaması (segment'i AABB'nin 6 düzlemine kırp).

- [x] **Capsule-AABB rotasyonlu AABB desteği yok** — `collision.rs:225-226`
  `check_capsule_aabb_manifold` fonksiyonunda AABB min/max hesaplanırken kutunun rotasyonu göz ardı ediliyor: `let min_b = pos_aabb - aabb.half_extents`. Rotasyonlu kutuya çarpan kapsül yanlış sonuç alır.
  **Çözüm:** AABB rotasyonlu ise GJK/EPA'ya yönlendir veya OBB-Capsule analitik çözüm yaz.

- [x] **Sphere-AABB inline fast-path, collision.rs'deki `check_sphere_aabb_manifold` ile kod tekrarı yapıyor** — `system.rs:249-280`
  Aynı Sphere-AABB hesaplaması hem `system.rs`'deki inline kodda hem de `collision.rs:92-121`'deki fonksiyonda var. Inline kopya rotasyonu kontrol ediyor, fonksiyon etmiyor. Tutarsızlık riski.
  **Çözüm:** Fast-path'i `check_sphere_aabb_manifold` fonksiyonuna yönlendir, tekrarlı kodu sil.

- [x] **Broad-phase AABB genişletmesi Capsule için aşırı büyük** — `system.rs:76-80`
  Capsule'ün broad-phase AABB'si `radius + half_height` boyutunda her üç eksende aynı şekilde genişletiliyor. Bu kutunun 3x aşırı büyük olmasına neden olur → gereksiz narrow-phase testleri.
  **Çözüm:** Kapsülün rotasyonuna göre daha sıkı AABB hesapla.

### SI Çözücü (system.rs)

- [x] **Warm-starting devre dışı bırakılmış** — `system.rs:503-510`
  Temas noktası tutarsızlığı nedeniyle warm-starting iptal edilmişti. Contact Point Matching (2cm threshold, %80 sönümleme) ile güvenle geri açıldı.
  **Çözüm:** Temas noktası eşleme (contact point matching) implementasyonu ile warm-starting geri açıldı ✅

- [x] **Çözücü iterasyonu sabit 8** — `system.rs:525`
  `for _iter in 0..8` — Basit sahneler için yeterli, ama yığılma veya zincirleme durumlarda yetersiz kalabilir. Dinamik iterasyon sayısı öğrenme fırsatı.
  **Çözüm:** Konfigüre edilebilir `solver_iterations` parametresi ekle veya artık iyileşme (residual) bazlı erken çıkış.

- [x] **Pseudo-random contact shuffle deterministik değil** — `system.rs:520-523`
  `island.contacts.swap(i, swap_idx)` ile `(i * 37 + 11) % len` kullanılıyor. Deterministik ama ardışık frame'lerde aynı sıralama → bias kalıcı olabilir.
  **Çözüm:** Frame sayısını seed olarak kullan: `(i * 37 + 11 + frame_count) % len`.

### Entegrasyon (integration.rs)

- [ ] **Pozisyon entegrasyonu Semi-Implicit Euler, ama sıralama YANLIŞ** — `integration.rs:106-125`
  Semi-Implicit Euler'de doğru sıralama: (1) hız güncelle, (2) pozisyonu yeni hızla güncelle. Ama koddaki akış: BATCH 2'de hız güncellenir, BATCH 3'te `let v = *vel_storage.get(e)` ile hız okunup pozisyon güncellenir. Bu doğru görünüyor AMA hız clamp ve damping BATCH 2'de uygulanıyor, solver impulse'ları ayrıca `physics_collision_system`'de uygulanıyor ve bu iki sistem AYRI çağrılıyor. Solver → Integration sırası main.rs'de doğru mu kontrol edilmeli.
  **Not:** `main.rs:107-117`'de sıra: `collision_system → character_system → constraints → vehicle → ai → movement`. Bu doğru sıralama.

- [x] **SIMD Batch artık döngüsünde `index += 8` sabit** — `integration.rs:102`
  Son batch'te geçersiz lane'ler (valid_count < 8) sıfır ile dolduruluyor. Bu doğru çalışıyor ama `grav[i]` sıfır olduğunda bile yerçekimi uygulanıyor (0.0 çarpılıyor, sorun yok). Kod doğru ama yorum eklenebilir.

### Bileşenler (components.rs)

- [x] **RigidBody varsayılan eylemsizlik 1x1x1 küp** — `components.rs:97`
  `RigidBody::new()` her zaman 1x1x1 bir küp eylemsizliği hesaplıyor. `calculate_box_inertia` / `calculate_sphere_inertia` / `calculate_capsule_inertia` fonksiyonları var ama sahne kurulumunda **çağrılmıyor**! Her obje aynı eylemsizliğe sahip.
  **Çözüm:** `scene_setup.rs`'de Collider oluşturduktan sonra boyutlara göre uygun inertia fonksiyonunu çağır.

- [ ] **Transform.scale fizik tarafından yok sayılıyor** — Genel tasarım
  Collider boyutları asla `Transform.scale` ile çarpılmıyor. Bu bilinçli bir tasarım kararı olabilir ama belgelenmemiş. Kullanıcı bir küpü `scale(2,2,2)` yaparsa collider aynı boyutta kalır.
  **Çözüm:** Ya otomatik Scale-aware collider yap, ya da bu kararı docs/README'ye belirgin şekilde yaz.

### Kapsül (shape.rs)

- [x] **Swept shape `unreachable!()` ile patlar** — `shape.rs:91`, `character.rs:142`
  `ColliderShape::Swept` ECS'de asla olmamalı ama `system.rs` broad-phase'de bu dalı `unreachable!()` ile yakalıyor. Eğer bir bug sweep'i ECS'ye yazarsa program panikler.
  **Çözüm:** `unreachable!()` yerine `continue` veya loglama koy (savunmacı programlama).

### Constraint Çözücü (constraints.rs)

- [x] **Joint solver her iterasyonda `borrow_mut::<Velocity>` çağırıyor** — `constraints.rs:186, 247, 264, 282, 319, 333, 358, 366`
  Her joint tipi ve her iterasyonda (`15 × joints.len()` kez) velocity storage yeniden borrow ediliyor. Bu RefCell overhead'i çok yüksek.
  **Çözüm:** Velocity'yi iterasyon döngüsünün DIŞINDA bir kez borrow et, local HashMap'e kopyala, sonunda geri yaz (collision solver'ın yaptığı gibi).

- [ ] **Joint solver Transform'u her iterasyonda sadece READ ediyor ama pozisyon düzeltmesi yapmıyor** — `constraints.rs:150-162`
  Pozisyon kısıtı sadece velocity bias ile çözülüyor. Fixed ve BallSocket joint'ler, nesneler belirli bir mesafede kalması gerektiğinde drifting yaşayabilir.
  **Çözüm:** Solver sonrası doğrudan Transform pozisyon düzeltmesi (collision solver'a benzer).

- [x] **Spring joint'te `borrow::<Velocity>` ve `borrow_mut::<Velocity>` aynı scope'da** — `constraints.rs:311-325`
  Satır 311'de `world.borrow::<Velocity>()` immutable olarak okunuyor, satır 319'da `world.borrow_mut::<Velocity>()` mutable olarak alınıyor. RefCell'de aynı scope'da ikisi birden olmamalı, ama scope farklı (`{}` bloğu ile ayrılmış). Çalışıyor ama fragile.
  **Çözüm:** Tek bir mutable borrow ile her şeyi yap.

---

## 🧠 AI SİSTEMİ (gizmo-ai)

- [x] **`path.remove(0)` O(n) karmaşıklığı** — `system.rs:83`
  Vec'in başından silmek tüm elemanları kaydırır. Uzun yollarda her frame O(n) maliyet.
  **Çözüm:** `VecDeque` kullan veya indeks takibi yap (`current_path_index` field).

- [ ] **AI navigasyon sistemi `borrow::<Transform>` ve `borrow_mut::<Velocity>` aynı anda tutuyor** — `system.rs:18-26`
  Bu RefCell kuralları açısından sorunsuz (farklı tipler) ama `borrow_mut::<NavAgent>` da tutulduğunda toplam 3 aktif borrow var. Gelecekte başka bir sistem de velocity borroow ederse çakışma olur.
  **Çözüm:** Dokümante et veya borrow scope'unu daralt.

- [x] **A* `find_path` 2000 iterasyon limitli ama NAV_GRID hücre boyutuna bağlı** — `pathfinding.rs`
  Eğer grid hücre boyutu çok küçükse (ör. 0.1) aynı mesafe için 100x daha fazla node var ve 2000 iterasyon yetersiz kalır.
  **Çözüm:** Limiti grid alanına orantılı yap: `max_iter = (area / cell_size²).min(5000)`.

- [x] **NavAgent `max_speed: 8.0` fizik çözücüyle uyumsuz olabilir** — `scene_setup.rs:306`
  AI hız 8 m/s ama fizik çöz gücüsü `max_force: 50.0`. Eğer AI hızla duvara koşarsa penetrasyon derinliği `8.0 * dt ≈ 0.13 m/frame` olabilir ki bu pozisyon düzeltmesinin slop'undan çok büyük. Pozisyon düzeltmesi eklendi ama hâlâ yüksek hızlarda sorun olabilir.
  **Çözüm:** AI max_speed'i fizik solver kapasitesiyle test et / limitini düşür.

---

## 🎨 RENDERER (gizmo-renderer)

- [x] **Renderer struct'ında 40+ public field var** — `renderer.rs:7-50`
  Struct çok şişmişti. Sub-struct'lara ayrıldı: `SceneState` (pipeline, shadow, skeleton), `PostProcessState` (HDR, bloom, blur, composite).
  **Çözüm:** `renderer.scene.*` ve `renderer.post.*` olarak yeniden yapılandırıldı ✅

- [x] **`PresentMode::Fifo` sabit kodlanmış — VSync her zaman açık** — `renderer.rs:92`
  FPS sınırı VSync ile kilitli. Performans testi veya "uncapped FPS" seçeneği yok.
  **Çözüm:** Konfigüre edilebilir PresentMode (Mailbox / Immediate).

- [x] **`gpu_particles` 100,000 parçacık sabit** — `renderer.rs:104`
  GPU particle buffer boyutu hardcoded. Küçük sahneler için bellek israfı, büyük efektler için yetersiz olabilir.
  **Çözüm:** Dinamik veya konfigüre edilebilir buf boyutu.

- [x] **GLTF loader'da bilinmeyen format fallback'i sessiz** — `asset.rs:482-490`
  GLTF'den gelen bilinmeyen piksel formatları sessizce beyaz piksele dönüştürülüyor. Debugging güçleşir.
  **Çözüm:** `eprintln!` veya log crate ile uyarı bas.

- [x] **GLTF material `unlit = 1.0` hardcoded** — `asset.rs:551`
  Tüm GLTF materyalleri unlit (PBR kapalı) olarak yükleniyor. PBR destekli modellerde ışıklandırma çalışmaz.
  **Çözüm:** GLTF material metadata'sından unlit/lit bilgisini oku.

- [x] **Texture cache anahtar olarak dosya yolunu kullanıyor** — `asset.rs:11, 318`
  Relative path (`demo/assets/stone.jpg`) ve absolute path (`/home/.../stone.jpg`) farklı cache entry oluşturur.
  **Çözüm:** `canonicalize()` ile normalize et.

---

## 🏗️ ECS CORE (gizmo-core)

- [x] **`World::despawn` tüm storage'ları iterasyonla tarıyordu** — `world.rs:60-62`
  Entity başına TypeId takibi (`entity_components`) eklendi. Artık sadece ilgili storage'lara dokunulur — O(S) → O(C).
  **Çözüm:** `entity_components: HashMap<u32, Vec<TypeId>>` ile hedefe yönelik silme ✅

- [x] **`iter_alive_entities` her çağrıda Vec allocate ediyordu** — `world.rs:74-83`
  Sıfır allocation `AliveEntityIter` iterator struct'ı eklendi. Vec döndüren `alive_entities()` kolaylık metodu korundu.
  **Çözüm:** Iterator pattern — `AliveEntityIter` struct ✅

- [x] **RefCell runtime borrow panikleri korumasız** — `world.rs:112-113, 122-123`
  `storage.borrow()` ve `storage.borrow_mut()` RefCell panikleri (BorrowMutError) korumasız. Aynı component'i aynı anda okuma+yazma denerseniz program panikler.
  **Çözüm:** `try_borrow` / `try_borrow_mut` ile hata mesajı döndür.

---

## 🎮 DEMO / OYUN KATMANI (demo)

### Ana Döngü (main.rs)

- [x] **`physics_collision_system` sabit `1.0/60.0` dt ile çağrılıyor ama `fixed_dt` farklı olabilir** — `main.rs:107`
  `gizmo::physics::system::physics_collision_system(world, 1.0 / 60.0)` — burada `fixed_dt` yerine sabit `1/60` kullanılmış. `target_physics_fps` 60 değilse tutarsızlık olur.
  **Çözüm:** `1.0 / 60.0` yerine `fixed_dt` kullan.

- [x] **Fizik adımı üst limiti 16, ama frame spike'larında simülasyon kaybı** — `main.rs:106`
  `while ... && steps < 16` — Eğer bir frame çok uzun sürerse (ör. alt+tab) max 16 fizik adımı atılır, gerisi kaybolur. Bu "death spiral"ı önler ama uzun süren duraklamalarda simülasyon atlama yapar.
  **Çözüm:** Accumulator'ı `fixed_dt * max_steps` ile sınırla.

- [x] **`Time::elapsed_seconds` her zaman 0.0** — `main.rs:100`
  `Time { dt, elapsed_seconds: 0.0 }` — toplam geçen süre asla güncellenmemiş.
  **Çözüm:** Bir `app_start` timestamp tutup `elapsed_seconds = now - app_start` hesapla.

- [ ] **Lua `run_scripts` ve `engine.update` çift çağrı** — `main.rs:163-176`
  Hem `engine.update(world, input, dt)` hem de `run_scripts(world, state, dt, input)` çağrılıyor. Bazı script fonksiyonları iki kez çalışabilir.
  **Çözüm:** Sorumlulukları netleştir veya birleştir.

### Sahne Kurulumu (scene_setup.rs)

- [x] **NPC Collider kapsül ama visual küp** — `scene_setup.rs:297-301`
  NPC'nin görseli `create_cube()` ile küp ama fizik collider'ı `Collider::new_capsule(0.5, 0.5)`. Görsel ve fiziksel sınır eşleşmiyor.
  **Çözüm:** Ya görseli kapsül/silindir yap, ya da collider'ı AABB yap.

- [x] **Duvar inertia hesaplanmamış** — `scene_setup.rs:282`
  Duvarlar `RigidBody::new_static()` ile oluşturuluyor (mass=0, inverse_inertia=ZERO). Statik objeler için sorun değil AMA NPC (`mass=1.0`) için `calculate_capsule_inertia` çağrılmamış.
  **Çözüm:** NPC RigidBody oluşturduktan sonra `rb.calculate_capsule_inertia(0.5, 0.5)` çağır.

- [x] **NavGrid engel padding 3x3 ama collider 1.0x2.0x1.0** — `scene_setup.rs:287-291`
  Padding 1 hücre her yöne ama collider yarı genişliği 1.0 metre. Hücre boyutu 1.0m olduğunda padding tam oturuyor, ama collider boyutu veya hücre boyutu değişirse mismatch olur.
  **Çözüm:** Padding'i collider boyutuna göre otomatik hesapla.

### Transform Hiyerarşi (systems.rs)

- [x] **Hiyerarşi BFS her frame'de `world.borrow_mut::<Transform>()` çağırıyor** — `systems.rs:34-39`
  BFS döngüsünde her node için `world.borrow_mut::<Transform>()` ve `world.borrow::<Children>()` çağrılıyor. Bu her seferinde RefCell lock/unlock yapar.
  **Çözüm:** Tüm transform'ları ve children'ı döngü dışında bir kez borrow et.

- [x] **Particle system `world.iter_alive_entities()` kullanılıyor** — `systems.rs:69`
  Bu fonksiyon TÜM entity'leri döndürür (binlerce). Sadece `ParticleEmitter` bileşenine sahip entity'ler üzerinde dönmek yeterli.
  **Çözüm:** `emitters.entity_dense` üzerinde doğrudan iterate et.

### Karakter Kontrolcüsü (character.rs)

- [x] **Zemin Y değeri hardcoded: `-1.0`** — `character.rs:234`
  `let ground_y = -1.0_f32;` — Bu sabit zemin yüksekliği. Multi-level haritalar veya rampalar için çalışmaz.
  **Çözüm:** Aşağı doğru raycast ile gerçek zemin yüksekliğini bul.

- [ ] **Slide vektörü hesaplaması `correction * -1` olmalı** — `character.rs:228`
  `let normal_component = normal * remaining.dot(normal)` — Burada `normal`, correction'dan gelen (collider'dan dışarı yönlü) bir vektör. Remaining vektörünün bu normalle projection'ı slide yönünü belirliyor. İşaret karışıklığı olabilir.
  **Çözüm:** Bir birim test ile slide mantığını doğrula.

### Araç Fiziği (vehicle.rs)

- [x] **Zemin Y değeri hardcoded: `-1.0`** — `vehicle.rs:140`
  Araç raycast sistemi de sabit zemin yüksekliği kullanıyor.
  **Çözüm:** Gerçek collider raycast sonuçlarını kullan.

- [x] **Motor gücü sadece arka 2 tekerleğe** — `vehicle.rs:199`
  `if i >= 2` hardcoded. 4WD/FWD/RWD konfigürasyonu yok.
  **Çözüm:** `Wheel` struct'ına `is_drive_wheel: bool` field ekle.

- [x] **Direksiyon ve sürtünme sabitleri hardcoded** — `vehicle.rs:217-229`
  `8000.0`, `5000.0`, `3000.0` gibi fizik sabitleri doğrudan kodda. Farklı araçlar için ayarlanamaz.
  **Çözüm:** VehicleController struct'ına bu sabitleri taşı.

---

## 🔊 SES SİSTEMİ (gizmo-audio)

- [x] **`audio_update_system` fonksiyonu kullanılmıyor** — `systems.rs:133` (compiler warning)
  Fonksiyon tanımlı ama hiçbir yerde çağrılmıyor.
  **Çözüm:** Ana döngüye ekle veya dead code olarak sil.

---

## 📜 LUA SCRIPTING (gizmo-scripting)

- [x] **Script fonksiyon adı dosya adına göre hardcoded** — `main.rs:256-262`
  `car_controller` içeriyorsa `"car_update"`, `rain` içeriyorsa `"rain_update"`, aksi halde `"on_update"`. Yeni script dosyaları için her seferinde bu if-chain'e ekleme yapılması lazım.
  **Çözüm:** Script bileşenine `entry_function: String` field ekle veya convention-over-configuration uygula.

---

## 🧹 GENEL KOD KALİTESİ

- [x] **Compiler warning'leri** — Birden fazla dosya
  `unused_mut`, `unused_variables`, `dead_code` uyarıları var. `cargo fix` ile tek seferde temizlenebilir.

- [ ] **Testler yetersiz** — Fizik motoru
  Sadece `gjk.rs`, `epa.rs` ve `collision.rs`'te temel birim testler var. Integration, solver, vehicle, constraint testleri yok.
  **Çözüm:** Her modüle "penetrasyon kalıcı mı?", "yığılma stabil mi?" gibi regresyon testleri ekle.

- [x] **TODO/FIXME/HACK yorum taraması** — Tüm kod tabanı
  Kodda birçok yerde Türkçe yorum ile "geçici çözüm" veya "ileride yapılacak" notları var. Bunları merkezi bir izleme listesine taşımak gerekir.
  **Çözüm:** `grep -rn "TODO\|FIXME\|HACK\|İPTAL\|geçici\|hatrlat"` ile tarayıp bu dosyaya ekle.

---

## 📊 İSTATİSTİKLER

| Kategori | Toplam Madde |
|----------|-------------|
| Fizik Motoru | 14 |
| AI Sistemi | 4 |
| Renderer | 6 |
| ECS Core | 3 |
| Demo/Oyun | 11 |
| Ses | 1 |
| Scripting | 1 |
| Genel | 3 |
| **TOPLAM** | **43** |
