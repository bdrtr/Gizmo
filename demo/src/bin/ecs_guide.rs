//! # ECS'e giriş turu
//!
//! ECS'in tam turu: oyuncular (varlık + bileşen), oyun durumu ve kuralları (kaynak),
//! her turda koşan sistemler, sistem kümeleriyle sıralama, ertelenmiş yapısal değişiklik
//! (`Commands`) ve dışlayıcı bir sistem. On tur oynanıp bir kazanan çıkıyor.
//!
//! **Bu, seri içinde penceresiz koşan ilk demo.** Motorun konsolda koşan çalışma zamanı
//! `gizmo::app::headless::App` — GPU'suz, pencere açmayan, her zaman derlenen ikinci çalışma
//! zamanı. Çıktı ekranda değil, terminalde.
//!
//! ## Motorda olmayan üç şey — üçü de burada elle yazıldı
//!
//! **1. ~~Sisteme ait yerel durum yok.~~ 2026-08-24'te geldi: [`Local<T>`].** Bu satır eskiden
//! "yok" diyordu ve demo sayacını sıradan bir kaynakta ([`RoundLog`]) tutuyordu. Fark yalnız üslup
//! değildi: kaynak dünyada durur, yani başka her sistem onu okuyabilir **ve çakışma çizelgeye
//! görünür** — kendi sayacını `ResMut<T>` içinde tutan iki sistem aynı türe yazdıklarını beyan
//! eder, o yüzden zamanlayıcı onları ayırır ve asla paralel koşmazlar. `Local` hiçbir şey beyan
//! etmiyor: ölçüldü, `Local<u32>` tutan iki sistem **tek** partide, aynı ikisi kaynakla **iki**
//! partide. Demo artık `Local` kullanıyor; `RoundLog` gitti.
//!
//! **2. `exclusive()` bir yetki değil, bariyer.** Dışlayıcı bir sistemin `&mut World` alıp dünyayı
//! doğrudan değiştirmesi beklenebilir; Gizmo'nun [`SystemConfig::exclusive`]'i — kendi belgesinin
//! sözleriyle — *"bir eşzamanlılık bariyeridir, değiştirilebilirlik yükseltmesi değil"*: sistem
//! yine `&World` alır, yapısal değişiklik yine [`Commands`]'tan geçer. Kazanılan tek şey,
//! sistemin kendi partisinde yalnız koşması. Demodaki `exclusive_player_system` bu yüzden işini
//! `Commands` ile yapıyor.
//!
//! **3. Çıkış iletisi yok.** Oyun bitince yazılıp döngüyü kapatan bir ileti motorda yok.
//! Motorun başsız döngüsü kendiliğinden dönmez; bu yüzden demo [`App::set_runner`] ile döngünün
//! sahipliğini alıyor ve turlar bitince çıkıyor. Sunucular için zaten önerilen kapı bu.
//!
//! ## Dördüncü tuzak, bu demo yazılırken bulundu: `set_runner` kurulumu yutuyor
//!
//! `App::run`, bir koşucu atanmışsa doğrudan ona devrediyor — yani **`set_setup` kancası hiç
//! çağrılmıyor**. İkisini birlikte yazan bir program sessizce kaynaksız başlar ve ilk sistemin
//! "kaynak Dünya'da bulunamadı" panikiyle ölür. Bu demo kurulumu bu yüzden serbest bir
//! [`setup`] fonksiyonunda tutuyor ve koşucunun ilk satırında çağırıyor.
//!
//! ## Sistem kümelerinin tuzağı (motorun kendi belgesinde yazılı)
//!
//! Üç küme — `BeforeRound → Round → AfterRound` — [`SetConfig`] ile sıraya diziliyor. Ama
//! `in_set` küme adını **sıradan bir etiket olarak da** kaydediyor, ve kümenin `before`/`after`
//! listeleri üyeleri birbirinden ayıklamadan her üyeye ekleniyor. Sonuç: aynı kümedeki iki sistem
//! birbirine karşı sıralanabilir, ve simetrik bir kurulum (`A.after::<B>()` + iki sisteme de ikisi)
//! çizelge kurulurken **panikletir**. Bu yüzden burada zincir tek yönlü: her küme yalnız bir
//! öncekini `after` ile adlandırıyor.
//!
//! ## Çalıştırma
//!
//! ```text
//! cargo run --release -p demo --bin ecs_guide
//! ```

use gizmo::app::headless::App;
use gizmo::core::commands::Commands;
use gizmo::core::query::{Mut, Query};
use gizmo::core::system::{IntoSystemConfig, Local, Res, ResMut, SetConfig, SystemSet};

// ── BİLEŞENLER ───────────────────────────────────────────────────────────────────────────────

/// Bir oyuncunun adı.
#[derive(Clone)]
struct Player {
    name: String,
}
gizmo::core::impl_component!(Player);

/// Oyuncunun puanı.
#[derive(Clone, Copy)]
struct Score {
    value: u32,
}
gizmo::core::impl_component!(Score);

/// Kaç turdur üst üste puan aldı ya da alamadı. Enum da bileşen olabilir.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PlayerStreak {
    Hot(u32),
    Cold(u32),
    None,
}
gizmo::core::impl_component!(PlayerStreak);

impl std::fmt::Display for PlayerStreak {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlayerStreak::Hot(n) => write!(f, "{n} turdur puan alıyor"),
            PlayerStreak::Cold(n) => write!(f, "{n} turdur puan alamıyor"),
            PlayerStreak::None => write!(f, "daha yeni"),
        }
    }
}

// ── KAYNAKLAR ────────────────────────────────────────────────────────────────────────────────

/// Oyunun o anki durumu.
#[derive(Default, Clone)]
struct GameState {
    current_round: u32,
    total_players: u32,
    winning_player: Option<String>,
}
gizmo::core::impl_component!(GameState);

/// Oyunun kuralları — kurulumda yazılıp bir daha değişmiyor.
#[derive(Clone, Copy)]
struct GameRules {
    winning_score: u32,
    max_rounds: u32,
    max_players: u32,
}
gizmo::core::impl_component!(GameRules);

/// Belirleyici "zar" — `rand` yerine, çıktının her koşuda aynı olması için.
#[derive(Clone, Copy)]
struct Dice(u32);
gizmo::core::impl_component!(Dice);

impl Dice {
    /// Xorshift'in bir adımı, 0..n aralığında.
    fn roll(&mut self, n: u32) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 % n.max(1)
    }
}

// ── SİSTEM KÜMELERİ ──────────────────────────────────────────────────────────────────────────

/// Turdan önce koşanlar: yeni tur, yeni oyuncu.
struct BeforeRound;
impl SystemSet for BeforeRound {
    fn set_name() -> &'static str {
        "before_round"
    }
}

/// Turun kendisi: puanlama.
struct Round;
impl SystemSet for Round {
    fn set_name() -> &'static str {
        "round"
    }
}

/// Turdan sonra: kazanan var mı, ve tur kaydı.
struct AfterRound;
impl SystemSet for AfterRound {
    fn set_name() -> &'static str {
        "after_round"
    }
}

// ── SİSTEMLER ────────────────────────────────────────────────────────────────────────────────

/// En basit sistem: hiçbir şey almıyor, sadece koşuyor.
fn print_message_system() {
    println!("bu oyun eğlenceli!");
}

/// Kaynak okuyup yazan sistem: turu ilerletir.
fn new_round_system(rules: Res<GameRules>, mut state: ResMut<GameState>) {
    state.current_round += 1;
    println!(
        "\n── tur {} / {} başlıyor",
        state.current_round, rules.max_rounds
    );
}

/// Her oyuncunun puanını ve serisini günceller.
fn score_system(mut players: Query<(&Player, Mut<Score>, Mut<PlayerStreak>)>, mut dice: ResMut<Dice>) {
    for (_entity, (player, mut score, mut streak)) in players.iter_mut() {
        let scored = dice.roll(2) == 0;
        if scored {
            score.value += 1;
            *streak = match *streak {
                PlayerStreak::Hot(n) => PlayerStreak::Hot(n + 1),
                _ => PlayerStreak::Hot(1),
            };
        } else {
            *streak = match *streak {
                PlayerStreak::Cold(n) => PlayerStreak::Cold(n + 1),
                _ => PlayerStreak::Cold(1),
            };
        }
        println!(
            "  {} {} — puan {} ({})",
            player.name,
            if scored { "puan aldı" } else { "kaçırdı  " },
            score.value,
            *streak
        );
    }
}

/// Kazanan çıktı mı: bileşen okuyup kaynağa yazar.
fn score_check_system(
    rules: Res<GameRules>,
    mut state: ResMut<GameState>,
    players: Query<(&Player, &Score)>,
) {
    for (_entity, (player, score)) in players.iter() {
        if score.value == rules.winning_score {
            state.winning_player = Some(player.name.clone());
        }
    }
}

/// Tur sonu kaydı — sayaç sistemin KENDİSİNDE, dünyada değil.
///
/// `Local<u32>` sistemin kendi değeri: dünyada görünmüyor, `insert_resource` istemiyor, ve
/// zamanlayıcıya hiçbir erişim beyan etmediği için bu sistemi kimseden ayırmıyor. Aynı fonksiyon
/// iki kez kaydedilseydi iki ayrı sayaç olurdu — "sistem" derken kastedilen KURULMUŞ sistem.
fn print_at_end_round(mut times_printed: Local<u32>) {
    *times_printed += 1;
    println!("  'son' kümesinde {}. kez", *times_printed);
}

/// Ertelenmiş yapısal değişiklik: yeni oyuncuyu [`Commands`] kuyruğa alır, çizelge partiden
/// sonra uygular. Sistemler paralel koştuğu için dünyayı doğrudan değiştiremezler.
fn new_player_system(
    mut commands: Commands,
    rules: Res<GameRules>,
    mut state: ResMut<GameState>,
    mut dice: ResMut<Dice>,
) {
    if dice.roll(3) == 0 && state.total_players < rules.max_players {
        state.total_players += 1;
        let number = state.total_players;
        commands
            .spawn()
            .insert(Player {
                name: format!("Oyuncu {number}"),
            })
            .insert(Score { value: 0 })
            .insert(PlayerStreak::None);
        println!("  Oyuncu {number} oyuna katıldı!");
    }
}

/// Dışlayıcı sistem.
///
/// `.exclusive()` bu sistemi kendi partisinde yalnız bırakır ama `&mut World` **vermez** — o
/// yüzden iş yine `Commands` üzerinden yapılıyor. Sonuç aynıdır, sadece dünyaya erişme yolu
/// farklıdır.
fn exclusive_player_system(
    mut commands: Commands,
    rules: Res<GameRules>,
    mut state: ResMut<GameState>,
    mut dice: ResMut<Dice>,
) {
    if dice.roll(4) == 0 && state.total_players < rules.max_players {
        state.total_players += 1;
        let number = state.total_players;
        commands
            .spawn()
            .insert(Player {
                name: format!("Oyuncu {number}"),
            })
            .insert(Score { value: 0 })
            .insert(PlayerStreak::None);
        println!("  Oyuncu {number} (dışlayıcı sistemle) katıldı!");
    }
}

/// Kurulum sistemi.
///
/// **Serbest fonksiyon, `set_setup` değil** — ve bunun bir gerekçesi var: `set_runner`
/// kullanıldığında `App::run` doğrudan koşucuya devrediyor ve **kurulum kancasını hiç
/// çağırmıyor**. `set_setup` + `set_runner` birlikte yazılırsa kurulum sessizce düşer; ilk
/// belirti, ilk sistemin "kaynak bulunamadı" diye paniklemesi olur.
fn setup(world: &mut gizmo::core::World) {
    world.insert_resource(GameState {
        current_round: 0,
        total_players: 2,
        winning_player: None,
    });
    world.insert_resource(GameRules {
        winning_score: 4,
        max_rounds: 10,
        max_players: 4,
    });
    world.insert_resource(Dice(0x2545_F491));

    for name in ["Ayşe", "Bora"] {
        world.spawn_bundle((
            Player {
                name: name.to_string(),
            },
            Score { value: 0 },
            PlayerStreak::None,
        ));
    }
}

fn main() {
    let app = App::<()>::new("Gizmo Engine - ECS Guide", 0, 0)
        // Kümelerin sırası: her biri yalnız bir öncekini adlandırıyor. Simetrik yazmak
        // (ikisine de hem before hem after) çizelge kurulurken panik demek.
        .configure_set(SetConfig::new::<Round>().after::<BeforeRound>())
        .configure_set(SetConfig::new::<AfterRound>().after::<Round>())
        // Turdan önce. `new_round_system` ile `new_player_system` arasındaki sıra da etiketle.
        .add_system(new_round_system.in_set::<BeforeRound>().label("new_round"))
        .add_system(
            new_player_system
                .in_set::<BeforeRound>()
                .after("new_round"),
        )
        .add_system(
            exclusive_player_system
                .in_set::<BeforeRound>()
                .after("new_round")
                .exclusive(),
        )
        // Turun kendisi.
        .add_system(print_message_system.in_set::<Round>().label("print_message"))
        .add_system(score_system.in_set::<Round>().after("print_message"))
        // Turdan sonra.
        .add_system(score_check_system.in_set::<AfterRound>().label("score_check"))
        .add_system(print_at_end_round.in_set::<AfterRound>().after("score_check"))
        // Motorda çıkış iletisi yok: turları saymak ve durmak koşucunun işi.
        .set_runner(|mut app| {
            // `schedule` sabit adımlı: bir `run` çağrısı biriktiriciyi boşalttığı için 0..N
            // tur koşturabilir. Bu yüzden döngü turu değil, DURUMU izliyor — tur ilerlemediyse
            // yeniden dener. `TICK_CAP` yalnız emniyet supabı: takılırsa sessizce dönmesin.
            setup(&mut app.world);

            const TICK_CAP: u32 = 10_000;
            let mut ticks = 0;
            loop {
                app.schedule.run(&mut app.world, 1.0 / 60.0);
                ticks += 1;

                let Some((round, winner)) = app
                    .world
                    .get_resource::<GameState>()
                    .map(|s| (s.current_round, s.winning_player.clone()))
                else {
                    break;
                };
                let Some(max_rounds) = app.world.get_resource::<GameRules>().map(|r| r.max_rounds)
                else {
                    break;
                };

                if let Some(winner) = winner {
                    println!("\n{winner} oyunu kazandı!");
                    break;
                }
                if round >= max_rounds {
                    println!("\nturlar bitti. kazanan yok!");
                    break;
                }
                if ticks >= TICK_CAP {
                    println!("\n{TICK_CAP} tik doldu, tur {round}'de duruldu — çizelge ilerlemiyor");
                    break;
                }
            }
        });

    app.run().expect("uygulama çalıştırılamadı");
}
