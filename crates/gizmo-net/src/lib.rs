#![deny(clippy::undocumented_unsafe_blocks)]
//! (`undocumented_unsafe_blocks` is a RATCHET: this crate carries no `unsafe` block without a
//! `// SAFETY:` line stating why it is sound, and the lint keeps it that way. Every crate in the
//! workspace except `gizmo-core` is at zero and denies it; `gizmo-core`'s ECS internals are the
//! measured remainder — see docs/ENGINE.md.)
#![warn(missing_docs)]
//! (`missing_docs` is a RATCHET, not a suggestion. The CI lint gate runs with `-D warnings`,
//! so every public item in this crate must carry a doc comment or the build fails. This crate
//! is Stage A — the dependency-light core that goes to 1.x first — and its documented surface
//! is part of that promise. Do not silence this with `#[allow]`; write the doc.)

//! Gizmo networking — two independent netcode architectures, selected by feature flags.
//!
//! - **`client-server`**: an authoritative-server architecture built on `renet`, with client-side
//!   prediction and snapshot interpolation. For games with a dedicated server.
//! - **`rollback`**: peer-to-peer deterministic rollback (GGPO-style), capturing and restoring
//!   physics state. For fighting/lockstep-style games.
//!
//! Both can be enabled at once, and they are independent of each other.

#[cfg(feature = "client-server")]
pub mod client_server;

#[cfg(feature = "rollback")]
pub mod rollback;
