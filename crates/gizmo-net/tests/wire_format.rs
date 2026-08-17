//! The rollback wire format, pinned byte for byte.
//!
//! # Why this exists
//!
//! `bincode` 1.x is unmaintained — `cargo deny` flags it, and it is `gizmo-net`'s only direct
//! advisory (ENGINE.md §7). Moving to 2.x is a dependency swap everywhere except in one
//! respect: **bincode 2's default configuration is not bincode 1's**. `config::standard()` uses
//! variable-length integers, so the same `NetworkPacket` encodes to different bytes and two peers
//! on different builds silently stop understanding each other. `config::legacy()` is the one that
//! reproduces 1.x.
//!
//! Round-trip tests cannot tell those apart: encode-then-decode succeeds under either config.
//! Only fixed bytes can, so the expectations below were **captured from bincode 1.3.3 before the
//! migration** and are unchanged by it. If they ever need editing, the wire format changed and
//! every peer has to be rebuilt — `packet.rs`'s module doc explains why there is no handshake to
//! soften that.
//!
//! What the bytes say: a 4-byte little-endian variant index, then fixed-width little-endian
//! fields in declaration order. The variant index is why `NetworkPacket`'s variant *order* is
//! part of the format.

use gizmo_net::rollback::{NetworkPacket, PlayerInput};

/// Encodes exactly as bincode 1.3.3 did. Captured 2026-08-07, pre-migration.
#[test]
fn input_packet_bytes_are_unchanged() {
    let packet = NetworkPacket::Input(PlayerInput {
        tick: 0x0102_0304_0506_0708,
        buttons: 0xDEAD_BEEF,
        joystick_x: -3,
        joystick_y: 127,
    });

    // Values chosen so every field is byte-order-sensitive and sign-sensitive: the tick's bytes
    // are all distinct, `buttons` has the high bit set, and `joystick_x` is negative (-3 = 0xFD)
    // so a signedness mistake shows up rather than cancelling.
    assert_eq!(
        gizmo_net::rollback::transport::encode_packet(&packet).unwrap(),
        vec![
            0, 0, 0, 0, // variant 0 (Input), u32 LE
            8, 7, 6, 5, 4, 3, 2, 1, // tick: u64 LE
            239, 190, 173, 222, // buttons: u32 LE (0xDEADBEEF)
            253,  // joystick_x: i8, -3
            127,  // joystick_y: i8
        ]
    );
}

/// The second variant, so the test also pins the variant *index*, not just the payload layout.
#[test]
fn ping_packet_bytes_are_unchanged() {
    let packet = NetworkPacket::Ping {
        timestamp: 0x1122_3344_5566_7788,
    };
    assert_eq!(
        gizmo_net::rollback::transport::encode_packet(&packet).unwrap(),
        vec![
            1, 0, 0, 0, // variant 1 (Ping), u32 LE
            136, 119, 102, 85, 68, 51, 34, 17, // timestamp: u64 LE
        ]
    );
}

/// Decoding is the half the transport actually depends on for correctness: a packet that
/// encodes right but decodes wrong is worse than one that fails outright.
#[test]
fn bytes_from_the_wire_decode_to_the_same_packet() {
    let bytes = vec![
        0, 0, 0, 0, 8, 7, 6, 5, 4, 3, 2, 1, 239, 190, 173, 222, 253, 127,
    ];
    let decoded = gizmo_net::rollback::transport::decode_packet(&bytes).expect("decodes");
    match decoded {
        NetworkPacket::Input(input) => {
            assert_eq!(input.tick, 0x0102_0304_0506_0708);
            assert_eq!(input.buttons, 0xDEAD_BEEF);
            assert_eq!(input.joystick_x, -3);
            assert_eq!(input.joystick_y, 127);
        }
        other => panic!("expected Input, got {other:?}"),
    }
}

/// A truncated datagram must fail rather than decode into something plausible — this is the
/// path `UdpTransport::poll_events` relies on to drop garbage instead of feeding the session a
/// half-read input.
#[test]
fn a_truncated_packet_is_rejected() {
    let full = vec![
        0, 0, 0, 0, 8, 7, 6, 5, 4, 3, 2, 1, 239, 190, 173, 222, 253, 127,
    ];
    for cut in 1..full.len() {
        assert!(
            gizmo_net::rollback::transport::decode_packet(&full[..cut]).is_err(),
            "a {cut}-byte prefix decoded; truncated datagrams must be rejected"
        );
    }
}
