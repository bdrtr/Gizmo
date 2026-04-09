use renet::RenetServer;
use renet_netcode::{NetcodeServerTransport, ServerAuthentication, ServerConfig};
use std::net::UdpSocket;
use std::time::{Duration, SystemTime};
use crate::protocol::{connection_config, PROTOCOL_ID};

pub struct NetworkServer {
    pub server: RenetServer,
    pub transport: NetcodeServerTransport,
}

impl NetworkServer {
    pub fn new(public_addr: &str) -> Self {
        let server = RenetServer::new(connection_config());

        let public_addr: std::net::SocketAddr = public_addr.parse().unwrap();
        let socket = UdpSocket::bind(public_addr).unwrap();
        let current_time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap();
        
        let server_config = ServerConfig {
            current_time,
            max_clients: 64,
            protocol_id: PROTOCOL_ID,
            public_addresses: vec![public_addr],
            authentication: ServerAuthentication::Unsecure,
        };

        let transport = NetcodeServerTransport::new(server_config, socket).unwrap();

        Self { server, transport }
    }

    pub fn update(&mut self, dt_secs: f64) {
        let dt = Duration::from_secs_f64(dt_secs);
        self.transport.update(dt, &mut self.server).unwrap();
    }

    pub fn send_packets(&mut self) {
        self.transport.send_packets(&mut self.server);
    }
}
