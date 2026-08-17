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
    MissingUuid {
        /// The UUID that was asked for.
        source: String,
    },

    /// An image file (texture) could not be decoded.
    ImageDecode {
        /// The file that failed to decode.
        path: PathBuf,
        /// What the `image` crate reported.
        source: image::ImageError,
    },

    /// An OBJ file could not be parsed by `tobj`.
    ObjLoad {
        /// The file that failed to parse.
        path: PathBuf,
        /// What `tobj` reported.
        source: tobj::LoadError,
    },

    /// An OBJ file parsed but contained no models.
    ObjEmpty {
        /// The file that parsed but held no geometry.
        path: PathBuf,
    },

    /// An OBJ vertex/normal/texcoord index pointed outside the available data.
    ObjIndexOutOfRange {
        /// The file the bad index was in.
        path: PathBuf,
        /// Which attribute array it indexed.
        kind: ObjIndexKind,
        /// The index itself.
        index: usize,
        /// How long that array actually is.
        len: usize,
    },

    /// A glTF / GLB file (or embedded slice) could not be imported.
    GltfImport {
        /// The file that failed to import.
        path: PathBuf,
        /// What the `gltf` crate reported.
        source: gltf::Error,
    },

    /// A texture upload requested a zero width or height.
    ZeroDimensionTexture {
        /// The texture's cache key, which is what identifies it before it has a GPU handle.
        cache_key: String,
        /// The requested width.
        width: u32,
        /// The requested height.
        height: u32,
    },

    /// A network fetch for an asset (WASM target) failed.
    Fetch {
        /// The URL that was fetched.
        url: String,
        /// What the fetch reported. A string because the browser's error is not a Rust error
        /// type.
        message: String,
    },

    /// A heightmap image was smaller than the 2x2 minimum required to build a
    /// terrain mesh.
    HeightmapTooSmall {
        /// The heightmap image.
        path: PathBuf,
        /// Its width, in texels.
        width: u32,
        /// Its height.
        height: u32,
    },

    /// The provided RGBA byte buffer did not match `width * height * 4`.
    RgbaSizeMismatch {
        /// The texture's cache key.
        cache_key: String,
        /// How many bytes were handed over.
        got: usize,
        /// How many `width × height × 4` comes to.
        expected: usize,
        /// The declared width.
        width: u32,
        /// The declared height.
        height: u32,
    },
}

/// Which OBJ attribute array an out-of-range index referred to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ObjIndexKind {
    /// The vertex position array.
    Position,
    /// The normal array.
    Normal,
    /// The texture-coordinate array.
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
