use gizmo_math::{Mat3, Vec3};
use gizmo_physics_core::BodyHandle;

/// Represents a single vertex/node in the FEM soft body mesh.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoftBodyNode {
    pub position: Vec3,
    pub velocity: Vec3,
    pub mass: f32,
    pub is_fixed: bool, // For pinning parts of the body (e.g. chassis mounts)
}

/// A 3D Tetrahedron (4 nodes) used as the fundamental finite element.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tetrahedron {
    /// Indices into the nodes array of the parent SoftBody
    pub node_indices: [u32; 4],

    /// Rest Volume (V0) - calculated once at initialization
    pub rest_volume: f32,

    /// The inverse of the reference shape matrix (Dm^-1)
    /// Used by the GPU to quickly calculate the Deformation Gradient (F = Ds * Dm^-1)
    pub inv_rest_matrix: Mat3,
}

impl Tetrahedron {
    /// Calculate the inverse rest matrix (Dm^-1) and rest volume for the tetrahedron.
    /// Needs the 4 rest positions (x0, x1, x2, x3) of the nodes.
    pub fn calculate_rest_data(p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3) -> (Mat3, f32) {
        // Dm = [ (p1-p0) | (p2-p0) | (p3-p0) ]
        let e1 = p1 - p0;
        let e2 = p2 - p0;
        let e3 = p3 - p0;

        let dm = Mat3::from_cols(e1, e2, e3);

        // V = 1/6 * det(Dm)
        let det = dm.determinant();
        let volume = (det / 6.0).abs();

        let inv_dm = dm.inverse();

        (inv_dm, volume)
    }
}

/// The main CPU-side structure for an FEM-based Soft Body.
/// It contains the geometry and material properties needed to upload to the GPU.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct SoftBodyMesh {
    pub nodes: Vec<SoftBodyNode>,
    pub elements: Vec<Tetrahedron>,

    // --- Material Properties (Elasto-Plasticity) ---
    /// Young's Modulus (E) - Stiffness of the material (e.g., higher = stiffer metal)
    pub youngs_modulus: f32,

    /// Poisson's Ratio (nu) - Incompressibility (0.0 to 0.499, e.g., 0.3 for steel, 0.49 for rubber)
    pub poissons_ratio: f32,

    /// Lame's first parameter (lambda) - derived
    pub lambda: f32,

    /// Shear Modulus (mu) - derived
    pub mu: f32,

    /// Damping factor to prevent infinite oscillation
    pub damping: f32,
}
gizmo_core::impl_component!(SoftBodyMesh);

pub fn resolve_node_collision(
    mut position: Vec3,
    mut velocity: Vec3,
    dt: f32,
    rigid_colliders: &[(
        BodyHandle,
        gizmo_physics_core::Transform,
        gizmo_physics_core::Collider,
    )],
) -> (Vec3, Vec3, bool) {
    let mut collided = false;
    let ray = gizmo_physics_core::raycast::Ray::new(position, velocity.normalize_or_zero());
    let dist = velocity.length() * dt;

    if dist > 1e-5 {
        for (_, col_trans, col) in rigid_colliders {
            if let Some((d, n)) = gizmo_physics_core::raycast::Raycast::ray_shape(&ray, &col.shape, col_trans) {
                if d <= dist + 0.1 {
                    let bounce = 0.5;
                    let friction = 0.8;

                    let vn = velocity.dot(n);
                    if vn < 0.0 {
                        let vt = velocity - n * vn;
                        velocity = vt * (1.0 - friction) - n * (vn * bounce);
                    }

                    position += ray.direction * (d - 0.1).max(0.0);
                    collided = true;
                }
            }
        }
    }

    (position, velocity, collided)
}

impl SoftBodyMesh {
    /// Creates a new (empty) FEM soft body from material parameters.
    ///
    /// # Errors
    ///
    /// Returns [`SoftBodyError::InvalidYoungsModulus`] if `youngs_modulus` is
    /// not finite and strictly positive, and [`SoftBodyError::InvalidPoissonsRatio`]
    /// if `poissons_ratio` is outside the physically valid range `[0.0, 0.5)`.
    /// `nu >= 0.5` zeroes the Lamé denominator `(1 - 2·nu)` (incompressible
    /// limit → `+inf`), `nu > 0.5` yields a negative (unstable) `lambda`, and
    /// `nu < 0.0` is unsupported. Rejecting them here keeps the derived
    /// `lambda`/`mu` finite and stable instead of silently clamping.
    pub fn new(youngs_modulus: f32, poissons_ratio: f32) -> Result<Self, crate::SoftBodyError> {
        if !youngs_modulus.is_finite() || youngs_modulus <= 0.0 {
            return Err(crate::SoftBodyError::InvalidYoungsModulus {
                value: youngs_modulus,
            });
        }
        let nu = poissons_ratio;
        if !nu.is_finite() || !(0.0..0.5).contains(&nu) {
            return Err(crate::SoftBodyError::InvalidPoissonsRatio { value: nu });
        }

        // Calculate Lame Parameters
        let mu = youngs_modulus / (2.0 * (1.0 + nu));
        let lambda = (youngs_modulus * nu) / ((1.0 + nu) * (1.0 - 2.0 * nu));

        Ok(Self {
            nodes: Vec::new(),
            elements: Vec::new(),
            youngs_modulus,
            poissons_ratio: nu,
            lambda,
            mu,
            damping: 0.99,
        })
    }

    pub fn add_node(&mut self, position: Vec3, mass: f32) -> u32 {
        let idx = self.nodes.len() as u32;
        self.nodes.push(SoftBodyNode {
            position,
            velocity: Vec3::ZERO,
            mass,
            is_fixed: false,
        });
        idx
    }

    /// Adds a tetrahedral element referencing four existing node indices.
    ///
    /// # Errors
    ///
    /// Returns [`SoftBodyError::NodeIndexOutOfBounds`] if any of the four
    /// indices refers to a node that has not been added yet. On success the
    /// element is appended exactly as before.
    pub fn add_element(
        &mut self,
        i0: u32,
        i1: u32,
        i2: u32,
        i3: u32,
    ) -> Result<(), crate::SoftBodyError> {
        let n = self.nodes.len() as u32;
        for index in [i0, i1, i2, i3] {
            if index >= n {
                return Err(crate::SoftBodyError::NodeIndexOutOfBounds {
                    index,
                    node_count: n,
                });
            }
        }

        let p0 = self.nodes[i0 as usize].position;
        let p1 = self.nodes[i1 as usize].position;
        let p2 = self.nodes[i2 as usize].position;
        let p3 = self.nodes[i3 as usize].position;

        let (inv_rest_matrix, rest_volume) = Tetrahedron::calculate_rest_data(p0, p1, p2, p3);

        // Reject (near-)degenerate tetrahedra at construction (fail-fast): a
        // near-zero rest volume means the four nodes are (nearly) coplanar, so
        // `Dm` is singular, `inv_rest_matrix` is undefined, and the derived
        // elastic forces would be near-zero / NaN — the element would never
        // recover from compression. Only well-conditioned elements are stored.
        const MIN_REST_VOLUME: f32 = 1e-6;
        if rest_volume <= MIN_REST_VOLUME || rest_volume.is_nan() || !inv_rest_matrix.is_finite() {
            return Err(crate::SoftBodyError::DegenerateTetrahedron {
                volume: rest_volume,
            });
        }

        self.elements.push(Tetrahedron {
            node_indices: [i0, i1, i2, i3],
            rest_volume,
            inv_rest_matrix,
        });
        Ok(())
    }

    /// Advances the FEM simulation by one timestep using a Neo-Hookean hyperelastic model.
    pub fn step(
        &mut self,
        dt: f32,
        gravity: Vec3,
        rigid_colliders: &[(
            BodyHandle,
            gizmo_physics_core::Transform,
            gizmo_physics_core::Collider,
        )],
    ) {
        let mut forces: Vec<Vec3> = self.nodes.iter().map(|n| gravity * n.mass).collect();

        // 1. Calculate and accumulate internal elastic forces from all tetrahedra in PARALLEL
        #[cfg(not(target_arch = "wasm32"))]
        use rayon::prelude::*;
        #[cfg(target_arch = "wasm32")]
        use crate::parallel_compat::*;

        let positions: Vec<Vec3> = self.nodes.iter().map(|n| n.position).collect();
        // Yalnızca gerçekten dejenere/ters (J ≤ eps) elemanlar atlanır — NaN/tekillik
        // koruması. Eskiden J < 0.1 ile GEÇERLİ ama sıkışmış elemanlar da tüm sertliğini
        // kaybediyordu (sıkışma altında çöküyor, geri toparlanamıyordu).
        const J_EPS: f32 = 1e-4;

        // Her tetrahedronun düğüm kuvvetlerini PARALEL hesapla, sonra DETERMİNİSTİK
        // (eleman sırasında) topla. (Paralel `reduce` float toplamı sırayı bozup
        // lockstep determinizmini kırıyordu — float toplaması birleşmeli değil.)
        let elem_forces: Vec<Option<([usize; 4], [Vec3; 4])>> = self
            .elements
            .par_iter()
            .map(|elem| {
                let idx = [
                    elem.node_indices[0] as usize,
                    elem.node_indices[1] as usize,
                    elem.node_indices[2] as usize,
                    elem.node_indices[3] as usize,
                ];
                let ds = Mat3::from_cols(
                    positions[idx[1]] - positions[idx[0]],
                    positions[idx[2]] - positions[idx[0]],
                    positions[idx[3]] - positions[idx[0]],
                );
                let f = ds * elem.inv_rest_matrix;
                let j = f.determinant();
                if j <= J_EPS {
                    return None;
                }
                let f_inv_t = f.inverse().transpose();
                let ln_j = j.ln();
                let p = f_inv_t * (-self.mu) + f * self.mu + f_inv_t * (self.lambda * ln_j);
                let h = p * elem.inv_rest_matrix.transpose() * elem.rest_volume;
                let f1 = -h.col(0);
                let f2 = -h.col(1);
                let f3 = -h.col(2);
                let f0 = -(f1 + f2 + f3);
                // NaN/Inf koruması: tutarsız kuvvet üretildiyse atla.
                if !(f0.is_finite() && f1.is_finite() && f2.is_finite() && f3.is_finite()) {
                    return None;
                }
                Some((idx, [f0, f1, f2, f3]))
            })
            .collect();

        for (idx, f) in elem_forces.into_iter().flatten() {
            forces[idx[0]] += f[0];
            forces[idx[1]] += f[1];
            forces[idx[2]] += f[2];
            forces[idx[3]] += f[3];
        }

        // 2. Integrate velocities and positions
        for (i, node) in self.nodes.iter_mut().enumerate() {
            // Skip pinned nodes and any node with non-positive / non-finite mass:
            // dividing `forces[i] / node.mass` by `mass <= 0` yields Inf/NaN that
            // then poisons velocity and position forever. Such a node behaves as
            // if it were fixed (immovable) rather than exploding the sim.
            if node.is_fixed || node.mass <= 0.0 || node.mass.is_nan() {
                continue;
            }

            // Explicit Euler Integration
            let acceleration = forces[i] / node.mass;
            node.velocity += acceleration * dt;

            // Apply damping normalized by dt
            node.velocity *= self.damping.powf(dt);

            let next_pos = node.position + node.velocity * dt;

            let (new_pos, new_vel, collided) =
                resolve_node_collision(node.position, node.velocity, dt, rigid_colliders);

            if collided {
                node.position = new_pos;
                node.velocity = new_vel;
            } else {
                node.position = next_pos;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Orta derecede sıkışmış GEÇERLİ bir eleman (J ≈ 0.4³ ≈ 0.064, eski 0.1 eşiğinin
    /// altında) artık direnç gösterip geri açılmalı. (Eski kod J < 0.1 olunca sıfır kuvvet
    /// uyguluyordu → eleman çökük kalıyordu.) Ayrıca NaN/Inf üretmemeli.
    #[test]
    fn resists_moderate_compression_and_stays_finite() {
        let mut sb = SoftBodyMesh::new(1000.0, 0.3).expect("valid material params");
        sb.add_node(Vec3::ZERO, 1.0);
        sb.add_node(Vec3::X, 1.0);
        sb.add_node(Vec3::Y, 1.0);
        sb.add_node(Vec3::Z, 1.0);
        sb.add_element(0, 1, 2, 3).expect("valid node indices");

        // Üniform sıkıştır: F = 0.4·I → J = 0.064 (< eski 0.1 eşiği).
        for node in &mut sb.nodes {
            node.position *= 0.4;
        }
        let d_before = (sb.nodes[1].position - sb.nodes[0].position).length();

        // Yerçekimsiz: yalnızca iç elastik kuvvetler etkili olsun.
        for _ in 0..120 {
            sb.step(1.0 / 240.0, Vec3::ZERO, &[]);
        }

        let d_after = (sb.nodes[1].position - sb.nodes[0].position).length();
        assert!(
            d_after > d_before + 1e-4,
            "sıkışmış eleman geri açılmalı (direnç): {d_before} -> {d_after}"
        );
        for node in &sb.nodes {
            assert!(node.position.is_finite(), "pozisyon NaN/Inf olmamalı");
            assert!(node.velocity.is_finite(), "hız NaN/Inf olmamalı");
        }
    }

    /// Kütlesi 0 (ve is_fixed=false) bir düğüm eskiden `forces/mass` = Inf/NaN
    /// üretip tüm simülasyonu zehirliyordu. Artık böyle düğüm sabit gibi atlanır.
    #[test]
    fn zero_mass_node_does_not_poison_simulation() {
        let mut sb = SoftBodyMesh::new(1000.0, 0.3).expect("valid material params");
        // Kütlesiz (0.0) düğüm — pinlenmemiş.
        let zero_idx = sb.add_node(Vec3::new(0.5, 2.0, 0.5), 0.0) as usize;
        // Normal kütleli düğümler.
        sb.add_node(Vec3::X, 1.0);
        sb.add_node(Vec3::Y, 1.0);
        sb.add_node(Vec3::Z, 1.0);

        for _ in 0..30 {
            sb.step(1.0 / 240.0, Vec3::new(0.0, -9.81, 0.0), &[]);
        }

        for node in &sb.nodes {
            assert!(
                node.position.is_finite(),
                "kütlesiz düğüm Inf/NaN yaymamalı: {:?}",
                node.position
            );
            assert!(node.velocity.is_finite(), "hız Inf/NaN olmamalı");
        }
        // Kütlesiz düğüm hareket etmemeli (sabit gibi davranmalı).
        assert_eq!(sb.nodes[zero_idx].position, Vec3::new(0.5, 2.0, 0.5));
        assert_eq!(sb.nodes[zero_idx].velocity, Vec3::ZERO);
    }

    /// Dört (neredeyse) düzlemsel düğümle oluşturulan dejenere bir tetrahedron
    /// rest_volume ≈ 0 verir; artık `add_element` bunu reddetmeli.
    #[test]
    fn degenerate_tetrahedron_is_rejected() {
        let mut sb = SoftBodyMesh::new(1000.0, 0.3).expect("valid material params");
        // Dört düğüm de z=0 düzleminde → koplanar → hacim ≈ 0.
        sb.add_node(Vec3::new(0.0, 0.0, 0.0), 1.0);
        sb.add_node(Vec3::new(1.0, 0.0, 0.0), 1.0);
        sb.add_node(Vec3::new(0.0, 1.0, 0.0), 1.0);
        sb.add_node(Vec3::new(1.0, 1.0, 0.0), 1.0);

        let result = sb.add_element(0, 1, 2, 3);
        assert!(
            matches!(
                result,
                Err(crate::SoftBodyError::DegenerateTetrahedron { .. })
            ),
            "koplanar (dejenere) tetrahedron reddedilmeli, ama: {result:?}"
        );
        // Reddedilen eleman eklenmemeli.
        assert!(sb.elements.is_empty());
    }
}
