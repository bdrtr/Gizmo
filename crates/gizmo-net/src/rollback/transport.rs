use std::net::{UdpSocket, SocketAddr};
use std::io::ErrorKind;
use super::packet::NetworkPacket;

/// Native (Masaüstü) ortamlar için Non-Blocking UDP haberleşme katmanı.
#[derive(Debug)]
pub struct UdpTransport {
    socket: UdpSocket,
    /// Address of the peer; set explicitly via [`UdpTransport::set_remote`] or learned from
    /// the first received packet.
    pub remote_addr: Option<SocketAddr>,
}

impl UdpTransport {
    /// Yeni bir UDP soketi oluşturur ve yerel porta bağlar.
    pub fn bind(local_port: u16) -> std::io::Result<Self> {
        let addr = format!("0.0.0.0:{}", local_port);
        let socket = UdpSocket::bind(addr)?;
        
        // Oyun döngüsünü (main loop) kilitlememesi için non-blocking yapıyoruz
        socket.set_nonblocking(true)?;

        Ok(Self {
            socket,
            remote_addr: None,
        })
    }

    /// Karşı tarafın (Peer) adresini ayarlar.
    pub fn set_remote(&mut self, addr: SocketAddr) {
        self.remote_addr = Some(addr);
    }

    /// Bir NetworkPacket'i bincode ile baytlara çevirip karşı tarafa yollar.
    pub fn send_packet(&self, packet: &NetworkPacket) -> std::io::Result<()> {
        if let Some(remote) = self.remote_addr {
            let bytes = bincode::serialize(packet)
                .map_err(|e| std::io::Error::new(ErrorKind::InvalidData, e))?;
                
            self.socket.send_to(&bytes, remote)?;
        }
        Ok(())
    }

    /// Gelen tüm UDP paketlerini okur ve NetworkPacket olarak döndürür.
    /// Non-blocking olduğu için eğer okunacak paket yoksa anında boş döner.
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
                    }

                    if let Ok(packet) = bincode::deserialize::<NetworkPacket>(&buf[..size]) {
                        events.push((src_addr, packet));
                    } else {
                        tracing::warn!("Unparseable packet received from {}", src_addr);
                    }
                }
                Err(e) => {
                    if e.kind() == ErrorKind::WouldBlock {
                        // Okunacak paket kalmadı
                        break;
                    } else {
                        tracing::error!("UDP Recv Error: {:?}", e);
                        break;
                    }
                }
            }
        }

        events
    }
}
