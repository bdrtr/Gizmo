use super::protocol::{connection_config, PROTOCOL_ID};
use renet::RenetClient;
use renet_netcode::{ClientAuthentication, NetcodeClientTransport};
use std::error::Error;
use std::net::UdpSocket;
use std::time::{Duration, SystemTime};

pub struct NetworkClient {
    pub client: RenetClient,
    pub transport: NetcodeClientTransport,
}

impl NetworkClient {
    /// Sunucuya bağlanacak bir istemci oluşturur.
    ///
    /// `client_id` çağıran tarafından benzersiz ve ideal olarak tahmin edilemez
    /// şekilde sağlanmalıdır. (Eskiden buraya gömülü olan "şu anki milisaniye"
    /// değeri hem çakışmaya hem de öngörülebilirliğe açıktı.)
    ///
    /// Adres ayrıştırma, soket bağlama veya transport kurulumu başarısız olursa
    /// panik yerine hata döndürür.
    pub fn new(server_addr: &str, client_id: u64) -> Result<Self, Box<dyn Error + Send + Sync>> {
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

        let transport = NetcodeClientTransport::new(current_time, authentication, socket)?;

        Ok(Self { client, transport })
    }

    pub fn update(&mut self, dt_secs: f64) {
        let dt = Duration::from_secs_f64(dt_secs);
        if let Err(e) = self.transport.update(dt, &mut self.client) {
            tracing::warn!("İstemci taşıma güncellemesi başarısız: {e}");
        }
    }

    pub fn send_packets(&mut self) {
        if let Err(e) = self.transport.send_packets(&mut self.client) {
            tracing::warn!("İstemci paket gönderimi başarısız: {e}");
        }
    }
}
