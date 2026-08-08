//! # ⚠️ Experimental — articulated-body (multibody) dynamics
//!
//! Featherstone Articulated Body Algorithm (ABA) forward dynamics in reduced
//! (joint-space) coordinates: an [`ArticulatedTree`] of [`ArticulatedLink`]s
//! connected by Fixed / Revolute / Prismatic joints, solved with 6D spatial
//! algebra. [`aba::compute_aba`] fills each link's `q_ddot`, and
//! [`system::step_articulated_trees`] integrates the joint state.
//!
//! **This module is gated behind the `experimental-multibody` feature and is
//! deliberately absent from the default (1.0) public API.** It is a correct,
//! property-tested *kernel*, not a finished subsystem. Known limitations:
//!
//! - **Not ECS-integrated.** [`ArticulatedTree`] is a plain struct, not a
//!   component; nothing in [`crate::world::PhysicsWorld`] or the ECS schedule
//!   steps it. You drive it yourself via [`system::step_articulated_trees`].
//! - **No collision coupling.** Links do not collide with each other or with
//!   [`crate::components::RigidBody`] world geometry — pure forward dynamics.
//! - **Fixed-base only.** Floating-base integration is unfinished: the base
//!   acceleration is not propagated back from the ABA passes (see
//!   [`system::step_articulated_trees`]), so `is_fixed_base = false` trees do
//!   not translate/rotate their root. Their *joints* are solved correctly and
//!   do feel gravity — that half was broken through 0.9.0 and is fixed.
//!
//! Wiring these into the engine (component + scheduled system + world-transform
//! export + rigid-body collision coupling + floating base) is tracked as a
//! larger task in the ROADMAP. Until then it stays opt-in.

#![allow(non_snake_case)]

/// The three-pass Featherstone kernel. Forward dynamics only: it takes no timestep and
/// advances nothing in time, so `q` and `q_dot` come back exactly as they went in and only
/// `q_ddot` plus the `#[serde(skip)]` scratch registers are written.
pub mod aba;

/// The semi-implicit Euler integrator layered on top of [`aba`] — the only code in this
/// module that writes `q_dot` and `q`, and therefore the only place a tree's joint state
/// advances in time.
pub mod system;

use gizmo_math::spatial::{SpatialInertia, SpatialMatrix, SpatialVector};
use gizmo_math::{Mat3, Quat, Vec3};
use serde::{Deserialize, Serialize};

/// Uzaysal Transformasyon Matrisi (6x6)
/// Bir frame'den diğer frame'e uzaysal vektörleri taşır.
#[derive(Clone, Copy, Debug)]
pub struct SpatialTransform {
    /// Orientation of the child frame expressed in the parent frame, as a rotation
    /// matrix — the `Quat` given to [`SpatialTransform::new`] is converted once, here,
    /// and the matrix is what every method uses.
    ///
    /// Assumed orthonormal: the two `inverse_*` methods invert it with `transpose()`
    /// rather than a real inverse, so if this is not a rotation they stop being the
    /// inverses of their forward counterparts.
    pub rotation: Mat3,
    /// Position of the child frame's origin in metres, expressed in the **parent**
    /// frame — not the parent's origin seen from the child.
    ///
    /// This is the moment arm, i.e. the only thing that couples the angular and linear
    /// halves of a spatial vector: with `Vec3::ZERO` all four transform methods
    /// degenerate to rotating the two halves independently.
    pub translation: Vec3,
}

impl SpatialTransform {
    /// Coincident frames: no rotation and no offset.
    ///
    /// With a zero moment arm it neither reorients a spatial vector nor mixes its
    /// angular and linear halves, so it is the neutral element to start a chain of
    /// frame changes from. Provided for callers only — the ABA solver never uses it,
    /// as it builds every link transform from that link's own joint state.
    pub const IDENTITY: Self = Self {
        rotation: Mat3::IDENTITY,
        translation: Vec3::ZERO,
    };

    /// Assembles a child→parent transform from the child frame's orientation in the
    /// parent (`rotation`) and the child origin's position in parent coordinates
    /// (`translation`, metres).
    ///
    /// `rotation` is expected to be a **unit** quaternion. Nothing checks or
    /// re-normalises it — a non-unit input is not generally a rotation once converted
    /// to the [`SpatialTransform::rotation`] matrix, and the error then rides along
    /// silently through every transform built on it.
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

/// The joint that attaches a link to its parent.
///
/// As it stands every variant is **at most single-DOF** — one for
/// [`Revolute`](JointType::Revolute) and [`Prismatic`](JointType::Prismatic), none at all
/// for [`Fixed`](JointType::Fixed) — which is why an [`ArticulatedLink`] carries one scalar
/// `q`/`q_dot`/`q_ddot` and one scalar `joint_force`: a ball or free joint has no
/// representation here, and must be approximated by chaining several links with zero-length
/// offsets. The variant also fixes the units of that scalar state — radians and N·m for a
/// hinge, metres and newtons for a slider.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum JointType {
    /// Welded: zero DOF, the link keeps exactly the pose given by its
    /// `transform_to_parent` / `rotation_to_parent`.
    ///
    /// Its motion subspace is `SpatialVector::ZERO`, which sends the articulated joint
    /// inertia `D` to zero; the solver reads that as "locked" and carries the child's
    /// *full* inertia and bias force up to the parent instead of projecting a DOF out of
    /// them. `q_ddot` is forced to `0.0`, and `q`, `q_dot` and `joint_force` influence
    /// neither the pose nor the dynamics.
    Fixed,
    /// Hinge about the given axis. `q` is the hinge angle in radians, `q_dot` rad/s,
    /// `q_ddot` rad/s² and `joint_force` a motor torque in N·m.
    ///
    /// The axis is expressed in the joint frame — the parent frame rotated by
    /// `rotation_to_parent`, which for a hinge is also the link's own frame, since
    /// rotating about the axis leaves the axis itself fixed. It is normalised at every
    /// use, so its length is irrelevant, and `Vec3::ZERO` is degenerate: it contributes
    /// no rotation and a zero motion subspace, collapsing to the same locked behaviour
    /// as [`JointType::Fixed`].
    Revolute(Vec3), // Dönme ekseni (lokal)
    /// Slider along the given axis. `q` is the extension in metres, `q_dot` m/s,
    /// `q_ddot` m/s² and `joint_force` an actuator force in newtons; the link's
    /// orientation relative to its parent stays `rotation_to_parent` at every `q`.
    ///
    /// The axis is read and degenerates exactly as described for
    /// [`Revolute`](JointType::Revolute), except that here it points along the direction of
    /// travel: `q` is measured along it, and the offset it produces is added on top of the
    /// link's `transform_to_parent`.
    Prismatic(Vec3), // Kayma ekseni (lokal)
}

/// One rigid link together with the single joint that attaches it to its parent — the
/// joint is owned by the *child*, so a tree of N links has exactly N joints.
///
/// The fields form three groups. **Structure** (`parent_index`, `joint_type`,
/// `transform_to_parent`, `rotation_to_parent`, `inertia`) is set up once and describes
/// the mechanism. **Joint state** is the scalars `q`, `q_dot` and `joint_force` going in
/// and `q_ddot` coming out — the only part that changes from step to step. The remaining
/// `#[serde(skip)]` fields are ABA scratch registers: [`aba::compute_aba`] rewrites all
/// of them on every call, so they carry no information across a serialisation round-trip
/// and lose nothing by coming back zeroed.
///
/// There is deliberately no `Default`, so a struct literal has to spell the scratch
/// registers out; `SpatialVector::ZERO` and `SpatialMatrix::ZERO` are the conventional
/// placeholders for them.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArticulatedLink {
    /// Index of this link's parent within [`ArticulatedTree::links`], or `usize::MAX`
    /// for a link hanging directly off the tree's base. `usize::MAX` is the *only* root
    /// marker the solver recognises; any other value is used as a raw index and will
    /// panic if it is out of bounds.
    ///
    /// **Ordering invariant:** a parent must appear *before* its child in `links`, i.e.
    /// `parent_index` must be smaller than this link's own index. The solver's passes
    /// are single linear sweeps that read the parent's velocity/acceleration going down
    /// and accumulate into the parent's articulated inertia going up; a child stored
    /// ahead of its parent silently produces wrong numbers rather than an error. Any
    /// topological order of the tree satisfies this.
    pub parent_index: usize, // Root ise usize::MAX (veya kendi indexi)
    /// The joint connecting the parent frame to this link, axis included.
    ///
    /// It selects the units of `q`/`q_dot`/`q_ddot`/`joint_force` and the motion
    /// subspace the solver projects onto. Swapping it mid-simulation reinterprets the
    /// existing `q` in place without converting it — radians and metres are not the same
    /// number.
    pub joint_type: JointType,
    
    // Sabit yapısal transformasyon (Parent linkin sonundan bu linkin başlangıcına)
    /// Constant offset in metres from the parent frame's origin to this link's joint
    /// frame origin, expressed in the **parent** frame. This is geometry, not state: the
    /// joint's own displacement is added on top of it, and only a
    /// [`JointType::Prismatic`] joint adds anything at all.
    ///
    /// For a root link the "parent" is the tree's base frame.
    pub transform_to_parent: Vec3,
    /// Constant orientation of this link's joint frame relative to the parent frame,
    /// i.e. the orientation at `q = 0`.
    ///
    /// The joint's own rotation is composed as `R_parent_child = rotation_to_parent *
    /// R_joint(q)`, which is exactly why a [`JointType::Revolute`] axis is read in the
    /// already-rotated joint frame and not in the parent's. Expected to be a unit
    /// quaternion; it is never re-normalised, here or in
    /// [`ArticulatedLink::compute_spatial_transform`].
    pub rotation_to_parent: Quat,

    // Fiziksel Özellikler
    /// Mass, rotational inertia and centre of mass of **this link alone** — never of its
    /// subtree, which the solver folds in separately each step.
    ///
    /// Frame trap: the `com` offset is measured from this link's joint frame origin,
    /// i.e. from the pivot, not from the link's own centroid. `Vec3::ZERO` therefore
    /// places the mass exactly on the pivot, and a hinge built that way gets no gravity
    /// torque from its own weight. The `rot` tensor is taken about the centre of mass
    /// (kg·m²) and the parallel-axis shift out to the pivot is applied for you at solve
    /// time.
    pub inertia: SpatialInertia,

    // Eklem Durumları (State)
    /// Joint coordinate: radians for [`JointType::Revolute`], metres for
    /// [`JointType::Prismatic`], ignored by [`JointType::Fixed`].
    ///
    /// Unbounded — this kernel has no joint limits, and an angle is never wrapped into
    /// `[-π, π]`, so a hinge left spinning accumulates a large `q` where f32 angle
    /// resolution degrades. Write it directly to pose the tree;
    /// [`system::step_articulated_trees`] integrates it from `q_dot`.
    pub q: f32,       // Eklem pozisyonu/açısı
    /// Joint velocity in rad/s or m/s, matching the unit of `q`.
    ///
    /// Besides being integrated, it is what produces the Coriolis/centrifugal bias
    /// term `c = v × (S·q_dot)` — which vanishes identically for any link whose `q_dot`
    /// is zero, so a test built only from rest states cannot observe bias-term errors.
    pub q_dot: f32,   // Eklem hızı
    /// Joint acceleration in rad/s² or m/s² — an **output**. [`aba::compute_aba`]
    /// overwrites it on every call, so anything stored here beforehand is discarded, and
    /// it comes out `0.0` for a joint the solver considers locked or numerically
    /// singular.
    pub q_ddot: f32,  // Eklem ivmesi (Hesaplanacak)
    
    /// Applied joint effort τ: a torque in N·m for [`JointType::Revolute`], a force in
    /// newtons for [`JointType::Prismatic`], without effect on a [`JointType::Fixed`]
    /// joint.
    ///
    /// A plain input the solver only reads: nothing clears it between steps, so a value
    /// written once keeps being applied on every subsequent step until you overwrite it.
    /// There is also no damping or friction term anywhere in this module — with τ = 0 a
    /// free joint coasts indefinitely.
    pub joint_force: f32, // Motor veya dış müdahale kuvveti (tau)

    // --- Geçici ABA değişkenleri (Pass 1-3 sırasında hesaplanırlar) ---
    /// Spatial velocity of this link expressed in **its own** frame: angular half rad/s,
    /// linear half m/s. Built in the first pass as the parent's velocity carried down
    /// through the joint plus the joint's own contribution `S·q_dot`.
    #[serde(skip)] pub v: SpatialVector,     // Spatial velocity
    /// Spatial acceleration of this link in its own frame, produced by the last pass.
    ///
    /// Not a world-frame acceleration: on the fixed-base path gravity is injected as a
    /// fictitious `-gravity` acceleration at the root, so what propagates down is
    /// acceleration relative to free fall: a link whose true acceleration is exactly
    /// `gravity` reads about zero here. `q_ddot` is unaffected by that offset and is the
    /// true joint acceleration.
    #[serde(skip)] pub a: SpatialVector,     // Spatial acceleration (Coriolis/bias)
    /// Velocity-product (Coriolis/centrifugal) bias acceleration `c = v × (S·q_dot)`, in
    /// the link frame. Exactly zero whenever this joint's `q_dot` is zero, whatever the
    /// rest of the tree is doing.
    #[serde(skip)] pub c: SpatialVector,     // Velocity product acceleration
    /// Articulated-body inertia `I^A`: the effective 6×6 inertia this link presents to
    /// its parent once its entire subtree has been folded in.
    ///
    /// Re-seeded from `inertia` at the start of every solve and then accumulated into by
    /// each child during the backward pass, so it only means anything once
    /// [`aba::compute_aba`] has returned — mid-solve it is a partial sum.
    #[serde(skip)] pub i_a: SpatialMatrix,   // Articulated Body Inertia
    /// Articulated bias force `p^A` in the link frame (angular half N·m, linear half N):
    /// the force this link would need in order to have zero acceleration — gyroscopic
    /// terms plus whatever the subtree pushes back. Accumulated alongside `i_a` and just
    /// as partial until the solve finishes.
    #[serde(skip)] pub p_a: SpatialVector,   // Bias force
    /// Motion subspace `S` — the joint axis as a 6-vector in the link's own frame: unit
    /// length for a movable joint with a usable axis, `SpatialVector::ZERO` for a
    /// [`JointType::Fixed`] (or zero-axis) joint, which is how the solver detects a
    /// locked DOF. Capitalised because it is Featherstone's symbol, which is what the
    /// module-level `allow(non_snake_case)` is for.
    #[serde(skip)] pub S: SpatialVector,     // Motion subspace (Joint axis in spatial coords)
    /// Residual joint effort `u = τ − Sᵀ·p^A`, in N·m or N: the applied effort minus the
    /// part already accounted for by the bias force. It is the leading term of the `q̈`
    /// numerator, from which the parent's acceleration contribution `Uᵀa'` is then
    /// subtracted.
    #[serde(skip)] pub u: f32,               // tau - S^T * p_a
    /// Articulated joint inertia `D = Sᵀ·I^A·S` — the scalar inertia felt at this single
    /// DOF, kg·m² for a hinge and kg for a slider.
    ///
    /// Divisor of the `q̈` solve, guarded by an **absolute** `D > 1e-6` test rather than
    /// a relative one: at or below that the joint is treated as locked (`q̈ = 0`, full
    /// inertia passed up to the parent), so a genuinely featherweight joint is caught by
    /// the same branch as a `Fixed` one.
    #[serde(skip)] pub d_val: f32,               // S^T * i_a * S
    /// `U = I^A·S`, the articulated inertia applied to the joint axis. It feeds three
    /// terms per link: the rank-1 `U D⁻¹ Uᵀ` projected out of `I^A` before it is handed to
    /// the parent, the `U D⁻¹ u` term added to the bias force passed up alongside it, and
    /// the `Uᵀa'` subtraction in the `q̈` numerator.
    #[serde(skip)] pub u_vec: SpatialVector,     // i_a * S
}

impl ArticulatedLink {
    /// Builds this link's child→parent [`SpatialTransform`] at the current `q`: the
    /// constant structural placement (`rotation_to_parent`, `transform_to_parent`)
    /// composed with the joint's own displacement.
    ///
    /// Pure and uncached — the solver rebuilds it in each of its passes rather than
    /// storing it, so the value always reflects the `q` at the moment of the call. The
    /// joint axis is normalised on the way through, meaning its length never scales the
    /// displacement and a zero-length axis produces none at all (identity rotation, zero
    /// translation), leaving only the structural part.
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

    /// The joint's motion subspace `S`: the one 6-D direction this joint is free to move
    /// along, expressed in the link's own frame. A hinge fills the angular half
    /// (`(axis, 0)`), a slider the linear half (`(0, axis)`), and a
    /// [`JointType::Fixed`] joint returns `SpatialVector::ZERO`.
    ///
    /// The axis is normalised here, and that is load-bearing rather than tidiness: the
    /// joint inertia `D` is quadratic in `S` while `q̈` is solved as a ratio built from
    /// it, so an unnormalised axis would rescale `q̈` and `q` would no longer read in
    /// radians or metres. A zero-length axis consequently yields `ZERO`, which the
    /// solver interprets as a locked joint — see [`JointType::Fixed`].
    pub fn compute_motion_subspace(&self) -> SpatialVector {
        // Normalize: normalize edilmemiş eksen S'i ölçekler → q̈ yanlış ölçeklenir.
        match self.joint_type {
            JointType::Fixed => SpatialVector::ZERO,
            JointType::Revolute(axis) => SpatialVector::new(axis.normalize_or_zero(), Vec3::ZERO),
            JointType::Prismatic(axis) => SpatialVector::new(Vec3::ZERO, axis.normalize_or_zero()),
        }
    }
}

/// A kinematic tree — one base plus a topologically ordered list of links — and the
/// entire input/output surface of the experimental ABA solver.
///
/// It is a plain value you own, with the limitations listed in the module docs: you call
/// [`system::step_articulated_trees`] (or [`aba::compute_aba`] plus your own integration)
/// yourself. `gizmo-net`'s rollback snapshot likewise stores per-entity rigid-body state
/// only, so an `ArticulatedTree` is neither captured nor restored by a rollback —
/// serialise it yourself if you need to rewind it.
///
/// The solver works entirely in coordinates relative to the base; see `base_position`
/// for what that means for placing the tree in a world.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArticulatedTree {
    /// The links, ordered so that every parent precedes its children (see
    /// [`ArticulatedLink::parent_index`]), with `links[0]` the root by convention.
    ///
    /// An empty list is legal and makes a solve a no-op. More than one link may declare
    /// `usize::MAX` as its parent, in which case each of them attaches directly to the
    /// base and the value is really a forest — stepped as one unit, with no coupling
    /// between the separate roots beyond the shared base.
    pub links: Vec<ArticulatedLink>, // links[0] her zaman ROOT'tur.
    
    // Kök gövdenin (Base) uzaydaki konumu
    /// Where the base frame sits in the world, in metres — bookkeeping only.
    ///
    /// Neither [`aba::compute_aba`] nor [`system::step_articulated_trees`] reads or
    /// writes it: every link transform is computed relative to the base, so moving this
    /// does not move the simulation, and a floating base does not update it either. It
    /// is here so a caller can keep the tree's placement with the tree and map link
    /// frames out into world space.
    pub base_position: Vec3,
    /// Orientation of the base frame in the world. Carried but unread, like
    /// `base_position` — and note in particular that the `gravity` vector handed to the
    /// solver is interpreted in **base** coordinates and is not rotated by this, so a
    /// tilted base needs its gravity pre-rotated by the caller.
    pub base_rotation: Quat,
    /// Spatial velocity of the base expressed in the base frame (angular rad/s, linear
    /// m/s). Read **only** when `is_fixed_base` is `false`, where it seeds the root
    /// link's velocity; on the fixed-base path it is ignored entirely.
    pub base_velocity: SpatialVector, // (w, v)
    /// Spatial acceleration of the base in the base frame (angular rad/s², linear m/s²),
    /// again read only when `is_fixed_base` is `false`.
    ///
    /// A pure input on that path: the solver never writes it back, and
    /// [`system::step_articulated_trees`] does not integrate it into `base_velocity`.
    /// It is *superposed on* the gravity term, not a substitute for it — the fictitious
    /// `-gravity` root acceleration is applied on both branches — so leaving this at zero
    /// gives a floating-base tree the same joint accelerations a fixed-base one would have.
    /// Set it to `(0, +gravity)`, i.e. a base in free fall, to get a weightless tree.
    ///
    /// Through 0.9.0 it *replaced* gravity: a floating-base tree left at the default zero was
    /// weightless whatever the `gravity` argument said, and one given a non-zero base
    /// acceleration got the response with gravity omitted entirely rather than superposed.
    pub base_acceleration: SpatialVector, // (w, v)
    /// `true` (and the `Default`) welds the root to the world: the base pose, velocity
    /// and acceleration fields are all ignored, and gravity enters as a fictitious
    /// `-gravity` acceleration at the root. This is the only fully implemented mode.
    ///
    /// `false` selects the unfinished floating-base path. `base_velocity` and
    /// `base_acceleration` are then consumed as given, and gravity still applies, but
    /// nothing propagates the resulting base acceleration back out — so the base itself
    /// never translates or rotates. See `base_acceleration` and the module docs.
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
