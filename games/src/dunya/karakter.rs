//! Canlıların bedenleri: oyuncu, köylüler, koyunlar, geyikler — derili, iskeletli, animasyonlu
//! glTF modelleri ([`super::modeller`]'de, hepsi CC0).
//!
//! Bir beden, yürüyen gövdenin (kapsül çarpıştırıcı, motorun karakter denetleyicisi) bir çocuğu:
//! yalnızca `GltfModel` taşıyan bir varlık, motor onu ilk karede dosyadan kuruyor. Her kare
//! [`animate`] gövdenin hızına bakıp türün yürüyüş tablosundan ([`Gait`]) klibi seçiyor — dururken
//! boşta (geyik bazen başını eğip otluyor), yavaşken yürüyüş, hızlıyken koşu, havada zıplama — ve
//! klibin hızını gövdenin hızına uyduruyor, ayaklar yerde kaymasın. Toplamak, konuşmak, kurmak
//! birer kez oynanan klip ([`act`]).
//!
//! Penceresiz sınamada model kurulmuyor (renderer yok); bileşenler ve klip seçimi yine yerinde.

use super::*;
use gizmo::core::component::Parent;
use gizmo::renderer::components::AnimationPlayer;
use gizmo::renderer::gltf_model::GltfModel;

pub(super) use super::modeller::KNIGHT;
use super::modeller::VILLAGERS;

/// Bir türün yürüyüşü: hangi klip ne zaman, ve klibin ayakları kaydırmadan gittiği hız (m/s, bu
/// oyundaki boyunda).
pub(super) struct Gait {
    /// Dururken; birden çoksa her duruşta sıradaki.
    idle: &'static [&'static str],
    walk: &'static str,
    walk_pace: f32,
    run: &'static str,
    run_pace: f32,
    /// Bu hızdan sonra koşu.
    run_from: f32,
    air: &'static str,
}

/// İnsanlar: KayKit'in maceracıları.
pub(super) const PERSON: Gait =
    Gait { idle: &["Idle"], walk: "Walking_A", walk_pace: 1.5, run: "Running_A", run_pace: 4.6, run_from: 3.0, air: "Jump_Idle" };
/// Koyun: dörtnalı yok, kaçarken hızlı yürüyor.
pub(super) const SHEEP: Gait = Gait { idle: &["Idle"], walk: "Walk", walk_pace: 1.1, run: "Walk", run_pace: 1.1, run_from: f32::MAX, air: "Jump" };
/// Geyik: dururken bazen otluyor, bazen başını kaldırıp bakıyor; kaçarken dörtnala.
pub(super) const DEER: Gait = Gait {
    idle: &["Idle", "Eating", "Idle_2", "Idle_Headlow"],
    walk: "Walk",
    walk_pace: 1.6,
    run: "Gallop",
    run_pace: 6.0,
    run_from: 3.2,
    air: "Gallop",
};

/// Bir gövdeye giydirilmiş beden; o an bir kez oynanan bir klip varsa adı ve kalan süresi.
#[derive(Clone)]
pub(super) struct Figure {
    action: Option<(&'static str, f32)>,
    gait: &'static Gait,
    /// Kaçıncı duruş: boşta klibi bununla seçiliyor.
    rests: usize,
    /// Son karede duruyor muydu.
    resting: bool,
}

gizmo::core::impl_component!(Figure);

/// `body`'ye `model`'i giydirir: modelin kendi birimindeki boyu `model_height`, oyundaki boyu
/// `height`; ayakları gövdenin `feet` metre altında, yüzü gövdenin önüne (`−Z`; glTF modelleri
/// `+Z`'ye bakıyor).
pub(super) fn dress(world: &mut World, body: Entity, model: (&str, f32), feet: f32, height: f32, gait: &'static Gait) -> Entity {
    let (path, model_height) = model;
    let figure = world.spawn_bundle((
        Transform::new(Vec3::new(0.0, -feet, 0.0))
            .with_rotation(Quat::from_rotation_y(std::f32::consts::PI))
            .with_scale(Vec3::splat(height / model_height)),
        GlobalTransform::default(),
        GltfModel::new(path),
        AnimationPlayer::default(),
        Figure { action: None, gait, rests: (body.id() as usize) % gait.idle.len(), resting: false },
    ));
    world.add_child(body, figure);
    figure
}

/// Bir köylünün modeli, kimliğinden.
pub(super) fn villager_model(id: (i32, i32, u8)) -> (&'static str, f32) {
    VILLAGERS[(hash(id.0, id.1, 600 + id.2 as u32) % VILLAGERS.len() as u32) as usize]
}

/// `body`'nin bedenine bir kez `clip` oynat (toplamak, konuşmak, kurmak).
pub(super) fn act(world: &mut World, body: Entity, clip: &'static str) {
    let Some(figure) = figure_of(world, body) else { return };
    // Klibin boyu; model daha kurulmadıysa (penceresiz sınamada hiç) bir saniye — oynanan iş
    // oyunun durumu, klip gelince oynuyor.
    let length = match world.get::<AnimationPlayer>(figure) {
        Some(p) if p.animations.is_empty() => 1.0,
        Some(p) => match p.animations.iter().find(|a| a.name == clip) {
            Some(a) => a.duration,
            None => return,
        },
        None => return,
    };
    if let Some(mut f) = world.query_mut::<&mut Figure>().get_mut(figure) {
        f.action = Some((clip, length));
    }
    if let Some(mut p) = world.query_mut::<&mut AnimationPlayer>().get_mut(figure) {
        // Aynı klip yeniden: baştan.
        if p.current_clip().is_some_and(|c| c.name == clip) {
            p.current_time = 0.0;
        }
    }
}

fn figure_of(world: &World, body: Entity) -> Option<Entity> {
    world.get::<gizmo::core::component::Children>(body)?.0.iter().copied().find(|c| world.get::<Figure>(*c).is_some())
}

/// Dururken sayılan hız, m/s.
const STILL: f32 = 0.35;

/// Hıza göre klip: (ad, klibin hızı, döngü mü). `rests`: kaçıncı duruş.
pub(super) fn pick(gait: &Gait, speed: f32, grounded: bool, rests: usize) -> (&'static str, f32, bool) {
    if !grounded {
        (gait.air, 1.0, true)
    } else if speed < STILL {
        (gait.idle[rests % gait.idle.len()], 1.0, true)
    } else if speed < gait.run_from {
        (gait.walk, (speed / gait.walk_pace).clamp(0.6, 3.5), true)
    } else {
        (gait.run, (speed / gait.run_pace).clamp(0.7, 1.6), true)
    }
}

fn motion(world: &World, body: Entity) -> (f32, bool) {
    let speed = world.get::<Velocity>(body).map_or(0.0, |v| Vec2::new(v.linear.x, v.linear.z).length());
    // Denetleyicisi olmayan bir gövde (arabadaki oyuncu) yerde sayılıyor.
    let grounded = world.get::<CharacterController>(body).is_none_or(|k| k.is_grounded);
    (speed, grounded)
}

/// Her kare: her bedenin klibi, gövdesinin hızından.
pub(super) fn animate(world: &mut World, dt: f32) {
    let figures: Vec<(Entity, Entity)> = world.query::<(Entity, &Figure, &Parent)>().iter().map(|(e, _, p)| (e, p.0)).collect();
    for (figure, body) in figures {
        let (speed, grounded) = motion(world, body);
        let Some(mut f) = world.query_mut::<&mut Figure>().get_mut(figure).map(|f| f.clone()) else { continue };
        // Yeni bir duruş: sıradaki boşta klibi.
        let resting = grounded && speed < STILL;
        if resting && !f.resting {
            f.rests += 1;
        }
        f.resting = resting;
        let want = match f.action {
            Some((clip, left)) if left > 0.0 => {
                f.action = Some((clip, left - dt));
                (clip, 1.0, false)
            }
            _ => {
                f.action = None;
                pick(f.gait, speed, grounded, f.rests)
            }
        };
        if let Some(mut slot) = world.query_mut::<&mut Figure>().get_mut(figure) {
            *slot = f;
        }
        let mut players = world.query_mut::<&mut AnimationPlayer>();
        let Some(mut p) = players.get_mut(figure) else { continue };
        // Model daha kurulmadı (ilk kare, ya da penceresiz sınama).
        if p.animations.is_empty() {
            continue;
        }
        let (clip, rate, looping) = want;
        p.play_animation_by_name(clip, 0.25, looping);
        p.speed = rate;
    }
}

/// Sınama için: bu karede `body`'nin bedeni hangi klibi oynamalı.
pub(super) fn wanted_clip(world: &World, body: Entity) -> Option<&'static str> {
    let figure = figure_of(world, body)?;
    let f = world.get::<Figure>(figure)?;
    if let Some((clip, left)) = f.action {
        if left > 0.0 {
            return Some(clip);
        }
    }
    let (speed, grounded) = motion(world, body);
    Some(pick(f.gait, speed, grounded, f.rests).0)
}
