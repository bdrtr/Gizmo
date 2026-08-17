//! Peer-to-peer deterministic rollback netcode (GGPO-style).

/// Per-player ring buffer of [`PlayerInput`] plus the GGPO prediction rule: when a tick
/// has not arrived yet, repeat the last confirmed input rather than stall.
///
/// Slots are addressed by `tick % capacity`, so a tick whose slot has already been
/// recycled reads as "not received" and is predicted — returning the stale occupant
/// instead would let a peer treat an ancient input as confirmed and desync.
pub mod input_buffer;

/// ECS-facing rollback driver: [`RollbackManager`] brackets the host's physics loop with
/// `begin_frame` / `end_frame` and rewinds a `gizmo_core::World` through [`snapshot`]'s
/// ring buffer. It does no networking of its own — the host feeds it remote inputs.
///
/// Rewinding this way restores only position/rotation/velocity/sleep, so the contact
/// solver's warm-start impulses and joint state survive the rewind untouched and the
/// re-simulation can drift from an uninterrupted run. [`RollbackSession`] — which
/// snapshots a whole `PhysicsWorld` instead — exists for exactly that reason and is what
/// the P2P example and the convergence tests use.
pub mod manager;

pub mod packet;
pub mod session;

/// Serde-serializable capture/restore of the ECS world's physics state, plus the ring
/// buffer of past ticks [`manager`] rewinds through.
///
/// Deliberately narrower than `gizmo_physics_rigid::WorldSnapshot` (what [`session`]
/// uses): that one carries solver-internal state — contact warm-start impulses, joint
/// latches, the sub-step accumulator — and never goes over the wire, whereas this one
/// holds only position/rotation/velocity/sleep per entity and is what
/// [`NetworkPacket::FullState`] carries to a resyncing peer.
///
/// `restore` validates each entity's *generation* before writing, so a snapshot taken
/// before a despawn cannot silently overwrite a new entity that reused the same id slot.
pub mod snapshot;

/// Non-blocking UDP socket transport for native (desktop) peers.
///
/// Native-only, at runtime rather than at compile time: `std::net::UdpSocket` does compile
/// for `wasm32-unknown-unknown`, but std has no socket backend there, so `UdpTransport::bind`
/// comes back as `io::ErrorKind::Unsupported`. Packets are bincode-encoded [`NetworkPacket`]s sent
/// unreliably and unordered; the peer address is either set explicitly or learned from
/// the first datagram that arrives (a stand-in for NAT hole punching).
pub mod transport;

pub use input_buffer::{InputBuffer, PlayerInput};
pub use manager::RollbackManager;
pub use packet::NetworkPacket;
pub use session::{ApplyInput, LoopbackTransport, RollbackSession, Transport};
pub use snapshot::{EntityState, PhysicsStateSnapshot, RollbackBuffer};
pub use transport::UdpTransport;
