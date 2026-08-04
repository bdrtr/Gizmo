# FIXPLAN — Denetim sonrası düzeltme kampanyası

> **Geçici belge.** `docs/AUDIT-2026-08.md` denetiminden çıkan işleri yürütmek için.
> Kampanya bitince kalan kalıcı kararlar `ENGINE.md`'ye taşınır ve **bu dosya silinir**
> (2026-07 sadeleştirmesinde 12 plan dosyasının birleştirilmesiyle aynı kural).
>
> **Çalışma yöntemi (ENGINE.md §8):** her madde → *düzelt → regresyon testi yaz →
> derle/test/clippy → işaretle.* Fizik davranışı değiştiren her şey `headless_stress_test`
> ile doğrulanır; determinizm hash'i değişiyorsa **kasıtlı** olduğu burada yazılır.

**Durum anahtarı:** ⬜ yapılmadı · 🔄 devam · ✅ bitti · ⏸️ bloke/ertelendi

---

## Faz A — Dürüstlük ve güvenlik

*1.0'dan önce şart. Sırasıyla: yalan söyleyen şeyler → UB → determinizm → kapılar.*

### ✅ A3 — Unsound `unsafe impl`'leri kaldır
> **Bitti (2026-08-04).** Sonuç: 5 test eklendi, determinizm hash'i değişmedi
> (`EF6E4AC3644BF3BA`), clippy temiz.
>
> **DENETİM BU MADDEDE KISMEN YANILDI — kayda geçiyor:** `gizmo-ui`'deki iki `unsafe impl`
> "gereksiz, sarılan tipler zaten Send+Sync" diye raporlanmıştı. **Değiller.** Silince
> derleyici kanıtladı: `taffy::Style` → `Dimension` → `CompactLength` 64-bit'te tag+değer+
> pointer'ı tek bir `*const ()` içine paketliyor → yapısal olarak `!Send + !Sync`.
> Yani impl'ler **taşıyıcıydı**. Yapılan: silmek yerine *kanıtlanabilir* hale getirildi —
> taffy'nin `calc` feature'ı kapatıldı (pointer varyantının tek public constructor'ı
> `CompactLength::calc` ve o `#[cfg(feature = "calc")]`), impl'lere gerçek SAFETY yorumu
> yazıldı. **Kalan risk:** Cargo feature birleştirmesi — graftaki başka bir crate taffy'yi
> varsayılan feature'larla çekerse `calc` herkes için açılır ve bunu bir bağımlılık
> feature'ı olarak `cfg` ile gözlemlemek mümkün değil. Gerçek çözüm `taffy::Style`'ı
> component'te değer olarak tutmayı bırakmak → **A3-followup** olarak aşağıda.
>
> `ScriptEngine` bulgusu ise **tamamen doğruydu**. Çözüm: `Mutex<Lua>`'ya sarmak yerine
> (mlua 0.9'un lifetime'lı `Table<'lua>` tipleri 20 çağrı yerini zorlaştırıyordu) VM'e
> dokunan iki `&self` metodu — `has_function`, `run_entity_update` — `&mut self` yapıldı.
> Artık borrow checker eşzamanlı VM erişimini temsil edilemez kılıyor: çağıran
> `ResMut<ScriptEngine>` istemek zorunda, o da scheduler için exclusive yazma.
> `unsafe impl Send` tamamen kaldırıldı — derleyici otomatik türetti (mlua `send` feature'ı).
> Crate dışında çağıran yoktu, dolayısıyla çağrı yeri değişikliği sıfır.

### ⬜ A3-followup — `taffy::Style` bağımlılığını component'ten çıkar
`Style(pub taffy::style::Style)` bir ECS component'i olduğu için `Send+Sync` zorunlu, ama
taffy'nin tipi yapısal olarak değil. Şu an feature-gate'e dayanıyoruz (yukarıdaki kalan risk).
Kalıcı çözüm: component'te kendi POD layout tipimizi tut, `taffy::Style`'a yalnız layout
hesabı sırasında `UiContext` içinde dönüştür. Bu aynı zamanda `lib.rs:76`'daki
`pub use taffy::style::*` sızıntısını da kapatır.

### ✅ A4 — `query_entity_mut` aliasing kontrolü
> **Bitti (2026-08-04).** `query_entity_mut` ve simetri için `query_entity`'ye
> `check_aliasing` eklendi. 5 regresyon testi (`world::query::aliasing_gate_tests`):
> `(Mut<T>, Mut<T>)` ve `(Mut<T>, &T)` artık panikliyor, farklı component'ler hâlâ
> çalışıyor, read-only yolda tekrarlı `&T` hâlâ serbest, ve kapı canlılık kontrolünden
> ÖNCE çalışıyor (ölü id'de bile hatalı query tipi bildiriliyor).
> Taranan diğer giriş noktaları: `Query::new`, `new_cached`, `SystemParam` — üçü de zaten
> çağırıyordu; atlayan tek yer tekil-entity yoluydu.

### ✅ A5 — `fracture`'daki seed'siz RNG
> **Bitti (2026-08-04).** 3 regresyon testi, determinizm hash'i değişmedi.
>
> **Kapsam denetimden dar çıktı — kayda geçiyor.** `voronoi_shatter` ZATEN seed alıyordu ve
> motorun kendi kırılma yolu (`system.rs:387` → `shatter_entity`) sabit seed 42 kullanıyor,
> yani ECS yolu deterministikti. Hata yalnızca `generate_fracture_chunks`'taydı — motor içinde
> **hiç çağıranı olmayan**, sadece `lib.rs:63`'ten re-export edilen bir public API fonksiyonu.
> Yani "motor deterministik değil" değil, "public API tüketicisi için determinizm tuzağı".
>
> Yapılan: `generate_fracture_chunks`'a `seed: u64` parametresi eklendi (0.x, iç çağıran yok →
> `_seeded` varyantı yerine doğrudan imza değişikliği; eski davranışı yaşatmak bug'ı yaşatmak
> olurdu). Voronoi hücre düzeni `seed`'den, debris spin'i `seed ^ 0x9E3779B97F4A7C15`'ten
> (SplitMix64 sabiti) — iki akış ayrık, biri diğerinin çekiş sırasını kaydırmıyor.
> Doc yorumuna determinizm sözleşmesi + çalışan bir doctest yazıldı.
>
> Testler: aynı seed → bit-eş debris (pozisyon/hız/açısal hız/kütle); farklı seed → farklı
> (seed'i yok sayan bir implementasyon ilk testi bedavaya geçerdi); ve spin'in ayrıca
> seed'li olduğunu kanıtlayan test (yalnız `voronoi_shatter`'ı seed'leyip jitter'ı entropy'de
> bırakan kısmi düzeltmeyi yakalar).

### ⬜ A5-followup — `shatter_entity` sabit seed 42 kullanıyor
Determinizm ihlali DEĞİL, kalite sorunu: her nesne her kırıldığında **aynı** parça deseni
çıkıyor. Doğrusu entity id + frame sayacından deterministik türetmek — hem çeşitlilik hem
tekrarlanabilirlik. Davranış (görsel) değiştirdiği için Faz A kapsamına alınmadı.

### ✅ A1 — README iddia düzeltmesi
> **Bitti (2026-08-04).** Kaldırılan/düzeltilen yanlış iddialar: "Sweep and Prune (SAP)
> Broad-Phase with Rayon" → gerçek algoritma (dynamic AABB tree/BVH) + rayon iddiası gerçekten
> paralel olan island çözücüsüne taşındı; "`gizmo-physics` … zero-dependency" → öyle bir crate
> yok, 4 crate'in gerçek adları + `gizmo-core`'un hâlâ zorunlu olduğu açıkça yazıldı (D1'e
> referansla); "headless sunucu → renderer'ı çıkarın" → A2 bitene kadar kaldırıldı;
> "powered by mimalloc" ve "Doppler effect support" → kaldırıldı/dürüstleştirildi;
> audio'nun "native-only" olduğu iddiası düzeltildi (wasm32 backend'i var); editör ve
> WASM pipeline iddiaları gerçek kapsamlarına çekildi.
>
> **Eklendi:** Features listesinin başına **determinizm** maddesi — projenin tek gerçek
> rekabet hendeği hiç pazarlanmıyordu. TGS-Soft çözücü ve gerçek FEM de artık adıyla anılıyor
> (ikisi de doğrulanmış, satılmaya değer).

### ✅ A8 — Temiz clone'da çalışmayan demolar
> **Bitti (2026-08-04).** 3 test eklendi.
>
> `demo/src/lib.rs` eklendi (crate'in ilk paylaşılan yüzeyi) → `demo::assets::find` /
> `find_or_warn`: `$GIZMO_ASSETS` → `<repo>/assets` → `./assets` sırasıyla arar, bulamazsa
> `None`. `car_demo` ve `wind_tunnel` artık eksik modelde panik yerine prosedürel geometriye
> düşüyor ve nedenini stderr'e yazıyor. `car_demo`'da GLB'ye özel düzeltmeler
> (scale 2.0, GLB kök çocuğunu yeniden merkezleme/180° çevirme) `has_gltf_chassis` ile
> kapılandı; kutu fallback'i araç ölçülerine (1.46×0.8×2.7, `calculate_box_inertia` ile aynı)
> ölçekleniyor → görsel/fizik hizası korunuyor.
>
> İki hardcoded mutlak yol silindi (biri zaten eski repo adına işaret ettiği için yazarın
> kendi makinesinde de kırıktı). Workspace'te `/home/bedir` kalmadı.
> `assets/README.md` yazıldı: neyin commit'li olduğu, kendi modelini nasıl vereceğin,
> ve üçüncü-taraf/marka-lisanslı `.meta` dosyalarının repo lisansı kapsamında OLMADIĞI notu.
> `.gitignore`'daki "tracked via Git LFS" yalanı düzeltildi (LFS yapılandırması hiç yoktu).

### ✅ A2 — Feature kompozisyonunu düzelt
> **Bitti (2026-08-04).** Facade artık **her** feature kombinasyonunda derleniyor.
> `cargo hack --feature-powerset --depth 2`: gizmo-app **39/39**, gizmo-engine tam geçiş.
> Determinizm hash'i değişmedi.
>
> **Kök neden denetimin gördüğünden daha derindi.** `#[cfg]` eklemek yetmedi, çünkü
> `Transform`/`GlobalTransform`/`Collider` **`gizmo-physics-core`'da** yaşıyor ve o crate
> `physics` feature'ının arkasındaydı → `--features render` de `--features audio` da
> derlenemiyordu: transform'suz ne çizebilirsin ne de sesi uzamsallaştırabilirsin.
> Çözüm: `gizmo-physics-core` facade'da **zorunlu** bağımlılık yapıldı; `physics` artık
> *simülasyonu* (rigid-body çözücü = `gizmo-physics-rigid`) kapılıyor, uzamsal tipleri değil.
> Bu, crate'in gerçekte ne olduğuna da uyuyor: physics-core "uzamsal tipler + çarpışma
> primitifleri", bir simülasyon motoru değil.
>
> **Non-additive `window` feature'ı düzeltildi.** `gizmo-app` şöyleydi:
> `#[cfg(feature="window")] pub mod windowed;` + `#[cfg(not(feature="window"))] pub mod headless;`
> — yani `window` açmak `headless::App`'i **siliyordu**. Cargo feature'ları tüm graf boyunca
> birleştirdiği için, ilgisiz bir crate `window`'u açtığında headless sunucunun `App`'i sessizce
> pencereli olana dönüşüyor ve her `set_setup`/`run` çağrısı tip hatası veriyordu. Artık ikisi
> **yan yana** yaşıyor, ikisi de tam yolla erişilebilir; kök `pub use` pencereli olanı tercih
> ediyor (mevcut `gizmo_app::App` kodu aynen derleniyor).
>
> **Bilinen sınır:** `Plugin::build` kök `App`'e karşı tiplenmiş, dolayısıyla iki runtime
> birlikteyken `headless::App::add_plugin` derlenmiyor (`#[cfg(not(all(window, render)))]`).
> `set_setup`/`set_update`/`set_runner`/`run` koşulsuz çalışıyor — yani sunucu senaryosu
> ayakta. Kalıcı çözüm `Plugin`'i runtime üzerinden generic yapmak (11 impl imzası) →
> **A2-followup**.
>
> **CI kapısı eklendi:** `feature-powerset` işi (`cargo hack --depth 2`, iki giriş crate'i).
> Bu olmadan hata geri gelir — workspace build'i yalnız varsayılan feature setini derler,
> bulguyu 37 hataya kadar büyüten şey tam olarak buydu.
>
> Dokunulan dosyalar: `crates/gizmo/Cargo.toml`, `lib.rs`, `prelude.rs`, `plugins.rs`,
> `bundles.rs`, `asset_server.rs`, `systems/{mod,physics,transform,render/mod}.rs`,
> `crates/gizmo-app/src/{lib,headless}.rs`, `.github/workflows/ci.yml`.

### ⬜ A2-followup — `Plugin`'i runtime üzerinden generic yap
`pub trait Plugin<State> { fn build(&self, app: &mut App<State>); }` tek bir somut `App`'e
bağlı. Runtime'ı da tip parametresi yapmak (veya bir `AppLike` trait'i) `headless::App`'in
pencereli runtime varken de plugin kabul etmesini sağlar. 11 impl imzası etkilenir.

### ⬜ A-followup — `Transform`'u `gizmo-core`'a taşı
A2'nin ortaya çıkardığı asıl mimari kusur: motorun en temel uzamsal tipi bir *fizik*
crate'inde duruyor. `gizmo-physics-core`'un facade'da zorunlu olması bunu şimdilik
gizliyor ama fizik crate'lerini bağımsız paketlemeyi (D1) zorlaştırıyor ve
"transform istiyorum ama fizik istemiyorum" tüketicisini imkânsız kılıyor.

### 🔄 A7 — Doc-test'ler gerçekten çalışsın
> **Kısmen bitti (2026-08-04).** Çalışan doc-test: **7 → 12**. Kalan 30 `ignore`.
>
> **Ölçüm düzeltmesi:** ilk denetimde "20 hedefin 19'unda 0 çalışan test" doğruydu ama
> bunu "tüm doc örnekleri ignore" diye özetlemek fazla genişti — `gizmo-core`'un 7 tanesi
> (4'ü `compile_fail`, ki aliasing sözleşmesini koruyan gerçek testler) hep koşuyordu.
> `docs/AUDIT-2026-08.md` düzeltildi.
>
> Gerçek örneğe çevrilenler: `gizmo-core/time.rs` (time_scale'in `dt`'yi ölçekleyip
> `raw_dt`'yi etkilemediğini ve spike clamp'ini iddia ediyor), `gizmo/systems/spin.rs`,
> `gizmo/systems/lifetime.rs`. Yeni yazılanlar: `fracture::generate_fracture_chunks`
> (workspace'in **ilk** çalışan fizik doc örneği) ve `demo::assets::find`.
>
> **Yan bulgu — `[lib] name` eksik.** `crates/gizmo/Cargo.toml`'da `[lib]` bölümü yok, yani
> kütüphane adı `gizmo_engine`. `gizmo::prelude::*` YALNIZCA tüketici manifest'inde
> `gizmo = { package = "gizmo-engine" }` diye yeniden adlandırırsa çalışıyor — demolar bunu
> yapıyor, ama README'nin quickstart'ı ve `lib.rs`'in "the library ... is simply `gizmo`"
> cümlesi bunu söylemiyor. crates.io'dan kuran biri README'yi kopyalayınca derlenmez.
> Düzeltme `[lib] name = "gizmo"` eklemek (tek satır, docs'u doğru kılar) ama crate yolu
> public API olduğu için sürüm kararıyla birlikte alınmalı → **A7-followup**.

- [ ] Kalan 30 `ignore`: `gizmo-engine` 13, `gizmo-core` 10, `gizmo-renderer` 3,
      `gizmo-{analysis,animation,app,scripting}` 1'er. Derlenebilir olanları gerçek örneğe
      çevir; GPU/pencere gerektirenleri `no_run` yap (derlenir, koşmaz); gerçekten
      sözde-kod olanları ` ```text ` işaretle ve örnek iddiasını bırak.

### ⬜ A7-followup — `[lib] name = "gizmo"` ekle
README ve `lib.rs` `gizmo::` yolunu vaat ediyor; gerçek yol `gizmo_engine::`. Tek satırlık
düzeltme ama public crate yolunu değiştirdiği için A9 (sürüm) ile birlikte kararlaştırılmalı.

### ✅ A6 — CI kapıları
> **Bitti (2026-08-04).** Üç yeni kapı + topluluk dosyaları. `cargo deny` tamamen yeşil.
>
> **`feature-powerset`** (A2 ile geldi) — `cargo hack --depth 2`, iki giriş crate'i.
>
> **`supply-chain`** (`cargo deny --all-features check`). İlk koşu **6 advisory** buldu:
> - `crossbeam-epoch` geçersiz pointer deref → **düzeltildi** (`cargo update` → 0.9.20).
> - `quick-xml` ×2 (DoS) → `wayland-scanner ^0.39` pinliyor, >=0.41 erişilemez; üstelik
>   build-time protokol XML'i ayrıştırıyor, saldırgan girdisi değil. Gerekçeli muafiyet.
> - `ttf-parser`, `paste` bakımsız → transitive, yükseltme yolu yok. Gerekçeli muafiyet.
> - **`bincode 1.x` bakımsız → BU BİZİM.** `gizmo-net`'in DOĞRUDAN bağımlılığı ve rollback
>   snapshot'larının wire formatı. bincode 2.x var ama serializer değişimi wire-compat
>   kırıcısı, kendi değişikliğini ve round-trip testlerini hak ediyor → **A6-followup**.
>
> Lisans tarafı: grafta `GPL-2.0-only` ve `LGPL-2.1-or-later` göründü ama ikisi de
> **`OR` seçenekli** (`r-efi`, `self_cell`) — gerçek bulaşma **yok**. Buna karşılık
> denetimin işaret ettiği **MPL-2.0 gerçek**: varsayılan `audio` feature'ı üzerinden
> `rodio → symphonia`. Dosya-düzeyi copyleft, linkleme sorunu değil, ama README'nin düz
> "MIT OR Apache-2.0"ı bunu söylemiyor → `deny.toml`'da açık ve gerekçeli muafiyet.
>
> Yan düzeltmeler: `cradle`/`demo`/`server`/`demo-web` **lisanssızdı** (`publish = false`
> ama `license` alanı yok) → `license.workspace = true` eklendi.
>
> `multiple-versions` bilinçli olarak `warn`: üç `rand`, üç `getrandom`, iki `ron`,
> iki `glam` majoru var; bunları tekilleştirmek semver sonuçlu ayrı bir iş (D5) —
> kapıyı kalıcı kırmızı bırakmak yerine uyarıya çekildi.
>
> **`CONTRIBUTING.md`, `SECURITY.md`, `.github/dependabot.yml`** yazıldı. Dependabot'ta
> grafik yığını (wgpu/winit/egui/naga) tek PR'da gruplanıyor — MSRV'yi birlikte etkiliyorlar;
> `glam` major'ı ise bilinçli olarak **ignore** (public dep, D5 ile planlı yapılacak).

### ⬜ A6-followup — `bincode` 1.x → 2.x (gizmo-net)
Bakımsız ve doğrudan bağımlılık. Wire formatı değiştiği için rollback snapshot'ı ve
client-server mesajları round-trip testleriyle birlikte taşınmalı.

### ✅ A9 — 0.9.0 sürüm bump'ı
> **Bitti (2026-08-04).** `0.8.1` değil **`0.9.0`**: bu turda üç public imza değişti
> (fracture seed, ScriptEngine metotları, gizmo-app runtime'ları), yani patch değil minor.
>
> Root `[workspace.package] version` + **81 sibling path-dep stringi** 0.9.0'a çekildi;
> `Cargo.lock`'ta 20 crate yansıdı; README/ENGINE.md/CHANGELOG güncellendi.
>
> **DENETİM BU MADDEDE YANILDI — kayda geçiyor (düşmanca doğrulama zaten çürütmüştü,
> ben de elle onayladım).** İddia şuydu: "81 string elle güncellenir, birini unutursan
> `cargo publish` sessizce eski sürüme pinler". **Yanlış.** Cargo, `path` + `version`
> yazılmış bir bağımlılığın sürüm şartını hedef crate'in GERÇEK sürümüne karşı çözüm
> anında doğruluyor. Denedim: `gizmo-ai`'de tek bir stringi 0.8.0'a çevirince
> `error: failed to select a version for the requirement gizmo-math = "^0.8.0" /
> candidate versions found which didn't match: 0.9.0` alıp derleme anında düştü.
> Yani bu bir **ergonomi külfeti**, korektlik tehlikesi değil. Bunu doğrulamak için
> yazdığım testi de sildim — Cargo'nun zaten yaptığı kontrolü tekrarlıyordu.
>
> **`publish_all.sh`'te iki gerçek hata düzeltildi:**
> 1. Sürüm okuma **hiç çalışmıyordu**. Her crate `version.workspace = true` yazdığı için
>    `grep -m1 '^version'` o satırı yakalıyor, `sed` tırnak bulamayıp satırı olduğu gibi
>    döndürüyordu → script hep `(workspace)` basıyordu. Başlıktaki "her crate'in kendi
>    sürümünü okur" iddiası bu yüzden yanlıştı. Artık workspace mirasını root manifest'e
>    karşı çözüyor (kademeli 1.0'da crate'ler ayrı sürümlere geçtiğinde de çalışacak).
> 2. Başlıktaki sürüm `0.2.0`'da kalmıştı.
>
> **DRY_RUN sınırı dürüstçe belgelendi:** `cargo publish --dry-run` path-dep'leri registry
> sürümleriyle değiştirdiği için, henüz yayınlanmamış bir sürüme bump'tan sonra ilk
> katman dışındaki her crate çözülemez (gizmo-core 0.9.0 registry'de yok). Bu dry-run'ın
> doğası, script hatası değil — ama script onu tam prova gibi sunuyordu.
>
> **CI powerset job'ında hata bulundu ve düzeltildi:** `--locked` ile `--no-dev-deps`
> karşılıklı dışlayıcı (`--no-dev-deps` manifest'i yeniden yazıp çözümü değiştirdiği için
> lock dosyasına dokunmak zorunda). Job yazdığım haliyle CI'da kırmızı olurdu; yerel
> koşumda `--locked` kullanmadığım için fark etmemiştim.
>
> **Yayınlama YAPILMADI.** crates.io'ya `publish_all.sh` ile çıkmak geri alınamaz
> (sürümler silinemez) → ayrı ve açık bir onay gerektirir.

---

## Faz B — Oyunun yazılabilir olması

- ⬜ **B1 — Per-frame `Update` schedule.** `schedule.run` yalnız `windowed/event.rs:383,429`'da,
  ikisi de sabit-adım döngüsünün içinde. `Input` frame başına yazılıp (`:301`) temizleniyor
  (`:724`) ama 0..N kez tüketiliyor; `AutoNoVsync` hardcoded (`renderer/construction.rs:222,304`)
  olduğu için render frame'lerinin çoğunda **hiçbir sistem çalışmıyor** → basmaların ve mouse
  delta'nın çoğu kayboluyor. `time.rs:193-210`'daki interpolasyon alpha'sının tüketicisi de yok.
  → `Update` (frame) / `FixedUpdate` (fizik) ayrımı + alpha'yı render'a bağla.
  **App API kırıcısı — 1.0 dondurmasından ÖNCE yapılmalı.**
- ⬜ **B2 — Sahne sorgu katmanı.** `cast_shape`, `overlap_shape`, `project_point`,
  `QueryFilter { layers, mask, exclude, predicate }`. Primitifler hazır:
  `DynamicAabbTree::query_aabb`, `NarrowPhase::test_collision`, `CollisionLayer::can_collide_with`,
  ve bağlanmamış `Gjk::conservative_advancement`. Kanıt ki gerekli: motorun kendi karakter
  (`character.rs:64-76`) ve araç (`dynamics/systems.rs:41-49`) kodu broadphase'i atlayıp
  her frame O(N) tarıyor.
- ⬜ **B3 — Sahne registry'sini aç.** Şu an **8 tip** (`gizmo-scene/src/registry.rs:9-51`) →
  ışık/kamera/animasyon/ses ve *her kullanıcı component'i* sessizce kaydedilmiyor.
  `App::register_scene_component::<T>()` + `version` alanı + migrasyon zinciri.
- ⬜ **B4 — Joint çözücüsü.** Biriken impuls + warm-start yok (`joints/solver/mod.rs:43-122`);
  `center_of_mass` yok sayılıyor (`joint_types/fixed.rs:30-31`); `break_force` tek iterasyonun
  transient'inden hesaplanıyor; joint'ler island kurulumuna dahil değil (`pipeline.rs:560`).
- ⬜ **B5** — Gamepad girdisi. ⬜ **B6** — `PresentMode` yapılandırılabilir.
  ⬜ **B7** — Cylinder + Heightfield collider. ⬜ **B8** — ışık limitini kaldır
  (`gpu_types.rs:127` `[LightData; 10]`) + boş point-shadow pass'lerini guard'la
  (`passes/shadow.rs:57-97`, point-shadow varsayılan KAPALI olmasına rağmen her frame 6 kez koşuyor).

## Faz C — Performans ve ölçüm
- ⬜ **C1** — `benches/step_bench.rs` (solver/broadphase/narrowphase) + commit'lenmiş baseline.
  Şu an bu üçü için **sıfır** benchmark var; ENGINE.md'deki tüm perf sayıları tekrarlanamaz.
- ⬜ **C2** — Broadphase refit (`pipeline.rs:145-176` her substep sıfırdan kuruyor, statikler dahil).
- ⬜ **C3** — `physics-rigid/src/system.rs:149-158` O(N²) writeback → handle→index map.
- ⬜ **C4** — Temas yolunda `ArrayVec` (`narrowphase/mod.rs:400-407`); rewind geçmişi opt-in
  (`world/step.rs:122-128` her frame tam klon).
- ⬜ **C5** — `[profile.release]` (`lto="thin"`, `codegen-units=1`). Kökte yok; `.cargo/config.toml`
  bu makinede `lto=off` zorluyor → tüm perf sayıları alt sınır.
- ⬜ **C6** — Index buffer (`components/mesh.rs:8-9`) + mipmap + anizotropik filtreleme.

## Faz D — Ekosistem ve 1.0
- ⬜ **D1** — `gizmo-core`'u fizik crate'lerinde opsiyonel yap (`ecs` feature'ı) + 4 fizik
  crate'ine kendi README/description/keywords. `PhysicsWorld` **zaten ECS'siz** (60 dosyadan
  5'i `gizmo_core`'a dokunuyor) → bu bir paketleme işi, yeniden yazım değil. 80 crate → ~40.
- ⬜ **D2** — `ENGINE.md`'yi İngilizce'ye çevir + `///` yorumlarında İngilizce kuralı +
  `CONTRIBUTING.md`. Bus factor = 1'in tek sebebi bu.
- ⬜ **D3** — Click-to-try WASM demosu (GitHub Pages).
- ⬜ **D4** — `#![warn(missing_docs)]` Stage A'da (public API'nin %53'ü belgesiz).
- ⬜ **D5** — `glam` 0.29 → 0.32. Tek kasıtlı public dep, 3 major geride, varsayılan-KAPALI
  `bevy_reflect 0.15`'in bağımlılığı tutuyor; grafta `transform-gizmo` üzerinden 0.32 de var.
  **1.0'dan önce şart** — sonrası 2.0-seviyesi kırıcı.
- ⬜ **D6** — İki yönlü soft↔rigid coupling (`soft_body.rs:74-120` impulsu hesaplayıp atıyor).
- ⬜ **D7** — `gizmo-ui` metin render'ı, ya da crate'i dürüstçe "deneysel" işaretle
  (şu an hiçbir şey çizmiyor: `gizmo-ui/src/lib.rs:39-52`).

## Faz E — 1.0
Kademeli 1.0 planı (ENGINE.md §4) sağlam. **Stage A'ya girmeden önce bitmiş olmalı:**
A1–A9, B1 (App API kırıcısı), D1, D4, D5.
Ayrıca `gizmo-scene`'in Stage A'da olması planın **kendi kuralıyla çelişiyor** — public error
enum'unda `ron 0.8` tipleri var (ENGINE.md §4'ün Stage B kriteri tam da bu).

---

## Kapsam dışı / bilinçli olarak yapılmayacaklar
- `gizmo-audio`'nun cfg-gate'li `unsafe impl Send/Sync`'i — doğru ve gerekçeli, dokunulmayacak.
- ENGINE.md §7'deki çürütülmüş false-positive'ler ve non-goal'lar (narrowphase batch-SIMD,
  cross-platform bit-determinizm, N≥48 kule) — yeniden kovalanmayacak.
- Denetimde **düşmanca doğrulamayı geçemeyen 9 iddia** rapordan çıkarıldı, burada da yok.
