use super::error::NetError;
use super::protocol::{connection_config, ClientChannel, ClientMessage, ServerChannel, ServerMessage, PROTOCOL_ID};
use renet::RenetClient;
use renet_netcode::{ClientAuthentication, NetcodeClientTransport};
use std::net::UdpSocket;
use std::time::{Duration, SystemTime};

/// A renet-based client: bundles the [`RenetClient`] with its netcode transport.
pub struct NetworkClient {
    /// The underlying renet client (message queues, connection state).
    pub client: RenetClient,
    /// The netcode UDP transport driving the connection.
    pub transport: NetcodeClientTransport,
}

impl NetworkClient {
    /// Creates a client that will connect to a server.
    ///
    /// `client_id` must be supplied by the caller, unique and ideally unpredictable. (The
    /// "current millisecond" that used to be baked in here was open to both collisions and
    /// prediction.)
    ///
    /// Returns an error rather than panicking if address parsing, socket binding or transport
    /// setup fails.
    pub fn new(server_addr: &str, client_id: u64) -> Result<Self, NetError> {
        let client = RenetClient::new(connection_config());

        let server_addr: std::net::SocketAddr = server_addr.parse()?;
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        let current_time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?;

        let authentication = ClientAuthentication::Unsecure {
            client_id,
            protocol_id: PROTOCOL_ID,
            server_addr,
            user_data: None,
        };

        let transport = NetcodeClientTransport::new(current_time, authentication, socket)
            .map_err(|e| NetError::Transport(Box::new(e)))?;

        tracing::info!(server = %server_addr, client_id, "Netcode istemcisi oluşturuldu, sunucuya bağlanılıyor");
        Ok(Self { client, transport })
    }

    /// Advances the client by `dt_secs`, processing incoming packets. Call once per frame.
    ///
    /// Both halves — see `NetworkServer::update` for what the missing one cost. `RenetClient::update`
    /// is the connection clock (reliability, resend timers, RTT); `NetcodeClientTransport::update`
    /// is the netcode. Only the second was being called.
    pub fn update(&mut self, dt_secs: f64) {
        let dt = Duration::from_secs_f64(dt_secs);
        // Connection clock first, then the netcode transport — the order renet's own examples use.
        self.client.update(dt);
        if let Err(e) = self.transport.update(dt, &mut self.client) {
            tracing::warn!(error = %e, "İstemci taşıma güncellemesi başarısız");
        }
    }

    /// Flushes queued messages out over the network. Call after enqueuing this frame's messages.
    pub fn send_packets(&mut self) {
        if let Err(e) = self.transport.send_packets(&mut self.client) {
            tracing::warn!(error = %e, "İstemci paket gönderimi başarısız");
        }
    }

    /// Whether the connection to the server is up.
    pub fn is_connected(&self) -> bool {
        self.client.is_connected()
    }

    /// Queues `message` for the server; it leaves with the next
    /// [`send_packets`](Self::send_packets). See [`NetworkServer::send`](super::server::NetworkServer::send)
    /// for why these exist.
    pub fn send(&mut self, message: &ClientMessage) -> Result<(), NetError> {
        let bytes = message.encode()?;
        self.client.send_message(ClientChannel::Command, bytes);
        Ok(())
    }

    /// Everything the server sent since the last call, reliable channel first. A malformed
    /// message is dropped with a warning.
    pub fn receive(&mut self) -> Vec<ServerMessage> {
        let mut out = Vec::new();
        for channel in [ServerChannel::Reliable, ServerChannel::Unreliable] {
            let id: u8 = channel.into();
            while let Some(bytes) = self.client.receive_message(id) {
                out.extend(ServerMessage::decode(&bytes));
            }
        }
        out
    }
}
