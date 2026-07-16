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
        // Resolve against the SINGLE nearest hit. The old loop applied the position
        // snap once PER collider within range, so a node facing two adjacent surfaces
        // (a tiled floor, a wall meeting a floor) was advanced twice — launching it
        // straight PAST the geometry instead of stopping at the first surface.
        let mut nearest: Option<(f32, Vec3)> = None;
        for (_, col_trans, col) in rigid_colliders {
            if let Some((d, n)) =
                gizmo_physics_core::raycast::Raycast::ray_shape(&ray, &col.shape, col_trans)
            {
                if d <= dist + 0.1 && nearest.is_none_or(|(nd, _)| d < nd) {
                    nearest = Some((d, n));
                }
            }
        }

        if let Some((d, n)) = nearest {
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

    /// A node whose sweep ray hits two adjacent colliders (a tiled floor, a wall
    /// meeting a floor) must stop at the NEAREST surface, not be advanced once per
    /// collider — the old loop summed the per-collider snaps and launched the node
    /// clean through the geometry.
    #[test]
    fn resolve_node_collision_stops_at_nearest_of_adjacent_colliders() {
        use gizmo_physics_core::{BodyHandle, BoxShape, Collider, ColliderShape, Transform};

        let node = Vec3::ZERO;
        let vel = Vec3::new(60.0, 0.0, 0.0); // dist = 60 * (1/60) = 1.0
        let dt = 1.0 / 60.0;
        let thin = |cx: f32| {
            (
                BodyHandle::from_id(1),
                Transform::new(Vec3::new(cx, 0.0, 0.0)),
                Collider::from_shape(ColliderShape::Box(BoxShape {
                    half_extents: Vec3::new(0.05, 5.0, 5.0),
                })),
            )
        };
        // Near faces at x = 0.50 and x = 0.60 (both within the dist+0.1 window).
        let colliders = vec![thin(0.55), thin(0.65)];

        let (pos, _v, hit) = resolve_node_collision(node, vel, dt, &colliders);
        assert!(hit, "the node should register a collision");
        // Nearest surface is at x=0.50 → snap to ~0.40. The old per-collider sum
        // pushed it to ~0.90, past BOTH boxes (which end at x=0.70) — a tunnel.
        assert!(
            pos.x < 0.5,
            "node must stop before the first surface, got x = {} (tunneled through?)",
            pos.x
        );
    }

    // ---------------------------------------------------------------------
    // Tetrahedron::calculate_rest_data — reference-shape math.
    // ---------------------------------------------------------------------

    /// The canonical unit tetrahedron (edges = the basis vectors) has rest volume 1/6 and an
    /// identity inverse reference matrix.
    #[test]
    fn calculate_rest_data_unit_tet() {
        let (inv_dm, vol) = Tetrahedron::calculate_rest_data(Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z);
        assert!((vol - (1.0 / 6.0)).abs() < 1e-6, "unit tet volume must be 1/6, got {vol}");
        assert!(inv_dm.abs_diff_eq(Mat3::IDENTITY, 1e-6), "Dm = I → inv = I");
    }

    /// Volume is UNSIGNED (node winding must not flip its sign) and scales with the cube of a
    /// uniform edge scaling.
    #[test]
    fn calculate_rest_data_volume_is_unsigned_and_scales() {
        // Swapping two nodes negates det(Dm); volume must stay positive and equal.
        let (_, v_swapped) = Tetrahedron::calculate_rest_data(Vec3::ZERO, Vec3::Y, Vec3::X, Vec3::Z);
        assert!((v_swapped - (1.0 / 6.0)).abs() < 1e-6, "reversed winding must not flip volume");

        // Edges scaled x2 → volume x8.
        let (inv_dm, v_big) = Tetrahedron::calculate_rest_data(
            Vec3::ZERO,
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(0.0, 2.0, 0.0),
            Vec3::new(0.0, 0.0, 2.0),
        );
        assert!((v_big - (8.0 / 6.0)).abs() < 1e-5, "scaled volume must be 8/6, got {v_big}");
        // inv(diag(2,2,2)) = diag(0.5,0.5,0.5).
        assert!(inv_dm.abs_diff_eq(Mat3::from_diagonal(Vec3::splat(0.5)), 1e-6));
    }

    /// The deformation gradient `F = Ds · Dm⁻¹` equals identity when the element is at its
    /// rest configuration — the fundamental FEM invariant.
    #[test]
    fn deformation_gradient_is_identity_at_rest() {
        let (p0, p1, p2, p3) = (Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z);
        let (inv_dm, _) = Tetrahedron::calculate_rest_data(p0, p1, p2, p3);
        let ds = Mat3::from_cols(p1 - p0, p2 - p0, p3 - p0); // == Dm at rest
        let f = ds * inv_dm;
        assert!(f.abs_diff_eq(Mat3::IDENTITY, 1e-5), "F must be identity at rest, got {f:?}");
        assert!((f.determinant() - 1.0).abs() < 1e-5, "J must be 1 at rest");
    }

    /// A uniaxial stretch of factor 2 along X yields `F = diag(2, 1, 1)` and `J = 2`.
    #[test]
    fn deformation_gradient_tracks_uniaxial_stretch() {
        let (p0, p1, p2, p3) = (Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z);
        let (inv_dm, _) = Tetrahedron::calculate_rest_data(p0, p1, p2, p3);
        // Deform: double every x coordinate.
        let (d0, d1, d2, d3) = (
            Vec3::ZERO,
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        );
        let ds = Mat3::from_cols(d1 - d0, d2 - d0, d3 - d0);
        let f = ds * inv_dm;
        assert!(f.abs_diff_eq(Mat3::from_diagonal(Vec3::new(2.0, 1.0, 1.0)), 1e-5), "got {f:?}");
        assert!((f.determinant() - 2.0).abs() < 1e-5, "J must equal the stretch factor");
    }

    // ---------------------------------------------------------------------
    // SoftBodyMesh::new — material validation & Lamé derivation.
    // ---------------------------------------------------------------------

    /// Valid parameters derive the Lamé coefficients from Young's modulus and Poisson's ratio.
    #[test]
    fn new_derives_lame_parameters() {
        let sb = SoftBodyMesh::new(1000.0, 0.3).expect("valid");
        // mu = E / (2(1+nu)); lambda = E*nu / ((1+nu)(1-2nu)).
        assert!((sb.mu - (1000.0 / 2.6)).abs() < 1e-2, "mu = {}", sb.mu);
        assert!((sb.lambda - (300.0 / 0.52)).abs() < 1e-1, "lambda = {}", sb.lambda);
        assert!((sb.damping - 0.99).abs() < 1e-6);
        assert!(sb.nodes.is_empty() && sb.elements.is_empty());

        // nu = 0 → lambda = 0, mu = E/2 (no volumetric coupling).
        let sb0 = SoftBodyMesh::new(1000.0, 0.0).expect("valid");
        assert!((sb0.lambda - 0.0).abs() < 1e-4);
        assert!((sb0.mu - 500.0).abs() < 1e-3);
    }

    /// Young's modulus must be finite and strictly positive.
    #[test]
    fn new_rejects_invalid_youngs_modulus() {
        for bad in [0.0, -5.0, f32::NAN, f32::INFINITY] {
            let r = SoftBodyMesh::new(bad, 0.3);
            assert!(
                matches!(r, Err(crate::SoftBodyError::InvalidYoungsModulus { .. })),
                "E = {bad} must be rejected, got {r:?}"
            );
        }
    }

    /// Poisson's ratio must lie in `[0.0, 0.5)`; the incompressible limit and negatives are
    /// rejected (they blow up or destabilize the Lamé denominator).
    #[test]
    fn new_rejects_invalid_poissons_ratio() {
        for bad in [-0.1, 0.5, 0.6, f32::NAN] {
            let r = SoftBodyMesh::new(1000.0, bad);
            assert!(
                matches!(r, Err(crate::SoftBodyError::InvalidPoissonsRatio { .. })),
                "nu = {bad} must be rejected, got {r:?}"
            );
        }
    }

    // ---------------------------------------------------------------------
    // Node / element construction.
    // ---------------------------------------------------------------------

    /// `add_node` returns sequential indices and initializes velocity to zero / unpinned.
    #[test]
    fn add_node_returns_sequential_indices_and_defaults() {
        let mut sb = SoftBodyMesh::new(1000.0, 0.3).expect("valid");
        assert_eq!(sb.add_node(Vec3::X, 1.0), 0);
        assert_eq!(sb.add_node(Vec3::Y, 2.0), 1);
        assert_eq!(sb.add_node(Vec3::Z, 3.0), 2);
        assert_eq!(sb.nodes.len(), 3);
        let n = sb.nodes[1];
        assert!(n.position.abs_diff_eq(Vec3::Y, 1e-6));
        assert_eq!(n.mass, 2.0);
        assert_eq!(n.velocity, Vec3::ZERO);
        assert!(!n.is_fixed);
    }

    /// An element referencing a node index that does not exist yet is rejected and not stored.
    #[test]
    fn add_element_out_of_bounds_is_rejected() {
        let mut sb = SoftBodyMesh::new(1000.0, 0.3).expect("valid");
        sb.add_node(Vec3::ZERO, 1.0);
        sb.add_node(Vec3::X, 1.0);
        sb.add_node(Vec3::Y, 1.0);
        // Index 3 is out of bounds (only 0..=2 exist).
        let r = sb.add_element(0, 1, 2, 3);
        assert_eq!(
            r,
            Err(crate::SoftBodyError::NodeIndexOutOfBounds { index: 3, node_count: 3 })
        );
        assert!(sb.elements.is_empty(), "rejected element must not be stored");
    }

    /// A well-conditioned tetrahedron is accepted and stored with the correct rest volume.
    #[test]
    fn add_element_accepts_valid_tet_and_stores_rest_volume() {
        let mut sb = SoftBodyMesh::new(1000.0, 0.3).expect("valid");
        sb.add_node(Vec3::ZERO, 1.0);
        sb.add_node(Vec3::X, 1.0);
        sb.add_node(Vec3::Y, 1.0);
        sb.add_node(Vec3::Z, 1.0);
        sb.add_element(0, 1, 2, 3).expect("valid tet");
        assert_eq!(sb.elements.len(), 1);
        assert!((sb.elements[0].rest_volume - (1.0 / 6.0)).abs() < 1e-6);
        assert_eq!(sb.elements[0].node_indices, [0, 1, 2, 3]);
    }

    // ---------------------------------------------------------------------
    // resolve_node_collision — single-hit sweep resolution.
    // ---------------------------------------------------------------------

    /// A (near-)stationary node travels no distance, so collision resolution is skipped and
    /// state is returned unchanged.
    #[test]
    fn resolve_node_collision_zero_velocity_is_noop() {
        use gizmo_physics_core::{BodyHandle, Collider, Transform};
        let colliders = vec![(
            BodyHandle::from_id(1),
            Transform::new(Vec3::new(0.5, 0.0, 0.0)),
            Collider::sphere(1.0),
        )];
        let pos = Vec3::new(0.0, 1.0, 0.0);
        let (p, v, hit) = resolve_node_collision(pos, Vec3::ZERO, 1.0 / 60.0, &colliders);
        assert!(!hit);
        assert_eq!(p, pos);
        assert_eq!(v, Vec3::ZERO);
    }

    /// A surface beyond the node's swept distance (plus the small margin) is ignored: no
    /// collision is registered and the position is left for the caller to advance.
    #[test]
    fn resolve_node_collision_out_of_range_is_noop() {
        use gizmo_physics_core::{BodyHandle, Collider, Transform};
        let colliders = vec![(
            BodyHandle::from_id(1),
            Transform::new(Vec3::new(2.0, 0.0, 0.0)), // sphere surface at x = 1.5
            Collider::sphere(0.5),
        )];
        // Slow node: dist = 1 * (1/60) ≈ 0.017, far short of the x=1.5 surface.
        let (p, _v, hit) = resolve_node_collision(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), 1.0 / 60.0, &colliders);
        assert!(!hit, "surface out of sweep range must not collide");
        assert_eq!(p, Vec3::ZERO, "position must be untouched when nothing is hit");
    }

    /// A node sweeping into a sphere stops just short of the surface and reflects its inward
    /// velocity component (bounce), leaving the state finite.
    #[test]
    fn resolve_node_collision_bounces_off_sphere() {
        use gizmo_physics_core::{BodyHandle, Collider, Transform};
        let colliders = vec![(
            BodyHandle::from_id(1),
            Transform::new(Vec3::new(2.0, 0.0, 0.0)),
            Collider::sphere(0.5), // near surface at x = 1.5
        )];
        // dist = 120 * (1/60) = 2 > 1.5 → the ray reaches the surface.
        let (p, v, hit) = resolve_node_collision(Vec3::ZERO, Vec3::new(120.0, 0.0, 0.0), 1.0 / 60.0, &colliders);
        assert!(hit, "the node must register the sphere hit");
        assert!(p.x < 1.5, "node must stop before the surface, got x = {}", p.x);
        assert!(v.x < 0.0, "inward velocity must reflect (bounce), got vx = {}", v.x);
        assert!(p.is_finite() && v.is_finite());
    }

    // ---------------------------------------------------------------------
    // SoftBodyMesh::step — integration behaviour.
    // ---------------------------------------------------------------------

    /// With no elements the step is pure gravity integration: a free node accelerates downward.
    #[test]
    fn step_free_node_falls_under_gravity() {
        let mut sb = SoftBodyMesh::new(1000.0, 0.3).expect("valid");
        sb.add_node(Vec3::new(0.0, 10.0, 0.0), 1.0);
        sb.step(0.1, Vec3::new(0.0, -10.0, 0.0), &[]);
        assert!(sb.nodes[0].velocity.y < 0.0, "gravity must accelerate downward");
        assert!(sb.nodes[0].position.y < 10.0, "node must descend");
        assert!(sb.nodes[0].position.is_finite() && sb.nodes[0].velocity.is_finite());
    }

    /// Fixed nodes and nodes with non-positive mass are skipped entirely (they never move and
    /// never inject NaN/Inf), while a normal free node in the same body still integrates.
    #[test]
    fn step_fixed_and_negative_mass_nodes_do_not_move() {
        let mut sb = SoftBodyMesh::new(1000.0, 0.3).expect("valid");
        let fixed = sb.add_node(Vec3::new(0.0, 5.0, 0.0), 1.0) as usize;
        sb.nodes[fixed].is_fixed = true;
        let neg = sb.add_node(Vec3::new(1.0, 5.0, 0.0), -2.0) as usize; // negative mass
        let free = sb.add_node(Vec3::new(2.0, 5.0, 0.0), 1.0) as usize;

        let (fixed0, neg0) = (sb.nodes[fixed].position, sb.nodes[neg].position);
        for _ in 0..20 {
            sb.step(1.0 / 60.0, Vec3::new(0.0, -9.81, 0.0), &[]);
        }
        assert_eq!(sb.nodes[fixed].position, fixed0, "fixed node must not move");
        assert_eq!(sb.nodes[fixed].velocity, Vec3::ZERO);
        assert_eq!(sb.nodes[neg].position, neg0, "negative-mass node must be inert");
        assert_eq!(sb.nodes[neg].velocity, Vec3::ZERO);
        assert!(sb.nodes[free].position.y < 5.0, "the healthy free node must still fall");
        for n in &sb.nodes {
            assert!(n.position.is_finite() && n.velocity.is_finite());
        }
    }

    /// The FEM step is deterministic: parallel element force accumulation is summed in a fixed
    /// (element) order, so two identical bodies stay bit-identical.
    #[test]
    fn softbody_step_is_deterministic() {
        let build = || {
            let mut sb = SoftBodyMesh::new(1.0e5, 0.3).expect("valid");
            for p in [Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z] {
                sb.add_node(p, 1.0);
            }
            sb.add_element(0, 1, 2, 3).expect("valid");
            // Perturb so internal forces are non-trivial.
            sb.nodes[1].position *= 1.3;
            sb
        };
        let (mut a, mut b) = (build(), build());
        for _ in 0..120 {
            a.step(1.0 / 120.0, Vec3::new(0.0, -9.81, 0.0), &[]);
            b.step(1.0 / 120.0, Vec3::new(0.0, -9.81, 0.0), &[]);
        }
        assert_eq!(a, b, "identical soft bodies must evolve identically");
    }
}
