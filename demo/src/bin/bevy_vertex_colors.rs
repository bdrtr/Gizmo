//! # Bevy'nin `3d/vertex_colors` örneğinin Gizmo Engine portu
//!
//! Rengi malzemeden değil **mesh'in kendi köşelerinden** almak. Bevy'de bu `Mesh`'e bir
//! `ATTRIBUTE_COLOR` eklemek ve malzemenin temel rengini beyaz bırakmak demek.
//!
//! ## Motorda köşe rengi hep var — ama onu okuyan tek yol var
//!
//! `Vertex`'in `color` alanı **her zaman** mevcut, çünkü motorun tek bir köşe düzeni var: renk
//! taşımayan bir dosya içe aktarılırken opak beyaza normalleştiriliyor. Yani "köşe rengi eklemek"
//! diye bir adım yok; alan zaten orada.
//!
//! Ama onu **okuyan** tek shader `baked_lit.wgsl`. Ertelenmiş PBR yolu köşe rengine bakmıyor;
//! G-tamponuna yalnız `inst_albedo × doku` yazıyor. Yani köşe renginin görünmesi için malzemenin
//! türü [`MaterialType::BakedLit`] olmalı.
//!
//! Bu, bir önceki portun (`bevy_deferred_rendering`) açık bıraktığı halkayı kapatıyor: orada
//! `BakedLit` ile `Unlit` **aynı pikselleri** üretmişti (222,23'e karşı 222,26), ve sebebi
//! "`BakedLit` ışığı köşe renginden okuyor, o mesh'in köşe rengi yok" diye yazılmıştı. Bu demo o
//! açıklamayı sınıyor: aynı iki tür, bu kez köşe rengi **olan** bir mesh üzerinde.
//!
//! ## `baked_lit` alfayı da okuyor
//!
//! Köşe renginin alfası dolgu değil gerçek bir kanal: `final_alpha = köşe.a × örnek.a × doku.a`.
//! Demo bunu da gösteriyor — üçüncü üçgenin köşeleri saydamlığa iniyor.
//!
//! ## Ölçülen
//!
//! Üç aynı üçgen, üçünün de malzeme albedo'su **beyaz**; tek fark tür ve köşe alfası. Ölçüt her
//! köşedeki **kanal ayrımı** (`maks(R,G,B) − min(R,G,B)`) — renk köşeden geliyorsa bu büyük olur,
//! gelmiyorsa sıfıra iner.
//!
//! Ölçüldü (2026-08-23, kare 240):
//!
//! | tür | sol alt | sağ alt | tepe | ortalama |
//! |-----|---------|---------|------|----------|
//! | `BakedLit` | 57 | 60 | 65 | **60,6** |
//! | `Pbr` | 1 | 1 | 1 | **0,9** |
//! | `BakedLit`, köşe alfası 0,25 | 39 | 37 | 56 | 44,0 |
//!
//! Köşe örneklerinin kendisi de aynı şeyi söylüyor: `BakedLit`'in sol alt köşesi
//! `(210, 177, 153)` — kırmızı baskın, sağ altı `(181, 220, 160)` — yeşil, tepesi
//! `(156, 184, 221)` — mavi. `Pbr`'nin üç köşesi de `(221, 221, 222)`, yani **nötr**.
//!
//! **67 kat fark**, ve bu bir önceki portun açık bıraktığı halkayı kapatıyor:
//! `bevy_deferred_rendering`'de `BakedLit` ile `Unlit` aynı pikselleri üretmişti, gerekçe olarak
//! da "`BakedLit` ışığı köşe renginden okuyor, o mesh'in köşe rengi yok" yazılmıştı. Köşe rengi
//! olan bir mesh üzerinde ikisi artık taban tabana ayrılıyor — açıklama doğruymuş.
//!
//! Üçüncü satır alfanın da gerçekten okunduğunu gösteriyor: aynı renkler, ama arka plana
//! karışmış (ayrım 60,6 → 44,0, ve mutlak parlaklıklar düşmüş).
//!
//! ## Kontroller
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::prelude::*;
use gizmo::renderer::components::MaterialType;
use gizmo::renderer::gpu_types::Vertex;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Üçgenin köşe renkleri — kırmızı, yeşil, mavi.
const CORNERS: [([f32; 3], [f32; 4]); 3] = [
    ([-1.0, -0.9, 0.0], [1.0, 0.15, 0.15, 1.0]),
    ([1.0, -0.9, 0.0], [0.15, 1.0, 0.15, 1.0]),
    ([0.0, 1.1, 0.0], [0.2, 0.35, 1.0, 1.0]),
];

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Bevy Vertex Colors", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;

            // Üç mesh: aynı geometri, farklı köşe renkleri.
            let opaque = Mesh::from_vertices(device, &triangle(1.0), "vertex_colors::opaque");
            let faded = Mesh::from_vertices(device, &triangle(0.25), "vertex_colors::faded");

            // Soldan sağa: BakedLit (köşe rengi okunur), Pbr (okunmaz), BakedLit + saydam köşeler.
            let cases: [(f32, MaterialType, &Mesh); 3] = [
                (-3.0, MaterialType::BakedLit, &opaque),
                (0.0, MaterialType::Pbr, &opaque),
                (3.0, MaterialType::BakedLit, &faded),
            ];
            for (x, material_type, mesh) in cases {
                // Malzemenin albedo'su BEYAZ: ekranda ne görünüyorsa köşelerden geliyor.
                let mut material = Material::new(white.clone()).with_pbr(Vec4::ONE, 0.6, 0.0);
                material.material_type = material_type;
                if matches!(material_type, MaterialType::BakedLit) {
                    // Saydam köşeler ancak saydam kovada görünür.
                    material.is_transparent = true;
                }
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, 0.0, 0.0)),
                    GlobalTransform::default(),
                    mesh.clone(),
                    material,
                    MeshRenderer::new(),
                ));
            }

            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 0.0, -3.0)).with_scale(Vec3::splat(24.0)),
                GlobalTransform::default(),
                AssetManager::create_sprite_quad(device, 1.0, 1.0),
                Material::new(white).with_unlit(Vec4::new(0.10, 0.10, 0.12, 1.0)),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_x(-0.6),
                intensity: 2.4,
                ..Default::default()
            });

            scene.spawn_camera(state, Vec3::new(0.0, 0.0, 7.5), Vec3::ZERO);
        })
        .set_render(|world, _state, encoder, view, renderer, _light_time| {
            renderer.gpu_physics = None;
            renderer.gpu_fluid = None;
            renderer.gpu_particles = None;
            renderer.ssr = None;
            renderer.ssgi = None;
            renderer.bloom_intensity = 0.0;

            gizmo::systems::default_render_pass(world, encoder, view, renderer);
        })
        .set_ui(|_world, _state, ctx| {
            gizmo::egui::Area::new("vc".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(450.0);
                        ui.heading("Köşe renkleri");
                        ui.label("soldan sağa: BakedLit · Pbr · BakedLit (köşe alfası 0,25)");
                        ui.label("üç malzemenin de albedo'su BEYAZ — renk köşelerden geliyor");
                        ui.separator();
                        ui.label("Vertex::color motorun tek köşe düzeninde HEP var:");
                        ui.label("  renksiz bir dosya içe aktarılırken opak beyaza çekiliyor");
                        ui.label("ama onu okuyan tek shader baked_lit.wgsl —");
                        ui.label("  ertelenmiş PBR yolu köşe rengine hiç bakmıyor.");
                        ui.separator();
                        ui.label("köşe alfası dolgu değil: final_alpha = köşe.a × örnek.a × doku.a");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Köşe renkli bir üçgen; `alpha` köşe renginin alfasını ölçekliyor.
fn triangle(alpha: f32) -> Vec<Vertex> {
    CORNERS
        .iter()
        .map(|(position, color)| Vertex {
            position: *position,
            normal: [0.0, 0.0, 1.0],
            color: [color[0], color[1], color[2], color[3] * alpha],
            tex_coords: [0.5, 0.5],
            ..Default::default()
        })
        .collect()
}
