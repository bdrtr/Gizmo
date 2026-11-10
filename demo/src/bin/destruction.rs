//! # Voronoi Yıkım Demosu — bir binayı Voronoi parçalarına böl (temiz sürüm)
//!
//! **SPACE** ile bir binayı 100 asimetrik Voronoi konveks-parçasına böl; **Sol tık** ile
//! ağır bir obüs fırlat. **WASD/QE** ile uç, **sağ-tık + fare** ile bak.
//!
//! Bu demo motorun modern idiomlarını NEREDE gerçekten uyduğu yerde kullanır — ama dürüst
//! olalım, hangi idiom NEDEN uymuyor da anlatalım:
//!   * **Voronoi parça meshleri = veri (`MeshData`).** `voronoi_shatter` her çağrıda benzersiz
//!     konveks gövdeler üretir; parçalar update'te üretilip köşeleriyle birlikte doğrudan
//!     `spawn_bundle` ile kurulur. GPU vertex-buffer'larını aynı kare motorun yükleyicisi kuruyor
//!     (dünyanın `AssetManager`'ıyla — setup onu bu yüzden kaynak olarak bırakıyor); ne render
//!     kancası ne bekleme tamponu var. Kare oyun dünyasının kopyasından render thread'inde
//!     çiziliyor (çıplak `RenderWorld`).
//!   * **Obüs = `spawn_bundle` + explicit `Collider::sphere`.** Mesh + materyal setup'ta BİR KEZ
//!     kurulur (her atışta yeniden üretilmez), her atış doğrudan `update`'te spawn'lanır —
//!     eski sürümdeki `pending_balls` + `Commands` + `RefCell` üçlüsü tamamen gitti.
//!   * **`is_key_just_*`** — tek-seferlik parçalama kenar-tespiti motordan (elle prev_* yok).
//!   * **`DespawnAfter` / `DespawnBelowY` BİLEREK YOK.** Bu demo `PhysicsPlugin` EKLEMEZ; yani
//!     fizik-adımı da ömür-temizliği de schedule'a kayıtlı değil — parçalar/obüsler statik
//!     Voronoi kırık geometrisini gösterir. Ömür komponentleri eklemek ya ölü kalır ya da
//!     `LifetimePlugin` ile davranışı değiştirir; ikisi de istenmez.
//!   * **`gpu_physics`** motora hiç dokunulmaz (varsayılanı zaten `None`).

use gizmo::physics::fracture::{voronoi_shatter, ProceduralChunk};
use gizmo::prelude::*;
use gizmo::renderer::gpu_types::Vertex;
use std::f32::consts::FRAC_PI_3;

// ------------------------------------------------------------------ ayarlar
const BALL_R: f32 = 2.0;
const BALL_MASS: f32 = 1000.0; // çok ağır obüs
const BALL_SPEED: f32 = 100.0;
const CHUNK_COUNT: u32 = 100;
const SHATTER_SEED: u64 = 12345;
const BUILD_EXTENTS: Vec3 = Vec3::new(15.0, 30.0, 15.0);
const BUILD_CENTER: Vec3 = Vec3::new(0.0, 25.0, 0.0);

// --------------------------------------------------------------- durum
struct DestructionGame {
    // kamera
    cam_id: Entity,
    cam_yaw: f32,
    cam_pitch: f32,
    cam_pos: Vec3,
    cam_speed: f32,
    // varlık blueprint'leri (setup'ta BİR KEZ kurulur)
    chunk_mat: Material,
    ball_mesh: Mesh,
    ball_mat: Material,
    // parçalama
    shattered: bool,
}

/// Yaw/pitch'ten bu demonun kamera dönüşü — motorun `Camera::forward_from`'undan FARKLI bir
/// konvansiyon (yaw+X yerine yaw ekseni). Görüş+hareket+atış yönü hepsi bunu paylaşır, o yüzden
/// davranışı korumak için olduğu gibi bırakıyoruz.
fn pitch_yaw_quat(pitch: f32, yaw: f32) -> Quat {
    let q_yaw = Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), yaw);
    let q_pitch = Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), pitch);
    q_yaw * q_pitch
}

// --------------------------------------------------------------- setup
fn setup(world: &mut World, renderer: &mut Renderer) -> DestructionGame {
    println!("##################################################");
    println!("    Voronoi Yıkım Demosu Başlıyor...");
    println!("    Binayı parçalamak için SPACE (BOŞLUK) tuşuna bas!");
    println!("##################################################");

    let mut assets = AssetManager::new();
    let white = assets.create_white_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );

    // Voronoi parça materyali (gri PBR) — tüm parçalar paylaşır.
    let chunk_mat = Material::new(white.clone()).with_pbr(Vec4::new(0.6, 0.6, 0.6, 1.0), 0.5, 0.0);
    // Obüs: küre mesh + kırmızı unlit materyal, BİR KEZ; her atış bunu klonlar.
    let ball_mesh = AssetManager::create_sphere(&renderer.device, BALL_R, 16, 16);
    let ball_mat = Material::new(white).with_unlit(Color::RED.to_vec4());
    // Dünyanın `AssetManager`'ı: parçaların `MeshData`'sını motorun yükleyicisi onunla GPU'ya
    // koyuyor — kaynak yoksa yükleyici hiç çalışmıyor.
    world.insert_resource(assets);

    // Kamera (WASD ile uçan; görüş yönü Camera bileşeninin yaw/pitch'inden okunur).
    let cam_yaw = 0.0_f32;
    let cam_pitch = -0.2_f32;
    let cam_pos = Vec3::new(0.0, 50.0, 150.0);
    let cam = world.spawn_bundle((
        Transform::new(cam_pos).with_rotation(pitch_yaw_quat(cam_pitch, cam_yaw)),
        Camera::new(FRAC_PI_3, 0.1, 500.0, cam_yaw, cam_pitch, true),
        EntityName("Kamera".into()),
    ));

    // Güneş
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, 500.0, 0.0))
            .with_rotation(Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), -1.0)),
        DirectionalLight::new(Vec3::new(1.0, 1.0, 0.95), 3.0, LightRole::Sun),
    ));

    DestructionGame {
        cam_id: cam,
        cam_yaw,
        cam_pitch,
        cam_pos,
        cam_speed: 40.0,
        chunk_mat,
        ball_mesh,
        ball_mat,
        shattered: false,
    }
}

// --------------------------------------------------------------- update
fn update(world: &mut World, state: &mut DestructionGame, dt: f32, input: &Input) {
    // --- kamera hareketi (WASD + QE + Shift) ---
    let mut speed = state.cam_speed;
    if input.is_key_pressed(KeyCode::ShiftLeft as u32) {
        speed *= 3.0;
    }
    // Tuşlar ve sol çubuk `Input::move_axis` ile tek yönde birleşiyor; dikey eksen
    // (Q/E) ayrı kalıyor çünkü çubukta karşılığı yok. Aşağıdaki normalize KIRPMAYA
    // döndü: normalize, yarım yatırılmış çubuğu tam hıza çıkarır ve çubuğun tek
    // kattığı şeyi — miktarı — yok ederdi. Tuşlarda davranış birebir aynı (herhangi
    // bir tuş bileşimi zaten 1 veya daha uzun).
    let (mx, my) = input.move_axis();
    let mut cam_move = Vec3::new(mx, 0.0, -my);
    if input.is_key_pressed(KeyCode::KeyQ as u32) {
        cam_move.y -= 1.0;
    }
    if input.is_key_pressed(KeyCode::KeyE as u32) {
        cam_move.y += 1.0;
    }

    let len = cam_move.length();
    if len > 1.0 {
        cam_move /= len;
    }
    cam_move *= speed * dt;

    // --- fareyle bakış (yalnız sağ-tık basılıyken) ---
    if input.is_mouse_button_pressed(1) {
        let md = input.mouse_delta();
        state.cam_yaw -= md.0 * 0.002;
        state.cam_pitch -= md.1 * 0.002;
        state.cam_pitch = state.cam_pitch.clamp(-1.5, 1.5);
    }

    // --- kamera Transform'unu güncelle ---
    if let Some(mut tr) = world.query_mut::<&mut Transform>().get_mut(state.cam_id) {
        let rot = pitch_yaw_quat(state.cam_pitch, state.cam_yaw);
        tr.rotation = rot;
        let forward = rot * Vec3::new(0.0, 0.0, -1.0);
        let right = rot * Vec3::new(1.0, 0.0, 0.0);
        let up = Vec3::new(0.0, 1.0, 0.0);
        tr.position += right * cam_move.x + up * cam_move.y - forward * cam_move.z;
        state.cam_pos = tr.position;
    }

    // --- obüs atışı (sol tık) — mesh/materyal önceden-kurulu, doğrudan spawn ---
    if input.is_mouse_button_just_pressed(0) {
        let forward = pitch_yaw_quat(state.cam_pitch, state.cam_yaw) * Vec3::new(0.0, 0.0, -1.0);
        world.spawn_bundle((
            Transform::new(state.cam_pos),
            state.ball_mesh.clone(),
            state.ball_mat.clone(),
            MeshRenderer::new(),
            RigidBodyBundle::dynamic(BALL_MASS)
                .with_collider(Collider::sphere(BALL_R))
                .with_velocity(forward * BALL_SPEED),
        ));
    }

    // --- parçalama (tek-seferlik) — voronoi CPU'da üretilip parçalar veri olarak kurulur ---
    if input.is_key_just_pressed(KeyCode::Space as u32) && !state.shattered {
        state.shattered = true;
        println!("Bina Voronoi parçalarına bölünüyor...");
        let chunks = voronoi_shatter(BUILD_EXTENTS, CHUNK_COUNT, SHATTER_SEED);
        println!("{} Voronoi konveks gövdesi üretildi!", chunks.len());
        spawn_chunks(world, chunks, &state.chunk_mat);
    }
}

// --------------------------------------------------------------- parçalar
/// Voronoi parçalarını dünyaya ekler: köşeleri kütle merkezine göre, `MeshData` olarak — GPU
/// vertex-buffer'larını aynı kare motorun yükleyicisi kuruyor.
fn spawn_chunks(world: &mut World, chunks: Vec<ProceduralChunk>, chunk_mat: &Material) {
    for chunk in chunks {
        let mut vertices = Vec::new();
        let mut min_pt = Vec3::splat(f32::MAX);
        let mut max_pt = Vec3::splat(f32::MIN);

        for (i, v) in chunk.vertices.iter().enumerate() {
            let local_pos = *v - chunk.center_of_mass;
            min_pt = min_pt.min(local_pos);
            max_pt = max_pt.max(local_pos);
            let n = chunk.normals[i];
            vertices.push(Vertex {
                position: [local_pos.x, local_pos.y, local_pos.z],
                color: [0.8, 0.7, 0.6, 1.0],
                normal: [n.x, n.y, n.z],
                tangent: [0.0, 0.0, 0.0, 1.0],
                tex_coords: [0.0, 0.0],
                joint_indices: [0; 4],
                joint_weights: [0.0; 4],
            });
        }

        let half_extents = (max_pt - min_pt) * 0.5;
        let mut rb = RigidBody::new(5.0, true);
        rb.is_sleeping = false;
        world.spawn_bundle((
            Transform::new(chunk.center_of_mass + BUILD_CENTER),
            MeshData::new(vertices),
            chunk_mat.clone(),
            MeshRenderer::new(),
            rb,
            Collider::aabb(half_extents),
        ));
    }
}

// --------------------------------------------------------------- main
fn main() {
    App::<DestructionGame>::new("Gizmo — Voronoi Destruction Demo", 1600, 900)
        .set_setup(setup)
        .set_update(update)
        .set_render_stage(RenderWorld::new())
        .run()
        .expect("uygulama çalıştırılamadı");
}
