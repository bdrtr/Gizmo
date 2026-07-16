//! `client-server` mimarisi için somut hata tipi.
//!
//! Eskiden yapıcılar `Box<dyn Error + Send + Sync>` döndürüyordu; bu, çağıranın
//! hatayı program-akışıyla (adres ayrıştırma mı, port-in-use mu, transport mu)
//! ayırt etmesini imkansız kılıyordu. [`NetError`] bunu eşleşilebilir
//! varyantlara ayırır.

use std::error::Error;
use std::fmt;

/// `client-server` netcode kurulumu sırasında oluşabilecek hatalar.
///
/// `#[non_exhaustive]`: ileride yeni varyant eklemek semver-kıran olmadan
/// mümkün kalsın diye. Çağıranlar `match`'lerinde `_ => ...` kullanmalıdır.
#[derive(Debug)]
#[non_exhaustive]
pub enum NetError {
    /// Verilen sunucu/genel adres geçerli bir `SocketAddr` olarak ayrıştırılamadı.
    AddrParse(std::net::AddrParseError),
    /// UDP soketi bağlanamadı (örn. port kullanımda) veya başka bir G/Ç hatası.
    Io(std::io::Error),
    /// Sistem saati UNIX epoch'tan önceydi (`SystemTime::duration_since` hatası).
    Time(std::time::SystemTimeError),
    /// Alttaki netcode taşıma katmanı kurulamadı.
    Transport(Box<dyn Error + Send + Sync + 'static>),
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NetError::AddrParse(_) => write!(f, "ağ adresi ayrıştırılamadı"),
            NetError::Io(_) => write!(f, "soket bağlama/G\u{2044}Ç hatası"),
            NetError::Time(_) => write!(f, "sistem saati geçersiz (epoch öncesi)"),
            NetError::Transport(_) => write!(f, "netcode taşıma katmanı kurulamadı"),
        }
    }
}

impl Error for NetError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            NetError::AddrParse(e) => Some(e),
            NetError::Io(e) => Some(e),
            NetError::Time(e) => Some(e),
            NetError::Transport(e) => Some(&**e),
        }
    }
}

impl From<std::net::AddrParseError> for NetError {
    fn from(e: std::net::AddrParseError) -> Self {
        NetError::AddrParse(e)
    }
}

impl From<std::io::Error> for NetError {
    fn from(e: std::io::Error) -> Self {
        NetError::Io(e)
    }
}

impl From<std::time::SystemTimeError> for NetError {
    fn from(e: std::time::SystemTimeError) -> Self {
        NetError::Time(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // `?` (From) geçersiz adresi eşleşilebilir AddrParse varyantına eşlemeli ve
    // kaynak zinciri altta yatan AddrParseError'ı vermeli (program-akışı ayrımı için).
    #[test]
    fn from_addr_parse_maps_to_addr_variant_and_chains_source() {
        let parse_err = "definitely not an address"
            .parse::<std::net::SocketAddr>()
            .unwrap_err();
        let net: NetError = parse_err.into();
        assert!(matches!(net, NetError::AddrParse(_)), "yanlış varyanta eşlendi");
        assert!(!net.to_string().is_empty(), "Display boş olmamalı");
        let src = net.source().expect("AddrParse kaynağı olmalı");
        assert!(
            src.downcast_ref::<std::net::AddrParseError>().is_some(),
            "source altta yatan AddrParseError olmalı"
        );
    }

    // io::Error → Io varyantı; kaynak zinciri G/Ç hata TÜRÜnü korumalı (örn. AddrInUse).
    #[test]
    fn from_io_maps_to_io_variant_and_preserves_kind() {
        let io = std::io::Error::new(std::io::ErrorKind::AddrInUse, "port dolu");
        let net: NetError = io.into();
        assert!(matches!(net, NetError::Io(_)));
        let src = net.source().expect("Io kaynağı olmalı");
        let inner = src
            .downcast_ref::<std::io::Error>()
            .expect("source io::Error olmalı");
        assert_eq!(inner.kind(), std::io::ErrorKind::AddrInUse, "io hata türü korunmalı");
    }

    // Dört varyantın Display'i boş olmamalı ve birbirinden AYIRT EDİLEBİLİR olmalı —
    // NetError'ın var oluş nedeni tam da bu (çağıran hatayı akışla ayırt edebilsin).
    #[test]
    fn all_variants_display_are_nonempty_and_distinct() {
        let addr = NetError::from("x".parse::<std::net::SocketAddr>().unwrap_err());
        let io = NetError::from(std::io::Error::other("x"));
        let time = NetError::from(
            std::time::SystemTime::UNIX_EPOCH
                .duration_since(std::time::SystemTime::now())
                .unwrap_err(),
        );
        let transport =
            NetError::Transport(Box::new(std::io::Error::other("t")));

        let msgs = [addr.to_string(), io.to_string(), time.to_string(), transport.to_string()];
        for m in &msgs {
            assert!(!m.is_empty(), "hata mesajı boş olmamalı");
        }
        for i in 0..msgs.len() {
            for j in (i + 1)..msgs.len() {
                assert_ne!(msgs[i], msgs[j], "varyant mesajları ayırt edilebilir olmalı");
            }
        }
    }
}
