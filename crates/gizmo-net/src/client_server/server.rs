use super::error::NetError;
use super::protocol::{connection_config, PROTOCOL_ID};
use renet::RenetServer;
use renet_netcode::{NetcodeServerTransport, ServerAuthentication, ServerConfig};
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
    /// Creates a server listening on the given public address.
    ///
    /// Returns an error rather than panicking if address parsing, socket binding or transport
    /// setup fails (e.g. the port is already in use).
    pub fn new(public_addr: &str) -> Result<Self, NetError> {
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

        let transport = NetcodeServerTransport::new(server_config, socket)
            .map_err(|e| NetError::Transport(Box::new(e)))?;

        tracing::info!(public_addr = %public_addr, max_clients = 64, "Otoriter netcode sunucusu oluşturuldu, dinleniyor");
        Ok(Self { server, transport })
    }

    /// Advances the server by `dt_secs`, processing incoming client packets. Call once per tick.
    ///
    /// **Both halves, and the second one was missing.** renet splits the per-frame drive in two:
    /// `NetcodeServerTransport::update` advances the *netcode* (encryption, keepalive, timeouts),
    /// and `RenetServer::update` advances each *connection* — reliability timers, RTT, the resend
    /// clock. This wrapper called only the first, so every connection's `current_time` stayed at
    /// zero for the life of the process and the reliable channel's resend gate
    /// (`current_time - last_sent >= resend_time`) could never open: a reliable message was
    /// transmitted exactly once, ever. Since the reliable channel is *ordered*, the first dropped
    /// datagram then stopped that peer from receiving any further reliable message at all, and the
    /// undeliverable backlog grew until renet disconnected it.
    pub fn update(&mut self, dt_secs: f64) {
        let dt = Duration::from_secs_f64(dt_secs);
        // Connection clock first, then the netcode transport — the order renet's own examples use.
        self.server.update(dt);
        // Geçici bir transport hatası tüm sunucu döngüsünü düşürmemeli.
        if let Err(e) = self.transport.update(dt, &mut self.server) {
            tracing::error!(error = %e, "Sunucu taşıma güncellemesi başarısız");
        }
    }

    /// Flushes queued per-client messages out over the network. Call at the end of each tick.
    pub fn send_packets(&mut self) {
        self.transport.send_packets(&mut self.server);
    }
}

#[cfg(test)]
mod frame_drive_tests {
    /// Source with its comments removed — a negative `contains` is satisfied by prose that merely
    /// names the call, and the paragraph above `update` names both of them.
    fn code_only(src: &str) -> String {
        src.lines()
            .map(|line| {
                let bytes = line.as_bytes();
                let mut end = line.len();
                let mut i = 0;
                while i + 1 < bytes.len() {
                    if bytes[i] == b'/' && bytes[i + 1] == b'/' && (i == 0 || bytes[i - 1] != b':') {
                        end = i;
                        break;
                    }
                    i += 1;
                }
                &line[..end]
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// **Both wrappers must advance the connection clock, not just the transport.**
    ///
    /// renet splits the per-frame drive: the transport moves the netcode (encryption, keepalive,
    /// timeouts) and `RenetServer`/`RenetClient::update` moves each connection (reliability, RTT,
    /// the resend timer). Only the transport half was being called, so `current_time` never left
    /// zero and the reliable channel's resend could never fire — one dropped datagram silenced
    /// that peer's ordered channel for good.
    ///
    /// A source-shape guard because the behavioural version needs two live sockets and a dropped
    /// packet; comments are cut first.
    #[test]
    fn both_wrappers_advance_the_connection_clock() {
        for (name, src) in [
            ("server", include_str!("server.rs")),
            ("client", include_str!("client.rs")),
        ] {
            let code: String = code_only(src)
                .split("#[cfg(test)]")
                .next()
                .unwrap_or("")
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            let expected = if name == "server" {
                "self.server.update(dt)"
            } else {
                "self.client.update(dt)"
            };
            assert!(
                code.contains(expected),
                "the {name} wrapper advances only the transport; renet's connection clock stays \
                 at zero and reliable messages are never resent"
            );
            assert!(
                code.contains("self.transport.update(dt"),
                "…and it must still advance the netcode transport"
            );
        }
    }
}
