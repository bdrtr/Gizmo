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
