//! Rüzgar Tüneli / Drag Odası — SÜREKLİ AKIŞ-ÇİZGİSİ ŞERİTLERİYLE.
//!
//! Araba sabit durur; havanın akışı, gövdenin etrafından sapan bir akış alanında CPU'da
//! entegre edilmiş SÜREKLİ ŞERİTLER (streamribbon) olarak çizilir — ayrı parçacıklar değil,
//! birbirine bağlı pürüzsüz kurdeleler (gerçek rüzgar-tüneli/CFD tekniği). Bespoke render
//! yolu YOK: şeritler motorun standart Mesh/Material yoluyla (`Mesh::from_vertices`) çizilir.
//!   • Akış çizgileri → nominal +Z akış + engel kürelerinden TEĞETSEL sapma (gövdeye sarılır).
//!   • Araba → herhangi bir GLB (model-bağımsız oto-ölçek/ortala/yere-otur).
//!   • Metalik boya → "Studio Neutral" ortam preset'i (IBL) + TAA.
//!
//! Motor idiomları (dürüst): sahne varlıkları tek `spawn_bundle` ile kurulur. Bu demo
//! `TransformPlugin` kullanır (fizik YOK → tüm gövdeler statik/görsel), bu yüzden ömür
//! komponentleri (DespawnAfter/BelowY) ya da Prefab-collider'ı YOK; `GlobalTransform` bundle'a
//! ELLE eklenir (PhysicsPlugin demoları bunu otomatik alır, TransformPlugin propagate'i yalnız
//! MEVCUT GlobalTransform'ları günceller). Render kancası BİLE-İSTEYE el-yazımı: her frame
//! şeritleri yeniden entegre eder ve gpu_particles/gpu_fluid/SSR/SSGI/volumetric'i kapatıp yalnız
//! IBL+TAA bırakır → `with_scene_render()` tek-satır kısayoluna çevrilMEZ.
//!
//! Kontrol: ↑/↓ (veya HUD slider) rüzgar hızı (HUD drag okuması). Çalıştır:
//! `cargo run -p demo --bin wind_tunnel`

use gizmo::app::App;
use gizmo::core::world::World;
use gizmo::egui;
use gizmo::math::{Mat4, Quat, Vec3, Vec4};
use gizmo::physics::components::GlobalTransform;
use gizmo::physics::Transform;
use gizmo::plugins::TransformPlugin;
use gizmo::renderer::asset::AssetManager;
use gizmo::renderer::components::{
    Camera, DirectionalLight, LightRole, Material, Mesh, MeshRenderer,
};
use gizmo::renderer::gpu_types::Vertex;

// Gerçek spor araba (Mercedes AMG GT4). Model-bağımsız oto-yerleştirme sayesinde herhangi
// bir araba GLB'siyle değiştirilebilir (ör. assets/bmw_z8__www.vecarz.com.glb).
const CAR_GLB: &str =
    "/home/bedir/Documents/code/Gizmo-engine/assets/mercedes_amg_gt4__www.vecarz.com.glb";
const TARGET_LEN: f32 = 7.0; // arabayı bu uzunluğa (birim) oto-ölçekle
const RIBBON_WIDTH: f32 = 0.32; // akış-çizgisi şerit genişliği (az sayıda, kalın)

// Araç ileri yönü = -Z; hava direnci TERSİNE (+Z) akar: girişte (-Z) doğar, akış +Z ilerler.
const FLOW_ENTRY_Z: f32 = -16.0;

// Aerodinamik (HUD drag okuması): ½·ρ·Cd·A·v².
const CAR_CD: f32 = 0.32; // varsayılan Cd (kullanıcı HUD'dan değiştirir — Cd ampirik/CFD gerektirir)
const AIR_RHO: f32 = 1.225;
// A (frontal alan) mesh'ten GERÇEK ölçülür; world→metre dönüşümü için gerçek araç uzunluğu.
// Model TARGET_LEN world-birime oto-ölçeklendi → ölçek = CAR_REAL_LENGTH_M / TARGET_LEN.
const CAR_REAL_LENGTH_M: f32 = 4.6; // Mercedes-AMG GT ~4.6 m (araç değişirse güncelle)

struct WindTunnel {
    wind_speed: f32, // m/s
    cam_id: u32,
    t: f32,
    ribbon_id: u32,           // akış-çizgisi şerit mesh entity'si
    seeds: Vec<Vec3>,         // akış çizgisi başlangıç noktaları (girişte)
    obstacles: Vec<[f32; 4]>, // arabayı temsil eden engel küreleri (her frame entegrasyonda)
    view_dir: Vec3,           // kameraya bakan şerit yönü
    frontal_area_m2: f32,     // mesh silüetinden GERÇEK ölçülen ön-izdüşüm alanı (m²)
    cd: f32,                  // drag katsayısı (kullanıcı girer; ampirik/CFD)
}

fn main() {
    gizmo::app::setup_panic_hook();
    println!("Rüzgar Tüneli — ↑/↓ veya HUD slider ile rüzgar hızını değiştir.");
    println!("Akış SÜREKLİ ŞERİTLER olarak çizilir (CPU-entegre streamribbon, gövdeye sarılır). Bespoke render yok.");

    App::<WindTunnel>::new("Gizmo — Rüzgar Tüneli", 1600, 900)
        .add_plugin(TransformPlugin)
        .set_setup(setup)
        .set_update(update)
        .set_ui(ui)
        .set_render(|world, state, encoder, view, renderer, _light_time| {
            renderer.gpu_particles = None; // akış artık şerit mesh'i (parçacık yok)
                                           // Metalik boya için stüdyo ortamı (IBL) → yansıma/görünürlük.
            renderer.environment_preset = 1; // Studio Neutral
            renderer.environment_preset_2 = 1;
            // Kullanılmayan ağır pass'leri kapat; TAA AÇIK (spekular parıltıyı temizler).
            renderer.gpu_fluid = None;
            renderer.ssr = None;
            renderer.ssgi = None;
            renderer.volumetric = None;
            // GERÇEK DİNAMİK AKIŞ: şeritleri HER FRAME zaman-değişen akış alanından yeniden
            // entegre et → geometri gerçekten dalgalanır/değişir (renk kaydırma değil). Hız
            // (wind_speed) türbülans faz-hızını ve bant hızını sürer; wind=0 → donuk (rüzgar yok).
            {
                let verts = build_ribbon_verts(
                    &state.seeds,
                    &state.obstacles,
                    state.view_dir,
                    RIBBON_WIDTH,
                    state.t,
                    state.wind_speed,
                );
                if let Some(m) = world.borrow::<Mesh>().get(state.ribbon_id) {
                    m.update_vertices(&renderer.queue, &verts);
                }
            }
            gizmo::systems::default_render_pass(world, encoder, view, renderer);
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn setup(world: &mut World, renderer: &gizmo::renderer::Renderer) -> WindTunnel {
    let mut assets = AssetManager::new();
    let white = assets.create_white_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );

    // Zemin (koyu tünel tabanı) — tek spawn_bundle (GlobalTransform elle: TransformPlugin).
    {
        let t = Transform::new(Vec3::ZERO);
        world.spawn_bundle((
            t,
            GlobalTransform {
                matrix: t.local_matrix,
            },
            AssetManager::create_plane(&renderer.device, 60.0),
            Material::new(white.clone()).with_pbr(Vec4::new(0.10, 0.10, 0.12, 1.0), 0.85, 0.0),
            MeshRenderer::new(),
        ));
    }

    // Işık
    {
        let t = Transform::new(Vec3::new(0.0, 10.0, 0.0)).with_rotation(
            Quat::from_rotation_x(-std::f32::consts::FRAC_PI_4)
                * Quat::from_rotation_y(std::f32::consts::FRAC_PI_4),
        );
        world.spawn_bundle((
            t,
            GlobalTransform {
                matrix: t.local_matrix,
            },
            DirectionalLight::new(Vec3::new(1.0, 0.97, 0.90), 3.4, LightRole::Sun),
        ));
    }

    // Araba (GLB) — YALNIZ GÖRSEL, fizik yok. Model-bağımsız oto-yerleştirme.
    let chassis = {
        let mut cmd = gizmo::prelude::SpawnCommands::new(world, renderer);
        cmd.spawn_gltf(Vec3::ZERO, CAR_GLB, false).unwrap().id()
    };
    fit_car(world, chassis.id());

    // Arabanın GERÇEK dünya AABB'si → engel/kamera/tohum bundan türetilir.
    let (car_min, car_max) = world_aabb(world, chassis.id())
        .unwrap_or((Vec3::new(-2.0, 0.0, -4.0), Vec3::new(2.0, 3.0, 4.0)));
    let center = (car_min + car_max) * 0.5;
    let diag = (car_max - car_min).length();
    println!("[wind_tunnel] araba AABB boyut={:?}", car_max - car_min);

    // GERÇEK frontal alanı mesh silüetinden ölç (m²).
    let frontal_area_m2 = measure_frontal_area_m2(world, chassis.id(), car_min, car_max);
    println!(
        "[wind_tunnel] ölçülen frontal alan ≈ {:.3} m²",
        frontal_area_m2
    );

    // Arabayı temsil eden engel küreleri (Z boyunca) — akış bunlara TEĞET sapıp gövdeye sarılır.
    let obstacles = build_obstacles(car_min, car_max);

    // Sabit 3/4 ön-yan kamera (akışa DİK → çizgiler yatay-paralel görünür).
    let dir = Vec3::new(1.0, 0.34, -0.32).normalize(); // arabadan kameraya
    let dist = diag * 1.5 + 4.0;
    let cam_pos = center + dir * dist;
    let cam_id = {
        let target = center + Vec3::new(0.0, diag * 0.02, 0.0);
        let look = (target - cam_pos).normalize();
        let yaw = look.z.atan2(look.x);
        let pitch = look.y.clamp(-1.0, 1.0).asin();
        let t = Transform::new(cam_pos);
        world
            .spawn_bundle((
                t,
                GlobalTransform {
                    matrix: t.local_matrix,
                },
                Camera::new(1.05, 0.1, 500.0, yaw, pitch, true),
            ))
            .id()
    };

    // AKIŞ-ÇİZGİSİ ŞERİTLERİ: ön kesiti kaplayan tohum ızgarası. Şeritler HER FRAME (render'da)
    // zaman-değişen akış alanından yeniden entegre edilir → gerçek dinamik dalgalanma; hız bunu
    // sürer. Mesh <20000 vertex (48 tohum × 60 × 6) → per-frame güncelleme meshopt'suz ucuz.
    // Tek-yüz (cull=None → her açıdan görünür).
    let seeds: Vec<Vec3> = {
        let cx = center.x;
        let half_w = (car_max.x - car_min.x) * 0.5;
        let top = car_max.y;
        let cols = 4usize;
        let rows = 6usize;
        let x_lo = cx - (half_w + 2.2);
        let x_hi = cx + (half_w + 2.2);
        let y_lo = 0.25f32;
        let y_hi = top + 1.2;
        let mut s = Vec::with_capacity(cols * rows);
        for iy in 0..rows {
            let y = y_lo + (y_hi - y_lo) * (iy as f32 / (rows - 1) as f32);
            for ix in 0..cols {
                let x = x_lo + (x_hi - x_lo) * (ix as f32 / (cols - 1) as f32);
                s.push(Vec3::new(x, y, FLOW_ENTRY_Z));
            }
        }
        s
    };
    let view_dir = dir; // kameraya bakan şerit yönü (kamera sabit)

    let ribbon_id = {
        let verts = build_ribbon_verts(&seeds, &obstacles, view_dir, RIBBON_WIDTH, 0.0, 0.0);
        println!("[wind_tunnel] şerit vertex sayısı: {}", verts.len());
        let t = Transform::new(Vec3::ZERO);
        world
            .spawn_bundle((
                t,
                GlobalTransform {
                    matrix: t.local_matrix,
                },
                Mesh::from_vertices(&renderer.device, &verts, "streamlines"),
                // Yarı-saydam unlit → araba aralardan görünür; vertex renkleri her frame animasyonlanır.
                Material::new(white.clone()).with_unlit(Vec4::new(0.95, 0.97, 1.0, 0.30)),
                MeshRenderer::new(),
            ))
            .id()
    };

    WindTunnel {
        wind_speed: 20.0,
        cam_id,
        t: 0.0,
        ribbon_id,
        seeds,
        obstacles,
        view_dir,
        frontal_area_m2,
        cd: CAR_CD,
    }
}

/// Modeli hedef uzunluğa ölçekle, uzun yatay kenarı Z'ye hizala (gerekirse 90°Y),
/// X/Z'de orijine ortala, alt yüzü y=0'a otur. Herhangi bir araba GLB'sine uyar.
fn fit_car(world: &mut World, root: u32) {
    // Root'u kimliğe sabitle → doğal ölçüm.
    {
        let mut transforms = world.borrow_mut::<Transform>();
        if let Some(mut t) = transforms.get_mut(root) {
            t.scale = Vec3::splat(1.0);
            t.rotation = Quat::IDENTITY;
            t.position = Vec3::ZERO;
            t.update_local_matrix();
        }
    }
    let (a0, b0) = world_aabb(world, root).unwrap_or((Vec3::splat(-1.0), Vec3::splat(1.0)));
    let size = b0 - a0;
    let long_horiz = size.x.max(size.z).max(1e-3);
    let scale = TARGET_LEN / long_horiz;
    let rot = if size.x > size.z {
        Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)
    } else {
        Quat::IDENTITY
    };
    let mut nmin = Vec3::splat(f32::INFINITY);
    let mut nmax = Vec3::splat(f32::NEG_INFINITY);
    for &cx in &[a0.x, b0.x] {
        for &cy in &[a0.y, b0.y] {
            for &cz in &[a0.z, b0.z] {
                let p = rot * (Vec3::new(cx, cy, cz) * scale);
                nmin = Vec3::new(nmin.x.min(p.x), nmin.y.min(p.y), nmin.z.min(p.z));
                nmax = Vec3::new(nmax.x.max(p.x), nmax.y.max(p.y), nmax.z.max(p.z));
            }
        }
    }
    let ncenter = (nmin + nmax) * 0.5;
    let pos = Vec3::new(-ncenter.x, -nmin.y, -ncenter.z);
    let mut transforms = world.borrow_mut::<Transform>();
    if let Some(mut t) = transforms.get_mut(root) {
        t.scale = Vec3::splat(scale);
        t.rotation = rot;
        t.position = pos;
        t.update_local_matrix();
    }
}

/// Arabayı temsil eden engel küreleri zinciri (Z boyunca).
fn build_obstacles(car_min: Vec3, car_max: Vec3) -> Vec<[f32; 4]> {
    let cx = (car_min.x + car_max.x) * 0.5;
    let cy = (car_min.y + car_max.y) * 0.5;
    let half_h = (car_max.y - car_min.y) * 0.5;
    let half_w = (car_max.x - car_min.x) * 0.5;
    let r = half_h.max(half_w) * 0.8;
    let n = 8usize;
    let z0 = car_min.z + r * 0.5;
    let z1 = car_max.z - r * 0.5;
    let mut v = Vec::with_capacity(n);
    for i in 0..n {
        let f = if n > 1 {
            i as f32 / (n - 1) as f32
        } else {
            0.5
        };
        v.push([cx, cy, z0 + (z1 - z0) * f, r]);
    }
    v
}

fn norm(v: Vec3) -> Vec3 {
    let l2 = v.length_squared();
    if l2 > 1e-12 {
        v / l2.sqrt()
    } else {
        Vec3::new(0.0, 0.0, 1.0)
    }
}

/// ZAMAN-DEĞİŞEN türbülans alanı — `phase` ilerledikçe dalgalar aşağı-akışa doğru advekte
/// olur (kayar). Böylece her frame yeniden entegre edilen akış çizgileri gerçekten dalgalanır.
fn flow_noise(p: Vec3, phase: f32) -> Vec3 {
    let mut v = Vec3::new(0.0, 0.0, 0.0);
    v.x += (p.y * 0.7 + phase * 0.5 + 1.3).sin() - (p.z * 0.6 - phase + 2.1).sin();
    v.y += (p.z * 0.8 - phase + 0.5).sin() - (p.x * 0.5 + phase * 0.5 + 4.2).sin();
    v.z += (p.x * 0.6 + 3.3).sin() - (p.y * 0.9 + phase * 0.7 + 1.7).sin();
    v * 0.35
}

/// Bir tohumdan akış alanını (nominal +Z + hafif türbülans + engel TEĞET sapması) sabit
/// adımla entegre eder → arabaya sarılan pürüzsüz bir polyline döndürür.
fn integrate_streamline(seed: Vec3, obstacles: &[[f32; 4]], phase: f32) -> Vec<Vec3> {
    const STEP: f32 = 0.6;
    // SABİT adım sayısı (erken çıkış YOK) → her frame aynı vertex sayısı (update_vertices şart).
    const MAX_STEPS: usize = 60; // 48 tohum × 60 × 6 = 17280 < 20000
    const TURB: f32 = 0.25;
    let mut pts = Vec::with_capacity(MAX_STEPS + 1);
    let mut pos = seed;
    pts.push(pos);
    for _ in 0..MAX_STEPS {
        let mut v = Vec3::new(0.0, 0.0, 1.0) + flow_noise(pos, phase) * TURB;
        v = norm(v);
        // Engel sapması: içeri gireni iptal et → yüzeye teğet kay (gövdeye sarılır).
        for o in obstacles {
            let c = Vec3::new(o[0], o[1], o[2]);
            let r = o[3];
            let d = pos - c;
            let dist = d.length();
            let influence = r * 1.25;
            if dist < influence && dist > 1e-4 {
                let nrm = d / dist;
                let vin = v.dot(nrm);
                if vin < 0.0 {
                    v -= nrm * vin;
                }
                let push = 1.0 - dist / influence;
                v += nrm * (push * 0.35);
                v = norm(v);
                if dist < r {
                    pos = c + nrm * r;
                }
            }
        }
        pos += v * STEP;
        pts.push(pos);
    }
    pts
}

/// Bir polyline'ı kameraya-bakan bir üçgen-şerit olarak `verts`'e ekler (tek-yüz; unlit
/// cull=None olduğundan her açıdan görünür). `tex_coords.y` = kümülatif arc-length →
/// akış animasyonunda ilerleyen bantlar için kullanılır.
fn append_ribbon(verts: &mut Vec<Vertex>, line: &[Vec3], view_dir: Vec3, width: f32, phase: f32) {
    if line.len() < 2 {
        return;
    }
    let hw = width * 0.5;
    let vtx = |p: Vec3, uv: [f32; 2]| {
        // Renk: arc-length boyunca ilerleyen parlak bant (akış yönü ipucu).
        let b = flow_brightness(uv[1], phase);
        Vertex {
            position: [p.x, p.y, p.z],
            color: [b * 0.95, b * 0.97, b],
            normal: [0.0, 1.0, 0.0],
            tex_coords: uv,
            joint_indices: [0; 4],
            joint_weights: [0.0; 4],
            ..Default::default()
        }
    };
    let side_at = |t: Vec3| -> Vec3 {
        let mut s = t.cross(view_dir);
        if s.length_squared() < 1e-8 {
            s = t.cross(Vec3::new(0.0, 1.0, 0.0));
        }
        norm(s) * hw
    };
    let mut arclen = 0.0f32;
    for i in 0..line.len() - 1 {
        let p0 = line[i];
        let p1 = line[i + 1];
        let seg = (p1 - p0).length();
        let t0 = norm(p1 - p0);
        let t1 = if i + 2 < line.len() {
            norm(line[i + 2] - p1)
        } else {
            t0
        };
        let s0 = side_at(t0);
        let s1 = side_at(t1);
        let l0 = p0 - s0;
        let r0 = p0 + s0;
        let l1 = p1 - s1;
        let r1 = p1 + s1;
        let u0 = arclen;
        let u1 = arclen + seg;
        verts.push(vtx(l0, [0.0, u0]));
        verts.push(vtx(r0, [1.0, u0]));
        verts.push(vtx(r1, [1.0, u1]));
        verts.push(vtx(l0, [0.0, u0]));
        verts.push(vtx(r1, [1.0, u1]));
        verts.push(vtx(l1, [0.0, u1]));
        arclen = u1;
    }
}

/// Arc-length boyunca ilerleyen parlak bant. Parlaklık 0.4..1.0 (0'a inince unlit shader
/// beyaza fallback ettiğinden alt sınır 0.4).
fn flow_brightness(arclen: f32, phase: f32) -> f32 {
    let band = 0.5 + 0.5 * ((arclen - phase) * 0.9).sin();
    0.4 + 0.6 * band
}

/// TÜM akış-çizgisi şeritlerini VERTEX olarak inşa eder — her frame yeniden hesaplanır.
/// `phase = t·wind·k` hem türbülans alanını (geometri dalgalanır) hem parlak bantları sürer;
/// wind=0'da faz donar (rüzgar yok). Vertex sayısı sabittir (erken-çıkışsız entegrasyon).
fn build_ribbon_verts(
    seeds: &[Vec3],
    obstacles: &[[f32; 4]],
    view_dir: Vec3,
    width: f32,
    t: f32,
    wind: f32,
) -> Vec<Vertex> {
    let phase = t * wind * 0.6;
    let mut verts = Vec::new();
    for &seed in seeds {
        let line = integrate_streamline(seed, obstacles, phase);
        append_ribbon(&mut verts, &line, view_dir, width, phase);
    }
    verts
}

/// Bir entity + alt hiyerarşisinin DÜNYA-uzayı AABB'si (Mesh.bounds × hiyerarşi matrisleri).
fn world_aabb(world: &World, root: u32) -> Option<(Vec3, Vec3)> {
    let meshes = world.borrow::<Mesh>();
    let transforms = world.borrow::<Transform>();
    let children = world.borrow::<gizmo::core::component::Children>();
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut found = false;
    let mut stack: Vec<(u32, Mat4)> = vec![(root, Mat4::IDENTITY)];
    while let Some((e, parent_m)) = stack.pop() {
        let local = transforms
            .get(e)
            .map(|t| t.local_matrix)
            .unwrap_or(Mat4::IDENTITY);
        let world_m = parent_m * local;
        if let Some(m) = meshes.get(e) {
            let b = &m.bounds;
            let (lo, hi) = (b.min, b.max);
            for &cx in &[lo.x, hi.x] {
                for &cy in &[lo.y, hi.y] {
                    for &cz in &[lo.z, hi.z] {
                        let p = world_m.transform_point3(Vec3::new(cx, cy, cz));
                        min = Vec3::new(min.x.min(p.x), min.y.min(p.y), min.z.min(p.z));
                        max = Vec3::new(max.x.max(p.x), max.y.max(p.y), max.z.max(p.z));
                        found = true;
                    }
                }
            }
        }
        if let Some(c) = children.get(e) {
            for &kid in &c.0 {
                stack.push((kid, world_m));
            }
        }
    }
    if found {
        Some((min, max))
    } else {
        None
    }
}

/// Arabanın GERÇEK ön-izdüşüm (frontal) alanını ölçer: tüm mesh üçgenlerini dünya-uzayında
/// X-Y düzlemine (akış Z boyunca → frontal kesit = X-Y) izdüşürüp bir ızgaraya rasterize eder,
/// kaplanan hücre alanını toplar → world² → (CAR_REAL_LENGTH_M/TARGET_LEN)² ile m²'ye çevirir.
/// Üçgen yoksa (cpu_vertices boş) AABB kutusunun %80'ine düşer.
fn measure_frontal_area_m2(world: &World, root: u32, amin: Vec3, amax: Vec3) -> f32 {
    const G: usize = 384;
    let min_x = amin.x;
    let min_y = amin.y;
    let w = (amax.x - amin.x).max(1e-4);
    let h = (amax.y - amin.y).max(1e-4);
    let mut grid = vec![false; G * G];

    let meshes = world.borrow::<Mesh>();
    let transforms = world.borrow::<Transform>();
    let children = world.borrow::<gizmo::core::component::Children>();
    let mut tri_count = 0usize;

    let mut stack: Vec<(u32, Mat4)> = vec![(root, Mat4::IDENTITY)];
    while let Some((e, parent_m)) = stack.pop() {
        let local = transforms
            .get(e)
            .map(|t| t.local_matrix)
            .unwrap_or(Mat4::IDENTITY);
        let world_m = parent_m * local;
        if let Some(m) = meshes.get(e) {
            let cv = &m.cpu_vertices;
            let mut i = 0;
            while i + 2 < cv.len() {
                let a = world_m.transform_point3(cv[i]);
                let b = world_m.transform_point3(cv[i + 1]);
                let c = world_m.transform_point3(cv[i + 2]);
                rasterize_tri_xy(
                    &mut grid,
                    G,
                    (a.x, a.y),
                    (b.x, b.y),
                    (c.x, c.y),
                    min_x,
                    min_y,
                    w,
                    h,
                );
                tri_count += 1;
                i += 3;
            }
        }
        if let Some(ch) = children.get(e) {
            for &k in &ch.0 {
                stack.push((k, world_m));
            }
        }
    }

    let scale = CAR_REAL_LENGTH_M / TARGET_LEN;
    let scale2 = scale * scale;
    let covered = grid.iter().filter(|&&x| x).count();
    if tri_count == 0 || covered == 0 {
        return w * h * 0.8 * scale2; // fallback: AABB kutusu × 0.8
    }
    let cell_area = (w / G as f32) * (h / G as f32);
    covered as f32 * cell_area * scale2
}

/// Bir üçgeni X-Y ızgarasına doldurur (bbox + kenar-işareti testi; winding'den bağımsız).
fn rasterize_tri_xy(
    grid: &mut [bool],
    g: usize,
    a: (f32, f32),
    b: (f32, f32),
    c: (f32, f32),
    min_x: f32,
    min_y: f32,
    w: f32,
    h: f32,
) {
    let gf = g as f32;
    let to_gx = |x: f32| (x - min_x) / w * gf;
    let to_gy = |y: f32| (y - min_y) / h * gf;
    let (ax, ay) = (to_gx(a.0), to_gy(a.1));
    let (bx, by) = (to_gx(b.0), to_gy(b.1));
    let (cx, cy) = (to_gx(c.0), to_gy(c.1));
    let lo_x = ax.min(bx).min(cx).floor().max(0.0) as usize;
    let hi_x = (ax.max(bx).max(cx).ceil() as isize).clamp(0, g as isize - 1) as usize;
    let lo_y = ay.min(by).min(cy).floor().max(0.0) as usize;
    let hi_y = (ay.max(by).max(cy).ceil() as isize).clamp(0, g as isize - 1) as usize;
    if lo_x > hi_x || lo_y > hi_y {
        return;
    }
    let edge = |px: f32, py: f32, x0: f32, y0: f32, x1: f32, y1: f32| {
        (px - x0) * (y1 - y0) - (py - y0) * (x1 - x0)
    };
    for py in lo_y..=hi_y {
        for px in lo_x..=hi_x {
            let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
            let e0 = edge(fx, fy, ax, ay, bx, by);
            let e1 = edge(fx, fy, bx, by, cx, cy);
            let e2 = edge(fx, fy, cx, cy, ax, ay);
            let inside =
                (e0 >= 0.0 && e1 >= 0.0 && e2 >= 0.0) || (e0 <= 0.0 && e1 <= 0.0 && e2 <= 0.0);
            if inside {
                grid[py * g + px] = true;
            }
        }
    }
}

fn update(_world: &mut World, state: &mut WindTunnel, dt: f32, input: &gizmo::core::input::Input) {
    state.t += dt;
    if input.is_key_pressed(gizmo::prelude::KeyCode::ArrowUp as u32) {
        state.wind_speed = (state.wind_speed + 20.0 * dt).min(60.0);
    }
    if input.is_key_pressed(gizmo::prelude::KeyCode::ArrowDown as u32) {
        state.wind_speed = (state.wind_speed - 20.0 * dt).max(0.0);
    }
    let _ = state.cam_id;
}

fn ui(_world: &mut World, state: &mut WindTunnel, ctx: &egui::Context) {
    let a = state.frontal_area_m2;
    let drag = 0.5 * AIR_RHO * state.cd * a * state.wind_speed * state.wind_speed;
    let power_kw = drag * state.wind_speed / 1000.0; // sürükleme gücü = F·v
    egui::Window::new("Rüzgar Tüneli — Aero")
        .default_pos([16.0, 16.0])
        .show(ctx, |ui| {
            ui.add(egui::Slider::new(&mut state.wind_speed, 0.0..=80.0).text("Hız (m/s)"));
            ui.add(egui::Slider::new(&mut state.cd, 0.15..=0.60).text("Cd (girdi)"));
            ui.separator();
            ui.label(format!("Hız: {:.0} m/s  ({:.0} km/h)", state.wind_speed, state.wind_speed * 3.6));
            ui.label(
                egui::RichText::new(format!("Frontal alan A ≈ {a:.2} m²  (mesh'ten ÖLÇÜLDÜ)"))
                    .strong()
                    .color(egui::Color32::from_rgb(140, 230, 140)),
            );
            ui.label(
                egui::RichText::new(format!("Sürükleme kuvveti: {drag:.0} N"))
                    .strong()
                    .color(egui::Color32::from_rgb(90, 200, 255)),
            );
            let max_drag = 0.5 * AIR_RHO * 0.6 * a.max(0.1) * 80.0 * 80.0;
            ui.add(egui::ProgressBar::new((drag / max_drag).clamp(0.0, 1.0)).text("drag"));
            ui.label(format!("Sürükleme gücü: {power_kw:.1} kW  ({:.0} hp)", power_kw * 1.341));
            ui.separator();
            ui.small("F = ½·ρ·Cd·A·v²   (ρ=1.225 kg/m³)");
            ui.small(
                egui::RichText::new("A = mesh silüetinden GERÇEK ölçüldü. Cd ampirik/CFD'dir — bu demo Cd ÖLÇMEZ (görselleştirme).")
                    .color(egui::Color32::from_rgb(210, 180, 120)),
            );
        });
}
