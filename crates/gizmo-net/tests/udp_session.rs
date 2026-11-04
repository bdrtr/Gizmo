//! The rollback session over **real** UDP sockets on the loopback interface — the path
//! `UdpTransport` takes in a shipped game, which until 2026-11-04 no test or game had run: every
//! session test used the in-memory `LoopbackTransport`.

#![cfg(feature = "rollback")]

use gizmo_math::Vec3;
use gizmo_net::rollback::{NetworkPacket, PlayerInput, RollbackSession, Transport, UdpTransport};
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};
use std::net::SocketAddr;

const DT: f32 = 1.0 / 60.0;

/// An endpoint on an OS-chosen port, and the loopback address a peer reaches it at.
fn endpoint() -> (UdpTransport, SocketAddr) {
    let t = UdpTransport::bind(0).expect("bind an ephemeral port");
    let port = t.local_addr().expect("the bound address").port();
    (t, SocketAddr::from(([127, 0, 0, 1], port)))
}

fn world() -> PhysicsWorld {
    let mut w = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0));
    w.add_body(
        BodyHandle::from_id(0),
        RigidBody::new_static(),
        Transform::new(Vec3::new(0.0, -0.5, 0.0)),
        Velocity::default(),
        Collider::box_collider(Vec3::new(20.0, 0.5, 20.0)),
    );
    for id in 1..=2u32 {
        let col = Collider::sphere(0.5);
        let mut rb = RigidBody::new(1.0, true);
        rb.update_inertia_from_collider(&col);
        w.add_body(BodyHandle::from_id(id), rb, Transform::new(Vec3::new(id as f32 * 2.0 - 3.0, 0.5, 0.0)), Velocity::default(), col);
    }
    w
}

fn apply(w: &mut PhysicsWorld, player: u32, input: &PlayerInput) {
    let idx = 1 + player as usize;
    if input.joystick_x != 0 {
        w.rigid_bodies[idx].wake_up();
        w.velocities[idx].linear.x += input.joystick_x as f32 / 127.0 * 0.5;
    }
}

fn input(player: u32, t: u64) -> PlayerInput {
    let x = if t < 90 { (((t * 7 + player as u64 * 13) % 9) as i8 - 4) * 30 } else { 0 };
    PlayerInput { tick: t, buttons: 0, joystick_x: x, joystick_y: 0 }
}

/// Two sessions over two sockets agree on the state of the same match played in one world.
#[test]
fn two_peers_over_real_udp_agree() {
    let (mut ta, addr_a) = endpoint();
    let (mut tb, addr_b) = endpoint();
    ta.set_remote(addr_b);
    tb.set_remote(addr_a);
    let mut a = RollbackSession::new(world(), ta, 0, 1, 60, DT);
    let mut b = RollbackSession::new(world(), tb, 1, 0, 60, DT);
    let end = 150u64;
    // Loopback delivery is not instant: each frame gives the other side a moment to be read.
    for _ in 0..end * 4 {
        if a.tick < end {
            a.try_advance(input(0, a.tick), &apply, 8);
        } else {
            a.poll_remote();
        }
        if b.tick < end {
            b.try_advance(input(1, b.tick), &apply, 8);
        } else {
            b.poll_remote();
        }
        if a.tick >= end && b.tick >= end {
            break;
        }
        std::thread::sleep(std::time::Duration::from_micros(300));
    }
    let mut truth = world();
    for t in 0..end {
        apply(&mut truth, 0, &input(0, t));
        apply(&mut truth, 1, &input(1, t));
        truth.step(DT).ok();
    }
    assert_eq!((a.tick, b.tick), (end, end), "the peers did not finish");
    // The last inputs may still be in flight; drain them into a final rollback.
    for _ in 0..50 {
        a.poll_remote();
        b.poll_remote();
        std::thread::sleep(std::time::Duration::from_micros(300));
    }
    assert_eq!(a.state_hash(), b.state_hash(), "the peers disagree");
    assert_eq!(a.state_hash(), truth.state_hash(), "the peers agree, but not on the real match");
}

/// Once the peer is known, a packet from anyone else is not the peer's input.
#[test]
fn a_stranger_cannot_speak_for_the_peer() {
    let (mut a, addr_a) = endpoint();
    let (b, addr_b) = endpoint();
    let (stranger, _) = endpoint();
    a.set_remote(addr_b);
    let mut b = b;
    b.set_remote(addr_a);
    let mut stranger = stranger;
    stranger.set_remote(addr_a);
    let forged = NetworkPacket::Input(PlayerInput { tick: 0, buttons: 1, joystick_x: 127, joystick_y: 0 });
    stranger.send(&forged);
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert!(a.poll().is_empty(), "a packet from a stranger was taken as the peer's");
    b.send(&NetworkPacket::Input(PlayerInput::empty(0)));
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert_eq!(a.poll().len(), 1, "the peer's own packet must still arrive");
}
