use crate::protocol::{connection_config, PROTOCOL_ID};
use renet::RenetClient;
use renet_netcode::{ClientAuthentication, NetcodeClientTransport};
use std::net::UdpSocket;
use std::time::{Duration, SystemTime};

pub struct NetworkClient {
    pub client: RenetClient,
    pub transport: NetcodeClientTransport,
}

impl NetworkClient {
    pub fn new(server_addr: &str) -> Self {
        let client = RenetClient::new(connection_config());

        let server_addr: std::net::SocketAddr = server_addr.parse().unwrap();
        let socket = UdpSocket::bind("0.0.0.0:0").unwrap();
        let current_time = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap();

        let client_id = current_time.as_millis() as u64; // Simple random ID for now
        let authentication = ClientAuthentication::Unsecure {
            client_id,
            protocol_id: PROTOCOL_ID,
            server_addr,
            user_data: None,
        };

        let transport = NetcodeClientTransport::new(current_time, authentication, socket).unwrap();

        Self { client, transport }
    }

    pub fn update(&mut self, dt_secs: f64) {
        let dt = Duration::from_secs_f64(dt_secs);
        if let Err(_e) = self.transport.update(dt, &mut self.client) {
            // tracing::info!("Network error: {}", e);
        }
    }

    pub fn send_packets(&mut self) {
        if let Err(_e) = self.transport.send_packets(&mut self.client) {
            // log error optionally
        }
    }
}
