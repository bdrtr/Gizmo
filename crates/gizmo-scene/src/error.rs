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
