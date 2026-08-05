//! The physics error type shared by this crate and `gizmo-physics-rigid`.
//!
//! [`GizmoError`] is a flat enum with no wrapped causes; the operations that use it return a
//! `Result` instead of panicking on degenerate meshes or NaN state. It is not the *only*
//! error type in the physics crates, so a `Result` there does not necessarily carry it.

use crate::BodyHandle;
use std::fmt;

/// Failures reported by the physics crates.
///
/// Most variants name the body they concern with a [`BodyHandle`] or carry a human-readable
/// message — but not all: `CollisionOverflow` carries only counts, and `BvhBuildFailed` is a
/// unit variant, so neither a total "which body?" nor a total "what message?" accessor can be
/// written over this enum. No variant wraps a lower-level cause, so
/// [`Error::source`](std::error::Error::source) is always `None` and the
/// [`Display`](std::fmt::Display) text is the whole story.
///
/// Of the variants below, the engine itself currently produces only
/// [`NaNVelocity`](Self::NaNVelocity), [`NaNPosition`](Self::NaNPosition) and
/// [`BvhBuildFailed`](Self::BvhBuildFailed); the rest are declared for guards that are not
/// wired up yet, though callers are free to construct them for their own checks. Because the
/// enum is `#[non_exhaustive]` it may gain variants without a breaking change, so downstream
/// `match`es need a wildcard arm.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum GizmoError {
    /// More contacts (or candidate pairs) were produced in one step than the configured
    /// budget allows.
    CollisionOverflow {
        /// How many were produced. Only meaningful next to `limit`; nothing here enforces
        /// that it actually exceeds it.
        count: usize,
        /// The budget that was blown, in the same unit as `count`.
        limit: usize,
    },
    /// A body's linear or angular velocity — or one of its damping coefficients — was
    /// non-finite when velocity integration began.
    ///
    /// The integrator runs this check before it writes anything, so the body is still in its
    /// pre-step state when the error comes back.
    NaNVelocity(BodyHandle),
    /// A body's position or integrated orientation went non-finite during position
    /// integration.
    ///
    /// Unlike [`NaNVelocity`](Self::NaNVelocity) the check runs *after* the write, so the
    /// transform already holds the bad value and the caller must reset it.
    NaNPosition(BodyHandle),
    /// A joint or constraint was rejected as unusable. The string is a diagnostic message
    /// for humans, not a stable machine-readable code.
    InvalidConstraint(String),
    /// A division by zero was hit while solving for this body — e.g. a zero effective mass
    /// along a constraint direction.
    DivideByZero(BodyHandle),
    /// A body is believed to have passed through geometry between two steps without any
    /// contact being generated.
    TunnelingDetected(BodyHandle),
    /// [`BvhTree::build`](crate::bvh::BvhTree::build) refused a mesh: either an index points
    /// past the end of the vertex slice, or the triangle count would overflow the `u32` used
    /// for BVH ranges.
    ///
    /// It carries no payload, so the offending index is not reported. An empty index slice
    /// is *not* a failure — that yields an empty tree.
    BvhBuildFailed,
    /// A joint referenced a body the world's handle→index map does not know, so it could not
    /// be resolved to a solver row.
    JointEntityNotFound(BodyHandle),
    /// A collider shape was degenerate or malformed (zero extents, empty mesh, …); the
    /// string describes which, for a human reader.
    InvalidShapeData(String),
    /// A body's sleep/wake bookkeeping was found inconsistent — the kind of corruption that
    /// breaks replay, since sleep state is part of the hashed simulation state.
    SleepStateCorrupted(BodyHandle),
}

impl fmt::Display for GizmoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GizmoError::CollisionOverflow { count, limit } => write!(
                f,
                "Too many collisions detected (count: {}, limit: {})",
                count, limit
            ),
            GizmoError::NaNVelocity(ent) => {
                write!(f, "NaN velocity detected for entity: {:?}", ent)
            }
            GizmoError::NaNPosition(ent) => {
                write!(f, "NaN position detected for entity: {:?}", ent)
            }
            GizmoError::InvalidConstraint(msg) => write!(f, "Invalid constraint: {}", msg),
            GizmoError::DivideByZero(ent) => {
                write!(f, "Divide by zero encountered for entity: {:?}", ent)
            }
            GizmoError::TunnelingDetected(ent) => {
                write!(f, "Tunneling detected for entity: {:?}", ent)
            }
            GizmoError::BvhBuildFailed => write!(f, "BVH build failed"),
            GizmoError::JointEntityNotFound(ent) => {
                write!(f, "Joint entity not found in entity_index_map: {:?}", ent)
            }
            GizmoError::InvalidShapeData(msg) => {
                write!(f, "Invalid shape data (degenerate shape): {}", msg)
            }
            GizmoError::SleepStateCorrupted(ent) => {
                write!(f, "Sleep/wake state corrupted for entity: {:?}", ent)
            }
        }
    }
}

impl std::error::Error for GizmoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        // İleride wrapped inner error'lar eklenirse buradan return edilebilir.
        // Şimdilik herhangi bir variant alt hata barındırmadığı için None dönüyoruz.
        None
    }
}
