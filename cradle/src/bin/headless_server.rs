use gizmo::prelude::*;
use gizmo::app::Plugin;

/// Bu eklenti (Plugin), motorun normal pencereli (Winit+Wgpu) döngüsünü 
/// tamamen ezip yerine penceresiz ve sadece CPU odaklı (Headless) bir döngü kurar.
struct HeadlessServerPlugin;

impl Plugin for HeadlessServerPlugin {
    fn build(&self, app: &mut App) {
        // Motorun Runner'ını değiştiriyoruz. Artık Winit veya Wgpu yüklenmeyecek!
        app.set_runner_mut(|mut app| {
            println!("\n[Sunucu] Gizmo Engine Headless (Penceresiz) Sunucu Başlatıldı!");
            println!("[Sunucu] Render ve Winit devre dışı. Sadece fizik simüle ediliyor...\n");
            
            // Not: PhysicsPlugin önceden eklendiği için PhysicsWorld zaten başlatıldı.

            // Başlangıç objeleri
            let ent = app.world.spawn();
            app.world.add_component(ent, gizmo::physics::components::Transform::new(gizmo::math::Vec3::new(0.0, 10.0, 0.0)));
            app.world.add_component(ent, gizmo::physics::components::Velocity::default());
            app.world.add_component(ent, gizmo::physics::components::RigidBody {
                mass: 1.0,
                ..Default::default()
            });

            let mut tick = 0;
            loop {
                // Sadece fizik ve oyun mantığı güncelleniyor
                gizmo::physics::system::physics_step_system(&mut app.world, 0.016);
                
                tick += 1;
                if tick % 60 == 0 { // Saniyede 1 kez log bas
                    if let Some(trans) = app.world.borrow::<gizmo::physics::components::Transform>().get(ent.id()) {
                        println!("[Simülasyon] Saniye: {} - Obje Y ekseni: {:.2}", tick / 60, trans.position.y);
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
}

fn main() {
    // App'i oluştur, Headless plugin'i tak ve çalıştır.
    App::new("Gizmo Dedicated Server", 0, 0)
        .add_plugin(PhysicsPlugin::default()) // Fizik Plugin'ini ekledik
        .add_plugin(HeadlessServerPlugin)
        .run();
}
