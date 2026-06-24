//! Eşler-arası (P2P) deterministik rollback netcode (GGPO tarzı).

pub mod input_buffer;
pub mod manager;
pub mod packet;
pub mod session;
pub mod snapshot;
pub mod transport;

pub use input_buffer::{InputBuffer, PlayerInput};
pub use manager::RollbackManager;
pub use packet::NetworkPacket;
pub use session::{ApplyInput, LoopbackTransport, RollbackSession, Transport};
pub use snapshot::{EntityState, PhysicsStateSnapshot, RollbackBuffer};
pub use transport::UdpTransport;
