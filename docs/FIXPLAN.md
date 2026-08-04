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

## Kapsam dışı / bilinçli olarak yapılmayacaklar
- `gizmo-audio`'nun cfg-gate'li `unsafe impl Send/Sync`'i — doğru ve gerekçeli, dokunulmayacak.
- ENGINE.md §7'deki çürütülmüş false-positive'ler ve non-goal'lar (narrowphase batch-SIMD,
  cross-platform bit-determinizm, N≥48 kule) — yeniden kovalanmayacak.
- Denetimde **düşmanca doğrulamayı geçemeyen 9 iddia** rapordan çıkarıldı, burada da yok.
