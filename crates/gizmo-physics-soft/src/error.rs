//! Concrete error type for fallible soft-body operations.
//!
//! This replaces the previous "swallow + log", `Option`, and silent-clamp
//! failure surfaces with an explicit, matchable [`SoftBodyError`]. The success
//! path of every converted function is unchanged; only the failure surface is
//! now a `Result`.

/// Errors produced by fallible soft-body construction and simulation operations.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum SoftBodyError {
    /// A tetrahedral element referenced a node index that does not exist yet.
    ///
    /// `index` is the offending node index and `node_count` is the number of
    /// nodes currently present in the mesh.
    NodeIndexOutOfBounds { index: u32, node_count: u32 },

    /// Poisson's ratio was outside the physically valid range `[0.0, 0.5)`.
    ///
    /// Values `>= 0.5` produce a singular / negative Lamé `lambda`
    /// (incompressible limit) and values `< 0.0` are unsupported here.
    InvalidPoissonsRatio { value: f32 },

    /// Young's modulus was not a finite, strictly-positive value.
    InvalidYoungsModulus { value: f32 },

    /// The flattened GPU node offset overflowed `u32` (too many nodes across
    /// all soft bodies in a single step).
    NodeOffsetOverflow,

    /// No compatible GPU adapter could be acquired.
    NoCompatibleAdapter,

    /// Requesting a logical GPU device from the adapter failed.
    DeviceRequestFailed(String),
}

impl std::fmt::Display for SoftBodyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SoftBodyError::NodeIndexOutOfBounds { index, node_count } => write!(
                f,
                "soft body node index {index} out of bounds (node count {node_count})"
            ),
            SoftBodyError::InvalidPoissonsRatio { value } => write!(
                f,
                "invalid Poisson's ratio {value} (must be in [0.0, 0.5))"
            ),
            SoftBodyError::InvalidYoungsModulus { value } => write!(
                f,
                "invalid Young's modulus {value} (must be finite and > 0)"
            ),
            SoftBodyError::NodeOffsetOverflow => {
                write!(f, "soft body node offset overflowed u32 (too many nodes)")
            }
            SoftBodyError::NoCompatibleAdapter => {
                write!(f, "no compatible GPU adapter available for soft-body compute")
            }
            SoftBodyError::DeviceRequestFailed(msg) => {
                write!(f, "failed to request GPU device for soft-body compute: {msg}")
            }
        }
    }
}

impl std::error::Error for SoftBodyError {}
