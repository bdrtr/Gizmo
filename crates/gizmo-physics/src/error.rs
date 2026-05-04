use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum GizmoError {
    CollisionOverflow,
    NaNVelocity(gizmo_core::entity::Entity),
    InvalidConstraint(String),
    DivideByZero(gizmo_core::entity::Entity),
    TunnelingDetected(gizmo_core::entity::Entity),
}

impl fmt::Display for GizmoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GizmoError::CollisionOverflow => write!(f, "Too many collisions detected"),
            GizmoError::NaNVelocity(ent) => write!(f, "NaN velocity detected for entity: {:?}", ent),
            GizmoError::InvalidConstraint(msg) => write!(f, "Invalid constraint: {}", msg),
            GizmoError::DivideByZero(ent) => write!(f, "Divide by zero encountered for entity: {:?}", ent),
            GizmoError::TunnelingDetected(ent) => write!(f, "Tunneling detected for entity: {:?}", ent),
        }
    }
}

impl std::error::Error for GizmoError {}
