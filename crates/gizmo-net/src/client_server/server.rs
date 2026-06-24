use super::protocol::{connection_config, PROTOCOL_ID};
use renet::RenetServer;
use renet_netcode::{NetcodeServerTransport, ServerAuthentication, ServerConfig};
use std::error::Error;
use std::net::UdpSocket;
use std::time::{Duration, SystemTime};

/// A renet-based authoritative server: bundles the [`RenetServer`] with its netcode transport.
pub struct NetworkServer {
    /// The underlying renet server (per-client message queues, connection state).
    pub server: RenetServer,
    /// The netcode UDP transport accepting and driving client connections.
    pub transport: NetcodeServerTransport,
}

impl NetworkServer {
    /// Verilen genel adreste dinleyen bir sunucu oluşturur.
    ///
    /// Adres ayrıştırma, soket bağlama veya transport kurulumu başarısız olursa
    /// (örn. port kullanımda) panik yerine hata döndürür.
    pub fn new(public_addr: &str) -> Result<Self, Box<dyn Error + Send + Sync>> {
        let server = RenetServer::new(connection_config());

        let public_addr: std::net::SocketAddr = public_addr.parse()?;
        let socket = UdpSocket::bind(public_addr)?;
        let current_time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?;

        let server_config = ServerConfig {
            current_time,
            max_clients: 64,
            protocol_id: PROTOCOL_ID,
            public_addresses: vec![public_addr],
            authentication: ServerAuthentication::Unsecure,
        };

        let transport = NetcodeServerTransport::new(server_config, socket)?;

        Ok(Self { server, transport })
    }

    /// Advances the transport by `dt_secs`, processing incoming client packets. Call once per tick.
    pub fn update(&mut self, dt_secs: f64) {
        let dt = Duration::from_secs_f64(dt_secs);
        // Geçici bir transport hatası tüm sunucu döngüsünü düşürmemeli.
        if let Err(e) = self.transport.update(dt, &mut self.server) {
            tracing::error!("Sunucu taşıma güncellemesi başarısız: {e}");
        }
    }

    /// Flushes queued per-client messages out over the network. Call at the end of each tick.
    pub fn send_packets(&mut self) {
        self.transport.send_packets(&mut self.server);
    }
}
