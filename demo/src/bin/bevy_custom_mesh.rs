//! # Bevy'nin `3d/generate_custom_mesh` örneğinin Gizmo Engine portu
//!
//! Elle kurulmuş bir küp: köşe konumları, normalleri ve UV'leri kodda yazılı, mesh de onlardan
//! üretiliyor. Space UV eşlemesini değiştiriyor — ve değişiklik **mesh'i yeniden kurmadan**,
//! köşe tamponunun içeriği yerinde güncellenerek uygulanıyor.
//!
//! ## Motorun prosedürel geometri yüzeyi
//!
//! İki çağrı yetiyor: [`Mesh::from_vertices`] bir üçgen listesinden mesh kuruyor,
//! [`Mesh::update_vertices`] aynı tamponu her frame yeniden yazabiliyor (aynı köşe sayısıyla).
//! Bu ikinci çağrı bu demonun asıl konusu: Bevy örneğinde `meshes.get_mut(handle)` ile mesh
//! varlığı değiştiriliyor, Gizmo'da karşılığı GPU tamponuna doğrudan yazmak.
//!
//! **Ve yazmanın YERİ motorun modelinin bir parçası.** Sistemler `wgpu::Queue`'ya erişemez —
//! `RenderContext` render kapanışında var, güncelleme fazında yok. Dolayısıyla demo ikiye
//! ayrılıyor: girdi sistemi yalnız bir "UV değişti" bayrağı koyuyor, köşe tamponu ise
//! `set_render` içinde, kuyruğun elde olduğu yerde yazılıyor. Depodaki `wind_tunnel` de
//! prosedürel şeridini aynı yerde güncelliyor.
//!
//! **Bir fark saklanmıyor: motorun prosedürel yolu indeks tamponu almıyor.** Bevy 24 köşe + 36
//! indeksle küpü kuruyor; burada aynı küp **36 köşeye açılıyor** (`from_vertices` üçgen listesi
//! bekliyor). Aynı geometri, aynı düz gölgeleme — yalnız köşeler paylaşılmıyor.
//!
//! Doku olarak Bevy'nin `textures/array_texture.png`'i yerine motorun kendi
//! [`AssetManager::create_uv_debug_texture`]'ı kullanıldı: depo hiçbir oyun varlığı taşımıyor ve
//! bu desen zaten "UV nereye düşüyor" sorusunu göstermek için var.
//!
//!
//! **Bir motor tuzağı, ve Bevy'den gelen biri için görünmez:** `add_system` sistemi **sabit
//! adımlı** çizelgeye kaydeder — kare başına sıfır ya da birkaç kez koşar. Girdi kenarları
//! (`is_key_just_pressed`) ise kare başına bir kez yakalanıp temizlenir, yani o çizelgede
//! okunursa basışlar kaçar ya da iki kez işlenir. Girdiye bakan her sistem bu yüzden
//! `add_update_system` ile kaydediliyor (kare başına tam bir kez, gerçek `dt` ile).
//! ## Kontroller
//!   * **Space** — UV eşlemesini değiştir (tüm yüz ↔ dokunun sol üst çeyreği)
//!   * **X / Y / Z** — o eksende döndür · **R** — duruşu sıfırla
//!   * **Sağ-tık + fare** — bak · **WASDQE** — kamera

use gizmo::core::input::Input;
use gizmo::core::query::{Mut, Query, With};
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::renderer::gpu_types::Vertex;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Bevy'nin `CustomUV` işaret bileşeni.
#[derive(Clone, Copy)]
struct CustomUv;
gizmo::core::impl_component!(CustomUv);

/// Space'in hangi UV setinde olduğumuzu ve tamponun yeniden yazılması gerekip gerekmediğini
/// tutan kaynak.
#[derive(Default, Clone, Copy)]
struct UvToggle {
    zoomed: bool,
    dirty: bool,
}
gizmo::core::impl_component!(UvToggle);

/// Bevy'nin `time.delta_secs() / 1.2` dönüş hızı.
const TURN_RATE: f32 = 1.0 / 1.2;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Bevy Custom Mesh", 1280, 720)
        .with_simple_scene(|scene, state| {
            let texture = scene.asset_manager.create_uv_debug_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );

            let mesh = Mesh::from_vertices(
                &scene.renderer.device,
                &cube_vertices(false),
                "bevy_custom_mesh::cube",
            );

            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO),
                GlobalTransform::default(),
                mesh,
                Material::new(texture).with_pbr(Vec4::new(1.0, 1.0, 1.0, 1.0), 0.6, 0.0),
                MeshRenderer::new(),
                CustomUv,
            ));

            scene.world.insert_resource(UvToggle::default());

            // Bevy'de kamera ve ışık aynı dönüşümü paylaşıyor: (1.8, 1.8, 1.8)'den orijine.
            scene.world.spawn_bundle(PointLightBundle {
                position: Vec3::splat(1.8),
                intensity: 12.0,
                radius: 20.0,
                ..Default::default()
            });
            scene.spawn_camera(state, Vec3::splat(1.8), Vec3::ZERO);
        })
        .add_update_system(input_handler.in_phase(Phase::Update))
        // Köşe tamponunun yazıldığı yer: kuyruk yalnız burada var.
        .set_render(|world, _state, encoder, view, renderer, _light_time| {
            // Basit sahnenin kapattığı geçişler — aynı görüntü için aynen kapatılıyor.
            renderer.gpu_physics = None;
            renderer.gpu_fluid = None;
            renderer.gpu_particles = None;
            renderer.ssr = None;
            renderer.ssgi = None;

            let pending = world
                .get_resource::<UvToggle>()
                .filter(|t| t.dirty)
                .map(|t| t.zoomed);
            if let Some(zoomed) = pending {
                let vertices = cube_vertices(zoomed);
                let meshes = world.borrow::<Mesh>();
                for (_id, mesh) in meshes.iter() {
                    mesh.update_vertices(&renderer.queue, &vertices);
                }
                drop(meshes);
                if let Some(mut toggle) = world.get_resource_mut::<UvToggle>() {
                    toggle.dirty = false;
                }
            }

            gizmo::systems::default_render_pass(world, encoder, view, renderer);
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Space UV setini değiştirir (bayrağı koyar), X/Y/Z döndürür, R sıfırlar.
fn input_handler(
    mut cubes: Query<(Mut<Transform>, With<CustomUv>)>,
    mut toggle: ResMut<UvToggle>,
    input: Res<Input>,
    time: Res<Time>,
) {
    use gizmo::winit::keyboard::KeyCode;
    let key = |k: KeyCode| input.is_key_pressed(k as u32);
    let dt = time.dt();

    if input.is_key_just_pressed(KeyCode::Space as u32) {
        toggle.zoomed = !toggle.zoomed;
        // Mesh'i buradan yazamayız (kuyruk yok) — render kapanışına haber bırakılıyor.
        toggle.dirty = true;
    }

    for (_entity, (mut transform, _)) in cubes.iter_mut() {
        if key(KeyCode::KeyX) {
            transform.rotation *= Quat::from_rotation_x(TURN_RATE * dt);
        }
        if key(KeyCode::KeyY) {
            transform.rotation *= Quat::from_rotation_y(TURN_RATE * dt);
        }
        if key(KeyCode::KeyZ) {
            transform.rotation *= Quat::from_rotation_z(TURN_RATE * dt);
        }
        if key(KeyCode::KeyR) {
            transform.rotation = Quat::IDENTITY;
        }
    }
}

/// Altı yüzü elle yazılmış küp — Bevy'nin 24 köşe + 36 indeksi, üçgen listesine açılmış hâli.
///
/// `zoomed` UV setini değiştirir: `false` her yüze dokunun tamamını, `true` sol üst çeyreğini
/// serer. Bevy'nin `toggle_texture`'ı da tam olarak bunu yapıyor (orada dokunun iki farklı
/// yatay bandı arasında geçiliyor).
fn cube_vertices(zoomed: bool) -> Vec<Vertex> {
    // Yüz başına: dört köşe (sırayla sol-alt, sağ-alt, sağ-üst, sol-üst) ve yüzün normali.
    const FACES: [([[f32; 3]; 4], [f32; 3]); 6] = [
        // üst (+y)
        ([[-0.5, 0.5, 0.5], [0.5, 0.5, 0.5], [0.5, 0.5, -0.5], [-0.5, 0.5, -0.5]], [0.0, 1.0, 0.0]),
        // alt (−y)
        ([[-0.5, -0.5, -0.5], [0.5, -0.5, -0.5], [0.5, -0.5, 0.5], [-0.5, -0.5, 0.5]], [0.0, -1.0, 0.0]),
        // sağ (+x)
        ([[0.5, -0.5, 0.5], [0.5, -0.5, -0.5], [0.5, 0.5, -0.5], [0.5, 0.5, 0.5]], [1.0, 0.0, 0.0]),
        // sol (−x)
        ([[-0.5, -0.5, -0.5], [-0.5, -0.5, 0.5], [-0.5, 0.5, 0.5], [-0.5, 0.5, -0.5]], [-1.0, 0.0, 0.0]),
        // arka (+z)
        ([[-0.5, -0.5, 0.5], [0.5, -0.5, 0.5], [0.5, 0.5, 0.5], [-0.5, 0.5, 0.5]], [0.0, 0.0, 1.0]),
        // ön (−z)
        ([[0.5, -0.5, -0.5], [-0.5, -0.5, -0.5], [-0.5, 0.5, -0.5], [0.5, 0.5, -0.5]], [0.0, 0.0, -1.0]),
    ];

    // UV'nin köşelere dağılımı — `zoomed` yalnız kapladığı alanı değiştirir.
    let hi = if zoomed { 0.5 } else { 1.0 };
    let uv = [[0.0, hi], [hi, hi], [hi, 0.0], [0.0, 0.0]];

    let mut vertices = Vec::with_capacity(36);
    for (corners, normal) in FACES {
        // İki üçgen: 0-1-2 ve 0-2-3.
        for &i in &[0usize, 1, 2, 0, 2, 3] {
            vertices.push(Vertex {
                position: corners[i],
                normal,
                tex_coords: uv[i],
                ..Default::default()
            });
        }
    }
    vertices
}
