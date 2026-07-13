// Gizmo — Su Sistemi Demosu (Subnautica-tarzı dikey dilim)
//
// Bu oturumda eklenen SU SİSTEMİNİ tek sahnede sergiler:
//   • W5  Gerstner okyanus yüzeyi  (Material::with_water → dalgalı su shader'ı)
//   • W1  FluidZone su hacmi        (PhysicsWorld.fluid_zones)
//   •     Buoyancy (Archimedes)     → yüzen kutular batıklık oranına göre sallanır, ağırlar batar
//   • W2  Yüzme karakter kontrolcüsü→ FPS dalgıç: WASD bak-yönünde yüz, Space yüksel, Ctrl dal
//   • W3+W4 Su-altı sisi            → kamera yüzeyin altına inince derinlik-bazlı mavi-yeşil sis
//
// Kontroller: sağ-tık + fare = bak, WASD = yüz, Space = yüksel, Sol-Ctrl = dal, Shift = hızlı.
//
// Çalıştır: cargo run -p demo --bin water_demo

use gizmo::physics::components::{CharacterController, Collider, RigidBody, Transform, Velocity};
use gizmo::physics::world::{FluidZone, PhysicsWorld, ZoneShape};
use gizmo::prelude::*;
use gizmo::renderer::asset::AssetManager;
use gizmo::renderer::components::{Camera, DirectionalLight, LightRole, Material, MeshRenderer};
use gizmo::winit::keyboard::KeyCode;

const WATER_SURFACE_Y: f32 = 0.0;
const SEABED_Y: f32 = -30.0;

struct WaterState {
    swimmer: gizmo::core::Entity,
    cam_yaw: f32,
    cam_pitch: f32,
    phys_accum: f32,
    depth: f32,       // HUD: kameranın yüzey altı derinliği
    underwater: bool, // HUD
    oxygen_frac: f32, // HUD: kalan hava oranı 0..1
    oxygen_secs: f32, // HUD: kalan hava (saniye)
    // --shot modu: N kare sonra offscreen render'ı okuyup raw RGBA'ya yazar, çıkar (headless PNG).
    frame: u32,
    shot: bool,
    shot_done: bool,
    // Ambient ses + su-altı boğma (kamera batınca AudioManager::set_underwater ile kısılır/boğuklaşır).
    audio: Option<gizmo::prelude::AudioManager>,
}

fn setup(world: &mut World, renderer: &Renderer) -> WaterState {
    println!("Su demosu yükleniyor... (sağ-tık+fare=bak, WASD=yüz, Space=yüksel, Ctrl=dal)");

    let mut asset = AssetManager::new();
    let white = asset.create_white_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );
    let checker = asset.create_checkerboard_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );

    // ── FİZİK DÜNYASI + SU HACMİ (FluidZone) ─────────────────────────────────
    // Yüzey y=0'dan tabana y=-30'a kadar dev bir su kutusu. Aynı hacim hem buoyancy'yi
    // hem yüzmeyi hem kamera-su-altı sisini sürer (hepsi water_at üzerinden).
    let mut phys_world = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0));
    phys_world.fluid_zones.push(FluidZone {
        shape: ZoneShape::Box {
            min: Vec3::new(-200.0, SEABED_Y, -200.0),
            max: Vec3::new(200.0, WATER_SURFACE_Y, 200.0),
        },
        density: 1000.0, // tatlı su ~1000, deniz ~1025
        viscosity: 1.0,
        linear_drag: 3.0,
        quadratic_drag: 0.8,
        fog_color: [0.02, 0.09, 0.13], // derin deniz lacivert-yeşili
        fog_density: 0.10,
    });
    world.insert_resource(phys_world);

    // ── GÖKYÜZÜ (asset'siz: ters küp + unlit mavi) ───────────────────────────
    let sky = world.spawn();
    world.add_component(sky, Transform::new(Vec3::ZERO).with_scale(Vec3::splat(900.0)));
    world.add_component(sky, AssetManager::create_inverted_cube(&renderer.device));
    world.add_component(sky, Material::new(white.clone()).with_unlit(Vec4::new(0.35, 0.55, 0.8, 1.0)));
    world.add_component(sky, MeshRenderer::new());

    // ── GÜNEŞ ────────────────────────────────────────────────────────────────
    let sun = world.spawn();
    world.add_component(
        sun,
        Transform::new(Vec3::new(40.0, 90.0, 30.0))
            .with_rotation(Quat::from_axis_angle(Vec3::new(1.0, 0.4, 0.0).normalize(), -0.9)),
    );
    world.add_component(sun, DirectionalLight::new(Vec3::new(1.0, 0.96, 0.85), 3.0, LightRole::Sun));

    // ── OKYANUS YÜZEYİ (Gerstner su shader'ı) ────────────────────────────────
    // Material::with_water → MaterialType::Water → water.wgsl (Gerstner dalga + Fresnel).
    // Çift-taraflı: su altından yukarı bakınca da görünür.
    let ocean = world.spawn();
    world.add_component(ocean, Transform::new(Vec3::new(0.0, WATER_SURFACE_Y, 0.0)));
    world.add_component(ocean, AssetManager::create_plane(&renderer.device, 400.0));
    world.add_component(
        ocean,
        Material::new(white.clone())
            .with_water(Vec4::new(0.10, 0.35, 0.50, 0.80))
            .with_double_sided(true),
    );
    world.add_component(ocean, MeshRenderer::new());

    // ── DENİZ TABANI (statik zemin) ──────────────────────────────────────────
    let seabed = world.spawn();
    world.add_component(seabed, Transform::new(Vec3::new(0.0, SEABED_Y, 0.0)));
    world.add_component(seabed, AssetManager::create_plane(&renderer.device, 400.0));
    world.add_component(
        seabed,
        Material::new(checker.clone()).with_pbr(Vec4::new(0.35, 0.32, 0.25, 1.0), 0.95, 0.0),
    );
    world.add_component(seabed, MeshRenderer::new());
    world.add_component(seabed, Collider::box_collider(Vec3::new(200.0, 0.1, 200.0)));
    world.add_component(seabed, RigidBody::new_static());
    world.add_component(seabed, Velocity::default());

    // ── YÜZEN KUTULAR (buoyancy) ─────────────────────────────────────────────
    // Yarı-boyut 0.5 → hacim 1 m³ → tam batıkta ~1000 N/kg kaldırma. mass<1000 → yüzer
    // (mass/1000 oranında batık), mass>1000 → batar. Yükseklikten düşüp yüzeyde sallanırlar.
    let cube = AssetManager::create_cube(&renderer.device);
    let floaters: &[(Vec3, f32, Vec4)] = &[
        (Vec3::new(-3.0, 6.0, -2.0), 250.0, Vec4::new(0.9, 0.5, 0.2, 1.0)), // hafif → yüzer
        (Vec3::new(0.0, 8.0, 0.0), 350.0, Vec4::new(0.8, 0.75, 0.2, 1.0)),
        (Vec3::new(3.5, 5.0, 1.5), 450.0, Vec4::new(0.3, 0.7, 0.4, 1.0)),
        (Vec3::new(-1.5, 7.0, 3.0), 600.0, Vec4::new(0.5, 0.4, 0.8, 1.0)), // yarı-batık
        (Vec3::new(2.0, 10.0, -3.5), 2500.0, Vec4::new(0.3, 0.3, 0.35, 1.0)), // ağır → dibe batar
    ];
    for &(pos, mass, color) in floaters {
        let e = world.spawn();
        world.add_component(e, Transform::new(pos).with_scale(Vec3::splat(0.5)));
        world.add_component(e, cube.clone());
        world.add_component(e, Material::new(white.clone()).with_pbr(color, 0.6, 0.0));
        world.add_component(e, MeshRenderer::new());
        world.add_component(e, Collider::box_collider(Vec3::splat(0.5)));
        world.add_component(e, RigidBody::new(mass, true));
        world.add_component(e, Velocity::default());
    }

    // ── DALGIÇ (yüzme karakter kontrolcüsü — kinematik, RigidBody YOK) ────────
    let swimmer = world.spawn();
    world.add_component(swimmer, Transform::new(Vec3::new(0.0, -1.0, 8.0)));
    world.add_component(swimmer, Velocity::default());
    world.add_component(swimmer, Collider::capsule(0.3, 0.6));
    world.add_component(
        swimmer,
        CharacterController {
            speed: 6.0,             // yüzme hızı
            buoyancy: 1.5,          // hafif yukarı meyil (girdisizken yavaş yüzeye çıkar)
            water_drag: 2.5,        // ağdalı su
            swim_acceleration: 9.0, // tepkisel itiş
            ..Default::default()
        },
    );
    // Oksijen: batıkken tükenir (~1/sn, 45 sn hava), yüzeyde hızla dolar.
    world.add_component(swimmer, gizmo::physics::Oxygen::default());

    // ── KAMERA (FPS: dalgıcın kafasında) ─────────────────────────────────────
    let cam = world.spawn();
    world.add_component(cam, Transform::new(Vec3::new(0.0, 0.0, 10.0)));
    world.add_component(cam, Camera::new(std::f32::consts::FRAC_PI_3, 0.1, 2000.0, 0.0, 0.0, true));

    // ── AMBIENT SES ──────────────────────────────────────────────────────────
    // Sürekli bir ambient loop; kamera su altına inince AudioManager::set_underwater(true) ile
    // kısılıp boğuklaşır (su-altı ses boğma). Ses cihazı yoksa (headless) sessizce None kalır.
    let audio = gizmo::prelude::AudioManager::new().ok().map(|mut a| {
        if a.load_sound("ambient", "demo/assets/audio/engine.wav").is_ok() {
            if let Ok(id) = a.play_looped("ambient") {
                a.set_volume(id, 0.22); // hafif ambient uğultu
            }
        }
        a
    });

    println!("Sahne hazır! Su altına dalınca (Ctrl) sis + ses boğması devreye girer.");

    WaterState {
        swimmer,
        cam_yaw: -std::f32::consts::FRAC_PI_2,
        cam_pitch: -0.15,
        phys_accum: 0.0,
        depth: 0.0,
        underwater: false,
        oxygen_frac: 1.0,
        oxygen_secs: 45.0,
        frame: 0,
        shot: std::env::args().any(|a| a == "--shot"),
        shot_done: false,
        audio,
    }
}

fn update(world: &mut World, state: &mut WaterState, dt: f32, input: &gizmo::core::input::Input) {
    state.frame += 1;
    // ── Fare ile bakış ───────────────────────────────────────────────────────
    if input.is_mouse_button_pressed(gizmo::core::input::mouse::RIGHT) {
        let d = input.mouse_delta();
        state.cam_yaw -= d.0 * 0.005;
        state.cam_pitch -= d.1 * 0.005;
        state.cam_pitch = state.cam_pitch.clamp(
            -std::f32::consts::FRAC_PI_2 + 0.05,
            std::f32::consts::FRAC_PI_2 - 0.05,
        );
    }

    // Bakış vektörleri.
    let (cy, sy) = (state.cam_yaw.cos(), state.cam_yaw.sin());
    let (cp, sp) = (state.cam_pitch.cos(), state.cam_pitch.sin());
    let forward = Vec3::new(cy * cp, sp, sy * cp).normalize_or_zero(); // tam 3B (yukarı/aşağı bakış dahil)
    let right = Vec3::new(-sy, 0.0, cy).normalize_or_zero();

    // ── Yüzme girdisi → target_velocity (3B) ─────────────────────────────────
    let fast = if input.is_key_pressed(KeyCode::ShiftLeft as u32) { 1.8 } else { 1.0 };
    {
        if let Some(mut kcc) = world
            .borrow_mut::<CharacterController>()
            .get_mut(state.swimmer.id())
        {
            let mut dir = Vec3::ZERO;
            if input.is_key_pressed(KeyCode::KeyW as u32) { dir += forward; } // baktığın yöne yüz
            if input.is_key_pressed(KeyCode::KeyS as u32) { dir -= forward; }
            if input.is_key_pressed(KeyCode::KeyD as u32) { dir += right; }
            if input.is_key_pressed(KeyCode::KeyA as u32) { dir -= right; }
            if input.is_key_pressed(KeyCode::Space as u32) { dir += Vec3::Y; } // yüksel
            if input.is_key_pressed(KeyCode::ControlLeft as u32) { dir -= Vec3::Y; } // dal
            kcc.target_velocity = dir.normalize_or_zero() * (kcc.speed * fast);
        }
    }

    // ── Sabit-adım fizik: yüzme kontrolcüsü + rigid buoyancy ────────────────
    const FIXED_DT: f32 = 1.0 / 120.0;
    state.phys_accum += dt.min(0.1);
    let mut steps = 0;
    while state.phys_accum >= FIXED_DT && steps < 16 {
        gizmo::physics::character_controller_system(world, FIXED_DT); // dalgıcı yüzdürür
        gizmo::physics::oxygen_system(world, FIXED_DT); // kafa batıksa hava tüken, yüzeyde dol
        gizmo::systems::cpu_physics_step_system(world, FIXED_DT); // yüzen kutulara buoyancy
        state.phys_accum -= FIXED_DT;
        steps += 1;
    }

    // Oksijeni HUD'a oku.
    if let Some(q) = world.query::<&gizmo::physics::Oxygen>() {
        if let Some(o) = q.get(state.swimmer.id()) {
            state.oxygen_frac = o.fraction();
            state.oxygen_secs = o.current;
        }
    }

    // ── Kamera dalgıcın kafasında (FPS) ─────────────────────────────────────
    // Kamera pozu: normalde dalgıcın kafası (FPS); --shot modunda su üstünden sinematik ocean
    // vantajı (grazing açı → Gerstner dalgalar + Fresnel + yüzeyde sallanan kutular kadraja girer).
    let (head, cam_rot) = if state.shot {
        let pos = Vec3::new(2.5, 1.4, 12.0);
        let yaw = -std::f32::consts::FRAC_PI_2 - 0.16;
        let pitch = -0.30;
        (pos, Quat::from_rotation_y(-yaw) * Quat::from_rotation_x(pitch))
    } else {
        let h = world
            .borrow::<Transform>()
            .get(state.swimmer.id())
            .map(|t| t.position + Vec3::new(0.0, 0.7, 0.0))
            .unwrap_or(Vec3::new(0.0, 0.7, 8.0));
        (h, Quat::from_rotation_y(-state.cam_yaw) * Quat::from_rotation_x(state.cam_pitch))
    };

    // HUD: kamera derinliği + su-altı durumu (render'daki su-altı tespitiyle aynı mantık).
    state.underwater = head.y < WATER_SURFACE_Y;
    state.depth = (WATER_SURFACE_Y - head.y).max(0.0);

    // Su-altı ses boğma: kamera yüzeyin altındaysa ambient kısılıp hafif boğuklaşır (idempotent).
    if let Some(audio) = &mut state.audio {
        audio.set_underwater(state.underwater);
        audio.update();
    }
    if let Some(mut q) =
        world.query_mut::<(gizmo::core::query::Mut<Transform>, gizmo::core::query::Mut<Camera>)>()
    {
        for (_, (mut t, mut c)) in q.iter_mut() {
            t.position = head;
            t.rotation = cam_rot;
            t.update_local_matrix();
            c.yaw = state.cam_yaw;
            c.pitch = state.cam_pitch;
        }
    }
}

fn render(
    world: &mut World,
    state: &WaterState,
    encoder: &mut gizmo::wgpu::CommandEncoder,
    view: &gizmo::wgpu::TextureView,
    renderer: &mut Renderer,
    _light_time: f32,
) {
    renderer.gpu_fluid = None;
    renderer.gpu_physics = None;

    // --shot modu: sahne yerleşsin diye ~90 kare bekle, sonra offscreen bir frame'i GPU'dan okuyup
    // raw RGBA olarak diske yaz ve çık (headless ekran görüntüsü — X/Wayland yakalama gerekmez).
    if state.shot && !state.shot_done && state.frame >= 140 {
        capture_and_save(world, renderer, "/tmp/water_demo_shot.rgba");
        std::process::exit(0);
    }

    gizmo::systems::default_render_pass(world, encoder, view, renderer);
}

/// Sahneyi offscreen bir dokuya render edip GPU'dan okur, BGRA→RGBA çevirip `raw_path`'e yazar.
/// (Golden-readback deseninin demo-yerel kopyası; ImageMagick ile PNG'ye çevrilir.)
fn capture_and_save(world: &mut World, renderer: &mut Renderer, raw_path: &str) {
    use gizmo::wgpu;
    let (w, h) = (renderer.config.width, renderer.config.height);
    let fmt = renderer.config.format;

    let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("shot-target"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: fmt,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let tview = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut enc = renderer
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("shot-enc") });
    gizmo::systems::default_render_pass(world, &mut enc, &tview, renderer);

    let bpp = 4u32;
    let unpadded = w * bpp;
    let padded = unpadded.div_ceil(256) * 256; // bytes_per_row 256-hizalı olmalı
    let staging = renderer.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("shot-buf"),
        size: (padded * h) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    renderer.queue.submit(Some(enc.finish()));

    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |v| {
        let _ = tx.send(v);
    });
    let _ = renderer
        .device
        .poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
    rx.recv().unwrap().unwrap();
    let data = slice.get_mapped_range();

    let is_bgra = matches!(
        fmt,
        wgpu::TextureFormat::Bgra8UnormSrgb | wgpu::TextureFormat::Bgra8Unorm
    );
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for row in 0..h as usize {
        let s = row * padded as usize;
        let line = &data[s..s + unpadded as usize];
        for px in line.chunks_exact(4) {
            if is_bgra {
                out.extend_from_slice(&[px[2], px[1], px[0], px[3]]); // BGRA→RGBA
            } else {
                out.extend_from_slice(px);
            }
        }
    }
    drop(data);
    staging.unmap();
    std::fs::write(raw_path, &out).unwrap();
    println!("SHOT_WRITTEN path={raw_path} {w}x{h} bytes={} fmt={fmt:?}", out.len());
}

fn ui(_world: &mut World, state: &mut WaterState, ctx: &gizmo::egui::Context) {
    use gizmo::egui;
    egui::Window::new("🌊 Su Sistemi Demosu").default_pos([10.0, 10.0]).show(ctx, |ui| {
        ui.label(if state.underwater { "Durum: SU ALTINDA 🐠" } else { "Durum: yüzeyde ☀" });
        ui.label(format!("Derinlik: {:.1} m", state.depth));
        // Oksijen barı (kafa batıkken azalır, yüzeyde dolar).
        let (col, warn) = if state.oxygen_frac < 0.25 {
            (egui::Color32::from_rgb(230, 80, 80), "  — NEFES AL!")
        } else {
            (egui::Color32::from_rgb(80, 180, 230), "")
        };
        ui.add(
            egui::ProgressBar::new(state.oxygen_frac)
                .fill(col)
                .text(format!("Hava: {:.0} s{}", state.oxygen_secs, warn)),
        );
        if state.underwater {
            ui.label("🔇 Ses: boğuk (su altı)");
        } else {
            ui.label("🔊 Ses: normal");
        }
        ui.separator();
        ui.label("WASD = yüz (baktığın yöne)");
        ui.label("Space = yüksel · Sol-Ctrl = dal");
        ui.label("Shift = hızlı · sağ-tık+fare = bak");
        ui.separator();
        ui.label("Gerstner dalga · FluidZone buoyancy");
        ui.label("Yüzme kontrolcüsü · su-altı sisi");
    });
}

fn main() {
    gizmo::app::setup_panic_hook();
    App::<WaterState>::new("Gizmo — Su Sistemi Demosu", 1280, 720)
        .set_setup(setup)
        .set_update(update)
        .set_render(render)
        .set_ui(ui)
        .run()
        .expect("uygulama çalıştırılamadı");
}
