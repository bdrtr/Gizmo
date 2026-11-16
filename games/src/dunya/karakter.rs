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
//! **Yürüyüşle koşu arasında sert geçiş yok:** model kurulunca bedenin oynatıcısı motorun durum
//! makinesine çevriliyor ([`AnimationStateMachine`]); `move` durumu yürüyüşle koşuyu gövdenin
//! hızına göre karıştırıyor (`BlendSpace`), ikisi aynı adım fazında. Her klip ayrıca kendi adında
//! bir durum (boşta, havada, iş klipleri); geçişler tetikleyiciyle, çeyrek saniyelik karışımla.
//! Yürürken yapılan bir iş üst gövde katmanında (`AnimationLayers`, `spine`'dan yukarısı) oynuyor;
//! dururken bütün beden.
//!
//! Penceresiz sınamada model kurulmuyor (renderer yok); bileşenler ve klip seçimi yine yerinde.

use super::*;
use gizmo::core::component::Parent;
use gizmo::renderer::components::{
    AnimationLayer, AnimationLayers, AnimationPlayer, AnimationState, AnimationStateMachine, AnimationTransition, BoneMask,
};
use gizmo::renderer::BlendSpace;
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
    air: &'static str,
}

impl Gait {
    /// Bu hızda ağır basan: yürüyüşle koşunun tam ortasından sonra koşu.
    fn mostly_running(&self, speed: f32) -> bool {
        speed >= (self.walk_pace + self.run_pace) * 0.5
    }

    /// Bu hızda karışık adımın ayakları kaydırmadan gittiği hız: iki klibin hızı, karışımın ağırlığıyla.
    fn pace(&self, speed: f32) -> f32 {
        let span = self.run_pace - self.walk_pace;
        if span <= f32::EPSILON {
            return self.walk_pace;
        }
        let t = ((speed - self.walk_pace) / span).clamp(0.0, 1.0);
        self.walk_pace + span * t
    }
}

/// Yürüyüşle koşuyu karıştıran durumun adı.
const MOVE: &str = "move";
/// Üst gövde katmanı: yürürken yapılan iş (toplamak, kullanmak) belden yukarıda, bacaklar yürüyor.
const UPPER: &str = "upper";

/// İnsanlar: KayKit'in maceracıları.
pub(super) const PERSON: Gait =
    Gait { idle: &["Idle"], walk: "Walking_A", walk_pace: 1.5, run: "Running_A", run_pace: 4.6, air: "Jump_Idle" };
/// Koyun: dörtnalı yok, kaçarken hızlı yürüyor.
pub(super) const SHEEP: Gait = Gait { idle: &["Idle"], walk: "Walk", walk_pace: 1.1, run: "Walk", run_pace: 1.1, air: "Jump" };
/// Geyik: dururken bazen otluyor, bazen başını kaldırıp bakıyor; kaçarken dörtnala.
pub(super) const DEER: Gait = Gait {
    idle: &["Idle", "Eating", "Idle_2", "Idle_Headlow"],
    walk: "Walk",
    walk_pace: 1.6,
    run: "Gallop",
    run_pace: 6.0,
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
        AnimationLayers::new(vec![AnimationLayer::new(UPPER, BoneMask::below(&["spine"]))]),
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
    let clips = world
        .get::<AnimationStateMachine>(figure)
        .map(|m| m.clips.clone())
        .or_else(|| world.get::<AnimationPlayer>(figure).map(|p| p.animations.clone()));
    let length = match clips {
        Some(c) if c.is_empty() => 1.0,
        Some(c) => match c.iter().find(|a| a.name == clip) {
            Some(a) => a.duration,
            None => return,
        },
        None => return,
    };
    if let Some(mut f) = world.query_mut::<&mut Figure>().get_mut(figure) {
        f.action = Some((clip, length));
    }
    if let Some(mut m) = world.query_mut::<&mut AnimationStateMachine>().get_mut(figure) {
        // Aynı klip yeniden: baştan.
        if m.current_state == clip && m.active_blend.is_none() {
            m.current_time = 0.0;
        }
    }
}

/// Model kurulup klipleri gelince oynatıcının yerine geçen durum makinesi: her klip kendi adında
/// bir durum, `move` yürüyüşle koşunun hıza göre karışımı, her duruma her yerden tetikleyiciyle
/// çeyrek saniyede geçiş.
fn machine_for(gait: &Gait, player: &AnimationPlayer) -> AnimationStateMachine {
    let clips = player.animations.clone();
    let index = |name: &str| clips.iter().position(|c| c.name == name).unwrap_or(0);
    let mut states: Vec<AnimationState> = clips.iter().enumerate().map(|(i, c)| AnimationState::clip(&c.name, i)).collect();
    states.push(AnimationState::blended(
        MOVE,
        BlendSpace::one_d("speed", &[(index(gait.walk), gait.walk_pace), (index(gait.run), gait.run_pace)]),
    ));
    let transitions = states
        .iter()
        .map(|s| AnimationTransition { from: "*".into(), to: s.name.clone(), blend_duration: 0.25, trigger: Some(s.name.clone()), has_exit_time: false })
        .collect();
    let first = gait.idle.first().copied().unwrap_or("Idle");
    AnimationStateMachine::new(first, clips, states, transitions)
}

fn figure_of(world: &World, body: Entity) -> Option<Entity> {
    world.get::<gizmo::core::component::Children>(body)?.0.iter().copied().find(|c| world.get::<Figure>(*c).is_some())
}

/// Dururken sayılan hız, m/s.
const STILL: f32 = 0.35;

/// Hıza göre durum: makinenin durumu ve ağır basan klip. `rests`: kaçıncı duruş.
pub(super) fn pick(gait: &Gait, speed: f32, grounded: bool, rests: usize) -> (&'static str, &'static str) {
    if !grounded {
        (gait.air, gait.air)
    } else if speed < STILL {
        let idle = gait.idle[rests % gait.idle.len()];
        (idle, idle)
    } else if gait.mostly_running(speed) {
        (MOVE, gait.run)
    } else {
        (MOVE, gait.walk)
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
        // Yürürken iş üst gövdede, dururken bütün bedende.
        let walking = grounded && speed >= STILL;
        let (state, upper) = match f.action {
            Some((clip, left)) if left > 0.0 => {
                f.action = Some((clip, left - dt));
                if walking { (pick(f.gait, speed, grounded, f.rests).0, Some(clip)) } else { (clip, None) }
            }
            _ => {
                f.action = None;
                (pick(f.gait, speed, grounded, f.rests).0, None)
            }
        };
        if let Some(mut layers) = world.query_mut::<&mut AnimationLayers>().get_mut(figure) {
            match upper {
                Some(clip) => {
                    layers.play(UPPER, clip, 0.2, false);
                }
                None => {
                    layers.stop(UPPER, 0.2);
                }
            }
        }
        let gait = f.gait;
        if let Some(mut slot) = world.query_mut::<&mut Figure>().get_mut(figure) {
            *slot = f;
        }
        // Model kuruldu, klipleri geldi: oynatıcının yerine makine.
        let built = world.get::<AnimationPlayer>(figure).filter(|p| !p.animations.is_empty()).map(|p| machine_for(gait, p));
        if let Some(machine) = built {
            world.remove_component::<AnimationPlayer>(figure);
            world.add_component(figure, machine);
        }
        let mut machines = world.query_mut::<&mut AnimationStateMachine>();
        // Model daha kurulmadı (ilk kare, ya da penceresiz sınama).
        let Some(mut m) = machines.get_mut(figure) else { continue };
        m.set_parameter("speed", speed);
        // Karışık adım, ayaklar kaymasın diye gövdenin hızında.
        if let Some(moving) = m.states.iter_mut().find(|s| s.name == MOVE) {
            moving.speed = (speed / gait.pace(speed)).clamp(0.6, 3.5);
        }
        let heading = m.active_blend.as_ref().map_or(m.current_state.as_str(), |b| b.to_state.as_str());
        if heading != state {
            m.trigger(state);
        }
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
    Some(pick(f.gait, speed, grounded, f.rests).1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo::renderer::AnimationClip;

    fn player(names: &[&str]) -> AnimationPlayer {
        let clips: Vec<AnimationClip> = names
            .iter()
            .map(|n| AnimationClip { name: (*n).into(), duration: 1.0, translations: vec![], rotations: vec![], scales: vec![] })
            .collect();
        AnimationPlayer { animations: std::sync::Arc::from(clips), ..Default::default() }
    }

    /// Her klip kendi adında bir durum; `move` yürüyüşü yürüme hızına, koşuyu koşu hızına koyuyor;
    /// her duruma tetikleyiciyle geçiliyor; makine boşta başlıyor.
    #[test]
    fn the_machine_blends_walk_and_run_by_speed() {
        let mut m = machine_for(&PERSON, &player(&["Idle", "Walking_A", "Running_A", "Jump_Idle"]));
        assert_eq!(m.current_state, "Idle");
        assert!(["Idle", "Walking_A", "Running_A", "Jump_Idle", MOVE].iter().all(|s| m.find_state(s).is_some()));
        assert!(m.find_transition("Idle", Some(MOVE), false).is_some_and(|t| t.to == MOVE));
        let moving = m.find_state(MOVE).cloned().expect("move");
        let clip = |m: &AnimationStateMachine, speed: f32| {
            let mut m = m.clone();
            m.set_parameter("speed", speed);
            m.state_clips(&moving, 0.0).iter().max_by(|a, b| a.2.total_cmp(&b.2)).map(|c| m.clips[c.0].name.clone())
        };
        assert_eq!(clip(&m, 1.5).as_deref(), Some("Walking_A"));
        assert_eq!(clip(&m, 4.6).as_deref(), Some("Running_A"));
        m.set_parameter("speed", 3.05);
        let both = m.state_clips(&moving, 0.0);
        assert_eq!(both.len(), 2, "half-way, both: {both:?}");
    }

    /// Ayaklar kaymasın: karışık adımın hızı yürüyüşten koşuya doğru, uçlarda sabit; ağır basan klip
    /// ikisinin ortasında değişiyor.
    #[test]
    fn the_pace_follows_the_blend() {
        assert_eq!(PERSON.pace(0.5), 1.5);
        assert_eq!(PERSON.pace(9.0), 4.6);
        assert!((PERSON.pace(3.05) - 3.05).abs() < 1e-5);
        assert_eq!(pick(&PERSON, 2.0, true, 0), (MOVE, "Walking_A"));
        assert_eq!(pick(&PERSON, 4.0, true, 0), (MOVE, "Running_A"));
        assert_eq!(pick(&PERSON, 0.0, true, 0), ("Idle", "Idle"));
        assert_eq!(pick(&PERSON, 2.0, false, 0), ("Jump_Idle", "Jump_Idle"));
        assert_eq!(SHEEP.pace(5.0), 1.1, "one clip at both ends");
    }
}
