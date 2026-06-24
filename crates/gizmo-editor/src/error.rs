//! Editör katmanı için somut hata tipi (1.0 hata kontratı).

/// Editör dosya/serileştirme işlemlerinde oluşabilecek hatalar.
#[derive(Debug)]
#[non_exhaustive]
pub enum EditorError {
    /// Bir dosya okuma/yazma işlemi başarısız oldu (yol bağlamı ile).
    Io {
        /// Hatanın oluştuğu işlem için açıklama (ör. yol veya bağlam).
        context: String,
        /// Alttaki G/Ç hatası.
        source: std::io::Error,
    },
    /// JSON serileştirme/ayrıştırma başarısız oldu (layout vb.).
    Json(serde_json::Error),
    /// TOML serileştirme başarısız oldu (tercihler).
    TomlSerialize(toml::ser::Error),
}

impl std::fmt::Display for EditorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EditorError::Io { context, .. } => write!(f, "G/Ç hatası: {}", context),
            EditorError::Json(_) => write!(f, "JSON işlenemedi"),
            EditorError::TomlSerialize(_) => write!(f, "TOML serileştirilemedi"),
        }
    }
}

impl std::error::Error for EditorError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            EditorError::Io { source, .. } => Some(source),
            EditorError::Json(e) => Some(e),
            EditorError::TomlSerialize(e) => Some(e),
        }
    }
}

impl From<serde_json::Error> for EditorError {
    fn from(e: serde_json::Error) -> Self {
        EditorError::Json(e)
    }
}

impl From<toml::ser::Error> for EditorError {
    fn from(e: toml::ser::Error) -> Self {
        EditorError::TomlSerialize(e)
    }
}
