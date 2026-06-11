#![allow(non_snake_case)]

pub mod aba;
pub mod system;

use gizmo_math::spatial::{SpatialInertia, SpatialMatrix, SpatialVector};
use gizmo_math::{Mat3, Quat, Vec3};
use serde::{Deserialize, Serialize};

/// Uzaysal Transformasyon Matrisi (6x6)
/// Bir frame'den diğer frame'e uzaysal vektörleri taşır.
#[derive(Clone, Copy, Debug)]
pub struct SpatialTransform {
    pub rotation: Mat3,
    pub translation: Vec3,
}

impl SpatialTransform {
    pub const IDENTITY: Self = Self {
        rotation: Mat3::IDENTITY,
        translation: Vec3::ZERO,
    };

    pub fn new(rotation: Quat, translation: Vec3) -> Self {
        Self {
            rotation: Mat3::from_quat(rotation),
            translation,
        }
    }

    /// Vektörü bu frame'den ebeveyn frame'e çevirir.
    /// V_parent = X_parent_child * V_child
    pub fn transform_motion(self, v: SpatialVector) -> SpatialVector {
        let rw = self.rotation.mul_vec3(v.w);
        let rv = self.rotation.mul_vec3(v.v);
        SpatialVector::new(rw, self.translation.cross(rw) + rv)
    }

    /// Vektörü ebeveyn frame'den bu frame'e çevirir (Ters transform).
    /// V_child = X_child_parent * V_parent
    pub fn inverse_transform_motion(self, v: SpatialVector) -> SpatialVector {
        let rw = self.rotation.transpose().mul_vec3(v.w);
        let r_trans_cross = self.translation.cross(v.w);
        let rv = self.rotation.transpose().mul_vec3(v.v - r_trans_cross);
        SpatialVector::new(rw, rv)
    }

    /// Kuvveti bu frame'den ebeveyn frame'e çevirir.
    pub fn transform_force(self, f: SpatialVector) -> SpatialVector {
        let rw = self.rotation.mul_vec3(f.w);
        let rv = self.rotation.mul_vec3(f.v);
        SpatialVector::new(rw + self.translation.cross(rv), rv)
    }

    /// Kuvveti ebeveyn frame'den bu frame'e çevirir (Ters transform).
    pub fn inverse_transform_force(self, f: SpatialVector) -> SpatialVector {
        let r_trans_cross = self.translation.cross(f.v);
        let rw = self.rotation.transpose().mul_vec3(f.w - r_trans_cross);
        let rv = self.rotation.transpose().mul_vec3(f.v);
        SpatialVector::new(rw, rv)
    }

    /// Spatial Inertia'yı ebeveyn frame'e çevirir: I_parent = X_parent_child * I_child * X_child_parent
    pub fn transform_inertia(self, i: SpatialInertia) -> SpatialInertia {
        // Tam dönüşüm çok maliyetlidir. Pratik olarak kütle aynı kalır, CoM kaydırılır ve Rotasyon çevrilir.
        SpatialInertia {
            mass: i.mass,
            com: self.translation + self.rotation.mul_vec3(i.com),
            rot: self.rotation * i.rot * self.rotation.transpose(),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum JointType {
    Fixed,
    Revolute(Vec3), // Dönme ekseni (lokal)
    Prismatic(Vec3), // Kayma ekseni (lokal)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArticulatedLink {
    pub parent_index: usize, // Root ise usize::MAX (veya kendi indexi)
    pub joint_type: JointType,
    
    // Sabit yapısal transformasyon (Parent linkin sonundan bu linkin başlangıcına)
    pub transform_to_parent: Vec3,
    pub rotation_to_parent: Quat,

    // Fiziksel Özellikler
    pub inertia: SpatialInertia,

    // Eklem Durumları (State)
    pub q: f32,       // Eklem pozisyonu/açısı
    pub q_dot: f32,   // Eklem hızı
    pub q_ddot: f32,  // Eklem ivmesi (Hesaplanacak)
    
    pub joint_force: f32, // Motor veya dış müdahale kuvveti (tau)

    // --- Geçici ABA değişkenleri (Pass 1-3 sırasında hesaplanırlar) ---
    #[serde(skip)] pub v: SpatialVector,     // Spatial velocity
    #[serde(skip)] pub a: SpatialVector,     // Spatial acceleration (Coriolis/bias)
    #[serde(skip)] pub c: SpatialVector,     // Velocity product acceleration
    #[serde(skip)] pub i_a: SpatialMatrix,   // Articulated Body Inertia
    #[serde(skip)] pub p_a: SpatialVector,   // Bias force
    #[serde(skip)] pub S: SpatialVector,     // Motion subspace (Joint axis in spatial coords)
    #[serde(skip)] pub u: f32,               // tau - S^T * p_a
    #[serde(skip)] pub d_val: f32,               // S^T * i_a * S
    #[serde(skip)] pub u_vec: SpatialVector,     // i_a * S
}

impl ArticulatedLink {
    pub fn compute_spatial_transform(&self) -> SpatialTransform {
        // Eklem durumuna (q) göre yerel transform
        // Eksen normalize edilir: `from_axis_angle` birim eksen bekler ve normalize
        // edilmemiş eksen NaN/bozuk dönüş (ve ölçeklenmiş q̈) üretir.
        let (local_rot, local_trans) = match self.joint_type {
            JointType::Fixed => (Quat::IDENTITY, Vec3::ZERO),
            JointType::Revolute(axis) => {
                (Quat::from_axis_angle(axis.normalize_or_zero(), self.q), Vec3::ZERO)
            }
            JointType::Prismatic(axis) => (Quat::IDENTITY, axis.normalize_or_zero() * self.q),
        };

        // Toplam transform (Parent'a göre)
        let total_rot = self.rotation_to_parent * local_rot;
        let total_trans = self.transform_to_parent + self.rotation_to_parent.mul_vec3(local_trans);
        
        SpatialTransform::new(total_rot, total_trans)
    }

    pub fn compute_motion_subspace(&self) -> SpatialVector {
        // Normalize: normalize edilmemiş eksen S'i ölçekler → q̈ yanlış ölçeklenir.
        match self.joint_type {
            JointType::Fixed => SpatialVector::ZERO,
            JointType::Revolute(axis) => SpatialVector::new(axis.normalize_or_zero(), Vec3::ZERO),
            JointType::Prismatic(axis) => SpatialVector::new(Vec3::ZERO, axis.normalize_or_zero()),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArticulatedTree {
    pub links: Vec<ArticulatedLink>, // links[0] her zaman ROOT'tur.
    
    // Kök gövdenin (Base) uzaydaki konumu
    pub base_position: Vec3,
    pub base_rotation: Quat,
    pub base_velocity: SpatialVector, // (w, v)
    pub base_acceleration: SpatialVector, // (w, v)
    pub is_fixed_base: bool,
}

impl Default for ArticulatedTree {
    fn default() -> Self {
        Self {
            links: Vec::new(),
            base_position: Vec3::ZERO,
            base_rotation: Quat::IDENTITY,
            base_velocity: SpatialVector::ZERO,
            base_acceleration: SpatialVector::ZERO,
            is_fixed_base: true,
        }
    }
}
