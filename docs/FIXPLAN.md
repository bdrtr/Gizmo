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

### ✅ A7-followup — `[lib] name = "gizmo"` eklendi
> **Bitti (2026-08-04), 0.9.0'a dahil.** `crates/gizmo/Cargo.toml`'a `[lib] name = "gizmo"`.
> Artık `cargo add gizmo-engine` + `use gizmo::prelude::*` manifest'te yeniden adlandırma
> olmadan çalışıyor — README quickstart'ı, crate dokümanı ve bütün örnekler bu yolu
> gösterdiği hâlde crates.io'dan kuran biri ilk satırda `unresolved crate` alıyordu.
>
> Crate'in **kendi** entegrasyon testleri (`car_demo_integration.rs`, `ccd_bundle.rs` —
> 8 satır) ve A7'de yazdığım iki doctest eski `gizmo_engine::` yolunu kullanıyordu;
> hepsi `gizmo::`'ye çevrildi. `lib.rs`'e, vaat edilen yolun gerçekten derlendiğini
> kanıtlayan bir doctest eklendi — asıl mesele buydu.
>
> Teknik olarak kırıcı (doğrudan `gizmo_engine::` yazan varsa), CHANGELOG'da geçiş
> notuyla belirtildi.
>
> **Not:** `cargo package -p gizmo-engine` hâlâ başarısız — ama `[lib]` yüzünden değil:
> path-dep'ler registry sürümleriyle değiştiği için `gizmo-ai 0.9.0` bulunamıyor. Tam da
> A9'da `publish_all.sh`'e yazdığım sınır. `cargo package -p gizmo-math` (yayınlanmamış
> kardeşi yok) sorunsuz geçiyor → paketleme makinesi sağlam.

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

- ✅ **B1 — Per-frame `Update` schedule.** *(2026-08-04)*
  > **Yapılan.** `App` artık iki schedule taşıyor:
  > - `schedule` — sabit adım, frame başına `0..N` kez, sabit `dt` (fizik). **Değişmedi**;
  >   hiç kimsenin sistemi yerinden oynamadı.
  > - `update_schedule` — **tam olarak** frame başına bir kez, gerçek frame `dt`'siyle.
  >   `App::add_update_system` / `add_update_system_mut` ile kaydedilir.
  >
  > Sıralama `gizmo-app/src/frame.rs::run_fixed_and_update`'e çıkarıldı: accumulator'ı boşalt
  > → alpha hesapla → update'i bir kez koştur. Event loop artık bunu çağırıyor. Çıkarmanın
  > asıl kazancı test edilebilirlik: denetimin "gizmo-app 2553 satır için 1 test" bulgusuna
  > karşılık bu dosya **7 testle** geliyor ve pencere/GPU gerektirmiyor.
  >
  > **Kanıtlanan sözleşme:** 600 fps'te 60 Hz accumulator'a karşı 9 frame boyunca sabit
  > schedule **hiç** koşmuyor ama update 9 kez koşuyor. Tersi de test edildi: uzun bir
  > frame'de sabit schedule 4 kez, update yine 1 kez. Ayrıca: iki schedule'a farklı `dt`
  > gidiyor (sabit → sabit adım, update → gerçek frame delta'sı), duraklatılmış simülasyon
  > (`sim_dt = 0`) update'i durdurmuyor, per-step hook'u (rollback snapshot'ı taşıyor)
  > adım başına tam bir kez ateşliyor ve adımsız frame'de hiç ateşlemiyor.
  >
  > **`FpsLookPlugin` update_schedule'a taşındı** — `mouse_delta`'yı okuyor ve o değer
  > render-frame başına toplanıp temizleniyor; sabit schedule'da hareketin bir kısmını
  > (adımsız frame'lerde hiçbirini) görüyordu. `crates/gizmo/tests/update_vs_fixed_schedule.rs`
  > yerleşimi davranışsal olarak kilitliyor: sabit schedule'ı koşturunca kamera **oynamamalı**,
  > update'i koşturunca **oynamalı**.
  >
  > **Kendi hatam, kayda geçiyor:** `lib.rs`'e `pub mod frame;` eklerken satır-eşleştirmeli
  > düzenleme `#[cfg(feature = "physics")]`'i `gameplay`'den alıp `frame`'e kaydırdı →
  > `--no-default-features` kırıldı. Workspace testi bunu göremezdi; **powerset kapısı
  > yakaladı** (A2'de eklediğim iş, ilk gerçek getirisi bu oldu).
  >
  > **Kapsam dışı bırakılanlar (bilinçli):** `add_system`'in varsayılanı sabit schedule
  > olarak KALDI — Update'e çevirmek mevcut kullanıcıların sistemlerini sessizce değişken
  > `dt`'ye taşırdı. Motorun diğer sistemlerinin (transform propagate, UI layout) taşınması
  > her biri ayrı davranış doğrulaması istediği için ayrı iş → **B1-followup**.
  > `AutoNoVsync` (B6) ve interpolasyon alpha'sının render'da tüketilmesi de ayrı.

- 🔄 **B1-followup — motorun kalan per-frame sistemlerini taşı** *(kısmen, 2026-08-04)*
  > **`gizmo-ui` taşındı.** Layout pencere boyutuna, interaction fare konumuna karşı
  > çözülüyor — ikisi de render-frame başına tazelenen sunum durumu. Sabit schedule'da
  > accumulator'a göre `0..N` kez koşuyordu; vsync kapalıyken bir hover ~10 frame'de bir
  > kaydediliyor, resize birkaç frame'de yansıyordu. Simülasyon hızıyla ilgisi yok.
  >
  > **`headless::App`'e de `update_schedule` eklendi.** `gizmo-ui`, `gizmo-app`'i
  > default-features'sız çektiği için oradaki `App` **headless** olan; alan olmayınca
  > `UiPlugin` derlenmiyordu. Bu, A2-followup'ta yazdığım `Plugin` genericity sorununun
  > pratikteki maliyeti. **Uyarı:** headless'ta sabit-adım döngüsü YOK — tek tick gerçek
  > `dt` ile koşuyor, dolayısıyla iki schedule aynı kadansta. Alan taşınabilirlik için var.
  >
  > **`TransformPlugin` TAŞINMADI — ve sebebi bir bulgu.** `default_render_pass` zaten
  > çizimden hemen önce `ensure_global_transforms(world)` çağırıyor
  > (`systems/render/mod.rs:101`) ve o da **aynı iki sistemi** (`TransformSyncSystem`,
  > `TransformPropagateSystem`) elle koşturuyor. Yani:
  > - `FpsLook`'u per-frame'e taşımam bayat kamera YARATMADI — doğruladım, render yolu
  >   kendi tazeliyor. (Endişelendim, kontrol ettim, endişe yersizdi.)
  > - Buna karşılık `TransformPlugin`'in sabit schedule'daki kaydı **mükerrer**: sistemler
  >   frame başına `0..N` kez orada, bir kez daha render'da koşuyor. Denetimin "transform
  >   local matrisleri her frame dirty-flag'siz yeniden hesaplanıyor" bulgusunun üstüne
  >   bir de duplikasyon biniyor.
  > - Kaydı silmek düz bir kazanç DEĞİL: `gizmo-studio` kendi render pipeline'ına sahip
  >   (`render_pipeline/mod.rs:234` `GlobalTransform` okuyor) ve `ensure_global_transforms`
  >   çağırmıyor → silmek editörü bozar. Tüketici-başına doğrulama gerektiriyor.
  >
  > Kalan: `streaming`, ve interpolasyon `alpha`'sının render'da tüketilmesi (judder).

### ✅ B1-followup-2 — `TransformPlugin` mükerrerliği çözüldü
> **Bitti (2026-08-04).** `TransformSyncSystem` + `TransformPropagateSystem`
> `update_schedule`'a taşındı. 2 test.
>
> **Kaydettiğim engel yokmuş — kontrol edince çıktı.** "gizmo-studio sabit-schedule kaydına
> bel bağlıyor" demiştim; `gizmo-studio/src/update.rs:43-46` bu iki sistemi **zaten kendisi
> koşturuyor**. Yani kimse o kayda bağlı değildi: render yolu `ensure_global_transforms`
> ile, studio kendi eliyle hallediyor.
>
> Taşıma bir maliyet düzeltmesinden fazlası: **sıralama artık yapısal.** `PhysicsPlugin`'in
> `physics_step` etiketinin yorumu transform sistemlerinin "kendini ondan sonraya
> sıralayabileceğini" söylüyor ama `.after("physics_step")` edge'i **hiç bağlanmamış** —
> sabit adım içinde sıra batcher'ın seçtiğiydi. `update_schedule` frame'in *bütün* sabit
> adımlarından sonra koştuğu için "transform'lar fizikten sonra yayılır" artık etikete değil
> yapıya dayanıyor. Ayrıca per-frame update sistemlerinden de sonra — `FpsLookSystem`'in
> oynattığı kameranın ihtiyacı tam olarak bu.
>
> `default_render_pass`'teki `ensure_global_transforms` **kaldı**: plugin'i hiç kaydetmeyen
> özel bir `App` için emniyet ağı, ve yeni spawn edilmiş mesh'e `GlobalTransform` iliştiren
> yer o. Plugin kayıtlıyken yayılım zaten güncel oluyor.

### ✅ B1-followup-3 — headless runtime artık sabit-adım koşuyor
> **Bitti (2026-08-04).** `run_default` artık `frame::run_fixed_and_update` kullanıyor —
> windowed runtime'la birebir aynı sıralama.
>
> **Öncesi:** döngü `schedule.run(world, dt)`'yi gerçek geçen süreyle, iterasyon başına bir
> kez çağırıyordu. Alttaki 1 ms `sleep` ile bu saniyede ~1000 tick demek; yani
> `PhysicsPlugin` kaydeden bir sunucu fizik sistemlerini **saniyede ~1000 kez** duvar-saati
> `dt`'siyle adımlıyordu, aynı plugin windowed'da sabit 60 Hz'de adımlarken. Bir plugin bir
> kez yazılıp iki runtime'da aynı davranamıyordu — ve bu, sabit adımın en çok önemsendiği
> yerde (adanmış sunucu) tersine dönmüş hâldeydi.
>
> **Not:** simülasyon determinizmi zaten çökmüyordu, çünkü `PhysicsWorld::step` kendi içinde
> 240 Hz sabit substep uyguluyor. Sorun kadans ve boşa iş idi, bit-eşlik değil —
> `headless_stress_test` hash'i değişmedi (`EF6E4AC3644BF3BA`).
>
> `server/` ve `cradle/` derleniyor.

- 🔄 **B2 — Sahne sorgu katmanı** *(çekirdek bitti, 2026-08-04)*
  > `crates/gizmo-physics-rigid/src/world/scene_query.rs` + 17 test.
  >
  > **Eklenen API:** `QueryFilter` (layer mask, çoklu body dışlama, trigger dahil/hariç,
  > builder'lı), `raycast_filtered`, `overlap_shape`, `point_query`, `cast_shape`, `cast_body`.
  > Hepsi broadphase-hızlandırmalı (`spatial_hash.query_aabb` → kesin narrowphase).
  >
  > **Yolda bulunan gerçek hata — `Gjk::conservative_advancement` bozuk.** Shapecast'i onun
  > üzerine kurmayı planlamıştım; denetim onu "tamamen implement edilmiş ama bağlanmamış"
  > diye işaretlemişti. **Bağlanmamış olduğu için bozuk, bozuk olduğu için bağlanmamış.**
  > Ölçtüm: yalnızca şekiller tam karşıdan hizalıyken çalışıyor. 5 birim uzaktaki iki birim
  > kutu, **0.2** yanal kayıklıkla → `None`:
  > ```
  > it0: dist=4.000  n=(-1,0,0)      closing= 20.0   ilerle
  > it1: dist=0.141  n=(-.71,0,.71)  closing= 14.1   ilerle
  > it2: dist=0.089  n=(-.45,0,.89)  closing=  8.9   ilerle
  > it3: dist=0.447  n=(+.89,0,.45)  closing=-17.9   REDDET
  > ```
  > `it3`'te kutular **zaten örtüşüyor** ama `Gjk::distance()` pozitif mesafe + ters normal
  > döndürüyor. İki kusur birlikte: `dist/closing_vel` adımı yanal kayıklıkta konservatif
  > değil (örtüşmenin içine taşıyor), ve `distance()` penetrasyonu bildiremiyor.
  >
  > `cast_shape` bu yüzden CA yerine **`test_collision` üzerinde march-and-bisect**: march
  > adımı sabit bölme yerine ilgili **en küçük extent'ten** türetiliyor (200 birimlik
  > süpürmede 2 cm'lik duvar testi bunu kilitliyor — sabit 512-örnekli march onu atlardı),
  > ardından 24 bisection.
  >
  > **Kalan:** `raycast_all` için filtre, `project_point`, sorguların facade/ECS katmanına
  > açılması.
  >
  > **Karakter/araç `O(N)` taramalarının taşınması — incelendi, ERTELENDİ.** Denetim bunu
  > "asıl kazanç" diye işaretlemişti ve öyle; ama sandığımdan büyük bir iş çıktı.
  > `character.rs:65-76` ile `dynamics/systems.rs:41-49` **`PhysicsWorld` üzerinde değil**,
  > ECS'ten toplanan bir `Vec<(BodyHandle, Transform, Collider)>` üzerinde çalışıyor
  > (`gather_colliders`). Yeni sorgu API'si `PhysicsWorld`'e ait, dolayısıyla taşımak bu
  > sistemleri farklı bir veri kaynağına bağlamak demek — ve `PhysicsWorld`'ün broadphase'i
  > denetimin de yazdığı gibi **bir substep bayat** (`world/step.rs`: broadphase, position
  > integration'dan ÖNCE kuruluyor). Yani düz bir çağrı değişimi değil, davranış değişimi;
  > kendi doğrulamasını ve muhtemelen broadphase tazeliği kararını istiyor.

### ✅ B2-followup — `Gjk::conservative_advancement` düzeltildi
> **Bitti (2026-08-04).** 5 yeni test + 1 vakum assertion gerçek yapıldı.
> **Determinizm hash'i DEĞİŞMEDİ** (`EF6E4AC3644BF3BA`) — tahmin edildiği gibi.
>
> **BENİM TEŞHİSİM YANLIŞTI, düzeltiyorum.** B2'de "adım `dist/closing_vel` yanal kayıklıkta
> konservatif değil" yazmıştım. **Değil** — 17-ajanlık analiz bunu çürüttü ve elle
> doğruladım: saf ötelemede Minkowski kümesi rijit ötelendiği için destek düzleminin ofseti
> **tam doğrusal** azalır, yani `Δt = gap/closing` en fazla temasa oturur, asla geçmez.
> Adım kuralı ders kitabı (Mirtich) bound'u ve doğru.
>
> **Gerçek kusur PAYDA değil, PAY'daydı.** `distance()` `max(best_sq.sqrt(), lb_max)`
> döndürüyor — *bayat bir iterasyondan* gelen bir ÜST sınır — ve normali başka bir
> iterasyondan alıyor. İzlemedeki ilk hata `it3` değil **`it1`**: orada `pos_a=(4,0,0.2)`,
> A x[3.5,4.5] ile B x[4.5,5.5] **tam temas ediyor** (gerçek mesafe 0, `t=0.2` tam TOI).
> Ama simplex indirgemesi orijine ulaşıp `min_dist_sq < 1e-8` break'ini tetikliyor,
> best-iterate güncellemesinden ÖNCE — ve bayat `0.1414`'ü raporluyor. Dokunma kapısı
> (`dist < 0.001`) bu yüzden hiç ateşlemiyor; sonraki adım şekilleri örtüşmeye taşıyor;
> oradan sonra `distance()` çöp döndürüyor ve ayrılma sertifikası tüm süpürmeyi reddediyor.
>
> **Yapılan (yalnız CA içinde, `distance()`'a tek bayt dokunulmadı):**
> - `distance()` artık yalnız **yön** sağlıyor; boşluk her iterasyonda sertifikalı destek-düzlemi
>   ofsetinden yeniden türetiliyor: `sep(n) = support(-n)·n`. Her `n` için gerçek mesafeden
>   küçük-eşit → adımlar eskisinden ASLA büyük değil.
> - `t=0`'da örtüşme artık `Gjk::test_collision` ile tespit edilip TOI 0 dönüyor (eskiden
>   duran-örtüşen çift "ıskaladı" sayılıyordu).
> - `closing <= 0` sertifikası **KORUNDU** — sezgisel değil, kalıcı-ayrılma ispatı.
> - Tam `max_t`'ye düşen temas artık düşürülmüyor; ufuk **döngüdeki dokunma ölçütüyle**
>   sınanıyor (`test_collision` ile değil — tam temas örtüşme DEĞİLDİR, o kontrol tam da
>   korumak istediğimiz teması eler; kendi testim yakaladı).
> - Dönen impact normal'i artık son *güvenilir* eksenden; `distance()` yakın-sıfır boşlukta
>   literal `Vec3::X` fallback'i veriyor.
>
> **Mevcut testin assertion'ı vakumdu:** `normal.x.abs() > 0.99` — `Vec3::X` fallback'i de
> geçiyordu. `normal.x < -0.99` yapıldı (gerçek B→A ekseni).
>
> **`distance()` neden dokunulmadı:** üretimde `speculative_contact` → `pipeline.rs:317`
> (CCD) kullanıyor ve dönen boşluk üç eşikten geçip doğrudan `ContactPoint::penetration`'a
> ve çözücüye akıyor. Penetrasyon kanalı eklemek CCD davranışını değiştirirdi. Yalnız bir
> doküman notu eklendi: örtüşen girdide sonuç anlamsız, containment için `test_collision`.

- 🔄 **B3 — Sahne registry'si açıldı + sürümlendi** *(2026-08-04)*
  > `gizmo-app/src/scene_registry.rs` (yeni, 3 test) + `SceneData.version` + facade delegesi.
  > `gizmo-app` 1 → **11 test**.
  >
  > **Boşluk sandığımdan kötüydü.** Denetim "registry 8 tip kapsıyor" demişti; asıl sorun
  > save/load yolunun `default_scene_registry()`'yi **her çağrı yerinde elle yeniden
  > kurması**ydı (`editor_runtime.rs` ×2, `windowed/lifecycle.rs` ×1), her seferinde `Script`'i
  > satır içi ekleyerek. Facade'ın `full_scene_registry()`'si ise **hiç çağrılmıyordu** — ölü
  > kod. Yani kullanıcının kendi bileşenini kaydedebileceği bir yer yoktu: nereye eklerse
  > eklesin save/load yolu onu görmüyordu.
  >
  > **Yapılan:** kurulum tek bir yere (`gizmo-app::scene_registry`) taşındı, 3 çağrı yeri
  > oraya bağlandı, facade'ın `full_scene_registry()`'si ona delege ediyor. Artık
  > `let mut reg = gizmo::full_scene_registry(); reg.register_serializable::<Health>("Health")`
  > gerçekten save/load yoluna ulaşıyor — facade'da derlenen bir doctest bunu gösteriyor.
  >
  > **Kaydedilenlere eklendi:** `Camera`, `Camera2D`, `PointLight`, `DirectionalLight`,
  > `SpotLight` (render feature'ı), `AudioSource` (audio). Yani "sahneyi kaydet, ışıklar
  > gitsin" bitti. `Material` **bilinçli olarak dışarıda**: canlı bir wgpu bind group tutuyor,
  > olduğu gibi serileştirilemez — sahne round-trip'inin PBR haritalarını kaybetmesi ayrı bir
  > bulgu ve çözümü ayrı.
  >
  > **Sürümleme:** `CURRENT_SCENE_VERSION = 1`, `SceneData.version` + `PrefabData.version`
  > (`serde(default)` → eski dosyalar 0 okunur), `SceneData::migrate()` ve yeni
  > `SceneError::UnsupportedVersion`. Daha YENİ bir motordan gelen dosya artık **hata
  > veriyor**, sessizce yüklenmiyor: bilinmeyen alanlar parse anında zaten kaybolmuş olurdu,
  > dolayısıyla yüklemek kullanıcının verisini sessizce atmak olurdu.
  >
  > **Kalan:** `Material` serileştirmesi (PBR round-trip), `Mesh`/`MeshSource` yolu,
  > animasyon/araç bileşenleri (`BoneAttachment` dışında serileştirilebilir olan yok),
  > ve gerçek bir migrasyon zinciri sınandığında 0→1'in ötesi.

- 🔄 **B4 — Joint çözücüsü.** Dört maddesi vardı; ikisi bitti, ikisi sırada. Ayrıntı için
  aşağıdaki **B4 bölümüne** bak. Kalan: `break_force` yeniden kalibrasyonu (commit 3),
  motor satırları (commit 4), substep'ler arası warm-start + rollback (commit 5), ve
  joint'lerin island kurulumuna dahil edilmesi (`pipeline.rs:560`).
- ⬜ **B5** — Gamepad girdisi. ⬜ **B6** — `PresentMode` yapılandırılabilir.
  ⬜ **B7** — Cylinder + Heightfield collider.
- 🔄 **B8** — iki parçası vardı:
  - ✅ **Boş point-shadow pass'leri** (2026-08-04, `af6f168`): `record_shadow_passes` artık
    shader'ın zaten okuduğu `point_shadows_enabled` bayrağına bakıyor. Varsayılan kapalı
    olduğu için her frame 6 depth pass'i (aydınlatılan bir batch'in 23 draw'ının 12'si)
    hiç örneklenmeyen bir 1024²×6 cubemap'e yazılıyordu. Golden-image testi bayrağın iki
    hâlinde bit-eş kare talep ediyor — hem iddiayı (atlanan iş gözlemlenemezdi) kanıtlıyor
    hem de shader'ın GERÇEKTEN örneklediği bir pass'i yanlışlıkla kapatmaya karşı koruyor.
  - ⬜ **10 ışık tavanı** hâlâ duruyor (`gpu_types.rs:127` `[LightData; 10]`, kararsız ECS
    iterasyon sırasına göre seçilen ilk 10, mesafe/öncelik culling'i yok) — clustered/tiled
    ışık culling'i gerekiyor. Deferred pipeline'a sahip olmanın asıl gerekçesi bu.

### ✅ CCD gating — uyanık dinamik hedefler artık backstop'lanıyor
> **Bitti (2026-08-04).** `ccd_hole_fast_body_vs_dynamic_awake_thin_plate` artık
> `#[ignore]`'suz geçen **Rung 8**; `ccd_analytical` 7→8 test.
>
> **Sorun bir GATING sorunuydu, geometri değil** (17-ajanlık analiz bunu tespit etti ve
> doğruladım). `pipeline.rs`'teki backstop `if (rb_j.is_dynamic() && !rb_j.is_sleeping) || is_trigger`
> ile uyanık dinamik hedefleri **topluca** atlıyordu. Rung 7 (birebir aynı sahne, plaka
> UYUYOR) zaten geçiyordu → mevcut ray-vs-şişirilmiş-AABB süpürmesi bu sahneyi hallediyor,
> yalnız kapı çalışmasına izin vermiyordu.
>
> Atlamanın gerekçesi gerçekti ama kapı gerekçeden **çok daha genişti**: hareketli bir
> hedefte süpürme kare-uyumsuz — merminin `old_pos`'u entegrasyon-ÖNCESİ, hedefin
> `compute_aabb`'si SONRASI. Kapıyı kaldırmak yerine **uyumsuzluğu** düzelttim: hedefin
> AABB'si artık kendi substep deltasıyla süpürülüyor (şimdiki kutu ⊕ substep başındaki
> kutu). Statik/uyuyan hedefte delta sıfır → eski davranış birebir korunuyor.
>
> **`ccd_hole_fast_spinning_thin_body` artık dürüst.** Denetim "assertion'ı yok, ignore
> kalkarsa hata dururken yeşil geçer" demişti — doğruydu. Gerçek bir oracle eklendi:
> `angular_damping = 0.0` olduğu için temas hiç üretilmezse ω tam 200.0'da kalır;
> `|ω| < 190` iddiası. `--ignored` ile koşturunca artık **gerçekten kırmızı** — dönme CCD'si
> hâlâ yok, ama test bunu artık kanıtlıyor, gizlemiyor.
>
> **Determinizm:** `headless_stress_test` hash'i pre-session temel değerimle aynı
> (`EF6E4AC3644BF3BA`). Bu anlamlı bir kanıt: `RigidBody::new` varsayılanı
> `ccd_enabled: true`, yani kuledeki 200 kutunun CCD'si AÇIK — değişiklik onları
> etkileyebilirdi. Engage kapısı (`travel <= min_half`) düşen bir kutu için hiç
> tetiklenmediği için etkilemiyor. Cross-process determinizm testi de geçiyor.
>
> **UYARI (analizden, kayda değer):** `headless_stress_test` **davranışı kilitlemiyor**.
> Üç koşuyu birbiriyle karşılaştırıyor (`hashes[0]==hashes[1]==hashes[2]`), hiçbir sabitle
> değil — `EF6E4AC3644BF3BA` yalnızca yazdırılıyor. Yani kapı *nondeterminizmi* yakalar,
> davranış değişimini yakalamaz. Yukarıdaki "hash aynı" kanıtı benim kendi temel değerimle
> elle karşılaştırmam; CI bunu yapmıyor. → **Golden hash fixture'ı** hâlâ açık iş.
>
> **Bilinen sınır (Rung 7'de de belgeli):** backstop clamp anında hedefe momentum
> aktarmıyor. Clamp'ten sonraki substep'te ayrık çözücü teması görüp aktarıyor, dolayısıyla
> etki sınırlı — ama hafif bir plakaya karşı mermi bir kare "ölü durur".

### ✅ CCD — belgelenen iki açık da kapandı (`ccd_analytical` 9 test, 0 ignored)
> **Bitti (2026-08-04).** Rung 8 (uyanık dinamik hedef) bir önceki turda gerçek düzeltmeydi.
> Rung 9 (dönen ince cisim) ise **bir düzeltme değil, bir çürütme**.
>
> **Belgelenen dönme açığı ölçümü geçemedi. Senaryosu da iddiası da hatalıydı:**
>
> 1. **Senaryo bozuktu.** Yassı kutu merkezi x=-1.0, x-yarı-uzanımı 1.0 → köşesi t=0'da
>    duvara (x=0) **zaten değiyordu**. İlk karede temas impulsu yiyip savruluyor (ω 200→194),
>    sonra 60 kare boyunca duvara bir daha hiç değmiyor (**ölçüm: 0/60 örtüşme**, AABB
>    uzaklaşıyor). "Dönen kenar duvarı süpürür ama temas üretilmez" diyordu; kenar duvarı
>    hiç süpürmüyordu. Üstelik assertion'ı yoktu → ignore kalksa hata dururken yeşil geçerdi.
>
> 2. **İddia tutmadı.** Temiz başlayan senaryolarda ayrık narrowphase yakalıyor:
>    | plaka | ω | substep yayı | pencere | sonuç |
>    |---|---|---|---|---|
>    | y=0.9 | 200 rad/s | 47.7° | 118° | ω 200→117 |
>    | y=0.985 | 600 rad/s | 143° | 65° | ω 600→544 |
>    Uzun bir çubukta penetrasyon penceresi substep yayından dar olmuyor — süpüren uç değil,
>    gövdenin tamamı.
>
> **Kendi ara bulgum da yanlıştı, kayda geçiyor.** Bir ara "ECS yolu hiç temas üretmiyor,
> doğrudan `PhysicsWorld` üretiyor" ölçtüm ve bunu ciddi bir ayrışma sandım. Artefaktmış:
> o denemede `sticky()` (**sürtünme = 0**) ve yarı süre kullanmıştım. Dönen bir cismi
> yavaşlatan teğetsel impulstur. Sürtünmeli malzeme + 2 saniyeyle ECS yolu ω'yı
> **117.487**'ye düşürüyor — doğrudan `PhysicsWorld::step` ölçümüyle **birebir aynı**.
> İki yol ayrışmıyor.
>
> **DİKKAT:** bu dönme CCD'si VAR demek değil. `speculative_contact` hâlâ yalnız öteleme,
> backstop yalnız doğrusal merkez deltasını süpürüyor. Yeterince uç bir konfigürasyon
> muhtemelen hâlâ tünelller — kurulamadı, o kadar. Belgelenen açık ise yanlıştı, ve yanlış
> bir "bilinen açık" taşımak gerçek bir açık taşımaktan daha pahalı: kimse kovalamıyor.

### ✅ Golden state fixture'ı — davranış artık kilitli
> **Bitti (2026-08-04).** `crates/gizmo-physics-rigid/tests/golden_state.rs`, 5 test.
>
> **Boşluk gösterildi, iddia edilmedi.** `PHYSICS_HZ`'i 240→120 yaptım (gerçek bir davranış
> regresyonu) ve iki kapıyı yan yana koştum:
> | | sonuç |
> |---|---|
> | `headless_stress_test` | ✅ "DETERMINISM VERIFIED" — **tamamen kör** |
> | golden fixture | ❌ **5'ten 3'ü kırmızı**, `reference/measured/delta` diff'iyle |
>
> **Neden hash değil, tolerans.** `state_hash` bit-eş ve açıkça **aynı-platform**
> (`ENGINE.md §5`) — CI Linux/macOS/Windows'ta koşuyor, commit'lenmiş bir hash üçünden
> ikisinde hata değil *platform* yüzünden kırmızı olurdu. Değer+tolerans platform
> saçılmasını soğuruyor ama anlamlı her değişimi yakalıyor.
>
> **Neden bu sahneler.** Hepsi bilinçli olarak **iyi-koşullu**: zemine oturma, sürtünmeyle
> durma, serbest düşüş, duran yığın. Son durumları yakınsıyor, dolayısıyla cross-platform
> f32 sapması toleransın çok altında kalıyor. Kaotik sahne (200-kutu kule çöküşü) tam tersi
> — son durumu son bit'e keyfî hassas, hiçbir tolerans "farklı platform" ile "farklı fizik"i
> ayıramaz. O yüzden burada YOK; onu değer değil sınır iddia eden soak testleri kapsıyor.
>
> `TOL = 1e-3` ölçümle seçildi: kilitlenen büyüklükler 1–100 mertebesinde, gerçek bir
> davranış değişimi (çözücü iterasyonu, sönümleme varsayılanı, sürtünme combine modu)
> onları 1e-2+ oynatıyor, platform sapması birkaç mertebe altında.
>
> Hata mesajı ölçülen değeri **yapıştırılabilir biçimde** basıyor ve commit mesajında
> eski→yeni değerin kaydını istiyor — kasıtlı değişimlerin sessizce yeniden kutsanmaması için.
>
> Ayrıca `state_hash`'in süreç-içi tekrarlanabilirliği artık demo binary'sinde değil test
> suite'inde de iddia ediliyor — ama bilinçli olarak bir sabitle KARŞILAŞTIRILMIYOR, çünkü o
> motorun vermediği bir cross-platform bit-eşlik iddiası olurdu.

### 🔄 B4 — Joint çözücüsü

Denetimin dört maddesi vardı. Sırayla ve her biri kendi başına doğrulanabilir olacak
şekilde iniyor.

**✅ commit 1 — `lever_arm` kütle merkezinden ölçülüyor** (`8495819`). Her joint satırı
`anchor - transforms[idx].position` — yani transform ORİJİNİ etrafındaki kolu —
hesaplıyordu. Bileşik collider'lar, kırılma parçaları ve araç şasileri için `center_of_mass`
sıfır değil, dolayısıyla bu cisimlere bağlı her eklem yanlış tork ve yanlış efektif kütle
görüyordu. 14 çağrı yeri tek yardımcıdan geçiyor; `tests/joint_com.rs` (3 test) yardımcıyı
geri alınca kırmızıya dönerek değişimin davranışsal olduğunu kanıtlıyor.

**✅ commit 2 — biriken λ + tek-yönlü satırların geri verebilmesi.** `JointRows([f32; 10])`
`Joint` üzerinde, `is_broken` gibi `#[serde(skip)]`; yuvalar `joints::solver::row` içinde
derleme-zamanı sabiti (ilerleyen bir imleç, koşullu atlanan satırlar yüzünden λ'ları yanlış
satıra yazardı). Clamp artık artıma değil geçiş boyunca birikmiş TOPLAMA uygulanıyor.

> **Ölçüldü — ve denetimin iddiası bu haliyle çürüdü.** Denetim "çözücü doğru impulse'ın
> `iterations` katına kadarını uygulayabiliyor" diyordu. Mevcut sahnelerde bu OLMUYOR:
> hinge-limit, slider-limit, koni-limit ve motor-limit sahnelerinde eski ve yeni kod
> **bit-eş veya 1 ULP** fark veriyor; 6 halkalı bir zincirde fark 600 adımda ~8e-4 m.
> Sebep, tek-yönlü satırın rakibinin zayıf olması: satır her iterasyonda `Jv`'yi zaten
> sıfırlıyor, sonraki artım ~0 kalıyor. Yani düzeltme DOĞRU ama mevcut sahnelerde ATIL —
> hiçbir committed threshold yeniden kutsanmadı, `EF6E4AC3644BF3BA` kıpırdamadı.
>
> Değeri iki yerde: (a) `break_force`'un doğru hesaplanabilmesi için bir λ TOPLAMI gerekiyor
> ve commit 3'ün önkoşulu bu; (b) cırcır gerçek bir kusur, sadece latent — güçlü rakibi olan
> sahnelerde (dış impuls, temas beslemesi) ortaya çıkardı. Bu yüzden senaryo eşiği yerine
> **ayırt edici birim testleri** yazıldı: `a_one_sided_row_can_return_the_impulse_it_applied`
> ve `a_one_sided_row_never_pushes_past_its_bound` eski semantikle kırmızı,
> `accumulated_lambda_does_not_leak_between_passes` sıfırlama silinince kırmızı. Üçü de
> tersine çevrilerek doğrulandı.

> **XPBD/CFM geri besleme terimi KASITLI OLARAK ERTELENDİ.** Plan `- α̃·λ_toplam` terimini
> birikimle birlikte zorunlu sayıyordu; ölçüm bunu çürüttü. Terim fizik olarak doğru
> (1 kg yük, α=0.03, dt=1/240 → öngörülen statik uzama α·m·g/β = 0.98 m, `max_correction_speed`
> 5000'e çekilince ölçülen 1.007 m ✓) ama bu çözücüde `position_bias` HIZ-KIRPILI. Kırpma
> ısırdığı anda denge λ_toplam = bias_max/α̃'ya tavanlanıyor, bu taşınacak yükün çok altında
> kalıyor ve kısıt sessizce boşalıyor: aynı sahnede 2 m'lik halat 600 adımda **27.4 m**'ye
> uzuyor (pratikte serbest düşüş), α=0.3'te 31.9 m. Terimsiz haliyle compliance davranışı
> HEAD ile bit-eş (2.014287 / 6.078255 / 27.403154) — yani "birikim tek başına compliant
> eklemleri rijitleştirir" uyarısı da bu şemada geçerli değil. Terim, kırpma rejimiyle
> BİRLİKTE ele alınmalı; compliance'ın iterasyon-sayısına bağımlılığı o zamana kadar açık.

**✅ commit 3 — `break_force` net tepkiden hesaplanıyor.** Sekiz iterasyon-içi kontrol tek
bir geçiş-sonrası kontrole indi; ölçülen şey artık `‖Σ λᵢ·nᵢ‖ / dt`. `JointRows`,
`JointScratch` oldu ve λ'ların yanında geçişin net doğrusal/açısal impulse vektörünü de
taşıyor. `Joint::check_break` (sıfır çağıranı olan **ölü kod**) tek yol hâline geldi.

> **Asıl kusur L1 toplamıydı, iterasyon bağımlılığı değil.** Eş-doğrusal OLMAYAN satırların
> büyüklüklerini toplamak, taşınan kuvveti yükün dünya eksenlerine göre yönelimine bağlı
> kılıyordu: aynı 9.81 N'luk yük, yerçekimi bir eksen boyunca iken 9.81 N, köşegen iken
> 17 N olarak raporlanıyordu. Ball-socket'te (koni/twist/swing dik bile değil) abartmanın
> üst sınırı yok. `break_force_measures_the_net_reaction_not_the_sum_of_axis_magnitudes`
> tam bunu — YÖNDEN BAĞIMSIZLIĞI — iddia ediyor ve eski kodda kırmızı.
>
> Denetimin "iterasyon sayısına bağımlı" iddiası ise ÖLÇÜMLE ÇÜRÜDÜ: 4/10/20 iterasyonda
> kopma eşiği 22.5617 N, üçünde de aynı. Sebep birikim cırcırını da atıl bırakan sebep —
> satır ilk iterasyonda `Jv`'yi sıfırlıyor, 2..N iterasyonlar toplama ≈0 katıyor.
> `break_force_does_not_depend_on_the_solver_iteration_count` bu yüzden bir REGRESYON
> BEKÇİSİ olarak etiketlendi, bu commit'in kanıtı olarak değil (eski kodda da geçiyor).
>
> İki kusur daha kapandı: `fixed.rs`'teki `err_len >= 1e-4` kapısı kusursuz sabitlenmiş bir
> kaynağın lineer kontrolünü tamamen atlıyordu; ve slider süspansiyon yayı ile hinge torsiyon
> yayı gerçek yük taşıdıkları hâlde break kontrolüne hiç görünmüyorlardı — "kopabilir" bir
> amortisör sonsuz yük taşıyabiliyordu (`a_suspension_spring_reports_its_load_to_break_force`,
> eski kodda kırmızı). Motorlar/sürücüler bilinçli olarak DIŞARIDA: onlar dış yük değil
> eyleyici. Eklem artık iterasyon ortasında değil geçiş sonunda kopuyor → kopma adımında bir
> adımlık fazla impuls transferi. CHANGELOG'a girdi.

**✅ commit 4 — motor bütçesi geçişin toplamına uygulanıyor + ilk joint golden'ı.**
`hinge.rs`/`slider.rs`'deki `motor_max_force * dt / self.iterations`, eksik olan birikimin
elle yazılmış vekiliydi (Türkçe yorumu bunu zaten söylüyordu). Motor artık yuva 9'da
`accumulate`'ten geçiyor ve bütçe TOPLAMA uygulanıyor.

> **Ölçülen fark: yakınsama.** Toplamı doğru sınırlaması açısından iki şema denk — kopma
> eşiği gibi burada da beklenen "N kat fazla kuvvet" gerçekleşmiyor. Fark, `iterations`'ın
> ne olduğunda: eski bölmeyle motorun etkisi koştuğu döngünün UZUNLUĞUNA bağlıydı, yani
> `iterations`'ı artırmak cevabı iyileştirmiyor, DEĞİŞTİRİYORDU. Yüklü bir servo kolunda,
> 5/10/20/40 iterasyon:
>
> | | 5 | 10 | 20 | 40 |
> |---|---|---|---|---|
> | eski, tepe açı | 1.7588705 | 1.7593216 | 1.7595481 | 1.7595280 |
> | yeni, tepe açı | 1.7590271 | 1.7590069 | 1.7590047 | **1.7590047** |
>
> Yeni şemada 20'de oturuyor ve 40 aynısını veriyor; eskisi hiç oturmuyor (stall'daki servo
> ve doymuş hız motorunda da aynı tablo). `a_force_limited_motor_converges_as_iterations_rise`
> tam bunu iddia ediyor — eski kodda kırmızı, üç rejimde de.

> **`golden_state.rs` artık bir joint sahnesi içeriyor** (`golden_hinge_pendulum_swing`).
> CI'ı kapatan hiçbir şeyde eklem yoktu: `headless_stress_test`, `determinism.rs`,
> `rollback.rs`, `soak_and_golden.rs` ve golden'daki diğer beş sahne joint'siz. Bu
> kampanyadaki dört joint değişikliğinin hiçbiri bir kapıyı kıpırdatamazdı; her biri elle
> kurulan sahnelerle ölçüldü. En keskin ölçüt kol UZUNLUĞU DEĞİL, kol HATASI (×1000):
> Baumgarte katsayısını 0.3→0.25 düşürmek onu 0.226 (226 tolerans) oynatırken `pendulum x`'i
> ancak 1.0e-3 oynatıyor — mutlak 1e-3'lük bir kol-uzunluğu kilidi ise kısıt hatasının iki
> katına çıkmasını "değişmedi" sayardı.

### ✅ Derin düzeltmeler — kökten giden üç madde (2026-08-04)

B4'ün beş commit'i "eklem çözücüsünün yaptığı işi" düzeltti. Bunlar ise ALTINDAKİ üç yapıyı
düzeltti; üçü de tek tek yamanmak yerine mekanizması değiştirildi.

**✅ `compliance` gerçek bir ters-sertlik oldu** (`d49bedb`). CFM (`k += α/dt²`) tek başına
yumuşaklık üretmiyor: `k`'yi büyütmek yalnız her iterasyonun adımını küçültür, seri yine
RİJİT çözüme yakınsar. Gözlenen tüm yumuşaklık `iterations`'ın sonlu olmasındandı — aynı
halat 5 iterasyonda 0.0194 m, 10'da 0.0096 m uzuyordu. Eklemler artık temas çözücüsünün
kullandığı soft-constraint formülasyonunda (`solver/tgs.rs:117-124`, `:597`); frekans satırın
kendi compliance'ı ve efektif kütlesinden geliyor: `ω = √(k/α)`. Sonuç Hooke: 1 kg,
α = 0.03 → 0.294 m, iki büyüklük mertebesi compliance ve bir mertebe kütle boyunca %0.2
içinde, 5/10/20/40 iterasyonda aynı.

> **İlk denemem 1 kg'da mükemmel ölçüp 4 kg'da patladı.** `impulse_scale` terimi `k`'ye
> bölününce λ yinelemesi `impulse_scale > 2k` olduğunda ıraksıyor; cisim 331 m düşüyordu
> (2000 adım serbest düşüş). Bölünmeyen biçimde `impulse_scale ∈ (0,1]` ve koşulsuz kararlı.
> Tek kütleli bir test bunu geçirirdi — bu yüzden `compliance_is_an_inverse_stiffness`
> kütleyi de tarıyor.
>
> Commit 2'de ertelenen CFM geri besleme terimi de böylece kapandı: soft formülasyonda `c`
> bir ÇARPAN (`λ = c·bias_rate·C`), CFM'de ise BÖLEN'di. Aynı fizik, kırpmalı bir çözücüde
> taban tabana zıt koşullanma.

**✅ Eklem durumu `WorldSnapshot`'a girdi** (`f5042af`). `is_broken` TEK YÖNLÜ bir mandal ve
`joints` snapshot'ta hiç yoktu → rollback penceresi içinde kopan eklem restore'dan sonra da
kopuk kalıyordu, kalıcı olarak. Aynısı `initial_relative_rotation` için: bütün koni/twist/swing
limitlerinin ölçüldüğü referans poz. İkisi de `state_hash`'e girmediği için desync hızlara
sızana kadar GÖRÜNMEZDİ. Türe ekleme kuralı da yazıldı: ölçüt "büyük mü" değil,
*transform/velocity'den türetilebilir mi*.

**✅ Uyandırma eklem bileşenine yayılıyor** (`699c68d`). `pipeline.rs`'teki yayılım tek geçiş
ve dizi sırasına bağlıydı: 12 halkalı zincirin derin ucu sarsıldığında bir adımda 5 halka
uyanıyordu (substep başına bir), kalan 7'si entegre etmediği eklem düzeltmelerini yutuyordu.
Temaslarda island'lar bunu zaten çözüyordu; eklemler island kurulumuna girmiyordu. Artık aynı
union-find eklem grafında: bileşende bir mover varsa bileşenin tamamı uyanır.
`an_undisturbed_chain_stays_asleep` fixi dürüst tutuyor — "hepsini uyandır" o testi kırar.

### ⬜ Ölçüldü: warm-start ne kadar önemli?

Denetim warm-start'ı "büyük olan" diye işaretlemişti. **Ölçüm bunu kısmen destekliyor: yalnız
YÜKSEK KÜTLE ORANLARINDA.** 16 halkalı rijit zincir, varsayılan 10 iterasyon, 400 adım:

| uç kütlesi | 10 iter | 40 iter | 160 iter |
|---|---|---|---|
| 1 kg | 16.0066 (%0.04) | 16.0014 | 16.0002 |
| 20 kg | 16.0229 (%0.14) | 16.0056 | 16.0012 |
| 200 kg | **16.1657 (%1.04)** | 16.0443 | 16.0106 |

Sıradan kütle oranlarında sapma 6.6 mm/16 m — görünmez. 200:1'de %1, ve 160 iterasyona
çıkmak 16 kat düzeltiyor; işte warm-start'ın kapatacağı boşluk bu. Yani: **yıkım topu zincirde,
halatta ağır platform** gibi sahneler için değerli, genel bir kusur değil.

İki engeli artık kalktı: λ zaten `Joint::scratch` içinde, o da artık snapshot'lanıyor; ve
uyuyan-uç kapısı için gereken bileşen bilgisi de var. Kalan iş: iterasyon 0'dan önce ayrı bir
warm-start sweep'i, bir satırın aktivasyon kapısı kapandığı substep'te λ'sının sıfırlanması
(gevşemiş halat yeniden çekmemeli), ve `tests/rollback.rs`'e yüksek-kütle-oranlı bir zincir
sahnesi.

### ⬜ Ölçülecek: temas çözücüsünde de aynı bölme var mı?

`solver/tgs.rs:597` `impulse_scale` terimini `/ k_n` ile bölüyor — eklemlerde 4 kg'da
kısıtı boşaltan yapının aynısı. Oradaki `impulse_scale` çok daha küçük (contact_hertz = 30,
ζ = 10 → ≈0.058), yani kararlılık sınırı `m_eff ≈ 34`'e çıkıyor ve mevcut soak sahnelerinde
ısırmıyor. **Körlemesine değiştirilecek bir şey değil** — önce ağır cisimli bir temas sahnesi
kurup ölçmek gerekiyor.

**⬜ commit 5 — warm-start + rollback (en riskli, en sonda).** İterasyon 0'dan ÖNCE ayrı bir
sweep'te `dir * λ_önceki` uygulanması (temas çözücüsündeki `solver/mod.rs:458-476` yapısının
aynısı). Aynı commit'te ZORUNLU: iki-uç-uykuda kapısı (bugün `joints/` içinde tek bir
`is_sleeping` referansı yok); bir satırın aktivasyon kapısı kapandığı substep'te λ'sının
sıfırlanması (gevşemiş halat yeniden çekmemeli); ve joint durumunun `WorldSnapshot`'a
eklenmesi. λ substep sınırını geçtiği an bu ŞART olur — `WorldSnapshot`'ın kendi Türkçe
gerekçesi (`world/mod.rs:294-301`) tam olarak bunu söylüyor. Aynı fırsatta bugün de var olan
delik kapanır: `is_broken`, `initial_relative_rotation`, `current_angle`/`current_position`
hiçbiri snapshot'lanmıyor, yani rollback bugün de bir eklemi "kırılmamış" hâline döndüremiyor.
`tests/rollback.rs`'te joint içeren sahne YOK → bu commit `build_scene`'e bir tane eklemeli;
yoksa desync yeşil ışıkta geçer ve `state_hash` (yalnız transform/velocity/sleep) onu göremez.

### ⬜ GPU test flake'i — kısmen çözüldü, kalanı ölçüldü
`crates/gizmo/src/systems/render/mod.rs`'teki golden-image testleri her biri kendi wgpu
cihazını açıyordu ve `cargo test` bir binary'nin testlerini paralel koşturuyor → eşzamanlı
cihaz oluşturma sürücüde **SIGSEGV**'e dönüyor, tüm workspace koşusunu düşürüyordu.
Rust panic'i değil, sürücü çöküşü olduğu için hiçbir test adı raporlanmıyordu.

**Yapılan:** üç GPU testi de process-genelinde tek bir mutex'in arkasına alındı (`gpu_lock`).

**Ölçüm (bu makine):**
| durum | sonuç |
|---|---|
| kilit ÖNCESİ, workspace koşusu | 2 koşuda 2 çöküş |
| kilit SONRASI, binary izole (paralel + tek-thread) | **8/8 temiz** |
| kilit SONRASI, tam workspace koşusu | ~12 koşuda 2 çöküş |

Yani binary-içi yarış çözüldü; kalan çöküş yalnızca **tam workspace koşusunda** ve izole
koşuda hiç tekrarlanmıyor → sürücü/sistem seviyesinde, sürekli yük altında.

**Etki alanı sanılandan geniş:** `cargo bench --workspace --benches -- --test` de release
modda lib testlerini koşturuyor, yani GPU testleri orada da çalışıyor. Flake bu yüzden
**iki** CI job'ını birden vurabilir (`test` ve `benchmarks`) — bu turda bir kez `benchmarks`
job'ının yerel karşılığında görüldü, ardışık 3 koşu temiz geçti.

**Kalan neden:** cihaz *sayısı*. `shadow-gate` (`af6f168`) `render_frame`'i bayrağın iki
hâliyle çağıran bir test ekledi; artık bir koşuda ~5 kez `Renderer::new_headless` çağrılıyor
(öncesi 3).

**Denendi ve REDDEDİLDİ: tek paylaşılan `Renderer`.** `OnceLock<Mutex<Renderer>>` ile
5 cihazı 1'e indirmeyi denedim (`Renderer: Send` — doğruladım). Derledi, ama
`skipping_the_point_shadow_passes_changes_no_pixel` **kırıldı**: iki kare arasında 9586 bayt
fark. Sebep öğretici — `Renderer` **kareler arası durum taşıyor** (TAA history ve muhtemelen
bloom/SSGI geçmişi). Golden testlerin tamamı "temiz durumdan tek kare" varsayımına dayanıyor;
renderer'ı paylaşmak tam da o varsayımı bozuyor. Yani sorun "cihazı paylaş" değil.

**Doğru çözüm: cihazı paylaş, renderer'ı DEĞİL.** Her test hâlâ taze bir `Renderer` almalı
ama hepsi aynı `wgpu::Device`/`Queue` üzerinde kurulmalı. Bunun için
`Renderer::new_headless_with_device(device, queue, w, h)` gibi bir constructor gerekiyor —
şu an yok. Renderer cerrahisi olduğu için ayrı iş.

**Ara çare (isteğe bağlı):** her test kendi cihazını bir kez kurup iki render'ını onunla
yapsın → 5 cihaz 3'e iner. Kısmi, ama `render_frame`'e `&mut Renderer` parametresi
eklemekten ibaret.

## Faz C — Performans ve ölçüm
- ✅ **C1 — `benches/step_bench.rs`** *(2026-08-05)*. `gizmo-physics-rigid`'de beş senaryo
  grubu: broadphase (temassız kafes), narrowphase (üst üste binmiş), solver (oturmuş kule),
  joints (asılı zincir), full_step (karışık). Her iterasyon sahneyi YENİDEN KURUYOR —
  `iter_batched`'in setup'ı ölçüme girmiyor, ve tek bir dünyayı bin kez adımlamak her seferinde
  başka bir simülasyon ölçer (cisimler oturur, uyur, maliyet çöker ve bu hızlanma sanılır).

  **Bu makinedeki baseline** (2026-08-05, `lto=off`/`codegen-units=4` ile — yani TAVAN DEĞİL,
  ALT SINIR; mutlak sayılar makineye özel, karşılaştırma için aynı makinede önce/sonra koş):

  | senaryo | 64/8 | 256/24 | 1024/48 |
  |---|---|---|---|
  | broadphase | 226 µs | 609 µs | 1.73 ms |
  | dense_contacts (solver-bound) | 6.96 ms | 27.66 ms | **151.00 ms** |
  | solver (kule) | 532 µs | 1.20 ms | 4.05 ms |
  | joints (zincir 8/32/128) | 161 µs | 317 µs | 755 µs |
  | full_step (128/512) | 635 µs | 2.43 ms | — |

  > **İlk ölçüm benim senaryo adlandırmamı ÇÜRÜTTÜ, ve asıl bulgu bu.** Grubu
  > `narrowphase_overlapping` diye adlandırmıştım. Motorun kendi `PhysicsMetrics`'iyle faz
  > kırılımını alınca (1024 cisim): broadphase 25 ms, narrowphase 36 ms, **solver 669 ms** —
  > yani zamanın **%91'i solver'da**. `step` üzerinden narrowphase'i izole etmek zaten mümkün
  > değil, çünkü ürettiği her temas sonra çözülüyor. Grup `dense_contacts_solver_bound` olarak
  > yeniden adlandırıldı; yanlış ad birini yanlış fazı optimize etmeye yollardı.
  >
  > **Süper-lineerliğin yeri kesinleşti:** temas SAYISI N ile lineer (her boyutta cisim başına
  > ~20 temas), ama solver'ın **temas başına** maliyeti sabit kalmıyor:
  >
  > | N | temas | solver | temas başına |
  > |---|---|---|---|
  > | 64 | 1631 | 33.99 ms | 20.84 µs |
  > | 256 | 4847 | 104.84 ms | 21.63 µs |
  > | 1024 | 20754 | 669.47 ms | **32.26 µs** |
  >
  > `island_count` boyunca 4'te sabit, yani island büyüyor ve temas başına maliyet onunla
  > birlikte artıyor.

- ✅ **Sebep bulundu: adaptif iterasyon sayısı** *(2026-08-05, 18 ajanlık soruşturma)*.
  **Ve elediğim gerekçe yanlıştı.** "Sıfır yerçekimi → yığın yok → derinlik < 5" demiştim.
  `island_depth` yığın yüksekliği DEĞİL, **temas grafının BFS eksantrikliği**.
  `support_order_manifolds` BFS'i statik/kinematik anchor'lardan kökler; o sahnedeki kutular
  y=5'te yüzüyor ve zemine hiç değmiyor, dolayısıyla anchor'suz yedek yol kafesin bir
  köşesinden kök alıyor ve derinlik kafes çapı oluyor: **√N−1 = 7 / 15 / 31**.

  `n_iterations = min(96, max(cfg, max(28, 1.5·derinlik)))` → **28 / 28 / 46** sweep.
  Büyümenin **~%93'ü** bu; sweep başına temas maliyeti neredeyse düz.

  > **Enstrümantasyonsuz, bit-eş doğrulama** (altı ajanın altısı bağımsız aynı sonuca vardı,
  > düşman yargıç bunu sıfır kaynak düzenlemesiyle tekrarladı, ben de elle tekrarladım): TGS
  > yolunda adım sonrası durum sweep sayısının saf fonksiyonu, yani `iterations = 1` ile aynı
  > `state_hash`'i veren en büyük `iterations` efektif sayının ta kendisi. Diz: havadaki sal
  > için **28 / 28 / 46**, aynı sal zemine indirilince **1 / 1 / 1**.

- ⏸️ **Düzeltme AÇIK — "anchor'suz island'da ölçekleme yapma" DENENDİ ve YANLIŞ ÇIKTI.**
  Bariz görünüyordu: anchor yoksa destek zinciri yok, ekstra sweep hiçbir şeye yakınsamıyor.
  Uyguladım, sahnede solver 669 → 360 ms'ye indi ve temas başına maliyet düzleşti
  (20.84/21.63/32.26 → 17.03/17.91/17.77). **Ama `soak_resting_stacks_stay_bounded` N=24 ve
  N=32'de patladı.** Sebep ölçüldü: uzun bir kule zeminden bir kıl payı ayrıldığında island
  anchor'suz kalıyor (24 cisimlik kule için 37 kez, 32 için 13 kez, max_depth 23/31) — ama
  içsel destek zincirini koruyor ve sweep'lere gerçekten ihtiyacı var. Değişiklik geri alındı.

  > **Ayırt edici ölçüt anchor'lanma DEĞİL.** Aranan şey "derinlik bir destek zinciri mi
  > ölçüyor yoksa bir kafes çapı mı" — 1B zincirde eksantriklik N−1 ve anlamlı, 2B salda √N ve
  > (en azından yüksüz/sıfır-yerçekimi halinde) anlamsız. Ayrıca salın sweep'i kısılınca
  > SİMÜLASYON KALİTESİNİN bozulup bozulmadığını ölçmedim; yalnız hızı ölçtüm. Düzeltme,
  > kalite ölçütü olmadan yapılmamalı.
  >
  > Yan bulgu: `BLOCK_ITERS_FLOOR = 28` varsayılan `iterations = 20`'den BÜYÜK, yani derinliği
  > ≥5 olan her island çağıranın indiremeyeceği bir tabana çarpıyor (24 → 32 sweep).
  >
  > Solver da 24→48 kulede 2× yükseklik için **3.39×** süre veriyor — ENGINE.md'nin
  > "N≥48 kuleler bükülüyor" notuyla aynı yere işaret ediyor.

- ✅ **Kalite ölçütü KURULDU — `tests/solver_quality.rs`** *(2026-08-05)*. Bir önceki oturum
  düzeltmeyi "yalnız hızı ölçtüm, kaliteyi ölçmedim" diye durdurmuştu. Eksik alet artık var:
  4 CI kapısı + 7 ölçüm koşusu, hepsi `PhysicsWorld`'ün genel API'sinden, solver'a hiçbir
  enstrümantasyon eklemeden.

  > **Artık penetrasyon enstrümantasyonsuz okunabiliyor.** Solver `ContactPoint::penetration`'ı
  > yalnız `pen0`'a okuyup geri yazmıyor (`solver/tgs.rs:302`), yani `collision_events()`'teki
  > derinlik o substep'in BAŞINDAKİ narrowphase derinliği — tam olarak bir önceki substep'in
  > çözümünün gideremediği örtüşme. `soak_and_golden.rs`'in eksen-hizalı yığın formülünün
  > aksine her geometride çalışıyor.

  **Yeni public API: `ConstraintSolver::adaptive_iterations`** (varsayılan `true`). Kapatınca
  `iterations` her island için TAM sayı oluyor. Bu olmadan merdivenin ilgi çekici yarısı —
  tabanın altı — erişilemez: adaptif formül `max(cfg, …)` olduğu için `iterations`'ı düşürmek
  efektif sayıyı düşürmüyor. Varsayılanda davranış-nötr: `EF6E4AC3644BF3BA` kıpırdamadı.

  **ÖLÇÜLEN BULGULAR — beşi de düzeltmenin nasıl yapılacağını değiştiriyor:**

  1. **Artık ölçümler burkulmaya KÖR, ve bu yapısal.** 4×12×4 sandık bloğu, varsayılan
     konfigürasyonla: 1559 kare boyunca `max|v|` 0.02–0.19, `pen_max` 0.0055'te düz, enerji
     11.270'te düz, `tilt` 0.1°. Sonra 1679. karede `max|v|`=2.18, 1799'da 16.3 → çökmüş.
     **26 saniyelik tertemiz okuma, sonra çöküş.** Yalnız artık-yakınsama ölçen bir ölçüt geri
     alınan düzeltmeyi ONAYLARDI.

  2. **Tek yörüngeli geç/kal kapısı float gürültüsüyle çevrilebiliyor.** Statik zeminin yarı-
     boyutunu 20 → 200 yapmak (zemin statik, üst yüzü iki halde de y=0; değişen tek şey daha
     büyük bir yüze karşı temas kırpmanın float ayrıntısı) N=24 kuleyi "1500 kare sınırlı"dan
     "1140'ta burkuldu"ya çeviriyor. N=16'da 200 m zeminde `peak|v|` = **0.485**, soak'ın 0.5
     eşiğinin kılpayı altı. → Kapı sürekli bir MARJ olmalı, tek koşunun ikili sonucu değil;
     ölçümler 3 zemin boyutuyla topluluk olarak koşuyor.

  3. **Sweep sayısında TEKDÜZE DEĞİL.** N=32 kule: 46 sweep 0/3 patlıyor, **96 sweep 3/3
     patlıyor**. "Daha çok sweep daha güvenli" varsayımı yanlış — hiçbir kısma kuralı buna
     yaslanamaz.

  4. **Bench'in salı kalite sorusunu CEVAPLAYAMAZ.** Sal bir oturma sahnesi değil, bir patlama:
     N=256'da ilk karede 3670 J kinetik enerji yaratılıyor, cisimler ~7.8 m/s savruluyor,
     215. karede temas sayısı sıfır. Başlangıç durumu (0.1 m örtüşen kafes) fiziksel değil ve
     kafesin genişlemeden ulaşabileceği örtüşmesiz bir konfigürasyon yok → karşılaştırılacak
     doğru cevap yok. Merdiven bunu doğruluyor: daha çok sweep daha ÇOK icat edilmiş enerji
     veriyor (103 J @1 sweep → 3780 J @46), hangisinin daha iyi olduğunu söyleyecek bir ölçüt
     yok. Aynı kafes TAM TEMASTA kurulunca (sahne `scene_floating_lattice`) her sweep sayısında
     **tam 0.0000** okuyor ve her cisim uyuyor: **yüksüz ankrajsız bir kümede sweep'lerin
     yakınsayacağı hiçbir şey yok.**

  5. **VE ASIL BULGU — 12 KATLI YIĞINLAR GÜVENİLİR DURMUYOR.** 3000 karede, varsayılan
     konfigürasyonla, 1'den 4'e her genişlikte, boşluklu ya da boşluksuz, denenen her sweep
     sayısında. 12, ENGINE.md'nin "oyun yapıları ≤~12, o yüzden önemi yok" dediği zarfın TAM
     TEPESİ, ve soak'ın yeşil tuttuğu 32'lik kuleden çok daha alçak.

     | blok | 28 sweep (zemin 20 / 200) | 96 sweep (20 / 200) |
     |---|---|---|
     | 1×12×1 | — / **2328** *(varsayılan)* | 8 zorlanmış sweep'te duruyor |
     | 2×12×2 | 2451 / 1979 | **2782 / 2037** — 96 KURTARMIYOR |
     | 3×12×3 | 2379 / 1373 | 2687 / — |
     | 4×12×4 | 1267 / 1447 | — / — ← 96'nın kurtardığı TEK şekil |
     | 2×6×2 | — / — | — / — ← yükseklik 6 sağlam |
     | 3×6×3 | — / 2670 | — / — |

     > ⚠️ **BU BİR DÜZELTME.** İlk okumada yalnız 4-genişlikteki bloğu ve yalnız TEK zemini
     > koşup "politika sweep'i ~4× eksik veriyor, derinlik tek başına yanlış girdi" yazmıştım
     > (commit `ce26c52`). Zeminleri ayrı ayrı koşunca çürüyor: **96 sweep yalnız 4-genişliği
     > kurtarıyor, 2-genişlikte hiç yardım etmiyor.** Yani bu bir sweep BÜTÇESİ sorunu değil,
     > sweep politikası da düzeltmesi değil — ve dolayısıyla "derinlik yanlış girdi"nin KANITI
     > da değil (o iddia hâlâ makul, ama bu veriyle desteklenmiyor).

     > **Verinin gerçekten desteklediği şey daha dar ve daha kötü:** yükseklik 12'de sonucu
     > fiziksel içeriği olmayan pertürbasyonlar belirliyor. Oynatılan her düğme sonucu
     > TEKDÜZE OLMAYAN biçimde çeviriyor — statik zeminin yarı-boyutu (20 duruyor, 200
     > çöküyor), sütunlar arası yanal boşluk (tam temas duruyor, 2 cm 70. karede çöküyor,
     > 20 cm duruyor), sweep sayısı (8 zorlanmış sweep duruyor, 28 adaptif sweep çöküyor),
     > genişlik (1 ile 2 farklı). Bu tam olarak `soak_and_golden.rs` kök-neden notunun tarif
     > ettiği imza: 1'in biraz üstünde bir özdeğer, float gürültüsüyle tohumlanmış. Yükseklik
     > 12'de mesele yerleşmiş değil, MARJİNAL.

     > **Test seti bunu neden hiç görmedi:** `soak_resting_stacks_stay_bounded` 1-genişlikte
     > kuleleri, TEK zeminde, 1500 kare koşuyor. Yukarıdaki çöküşlerin biri hariç hepsi
     > 1979–2782 arasında — ufkunun ötesinde — ve ufkunun içinde kalanlar hiç kurmadığı bir
     > zemin boyutunda. `height_12_stacks_stay_standing` olarak `#[ignore]`'lu eklendi
     > (tıpkı `soak_extreme_tower_n48` gibi: bilinen, kayıtlı, açık kusur).

     > **Sweep işi için dürüst sonuç:** kararlılığı bu kadar marjinal bir sahne sınıfına karşı
     > sweep politikası ayarlanamaz, çünkü ölçülen her iyileşme gürültünün içinde kalır.
     > Önce yükseklik-12 kararlılık boşluğu kapanmalı.

  > **Kapıların eşikleri kutsanmadı.** Serbest zincir (sıfır yerçekiminde, iki ucundan içe
  > itilmiş, ankrajsız ama tamamı destek zinciri olan sıra) tam olarak bilinen iki yasa
  > taşıyor: net momentum 0 başlıyor ve dış kuvvet yok → 0 kalmalı; enerji kaynağı yok → KE
  > uçların aldığı 1.0 J'ü aşamaz; restitution 0 → durmalı. Ölçülen: `max|p|` 1 sweep'te
  > 3.2e-3, 16+ sweep'te ~0; `frames_to_rest` 8→16 sweep arasında **300× diz** (n=32: 178 → 4).
  > `sweep_throttling_is_visible_to_this_file` aletin KENDİ duyarlılığını sınıyor — 4 sweep'e
  > aç, hasar görünür olmak ZORUNDA; geçemezse okuma "kısma güvenli oldu" değil, "bu dosya
  > körleşti"dir. `slept_before_rest` de uyku sistemini solver sanmayı engelliyor (bir cisim
  > 15 karede uyuyor ve uyuyanın hızı sıfır okunur).

- ✅ **C2a — efektif sweep sayısı artık ölçülebiliyor** *(2026-08-05)*. `PhysicsMetrics`'e iki
  alan: `solver_sweeps` (ada ve substep boyunca toplanmış GERÇEK biased sweep sayısı) ve
  `max_island_depth`. `ConstraintSolver::solve_contacts` artık `SolveStats { island_depth,
  iterations }` döndürüyor. Hash kıpırdamadı (`EF6E4AC3644BF3BA`).

  > **Dürüstlük ayrıntısı:** split-impulse yolu adaptif sayıyı hiç tüketmiyor, `self.iterations`
  > süpürüyor. `SolveStats` o yolda `iterations` bildiriyor — `n_iterations` bildirmek
  > yapılmamış işi raporlamak olurdu.

  Bu, geçen oturumun `state_hash` merdiveniyle ÇIKARSADIĞI sayıları artık DOĞRUDAN ölçüyor
  (`audit_effective_sweeps_per_scene`, bir kare = 4 substep, ada başına ortalama):

  | sahne | depth | ada başına sweep |
  |---|---|---|
  | kule N=16 / N=24 / N=32 | 16 / 24 / 32 | 28 / 36 / 48 |
  | **yığın 4×6×4** | **6** | **28** |
  | **yığın 4×12×4** | **12** | **28** ← çöktüğü sayı; 96 istiyor |
  | bench salı N=64 / N=256 | 7 / 15 | 28 / 28 |
  | tam-temas kafes N=64 / N=256 | 8 / 16 | 28 / 28 |
  | serbest zincir n=32 (kare 0→4) | 3→7→11→15→**31** | 20→26→28→28→**46** |

  > Zincirin derinliği kademeli: yalnız iki uç temasta başlıyor, sıkışma dalgası ortaya
  > ilerledikçe ada birleşiyor ve 4. karede aynı yükseklikteki kuleyle birebir aynı sayıya
  > (46) varıyor. Sahne 4. kareden ÖNCE okunursa politikayı sınamıyor.

  **Ve bu yeni bir kapıyı mümkün kıldı:** `the_gated_scenes_reach_the_adaptive_policy`.
  Diğer bütün kapılar bir sahnenin özelliğini doğruluyor; hiçbiri o sahnenin test edilen KODA
  UĞRADIĞINI söyleyemiyor. Sweep'i zorlayan bir merdiven yalnız sweep sayısına göre
  anahtarlanmış bir kısmayı görür; uyku sayacına, ada boyutuna ya da `island_depth`'in kendi
  hesabına göre anahtarlanmış bir değişiklik sahneleri adaptif daldan sessizce düşürüp her
  iddiayı yeşil bırakabilirdi. Kapı `max_island_depth >= 5` ve ada başına sweep > yapılandırılan
  `iterations` istiyor. Düşerse bu dosyadaki hiçbir sonuç kanıt değildir.
- 🔄 **Yükseklik-12 kararlılık boşluğu — MEKANİZMA BULUNDU (tohum tarafı)** *(2026-08-06)*.
  Zemin-boyutu duyarlılığının (N1) sebebi float hassasiyeti DEĞİL; tek satırlık geometrik bir
  kusur. `gizmo-physics-core/src/narrowphase/contacts.rs`, `clip_box_box`:

  ```rust
  let signed_depth = ref_face_d - corner.dot(normal);
  if signed_depth <= 0.0 { return None; }   // <-- toleranssız
  ```

  **TAM TEMASTA dört köşenin de `signed_depth`'i tam 0.0**, dolayısıyla hepsi eleniyor,
  Sutherland–Hodgman boş dönüyor, ters-referans yedeği aynı sebeple boş dönüyor ve çift
  GJK/EPA yedeğine düşüyor — o da **TEK** temas döndürüyor. Ele veren asimetri: hemen altındaki
  yanal slab testinde bilinçli bir `SLAB_TOLERANCE = 1e-3` var ("floating-point edge-case
  rejections"ı önlemek için), derinlik testinde hiç yok.

  **Ve tek nokta merkezde değil.** Ofseti zeminin yarı-boyutuyla büyüyüp duran kutunun KENDİ
  kenarına yakınsıyor (`H/(2H+1)`): yarı-boyut 2'de 0.400, 3'te 0.4286, 5'te 0.4545, 20'de
  0.4878, 200'de 0.4988. Tek noktalı bir manifold — blok çözücü ne yaparsa yapsın — sıfır
  tilt-restoring tork taşır; kenara konmuş biri ise geometrinin gerektirmediği bir tork
  darbesi uygular.

  | zemin yarı-boyutu | doğum → kararlı nokta sayısı | doğum noktası |
  |---|---|---|
  | 0.60 … 1.50 | **1 → 1** (hiç toparlamıyor) | merkez |
  | 2.00 | 1 → 4 | −0.400 |
  | 20.00 | 1 → 4 | −0.4878 |
  | 200.00 | 1 → 4 | −0.4988 |

  > ⚠️ **N1'i AÇIKLAMIYOR — bu benim düzeltmem.** Önce "büyük zemin → daha büyük merkez-dışı
  > tork darbesi → daha erken çöküş" yazmıştım. Ofset büyüyor, ama DARBE büyümüyor. Birim küp
  > için tek normal temasın verdiği açısal hız `6r·Δv/(1+6r²)`, ve bu **`r = 1/√6 = 0.408`'de
  > MAKSİMUM** — ölçülen ofsetler tam oradan başlıyor, dışarı ittikçe değişmiyor.
  > Ölçülen `|Δω|`: yarı-boyut 2'de 0.028351, 20'de 0.030458, 200'de 0.030636, 1000'de 0.030651.
  > Bir yığının kaderini belirleyen iki zemin arasında **%0.58 tohum farkı**; deponun kendi
  > ölçtüğü büyüme hızıyla (lean ~100 karede ikiye katlanıyor, λ ≈ 0.0069/kare) bu
  > `ln(1.0058)/λ ≈ 0.8` karelik bir kayma öngörür. Gözlenen kayma en az 672 kare. Üç
  > büyüklük mertebesi sapma. (`does_a_bigger_ground_deliver_a_bigger_birth_kick`)
  >
  > **Kusurun kendisi duruyor ve kendi başına düzeltilmeye değer:** motordaki her arayüz sıfır
  > tilt sertliğiyle ve hak etmediği `Δω ≈ 0.03 rad/s`'lik bir darbeyle doğuyor — hem de doğru
  > cevabı "hiçbir şey kıpırdamaz" olan bir sahnede. **N1 hâlâ açıklanmamış.**

  > **Yarı-boyut ~1.5'in altında arayüz hiç toparlamıyor** — GJK noktası merkeze düşüyor,
  > kutuyu torksuz tutuyor, kutu hiç batmıyor, `signed_depth` hiç pozitif olmuyor ve kırpma
  > yoluna bir daha girilmiyor. **Küçük bir platformun üstündeki sandığın tilt sertliği sıfır.**

  > Bu dosyadaki ve `soak_and_golden.rs`'teki HER sahne kutularını tam temasta kuruyor, yani
  > hepsi bu yoldan doğuyor.

  Ölçümler: `what_does_a_manifold_look_like_when_it_is_born`,
  `how_many_points_does_a_settled_interface_carry` (yerleşmiş arayüz her zemin boyutunda 4 köşe
  noktası taşıyor — blok çözücünün varsayımı kararlı halde SAĞLAM, sorun yalnız doğumda),
  `does_a_bigger_ground_degrade_the_contact`.

  - ⬜ **Aday düzeltme, HENÜZ UYGULANMADI:** derinlik testine slab testindekiyle aynı türden bir
    tolerans (speculative margin) ver → tam temasta 4 nokta. Tek satır, ama narrowphase'in
    tamamını ve determinizm hash'ini etkiler. Bu oturumun kalite ölçütü tam olarak bunu
    ölçmek için var; ölçmeden uygulanmayacak (bkz. `ba9224b`'deki ders).
  - 🔄 **N2 (2 cm yanal boşlukta hızlı çöküş) — daraltıldı, sebep hâlâ açık.** Boşluk taraması
    (`where_is_the_fast_collapse_band`, zemin 20): 0.010/0.015/0.025/0.030/0.040/0.050/0.060
    hepsi 3000 kare duruyor, **yalnız 0.020 çöküyor** — tek hücrelik keskin bir sivri.

    > **`warm_start_match_tolerance` ÇÜRÜTÜLDÜ** (varsayılanı da 0.02 olduğu için baş şüpheliydi).
    > Toleransı 0.002 / 0.02 / 0.05 yapınca çöküş **aynı boşlukta ve aynı karede** kalıyor —
    > 25× aralık, sıfır etki. Rastlantıymış.
    > `max_linear_correction` da 0.02, ama koşmadan eleniyor: yalnız `solver/mod.rs:631` ve
    > `:787`'de, ikisi de varsayılanın girmediği split-impulse yolunda.

    > **"70. kare" ölçüm aracının kendi kusuruydu, düzeltildi.** `run()`, `max|v|`'nin 0.5'i ilk
    > geçtiği kareyi yazıyor; 70'te tek karelik bir hız sivrisi var, kule hâlâ ayakta ve gerçek
    > devrilme ~200-250. Yani "diğerlerinden iki kat büyüklük hızlı" değil, bir kat.

    > **Ama İMZASI gerçekten farklı, ve asıl kullanışlı olan bu.** Tilt ilk karelerden itibaren
    > düzenli tırmanıyor (95. karede 3.2°) ve `pen_max` 95. karede slop'un 3 katına (0.0163)
    > çıkıyor. Karşılaştır: tam temastaki 4×12×4 yavaş burkulmasında `pen_max` 1559 kare boyunca
    > 0.0055'te DÜZ kalıp sonra aniden deviriyor. Bu sahne baştan itibaren batıyor ve yatıyor —
    > gürültü-tohumlu üstel değil, sürekli bir yetmezlik. Yani **ayrı bir mekanizma**, ve
    > gözlemciler onu 30. kareden itibaren görüyor.

- 🔄 **N1 KAPANDI — mekanizma UYKU YOLU, zemin boyutu yalnız ne zaman uyunduğunu kaydırıyor**
  *(2026-08-06)*. Her cismi zorla uyanık tutunca her şey kayboluyor:

  | zemin yarı-boyutu | doğal uyku (çöküş / peak lean) | zorla uyanık |
  |---|---|---|
  | 20 | — / 0.010417 | — / **0.000106** |
  | 100 | — / 0.014219 | — / **0.000106** |
  | 140 | **2312** / 10.168 | — / **0.000106** |
  | 150 | **2875** / 4.218 | — / **0.000106** |
  | 200 | **2328** / 10.038 | — / **0.000106** |

  Aynı anda üç şey: lean ~100× küçülüyor, ÜÇ çöküşün üçü de kayboluyor, ve **zemin-boyutu
  bağımlılığı tamamen yok oluyor** (beş boyutta birebir 0.000106). Beş aday mekanizmanın
  arka arkaya düşmesinin sebebi buymuş: hepsi geometriye ve solver'a bakıyordu, etki ise
  ikisinde de değil.

  > **KOD DÜZEYİNDEKİ KUSUR (doğrulandı, hipotez değil).** `solver/tgs.rs:167-172` bir cismin
  > `inv_mass()` ve `inv_world_inertia_tensor()`'ını UYKU DURUMUNA BAKMADAN kullanıyor; tek
  > kapı `is_dynamic()`. Yani uyuyan dinamik bir cisim impuls alışverişine **sonlu** ters
  > kütlesiyle giriyor — solver onun hareket edeceğini varsayıyor — ama `integrator.rs` onu
  > hiç entegre etmiyor. Uyanık komşu tepkinin kendi payını alıyor, uyuyan payını almıyor:
  > o arayüzde momentum ve enerji korunmuyor. Kısmen uyuyan bir kolon tam da bunu yaşıyor
  > (trace'lerde `asleep` 0 ile 6 arasında titriyor).
  >
  > ⚠️ **Standart çare DENENDİ, ÖLÇÜLDÜ, GERİ ALINDI** *(2026-08-06)*. Uyuyan cismi çözümde
  > statik gibi ele almak (`solve_inv_mass` / `solve_inv_inertia` / `solve_movable`, beş çağrı
  > yeri). Yükseklik-12'de **iyi görünüyordu:** `height_12_stacks_stay_standing` 5/6 çöküşten
  > 2/6'ya indi, kalan ikisi ~2× geç çöktü, mevcut soak'ların HEPSİ yeşil kaldı (önceki
  > denemenin bozduğu `soak_demo_tower_awake_stays_upright` dahil), determinizm yeni hash'te
  > sağlamdı (`D436C9CF320FAF85`, 3/3).
  >
  > **Sonra yükseklik-6 topluluğu** (varsayılan konfigürasyon, 3000 kare, 9 hücre):
  >
  > | blok | zemin | düzeltmesiz | düzeltmeli |
  > |---|---|---|---|
  > | 2×6×2 | 20 | — | **1249** |
  > | 4×6×4 | 100 | — | **2724** |
  > | 4×6×4 | 200 | — | **1851** |
  > | (kalan 6 hücre) | | — | — |
  > | **toplam** | | **0/9** | **3/9** |
  >
  > Yükseklik-6, motorun şu an GÜVENİLİR taşıdığı sınıf ve ≤~12 zarfının rahatça içinde.
  > Çalışan bir sınıfı, yükseklik-12'deki kısmi bir iyileşme için takas etmek düzeltme değil.
  > Geri alındı; motorda değişiklik yok.
  >
  > **Neden geri tepiyor (hipotez, devralan için):** kolonun ortasındaki sonsuz kütleli bir
  > uyuyan, kütle dağılımında bir SÜREKSİZLİK; herhangi bir cisim uyur uymaz yığının dinamiği
  > birden değişiyor. İki ele alış da yanlış — sonlu kütle + entegrasyonsuz korunumsuz, sonsuz
  > kütle ise basamak değişimi. Bu da asıl koşulun **yığının KISMEN uyuyabilmesi** olduğunu
  > gösteriyor. Cisimler `integrator.rs`'te tek tek uykuya dalıyor; yalnız UYANMA ada-kolektif
  > (`pipeline.rs`). Ölçülen tek kusursuz konfigürasyon — her şeyin uyanık tutulması — tam
  > olarak kısmi uykunun var olmadığı durum.
  >
- ✅ **DÜZELTİLDİ — ADA-KOLEKTİF UYKU** *(2026-08-06)*. Bir cisim ancak temas adasının TAMAMI
  uygun olduğunda uyuyor. `RigidBody::update_sleep_state` ikiye ayrıldı: sayacı ilerleten ve
  uyanmayı anında yapan `advance_sleep_counter` (integrator bunu çağırıyor) + `sleep_eligible`.
  Uyutma kararı `pipeline.rs`'te, çözümden SONRA, ada başına veriliyor. Temassız cisim (yalnız
  eklemle bağlı olanlar dahil) eskisi gibi kendi sayacıyla uyuyor — o geçiş eklem pasından
  sonra, çünkü eklem uyandırması `wake_up()` ile sayacı sıfırlıyor.

  | ölçüm | öncesi | sonrası |
  |---|---|---|
  | `height_12_stacks_stay_standing` (6 hücre) | **5/6 çöküyor** | **GEÇİYOR** |
  | `wide_block_collapse_per_ground` (20 hücre) | **10/20 çöküyor** | **0/20** |
  | yükseklik-6 topluluğu (9 hücre) | 0/9 (lean 0.0018–0.0055) | 0/9 (lean **0.0005–0.0007**) |
  | 1×12×1 kolon, doğal uyku | lean 0.0104–10.17, 3 çöküş | **0.000106**, çöküş yok |
  | aynı kolon, zorla uyanık | 0.000106 | 0.000106 — **birebir eşleşiyor** |
  | 4×6×4 yığın, **1 sweep** | 193. karede patlıyor | duruyor (uyanıkken de) |

  > **Mekanizmanın kesin kanıtı:** düzeltmesiz, 1 sweep'te yığın doğal koşuda 193. karede
  > patlıyor (25/96 uyuyor) ama **zorla uyanık kolda hiç patlamıyor** (0.159 / 0.0021).
  > Yani 1-sweep patlaması hiçbir zaman az-çözmeden değil, KISMİ UYKUDAN kaynaklanıyormuş.
  > Düzeltmeden sonra iki kol da 0.159 / 0.0021.

  > **Determinizm re-bless (gerekçeli):** `EF6E4AC3644BF3BA` → **`46EB56180318E43C`**, 3/3
  > eşleşiyor. `golden_state.rs::golden_box_settling_on_the_ground`'da `settle vy`
  > `-0.040_873_3` → `0.0`; o sayı tam bir substep'lik yerçekimiydi (9.81/240) ve substep
  > oranının değil KUSURUN parmak iziymiş: cisim hız entegrasyonunun sonunda, yerçekimi
  > uygulanmış ama temas çözümü onu iptal etmemişken uykuya dalıp o çözülmemiş değeri
  > donduruyordu. `settle y` DEĞİŞMEDİ — dinlenme konumu, buradaki asıl yük taşıyan sayı,
  > kıpırdamadı.

  > **Yan kazanç, ölçüldü:** `headless_stress_test` 1.62 s → **0.51 s** (3.2×),
  > `wide_block_collapse_per_ground` 386 s → 43 s (9×), CI kapı seti debug'da 63.5 s → 15.2 s.
  > Sebep aynı: yerleşmiş yığınlar artık gerçekten topluca uyuyor. Önceden bir yığın hiçbir
  > zaman tam uyuyamıyordu.

  > **Bedeli:** bir adanın tek bir üyesi bile kıpırdıyorsa ada uyumuyor. Titreyen tek bir kutu
  > koca bir yığını uyanık tutabilir. Ölçülen sahnelerde bu olmadı (tam tersi oldu), ama
  > patolojik bir sahnede olabilir.

  > **Ölçüm aracının kendi negatif kontrolü de çürüdü ve bu öğretici:**
  > `negative_control_starved_pile_must_fail_the_gate` "sweep'i kıs, kapı düşsün" diyordu ve
  > düşüyordu. Artık 1 sweep'te bile düşmüyor — çünkü o kontrol sweep sayısını değil, KUSURU
  > ölçüyormuş (açlık cisimleri kısmi-uyku rejimine sokuyordu). Emekliye ayrıldı; sweep
  > duyarlılığını hâlâ serbest zincirdeki `sweep_throttling_is_visible_to_this_file` koruyor.

  > ⚠️ **Denenen ve ÇÜRÜYEN ilk düzeltme (uygulandı, ölçüldü, geri alındı).** "Uyanmayacak
  > uyuyan cisme yazma yapma" (`pipeline.rs` writeback'inde tek koşul). 1×12×1'de üç çöküşü
  > de kaldırdı — ama zorla-uyanık davranışını **yeniden üretmedi** (lean 0.0099 vs 0.000106),
  > `height_12_stacks_stay_standing`'de 6 hücrenin hâlâ 4'ü düşüyordu (öncesi 5), ve YEŞİL bir
  > testi bozdu: `soak_demo_tower_awake_stays_upright` 532. karede patladı. Geri alındı.
  > Sebebi de netleşti: yazmayı kesmek, uyuyan cismin çözüme sonlu kütleyle GİRMESİNİ
  > engellemiyor — asıl tutarsızlık orada.

  > ⚠️ **Ve birleştirme denemesi de ÇÜRÜDÜ.** "Uyku, çifti narrowphase'den düşürüyor, uyanınca
  > manifold yeniden doğuyor, doğum teması da tek noktalı ve merkez-dışı" hikâyesi çekiciydi ve
  > iki bulguyu birleştirecekti. Ölçüldü: dejenere (1 noktalı) olay sayısı doğal uykuda da
  > zorla uyanıkta da, iki zeminde de **birebir 13**. Uyku döngüsü dejenere manifold ÜRETMİYOR.
  > (`Started` olayları 18 vs 12, yani yeniden doğum var — ama dejenere değil.)

- 🔄 **N1 üzerine ÖNCE elenen beş aday** *(2026-08-06)*. Hiçbiri tutmadı;
  eleme sonucunda N1'in şekli netleşti ve yukarıdaki cevaba giden yol açıldı.

  | aday | ölçüm | sonuç |
  |---|---|---|
  | Doğum darbesinin büyüklüğü | `does_a_bigger_ground_deliver_a_bigger_birth_kick` | **ÇÜRÜK** — 20→200 arası %0.58 |
  | Teğetsel cırcır (birikme) | `is_the_lean_slip_or_rotation` | **ÇÜRÜK** — kayma salınıyor, path/net ≈ 21 |
  | Birikmiş kaymanın ayırt ediciliği | aynı | **ÇÜRÜK** — hayatta kalanlar DAHA ÇOK biriktiriyor (0.521 vs 0.462) |
  | Temas hassasiyeti (sürekli gürültü) | `does_a_settled_contact_jitter_more_on_a_bigger_ground` | **ÇÜRÜK** — yerleşmiş temas her boyutta bit-kararlı, 600 karede jitter 0 |

  > **Ve "mekanizma yok, sadece farklı bir örnek" (H-C) de çürük.** Zemini 20.000 → 20.001 →
  > 20.01 → 20.1 → 21.0 yapmak (40 metrelik statik bir kutuda bir milimetre) sonucu SAÇMIYOR:
  > hepsi ayakta, peak_lean'ler 0.004 içinde. Kaotik dekorelasyon olsaydı saçardı.

  > **Ama eşik de yok.** Daraltınca: 140 ve 150 çöküyor, 160/175/190 ayakta, 200 çöküyor.
  > (Ara okumamda "100 ile 200 arasında açılıyor" demiştim — daraltma bunu çürüttü, dağınık
  > bir başarısızlık bölgesi, anahtar değil.)

  > **Boyutun gerçekte yaptığı şey DİNLENME GENLİĞİNİ yükseltmek**, ve o kısmı düzenli:
  > peak_lean yarı-boyut 21'e kadar ~0.011, 50'den itibaren ~0.018–0.024 ve orada doyuyor.
  > Çöküşler yalnız üst bantta beliriyor, o banttaki HANGİ boyutun çökeceği ise kaotik.
  >
  > Yani N1 gerçek, boyuta bağlı bir GENLİK etkisi + üstüne binmiş kaotik bir sonuç.
  > **Genliği neyin yükselttiği hâlâ belirlenmedi.** Hassasiyet yalnız iki-kutulu yığında
  > çürütüldü; o yığın tam bir sabit noktaya oturduğu için zayıf bir sınav — çekilecek ip bu.

- ⬜ **BÜYÜME HIZI İÇİN ADAY (hipotezin cırcır hâli ÇÜRÜDÜ, kalanı duruyor): teğet kanalında
  konum terimi YOK.**
  `solver/tgs.rs`'te `Prepared` normal için `pen0` taşıyor ve üç sweep de onu bias'a sürüyor
  (`:580`, `:705`, `:820`) — depenetrasyonu sağlayan ve TGS'i çalıştıran şey bu. **Teğet için
  karşılığı yok:** üç sürtünme çözümü de saf `acc_t − rel·t/k_t` (`:618-627`, `:748-757`,
  `:862-871`), yani yalnız hız düzeyinde. Sürtünme teğetsel HIZA direniyor, teğetsel
  YER DEĞİŞTİRMEYE değil.

  > ⚠️ **"Cırcır gibi birikiyor" kısmı ÖLÇÜLDÜ ve YANLIŞ ÇIKTI.** `is_the_lean_slip_or_rotation`:
  > birikmiş kayma YOLU tekdüze büyüyor (2200 karede 0.09 → 0.46) ama NET kayma büyümüyor
  > (0.006–0.022 arası salınıyor) — oran 21:1. Yani arayüzler durmadan mikro-kayıyor ama
  > ileri-geri; tek yöne birikmiyorlar. Ayrıca kayma ile tilt sabit oranda kilitli, yani
  > "kayma mı dönme mi" diye sorduğum ayrıştırma yanlış soruymuş: ikisi aynı modun iki ölçümü.
  >
  > **Kalan ve hâlâ ayakta olan kısım:** temas hiç KİLİTLENMİYOR. Gerçek statik sürtünme
  > arayüzü kilitlerdi; burada sonsuza dek mikro-kayma sürüyor, ve konum-düzeyi terim yokluğu
  > bunu açıklıyor. Ama bu tek başına çöküşü ayırt etmiyor (yukarıdaki tabloya bak).

  > **Kayıtlı bütün çürütmelerle tutarlı olması bu adayı öne çıkarıyor:** sürtünmenin
  > KARARLILAŞTIRICI olması (hıza direniyor, o yüzden kaldırınca felaket), buna rağmen
  > instabilitenin sürmesi (konum terimi yok), ve warm-start'ı SIKMANIN durumu KÖTÜLEŞTİRMESİ
  > (birikmiş teğet impuls, teğet kanalının sahip olduğu tek hafıza).

  Doğrulama: lean büyümesinin birikmiş teğetsel kaymayla ilişkisini ölç; ya da teğete
  konum-düzeyi bir terim ekleyip büyüme hızının düşüp düşmediğine bak. İkincisi solver
  cerrahisi ve determinizm hash'ini oynatır.

- ✅ **Ölçüm aracı denetlendi — tablolar geçerli** *(2026-08-06)*. `pipeline.rs` yazma
  geri-dönüşü uyku bayrağına bakmıyor: aktif bir adadaki her dinamik üyenin hızını yazıyor,
  ama integrator uyuyanı atlıyor → uyuyan bir cisim hiç etki etmeyeceği ve hiç sönmeyeceği bir
  hız tutabiliyor. Bir patlama dedektörü bu donmuş sayıyı hareket sanabilirdi.
  `Frame::max_speed_awake` + `Run::blew_up_at_awake` eklendi ve tüm yükseklik-12 hücreleri
  yeniden koşuldu: **her hücrede iki sayı birebir aynı**. Yayımlanan tablolar etkilenmemiş.
  `trip_lean`/`trip_tilt_deg` de eklendi, böylece dedektörün tetiklendiği an bir devrilme mi
  yoksa bir seğirme mi olduğu okunabiliyor.
  - ⬜ **Ayrı, küçük kusur:** uyuyan cisme yazılan hız uyandığında uygulanıyor — bayat bir
    impuls. Tek `if` ile düzelir ama determinizm hash'ini oynatır; bu iş kapsamı dışında.

- ✅ **Bayat doküman düzeltildi:** `ConstraintSolver::support_ordering` yorumu "VARSAYILAN
  KAPALI" diyordu, `Default` ise `true` veriyor. Yığın kararlılığına bakan biri için önemli:
  sıralama zaten devrede, "açmayı denesek mi" sorusu kapalı.

- ⬜ **C2** — Broadphase refit (`pipeline.rs:145-176` her substep sıfırdan kuruyor, statikler dahil).
- ⬜ **C3** — `physics-rigid/src/system.rs:149-158` O(N²) writeback → handle→index map.
- ⬜ **C4** — Temas yolunda `ArrayVec` (`narrowphase/mod.rs:400-407`); rewind geçmişi opt-in
  (`world/step.rs:122-128` her frame tam klon).
- ⬜ **C5** — `[profile.release]` (`lto="thin"`, `codegen-units=1`). Kökte yok; `.cargo/config.toml`
  bu makinede `lto=off` zorluyor → tüm perf sayıları alt sınır.
- ⬜ **C6** — Index buffer (`components/mesh.rs:8-9`) + mipmap + anizotropik filtreleme.

## Faz D — Ekosistem ve 1.0
- 🔄 **D1** — `gizmo-core`'u fizik crate'lerinde opsiyonel yap (`ecs` feature'ı).
  **`gizmo-physics-core` bitti (2026-08-05):** `default = ["ecs"]`, kapatınca `gizmo-core`
  graftan tamamen düşüyor (kalan: `gizmo-math`, `arrayvec`, `serde`, `tracing`). Kendi
  description/keywords/categories'i de eklendi. **`gizmo-physics-rigid` bitti:** aynı desen —
  `system.rs` (25 referans, ECS köprüsü) ve 5 `impl_component!` çağrısı `ecs`'in arkasında;
  `PhysicsWorld` zaten ECS'siz olduğu için feature kapalıyken crate `step` çağrılarak
  sürülen bağımsız bir rigid-body simülatörü olarak kalıyor. **`-soft` ve `-dynamics` de
  bitti — D1'in fizik tarafı TAMAM:** dördü de `ecs` kapalıyken `gizmo-core`'u graftan
  tamamen düşürüyor.

  > **Tuzak, kaydedilmeye değer:** `-rigid`'de `ecs`'i kapatmak `gizmo-core`'u tek başına
  > düşürmedi — bağımlılık `gizmo-physics-core` üzerinden geliyordu, çünkü onun kendi
  > `default = ["ecs"]`'i devredeydi. Zincirdeki her fizik bağımlılığını
  > `default-features = false` ile bildirmek gerekti.

  > **Plandaki ölçüm yanlıştı — ama benim düzeltmem de yanlıştı, ve ikincisi daha öğretici.**
  > Plan "60 dosyadan 5'i" diyordu; ben DOSYA SAYISI ile ölçüp `-dynamics` 5/7, `-soft` 5/8
  > bulunca "bu paketleme değil, ayrıştırma işi" diye düzelttim. Dosya sayısı yanlış metrikmiş:
  > `soft_body.rs` 801 satır ama TEK referans, `cloth.rs` 1140 satır TEK referans,
  > `vehicle/mod.rs` 1268 satır TEK referans — hepsi birer `Component` impl'i. Referans
  > YOĞUNLUĞUYLA ölçünce planın sonucu benim düzeltmemden daha doğruydu: dördü de paketleme
  > işiydi. Tek gerçek ayrıştırma `-dynamics`'te `oxygen`/`ragdoll` modüllerini `ecs` arkasına
  > almaktı (ikisi de `World` alıyor, ragdoll entity spawn ediyor).
  >
  > `-core`'da bağımlılık iki şeye indi: 11 `impl_component!` çağrısı (önemsiz, `#[cfg]`
  > yeterli) ve `FighterController`. İkincisi gerçek bir engeldi: bir `FighterInputBuffer`
  > tutuyor ve o tipin `update`'i `&Input` + `&ActionMap` alıyor, yani girdi alt-sistemine
  > bağlı. Taşımak tüm input sistemini taşımak demekti (ve workspace'te tek tüketicisi bu),
  > o yüzden `components::fighter` modülü `ecs`'in arkasına alındı. `ecs` kapalıyken
  > `FighterController` yok — kayıtlı ve kabul edilmiş bir ödün.
- ⬜ **D2** — `ENGINE.md`'yi İngilizce'ye çevir + `///` yorumlarında İngilizce kuralı +
  `CONTRIBUTING.md`. Bus factor = 1'in tek sebebi bu.
- ⬜ **D3** — Click-to-try WASM demosu (GitHub Pages).
- ✅ **D4 — `#![warn(missing_docs)]` Stage A'da.** **1. parti bitti (2026-08-05):**
  `gizmo-math`, `gizmo-net`, `gizmo-scene`, `gizmo-animation` sıfır eksik dokümanla ratchet
  altında. **2. parti bitti (2026-08-05):** `gizmo-physics-soft`, `gizmo-physics-dynamics` ve
  `gizmo-ai` de ratchet altında. **3. parti bitti (2026-08-05):** `gizmo-core` de temiz.
  **4. parti bitti (2026-08-05) — STAGE A TAMAM.** `gizmo-physics-rigid` ve
  `gizmo-physics-core` da ratchet altında; Stage A'nın **11 crate'inin tamamı** sıfır eksik
  dokümanda ve `-D warnings` kapısıyla korunuyor. Başlangıç: 1360 öğe.

  > **Dört partinin hata oranı: 0.21 → 0.11 → 0.21 → 0.103.** Üçüncüdeki sıçrama
  > `gizmo-core`'un yoğunluğundan değil, hataların ŞEKİL DEĞİŞTİRMESİNDEN geldi (uydurma
  > modüller-arası iddia yerine aşırı kesinlik). Dördüncü parti iki kuralı da birlikte
  > uyguladı — "kendi sözleşmesini belgele" + "zayıflat, tahmin etme" — ve en yoğun iki fizik
  > crate'inde en düşük oranı verdi. Doğrulanamayan modüller-arası iddia beş alanın toplamında
  > **1**'e indi (1. partide kategori olarak 18'di).
  >
  > Şemaya eklenen `weakened` alanı da kendini kanıtladı: ajanlar **114 kez** bilinçli olarak
  > zayıf-ama-doğru iddiayı seçtiklerini bildirdi. Bu, ölçülemeyen bir davranışı ölçülebilir
  > hale getiriyor.
  >
  > **Stage B (D4 kapsamı DIŞI, kayıt için):** `gizmo-renderer` (895), `gizmo-editor` (249),
  > `gizmo` (153), `gizmo-scripting` (99), `gizmo-studio` (34), `gizmo-analysis` (33),
  > `gizmo-app` (24), `gizmo-ui` (5). Bunlar 0.y'de kalıyor, ratchet zorunlu değil.

  > **3. partide hata oranı yeniden yükseldi (0.11 → 0.21) ve sebebi öğretici.** Doğrulanamayan
  > modüller-arası iddia 18'den 6'ya düştü — kural o eksende tuttu. Ama hatalar **AŞIRI
  > KESİNLİĞE** kaydı: kodun daha zayıf söz verdiği yerde doküman spesifik garanti yazıyordu
  > ("arketipleri artan indekste gezer", "yalnızca erişimleri çakışırsa ayrılırlar", "bir
  > indeks yalnızca bir sonraki silmeye kadar anlamlıdır"). Üçü de ölçülüp çürütüldü — biri
  > elle Kahn sıralaması izlenerek, biri `swap_rows`'un üretimde satır permüte ettiği
  > gösterilerek.
  >
  > 3. turun talimatı bu yüzden "daha kesin yaz" değil **"zayıflat ya da sil"** oldu:
  > *"iteration order is an implementation detail"* BİTMİŞ ve DOĞRU bir dokümandır;
  > *"visits archetypes in ascending index"* motorun sonsuza kadar tutmak zorunda kalacağı bir
  > sözdür. Denetime de simetrik kural konuldu (sırayı "implementation detail" demek bulgu
  > DEĞİLDİR), yoksa denetçi doğru zayıflatmayı hata sanıyordu. 60 → 26 → 11 → elle bitti.

  > **"Kendi sözleşmesini belgele" kuralı ölçülebilir şekilde işe yaradı.** Hata oranı öğe
  > başına **0.21 → 0.11**'e düştü (1. parti: 177 öğede 38 yanlışlık; 2. parti: 297 öğede 33).
  > Yakınsama 3 turdan **2 tura** indi. Şemaya eklediğim hesap-verme alanları da amacına
  > ulaştı: ajanlar 79 modüller-arası iddia yaptıklarını ve **47'sinden vazgeçtiklerini**
  > bildirdi — 1. partideki 38 hatanın çoğunu üreten davranış artık görünür ve kendini
  > sınırlıyor. Onarımdan sonra `physics-soft`'ta doğrulanamayan modüller-arası iddia sıfır.
  >
  > Onarım turunun üç kategoriyi FARKLI ele alması belirleyiciydi: doğrulanamayan iddiayı
  > yeniden doğrulamadan SİL, yanlış iddiayı doğrula-sonra-silmeyi-tercih-et, dolguyu ise
  > SİLME (lint doküman istiyor) — yerine yük taşıyan bir şey yaz.

  > **Fan-out doküman yazmanın başarısızlık modu dolgu yazı DEĞİL.** Anti-filler talimatı
  > tuttu: üç tur boyunca dört crate'te sıfır dolgu bulundu. Bulunan şey **kendinden emin ve
  > yanlış** dokümanlardı — 38 → 16 → 9, her turda düşerek. Örnekler: "`cos(0)` ONE'a eşit
  > değil" (eşit, 65536 == 65536), "hiçbir zaman JSON değildi" (f6ab53b'de serde_json),
  > "wasm'da UdpSocket yok" (derleniyor), "Lua API ve editör kullanıyor" (ikisi de yok).
  >
  > Ortak paydası: iddiaların neredeyse tamamı **modüller arası** — "renderer şunu normalize
  > eder", "X çağırır", "SAP broadphase'le paylaşılır". Tek dosyaya bakan bir ajanın
  > doğrulayamayacağı, dolayısıyla uyduracağı tür. 3. turun talimatı bu yüzden
  > "belirsizse SİL, yeniden yazma" oldu ve yakınsamayı hızlandırdı.
  >
  > **Sonraki partiler için kural:** öğenin KENDİ sözleşmesini belgele (birim, aralık, yerel
  > invaryant, fonksiyonun kendi davranışı). Uzak tüketici hakkında iddia, ancak elle
  > doğrulanmışsa yazılsın. Bir de: kendi yazdığım `evaluate_clip` dokümanını denetim ajanı
  > düzeltti (`Hips` kontrolü çözümlenmiş eklemin adında değil, `track.target_node_name`
  > üzerinde) — bu hata modundan kimse muaf değil.

- ⬜ **D4-followup — `Schedule` build sonrası eklenen sistemde ÖNCEKİLERİ DÜŞÜRÜYOR.**
  `gizmo-core` doküman turunda bulundu, probe ile doğrulandı (ilk sistemin sayacı ikinci
  koşudan sonra 2 değil 1'de kalıyor).

  **Mekanizma:** `Schedule::build` config'leri `std::mem::take` ile batch'lere TAŞIYOR;
  `invalidate()` ise yalnızca `phase_batches`/`legacy_batches`'i `clear()` ediyor. Geri
  kurulacak config kalmadığı için build sonrası her `add_system` / `add_di_system` /
  `configure_set` çağrısı, önceden derlenmiş TÜM sistemleri kalıcı olarak atıyor.
  `run()` ilk frame'de tembel build ettiğinden ısıran kalıp sıradan: sistemleri ekle, bir
  frame koş, sonra bir tane daha kaydet (çalışma zamanı plugin'i, editör, script) → schedule'da
  yalnızca sonuncusu kalır. `configure_set` build sonrası çağrılırsa schedule TAMAMEN boşalır.

  **Şimdilik yapılan (düzeltme DEĞİL):** `invalidate()` artık kaç sistemi attığını
  `tracing::error!` ile bildiriyor — sessiz kayıp en azından görünür. Ve
  `modify_after_build::adding_a_system_after_the_first_run_drops_the_earlier_ones` mevcut
  (hatalı) davranışı pinliyor; testin yorumu açıkça diyor ki bu `2` ile kırmızıya döndüğünde
  düzeltme inmiş demektir, beklenti güncellenmeli, test SİLİNMEMELİ.

  **Gerçek düzeltme** `Schedule`'ın build sonrası sahiplik modelini değiştirmeyi gerektiriyor:
  ya config'ler build'den sonra da saklanmalı (sistemler batch'lere ödünç verilmeli ya da
  paylaşılmalı), ya da `invalidate()` batch'lerden sistemleri geri çıkarıp config'e
  dönüştürmeli — ikincisinde label/ordering meta-verisi build sırasında tüketildiği için
  kaybolur. Kendi oturumunu hak eden bir refactor.

- ✅ **D4-followup — `Track::sample` tek keyframe + NaN'de PANİKLİYORDU** *(düzeltildi 2026-08-05)*. Doküman turunda
  bulundu, gerçek koda karşı doğrulandı (probe: `single_kf_nan_time panicked=true`,
  `single_nan_timestamp panicked=true`, iki-keyframe kontrolü `false`). `clip.rs:218`
  `idx.clamp(1, len - 1)` yapıyor; `len == 1` ve NaN her iki erken-dönüşü de atlattığında
  bu `0.clamp(1, 0)` oluyor ve `Ord::clamp` `min > max` diye panikliyor. İronik olarak
  `clip.rs:213-215`'teki yorum bu clamp'in NaN koruması olduğunu söylüyor — panikleyen şey o.
  Düzeltme: `len < 2` erken-dönüşü (tek keyframe'in cevabı zaten o keyframe) + 3 regresyon
  testi. Fix geri alınınca `a_single_keyframe_track_does_not_panic_on_nan` gerçekten
  `min > max. min = 1, max = 0` ile kırmızıya dönüyor.
- ✅ **D5 — `glam` 0.29 → 0.32 + `bevy_reflect` 0.15 → 0.19** *(2026-08-04)*. Grafta artık
  TEK glam var (0.32.1). Fizik kıpırdamadı: hash `EF6E4AC3644BF3BA`, `golden_state.rs`'in
  hiçbir değeri yeniden kutsanmadı — bir matematik kütüphanesinin major bump'ı sessiz sayısal
  kaymanın saklanacağı yerdir, golden fixture tam bu soruyu cevaplamak için vardı.

  > **Plandaki teşhis kısmen yanlıştı.** "bevy_reflect'in bağımlılığı tutuyor" doğru ama
  > eksik: 0.29'u tutan tek şey `gizmo-math`'in KENDİ manifest'iydi; `bevy_reflect` yalnızca
  > `reflect` feature'ı açıkken kırıyordu (0.15 ve 0.16'nın ikisi de glam 0.29'da; 0.32'ye
  > geçen ilk sürüm 0.19 — dört minor'lük sıçramanın sebebi bu).
  >
  > Bench dev-dep'leri (`bevy_math`/`bevy_picking`/`bevy_mesh`) de 0.19'a çekildi; API
  > göçleri: `new_bezier`→`new_bezier_easing`, `VectorSpace::Scalar`, `Affine3A` + `uvs`,
  > `clone_dynamic`→`to_dynamic_{map,list,struct}` ve `Map`/`List`/`Struct`'ın kökten
  > alt-modüle taşınması.

  > **Yan bulgu — bakılması gereken:** `gizmo-core/benches/` altındaki `map_bench`,
  > `reflect_bench`, `struct_bench`, `path_bench` dosyalarında **sıfır `gizmo_core`
  > referansı** var; bunlar bevy_reflect'in kendi benchmark'ları, yani bir BAĞIMLILIĞI
  > ölçüyorlar. Her bevy yükseltmesinde kırılıyorlar ve motorun performansı hakkında hiçbir
  > şey söylemiyorlar. `ecs_bench/` (20 dosya) ise gerçekten `gizmo_core`'u ölçüyor.
  > Silme kararı senin — bu commit yalnızca göç ettirdi.
- ⬜ **D6** — İki yönlü soft↔rigid coupling (`soft_body.rs:74-120` impulsu hesaplayıp atıyor).
- ⬜ **D7** — `gizmo-ui` metin render'ı, ya da crate'i dürüstçe "deneysel" işaretle
  (şu an hiçbir şey çizmiyor: `gizmo-ui/src/lib.rs:39-52`).

## Faz E — 1.0
Kademeli 1.0 planı (ENGINE.md §4) sağlam. **Stage A'ya girmeden önce bitmiş olmalı:**
A1–A9, B1 (App API kırıcısı), D1, D4, D5.
Ayrıca `gizmo-scene`'in Stage A'da olması planın **kendi kuralıyla çelişiyor** — public error
enum'unda `ron 0.8` tipleri var (ENGINE.md §4'ün Stage B kriteri tam da bu).

---

---

## Doküman turlarında bulunan kusurlar — önceliklendirilmiş backlog

D4 boyunca (4 parti, ~1360 öğe) ajanlar kodu satır satır okurken **doküman dışı** kusurlar da
buldu. Aşağıdakiler commit mesajlarıyla workflow çıktılarında dağınık kalmasın diye buraya
toplandı.

> **Doğrulama durumu:** bu maddelerin çoğu AJAN RAPORU, benim elle doğruladıklarım değil.
> İşaretliler: ✅ elle doğrulandı (probe/test ile), ⚠️ yalnız ajan raporu. CLAUDE.md'nin kuralı
> burada da geçerli — düzeltmeden önce her birini elle doğrula, bu depoda yanlış pozitif
> geçmişi var.

### A — Sessiz yanlış davranış (en yüksek değer)

- ✅ **`Schedule` build sonrası eklemede öncekileri düşürüyor** — ayrı madde olarak yukarıda.
- ⚠️ **`shatter_entity` Box olmayan collider'da kalıcı ölü bırakıyor.** `system.rs:365-381`
  erken dönüyor ama çağıranlar (`:319-331`, `:504-519`) `breakable.is_broken = true`'yu ÇOKTAN
  yazmış. Sphere/capsule/hull bir breakable sıfır cana iner, debris üretmez, despawn olmaz, ve
  sonraki her kontrol `!is_broken && …` olduğu için bir daha hasar da alamaz.
- ⚠️ **`FrameProfiler::avg_frame_ms` ring wraparound sonrası yanlış kareleri ortalıyor.**
  `profiler.rs:168-204` `history.iter().rev().take(count)` yapıyor ama `history` bir ring
  buffer; 300 kareden sonra fiziksel sıra kronolojik değil, dolayısıyla en yeni ve ~300 kare
  eski veriyi karıştırıyor. `estimated_fps()` de miras alıyor. `last_frame()` doğru (modüler
  aritmetiği açıkça yapıyor).
- ⚠️ **`register_serializable` sessizce hiçbir şey yapmıyor olabilir.** `register::<T>("T")`
  sonrası `register_serializable::<T>("T")` çağrısı, "aynı tip+ad zaten kayıtlı" kısa devresine
  takılıp `Ok(())` dönüyor ve dört fonksiyon işaretçisi HİÇ kurulmuyor — tip sessizce
  serileştirilemez kalıyor. `register_reflect` aynı durumu doğru ele alıyor (yerinde upgrade).
- ⚠️ **`PoolManager::destroy` aynı entity'yi iki kez park edebiliyor** (`pool.rs:117-124`,
  üyelik/canlılık kontrolü yok) → `instantiate` aynı entity'yi iki çağırana veriyor.
- ⚠️ **`UtilityAction::evaluate` boş `considerations`'da `base_score`'u KIRPMADAN döndürüyor**
  (`utility_ai.rs:216`). `base_score = 5.0` her düzgün skorlanmış eylemi yeniyor.
- ⚠️ **`Input::release_all` `keys_just_pressed`'i temizlemiyor** (`input/mod.rs:115-124`) — o
  kare boyunca bir tuş hem "just_pressed" hem "just_released" olabiliyor.
- ⚠️ **`add_bundle`/`spawn_batch` hızlı yolları hook ATEŞLEMİYOR** (`component_ops.rs:29-105`,
  `entity_lifecycle.rs:189-225`). `World::add_observer` ile kaydedilen gözlemciler bundle ile
  spawn edilen entity'leri kaçırıyor. `remove_bundle` de yalnız SparseSet bileşenleri için
  `on_remove` ateşliyor (`:107-190`), Table olanlar arketip göçüyle sessizce kopuyor.
- ⚠️ **Patlama uyuyan cismi uyandırmıyor** — `physics_explosion_system` yalnız `is_dynamic()`
  kapısı koyuyor, `wake_up()` çağırmıyor; impuls `Velocity`'ye yazılıp entegrasyonda atılıyor.
- ⚠️ **Soft-body GPU/CPU `damping` ~100 kat ayrışıyor.** Shader `v *= max(1-damping*dt, 0)`
  (oran), CPU `v *= damping.powf(dt)` (tutma katsayısı). Varsayılan 0.99 ve dt=1/60'ta
  0.9835'e karşı 0.99983. Aynı mesh GPU yolunda ~100 kat sert sönümleniyor.
- ⚠️ **GPU yolunda çarpışmada çift ilerletme** (`gpu_compute.rs:570`): sweep, shader'ın ZATEN
  entegre ettiği pozisyondan başlıyor; CPU yolu (`soft_body.rs:313`) entegrasyon ÖNCESİNDEN.

### B — Panik / çökme

- ⚠️ **Sıfır eksenli Hinge/Slider NaN üretiyor.** İkisi de `Default` türetiyor, `axis` ZERO
  kalıyor, `slider.rs:27` çıplak `.normalize()` çağırıyor. Kardeşi `solve_slider_spring`
  (`:246`) `normalize_or_zero` kullanıp bailing yapıyor. Yalnız `Joint::hinge`/`Joint::slider`
  yapıcıları `Vec3::Y` koyuyor; `..Default::default()` bunu atlıyor.
- ✅ **Negatif `max_correction_speed` çözücüyü paniklatıyor** — `bias.clamp(-x, x)` ile
  min > max. Alanlar public ve doğrulanmıyor.
- ⚠️ **`add_bundle` flush edilmemiş rezerve entity'de OOB panik** (`component_ops.rs:34-41`).
- ⚠️ **`solve_joints` `dt == 0`'da her kopabilir eklemi koparıyor** (`impulse/dt` → `inf`).

### C — 1.0'a dondurulacak ÖLÜ public alanlar

- ⚠️ `GravityField::falloff_radius`, `FluidZone::viscosity` — hiçbir yer okumuyor, yorumları
  var olmayan davranışı anlatıyor.
- ⚠️ `Explosion::damage_radius` — okunmuyor; hasar `force_radius`'a bakıyor.
- ⚠️ `Breakable::{max_health, debris_lifetime, break_sound, piece_prefab}` — hiçbiri okunmuyor.
- ⚠️ `AeroPackage::ground_effect_height` — formülde tam olarak sadeleşiyor (≥1 mm için).
- ⚠️ `SpatialHash` bir spatial hash DEĞİL (`DynamicAabbTree` sarmalıyor) ve
  `PhysicsWorld::with_cell_size` argümanını atıyor.
- ⚠️ `multibody::{base_position, base_rotation}` okunmuyor; `gravity` base koordinatında
  yorumlanıp `base_rotation` ile hiç döndürülmüyor.

### D — Model doğruluğu

- ⚠️ **`cone_limit_angle` twist'i çifte sayıyor** (`ball_socket.rs:61`): tam sapma
  quaternion'ının açısını alıyor, swing-twist ayrıştırmasının swing bileşenini değil. Koni ve
  twist limitleri birlikte açıkken twist, koni bütçesini yiyor.
- ⚠️ **Swing limitleri sistematik olarak gevşek**: radyan sınır `2·sin(θ/2)` ile
  karşılaştırılıyor, yazılan 90° limit ~103°'ye kadar devreye girmiyor. `d6.rs:72-75` aynı.
- ⚠️ **Adaptif iterasyon sayısı SI yolunda ölü** (`solver/mod.rs:370-375` yalnız
  `solve_contacts_tgs`'e geçiriyor) — CCD içeren island'lar tall-stack stabilizasyonunu
  kaybediyor.
- ⚠️ **ABA'da `is_fixed_base == false` yerçekimini sessizce düşürüyor** (`aba.rs:132`).
- ⚠️ **Navmesh `agent_radius` gerçek clearance vermiyor** — yürünebilir alan hiç aşındırılmıyor.

### E — Performans

- ⚠️ **GOAP `PlanNode::clone` tüm ata zincirini DERİN kopyalıyor** (her atanın `GoapState`
  HashMap'i dahil) — `build_plan`'de her ardıl için.

### F — Gürültü

- ⚠️ **`resolve_node_collision` her dinlenen düğüm için her adımda `warn!` basıyor**
  (`soft_body.rs:133`): `Ray::new` `dist > 1e-5` kapısından ÖNCE kuruluyor ve sıfır yön
  uyarısı veriyor. `Ray::new`'i kapının içine almak davranışı değiştirmeden düzeltir.

## Kapsam dışı / bilinçli olarak yapılmayacaklar
- `gizmo-audio`'nun cfg-gate'li `unsafe impl Send/Sync`'i — doğru ve gerekçeli, dokunulmayacak.
- ENGINE.md §7'deki çürütülmüş false-positive'ler ve non-goal'lar (narrowphase batch-SIMD,
  cross-platform bit-determinizm, N≥48 kule) — yeniden kovalanmayacak.
- Denetimde **düşmanca doğrulamayı geçemeyen 9 iddia** rapordan çıkarıldı, burada da yok.
