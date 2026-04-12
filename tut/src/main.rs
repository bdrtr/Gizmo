/// Gizmo Yarış — Burnout Revenge Lone Peak Haritası
///
/// Kontroller:
///   W / ↑   — Gaz         S / ↓   — Fren/Geri
///   A / ←   — Sola        D / →   — Sağa
///   Space   — El freni    R       — Sıfırla / Yeni yarış
use gizmo::physics::components::{PhysicsConfig, RigidBody, Velocity};
use gizmo::physics::shape::Collider;
use gizmo::physics::system::{physics_collision_system, PhysicsSolverState};
use gizmo::physics::vehicle::{physics_vehicle_system, VehicleController, Wheel};
use gizmo::physics::{
    integration::{physics_apply_forces_system, physics_movement_system},
    race_ai::{race_ai_system, RaceAI},
    JointWorld,
};
use gizmo::prelude::*;

// ─── Oyun Durumu ──────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
enum Phase { Countdown(f32), Racing, Finished(f32) }

struct GameState {
    player_id: u32,
    camera_id: u32,
    ai_id:     u32,
    phase: Phase,
    race_timer:      f32,
    player_laps:     u32,
    player_last_wp:  usize,
    player_wp_total: u32,
    ai_laps:         u32,
    spawn_pos: Vec3,
    spawn_rot: Quat,
    physics_acc: f32,
}

const TOTAL_LAPS: u32  = 3;
const PHYSICS_HZ: f32  = 120.0;

// ─── Burnout Lone Peak — Harita Parametreleri ─────────────────────────────────
//
// Modeli çalıştırınca bu değerleri kameradan bakarak ayarla.
// Başlangıç için pist genellikle Y≈0 seviyesinde, X/Z boyutu ~200-400 birim.
//
const MAP_PATH:  &str = "demo/assets/burnout_revenge_lone_peak.glb";
const MAP_SCALE: Vec3 = Vec3::new(1.0, 1.0, 1.0); // Gerekirse (0.1,0.1,0.1) ile küçült

// Araç başlangıç noktası (modeli açınca cam ile bakarak ayarla)
const START_POS: Vec3 = Vec3::new(0.0, 3.0, 0.0);
const START_YAW: f32  = 0.0; // Radyan. 0 = +Z yönüne bak

// ─── Waypoint'ler ─────────────────────────────────────────────────────────────
// İlk çalıştırmada araç pist üzerinde gezinirken pozisyonları not al,
// sonra buraya yaz. Şimdilik büyük oval placeholder.
fn track_waypoints() -> Vec<Vec3> {
    let n  = 24usize;
    let rx = 80.0_f32;
    let rz = 120.0_f32;
    (0..n).map(|i| {
        let a = (i as f32 / n as f32) * std::f32::consts::TAU;
        Vec3::new(a.cos() * rx, 1.0, a.sin() * rz)
    }).collect()
}

// ─────────────────────────────────────────────────────────────────────────────

fn main() {
    App::<GameState>::new("Gizmo Yarış — Burnout Lone Peak", 1280, 720)
        .set_setup(|world, renderer| {
            world.insert_resource(PhysicsSolverState::new());
            world.insert_resource(JointWorld::new());
            world.insert_resource(PhysicsConfig { ground_y: -2.0 });

            let mut cmd = Commands::new(world, renderer);

            // ── Çevre ──
            cmd.spawn_skybox(Color::rgb(0.60, 0.70, 0.85)).with_name("Skybox");
            cmd.spawn_sun(Vec3::new(-0.6, -1.0, 0.4), Color::rgb(1.0, 0.95, 0.88), 2.5)
                .with_name("Sun");

            // ── GLB Harita ──
            // Görsel olarak yüklenir; fizik zemini + duvarlar ayrı AABB ile eklenir.
            {
                cmd.spawn_gltf(Vec3::ZERO, MAP_PATH).with_name("Track");
            }

            // ── Fizik Zemini ──
            // Burnout Lone Peak genellikle bir şehir + dağ yolunu temsil eder.
            // Modelin zemin Y ≈ 0. Büyük düz AABB ile kaplıyoruz.
            // NOT: Gerçek mesh collision yok; zemin düzdür.
            {
                let e = cmd.world.spawn();
                cmd.world.add_component(e, Transform::new(Vec3::new(0.0, -0.2, 0.0)));
                cmd.world.add_component(e, RigidBody::new_static());
                cmd.world.add_component(e, Collider::new_aabb(500.0, 0.2, 500.0));
            }

            // ── Araçlar ──
            let start_pos = START_POS;
            let start_rot = Quat::from_axis_angle(Vec3::Y, START_YAW);

            let player_e  = spawn_car(cmd.world, cmd.renderer, start_pos, start_rot, Color::rgb(0.9, 0.15, 0.1));
            let player_id = player_e.id();
            cmd.world.add_component(player_e, EntityName("Oyuncu".into()));

            let ai_pos    = start_pos + Vec3::new(4.0, 0.0, 0.0);
            let ai_e      = spawn_car(cmd.world, cmd.renderer, ai_pos, start_rot, Color::rgb(0.1, 0.3, 0.9));
            let ai_id     = ai_e.id();
            cmd.world.add_component(ai_e, EntityName("AI".into()));
            cmd.world.add_component(ai_e, RaceAI::new(track_waypoints(), 0.80, 10.0));

            // ── Chase Kamera ──
            let cam_e     = cmd.world.spawn();
            let camera_id = cam_e.id();
            cmd.world.add_component(cam_e, Transform::new(start_pos + Vec3::new(0.0, 5.0, -13.0)));
            cmd.world.add_component(cam_e, Camera {
                fov: 72.0_f32.to_radians(), near: 0.1, far: 2000.0,
                yaw: 0.0, pitch: -0.2, primary: true,
            });

            GameState {
                player_id, camera_id, ai_id,
                phase: Phase::Countdown(4.0),
                race_timer: 0.0, player_laps: 0,
                player_last_wp: 0, player_wp_total: 0, ai_laps: 0,
                spawn_pos: start_pos, spawn_rot: start_rot,
                physics_acc: 0.0,
            }
        })
        .set_update(|world, state, dt, input| {
            // ── Sabit Fizik Adımı ─────────────────────────────────
            state.physics_acc += dt;
            let step = 1.0 / PHYSICS_HZ;
            while state.physics_acc >= step {
                physics_apply_forces_system(world, step);
                physics_vehicle_system(world, step);
                race_ai_system(world, step);
                physics_collision_system(world, step);
                physics_movement_system(world, step);
                state.physics_acc -= step;
            }

            // ── Araç Girişi ────────────────────────────────────────
            let racing = state.phase == Phase::Racing;
            if let Some(mut vcs) = world.borrow_mut::<VehicleController>() {
                if let Some(vc) = vcs.get_mut(state.player_id) {
                    if racing {
                        let gas   = input.pressed(Key::ArrowUp)   || input.pressed(Key::KeyW);
                        let rev   = input.pressed(Key::ArrowDown)  || input.pressed(Key::KeyS);
                        let left  = input.pressed(Key::ArrowLeft)  || input.pressed(Key::KeyA);
                        let right = input.pressed(Key::ArrowRight) || input.pressed(Key::KeyD);
                        vc.engine_force   = if gas { 16000.0 } else if rev { -8000.0 } else { 0.0 };
                        vc.brake_force    = if input.pressed(Key::Space) { 25000.0 } else { 0.0 };
                        vc.steering_angle = if left { 0.42 } else if right { -0.42 } else {
                            vc.steering_angle * 0.78
                        };
                    } else {
                        vc.engine_force = 0.0; vc.brake_force = 30000.0; vc.steering_angle = 0.0;
                    }
                }
            }

            // ── Otomatik Respawn ────────────────────────────────────
            let fell = world.borrow::<Transform>()
                .and_then(|ts| ts.get(state.player_id).map(|t| t.position.y < -5.0))
                .unwrap_or(false);
            if fell { reset_car(world, state.player_id, state.spawn_pos, state.spawn_rot); }

            // R → sıfırla / yeni yarış
            if input.just_pressed(Key::KeyR) {
                match &state.phase {
                    Phase::Finished(_) => {
                        reset_car(world, state.player_id, state.spawn_pos, state.spawn_rot);
                        reset_car(world, state.ai_id, state.spawn_pos + Vec3::new(4.0,0.0,0.0), state.spawn_rot);
                        if let Some(mut ais) = world.borrow_mut::<RaceAI>() {
                            if let Some(ai) = ais.get_mut(state.ai_id) {
                                ai.laps_completed = 0; ai.total_wp_passed = 0;
                            }
                        }
                        state.race_timer = 0.0; state.player_laps = 0;
                        state.player_last_wp = 0; state.player_wp_total = 0; state.ai_laps = 0;
                        state.phase = Phase::Countdown(4.0);
                    }
                    _ => reset_car(world, state.player_id, state.spawn_pos, state.spawn_rot),
                }
            }

            // ── Phase Güncelleme ─────────────────────────────────────
            let waypoints = track_waypoints();
            let wp_count  = waypoints.len();
            match &mut state.phase {
                Phase::Countdown(t) => { *t -= dt; if *t <= 0.0 { state.phase = Phase::Racing; } }
                Phase::Racing => {
                    state.race_timer += dt;
                    let pp = world.borrow::<Transform>()
                        .and_then(|ts| ts.get(state.player_id).map(|t| t.position));
                    if let Some(pos) = pp {
                        let next = state.player_last_wp % wp_count;
                        let wp = waypoints[next];
                        if ((pos.x-wp.x).powi(2) + (pos.z-wp.z).powi(2)).sqrt() < 12.0 {
                            state.player_last_wp += 1; state.player_wp_total += 1;
                            if state.player_last_wp >= wp_count {
                                state.player_last_wp = 0; state.player_laps += 1;
                                println!("[Yarış] TUR {}! {:.2}s", state.player_laps, state.race_timer);
                                if state.player_laps >= TOTAL_LAPS {
                                    let t = state.race_timer;
                                    state.phase = Phase::Finished(t);
                                }
                            }
                        }
                    }
                    if let Some(ais) = world.borrow::<RaceAI>() {
                        if let Some(ai) = ais.get(state.ai_id) { state.ai_laps = ai.laps_completed; }
                    }
                }
                Phase::Finished(_) => {}
            }

            // ── Chase Kamera ─────────────────────────────────────────
            let car_info = world.borrow::<Transform>()
                .and_then(|ts| ts.get(state.player_id).map(|t| (t.position, t.rotation)));
            if let Some((cp, cr)) = car_info {
                let fwd = cr.mul_vec3(Vec3::new(0.0, 0.0, 1.0));
                let tgt = cp - fwd * 13.0 + Vec3::new(0.0, 5.5, 0.0);
                if let Some(mut ts) = world.borrow_mut::<Transform>() {
                    if let Some(ct) = ts.get_mut(state.camera_id) {
                        ct.position = ct.position.lerp(tgt, (dt * 5.0).min(1.0));
                        ct.update_local_matrix();
                    }
                }
                let ldir = world.borrow::<Transform>()
                    .and_then(|ts| ts.get(state.camera_id).map(|ct| {
                        (cp + Vec3::new(0.0, 1.2, 0.0) - ct.position).normalize()
                    }));
                if let Some(d) = ldir {
                    if let Some(mut cams) = world.borrow_mut::<Camera>() {
                        if let Some(cam) = cams.get_mut(state.camera_id) {
                            cam.yaw   = d.x.atan2(d.z);
                            cam.pitch = (-d.y).asin().clamp(-0.5, 0.35);
                        }
                    }
                }
            }
        })
        .set_render(|world, state, encoder, view, renderer, _t| {
            default_render_pass(world, encoder, view, renderer);

            let speed = world.borrow::<Velocity>()
                .and_then(|vs| vs.get(state.player_id).map(|v| v.linear.length() * 3.6))
                .unwrap_or(0.0);
            match &state.phase {
                Phase::Countdown(t) => {
                    let s = if *t > 3.0 { "HAZIR OL..." }
                              else if *t > 2.0 { "3" } else if *t > 1.0 { "2" } else { "1 — GİT!" };
                    eprintln!("\r[GİZMO YARIŞ] {}", s);
                }
                Phase::Racing => {
                    let m = state.race_timer as u32 / 60;
                    let s = state.race_timer % 60.0;
                    eprint!("\r Tur {}/{TOTAL_LAPS} | {:02}:{:05.2} | {:.0} km/h | AI: {} tur   ",
                        state.player_laps, m, s, speed, state.ai_laps);
                }
                Phase::Finished(t) => {
                    eprintln!("\r[BİTİŞ!] {:.2}s  |  R = Yeni Yarış          ", t);
                }
            }
        })
        .run();
}

// ─── Araç ─────────────────────────────────────────────────────────────────────

fn spawn_car(
    world: &mut World,
    renderer: &gizmo::renderer::Renderer,
    pos: Vec3, rot: Quat, color: Color,
) -> Entity {
    use gizmo::renderer::{asset::AssetManager, components::MeshRenderer};
    let entity = world.spawn();

    let mut t = Transform::new(pos).with_rotation(rot).with_scale(Vec3::new(1.8, 0.55, 4.0));
    t.update_local_matrix();
    world.add_component(entity, t);

    let mesh = AssetManager::create_cube(&renderer.device);
    let mut am = AssetManager::new();
    let tex = am.create_white_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout);
    world.add_component(entity, mesh);
    world.add_component(entity, Material::new(tex).with_pbr(color.to_vec4(), 0.3, 0.1));
    world.add_component(entity, MeshRenderer::new());

    let mut rb = RigidBody::new(900.0, 0.02, 0.7, true);
    rb.calculate_box_inertia(1.8, 0.55, 4.0);
    rb.ccd_enabled = true;
    world.add_component(entity, rb);
    world.add_component(entity, Velocity::new(Vec3::ZERO));
    world.add_component(entity, Collider::new_aabb(0.9, 0.275, 2.0));

    let rest = 0.50; let k = 28000.0; let d = 3500.0; let r = 0.36;
    let mut vc = VehicleController::new();
    vc.lateral_grip = 12000.0; vc.steering_force_mult = 10000.0;
    vc.anti_slide_force = 9000.0; vc.drag_coefficient = 0.20;
    vc.add_wheel(Wheel::new(Vec3::new(-0.82, -0.28,  1.5), rest, k, d, r));
    vc.add_wheel(Wheel::new(Vec3::new( 0.82, -0.28,  1.5), rest, k, d, r));
    vc.add_wheel(Wheel::new(Vec3::new(-0.82, -0.28, -1.5), rest, k, d, r).with_drive());
    vc.add_wheel(Wheel::new(Vec3::new( 0.82, -0.28, -1.5), rest, k, d, r).with_drive());
    world.add_component(entity, vc);
    entity
}

fn reset_car(world: &mut World, id: u32, pos: Vec3, rot: Quat) {
    if let Some(mut ts) = world.borrow_mut::<Transform>() {
        if let Some(t) = ts.get_mut(id) { t.position = pos; t.rotation = rot; t.update_local_matrix(); }
    }
    if let Some(mut vs) = world.borrow_mut::<Velocity>() {
        if let Some(v) = vs.get_mut(id) { v.linear = Vec3::ZERO; v.angular = Vec3::ZERO; }
    }
    if let Some(mut rbs) = world.borrow_mut::<RigidBody>() {
        if let Some(rb) = rbs.get_mut(id) { rb.wake_up(); }
    }
}
