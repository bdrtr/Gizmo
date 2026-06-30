//! Gerçek-UDP P2P deterministik rollback istemcisi (Faz 3 — "gerçek istemci binary'si").
//!
//! Yeni `RollbackSession` + deterministik `PhysicsWorld::snapshot` üzerine kuruludur (eski
//! ECS-tabanlı manuel döngü warm-start'ı kaybediyordu). İki örneği AYRI süreçlerde, çapraz
//! portlarla çalıştırın; gerçek UDP üzerinden girdi alışverişi yapıp rollback ile senkron
//! kalırlar.
//!
//! NOT (GGPO davranışı): anlık (frontier) `state_hash`, uzak girdi henüz ONAYLANMADIĞI
//! için TAHMİN içerir → iki tarafta canlı tick'lerde FARKLI olabilir; onaylı geçmiş ve
//! (tüm girdiler geldikten sonra) SON hash yakınsar/eşleşir. Bit-bit rigorlu kanıt:
//! `cargo test -p gizmo-net --features rollback` → `two_peers_converge_under_lag_and_packet_loss`
//! (tam kontrollü loopback; iki peer ground-truth'a state_hash-eşit yakınsar).
//!
//! Çalıştırma (iki terminal):
//!   cargo run -p gizmo-net --features rollback --example p2p_rollback_test -- 8000 8001 0
//!   cargo run -p gizmo-net --features rollback --example p2p_rollback_test -- 8001 8000 1
//!
//! Argümanlar: <local_port> <remote_port> <local_player_id (0|1)>

use gizmo_physics_rigid::BodyHandle;
use gizmo_math::Vec3;
use gizmo_net::rollback::{ApplyInput, PlayerInput, RollbackSession, UdpTransport};
use gizmo_physics_core::components::{Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::PhysicsWorld;
use std::net::SocketAddr;
use std::thread::sleep;
use std::time::Duration;

const DT: f32 = 1.0 / 60.0;

// player 0 → cisim idx 1, player 1 → cisim idx 2 (zemin idx 0).
fn body_of(player_id: u32) -> usize {
    if player_id == 0 { 1 } else { 2 }
}

fn build_scene() -> PhysicsWorld {
    let mut w = PhysicsWorld::new();
    let mut g = RigidBody::new_static();
    g.wake_up();
    w.add_body(
        BodyHandle::from_id(0),
        g,
        Transform::new(Vec3::new(0.0, -1.0, 0.0)),
        Velocity::default(),
        Collider::box_collider(Vec3::new(20.0, 1.0, 20.0)),
    );
    for id in 1..=2u32 {
        let mut rb = RigidBody::new(1.0, true);
        rb.wake_up();
        let col = Collider::box_collider(Vec3::splat(0.5));
        rb.update_inertia_from_collider(&col);
        w.add_body(
            BodyHandle::from_id(id),
            rb,
            Transform::new(Vec3::new(id as f32 * 1.5 - 1.5, 0.5, 0.0)),
            Velocity::default(),
            col,
        );
    }
    w
}

fn apply(w: &mut PhysicsWorld, player_id: u32, input: &PlayerInput) {
    let idx = body_of(player_id);
    if input.joystick_x != 0 && w.rigid_bodies[idx].is_sleeping {
        w.rigid_bodies[idx].wake_up();
    }
    let inv_m = w.rigid_bodies[idx].inv_mass();
    w.velocities[idx].linear.x += (input.joystick_x as f32 / 127.0) * 2.0 * inv_m;
}

// Her oyuncunun KENDİ scripted girdisi (deterministik; insan girdisi olmadan senkron gösterimi).
fn local_input(player_id: u32, tick: u64) -> PlayerInput {
    let phase = if player_id == 0 { 0 } else { 30 };
    let jx = if ((tick + phase) / 30).is_multiple_of(2) { 90 } else { -90 };
    PlayerInput { tick, buttons: 0, joystick_x: jx, joystick_y: 0 }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("Kullanım: ... -- <local_port> <remote_port> <local_id 0|1>");
        return;
    }
    let local_port: u16 = args[1].parse().expect("geçersiz local_port");
    let remote_port: u16 = args[2].parse().expect("geçersiz remote_port");
    let local_id: u32 = args[3].parse().expect("geçersiz local_id");
    let remote_id = 1 - local_id;

    let mut transport = UdpTransport::bind(local_port).expect("UDP bind başarısız");
    let remote: SocketAddr = format!("127.0.0.1:{remote_port}").parse().unwrap();
    transport.set_remote(remote);

    println!(
        "P2P rollback düğümü: port {local_port} → {remote_port}, oyuncu {local_id}. \
         Diğer örnekle senkron için state_hash'ler eşleşmeli."
    );

    let mut session = RollbackSession::new(build_scene(), transport, local_id, remote_id, 120, DT);
    let apply_fn: &ApplyInput = &apply;

    for tick in 0..600u64 {
        let inp = local_input(local_id, tick);
        session.advance(inp, apply_fn);
        if tick % 60 == 0 {
            println!("[tick {tick:3}] state_hash = {:016X}", session.state_hash());
        }
        sleep(Duration::from_millis(16)); // ~60 Hz
    }
    println!("Bitti. Son state_hash = {:016X}", session.state_hash());
}
