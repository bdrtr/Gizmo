//! The client-server netcode over **real** sockets on 127.0.0.1: a client connects, sends an
//! input, the server reads it and broadcasts a world state, the client reads that — all through
//! the crate's own calls, no `renet` and no serializer in the test.
//!
//! No test or game had connected the two ends before 2026-11-04, and a game that wanted to had to
//! depend on `renet` and a serializer itself and agree on a byte format by hand.

#![cfg(feature = "client-server")]

use gizmo_net::client_server::client::NetworkClient;
use gizmo_net::client_server::protocol::{ClientMessage, PlayerInput, ServerChannel, ServerMessage, TransformData};
use gizmo_net::client_server::server::NetworkServer;
use std::collections::HashMap;

fn free_port() -> u16 {
    std::net::UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// Drives both ends until `done` says so, or about two seconds pass.
fn pump(server: &mut NetworkServer, client: &mut NetworkClient, mut done: impl FnMut(&mut NetworkServer, &mut NetworkClient) -> bool) -> bool {
    for _ in 0..600 {
        client.update(1.0 / 60.0);
        server.update(1.0 / 60.0);
        if done(server, client) {
            return true;
        }
        client.send_packets();
        server.send_packets();
        std::thread::sleep(std::time::Duration::from_millis(3));
    }
    false
}

#[test]
fn a_client_and_a_server_talk_over_loopback() {
    let port = free_port();
    let mut server = NetworkServer::new(&format!("127.0.0.1:{port}")).expect("server");
    let mut client = NetworkClient::new(&format!("127.0.0.1:{port}"), 7).expect("client");
    assert!(pump(&mut server, &mut client, |_, c| c.is_connected()), "the client never connected");
    assert!(pump(&mut server, &mut client, |s, _| s.clients() == vec![7]), "the server does not list the client");

    let input = PlayerInput { tick: 12, move_x: 0.5, move_z: -1.0, jump: true, dt: 1.0 / 60.0 };
    client.send(&ClientMessage::Input(input)).unwrap();
    let mut got = Vec::new();
    assert!(
        pump(&mut server, &mut client, |s, _| {
            got.extend(s.receive(7));
            !got.is_empty()
        }),
        "the server never received the input"
    );
    assert!(matches!(got.as_slice(), [ClientMessage::Input(i)] if *i == input), "{got:?}");

    let state = ServerMessage::WorldStateUpdate {
        server_tick: 12,
        players: HashMap::from([(7, TransformData { position: [1.0, 2.0, 3.0], rotation: [0.0, 0.0, 0.0, 1.0] })]),
    };
    server.broadcast(ServerChannel::Unreliable, &state).unwrap();
    let mut heard = Vec::new();
    assert!(
        pump(&mut server, &mut client, |_, c| {
            heard.extend(c.receive());
            !heard.is_empty()
        }),
        "the client never received the state"
    );
    match heard.first() {
        Some(ServerMessage::WorldStateUpdate { server_tick: 12, players }) => {
            assert_eq!(players.get(&7).map(|t| t.position), Some([1.0, 2.0, 3.0]));
        }
        other => panic!("{other:?}"),
    }
}

/// The codec round-trips, and refuses bytes that are not a message.
#[test]
fn the_messages_have_one_wire_format() {
    let m = ClientMessage::Input(PlayerInput { tick: 3, move_x: 0.25, move_z: 0.0, jump: false, dt: 1.0 / 60.0 });
    let bytes = m.encode().unwrap();
    assert!(matches!(ClientMessage::decode(&bytes), Some(ClientMessage::Input(i)) if i.tick == 3 && i.move_x == 0.25));
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(ClientMessage::decode(&trailing).is_none(), "bytes left over must not pass");
    assert!(ServerMessage::decode(&[0xff, 0xff, 0xff, 0xff]).is_none());
}
