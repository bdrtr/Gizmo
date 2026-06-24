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
