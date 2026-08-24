//! # Sonsuz ızgara
//!
//! Editör benzeri uygulamaların zemini: ufka kadar giden, ölçek duygusu veren bir ızgara.
//!
//! ## Motorda ızgara **var**, ama oyun yolu onu çizmiyor
//!
//! Bu demonun asıl bulduğu şey bu, ve uydurma değil — motorun kendi `routing.rs`'i yazıyor:
//!
//!   * `MaterialType::Grid` diye bir malzeme türü var.
//!   * Bir `grid.wgsl` ve derlenmiş bir `grid_pipeline` var.
//!   * O boru hattını **yalnız `gizmo-studio`** çağırıyor (`batch.is_grid`, editör tercihine
//!     bağlı). Oyun yolunun (`default_render_pass`) o dala hiç kolu yok: `route()` `Grid`'e
//!     `instance_flag: 0.0` veriyor, yani ızgara malzemesi oyun içinde **sıradan bir opak PBR
//!     yüzeyi** olarak çiziliyor.
//!
//! Demo bunu saklamıyor, **yan yana koyuyor**: bir yanda motorun `MaterialType::Grid`
//! malzemesiyle bir zemin (oyun yolunda ne görünüyorsa o), öbür yanda [`Gizmos`] çizgileriyle
//! kurulmuş, kamerayı takip ettiği için ufka kadar gidiyormuş gibi duran çalışan bir ızgara.
//! **G** ikisi arasında geçiş yapıyor, `GIZMO_GRID_ENGINE=1` da başsız koşuda motorunkiyle açar.
//!
//! **Ölçüldü (2026-08-23, kare 200):** motorun malzemesiyle açılan kare düz, gri, **opak bir
//! zemin** — tek bir ızgara çizgisi yok, ve zeminin altındaki küpü de tamamen kapatıyor. Yani
//! `routing.rs`'in "0.0 oyun yolunun yaptığını korur" notu ekranda böyle görünüyor: `Grid`
//! malzemesi oyun içinde ızgara değil, sadece bir düzlem.
//!
//! ## Gizleme: bu bölüm 2026-08-24'e kadar YANLIŞTI
//!
//! Burada "motorda görünürlük bileşeni yok, gizlemenin tek yolu `ShadowCasting::Only`, o da
//! gölgeyi bırakır" yazıyordu. Yanlıştı, ve yazıldığında zaten dört gün eskiydi: `IsHidden`
//! 2026-08-19'da iki çizim yoluna da bağlanmıştı ve **tam bir gizleme**. Ölçüldü — `IsHidden`
//! taşıyan bir küp, o küp sahnede hiç yokmuş gibi bir kare veriyor: **0 piksel** fark.
//! `ShadowCasting::Only` ise gerçekten gölgeyi bırakıyor; ikisi farklı şeyler ve demo artık
//! doğru olanı kullanıyor.
//!
//! `Visibility` diye bir tip var ama o kare başına hesaplanan frustum sonucu, kullanıcı bileşeni
//! değil — o kadarı doğruydu. Kullanıcının bileşeni `gizmo::core::component::IsHidden`, ve
//! 2026-08-24'ten beri **kalıtımlı**: gizlenen bir ebeveyn çocuklarını da gizliyor (öncesinde bir
//! çiftin 2 946 pikselinin 1 886'sı ekranda kalıyordu).
//!
//! ## Kontroller
//!   * **P** — ızgarayı gizle/göster · **G** — motor malzemesi ↔ gizmo çizgileri
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::input::Input;
use gizmo::core::commands::Commands;
use gizmo::core::query::{Query, With};
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Motorun `MaterialType::Grid` malzemesini taşıyan zeminin işareti.
#[derive(Clone, Copy)]
struct EngineGrid;
gizmo::core::impl_component!(EngineGrid);

/// Izgaranın o anki durumu.
#[derive(Clone, Copy)]
struct GridState {
    visible: bool,
    /// `true` → motorun malzemesi, `false` → gizmo çizgileri.
    use_engine_material: bool,
}

impl Default for GridState {
    fn default() -> Self {
        Self {
            visible: true,
            // Başsız bir karede G'ye basacak kimse yok: `GIZMO_GRID_ENGINE=1` motorun
            // malzemesiyle açar, ki oyun yolunun `MaterialType::Grid`'e ne yaptığı
            // ekran görüntüsüyle gösterilebilsin.
            use_engine_material: std::env::var("GIZMO_GRID_ENGINE").is_ok(),
        }
    }
}
gizmo::core::impl_component!(GridState);

/// Çizgi ızgarasının hücre boyu ve kaç hücre uzağa gittiği.
const CELL: f32 = 1.0;
const CELLS: i32 = 40;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Infinite Grid", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );

            // Motorun ızgara malzemesini taşıyan bir zemin. Oyun yolunda ne olduğunu görmek için
            // burada.
            let mut grid_material = Material::new(white.clone());
            grid_material.material_type = gizmo::renderer::components::MaterialType::Grid;
            grid_material.albedo = Vec4::new(0.45, 0.47, 0.52, 1.0);
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO),
                GlobalTransform::default(),
                AssetManager::create_plane(&scene.renderer.device, (CELLS as f32) * CELL * 2.0),
                grid_material,
                MeshRenderer::new(),
                EngineGrid,
            ));

            // İki yarı saydam küp: biri ızgaranın üstünde, biri altında. Alttaki, zeminin
            // arkasından görünüp görünmediğini söyler.
            let cube = AssetManager::create_cube(&scene.renderer.device);
            for y in [2.0, -2.0] {
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(0.0, y, 0.0)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone())
                        .with_pbr(Vec4::new(1.0, 1.0, 1.0, 0.5), 0.4, 0.0),
                    MeshRenderer::new(),
                ));
            }

            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.9) * Quat::from_rotation_x(-0.9),
                intensity: 2.5,
                ..Default::default()
            });

            scene.world.insert_resource(GridState::default());
            scene.world.insert_resource(Gizmos::default());
            scene
                .spawn_camera(state, Vec3::new(-12.5, 5.0, 10.0), Vec3::new(0.0, 0.0, 0.0));
        })
        .add_update_system(toggle_grid.in_phase(Phase::Update).label("toggle_grid"))
        .add_update_system(draw_line_grid.in_phase(Phase::Update).after("toggle_grid"))
        .set_ui(|world, _state, ctx| {
            let Some(grid) = world.get_resource::<GridState>().map(|g| *g) else {
                return;
            };
            gizmo::egui::Area::new("grid".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    ui.heading("Sonsuz ızgara");
                    ui.label(format!(
                        "görünür: {}",
                        if grid.visible { "evet" } else { "hayır" }
                    ));
                    ui.label(format!(
                        "kaynak: {}",
                        if grid.use_engine_material {
                            "motorun MaterialType::Grid malzemesi"
                        } else {
                            "Gizmos çizgileri (kamerayı takip eder)"
                        }
                    ));
                    ui.separator();
                    ui.label("grid_pipeline motorda var — ama onu yalnız editör çağırıyor");
                    ui.label("gizleme = IsHidden (tam gizler, kalıtımlı)");
                    ui.separator();
                    ui.label("P — gizle/göster · G — kaynağı değiştir");
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// **P** görünürlüğü, **G** kaynağı çevirir.
///
/// Gizleme `IsHidden` ile — motorun gerçek gizleme mekanizması bu, ve tam: nesne çizilmiyor ve
/// gölge de düşürmüyor. `ShadowCasting::Only` BAŞKA bir şey ("çizme ama gölgeyi bırak") ve bu
/// demo eskiden gizleme diye onu kullanıyordu.
///
/// `Commands` ile, çünkü bir bileşeni takıp çıkarmak yapısal bir değişiklik ve bir sistem
/// `&mut World` tutamıyor (bkz. `docs/CAPABILITY_GAPS.md` C2).
fn toggle_grid(
    mut grid: ResMut<GridState>,
    planes: Query<(&EngineGrid, With<EngineGrid>)>,
    mut commands: Commands,
    input: Res<Input>,
) {
    use gizmo::winit::keyboard::KeyCode;
    if input.is_key_just_pressed(KeyCode::KeyP as u32) {
        grid.visible = !grid.visible;
    }
    if input.is_key_just_pressed(KeyCode::KeyG as u32) {
        grid.use_engine_material = !grid.use_engine_material;
    }

    let draw_plane = grid.visible && grid.use_engine_material;
    for (entity, _) in planes.iter() {
        // `Entity::new(id, 0)`: sorgular ham `u32` veriyor ve geri çevirmek `&World` istiyor —
        // C1'de yazılı bir boşluk. Burada güvenli, çünkü bu varlık kurulumda doğuruldu ve hiç
        // silinmedi, yani kuşağı sıfır.
        let e = gizmo::core::entity::Entity::new(entity, 0);
        if draw_plane {
            commands.entity(e).remove::<gizmo::core::component::IsHidden>();
        } else {
            commands.entity(e).insert(gizmo::core::component::IsHidden);
        }
    }
}

/// Çalışan ızgara: kameranın altındaki hücreye oturan, o yüzden hep kameranın çevresinde kalan
/// bir çizgi ağı.
///
/// "Sonsuz" olmasının hilesi bu — çizgiler sabit dünyada durmuyor, her kare kameranın hücresine
/// yuvarlanıp yeniden çiziliyor, ve uzaktaki çizgiler solduruluyor.
fn draw_line_grid(
    grid: Res<GridState>,
    cameras: Query<(&Camera, &Transform)>,
    mut gizmos: ResMut<Gizmos>,
) {
    gizmos.clear();
    if !grid.visible || grid.use_engine_material {
        return;
    }

    // Kameranın oturduğu hücrenin merkezi.
    let mut origin = Vec3::ZERO;
    for (_entity, (camera, transform)) in cameras.iter() {
        if camera.primary {
            origin = transform.position;
            break;
        }
    }
    let cx = (origin.x / CELL).round() * CELL;
    let cz = (origin.z / CELL).round() * CELL;
    let span = CELLS as f32 * CELL;

    for i in -CELLS..=CELLS {
        let offset = i as f32 * CELL;
        // Merkezden uzaklaştıkça sönümle: kenarda kesildiği belli olmasın.
        let fade = 1.0 - (offset.abs() / span);
        let alpha = (fade * fade * 0.55).max(0.02);
        // Ana eksenler daha parlak: x/z eksenleri ayırt edilsin.
        let axis = (i == 0) as u32 as f32;
        let color = [
            0.35 + axis * 0.45,
            0.38 + axis * 0.25,
            0.42 + axis * 0.2,
            alpha + axis * 0.35,
        ];
        gizmos.draw_line(
            Vec3::new(cx + offset, 0.0, cz - span),
            Vec3::new(cx + offset, 0.0, cz + span),
            color,
        );
        gizmos.draw_line(
            Vec3::new(cx - span, 0.0, cz + offset),
            Vec3::new(cx + span, 0.0, cz + offset),
            color,
        );
    }
}
