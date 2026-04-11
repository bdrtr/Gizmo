use gizmo::prelude::*;
use gizmo_app::App;

fn main() {
    let app = App::<()>::new("Gizmo Tutorial - Özel Proje", 800, 600)
        .set_setup(|world, renderer| {
            // ECS'ye Nesneleri Ekleyebiliriz
            let entity_id = world.spawn();
            world.add_component(entity_id, EntityName("Ilk Objem".to_string()));
            
            let pos = Transform::new(Vec3::new(0.0, 0.0, -10.0));
            world.add_component(entity_id, pos);

            let mesh = gizmo::renderer::asset::AssetManager::create_cube(&renderer.device);
            world.add_component(entity_id, mesh);

            let mut asset_manager = gizmo::renderer::asset::AssetManager::new();
            let bg = asset_manager.create_white_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout);
            let material = Material::new(bg).with_unlit(Vec4::new(1.0, 0.0, 0.0, 1.0)); // Kirmizi kup
            world.add_component(entity_id, material);
            world.add_component(entity_id, gizmo::renderer::components::MeshRenderer::new());

            println!("Motor baslatildi ve Obje ECS'ye eklendi!");

            // Kamera ECS uzerinde tanimlaniyor
            let cam_id = world.spawn();
            world.add_component(cam_id, Transform::new(Vec3::new(0.0, 2.0, 5.0)));
            world.add_component(cam_id, Camera {
                fov: 60.0_f32.to_radians(), near: 0.1, far: 1000.0,
                yaw: -std::f32::consts::FRAC_PI_2, pitch: -0.2, primary: true
            });
            
            // Dummy State donduruyoruz
            ()
        })
        .set_update(|world, _state, delta_time, input| {
            // Hangi tuslara basildigini gormek icin (Eger calisiyorsa ekrana yazar)
            let keys = input.get_pressed_keys();
            if !keys.is_empty() {
                // println!("Basili tuslarin u32 kodlari: {:?}", keys);
            }

            // 1. "Ilk Objem" etiketli kirmizi kupun ID'sini arayalim
            let mut player_id = None;
            if let Some(names) = world.query_ref::<EntityName>() {
                for (id, name) in names.iter() {
                    if name.0 == "Ilk Objem" {
                        player_id = Some(id);
                    }
                }
            }

            // 2. O kupun Uzaydaki pozisyon guncellemesi (Hareket)
            if let Some(id) = player_id {
                if let Some(mut transforms) = world.query_mut::<Transform>() {
                    for (trans_id, trans) in transforms.iter_mut() {
                        if trans_id == id {
                            let hiz = 20.0 * delta_time; // Daha hizli! Saniyede 20 metre
                            
                            // WASD tuslariyla nesneyi kontrol et!
                            if input.is_key_pressed(KeyCode::KeyW as u32) { trans.position.y += hiz; } // Yukari
                            if input.is_key_pressed(KeyCode::KeyS as u32) { trans.position.y -= hiz; } // Asagi
                            if input.is_key_pressed(KeyCode::KeyA as u32) { trans.position.x -= hiz; } // Sola
                            if input.is_key_pressed(KeyCode::KeyD as u32) { trans.position.x += hiz; } // Saga

                            // COK ONEMLI: Pozisyon degistigi icin GPU'nun anlayacagi Transform Matrisini guncellememiz sart!
                            trans.update_local_matrix();
                            trans.global_matrix = trans.local_matrix();
                        }
                    }
                }
            }
        })
        .set_render(|world, _state, encoder, view, renderer, _light_time| {
            // Bevy'deki DefaultPlugins gibi saniyeler icinde ayarlamalari es gecip
            // sahnedeki objeleri ekrana otomatik basmak icin kullanilan paketimiz:
            default_render_pass(world, encoder, view, renderer);
        });

    app.run();
}
