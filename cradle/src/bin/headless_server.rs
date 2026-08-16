use gizmo::prelude::*;

/// Motorun normal pencereli (Winit+Wgpu) döngüsünü tamamen ezip yerine penceresiz, sadece CPU
/// odaklı bir döngü kurar.
///
/// **Neden `Plugin` değil, düz fonksiyon.** `Plugin::build` bir `AppLike` alıyor — dünya ve iki
/// schedule — çünkü ancak o zaman aynı plugin hem pencereli hem headless runtime'a takılabiliyor.
/// Bu kod ise `set_runner_mut` çağırıyor: runtime'ın KENDİSİNİ değiştiriyor, yani bir plugin'in
/// tanımı gereği yapamayacağı şeyi yapıyor. Somut `App`'i alan bir fonksiyon dürüst karşılığı.
fn install_headless_runner(app: &mut App<()>) {
        // Motorun Runner'ını değiştiriyoruz. Artık Winit veya Wgpu yüklenmeyecek!
        app.set_runner_mut(|mut app| {
            println!("\n[Sunucu] Gizmo Engine Headless (Penceresiz) Sunucu Başlatıldı!");
            println!("[Sunucu] Render ve Winit devre dışı. Sadece fizik simüle ediliyor...\n");

            // Not: PhysicsPlugin önceden eklendiği için PhysicsWorld zaten başlatıldı.

            // Başlangıç objeleri
            let ent = app.world.spawn();
            app.world.add_component(
                ent,
                gizmo::physics::components::Transform::new(gizmo::math::Vec3::new(0.0, 10.0, 0.0)),
            );
            app.world
                .add_component(ent, gizmo::physics::components::Velocity::default());
            app.world.add_component(
                ent,
                gizmo::physics::components::RigidBody {
                    mass: 1.0,
                    ..Default::default()
                },
            );

            let mut tick = 0;
            loop {
                // Sadece fizik ve oyun mantığı güncelleniyor
                gizmo::physics::system::physics_step_system(&app.world, 0.016);

                tick += 1;
                if tick % 60 == 0 {
                    // Saniyede 1 kez log bas
                    if let Some(trans) = app
                        .world
                        .borrow::<gizmo::physics::components::Transform>()
                        .get(ent.id())
                    {
                        println!(
                            "[Simülasyon] Saniye: {} - Obje Y ekseni: {:.2}",
                            tick / 60,
                            trans.position.y
                        );
                    }
                }

                // Saniyede 60 tick (16ms) sabitleme
                std::thread::sleep(std::time::Duration::from_millis(16));

                // Demo amaçlı 5 saniye sonra çık
                if tick > 300 {
                    println!("\n[Sunucu] Simülasyon tamamlandı. Çıkılıyor.");
                    break;
                }
            }
        });
}

fn main() {
    // App'i oluştur, fizik plugin'ini tak, headless runner'ı kur ve çalıştır.
    let mut app = App::<()>::new("Gizmo Dedicated Server", 0, 0)
        .add_plugin(PhysicsPlugin::default());
    install_headless_runner(&mut app);
    app.run().expect("uygulama çalıştırılamadı");
}
