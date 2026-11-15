//! Karakterler: oyuncu ve köylüler, KayKit'in maceracıları (CC0 — Kay Lousberg,
//! <https://kaylousberg.com>; `games/assets/kaykit/LICENSE.txt`).
//!
//! Her model derili, iskeletli bir glTF: gövde, kollar, bacaklar, baş bir iskelete giydirilmiş;
//! miğfer, şapka, pelerin kemiklere asılı. Dosyalarda pakettekilerden yalnızca bu oyunun
//! kullandığı on üç klip ve karakterin kendi parçaları var — silahlar ve öbür klipler ayıklandı.
//! Modeller ikilinin içine gömülü ([`super::modeller`]).
//!
//! Bir karakter, yürüyen gövdenin (kapsül çarpıştırıcı, motorun karakter denetleyicisi) bir çocuğu:
//! yalnızca `GltfModel` taşıyan bir varlık, motor onu ilk karede dosyadan kuruyor. Her kare
//! [`animate`] gövdenin hızına bakıp klibi seçiyor — durunca `Idle`, yürürken `Walking_A`, koşarken
//! `Running_A`, havada `Jump_Idle` — ve klibin hızını gövdenin hızına uyduruyor, ayaklar yerde
//! kaymasın. Toplamak, konuşmak, kurmak birer kez oynanan klip ([`act`]).
//!
//! Penceresiz sınamada model kurulmuyor (renderer yok); bileşenler yine de yerinde.

use super::*;
use gizmo::core::component::Parent;
use gizmo::renderer::components::AnimationPlayer;
use gizmo::renderer::gltf_model::GltfModel;

pub(super) use super::modeller::KNIGHT;
use super::modeller::VILLAGERS;

/// Modelin boyu, kendi biriminde (ayaktan başın tepesine): KayKit'in iri başlı karakterleri.
const MODEL_HEIGHT: f32 = 2.31;

/// Yürüme ve koşma kliplerinin, modelin bu oyundaki boyunda, ayakları kaydırmadan gittiği hız
/// (m/s, 1,8 m'lik bir karakter için; boy küçüldükçe orantılı).
const WALK_PACE: f32 = 1.5;
const RUN_PACE: f32 = 4.6;

/// Bir gövdeye giydirilmiş karakter; o an bir kez oynanan bir klip varsa adı ve kalan süresi.
#[derive(Clone, Default)]
pub(super) struct Figure {
    action: Option<(&'static str, f32)>,
    /// Boy, metre: kliplerin hızı buna göre.
    height: f32,
}

gizmo::core::impl_component!(Figure);

/// `body`'ye `model`'i giydirir: ayakları gövdenin `feet` metre altında, boyu `height`, yüzü
/// gövdenin önüne (`−Z`; glTF karakterleri `+Z`'ye bakıyor).
pub(super) fn dress(world: &mut World, body: Entity, model: &str, feet: f32, height: f32) -> Entity {
    let figure = world.spawn_bundle((
        Transform::new(Vec3::new(0.0, -feet, 0.0))
            .with_rotation(Quat::from_rotation_y(std::f32::consts::PI))
            .with_scale(Vec3::splat(height / MODEL_HEIGHT)),
        GlobalTransform::default(),
        GltfModel::new(model),
        AnimationPlayer::default(),
        Figure { action: None, height },
    ));
    world.add_child(body, figure);
    figure
}

/// Bir köylünün modeli, kimliğinden.
pub(super) fn villager_model(id: (i32, i32, u8)) -> &'static str {
    VILLAGERS[(hash(id.0, id.1, 600 + id.2 as u32) % VILLAGERS.len() as u32) as usize]
}

/// `body`'nin karakterine bir kez `clip` oynat (toplamak, konuşmak, kurmak).
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

/// Hıza göre klip: (ad, klibin hızı, döngü mü).
pub(super) fn pick(speed: f32, grounded: bool, height: f32) -> (&'static str, f32, bool) {
    let scale = height / 1.8;
    if !grounded {
        ("Jump_Idle", 1.0, true)
    } else if speed < 0.35 {
        ("Idle", 1.0, true)
    } else if speed < 3.0 * scale {
        ("Walking_A", (speed / (WALK_PACE * scale)).clamp(0.6, 1.8), true)
    } else {
        ("Running_A", (speed / (RUN_PACE * scale)).clamp(0.7, 1.6), true)
    }
}

/// Her kare: her karakterin klibi, gövdesinin hızından.
pub(super) fn animate(world: &mut World, dt: f32) {
    let figures: Vec<(Entity, Entity)> = world.query::<(Entity, &Figure, &Parent)>().iter().map(|(e, _, p)| (e, p.0)).collect();
    for (figure, body) in figures {
        let speed = world.get::<Velocity>(body).map_or(0.0, |v| Vec2::new(v.linear.x, v.linear.z).length());
        // Denetleyicisi olmayan bir gövde (arabadaki oyuncu) yerde sayılıyor.
        let grounded = world.get::<CharacterController>(body).is_none_or(|k| k.is_grounded);
        let Some(mut f) = world.query_mut::<&mut Figure>().get_mut(figure).map(|f| f.clone()) else { continue };
        let want = match f.action {
            Some((clip, left)) if left > 0.0 => {
                f.action = Some((clip, left - dt));
                (clip, 1.0, false)
            }
            _ => {
                f.action = None;
                pick(speed, grounded, f.height)
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
        p.play_animation_by_name(clip, 0.2, looping);
        p.speed = rate;
    }
}

/// Sınama için: bu karede `body`'nin karakteri hangi klibi oynamalı.
pub(super) fn wanted_clip(world: &World, body: Entity) -> Option<&'static str> {
    let figure = figure_of(world, body)?;
    let f = world.get::<Figure>(figure)?;
    if let Some((clip, left)) = f.action {
        if left > 0.0 {
            return Some(clip);
        }
    }
    let speed = world.get::<Velocity>(body).map_or(0.0, |v| Vec2::new(v.linear.x, v.linear.z).length());
    let grounded = world.get::<CharacterController>(body).is_none_or(|k| k.is_grounded);
    Some(pick(speed, grounded, f.height).0)
}
