use std::net::{UdpSocket, SocketAddr};
use std::io::ErrorKind;
use super::packet::NetworkPacket;

/// The non-blocking UDP communication layer, for native (desktop) environments.
#[derive(Debug)]
pub struct UdpTransport {
    socket: UdpSocket,
    /// Address of the peer; set explicitly via [`UdpTransport::set_remote`] or learned from
    /// the first received packet.
    pub remote_addr: Option<SocketAddr>,
}

/// The wire configuration, fixed for the life of the protocol.
///
/// `legacy()` — NOT `standard()`. bincode 2's default uses variable-length integers, which
/// encodes the same packet to different bytes than 1.x did; `legacy()` reproduces 1.x exactly.
/// Round-trip tests cannot tell the two apart (both encode and decode fine), so the guarantee
/// lives in `tests/wire_format.rs`, whose expectations were captured from 1.3.3 before this
/// migration and did not change with it.
const WIRE_CONFIG: bincode::config::Configuration<
    bincode::config::LittleEndian,
    bincode::config::Fixint,
> = bincode::config::legacy();

/// Serialises a [`NetworkPacket`] for the wire.
///
/// **The single place the bincode configuration lives.** Encoding and decoding must agree, and
/// two call sites choosing their own config is exactly how a wire format drifts silently — so
/// both go through here, and `tests/wire_format.rs` pins the resulting bytes.
pub fn encode_packet(packet: &NetworkPacket) -> std::io::Result<Vec<u8>> {
    bincode::serde::encode_to_vec(packet, WIRE_CONFIG)
        .map_err(|e| std::io::Error::new(ErrorKind::InvalidData, e))
}

/// Parses a datagram back into a [`NetworkPacket`], rejecting anything malformed or truncated.
///
/// See [`encode_packet`] — the configuration is shared, deliberately.
pub fn decode_packet(bytes: &[u8]) -> std::io::Result<NetworkPacket> {
    let (packet, consumed) = bincode::serde::decode_from_slice::<NetworkPacket, _>(bytes, WIRE_CONFIG)
        .map_err(|e| std::io::Error::new(ErrorKind::InvalidData, e))?;
    // bincode 1's `deserialize` rejected trailing bytes; 2.x's `decode_from_slice` reports how
    // many it consumed and is happy to leave a tail. Restoring the strictness deliberately: a
    // datagram with junk after a valid packet is a malformed datagram, and silently accepting it
    // would make `poll_events` feed the session a packet a peer never sent whole.
    if consumed != bytes.len() {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            format!("trailing bytes after packet: consumed {consumed} of {}", bytes.len()),
        ));
    }
    Ok(packet)
}

impl UdpTransport {
    /// Creates a new UDP socket and binds it to the local port.
    pub fn bind(local_port: u16) -> std::io::Result<Self> {
        let addr = format!("0.0.0.0:{}", local_port);
        let socket = UdpSocket::bind(addr)?;
        
        // Oyun döngüsünü (main loop) kilitlememesi için non-blocking yapıyoruz
        socket.set_nonblocking(true)?;

        tracing::info!(local_port, "UDP taşıma katmanı porta bağlandı (non-blocking)");
        Ok(Self {
            socket,
            remote_addr: None,
        })
    }

    /// Sets the peer's address.
    pub fn set_remote(&mut self, addr: SocketAddr) {
        tracing::debug!(remote = %addr, "Uzak eş (peer) adresi elle ayarlandı");
        self.remote_addr = Some(addr);
    }

    /// Encodes a NetworkPacket to bytes with bincode and sends it to the peer.
    pub fn send_packet(&self, packet: &NetworkPacket) -> std::io::Result<()> {
        if let Some(remote) = self.remote_addr {
            let bytes = encode_packet(packet)?;
            self.socket.send_to(&bytes, remote)?;
        }
        Ok(())
    }

    /// Reads every incoming UDP packet and returns them as NetworkPackets.
    /// It is non-blocking, so it returns empty immediately when there is nothing to read.
    #[tracing::instrument(skip_all, name = "udp_poll_events")]
    pub fn poll_events(&mut self) -> Vec<(SocketAddr, NetworkPacket)> {
        let mut events = Vec::new();
        let mut buf = [0u8; 65535]; // Maksimum UDP paket boyutu (64KB)

        loop {
            match self.socket.recv_from(&mut buf) {
                Ok((size, src_addr)) => {
                    // İlk defa paket aldığımız biriyse otomatik olarak remote_addr kaydet
                    // (P2P için pratik bir "hole punching" veya eşleşme simülasyonu)
                    if self.remote_addr.is_none() {
                        self.remote_addr = Some(src_addr);
                        tracing::info!(remote = %src_addr, "Uzak eş adresi ilk paketten öğrenildi (P2P eşleşme)");
                    }

                    if let Ok(packet) = decode_packet(&buf[..size]) {
                        events.push((src_addr, packet));
                    } else {
                        tracing::warn!(src = %src_addr, size, "Ayrıştırılamayan paket alındı (bincode deserialize hatası)");
                    }
                }
                Err(e) => {
                    if e.kind() == ErrorKind::WouldBlock {
                        // Okunacak paket kalmadı
                        break;
                    } else {
                        tracing::error!(error = ?e, "UDP alım (recv) hatası");
                        break;
                    }
                }
            }
        }

        if !events.is_empty() {
            tracing::trace!(received = events.len(), "UDP paketleri alındı");
        }
        events
    }
}
