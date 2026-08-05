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
    NodeIndexOutOfBounds {
        /// The rejected node index. All four indices of an element must be
        /// `< node_count`; this is the first one found that was not.
        index: u32,
        /// How many nodes the mesh held at the moment the element was submitted,
        /// i.e. the exclusive upper bound the index was checked against.
        node_count: u32,
    },

    /// Poisson's ratio was outside the physically valid range `[0.0, 0.5)`.
    ///
    /// Values `>= 0.5` produce a singular / negative Lamé `lambda`
    /// (incompressible limit) and values `< 0.0` are unsupported here.
    InvalidPoissonsRatio {
        /// The rejected ratio, reported verbatim (dimensionless). Non-finite
        /// input fails the same check, so this may be NaN or ±inf rather than a
        /// finite value outside `[0.0, 0.5)`.
        value: f32,
    },

    /// Young's modulus was not a finite, strictly-positive value.
    InvalidYoungsModulus {
        /// The rejected modulus, reported verbatim. Young's modulus is a
        /// pressure (pascals, N/m²) and must be finite and `> 0`; zero, negative
        /// values, NaN and ±inf all end up here.
        value: f32,
    },

    /// A tetrahedral element was (near-)degenerate: its rest volume is not a
    /// finite, strictly-positive value above the acceptance epsilon.
    ///
    /// Such elements have a singular reference shape matrix (`Dm`), so the
    /// deformation gradient and the derived elastic forces are undefined
    /// (near-zero stiffness / NaN propagation). `volume` is the offending rest
    /// volume that was computed.
    DegenerateTetrahedron {
        /// The rest volume computed for the element, in cubic metres. It is an
        /// unsigned magnitude, so an inverted winding is not what makes it
        /// small — near-coplanar nodes are.
        ///
        /// The element is rejected when this is `<= 1e-6` or NaN — but also when
        /// the accompanying inverse reference shape matrix came out non-finite.
        /// So a reported volume comfortably *above* the threshold does not mean
        /// the volume itself was the problem: the shape matrix was.
        volume: f32,
    },

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
            SoftBodyError::DegenerateTetrahedron { volume } => write!(
                f,
                "degenerate tetrahedral element (rest volume {volume} must be finite and > 0)"
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Each variant's `Display` message embeds the offending value(s), so a caller/log can
    /// see *what* was wrong, not just *that* something was.
    #[test]
    fn display_embeds_offending_values() {
        let s = SoftBodyError::NodeIndexOutOfBounds { index: 7, node_count: 4 }.to_string();
        assert!(s.contains('7') && s.contains('4'), "message must name index and count: {s}");

        let s = SoftBodyError::InvalidPoissonsRatio { value: 0.75 }.to_string();
        assert!(s.contains("0.75"), "message must name the bad ratio: {s}");

        let s = SoftBodyError::InvalidYoungsModulus { value: -3.0 }.to_string();
        assert!(s.contains("-3"), "message must name the bad modulus: {s}");

        let s = SoftBodyError::DegenerateTetrahedron { volume: 0.0 }.to_string();
        assert!(!s.is_empty() && s.to_lowercase().contains("degenerate"), "got: {s}");

        // Payload-free variants must still render a non-empty message.
        assert!(!SoftBodyError::NodeOffsetOverflow.to_string().is_empty());
        assert!(!SoftBodyError::NoCompatibleAdapter.to_string().is_empty());
        let s = SoftBodyError::DeviceRequestFailed("boom".into()).to_string();
        assert!(s.contains("boom"), "device error must include the cause: {s}");
    }

    /// `PartialEq` distinguishes variants and payloads (the whole point of a matchable error).
    #[test]
    fn equality_distinguishes_variants_and_payloads() {
        assert_eq!(
            SoftBodyError::NodeIndexOutOfBounds { index: 1, node_count: 2 },
            SoftBodyError::NodeIndexOutOfBounds { index: 1, node_count: 2 }
        );
        assert_ne!(
            SoftBodyError::NodeIndexOutOfBounds { index: 1, node_count: 2 },
            SoftBodyError::NodeIndexOutOfBounds { index: 1, node_count: 3 }
        );
        assert_ne!(
            SoftBodyError::InvalidYoungsModulus { value: 1.0 },
            SoftBodyError::InvalidPoissonsRatio { value: 1.0 }
        );
        assert_ne!(SoftBodyError::NoCompatibleAdapter, SoftBodyError::NodeOffsetOverflow);
    }
}
