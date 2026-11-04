//! What a [`FluidZone`] does to a body: buoyancy and drag, summed over cells of the body.

use super::{FluidZone, ZoneShape};
use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::{Collider, ColliderShape};

/// Cells per axis of the grid a non-spherical body is split into. 3 is the smallest count with
/// a middle row, so a level hull's centre cell sits on its axis and a tipped one has a low side
/// and a high side to push differently.
const CELLS: usize = 3;

/// The fluid's push on one body this step.
pub(crate) struct FluidPush {
    /// Net force, newtons.
    pub force: Vec3,
    /// Net torque about the centre of mass, N·m.
    pub torque: Vec3,
}

impl ZoneShape {
    /// The water surface's height: the top face of a box, the top of a sphere.
    pub(crate) fn surface_y(&self) -> f32 {
        match *self {
            ZoneShape::Box { max, .. } => max.y,
            ZoneShape::Sphere { center, radius } => center.y + radius,
        }
    }

    /// Whether the column of water at `p`'s x/z holds `p` — `p` clamped to just under the
    /// surface first, so a point *above* the water over the pool answers yes (it is then
    /// simply not submerged) and a point beside the pool answers no.
    fn holds_column(&self, p: Vec3, surface_y: f32) -> bool {
        self.contains(Vec3::new(p.x, p.y.min(surface_y - 1e-4), p.z))
    }
}

impl FluidZone {
    /// Buoyancy and drag on a body with `collider` at `position`/`rotation`, moving at `linear`
    /// and `angular`, its centre of mass at `com` (world). `None` when no part of it is in the
    /// water.
    ///
    /// A sphere is one cell: its submerged share is linear in depth, the push acts through its
    /// centre (a ball has nothing to right). Every other shape is a [`CELLS`]³ grid through its
    /// local bounds, each cell displacing `volume / CELLS³` times the share of its own vertical
    /// extent below the surface, pushed and dragged **at the cell** — which is what gives a
    /// tipped hull a righting moment and a spinning one a drag torque. Drag per cell is
    /// `linear_drag·|v| + quadratic_drag·|v|²` against the cell's own velocity, weighted by its
    /// share of the body, so a body moving without turning feels exactly the old drag.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn push_on(
        &self,
        collider: &Collider,
        position: Vec3,
        rotation: Quat,
        com: Vec3,
        linear: Vec3,
        angular: Vec3,
        gravity: Vec3,
    ) -> Option<FluidPush> {
        let surface_y = self.shape.surface_y();
        let volume = collider.volume();
        if volume <= 0.0 {
            return None;
        }

        if let ColliderShape::Sphere(s) = &collider.shape {
            if !self.shape.holds_column(position, surface_y) {
                return None;
            }
            let r = s.radius;
            let share = ((surface_y - (position.y - r)) / (2.0 * r)).clamp(0.0, 1.0);
            if share <= 0.0 {
                return None;
            }
            let force = -gravity * (volume * share * self.density) + self.drag(linear) * share;
            return Some(FluidPush { force, torque: (position - com).cross(force) });
        }

        // The body's local bounds, centred where they are (a compound or a hull need not be
        // centred on its origin).
        let local = collider.compute_aabb(Vec3::ZERO, Quat::IDENTITY);
        let (lo, hi) = (local.min, local.max);
        let extent = Vec3::new(hi.x - lo.x, hi.y - lo.y, hi.z - lo.z);
        if extent.y <= 0.0 {
            return None; // a plane displaces nothing
        }
        let cell = extent / CELLS as f32;
        // Half the vertical size of a turned cell: its half extents projected on world Y.
        let half = cell * 0.5;
        let (ax, ay, az) = (rotation * Vec3::X, rotation * Vec3::Y, rotation * Vec3::Z);
        let cell_half_y = ax.y.abs() * half.x + ay.y.abs() * half.y + az.y.abs() * half.z;
        let cell_volume = volume / (CELLS * CELLS * CELLS) as f32;
        let weight = 1.0 / (CELLS * CELLS * CELLS) as f32;

        let mut force = Vec3::ZERO;
        let mut torque = Vec3::ZERO;
        let mut any = false;
        for i in 0..CELLS {
            for j in 0..CELLS {
                for k in 0..CELLS {
                    let local_p = Vec3::new(
                        lo.x + cell.x * (i as f32 + 0.5),
                        lo.y + cell.y * (j as f32 + 0.5),
                        lo.z + cell.z * (k as f32 + 0.5),
                    );
                    let p = position + rotation * local_p;
                    if !self.shape.holds_column(p, surface_y) {
                        continue;
                    }
                    let share = if cell_half_y > 0.0 {
                        ((surface_y - (p.y - cell_half_y)) / (2.0 * cell_half_y)).clamp(0.0, 1.0)
                    } else if p.y < surface_y {
                        1.0
                    } else {
                        0.0
                    };
                    if share <= 0.0 {
                        continue;
                    }
                    any = true;
                    let v = linear + angular.cross(p - com);
                    let f = -gravity * (cell_volume * share * self.density)
                        + self.drag(v) * (share * weight);
                    force += f;
                    torque += (p - com).cross(f);
                }
            }
        }
        any.then_some(FluidPush { force, torque })
    }

    /// The drag force on the whole body moving at `v`, before the submerged share.
    fn drag(&self, v: Vec3) -> Vec3 {
        let speed = v.length();
        if speed <= 1e-4 {
            return Vec3::ZERO;
        }
        -(v / speed) * (self.linear_drag * speed + self.quadratic_drag * speed * speed)
    }
}
