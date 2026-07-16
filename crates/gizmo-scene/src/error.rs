//! Scene serialization error types.

/// Errors that can occur while saving or loading scenes and prefabs.
#[derive(Debug)]
#[non_exhaustive]
pub enum SceneError {
    /// Filesystem I/O failure (reading or writing the scene file).
    Io(std::io::Error),
    /// RON deserialization (parse) failure when loading a scene/prefab.
    Parse(ron::error::SpannedError),
    /// RON serialization failure when saving a scene/prefab.
    Serialize(ron::Error),
}

impl std::fmt::Display for SceneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SceneError::Io(_) => write!(f, "scene file I/O error"),
            SceneError::Parse(_) => write!(f, "scene file parse error"),
            SceneError::Serialize(_) => write!(f, "scene serialization error"),
        }
    }
}

impl std::error::Error for SceneError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SceneError::Io(e) => Some(e),
            SceneError::Parse(e) => Some(e),
            SceneError::Serialize(e) => Some(e),
        }
    }
}

impl From<std::io::Error> for SceneError {
    fn from(e: std::io::Error) -> Self {
        SceneError::Io(e)
    }
}

impl From<ron::error::SpannedError> for SceneError {
    fn from(e: ron::error::SpannedError) -> Self {
        SceneError::Parse(e)
    }
}

impl From<ron::Error> for SceneError {
    fn from(e: ron::Error) -> Self {
        SceneError::Serialize(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    // `?`/`From` glue: an `io::Error` bubbling out of a scene read/write must land in
    // the `Io` variant (not silently reclassified), keep a human-readable Display, and
    // expose the underlying io::Error via `source()` so callers can downcast/inspect it.
    #[test]
    fn io_error_converts_and_preserves_source() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "no such scene");
        let err: SceneError = io.into();
        assert!(matches!(err, SceneError::Io(_)), "io::Error must map to Io variant");
        assert_eq!(err.to_string(), "scene file I/O error");
        let src = err.source().expect("Io variant must expose its underlying source");
        // The wrapped source is the original io::Error, so its own message survives.
        assert!(src.to_string().contains("no such scene"));
    }

    // A RON PARSE failure (loading a malformed scene) is a `SpannedError` and must map to
    // the `Parse` variant — distinct from `Serialize`, so load vs save failures never blur.
    #[test]
    fn parse_error_converts_and_preserves_source() {
        let spanned: ron::error::SpannedError =
            ron::from_str::<i32>("definitely not ron").unwrap_err();
        let err: SceneError = spanned.into();
        assert!(matches!(err, SceneError::Parse(_)), "SpannedError must map to Parse variant");
        assert_eq!(err.to_string(), "scene file parse error");
        assert!(err.source().is_some(), "Parse variant must expose its source");
    }

    // A RON SERIALIZE failure (saving) is a bare `ron::Error` and must map to `Serialize`.
    #[test]
    fn serialize_error_converts_and_preserves_source() {
        let ron_err = ron::Error::Message("boom".to_string());
        let err: SceneError = ron_err.into();
        assert!(matches!(err, SceneError::Serialize(_)), "ron::Error must map to Serialize variant");
        assert_eq!(err.to_string(), "scene serialization error");
        assert!(err.source().is_some(), "Serialize variant must expose its source");
    }

    // Each variant's Display must be distinct so a log/UI message tells the three failure
    // classes (I/O vs parse vs serialize) apart at a glance.
    #[test]
    fn each_variant_has_a_distinct_display() {
        let io: SceneError = std::io::Error::other("x").into();
        let parse: SceneError = ron::from_str::<i32>("zz").unwrap_err().into();
        let ser: SceneError = ron::Error::Message("y".to_string()).into();

        let msgs = [io.to_string(), parse.to_string(), ser.to_string()];
        // All three must differ from one another.
        assert_ne!(msgs[0], msgs[1]);
        assert_ne!(msgs[1], msgs[2]);
        assert_ne!(msgs[0], msgs[2]);
        // And every variant must expose a source (the `?`-chain is never dropped).
        assert!(io.source().is_some() && parse.source().is_some() && ser.source().is_some());
    }
}
