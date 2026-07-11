use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────────────────────────────────────
// Kombine Fonksiyon: iki malzemenin değerlerini nasıl birleştireceğimizi belirler
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[non_exhaustive]
pub enum CombineMode {
    Average, // (a + b) / 2
    Min,     // a.min(b)
    Max,     // a.max(b)
    #[default]
    GeometricMean, // sqrt(a * b)  — geometric mean (sürtünme için ideal)
}

impl CombineMode {
    pub fn combine(self, a: f32, b: f32) -> f32 {
        match self {
            CombineMode::Average => (a + b) * 0.5,
            CombineMode::Min => a.min(b),
            CombineMode::Max => a.max(b),
            CombineMode::GeometricMean => (a * b).max(0.0).sqrt(), // geometric mean
        }
    }
}

fn resolve_combine_mode(m1: CombineMode, m2: CombineMode) -> CombineMode {
    match (m1, m2) {
        (CombineMode::Max, _) | (_, CombineMode::Max) => CombineMode::Max,
        (CombineMode::Min, _) | (_, CombineMode::Min) => CombineMode::Min,
        (CombineMode::GeometricMean, _) | (_, CombineMode::GeometricMean) => {
            CombineMode::GeometricMean
        }
        _ => CombineMode::Average,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// PhysicsMaterial
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
// NOT `#[non_exhaustive]`: this is a plain value type that users routinely build
// with `PhysicsMaterial { static_friction: 0.9, ..Default::default() }` to author
// custom materials (the preset consts only cover a fixed set). Keeping it
// exhaustive preserves that ergonomic struct-literal API.
pub struct PhysicsMaterial {
    pub static_friction: f32,
    pub dynamic_friction: f32,
    pub restitution: f32,
    pub density: f32,
    pub friction_combine: CombineMode,
    pub restitution_combine: CombineMode,
}

impl Default for PhysicsMaterial {
    fn default() -> Self {
        Self {
            static_friction: 0.6,
            dynamic_friction: 0.5,
            restitution: 0.3,
            density: 1.0,
            friction_combine: CombineMode::GeometricMean,
            restitution_combine: CombineMode::Max,
        }
    }
}

impl PhysicsMaterial {
    /// Yalnız zıplaklığı (restitution) verilmiş malzeme kısayolu. `restitution_combine`
    /// varsayılan `Max` olduğundan bu malzeme, karşı yüzey mat olsa bile zıplar.
    /// Örn: `Collider::sphere(r).with_material(PhysicsMaterial::bouncy(0.9))`.
    pub fn bouncy(restitution: f32) -> Self {
        Self {
            restitution: restitution.clamp(0.0, 1.0),
            ..Default::default()
        }
    }

    /// Zıplaklığı (0=inelastik, 1=tam elastik) ayarlar. Zincirlenebilir.
    pub fn with_restitution(mut self, restitution: f32) -> Self {
        self.restitution = restitution.clamp(0.0, 1.0);
        self
    }

    /// Sürtünmeyi ayarlar (statik = dinamik = `friction`). Zincirlenebilir.
    pub fn with_friction(mut self, friction: f32) -> Self {
        let f = friction.max(0.0);
        self.static_friction = f;
        self.dynamic_friction = f;
        self
    }

    /// Sürtünmesiz malzeme kısayolu (buz gibi kaygan; restitution varsayılan).
    pub fn frictionless() -> Self {
        Self {
            static_friction: 0.0,
            dynamic_friction: 0.0,
            ..Default::default()
        }
    }

    /// Yoğunluğu (kütle/ hacim hesapları için) ayarlar. Zincirlenebilir.
    pub fn with_density(mut self, density: f32) -> Self {
        self.density = density.max(0.0);
        self
    }

    /// İki malzemenin temas özelliklerini birleştir
    pub fn combine(a: &PhysicsMaterial, b: &PhysicsMaterial) -> CombinedMaterial {
        let f_mode = resolve_combine_mode(a.friction_combine, b.friction_combine);
        let r_mode = resolve_combine_mode(a.restitution_combine, b.restitution_combine);

        CombinedMaterial {
            static_friction: f_mode.combine(a.static_friction, b.static_friction),
            dynamic_friction: f_mode.combine(a.dynamic_friction, b.dynamic_friction),
            restitution: r_mode.combine(a.restitution, b.restitution),
            density: CombineMode::Average.combine(a.density, b.density),
        }
    }

    // ── Hazır Malzemeler ──────────────────────────────────────────────────────

    pub const RUBBER: Self = Self {
        static_friction: 1.0,
        dynamic_friction: 0.9,
        restitution: 0.8,
        density: 1.1,
        friction_combine: CombineMode::Max,
        restitution_combine: CombineMode::Max,
    };

    pub const ICE: Self = Self {
        static_friction: 0.05,
        dynamic_friction: 0.03,
        restitution: 0.05,
        density: 0.92,
        friction_combine: CombineMode::Min,
        restitution_combine: CombineMode::Min,
    };

    pub const METAL: Self = Self {
        static_friction: 0.4,
        dynamic_friction: 0.3,
        restitution: 0.3,
        density: 7.8,
        friction_combine: CombineMode::GeometricMean,
        restitution_combine: CombineMode::Average,
    };

    pub const WOOD: Self = Self {
        static_friction: 0.5,
        dynamic_friction: 0.4,
        restitution: 0.4,
        density: 0.6,
        friction_combine: CombineMode::GeometricMean,
        restitution_combine: CombineMode::Average,
    };

    pub const CONCRETE: Self = Self {
        static_friction: 0.8,
        dynamic_friction: 0.7,
        restitution: 0.1,
        density: 2.4,
        friction_combine: CombineMode::GeometricMean,
        restitution_combine: CombineMode::Min,
    };

    pub const GLASS: Self = Self {
        static_friction: 0.2,
        dynamic_friction: 0.15,
        restitution: 0.6,
        density: 2.5,
        friction_combine: CombineMode::Min,
        restitution_combine: CombineMode::Max,
    };

    pub const ASPHALT: Self = Self {
        static_friction: 0.75,
        dynamic_friction: 0.65,
        restitution: 0.05,
        density: 2.3,
        friction_combine: CombineMode::GeometricMean,
        restitution_combine: CombineMode::Min,
    };

    pub const SAND: Self = Self {
        static_friction: 0.55,
        dynamic_friction: 0.45,
        restitution: 0.02,
        density: 1.6,
        friction_combine: CombineMode::Average,
        restitution_combine: CombineMode::Min,
    };
}

/// İki malzemenin birleşiminden elde edilen temas parametreleri
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CombinedMaterial {
    pub static_friction: f32,
    pub dynamic_friction: f32,
    pub restitution: f32,
    pub density: f32,
}

gizmo_core::impl_component!(PhysicsMaterial);

// ─────────────────────────────────────────────────────────────────────────────
// Testler
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ergonomic_material_builders() {
        // `bouncy(e)`: restitution ayarlı, combine=Max (varsayılan) → karşı yüzey mat
        // olsa bile zıplar.
        let m = PhysicsMaterial::bouncy(0.9);
        assert_eq!(m.restitution, 0.9);
        assert_eq!(m.restitution_combine, CombineMode::Max);
        // `with_restitution` / `with_friction` zincirlenebilir + clamp'li.
        let m2 = PhysicsMaterial::default().with_restitution(1.5).with_friction(0.7);
        assert_eq!(m2.restitution, 1.0, "restitution [0,1] aralığına clamp'lanmalı");
        assert_eq!(m2.static_friction, 0.7);
        assert_eq!(m2.dynamic_friction, 0.7);
        assert_eq!(
            PhysicsMaterial::default().with_restitution(-0.5).restitution,
            0.0
        );
        // frictionless: sürtünme sıfır.
        let f = PhysicsMaterial::frictionless();
        assert_eq!(f.static_friction, 0.0);
        assert_eq!(f.dynamic_friction, 0.0);
        // with_density zincirlenebilir.
        assert_eq!(PhysicsMaterial::default().with_density(3.0).density, 3.0);
    }

    #[test]
    fn test_combine_geometric_mean() {
        let a = PhysicsMaterial {
            static_friction: 0.9,
            ..Default::default()
        };
        let b = PhysicsMaterial {
            static_friction: 0.4,
            ..Default::default()
        };
        let c = PhysicsMaterial::combine(&a, &b);
        let expected = (0.9f32 * 0.4).sqrt();
        assert!((c.static_friction - expected).abs() < 1e-5);
    }

    #[test]
    fn test_rubber_ice_low_friction() {
        let r = PhysicsMaterial::RUBBER;
        let i = PhysicsMaterial::ICE;
        // Rubber has CombineMode::Max, Ice has CombineMode::Min.
        // Due to priority Max > Min, the resolved mode is Max.
        // So the dynamic friction is Max(0.9, 0.03) = 0.9
        let c = PhysicsMaterial::combine(&r, &i);
        assert!(
            c.dynamic_friction > 0.5,
            "Rubber's Max mode dominates Ice's Min mode"
        );
    }

    #[test]
    fn test_restitution_max() {
        let a = PhysicsMaterial {
            restitution: 0.9,
            restitution_combine: CombineMode::Max,
            ..Default::default()
        };
        let b = PhysicsMaterial {
            restitution: 0.2,
            restitution_combine: CombineMode::Max,
            ..Default::default()
        };
        let c = PhysicsMaterial::combine(&a, &b);
        assert!((c.restitution - 0.9).abs() < 1e-5);
    }
}
