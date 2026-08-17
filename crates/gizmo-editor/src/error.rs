//! The concrete error type for the editor layer (the 1.0 error contract).

/// The errors the editor's file and serialisation operations can produce.
#[derive(Debug)]
#[non_exhaustive]
pub enum EditorError {
    /// A file read or write failed (with the path for context).
    Io {
        /// A description of the operation that failed (e.g. the path, or context).
        context: String,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// JSON serialisation or parsing failed (layout and the like).
    Json(serde_json::Error),
    /// TOML serialisation failed (preferences).
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    #[test]
    fn io_display_includes_context_and_exposes_source() {
        let err = EditorError::Io {
            context: "dosya açılamadı: /yok".to_string(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "no such file"),
        };
        assert_eq!(err.to_string(), "G/Ç hatası: dosya açılamadı: /yok");
        // source() alttaki io::Error'a zincirlenmeli
        let src = err.source().expect("source olmalı");
        assert_eq!(
            src.downcast_ref::<std::io::Error>().map(|e| e.kind()),
            Some(std::io::ErrorKind::NotFound)
        );
    }

    /// `From<serde_json::Error>` must land on the Json variant: Display a fixed message, with
    /// the underlying serde error as the source.
    #[test]
    fn json_from_conversion_and_display() {
        let json_err = serde_json::from_str::<i32>("not a number").unwrap_err();
        let err: EditorError = json_err.into();
        assert!(matches!(err, EditorError::Json(_)));
        assert_eq!(err.to_string(), "JSON işlenemedi");
        assert!(err.source().is_some());
    }

    /// `From<toml::ser::Error>` must land on the TomlSerialize variant. A top-level integer is
    /// not a TOML table, so serialising one produces the error.
    #[test]
    fn toml_serialize_from_conversion_and_display() {
        let toml_err = toml::to_string_pretty(&42i32).unwrap_err();
        let err: EditorError = toml_err.into();
        assert!(matches!(err, EditorError::TomlSerialize(_)));
        assert_eq!(err.to_string(), "TOML serileştirilemedi");
        assert!(err.source().is_some());
    }
}
