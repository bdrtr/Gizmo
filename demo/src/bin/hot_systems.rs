//! # Çalışırken sistem değiştirmek
//!
//! Bir sistemin gövdesini oyun çalışırken değiştirip, yeniden başlatmadan yeni kodu koşturmak.
//! Gizmo bunu `hotpatching` özelliğiyle yapıyor: sistemler, koşullar, `pipe` aşamaları ve
//! gözlemciler [subsecond](https://docs.rs/subsecond)'ın atlama tablosundan çağrılıyor, ve
//! Dioxus CLI değişen kodu derleyip çalışan programa bağlıyor — Bevy'nin kullandığı yol.
//!
//! ```sh
//! cargo install dioxus-cli --version 0.7.10 --locked
//! dx serve --hot-patch -p demo --bin hot_systems --features hotpatching
//! ```
//!
//! Sonra `greet`in döndürdüğü cümleyi değiştirip dosyayı kaydet: bir sonraki satır yeni cümleyi
//! basıyor, sayaç ise kaldığı yerden sürüyor — dünya, varlıklar ve kaynaklar yerinde, değişen
//! yalnız kod.
//!
//! Pencere açmıyor: başsız `App`. Sınırlar `gizmo::core::hotpatch`'te: yalnız debug derlemesi,
//! yalnız `main`'in bulunduğu crate, gövde değişir ama parametreler değişmez.

use gizmo::app::headless::App;
use gizmo::core::system::{In, Res, ResMut, SystemPipe};
use gizmo::core::time::Time;

/// Kaç saniye geçtiği ve en son ne zaman yazıldığı.
#[derive(Default)]
struct Clock {
    seconds: f32,
    printed: u32,
}

/// Değiştirilecek sistem: döndürdüğü cümle her saniye basılıyor.
fn greet(clock: Res<Clock>) -> String {
    format!("merhaba, {} saniye geçti", clock.printed)
}

/// Saniyede bir, `greet`in cümlesini basıyor.
fn print_every_second(In(line): In<String>, mut clock: ResMut<Clock>, time: Res<Time>) {
    clock.seconds += time.dt();
    if clock.seconds >= 1.0 {
        clock.seconds -= 1.0;
        clock.printed += 1;
        println!("{line}");
    }
}

fn main() {
    App::<()>::new("Gizmo - hot systems", 0, 0)
        .insert_resource(Clock::default())
        .set_setup(|_world| ())
        .add_systems(gizmo::core::system::Update, greet.pipe(print_every_second))
        .run()
        .expect("uygulama çalıştırılamadı");
}
