//! Concrete error type for asset loading / decoding / GPU upload.
//!
//! Replaces the previous stringly-typed `Result<_, String>` surface so callers
//! can match on variants, chain `?`, and access source errors.

use std::path::PathBuf;

/// Errors produced while resolving, decoding, or uploading renderer assets.
///
/// Open / growing type: new variants may be added in future minor releases.
#[derive(Debug)]
#[non_exhaustive]
pub enum AssetError {
    /// A load source referenced an asset UUID that is not registered.
    MissingUuid { source: String },

    /// An image file (texture) could not be decoded.
    ImageDecode {
        path: PathBuf,
        source: image::ImageError,
    },

    /// An OBJ file could not be parsed by `tobj`.
    ObjLoad {
        path: PathBuf,
        source: tobj::LoadError,
    },

    /// An OBJ file parsed but contained no models.
    ObjEmpty { path: PathBuf },

    /// An OBJ vertex/normal/texcoord index pointed outside the available data.
    ObjIndexOutOfRange {
        path: PathBuf,
        kind: ObjIndexKind,
        index: usize,
        len: usize,
    },

    /// A glTF / GLB file (or embedded slice) could not be imported.
    GltfImport {
        path: PathBuf,
        source: gltf::Error,
    },

    /// A texture upload requested a zero width or height.
    ZeroDimensionTexture {
        cache_key: String,
        width: u32,
        height: u32,
    },

    /// A network fetch for an asset (WASM target) failed.
    Fetch { url: String, message: String },

    /// A heightmap image was smaller than the 2x2 minimum required to build a
    /// terrain mesh.
    HeightmapTooSmall {
        path: PathBuf,
        width: u32,
        height: u32,
    },

    /// The provided RGBA byte buffer did not match `width * height * 4`.
    RgbaSizeMismatch {
        cache_key: String,
        got: usize,
        expected: usize,
        width: u32,
        height: u32,
    },
}

/// Which OBJ attribute array an out-of-range index referred to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ObjIndexKind {
    Position,
    Normal,
    TexCoord,
}

impl std::fmt::Display for ObjIndexKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ObjIndexKind::Position => "position",
            ObjIndexKind::Normal => "normal",
            ObjIndexKind::TexCoord => "texcoord",
        };
        f.write_str(s)
    }
}

impl std::fmt::Display for AssetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AssetError::MissingUuid { source } => {
                write!(f, "missing UUID reference: {source}")
            }
            AssetError::ImageDecode { path, .. } => {
                write!(f, "cannot read texture ({})", path.display())
            }
            AssetError::ObjLoad { path, .. } => {
                write!(f, "OBJ load failed ({})", path.display())
            }
            AssetError::ObjEmpty { path } => {
                write!(f, "OBJ file contains no models: {}", path.display())
            }
            AssetError::ObjIndexOutOfRange {
                path,
                kind,
                index,
                len,
            } => write!(
                f,
                "OBJ ({}): {kind} index {index} out of range (len={len})",
                path.display()
            ),
            AssetError::GltfImport { path, .. } => {
                write!(f, "glTF import failed ({})", path.display())
            }
            AssetError::Fetch { url, message } => {
                write!(f, "fetch failed for '{url}': {message}")
            }
            AssetError::HeightmapTooSmall {
                path,
                width,
                height,
            } => write!(
                f,
                "heightmap must be at least 2x2 to build terrain: {}x{} ({})",
                width,
                height,
                path.display()
            ),
            AssetError::ZeroDimensionTexture {
                cache_key,
                width,
                height,
            } => write!(
                f,
                "cannot create texture with zero dimension: {width}x{height} (key={cache_key})"
            ),
            AssetError::RgbaSizeMismatch {
                cache_key,
                got,
                expected,
                width,
                height,
            } => write!(
                f,
                "RGBA size mismatch for '{cache_key}': got {got} bytes, expected {expected} ({width}x{height}x4)"
            ),
        }
    }
}

impl std::error::Error for AssetError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AssetError::ImageDecode { source, .. } => Some(source),
            AssetError::ObjLoad { source, .. } => Some(source),
            AssetError::GltfImport { source, .. } => Some(source),
            _ => None,
        }
    }
}
