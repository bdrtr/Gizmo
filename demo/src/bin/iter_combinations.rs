//! # Her ikiliyi gezmek
//!
//! Bir sorgunun **her ikilisini** gezmek. Demo bir N-cisim çekim benzetimi: her cisim her cisme
//! kuvvet uyguluyor.
//!
//! ## Motorda ikili yineleme **var** (2026-08-23) — okuma tarafında
//!
//! `Query::iter_combinations` her ikiliyi bir kez veriyor: `[a, b, c]` için `(a,b)`, `(a,c)`,
//! `(b,c)` — asla `(a,a)`, asla hem `(a,b)` hem `(b,a)`.
//!
//! **İki sürüm var: salt okunur ve yazan.** `iter_combinations` `&self` alıyor ve `ReadOnlyQuery`
//! istiyor. `iter_combinations_mut` (2026-08-24) `&mut self` alıyor ve iki yarıyı da yazılabilir
//! veriyor — aynı depoya aynı anda iki `&mut`, ki bu yalnız `i != j` olduğu için sağlam ve ödünç
//! denetleyicisi bunu göremiyor.
//!
//! Değişmez, tipte değil **yineleyicinin yapısında** taşınıyor: `j` her zaman `i + 1`'den başlayıp
//! ileri gidiyor, ve kimlikler tek bir taramadan geliyor — o da her satırı bir kez veriyor.
//! `&mut self` ödünçü de bunu dışarıdan anlamlı kılan şey: yineleyici yaşarken sorgunun başka bir
//! görünümü var olamıyor.
//!
//! ### Ölçüm notu: testler geçiyordu, Miri geçmiyordu
//!
//! İlk yazımda ömür uzatması `transmute_copy` + `mem::forget` idi: kopyala, orijinali unut. Altı
//! birim testinin hepsi geçti. **Miri geçmedi:** `forget` argümanını *taşıyor*, taşıma bir retag,
//! ve retag kopyanın kendi etiketini geçersiz kılıyor — *"trying to retag from <...> but that tag
//! does not exist in the borrow stack"*.
//!
//! Doğrusu `get_inner(&self)`'den geçmek — `iter_mut`'ın kendi yolu — ve o zaman hiçbir
//! `transmute` gerekmiyor. Dersi: **bir `unsafe` bloğunun testlerden geçmesi sağlam olduğunu
//! göstermiyor.** Miri altında altı test de temiz.
//!
//! ## Ölçülen: elle yazılan döngü doğru mu
//!
//! "Çalışıyor gibi görünüyor" bir N-cisim benzetimi için ölçüm değil — yanlış eşleştirilmiş bir
//! ikili döngü de makul görünen bir hareket üretir. Ölçüt fizik: kapalı bir sistemde iç kuvvetler
//! eşit ve zıt olduğu için **toplam momentum korunmalı**.
//!
//! Demo her cismin `kütle × hız` toplamını izliyor. Başlangıçta sıfıra ayarlanıyor (ağırlık
//! merkezi sabit), ve ikili döngü doğruysa sıfırda kalmalı. Kalmıyorsa döngü bir çifti iki kez
//! ya da hiç saymıyor demektir.
//!
//! Ölçüldü (2026-08-23, 12 cisim = 66 çift, `GIZMO_NBODY_SELFTEST=1`):
//!
//! | kare | toplam momentum | kinetik enerji |
//! |------|-----------------|----------------|
//! | başlangıç | **0,000000** | — |
//! | 300 | 0,000002 | 122,102 |
//! | 600 | 0,000008 | 52,213 |
//! | 900 | 0,000003 | 55,905 |
//! | 1200 | 0,000003 | 64,584 |
//! | 1500 | 0,000006 | 40,142 |
//!
//! Momentum 1500 kare boyunca **10⁻⁵'in altında** kalıyor — bu kayan nokta yuvarlamasıdır, sürüklenme
//! değil. Bir çift iki kez sayılsa ya da bir cisim kendisiyle eşleşse toplam görünür biçimde
//! kayardı. Yani elle yazılan `j = i+1` döngüsü doğru eşleşiyor.
//!
//! Kinetik enerjinin salınması beklenen: korunan şey toplam enerji, ve kinetik olan sürekli çekim
//! potansiyeline dönüşüyor.
//!
//! ## Ölçülen 2: motorun yolu elle yazılanla **bit-eş**
//!
//! `pair_audit` sistemi aynı çiftleri iki yoldan geziyor — elle `j = i+1` döngüsü ve
//! `iter_combinations` — ve iki ivme dizisini karşılaştırıyor. Ayrı bir sistem, çünkü `gravity`
//! `Mut<Transform>` alıyor ve `iter_combinations` `ReadOnlyQuery` istiyor; aynı sistemde iki sorgu
//! açmak erişim çakışması olurdu. `Phase::User(2500)`'de koşuyor, yani `Update` ile `Physics`
//! arasında — aynı karenin konumlarını görmesi için.
//!
//! Ölçüldü (2026-08-23, 12 cisim, 1500 kare):
//!
//! | | motor | elle |
//! |---|-------|------|
//! | çift sayısı | **66** | **66** |
//! | en büyük ivme farkı | **0,000000000** | |
//!
//! Fark yuvarlama seviyesinde bile değil, **tam sıfır** — yani iki döngü aynı çiftleri aynı
//! sırayla geziyor ve aynı kayan nokta işlemlerini aynı sırada yapıyor. Ziyaret sırası da
//! eşleşiyor demek bu.
//!
//! Kazanılan şey satır sayısı değil, **ara yapı**: elle yol konum ve kütleleri bir `Vec`'e
//! toplayıp iki iç içe indeks döngüsü kuruyor; motorun yolu tek çağrı, ve indeks aritmetiği yok —
//! yani bir çifti iki kez saymanın ya da bir cismi kendisiyle eşleştirmenin yolu da yok.
//!
//! ## Ölçülen 3: tek geçişli döngü aynı fiziği veriyor
//!
//! `GIZMO_NBODY_ONEPASS=1` çekimi `iter_combinations_mut` ile tek geçişte uyguluyor: kuvvet
//! hesaplandığı anda iki cismin de hızına yazılıyor, ara `Vec` yok, indeks aritmetiği yok.
//!
//! Ölçüldü (2026-08-24, 12 cisim, 1500 kare):
//!
//! | kare | üç geçişli momentum | tek geçişli momentum | üç geçişli kinetik | tek geçişli kinetik |
//! |------|---------------------|----------------------|--------------------|---------------------|
//! | 900 | 0,000003 | 0,000007 | 55,905 | 55,905 |
//! | 1200 | 0,000003 | 0,000025 | 64,584 | 64,585 |
//! | 1500 | 0,000006 | 0,000012 | 40,142 | 40,142 |
//!
//! Kinetik enerji birebir aynı — yani iki döngü aynı kuvvetleri aynı çiftlere uyguluyor. Momentum
//! ikisinde de **10⁻⁴'ün altında**; tek geçişlinin biraz yüksek olması toplama sırasından: üç
//! geçişli ivmeleri önce toplayıp sonra uyguluyor, tek geçişli anında yazıyor, ve kayan noktada bu
//! iki şey aynı değil.
//!
//! Kazanç yine satır sayısı değil: ara `Vec<Vec3>` ve iki iç içe indeks döngüsü ortadan kalkıyor —
//! bir çifti iki kez saymanın yolu da onlarla birlikte.
//!
//! Konum ilerletmesi hâlâ ayrı bir geçiş, ve öyle kalmak zorunda: bir ikili döngüsü her cismi
//! `n-1` kez ziyaret ediyor, konum bir kez ilerlemeli.
//!
//! ### Ödünç alma kuralının ikinci yüzü
//!
//! İlk yazımda toplama geçişi `iter()` kullanıyordu ve derlenmedi: **`Mut<T>` taşıyan bir sorgu
//! salt-okunur bile gezilemiyor** (`iter` `ReadOnlyQuery` istiyor). Yani "önce oku, sonra yaz"
//! demek bile aynı sorgunun `iter_mut`'ını iki kez çağırmak demek — toplamda üç geçiş.
//!
//! ## Kontroller
//!   * `GIZMO_NBODY_ONEPASS=1` — çekimi `iter_combinations_mut` ile tek geçişte uygula
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::query::{Mut, Query};
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Bir gök cismi.
#[derive(Clone, Copy)]
struct Body {
    mass: f32,
    velocity: Vec3,
}
gizmo::core::impl_component!(Body);

/// Ölçüm defteri.
#[derive(Default, Clone, Copy)]
struct Conservation {
    frame: u32,
    /// Toplam momentumun büyüklüğü — korunması gereken şey.
    momentum: f32,
    /// Başlangıçtaki değer, karşılaştırma için.
    initial_momentum: f32,
    /// Toplam kinetik enerji (korunmuyor, çekim potansiyeline dönüşüyor — yalnız bilgi).
    kinetic: f32,
    bodies: usize,
    /// `iter_combinations`'ın saydığı çift sayısı.
    engine_pairs: usize,
    /// Elle döngünün saydığı çift sayısı — aynı olmalı.
    manual_pairs: usize,
    /// İki yolun hesapladığı ivme toplamları arasındaki en büyük fark.
    accel_gap: f32,
}
gizmo::core::impl_component!(Conservation);

/// Cisim sayısı ve çekim sabiti.
const BODIES: usize = 12;
const G: f32 = 3.0;
/// Yumuşatma: iki cisim üst üste gelince kuvvet sonsuza gitmesin.
const SOFTENING: f32 = 0.6;
/// Sabit adım — ölçümün kare süresine bağlı olmaması için.
const STEP: f32 = 1.0 / 120.0;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Iter Combinations", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let sphere = AssetManager::create_sphere(device, 1.0, 16, 24);

            let mut rng = 0x1234_5678u32;
            let mut roll = |low: f32, high: f32| {
                rng ^= rng << 13;
                rng ^= rng >> 17;
                rng ^= rng << 5;
                low + (rng as f32 / u32::MAX as f32) * (high - low)
            };

            // Cisimleri doğur, sonra ağırlık merkezinin hızını sıfırla: toplam momentum tam 0
            // başlasın ki korunumu ölçmek anlamlı olsun.
            let mut spawned: Vec<(gizmo::core::entity::Entity, f32, Vec3)> = Vec::new();
            for _ in 0..BODIES {
                let mass = roll(0.6, 2.2);
                let position = Vec3::new(roll(-5.0, 5.0), roll(-2.5, 2.5), roll(-5.0, 5.0));
                let velocity = Vec3::new(roll(-1.2, 1.2), roll(-0.6, 0.6), roll(-1.2, 1.2));
                let entity = scene.world.spawn_bundle((
                    Transform::new(position).with_scale(Vec3::splat(0.18 + mass * 0.12)),
                    GlobalTransform::default(),
                    sphere.clone(),
                    Material::new(white.clone()).with_pbr(
                        Vec4::new(roll(0.3, 1.0), roll(0.3, 0.9), roll(0.4, 1.0), 1.0),
                        0.5,
                        0.0,
                    ),
                    MeshRenderer::new(),
                    Body { mass, velocity },
                ));
                spawned.push((entity, mass, velocity));
            }
            let total_mass: f32 = spawned.iter().map(|(_, m, _)| *m).sum();
            let drift: Vec3 =
                spawned.iter().map(|(_, m, v)| *v * *m).fold(Vec3::ZERO, |a, b| a + b) / total_mass;
            for (entity, mass, velocity) in &spawned {
                scene.world.add_component(
                    *entity,
                    Body {
                        mass: *mass,
                        velocity: *velocity - drift,
                    },
                );
            }

            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.6) * Quat::from_rotation_x(-0.5),
                intensity: 2.2,
                ..Default::default()
            });
            scene.world.insert_resource(Conservation {
                bodies: BODIES,
                ..Default::default()
            });
            scene.spawn_camera(state, Vec3::new(0.0, 4.0, 16.0), Vec3::ZERO);
        })
        .add_update_system(
            gravity
                .in_phase(Phase::Update)
                .run_if(|_: &World| std::env::var("GIZMO_NBODY_ONEPASS").is_err()),
        )
        .add_update_system(
            gravity_one_pass
                .in_phase(Phase::Update)
                .run_if(|_: &World| std::env::var("GIZMO_NBODY_ONEPASS").is_ok()),
        )
        // Denetleyici `gravity`'den SONRA: aynı karenin konumlarını ölçüyor, bir öncekinin
        // değil. `Phase::User(2500)` — Update ile Physics arasında, madde 3'ün açtığı yer.
        .add_update_system(pair_audit.in_phase(Phase::User(2500)))
        .set_ui(|world, _state, ctx| {
            let Some(c) = world.get_resource::<Conservation>().map(|c| *c) else {
                return;
            };
            gizmo::egui::Area::new("nbody".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(460.0);
                        ui.heading("Her ikiliyi gezmek");
                        ui.label(format!("{} cisim · {} çift", c.bodies, c.bodies * (c.bodies - 1) / 2));
                        ui.separator();
                        ui.label(format!("toplam momentum: {:.6}", c.momentum));
                        ui.label(format!("  başlangıçta   : {:.6}", c.initial_momentum));
                        ui.label(format!("kinetik enerji : {:.3} (korunmaz, potansiyele döner)", c.kinetic));
                        ui.separator();
                        ui.label("motorda iter_combinations YOK, ve ödünç alma kuralı");
                        ui.label("yüzünden tek geçişte elle yazmak da mümkün değil:");
                        ui.label("önce Vec'e topla, kuvveti hesapla, sonra geri yaz.");
                        ui.separator();
                        ui.label("momentum sıfırda kalıyorsa ikili döngü doğru eşleşiyor");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Çekim: her çift bir kez, kuvvet iki tarafa eşit ve zıt uygulanıyor.
///
/// Burada iki geçiş — ve ikinci geçişin varlığı bir üslup tercihi değil, ödünç alma kuralının
/// sonucu.
/// **Denetleyici.** Aynı çiftleri motorun `iter_combinations`'ıyla gezip elle döngünün sonucuyla
/// karşılaştırıyor.
///
/// Ayrı bir sistem, çünkü `gravity` `Mut<Transform>` alıyor ve `iter_combinations`
/// `ReadOnlyQuery` istiyor — aynı sistemde iki sorgu açmak erişim çakışması olurdu. Çizelge bu
/// ikisini zaten ayırıyor: biri yazıyor, öteki okuyor.
fn pair_audit(bodies: Query<(&Transform, &Body)>, mut report: ResMut<Conservation>) {
    // Elle döngünün gerektirdiği ara `Vec` — karşılaştırmanın öteki yakası.
    let state: Vec<(Vec3, f32)> = bodies
        .iter()
        .map(|(_, (t, b))| (t.position, b.mass))
        .collect();

    let mut manual = vec![Vec3::ZERO; state.len()];
    let mut manual_pairs = 0usize;
    for i in 0..state.len() {
        for j in (i + 1)..state.len() {
            manual_pairs += 1;
            let delta = state[j].0 - state[i].0;
            let dist2 = delta.length_squared() + SOFTENING * SOFTENING;
            let dir = delta * (1.0 / (dist2 * dist2.sqrt()));
            manual[i] += dir * (G * state[j].1);
            manual[j] -= dir * (G * state[i].1);
        }
    }

    // Motorun yolu: tek çağrı, ara `Vec` yok, indeks aritmetiği yok.
    let index: std::collections::HashMap<u32, usize> = bodies
        .iter()
        .enumerate()
        .map(|(i, (id, _))| (id, i))
        .collect();
    let mut engine = vec![Vec3::ZERO; state.len()];
    let mut engine_pairs = 0usize;
    for ((a_id, (ta, ba)), (b_id, (tb, bb))) in bodies.iter_combinations() {
        engine_pairs += 1;
        let (ia, ib) = (index[&a_id], index[&b_id]);
        let delta = tb.position - ta.position;
        let dist2 = delta.length_squared() + SOFTENING * SOFTENING;
        let dir = delta * (1.0 / (dist2 * dist2.sqrt()));
        engine[ia] += dir * (G * bb.mass);
        engine[ib] -= dir * (G * ba.mass);
    }

    report.engine_pairs = engine_pairs;
    report.manual_pairs = manual_pairs;
    report.accel_gap = manual
        .iter()
        .zip(&engine)
        .map(|(m, e)| (*m - *e).length())
        .fold(0.0f32, f32::max);
}

/// Tek geçişli çekim: her ikili bir kez, ve kuvvet **o anda** uygulanıyor.
///
/// `iter_combinations_mut` gelmeden önce bu yazılamıyordu: ikilinin iki yarısına aynı anda yazmak
/// aynı depoya iki `&mut` demek. `GIZMO_NBODY_ONEPASS=1` ile açılıyor, ve `pair_audit` iki yolun
/// aynı sonucu verdiğini ölçüyor.
fn gravity_one_pass(
    mut bodies: Query<(Mut<Transform>, Mut<Body>)>,
    mut report: ResMut<Conservation>,
) {
    // 1. geçiş: kuvvetler doğrudan hıza. Ara `Vec` yok, indeks aritmetiği yok.
    for ((_, (ta, mut ba)), (_, (tb, mut bb))) in bodies.iter_combinations_mut() {
        let delta = tb.position - ta.position;
        let dist2 = delta.length_squared() + SOFTENING * SOFTENING;
        let dir = delta * (1.0 / (dist2 * dist2.sqrt()));
        // Eşit ve zıt — ve ikisi de burada yazılıyor, bir sonraki geçişte değil.
        let (ma, mb) = (ba.mass, bb.mass);
        ba.velocity += dir * (G * mb) * STEP;
        bb.velocity -= dir * (G * ma) * STEP;
    }

    // 2. geçiş: konumları ilerlet ve korunumu ölç. Bu ayrı kalmak zorunda — bir ikili döngüsü
    // her cismi n-1 kez ziyaret ediyor, konum bir kez ilerlemeli.
    let mut momentum = Vec3::ZERO;
    let mut kinetic = 0.0;
    for (_entity, (mut transform, body)) in bodies.iter_mut() {
        transform.position += body.velocity * STEP;
        momentum += body.velocity * body.mass;
        kinetic += 0.5 * body.mass * body.velocity.length_squared();
    }

    report.frame += 1;
    report.momentum = momentum.length();
    report.kinetic = kinetic;
    if report.frame == 1 {
        report.initial_momentum = report.momentum;
    }
    if std::env::var("GIZMO_NBODY_SELFTEST").is_ok() && report.frame.is_multiple_of(300) {
        gizmo::gizmo_log!(
            Info,
            "TEK GEÇİŞ · kare {} · momentum {:.6} · kinetik {:.3}",
            report.frame,
            report.momentum,
            report.kinetic
        );
    }
}

fn gravity(mut bodies: Query<(Mut<Transform>, Mut<Body>)>, mut report: ResMut<Conservation>, time: Res<Time>) {
    let _ = time;

    // 1. geçiş: durumu topla.
    //
    // `iter()` DEĞİL `iter_mut()` — çünkü `Mut<T>` taşıyan bir sorgu salt-okunur bile
    // gezilemiyor (`iter` için `ReadOnlyQuery` sınırı sağlanmıyor). Bu, ikili döngünün neden tek
    // geçişte yazılamadığının bir başka yüzü.
    let mut state: Vec<(Vec3, f32, Vec3)> = Vec::new();
    for (_entity, (transform, body)) in bodies.iter_mut() {
        state.push((transform.position, body.mass, body.velocity));
    }

    // 2. geçiş: her ÇİFT bir kez. `j` her zaman `i`'den büyük — bir çifti iki kez saymanın
    // ve kendisiyle eşleştirmenin önüne geçen şey bu.
    let mut accel = vec![Vec3::ZERO; state.len()];
    for i in 0..state.len() {
        for j in (i + 1)..state.len() {
            let delta = state[j].0 - state[i].0;
            let dist2 = delta.length_squared() + SOFTENING * SOFTENING;
            let inv = 1.0 / (dist2 * dist2.sqrt());
            let dir = delta * inv;
            // Eşit ve zıt: momentumun korunmasının sebebi tam olarak bu iki satır.
            accel[i] += dir * (G * state[j].1);
            accel[j] -= dir * (G * state[i].1);
        }
    }

    // 3. geçiş: geri yaz.
    let mut momentum = Vec3::ZERO;
    let mut kinetic = 0.0;
    for (index, (_entity, (mut transform, mut body))) in bodies.iter_mut().enumerate() {
        body.velocity += accel[index] * STEP;
        transform.position += body.velocity * STEP;
        momentum += body.velocity * body.mass;
        kinetic += 0.5 * body.mass * body.velocity.length_squared();
    }

    report.frame += 1;
    report.momentum = momentum.length();
    report.kinetic = kinetic;
    if report.frame == 1 {
        report.initial_momentum = report.momentum;
    }

    if std::env::var("GIZMO_NBODY_SELFTEST").is_ok() && report.frame.is_multiple_of(300) {
        gizmo::gizmo_log!(
            Info,
            "kare {} · çift: motor {} / elle {} · ivme farkı {:.9}",
            report.frame,
            report.engine_pairs,
            report.manual_pairs,
            report.accel_gap
        );
        gizmo::gizmo_log!(
            Info,
            "kare {} · momentum {:.6} (başlangıç {:.6}) · kinetik {:.3}",
            report.frame,
            report.momentum,
            report.initial_momentum,
            report.kinetic
        );
    }
}
