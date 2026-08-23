//! # Newton Sarkacı — N elastik top kirişe **ip eklemiyle** asılı
//!
//! Gerçek fizik: her top X–Y düzleminde sallanır, çarpışmalar elastik
//! (restitution≈1, sürtünme≈0). Toplar dinlenerek başlar; en soldaki top
//! geri-çekilmiş doğar ve açılışta salınıma girer.
//!
//! Bu sürüm motorun modern olanaklarını kullanır; NEYİN motora NEYİN oyuna ait
//! olduğu konusunda dürüst olalım:
//!   * **Toplar = `spawn_bundle` + explicit `Collider::sphere`** — Prefab DEĞİL:
//!     Prefab yalnız kutu-collider verir ve her örneğin kendi başlangıç açısı/hızı
//!     olduğundan (Prefab bunları gömemez) doğrudan bundle ile spawn edilir.
//!   * **İp = `Joint::rope` (motorda birinci-sınıf)** — esnemez ama gevşeyebilir
//!     (dist ≤ L). Ankor A = kiriş pivotu, B = top merkezi. Elle konum kırpma HİLESİ yok.
//!   * **Görsel ip = fiziksiz ince çubuk** — her kare topun konumuna göre gerilir.
//!   * **Sürükleme = `Camera::screen_to_ray` + fizik `raycast` + hız servosu** — sol tıkla
//!     topu seç ve hedefe doğru sür. Gövde **DİNAMİK kalır**: eskiden kinematik yapılırdı
//!     ("komşuları itsin" diye) ve bu ölçülerek çürüdü — kinematik cisim geri itilemediği
//!     için servo tıkalıyken bile ısrar ediyor, toplar birbirine 0,572 m gömülüyordu (çap
//!     1,00 m; çakışan çift 0-2, yani aradaki topun içinden geçilmiş) ve çözücü o girişimi
//!     çözerken sahneyi 20-27 m/s'ye savuruyordu. Dinamikte servo yine iter, ama temas ona
//!     karşı koyabilir.
//!   * **Sıfırlama = YERİNDE restore + `Joint::reset_warm_start`** — despawn/respawn DEĞİL:
//!     ip eklemleri entity-id tutar, varlıkları yeniden yaratmak eklemleri koparırdı.
//!     Kenar-tespiti motorun `is_key_just_pressed` API'sinden (elle `prev_r` takibi yok).
//!   * **Sahne render = `default_render_pass` DOĞRUDAN** — `with_scene_render()` kısayolu
//!     SSR/SSGI/volumetric/TAA'yı kapatırdı; bu sahne yansımaları/keskinliği ister.
//!
//! Bu demoda geçici/uçan varlık (mermi/konfeti) yok ve sıfırlama yerinde yapılır →
//! dolayısıyla `DespawnAfter`/`despawn_all_with` idiomları uygulanmaz.
//!
//! ## Kontroller
//!   * **Sol tık + sürükle** — bir topu yakala, ark üzerinde sürükle, bırak → salınır.
//!   * **R** — sahneyi sıfırla: toplar ev pozuna, hızlar sıfır, açılış salınımı yeniden.
//!   * **1..5** — en soldaki topu 10/20/40/80/160 m/s ile fırlat (ip teğeti boyunca).
//!     Fare ile "hızlı sallamak" tekrarlanabilir değil; bu tuşlar aynı olayı aynı şiddette
//!     kurar ve sürüklemenin 15 m/s kelepçesinin üstüne çıkar.

use std::f32::consts::{FRAC_PI_2, FRAC_PI_3};

use gizmo::core::input::mouse;
use gizmo::physics::components::{BodyType, CombineMode, PhysicsMaterial};
use gizmo::physics::joints::Joint;
use gizmo::physics::raycast::Ray;
use gizmo::physics::world::PhysicsWorld;
use gizmo::physics::BodyHandle;
use gizmo::prelude::*;

// ------------------------------------------------------------------ ayarlar
const N: usize = 5; // top sayısı
const R: f32 = 0.5; // top yarıçapı
const L: f32 = 4.0; // ip uzunluğu (pivot → top merkezi)
const PIVOT_Y: f32 = 6.0; // asma yüksekliği
const MASS: f32 = 1.0;
const GAP: f32 = 0.01; // toplar dinlenirken sadece değsin
/// Girişim ölçerinin tuttuğu kare sayısı. Yarım saniyelik pencere: yaklaşmanın tamamını
/// kapsar, konsolu boğacak kadar uzun değil.
const HISTORY: usize = 30;

/// Elastik (mükemmel yansıyan, sürtünmesiz) çarpışma malzemesi.
fn elastic() -> PhysicsMaterial {
    PhysicsMaterial {
        restitution: 1.0,
        static_friction: 0.0,
        dynamic_friction: 0.0,
        restitution_combine: CombineMode::Max,
        ..Default::default()
    }
}

/// Statik ön-görünüm kamerası + sürükleme durumu + sıfırlama için ev pozları.
struct Cradle {
    balls: Vec<u32>,
    ropes: Vec<u32>, // her topa karşılık gelen görsel ip (fiziksiz ince çubuk)
    pivots: Vec<Vec3>,
    homes: Vec<Transform>, // topların kuruluş pozu — R ile buraya dönülür
    cam: Camera,
    cam_pos: Vec3,
    dragging: Option<usize>, // balls içindeki index
    /// Şimdiye kadar görülen en derin top-top girişimi (metre).
    ///
    /// **Tanı, mekanizma değil.** "Hızlı sallayınca toplar birbirinin içinden geçiyor"
    /// başsız harness'ta ÜRETİLEMEDİ: iki küre 320 m/s'ye kadar, değen beşli zincir
    /// 320 m/s'ye kadar, ipli tam sarkaç doğal salınımda (≤10 m/s) ve kinematik
    /// sürükleme ipe bağlı komşulara karşı — hiçbirinde girişim yok. Kare takılması da
    /// suçlu olamaz: `PhysicsWorld::step` tavana vurunca adımı büyütmez, zamanı düşürür.
    /// Geriye gerçek koşuda ölçmek kaldı, o yüzden bu satır burada.
    worst_overlap: f32,
    /// Son karelerin konum/hız kaydı — olayın kendisi kadar öncesi de lazım.
    history: std::collections::VecDeque<(f32, Vec<Vec3>, Vec<Vec3>)>,
    /// En son hangi derinlikte döküm yazıldı. Rekor katlandıkça yeniden yazılır.
    dumped_at: f32,
    /// Beklenmedik hız için döküm bir kez yazılır.
    dumped_fast: bool,
}

// --------------------------------------------------------------- setup
fn setup(world: &mut World, renderer: &mut Renderer) -> Cradle {
    let mut assets = AssetManager::new();
    let tex = assets.create_white_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );
    let sphere = AssetManager::create_sphere(&renderer.device, R, 32, 32);
    let cube = AssetManager::create_cube(&renderer.device);

    // Güneş
    world.spawn_bundle((
        Transform::new(Vec3::new(20.0, 40.0, 20.0)).with_rotation(Quat::from_rotation_x(-0.9)),
        DirectionalLight::new(Vec3::new(1.0, 0.97, 0.9), 3.0, LightRole::Sun),
    ));

    // Sabit ön-görünüm kamerası
    let cam = Camera::new(FRAC_PI_3, 0.1, 500.0, -FRAC_PI_2, -0.05, true);
    let cam_pos = Vec3::new(0.0, PIVOT_Y - L + 1.0, 11.0);
    world.spawn_bundle((Transform::new(cam_pos), cam));

    let spacing = 2.0 * R + GAP;
    let start_x = -((N as f32 - 1.0) / 2.0) * spacing;

    // Üst kiriş (statik): ipler buradaki sabit pivotlara bağlanır.
    let beam = world.spawn_bundle((
        Transform::new(Vec3::new(0.0, PIVOT_Y, 0.0)).with_scale(Vec3::new(
            N as f32 * spacing + 1.0,
            0.15,
            0.15,
        )),
        cube.clone(),
        Material::new(tex.clone()).with_pbr(Vec4::new(0.15, 0.15, 0.18, 1.0), 0.4, 0.5),
        MeshRenderer::new(),
        RigidBodyBundle::static_body(),
    ));

    let mut phys = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0));
    let mut balls = Vec::new();
    let mut ropes = Vec::new();
    let mut pivots = Vec::new();
    let mut homes = Vec::new();

    for i in 0..N {
        let pivot = Vec3::new(start_x + i as f32 * spacing, PIVOT_Y, 0.0);
        // Top 0 geri-çekilmiş doğar (pivottan L uzakta) → AÇILIŞTA salınır.
        let (center, rot) = if i == 0 {
            let a = 55.0_f32.to_radians();
            (
                pivot + L * Vec3::new(-a.sin(), -a.cos(), 0.0),
                Quat::from_rotation_z(-a),
            )
        } else {
            (pivot - Vec3::new(0.0, L, 0.0), Quat::IDENTITY)
        };
        let color = if i == 0 || i == N - 1 {
            Vec4::new(0.85, 0.15, 0.15, 1.0)
        } else {
            Vec4::new(0.82, 0.82, 0.86, 1.0)
        };

        // Küre gövde: collider'dan atalet otomatik türetilir, malzeme elastik.
        // `home` hem spawn pozu hem de R ile dönülecek hedef — tek kaynaktan, ikisi ayrışamaz.
        let home = Transform::new(center).with_rotation(rot);
        let ball = world.spawn_bundle((
            home,
            sphere.clone(),
            Material::new(tex.clone()).with_pbr(color, 0.9, 0.2),
            MeshRenderer::new(),
            RigidBodyBundle::dynamic(MASS)
                .with_collider(Collider::sphere(R).with_material(elastic())),
        ));

        // İp eklemi: gevşekken serbest düşer, gerilince yakalar (dist ≤ L).
        phys.joints.push(Joint::rope(
            BodyHandle::from_id(beam.id()),
            BodyHandle::from_id(ball.id()),
            pivot - Vec3::new(0.0, PIVOT_Y, 0.0),
            Vec3::ZERO,
            L,
        ));
        // Görsel ip: fiziksiz ince çubuk (pivot↔top), her kare `update`'te konumlanır.
        let rope = world.spawn_bundle((
            Transform::new(pivot),
            cube.clone(),
            Material::new(tex.clone()).with_pbr(Vec4::new(0.05, 0.05, 0.06, 1.0), 0.7, 0.2),
            MeshRenderer::new(),
        ));

        balls.push(ball.id());
        ropes.push(rope.id());
        pivots.push(pivot);
        homes.push(home);
    }

    world.insert_resource(phys);
    world.insert_resource(assets);
    Cradle {
        balls,
        ropes,
        pivots,
        homes,
        cam,
        cam_pos,
        dragging: None,
        worst_overlap: 0.0,
        history: std::collections::VecDeque::new(),
        dumped_at: 0.0,
        dumped_fast: false,
    }
}

// --------------------------------------------------------------- update
fn update(world: &mut World, state: &mut Cradle, _dt: f32, input: &Input) {
    // ── R = sıfırla ──────────────────────────────────────────────────────────
    // Sürükleme bloğundan ÖNCE: sıfırlama yarım kalmış bir sürüklemeyi de iptal eder,
    // aşağıdaki raycast ise artık ışınlanmış (ev pozundaki) toplara bakar.
    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        reset(world, state);
    }

    // ── 1..5 = ölçülü fırlatma ───────────────────────────────────────────────
    // Fare ile "çok hızlı sallamak" tekrarlanabilir değil: her denemede başka bir hız
    // çıkar ve bir olayı iki kez aynı şiddette kurmak imkânsız. Tuşlar en soldaki topa
    // ipin teğeti yönünde bilinen bir hız verir — ip gergin kalır, yani verilen şey hız
    // olur, ipin sert yakalayışı değil. Sürükleme servosunun 15 m/s kelepçesini de
    // aşarlar, ki asıl merak edilen aralık orası.
    for (k, speed) in [(KeyCode::Digit1, 10.0f32), (KeyCode::Digit2, 20.0), (KeyCode::Digit3, 40.0),
                       (KeyCode::Digit4, 80.0), (KeyCode::Digit5, 160.0)] {
        if input.is_key_just_pressed(k as u32) {
            let idx = 0;
            let id = state.balls[idx];
            let (pivot, at) = {
                let ts = world.borrow::<Transform>();
                (state.pivots[idx], ts.get(id).map(|t| t.position).unwrap_or(state.pivots[idx]))
            };
            // İpe dik yön, salınım düzleminde: (pivot→top) vektörünü 90° çevir.
            let along = (at - pivot).normalize_or_zero();
            let tangent = Vec3::new(-along.y, along.x, 0.0).normalize_or_zero();
            // Diğer topların bulunduğu yana doğru: sağa.
            let dir = if tangent.x < 0.0 { -tangent } else { tangent };
            let mut vs = world.borrow_mut::<Velocity>();
            if let Some(mut v) = vs.get_mut(id) {
                v.linear = dir * speed;
                v.angular = Vec3::ZERO;
            }
            println!("fırlatma: top {idx} → {speed:.0} m/s (ip teğeti boyunca)");
        }
    }

    // ── Fare ile sürükle-bırak ───────────────────────────────────────────────
    let viewport = world
        .get_resource::<WindowInfo>()
        .map(|w| (w.width, w.height))
        .unwrap_or((1280.0, 720.0));
    // Ekran pikselinden dünya ışını (unproject).
    let ray = state
        .cam
        .screen_to_ray(input.mouse_position(), viewport, state.cam_pos);
    // screen_to_ray SIMD `Ray` döner; fizik Ray/matematik Vec3 ister → dönüştür.
    let (ro, rd) = (Vec3::from(ray.origin), Vec3::from(ray.direction));
    let lmb = input.is_mouse_button_pressed(mouse::LEFT);

    // Yakalama: LMB basılıyken ışını topa raycast et (henüz sürüklenmiyorsa).
    if lmb && state.dragging.is_none() {
        let hit_id = world
            .get_resource::<PhysicsWorld>()
            .and_then(|p| p.raycast(&Ray::new(ro, rd), 100.0))
            .map(|h| h.entity.id());
        if let Some(idx) = hit_id.and_then(|id| state.balls.iter().position(|&b| b == id)) {
            state.dragging = Some(idx);
            // **Tutulan top DİNAMİK kalır.** Eskiden kinematik yapılırdı, gerekçesi de
            // makuldü — "komşuları itsin" — ama kinematik cisim tanım gereği **geri
            // itilemez**, ve servo hedefe varamadığı sürece her kare aynı hızı yeniden
            // dayattığı için tıkalıyken bile ısrarla ileri sürer. Sonucu ölçüldü: canlı
            // koşuda toplar birbirine **0,572 m** gömüldü (çap 1,00 m), üstelik çakışan
            // çift 0-2 idi — yani 0, aradaki 1'in İÇİNDEN geçmişti. Ardından çözücü o
            // girişimi çözmeye çalışırken tek karede 19,4 m/s'lik düzeltmeler üretti ve
            // sahne 20-27 m/s'lik hızlara savruldu; demonun kendi kelepçeleri 15 ve 12.
            //
            // Dinamik kalınca servo yine iter — ama temaslar ona karşı koyabilir, ki
            // "topu ötekinin içine sokamamak" bir kısıtlama değil, sahnenin doğrusu.
        }
    }

    if lmb {
        // Sürükleme: ışını salınım düzlemi (z=0) ile kesiştir → fare noktası.
        if let Some(idx) = state.dragging {
            let pivot = state.pivots[idx];
            if rd.z.abs() > 1e-5 {
                let t = (pivot.z - ro.z) / rd.z;
                let p = ro + rd * t;
                // Top fareyi düzlemde TAKİP eder; ip boyunu (L) aşmasın diye mesafe ≤ L kırpılır.
                let from_pivot = p - pivot;
                let dist = from_pivot.length();
                let target = if dist > L {
                    pivot + from_pivot / dist * L
                } else {
                    p
                };

                let id = state.balls[idx];
                // Hedefe SABİT-KAZANÇLI hız servosu ile sür (dt'ye bölme YOK). Sabit ılımlı
                // kazanç dt tutarsızlığına bağışık ve pürüzsüz takip eder. Gövde DİNAMİK
                // olduğu için bu bir *istek*: temas ona karşı koyabilir, ve koymalıdır.
                let cur = world
                    .borrow::<Transform>()
                    .get(id)
                    .map(|t| t.position)
                    .unwrap_or(target);
                const DRAG_GAIN: f32 = 18.0;
                let vel = ((target - cur) * DRAG_GAIN).clamp_length_max(15.0);
                let mut vs = world.borrow_mut::<Velocity>();
                if let Some(mut v) = vs.get_mut(id) {
                    v.linear = vel;
                    v.angular = Vec3::ZERO;
                }
            }
        }
    } else if let Some(idx) = state.dragging.take() {
        // Bırakma: gövde tipi zaten Dynamic (artık kinematiğe hiç geçilmiyor); yalnız servo
        // hızı doğal bir fiske olarak bırakılır, kelepçeyle.
        let id = state.balls[idx];
        let mut vs = world.borrow_mut::<Velocity>();
        if let Some(mut v) = vs.get_mut(id) {
            v.linear = v.linear.clamp_length_max(12.0);
        }
    }

    // ── Görsel ipleri toplara bağla ──────────────────────────────────────────
    // Fizik (schedule'da) bu kareden önce koştu → top konumları güncel. İpi pivot
    // ile topun pivota bakan yüzeyi arasına gerilmiş ince çubuk yap.
    let centers: Vec<Vec3> = {
        let ts = world.borrow::<Transform>();
        state
            .balls
            .iter()
            .map(|&b| ts.get(b).map(|t| t.position).unwrap_or(Vec3::ZERO))
            .collect()
    };
    // ── Girişim ölçeri ───────────────────────────────────────────────────────
    // Her kare, merkezler arası mesafeden gerçek girişimi hesapla. Merkez mesafesi
    // 2R'nin altına inerse toplar birbirinin İÇİNDE demektir — ve bunu x sırasına
    // bakarak ölçmek yanlış olurdu: yeterince hızlı bir top ipin üstünden pivotun
    // tepesini aşar, o sırada x'i komşusunu geçer ama hiç temas yoktur.
    //
    // Yalnız yeni bir rekor kırıldığında yazar: her kare yazmak konsolu boğar ve
    // aranan şey zaten en kötü an. Yanına hızı ve kare süresini de koyar, çünkü
    // şüpheliler onlar.
    {
        let vs = world.borrow::<Velocity>();
        let vels: Vec<Vec3> = state
            .balls
            .iter()
            .map(|&b| vs.get(b).map(|v| v.linear).unwrap_or(Vec3::ZERO))
            .collect();
        let fastest = vels.iter().fold(0.0f32, |a, v| a.max(v.length()));

        // Son kareler, halka tampon. Bir kez olan bir şeyi başsız olarak yeniden kurabilmek
        // için tek satır yetmez: olayın kendisi kadar ÖNCESİ de lazım — hangi hızla girildi,
        // kare süresi sıçradı mı, hangi çift yaklaşıyordu.
        state.history.push_back((_dt, centers.clone(), vels.clone()));
        while state.history.len() > HISTORY {
            state.history.pop_front();
        }

        // **Beklenmedik hız.** Demonun kendi kolları hızı 15 m/s (sürükleme servosu) ve
        // 12 m/s (bırakma) ile kelepçeliyor, doğal sarkaç da 10'u geçmiyor. Canlı koşunun
        // dökümünde bir top tek karede 0,6'dan **26,5 m/s**'ye çıktı — yani bir yerden
        // kelepçelerin üstünde enerji giriyor ve bunun nereden geldiği ölçülmedi. Eşik
        // 20: her kolun üstünde, gürültünün dışında.
        if fastest > 20.0 && !state.dumped_fast {
            state.dumped_fast = true;
            let who = vels.iter().position(|v| v.length() == fastest).unwrap_or(0);
            println!("BEKLENMEDİK HIZ: top {who} → {fastest:.1} m/s (sürükleme kelepçesi 15, bırakma 12)");
            for (k, (fdt, pos, vel)) in state.history.iter().enumerate() {
                println!(
                    "  {k:>3}  {:>5.1} ms  top {who}: ({:>7.2},{:>6.2}) v=({:>6.1},{:>6.1}) |v|={:>5.1}",
                    fdt * 1000.0, pos[who].x, pos[who].y, vel[who].x, vel[who].y, vel[who].length()
                );
            }
        }
        for i in 0..centers.len() {
            for j in (i + 1)..centers.len() {
                let overlap = 2.0 * R - (centers[i] - centers[j]).length();
                // 1 cm'in altı çözücünün normal nefes payı; onu gürültü olarak geç.
                if overlap > 0.01 && overlap > state.worst_overlap + 0.005 {
                    state.worst_overlap = overlap;
                    println!(
                        "girişim {overlap:.3} m (2R={:.2}) · toplar {i}-{j} · en hızlı top \
                         {fastest:>5.1} m/s · kare {:.1} ms{}",
                        2.0 * R,
                        _dt * 1000.0,
                        if overlap >= 2.0 * R { "  ← TAM GEÇİŞ" } else { "" }
                    );
                    // Gerçek bir olay (5 cm üstü) tek satırla anlaşılmaz; öncesini de dök.
                    //
                    // **"Bir kez yaz" yanlıştı ve bunu kullanım gösterdi.** İlk sürümde
                    // döküm tek seferlikti; canlı koşuda ilk olay 6 cm'de yakalandı, sonra
                    // girişim 27 cm'ye tırmandı ve o derin olayların HİÇBİRİ dökülmedi —
                    // yani tam gereken veriyi eleyen şey ölçenin kendisiydi. Artık rekor
                    // her KATLANDIĞINDA yeniden yazar: sığ olay bir kez, derinleşen olay
                    // her kademede, ve konsol yine dolmaz.
                    if overlap > 0.05 && overlap > state.dumped_at * 2.0 {
                        state.dumped_at = overlap;
                        println!("  son {} kare (kare süresi · top {i} ve {j}: konum / hız):", state.history.len());
                        for (k, (fdt, pos, vel)) in state.history.iter().enumerate() {
                            println!(
                                "  {k:>3}  {:>5.1} ms  {}: ({:>7.2},{:>6.2}) v=({:>6.1},{:>6.1})  \
                                 {}: ({:>7.2},{:>6.2}) v=({:>6.1},{:>6.1})  aralık {:>5.3}",
                                fdt * 1000.0,
                                i, pos[i].x, pos[i].y, vel[i].x, vel[i].y,
                                j, pos[j].x, pos[j].y, vel[j].x, vel[j].y,
                                (pos[i] - pos[j]).length()
                            );
                        }
                    }
                }
            }
        }
    }

    let mut ts = world.borrow_mut::<Transform>();
    for (i, &rope) in state.ropes.iter().enumerate() {
        let pivot = state.pivots[i];
        let seg = pivot - centers[i]; // topun merkezinden pivota
        let len = (seg.length() - R).max(0.0); // yüzeyden pivota (topun içine girmesin)
        let dir = seg.normalize_or_zero();
        let surface = centers[i] + dir * R;
        if let Some(mut tr) = ts.get_mut(rope) {
            tr.position = surface + dir * (len * 0.5);
            tr.rotation = Quat::from_rotation_arc(Vec3::Y, dir);
            tr.scale = Vec3::new(0.03, len * 0.5, 0.03);
            tr.update_local_matrix();
        }
    }
}

/// Sahneyi kuruluş anına döndürür: toplar ev pozunda, hızlar sıfır, salınım baştan.
///
/// Gövdeler YERİNDE sıfırlanır — despawn/respawn yok: ip eklemleri topların entity-id'sini
/// tutar, varlıkları yeniden yaratmak eklemleri koparırdı.
fn reset(world: &mut World, state: &mut Cradle) {
    // Sürükleme iptal; gövde tipi zaten aşağıda Dynamic'e döndürülüyor.
    state.dragging = None;
    // Ölçer de sıfırlanır: R'den sonraki rekor, R'den ÖNCEKİ denemenin kalıntısı olmamalı.
    state.worst_overlap = 0.0;
    state.history.clear();
    state.dumped_at = 0.0;
    state.dumped_fast = false;

    {
        let mut ts = world.borrow_mut::<Transform>();
        for (&b, home) in state.balls.iter().zip(state.homes.iter()) {
            if let Some(mut t) = ts.get_mut(b) {
                *t = *home; // `home` matris önbelleğiyle birlikte kuruldu → bayat kalmaz
            }
        }
    }
    {
        let mut vs = world.borrow_mut::<Velocity>();
        for &b in &state.balls {
            if let Some(mut v) = vs.get_mut(b) {
                *v = Velocity::default();
            }
        }
    }
    {
        let mut rbs = world.borrow_mut::<RigidBody>();
        for &b in &state.balls {
            if let Some(mut rb) = rbs.get_mut(b) {
                rb.body_type = BodyType::Dynamic; // tip zaten bu; sıfırlama onu da garantiye alır
                rb.wake_up(); // uyuyan gövde ışınlanmayı yoksayardı (integratör atlıyor)
            }
        }
    }
    // Işınlanma eklemlerin eski yapılandırmada biriktirdiği λ'yı geçersiz kılar; atılmazsa
    // bir sonraki geçiş onları YENİ ip doğrultularında tekrar oynatır → ev pozunda fiske.
    if let Some(mut phys) = world.get_resource_mut::<PhysicsWorld>() {
        for joint in phys.joints.iter_mut() {
            joint.reset_warm_start();
        }
    }
}


// --------------------------------------------------------------- render + main
// `default_render_pass` DOĞRUDAN: SSR/SSGI/volumetric/TAA'yı AÇIK tutar
// (`with_scene_render()` kısayolu bunları kapatırdı). gpu_physics motor-varsayılanı
// zaten None olduğundan render'da state-mutasyonu gerekmez.
fn render(
    world: &mut World,
    _s: &Cradle,
    encoder: &mut gizmo::wgpu::CommandEncoder,
    view: &gizmo::wgpu::TextureView,
    renderer: &mut Renderer,
    _light_time: f32,
) {
    default_render_pass(world, encoder, view, renderer);
}

fn main() {
    App::<Cradle>::new("Gizmo — Newton Sarkacı", 1280, 720)
        .add_plugin(PhysicsPlugin::new())
        .set_setup(setup)
        .set_update(update)
        .set_render(render)
        .run()
        .expect("uygulama çalıştırılamadı");
}
