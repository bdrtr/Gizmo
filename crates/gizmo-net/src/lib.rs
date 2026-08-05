#![warn(missing_docs)]
//! (`missing_docs` is a RATCHET, not a suggestion. The CI lint gate runs with `-D warnings`,
//! so every public item in this crate must carry a doc comment or the build fails. This crate
//! is Stage A — the dependency-light core that goes to 1.x first — and its documented surface
//! is part of that promise. Do not silence this with `#[allow]`; write the doc.)

//! Gizmo networking — özellik bayraklarıyla (feature flags) seçilen iki bağımsız netcode mimarisi.
//!
//! - **`client-server`**: `renet` tabanlı, otoriter sunuculu mimari; istemci tahmini
//!   (prediction) ve snapshot interpolasyonu içerir. Adanmış sunuculu oyunlar için.
//! - **`rollback`**: eşler-arası (P2P) deterministik rollback (GGPO tarzı); fizik
//!   durumunu yakalayıp geri yükler. Dövüş/lockstep tarzı oyunlar için.
//!
//! İki mimari de aynı anda etkinleştirilebilir ama birbirinden bağımsızdır.

#[cfg(feature = "client-server")]
pub mod client_server;

#[cfg(feature = "rollback")]
pub mod rollback;
