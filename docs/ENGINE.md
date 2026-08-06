# Gizmo Engine — Mühendislik Belgesi

> Motorun **tek** iç referans belgesi: mimari, canlı yol haritası, sürüm stratejisi,
> determinizm/migration sözleşmeleri, kapatılmış araştırmalar ve çalışma yöntemi.
> Kullanıcıya dönük giriş `README.md`'de, sürüm geçmişi `CHANGELOG.md`'de.
>
> Bu belge, 2026-07 sadeleştirmesinde 12 ayrı plan/FIXPLAN/referans dosyasını birleştirir; bitmiş
> işlerin ayrıntılı anlatısı budandı, kalıcı kararlar/dersler korundu.

---

## 1. Genel Bakış

Gizmo — hafif, **saf-Rust**, ECS tabanlı 3B motor + sıfırdan yazılmış fizik simülatörü
(harici fizik bağımlılığı yok). crates.io'da yayında (**0.9.0**, 19 crate).

- **ECS:** Entity = id, Component = veri, System'ler Archetype'ları sorgular. `World`
  merkezi durum; `Query`/`Mut`/`With`/`Without`/`Changed`/`Added` filtreleri; `Commands`
  ertelenmiş yapı; Table + SparseSet depolama.
- **Fizik:** rigid (TGS-Soft çözücü), soft-body (FEM/cloth/rope), fracture, joints,
  vehicle/character dinamiği, CCD, GJK/EPA narrowphase, BVH broadphase.
- **Renderer:** WGPU deferred PBR + gölge/SSAO/SSGI/volumetrik/TAA; egui HUD/editör.
- **Platform:** native + WASM (sim çekirdeği + renderer + window tarayıcıda çalışır).

---

## 2. Mimari (20 crate — kararlı)

Alttan-üste temiz katmanlama, döngüsel bağımlılık YOK:

```
gizmo-math ─┬─ gizmo-core ─┬─ gizmo-physics-{core,rigid,dynamics,soft}
            │              ├─ gizmo-renderer ─ gizmo-{window,ui,editor}
            │              ├─ gizmo-{scene,net,ai,animation,audio,scripting}
            └──────────────┴─ gizmo-app ─ gizmo (facade) ─ demo/
```

**Refactor sözleşmesi (tanrı-dosya bölme turlarından, tamamlandı):** yalnız saf/birebir
taşımalar (aynı adımda mantık düzenleme yok), orijinal yoldan `pub use` re-export
(çağrı yerleri değişmez), her adım derle+test+clippy ile doğrulanır. 10 mega-dosya
bölündü; determinizm hash'i değişmedi. AÇIK (opsiyonel, davranış-bitişik, saf-taşıma
değil): `update_vehicle` / `execute_render_pipeline` gibi hâlâ-büyük fonksiyonların
bölünmesi — gerektiğinde.

---

## 3. Yol Haritası (CANLI — yalnız kalan iş)

> **2026-08-04:** Bağımsız bir tam-motor denetimi yapıldı → `docs/AUDIT-2026-08.md`
> (her bulgu `dosya:satır` kanıtlı; düşmanca doğrulamayı geçemeyen 9 iddia çıkarıldı).
> Çıkan işler `docs/FIXPLAN.md`'de yürütülüyor — o kampanya bitince kalıcı kararlar buraya
> taşınacak ve FIXPLAN silinecek. Aşağıdaki Faz 6/7 maddeleri geçerliliğini koruyor;
> denetim bunların **önüne** giren bir Faz A (dürüstlük + güvenlik) ekledi.

Faz 0–5 (stabilizasyon, test+CI, determinizm, P2P rollback netcode, fizik-derinliği,
renderer/WASM/editör) **TAMAM**. Kalan:

**Faz 6 — API kararlılığı & 1.0 mekaniği**
- Public API dondurma + `unsafe` sözleşmelerini belgeleme.
- Kademeli 1.0 (bkz. §4): Stage A çekirdek 1.x, Stage B grafik katmanı 0.y.
- `RigidBody::friction`/`restitution` alanları temas çözücü tarafından **YOK SAYILIYOR**
  (kaynak = collider material) → köprüle mi kaldır mı? (API kararı; köprüleme
  varsayılanları kaydırır.)

**Faz 7 — Ürün katmanı (şipilebilir oyun)**
- M7.4 otoriter client-server netcode; M7.5 audio mixer/bus/DSP; M7.6 UI font/text/
  widget/z-index; M7.7 WASM özellik pariteleri, editör panelleri + AssetServer hot-reload.
- 1.0 CI kapıları: rustfmt / `missing_docs` / coverage / cargo-deny / benchmark regresyonu.
- Opsiyonel: cross-platform determinizm (bir feature olarak — bkz. §5), gizmo-net WASM.
- İnsan-gözü A/B gated: textured-glTF `material_demo` asset'i, `car_demo` sürüş/geometri.

---

## 4. Sürüm Stratejisi — Kademeli 1.0

1.0 = "2.0 olmadan kırıcı değişiklik yok" sert sözü. Public API'sinde 0.x bir bağımlılığı
(wgpu/winit/egui, bevy_reflect) re-export eden bir crate bu sözü veremez → lock-step 1.0
ya motoru eski deps'e dondurur ya da ilk dep bump'ında 1.0'ı yakar. Çözüm **kademeli**:

- **Stage A (1.x olabilir):** bağımlılık-hafif, sahip-olunan-yüzey crate'ler —
  gizmo-math, -core, -physics-{core,rigid,dynamics,soft}, -scene, -net, -audio, -ai,
  -animation.
- **Stage B (0.y kalır):** grafik/entegrasyon — gizmo-renderer, -window, -editor, -ui,
  -app, -scripting + `gizmo` facade (wgpu/winit/egui 1.0'a pinlenene dek).
- **Sonuç:** staging başlayınca crate'ler artık TEK workspace sürümünü paylaşmaz
  (`publish_all.sh` + sürüm-miras varsayımı güncellenmeli).

**Harici-tip sözleşmesi (kalıcı):** `glam` = kalıcı, KASITLI public dep (gizmo-math
re-export eder). `bevy_reflect` = varsayılan-KAPALI `reflect` feature'ının arkasında
mühürlü (serde fallback'li). `wgpu`/`winit`/`egui` = 0.x süresince semver bedeli olmayan
kasıtlı sızıntı. `ron` = gizmo-scene'in public dep'i (RON dosya formatı + SceneError API).
96 public tip `#[non_exhaustive]`; 13 Error enum + fn→Result dönüşümleri; `arrayvec`
public API'den çıkarıldı (opak `ContactPoints`).

---

## 5. Determinizm (referans)

- Simülasyon durumu (Transform/Velocity/solver) tamamen **glam/f32** üzerinde çalışır.
- **Hedef:** aynı-platform tekrar-oynatma (replay) + rollback bit-eş. `state_hash` +
  süreçler-arası test ile doğrulandı.
- **Kapsam DIŞI:** cross-platform bit-eş determinizm — Fp32/softfloat migrasyonu
  gerektirir (Q16.16 `Fp32` tipi gizmo-math'te VAR ama sim kullanmıyor, deneysel). 1.0
  sonrası opsiyonel bir feature olabilir.
- Belgede geçen tarihsel hash'ler (AAC365945335779E vb.) nokta-anlık; sonraki fix'lerle
  aşıldı — tarihsel.

---

## 6. Migration & Grafik Yükseltmesi (0.1 → 0.2, tamamlandı)

"1.0-hazırlık sertleştirme + grafik yükseltme" kırıcı sürümü (2026-06-25):

- **MSRV → Rust 1.92** (egui 0.34 tabanı; eski 1.89).
- **Grafik yığını:** wgpu 0.20→**29.0.3**, winit 0.29→**0.30.13**, egui→**0.34.3**
  (+ egui-wgpu/winit 0.34.3, egui_dock 0.19.1, transform-gizmo-egui 0.9.0), naga 29.
  Determinizm hash'i (598E315D0E7499FF) tüm yükseltme boyunca değişmedi.
- **API kırıcıları:** `glam` re-export'u resmileşti; `bevy_reflect` `reflect` feature'ına
  alındı; `arrayvec` public'ten çıktı; 96 tip `#[non_exhaustive]`; Error enum'ları +
  Result dönüşleri. Ayrıntılı 11-madde geçiş adımı için git tarihçesine (0.2.0 commit'leri)
  bakılabilir.
- **Kod kararı (mevcut kodu açıklar):** winit 0.30 hâlâ deprecated `EventLoop::run(closure)`
  sunuyor → gizmo-app'in ~600 satırlık closure event-loop'u KASITLI olarak
  `ApplicationHandler`'a taşındı (bkz. `crates/gizmo-app/src/windowed/`).

---

## 7. Kapatılmış Araştırmalar & Non-Goal'lar

**Solver istif kararsızlığı — ÇÖZÜLDÜ.** Dinlenen N≥5 kutu kolonu doğrusal kararsızdı
(yanal BUCKLING / ters-sarkaç, dikey enerji-pompası değil): iteratif temas çözücünün
etkin yanal restoring stiffness'i buckling-kritik değerin altındaydı.
- **Fix (2 katman):** (1) manifold **BLOCK solver** (`solver/block.rs` + `tgs.rs::
  tgs_sweep_block`) — bir manifoldun ≤4 KOPLANAR normal impulsunu BİRLİKTE çözer
  (regularize aktif-küme LCP). İki kritik detay: 4-coplanar blok RANK-EKSİK (4 temas,
  3 DOF) → **Tikhonov reg** (`block_regularization=0.1`) şart; blok **RİJİT** kalmalı
  (soft ölçekleme zayıflatır). (2) **Tam warm-start** (`warm_start_factor` 0.85→1.0) —
  kısmi warm-start her substep %15 impuls atıp re-konverjansta marjinal enerji enjekte
  ediyordu; tam warm-start kapatır. **Sonuç: 1-genişlikte N≤32 kule kararlı** (3000 frame,
  tek zemin boyutu) — aşağıdaki daraltmaya bak, bu cümle eskiden koşulsuz yazılmıştı.
  Determinizm re-bless YOK. Regresyon: `soak_resting_stacks_stay_bounded` (N∈{2,5,16,24,32}).
- **AÇIK:** N≥48 aşırı kule hâlâ buckle olur — friction-aware whole-chain direct/global
  solver gerek (`direct_chain_solve` opt-in flag + `solve_island_normals` yalnız normal
  çözüyor, O(n³)). `soak_extreme_tower_n48` #[ignore].
**Tam temasta manifold TEK NOKTAYA düşüyordu — ÇÖZÜLDÜ** *(2026-08-06)*.
`narrowphase/contacts.rs::clip_box_box` derinlik testi toleranssızdı (`signed_depth <= 0.0`).
Tam temasta dört köşenin de derinliği tam sıfır → hepsi elenip kırpma boş dönüyor → çift
GJK'nın **tek nokta** yedeğine düşüyordu. Tek noktalı manifold sıfır tilt-restoring tork taşır
(blok çözücünün varlık sebebi tam da bu) ve GJK'nın döndürdüğü nokta merkezde değil: ofseti
karşı çarpışıcının boyutuyla büyüyüp duran kutunun kenarına varıyor, hak edilmemiş bir tork
darbesi uyguluyordu. Yarı-boyutu ≲1.5 desteklerde arayüz hiç toparlamıyordu (merkezdeki nokta
kutuyu torksuz tutuyor → hiç batmıyor → kırpma yoluna bir daha girilmiyor).
- **Fix:** slab testinin zaten taşıdığı türden bir tolerans (`DEPTH_TOLERANCE = 1e-4`).
- **Sonuç:** her destek boyutunda doğumda da kararlı halde de 4 köşe noktası; doğum darbesi
  0.03 rad/s → **0**; küçük platformdaki 12 katlı kulenin lean'i 0.024 → **0.0000**.
- **Ortaya çıkardığı yakınsama bedeli:** `block_regularization` 0.1 → **0.05**. Maliyet
  4-noktalı arayüzün kendisinde ve hep oradaydı (1 mm örtüşen zincir birebir aynı yavaşlıkta);
  tolerans onu gizleyen dejenere noktayı kaldırdı. Bir Tikhonov terimi aynı zamanda bir
  yumuşatmadır ve 0.1, terimin hiç uygulanmadığı dönemde seçilmişti. 0.05'te sıkıştırılmış
  zincir 379 kare yerine 0. karede dinleniyor ve momentum sızıntısı 5.4e-4 → 4e-6.
- **Determinizm re-bless:** `46EB56180318E43C` → `15D4FD6845119D8B` (3/3).

**Kısmi uyku istifleri bozuyordu — ÇÖZÜLDÜ** *(2026-08-06)*. Bir cisim, temas adasının ortasında
uykuya dalınca artık ENTEGRE EDİLMİYOR ama hâlâ ÇÖZÜLÜYOR: `solver/tgs.rs` kütlesini uyku
durumuna bakmadan okuyor, tek kapı `is_dynamic()`. Uyanık komşu tepkinin payını alıyor, uyuyan
almıyor → o arayüzde momentum korunmuyor. 12 katlı yığınlar bu yüzden 3000 karede güvenilir
durmuyordu ve statik zeminin yarı-boyutu (20 vs 200) sonucu çeviriyordu.
- **Fix:** uyutma kararı cisim başına değil **temas ADASI** başına, ve çözümden SONRA
  (`RigidBody::advance_sleep_counter` + `pipeline.rs` ada pası). Temassız cisim eskisi gibi
  kendi sayacıyla uyur (o geçiş eklem pasından sonra).
- **Sonuç:** `wide_block_collapse_per_ground` 10/20 çöküşten **0/20**'ye;
  `height_12_stacks_stay_standing` (6 hücre, 3000 kare) **geçiyor**; 1×12×1 kolonun doğal
  uykudaki lean'i 0.0104–10.17'den **0.000106**'ya, yani zorla-uyanık değerle birebir.
- **Kesin kanıt:** düzeltmesiz, 1 sweep'te yığın doğal koşuda 193. karede patlıyor ama zorla
  uyanık kolda hiç patlamıyor → patlama az-çözmeden değil, kısmi uykudan.
- **Yan kazanç:** yerleşmiş yığınlar artık topluca uyuyabildiği için `headless_stress_test`
  1.62 s → 0.51 s.
- **Determinizm re-bless:** `EF6E4AC3644BF3BA` → `46EB56180318E43C` (3/3).
  `golden_state` `settle vy` `-0.0408733` → `0.0`; o sayı bir substep'lik yerçekimiydi ve
  kusurun parmak iziymiş. `settle y` değişmedi.
- **Bedeli:** adanın tek üyesi kıpırdıyorsa ada uyumaz; titreyen bir kutu koca yığını uyanık
  tutabilir. Ölçülen sahnelerde olmadı.
- ⚠️ **"N≤32 robust kararlı" ve "oyun yapıları ≤~12 → gerek yok" ifadeleri yine de dar
  örneklemdi** *(2026-08-05'te daraltıldı)*: 1-GENİŞLİKTE kule, TEK zemin boyutu, 1500 kare.
  Yukarıdaki fix 12 katlı yığınları kurtardı, ama dersin kendisi duruyor — ufuk kadar
  ÖRNEKLEM de genişletilmeli.
- **DERS:** soak-testi ufkunu instabilite başlangıcından ÖTEYE seç (eski `n16` testi 600
  frame'di, patlama ~853'te → yeşil ship edip bug'ı gizledi). **Ve ufuk kadar ÖRNEKLEMİ de
  genişlet:** yukarıdaki daralt-ma, ufku yeterli olan bir testin tek bir şekil ve tek bir
  zemin boyutu denediği için kaçırdığı bir kusurdu.

**Fizik perf, ikinci tur — ÇÖZÜLDÜ** *(2026-08-06)*. Üç kalem, hepsi ölçülü:
- **Broadphase artımlı** (C2): ağaç substep'ler arasında korunuyor. Şişman-marjlı AABB ağacının
  `insert`'ü kutusundan çıkmamış cisim için zaten erken çıkışlı; `clear()` o kazancı çöpe
  atıyordu. `DynamicAabbTree::retain` ile silmeler uzlaştırılıyor. **Determinizm hash'i
  kıpırdamadı** — artımlı ağaç farklı sırada çift yayar, ve bunun simülasyonu değiştirmemesi
  "pair-emission invariance" özelliğinin ampirik kanıtı.
- **ECS köprüsündeki O(N²) writeback** (C3) → handle→index map. (Köprünün benchmark'ı yok;
  saf karmaşıklık düzeltmesi.)
- **Rewind geçmişi opt-in** (C4a): `max_history_frames` 600 → 0. Kare başına 160 B/cisim; eski
  varsayılan 2000 kutuluk stress sahnesinde 192 MB resident tutuyordu.

| senaryo | 2026-08-05 | 2026-08-06 |
|---|---|---|
| `broadphase/1024` | 1.73 ms | **564 µs** |
| `solver_settled_stack/48` | 4.05 ms | **115 µs** |
| `full_step_mixed/512` | 2.43 ms | **1.64 ms** |
| `headless_stress_test` | 1.62 s | **392 ms** |

Yerleşmiş yığındaki büyük düşüş ada-kolektif uykudan, broadphase ve full_step'teki artımlı
ağaçtan geliyor.

**Fizik perf (N² darboğazları) — ÇÖZÜLDÜ.** broadphase `query_pairs` çift-üretimi
(O(P²)→O(P)), TGS per-island scratch'i tüm-dünya yerine ada-boyutunda, per-contact TGS
sabitlerinin 24-sweep döngüsünden HOIST'i → en kötü frame 262→46ms (~5.7×), bit-eş
determinizm.

**6 latent bug (2026-07-13 avı) — HEPSİ DÜZELTİLDİ.** tangent (model_mat3, inverse-transpose
değil), PBR-pack taşması (`.min(999.0)`), query get/contains table-storage With/Without
kapısı, batch-shadow instance-region ayrımı, glTF `AlphaMode::Mask` cutout. **Yanlış-pozitif
olarak elenenler (tekrar kovalamayın):** deferred_lighting f16 aniso, gbuffer bitangent-collapse,
vehicle point-velocity COM, narrowphase incident-corner. *Kalan minör:* PBR params hâlâ tek f32'ye
ondalık paketli (>2²⁴ hassasiyet düşer) — uzun-vade ayrı slot.

**NON-GOAL: narrowphase batch-SIMD.** Araştırıldı, REDDEDİLDİ (2026-07-14). Ölçüm
(wide_scene 2000 kutu, ~30ms frame): box-box SAT compute yalnız **~%3.3**; narrowphase
post-processing batchlenemez; iki per-pair SIMD denemesi de regresyon yaptı (skaler kod
zaten auto-vektörize). Step-0 gate'i yeniden geçmeden TEKRAR DENEME. ("~%82 narrowphase"
figürü OBSOLETE.)

---

## 8. Çalışma Yöntemi

- Her madde: **düzelt → regresyon testi yaz → derle/test/clippy → işaretle.**
- Davranış-değiştiren fizik fix'lerini `headless_stress_test` + odaklı senaryolarla doğrula;
  soak ufkunu instabilite başlangıcının ötesine seç.
- Bug-avı turlarında subagent fan-out kullan, sonra her bulguyu ELLE doğrula
  (false-positive'leri ele).
- CI: `cargo clippy --all-features --all-targets -- -D warnings -A too_many_arguments
  -A type_complexity` (grandfather'lı iki mimari lint). Giriş crate'i `gizmo-engine`
  (`-p gizmo` DEĞİL); `| tail` cargo exit kodunu maskeler — exit status'u ayrı kontrol et.
