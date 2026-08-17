//! Authoritative-server (client-server) netcode, built on `renet`.

/// Client-side connection: a renet client bundled with its netcode transport, driven by one
/// `update` / `send_packets` pair per frame.
///
/// Owning both halves together is the point — updating the transport without flushing it, or
/// the reverse, is the usual way a renet integration ends up silently sending nothing.
///
/// No in-repo binary uses this: `server/src/main.rs` is the only reference integration and it
/// drives the server half. Applications supply their own client loop.
pub mod client;
pub mod error;
pub mod interpolation;
pub mod prediction;
pub mod protocol;

/// Authoritative server-side connection: a renet server plus its netcode transport, bound to a
/// public address and capped at 64 clients.
///
/// Authentication is `ServerAuthentication::Unsecure` — there is no token exchange and clients
/// pick their own id, so anything reachable from the open internet needs its own gate in front.
/// See [`protocol::PlayerInput::dt`] for the matching caution about trusting client-sent input.
pub mod server;
