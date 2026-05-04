use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────────────────────────────────────
// Kombine Fonksiyon: iki malzemenin değerlerini nasıl birleştireceğimizi belirler
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum CombineMode {
    Average,   // (a + b) / 2
    Min,       // a.min(b)
    Max,       // a.max(b)
    Multiply,  // sqrt(a * b)  — geometric mean (sürtünme için ideal)
}

impl Default for CombineMode {
    fn default() -> Self { CombineMode::Multiply }
}

impl CombineMode {
    pub fn combine(self, a: f32, b: f32) -> f32 {
        match self {
            CombineMode::Average  => (a + b) * 0.5,
            CombineMode::Min      => a.min(b),
            CombineMode::Max      => a.max(b),
            CombineMode::Multiply => (a * b).sqrt(), // geometric mean
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// PhysicsMaterial
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PhysicsMaterial {
    pub static_friction:  f32,
    pub dynamic_friction: f32,
    pub restitution:      f32,
    pub density:          f32,
    pub friction_combine:    CombineMode,
    pub restitution_combine: CombineMode,
}

impl Default for PhysicsMaterial {
    fn default() -> Self {
        Self {
            static_friction:     0.6,
            dynamic_friction:    0.5,
            restitution:         0.3,
            density:             1.0,
            friction_combine:    CombineMode::Multiply,
            restitution_combine: CombineMode::Max,
        }
    }
}

impl PhysicsMaterial {
    /// İki malzemenin temas özelliklerini birleştir
    pub fn combine(a: &PhysicsMaterial, b: &PhysicsMaterial) -> CombinedMaterial {
        CombinedMaterial {
            static_friction:  a.friction_combine.combine(a.static_friction,  b.static_friction),
            dynamic_friction: a.friction_combine.combine(a.dynamic_friction, b.dynamic_friction),
            restitution:      a.restitution_combine.combine(a.restitution,   b.restitution),
        }
    }

    // ── Hazır Malzemeler ──────────────────────────────────────────────────────

    pub fn rubber() -> Self {
        Self {
            static_friction: 1.0, dynamic_friction: 0.9,
            restitution: 0.8, density: 1.1,
            friction_combine: CombineMode::Max,
            restitution_combine: CombineMode::Max,
        }
    }

    pub fn ice() -> Self {
        Self {
            static_friction: 0.05, dynamic_friction: 0.03,
            restitution: 0.05, density: 0.92,
            friction_combine: CombineMode::Min,
            restitution_combine: CombineMode::Min,
        }
    }

    pub fn metal() -> Self {
        Self {
            static_friction: 0.4, dynamic_friction: 0.3,
            restitution: 0.3, density: 7.8,
            friction_combine: CombineMode::Multiply,
            restitution_combine: CombineMode::Average,
        }
    }

    pub fn wood() -> Self {
        Self {
            static_friction: 0.5, dynamic_friction: 0.4,
            restitution: 0.4, density: 0.6,
            friction_combine: CombineMode::Multiply,
            restitution_combine: CombineMode::Average,
        }
    }

    pub fn concrete() -> Self {
        Self {
            static_friction: 0.8, dynamic_friction: 0.7,
            restitution: 0.1, density: 2.4,
            friction_combine: CombineMode::Multiply,
            restitution_combine: CombineMode::Min,
        }
    }

    pub fn glass() -> Self {
        Self {
            static_friction: 0.2, dynamic_friction: 0.15,
            restitution: 0.6, density: 2.5,
            friction_combine: CombineMode::Min,
            restitution_combine: CombineMode::Max,
        }
    }

    pub fn asphalt() -> Self {
        Self {
            static_friction: 0.75, dynamic_friction: 0.65,
            restitution: 0.05, density: 2.3,
            friction_combine: CombineMode::Multiply,
            restitution_combine: CombineMode::Min,
        }
    }

    pub fn sand() -> Self {
        Self {
            static_friction: 0.55, dynamic_friction: 0.45,
            restitution: 0.02, density: 1.6,
            friction_combine: CombineMode::Average,
            restitution_combine: CombineMode::Min,
        }
    }
}

/// İki malzemenin birleşiminden elde edilen temas parametreleri
#[derive(Debug, Clone, Copy)]
pub struct CombinedMaterial {
    pub static_friction:  f32,
    pub dynamic_friction: f32,
    pub restitution:      f32,
}

gizmo_core::impl_component!(PhysicsMaterial);

// ─────────────────────────────────────────────────────────────────────────────
// Testler
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_combine_multiply() {
        let a = PhysicsMaterial { static_friction: 0.9, ..Default::default() };
        let b = PhysicsMaterial { static_friction: 0.4, ..Default::default() };
        let c = PhysicsMaterial::combine(&a, &b);
        let expected = (0.9f32 * 0.4).sqrt();
        assert!((c.static_friction - expected).abs() < 1e-5);
    }

    #[test]
    fn test_rubber_ice_low_friction() {
        let r = PhysicsMaterial::rubber();
        let i = PhysicsMaterial::ice();
        // Rubber has CombineMode::Max for friction, ice has Min
        // a.friction_combine (rubber) is used → Max(1.0, 0.05) = 1.0
        let c = PhysicsMaterial::combine(&r, &i);
        assert!(c.dynamic_friction > 0.5, "Rubber dominates on ice: {}", c.dynamic_friction);
    }

    #[test]
    fn test_restitution_max() {
        let a = PhysicsMaterial { restitution: 0.9, restitution_combine: CombineMode::Max, ..Default::default() };
        let b = PhysicsMaterial { restitution: 0.2, restitution_combine: CombineMode::Max, ..Default::default() };
        let c = PhysicsMaterial::combine(&a, &b);
        assert!((c.restitution - 0.9).abs() < 1e-5);
    }
}
