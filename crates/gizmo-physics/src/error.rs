use gizmo_core::entity::Entity;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum GizmoError {
    CollisionOverflow { count: usize, limit: usize },
    NaNVelocity(Entity),
    NaNPosition(Entity),
    InvalidConstraint(String),
    DivideByZero(Entity),
    TunnelingDetected(Entity),
    BvhBuildFailed,
    JointEntityNotFound(Entity),
    InvalidShapeData(String),
    SleepStateCorrupted(Entity),
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
