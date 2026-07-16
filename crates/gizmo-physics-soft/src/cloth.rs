use gizmo_math::Vec3;
use gizmo_physics_core::{BodyHandle, Collider, ColliderShape, Transform};

/// If `pos` is inside `shape` (within `thickness` of its surface), returns the nearest
/// surface point (pushed out by `thickness`) and the outward normal. Handles spheres and
/// boxes (the common draping targets); other shapes are skipped. Position-based (static)
/// so it catches RESTING nodes draped on a surface, unlike a swept raycast.
fn project_point_out(
    pos: Vec3,
    thickness: f32,
    shape: &ColliderShape,
    trans: &Transform,
) -> Option<(Vec3, Vec3)> {
    match shape {
        ColliderShape::Sphere(s) => {
            let d = pos - trans.position;
            let len = d.length();
            let min = s.radius + thickness;
            if len < min {
                let n = if len > 1e-6 { d / len } else { Vec3::Y };
                Some((trans.position + n * min, n))
            } else {
                None
            }
        }
        ColliderShape::Box(b) => {
            // Into the box's local frame; if inside, push out along the least-penetrated face.
            let local = trans.rotation.inverse() * (pos - trans.position);
            let he = b.half_extents + Vec3::splat(thickness);
            if local.x.abs() < he.x && local.y.abs() < he.y && local.z.abs() < he.z {
                let pen = he - local.abs();
                let (new_local, n_local) = if pen.x <= pen.y && pen.x <= pen.z {
                    let s = if local.x >= 0.0 { 1.0 } else { -1.0 };
                    (Vec3::new(s * he.x, local.y, local.z), Vec3::new(s, 0.0, 0.0))
                } else if pen.y <= pen.z {
                    let s = if local.y >= 0.0 { 1.0 } else { -1.0 };
                    (Vec3::new(local.x, s * he.y, local.z), Vec3::new(0.0, s, 0.0))
                } else {
                    let s = if local.z >= 0.0 { 1.0 } else { -1.0 };
                    (Vec3::new(local.x, local.y, s * he.z), Vec3::new(0.0, 0.0, s))
                };
                Some((
                    trans.position + trans.rotation * new_local,
                    trans.rotation * n_local,
                ))
            } else {
                None
            }
        }
        ColliderShape::Capsule(c) => {
            // Capsule axis is local Y, running from -half_height to +half_height with
            // hemispherical caps of `radius`. Closest axis point = clamp local.y, then it
            // reduces to a sphere test at that point (covers cylinder body AND both caps).
            let local = trans.rotation.inverse() * (pos - trans.position);
            let axis_pt = Vec3::new(0.0, local.y.clamp(-c.half_height, c.half_height), 0.0);
            let d = local - axis_pt;
            let len = d.length();
            let min = c.radius + thickness;
            if len < min {
                let n_local = if len > 1e-6 { d / len } else { Vec3::X };
                let new_local = axis_pt + n_local * min;
                Some((
                    trans.position + trans.rotation * new_local,
                    trans.rotation * n_local,
                ))
            } else {
                None
            }
        }
        _ => None, // plane / trimesh / convex-hull / compound: not yet handled
    }
}

/// Bounding-sphere radius of a shape `project_point_out` understands, for broad-phase
/// culling of cloth edges. `None` for shapes the edge pass cannot resolve (so it skips them).
fn collider_bound(shape: &ColliderShape) -> Option<f32> {
    match shape {
        ColliderShape::Sphere(s) => Some(s.radius),
        ColliderShape::Box(b) => Some(b.half_extents.length()),
        ColliderShape::Capsule(c) => Some(c.half_height + c.radius),
        _ => None,
    }
}

/// Squared distance from point `p` to the segment `[a, b]`.
fn seg_point_dist_sq(a: Vec3, b: Vec3, p: Vec3) -> f32 {
    let ab = b - a;
    let len2 = ab.length_squared();
    let t = if len2 > 1e-12 {
        ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (a + ab * t - p).length_squared()
}

/// A single particle (mass point) in the cloth simulation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClothNode {
    /// Current world-space position.
    pub position: Vec3,
    /// Position at the previous sub-step (used for implicit velocity in XPBD).
    pub prev_position: Vec3,
    /// Mass of this node; `0.0` means the node is pinned (immovable).
    pub mass: f32,
    /// Inverse mass (`1/mass`); `0.0` for pinned nodes.
    pub inv_mass: f32,
}

/// A distance (stretch/bend/shear) constraint between two cloth nodes, solved with XPBD.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[non_exhaustive]
pub struct DistanceConstraint {
    pub node_a: usize,
    pub node_b: usize,
    pub rest_length: f32,
    pub compliance: f32, // Inverse stiffness
    pub lambda: f32,     // Accumulated XPBD multiplier
}

/// A cloth sheet simulated with Extended Position Based Dynamics (XPBD).
///
/// Built as a regular grid of [`ClothNode`]s linked by structural, bend and shear
/// [`DistanceConstraint`]s. Step the simulation with [`Cloth::step`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Cloth {
    /// All particles of the cloth, laid out row-major (`idx = y * width + x`).
    pub nodes: Vec<ClothNode>,
    /// Distance constraints connecting the nodes.
    pub constraints: Vec<DistanceConstraint>,
    /// Collision thickness against the floor plane (`y = thickness`).
    pub thickness: f32,
    /// Horizontal friction applied on floor contact, in `[0, 1]`.
    pub friction: f32,
    /// Tearing: a constraint whose length exceeds `rest_length * tear_factor` is removed
    /// (the cloth rips there). `f32::INFINITY` (the default) disables tearing.
    pub tear_factor: f32,
}

impl Cloth {
    /// Builds a `width` x `height` grid of nodes spaced `spacing` units apart in the
    /// XY plane, each with mass `mass_per_node` (`0.0` makes a node pinned).
    pub fn new(width: usize, height: usize, spacing: f32, mass_per_node: f32) -> Self {
        let mut nodes = Vec::with_capacity(width * height);
        let mut constraints = Vec::new();

        for y in 0..height {
            for x in 0..width {
                let position = Vec3::new(x as f32 * spacing, y as f32 * spacing, 0.0);
                nodes.push(ClothNode {
                    position,
                    prev_position: position,
                    mass: mass_per_node,
                    inv_mass: if mass_per_node > 0.0 {
                        1.0 / mass_per_node
                    } else {
                        0.0
                    },
                });

                let idx = y * width + x;

                // Structural constraints
                if x > 0 {
                    constraints.push(DistanceConstraint {
                        node_a: idx,
                        node_b: idx - 1,
                        rest_length: spacing,
                        compliance: 0.001,
                        lambda: 0.0,
                    });
                }
                if y > 0 {
                    constraints.push(DistanceConstraint {
                        node_a: idx,
                        node_b: idx - width,
                        rest_length: spacing,
                        compliance: 0.001,
                        lambda: 0.0,
                    });
                }

                // Bend constraints
                if x > 1 {
                    constraints.push(DistanceConstraint {
                        node_a: idx,
                        node_b: idx - 2,
                        rest_length: spacing * 2.0,
                        compliance: 0.1,
                        lambda: 0.0,
                    });
                }
                if y > 1 {
                    constraints.push(DistanceConstraint {
                        node_a: idx,
                        node_b: idx - width * 2,
                        rest_length: spacing * 2.0,
                        compliance: 0.1,
                        lambda: 0.0,
                    });
                }

                // Shear constraints
                if x > 0 && y > 0 {
                    let diag = spacing * std::f32::consts::SQRT_2;
                    constraints.push(DistanceConstraint {
                        node_a: idx,
                        node_b: idx - width - 1,
                        rest_length: diag,
                        compliance: 0.005,
                        lambda: 0.0,
                    });
                    constraints.push(DistanceConstraint {
                        node_a: idx - 1,
                        node_b: idx - width,
                        rest_length: diag,
                        compliance: 0.005,
                        lambda: 0.0,
                    });
                }
            }
        }

        Self {
            nodes,
            constraints,
            thickness: 0.02,
            friction: 0.5,
            tear_factor: f32::INFINITY,
        }
    }

    /// Pins the node at `idx` so it becomes immovable (no-op if out of range).
    pub fn pin_node(&mut self, idx: usize) {
        if idx < self.nodes.len() {
            self.nodes[idx].inv_mass = 0.0;
            self.nodes[idx].mass = 0.0;
        }
    }

    /// Advances the cloth by one XPBD timestep of length `dt` (seconds), applying
    /// `gravity` (units/s²), dividing the step into `sub_steps` solver iterations, and
    /// resolving collisions against `colliders` (spheres/boxes — the cloth drapes over
    /// them) in addition to the floor plane. Pass `&[]` for floor-only behaviour.
    pub fn step(
        &mut self,
        dt: f32,
        gravity: Vec3,
        sub_steps: usize,
        colliders: &[(BodyHandle, Transform, Collider)],
    ) {
        let sub_dt = dt / (sub_steps as f32);
        let sub_dt2 = sub_dt * sub_dt;

        // Per-node collision scratch, reused across sub-steps: the pre-collision impact
        // velocity and (if the node touches a collider) the contact normal. Kept out of
        // `ClothNode` so the public particle struct — and its `PartialEq`/determinism —
        // stays clean. Only populated when there are rigid colliders to resolve against.
        let mut impact_vel: Vec<Vec3> = Vec::new();
        let mut contact_normal: Vec<Option<Vec3>> = Vec::new();
        // Edge-pass accumulation: corrections are summed per node and applied as an AVERAGE,
        // so a node shared by many edges is not shoved once per incident edge (compounding).
        let mut edge_push: Vec<Vec3> = Vec::new();
        let mut edge_cnt: Vec<u32> = Vec::new();
        if !colliders.is_empty() {
            impact_vel = vec![Vec3::ZERO; self.nodes.len()];
            contact_normal = vec![None; self.nodes.len()];
            edge_push = vec![Vec3::ZERO; self.nodes.len()];
            edge_cnt = vec![0u32; self.nodes.len()];
        }

        for _ in 0..sub_steps {
            for c in &mut self.constraints {
                c.lambda = 0.0;
            }

            // Predict
            for node in &mut self.nodes {
                if node.inv_mass == 0.0 {
                    continue;
                }
                let velocity = (node.position - node.prev_position) / sub_dt;
                node.prev_position = node.position;

                // Add gravity and damping (frame-rate independent)
                let damping = 0.99f32;
                let next_vel = velocity * damping.powf(sub_dt) + gravity * sub_dt;
                node.position += next_vel * sub_dt;
            }

            // Solve Constraints
            for constraint in &mut self.constraints {
                let (pos_a, pos_b, inv_m_a, inv_m_b) = {
                    let a = &self.nodes[constraint.node_a];
                    let b = &self.nodes[constraint.node_b];
                    (a.position, b.position, a.inv_mass, b.inv_mass)
                };

                let w_sum = inv_m_a + inv_m_b;
                if w_sum == 0.0 {
                    continue;
                }

                let diff = pos_a - pos_b;
                let dist = diff.length();
                if dist < 1e-6 {
                    continue;
                }

                let n = diff / dist;
                let c = dist - constraint.rest_length;
                let alpha = constraint.compliance / sub_dt2;

                let delta_lambda = (-c - alpha * constraint.lambda) / (w_sum + alpha);
                constraint.lambda += delta_lambda;

                let p = n * delta_lambda;

                self.nodes[constraint.node_a].position += p * inv_m_a;
                self.nodes[constraint.node_b].position -= p * inv_m_b;
            }

            // Rigid-body collision — drape the cloth over every sphere/box collider.
            //
            // Structured as THREE passes so each node gets ONE velocity response per step
            // (applying friction per-edge/per-collider would damp a node 6-10x and stall the
            // cloth). Passes only correct POSITION; the reconstructed impact velocity is
            // captured up front and the response is applied once at the end.
            if !colliders.is_empty() {
                let (thickness, friction) = (self.thickness, self.friction);

                // Impact velocity (post-predict, post-constraint) BEFORE any collision push,
                // and the contact normal for each node that ends up touching something.
                for ((node, cn), iv) in self
                    .nodes
                    .iter()
                    .zip(contact_normal.iter_mut())
                    .zip(impact_vel.iter_mut())
                {
                    *cn = None;
                    *iv = if node.inv_mass != 0.0 {
                        (node.position - node.prev_position) / sub_dt
                    } else {
                        Vec3::ZERO
                    };
                }

                // Pass 1 — node contact: project each dynamic node out of each collider.
                for (node, cn) in self.nodes.iter_mut().zip(contact_normal.iter_mut()) {
                    if node.inv_mass == 0.0 {
                        continue;
                    }
                    for (_, ctrans, col) in colliders {
                        if let Some((new_pos, n)) =
                            project_point_out(node.position, thickness, &col.shape, ctrans)
                        {
                            node.position = new_pos;
                            *cn = Some(n);
                        }
                    }
                }

                // Pass 2 — edge contact: a collider can fit BETWEEN sparse nodes, so a
                // low-resolution cloth (or a coarse "board") would tunnel through if only the
                // nodes were tested. For each edge we find the point ALONG the segment that
                // penetrates deepest (sampling several parameters `t`, not just the midpoint —
                // an off-centre/tilted edge is deepest nearest the collider, not at its middle)
                // and push the two endpoints out with barycentric weights so that contact point
                // reaches the surface.
                //
                // Gate: skip the edge if EITHER endpoint is already at/near this collider's
                // surface — the node pass owns that region and pushing the edge too would
                // inflate the drape off the surface. The gate uses a slightly widened band
                // (`2*thickness`) so a node RESTING exactly at `radius+thickness` still counts
                // as contact (`project_point_out` uses a strict `<`, so the exact-surface node
                // would otherwise leak through). Corrections are accumulated and applied as a
                // per-node AVERAGE, and an endpoint is never pushed INTO the collider (which,
                // for an edge straddling a box, the shared contact normal would otherwise do).
                for i in 0..edge_push.len() {
                    edge_push[i] = Vec3::ZERO;
                    edge_cnt[i] = 0;
                }
                const SAMPLES: usize = 8;
                for ci in 0..self.constraints.len() {
                    let a_idx = self.constraints[ci].node_a;
                    let b_idx = self.constraints[ci].node_b;
                    let (ma, mb) = (self.nodes[a_idx].inv_mass, self.nodes[b_idx].inv_mass);
                    if ma == 0.0 && mb == 0.0 {
                        continue;
                    }
                    for (_, ctrans, col) in colliders {
                        let Some(bound) = collider_bound(&col.shape) else {
                            continue;
                        };
                        // Re-read per collider: an earlier collider may have moved the nodes.
                        let pa = self.nodes[a_idx].position;
                        let pb = self.nodes[b_idx].position;
                        // Broad phase: skip unless the segment comes within reach of the collider.
                        let reach = bound + thickness;
                        if seg_point_dist_sq(pa, pb, ctrans.position) > reach * reach {
                            continue;
                        }
                        // Gate: endpoint already resolved by the node pass → skip this edge.
                        let gate = thickness * 2.0;
                        if project_point_out(pa, gate, &col.shape, ctrans).is_some()
                            || project_point_out(pb, gate, &col.shape, ctrans).is_some()
                        {
                            continue;
                        }
                        // Deepest-penetrating interior sample point.
                        let mut best: Option<(f32, Vec3, Vec3, f32)> = None; // (t, corr, n, pen²)
                        for k in 1..SAMPLES {
                            let t = k as f32 / SAMPLES as f32;
                            let pt = pa + (pb - pa) * t;
                            if let Some((new_pt, n)) =
                                project_point_out(pt, thickness, &col.shape, ctrans)
                            {
                                let pen = (new_pt - pt).length_squared();
                                if best.is_none_or(|(_, _, _, bp)| pen > bp) {
                                    best = Some((t, new_pt - pt, n, pen));
                                }
                            }
                        }
                        let Some((t, corr, n, _)) = best else {
                            continue;
                        };
                        // Barycentric split so the contact point at `t` moves out by `corr`.
                        // Never drive an endpoint INTO the collider (which, for an edge
                        // straddling a box, the shared contact normal would otherwise do).
                        let (ua, ub) = (1.0 - t, t);
                        let denom = (ua * ua + ub * ub).max(1e-6);
                        if ma != 0.0 {
                            let d = corr * (ua / denom);
                            if project_point_out(pa + d, thickness, &col.shape, ctrans).is_none() {
                                edge_push[a_idx] += d;
                                edge_cnt[a_idx] += 1;
                                contact_normal[a_idx] = Some(n);
                            }
                        }
                        if mb != 0.0 {
                            let d = corr * (ub / denom);
                            if project_point_out(pb + d, thickness, &col.shape, ctrans).is_none() {
                                edge_push[b_idx] += d;
                                edge_cnt[b_idx] += 1;
                                contact_normal[b_idx] = Some(n);
                            }
                        }
                    }
                }
                // Apply the averaged accumulated edge corrections.
                for (node, (push, cnt)) in self
                    .nodes
                    .iter_mut()
                    .zip(edge_push.iter().zip(edge_cnt.iter()))
                {
                    if *cnt > 0 {
                        node.position += *push / *cnt as f32;
                    }
                }

                // Pass 3 — velocity response, ONCE per contacted node: remove the inward normal
                // component of the pre-collision impact velocity (inelastic) and apply friction
                // to the tangential remainder, then rebuild prev_position from the new velocity.
                for ((node, cn), iv) in self
                    .nodes
                    .iter_mut()
                    .zip(contact_normal.iter())
                    .zip(impact_vel.iter())
                {
                    if node.inv_mass == 0.0 {
                        continue;
                    }
                    if let Some(n) = *cn {
                        let vel = *iv;
                        let vn = vel.dot(n);
                        let new_vel = if vn < 0.0 {
                            (vel - n * vn) * (1.0 - friction)
                        } else {
                            vel
                        };
                        node.prev_position = node.position - new_vel * sub_dt;
                    }
                }
            }

            // Floor Collision
            for node in &mut self.nodes {
                if node.inv_mass == 0.0 {
                    continue;
                }
                if node.position.y < self.thickness {
                    // Capture the true predicted impact velocity BEFORE clamping the
                    // position: computing it from the clamped position would understate
                    // the vertical impact speed, corrupting friction/`prev_position`
                    // reconstruction and injecting a wrong impulse next frame.
                    let mut vel = (node.position - node.prev_position) / sub_dt;

                    node.position.y = self.thickness;

                    // Simple friction: damp horizontal velocity when touching ground.
                    vel.x *= 1.0 - self.friction;
                    vel.z *= 1.0 - self.friction;
                    // Aşağı yönlü hızı koru-MA: zemine doğru momentum biriktirmek
                    // titreme/enerji enjeksiyonuna yol açıyordu (yukarı serbest).
                    vel.y = vel.y.max(0.0);
                    node.prev_position = node.position - vel * sub_dt;
                }
            }
        }

        // Tearing: drop any constraint stretched beyond rest_length * tear_factor so the
        // cloth rips under stress. Disjoint field borrows (nodes read, constraints mutated).
        if self.tear_factor.is_finite() {
            let nodes = &self.nodes;
            let tf = self.tear_factor;
            self.constraints.retain(|c| {
                (nodes[c.node_a].position - nodes[c.node_b].position).length() <= c.rest_length * tf
            });
        }
    }
}

impl gizmo_core::Component for Cloth {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Zemine çarpan bir düğümün çarpma hızı, pozisyon zemine KENETLENMEDEN ÖNCE
    /// hesaplanmalı. Kenetlenmiş pozisyondan hesaplanırsa (bug) düğüm zaten zeminin
    /// altındayken sahte bir YUKARI yönlü dikey hız üretilir: kenetlenmiş y (thickness)
    /// önceki (daha da alçak) y'den büyük olduğu için `(clamped.y - prev.y)/dt > 0`.
    /// Bu, `prev_position`'ı yanlış kurar ve bir sonraki karede yanlış (yukarı) impuls
    /// enjekte eder.
    ///
    /// Kurulum: düğümü zaten zeminin ALTINA yerleştir (prev ve tahmini konum ikisi de
    /// thickness'ın altında, aşağı yönlü hareket). Doğru davranışta yakalanan gerçek
    /// hız aşağı yönlü (< 0) olup `vel.y.max(0.0)` ile SIFIRLANIR → yeniden kurulan
    /// prev_position.y == thickness. Buggy davranışta ise pozitif bir dikey hız kalır
    /// → prev_position.y < thickness olur.
    #[test]
    fn floor_collision_uses_pre_clamp_velocity() {
        let sub_dt = 1.0 / 60.0;
        let mut cloth = Cloth::new(1, 1, 1.0, 1.0);
        cloth.thickness = 0.02;
        cloth.friction = 0.0; // Yalnızca dikey davranışı izole et.

        // Düğüm zaten zeminin altında ve aşağı doğru hareket ediyor.
        // Predict adımının (yerçekimsiz, sönümlü) düğümü yine zeminin altında
        // bırakacağı bir konfigürasyon seç.
        cloth.nodes[0].prev_position = Vec3::new(0.0, -0.5, 0.0);
        cloth.nodes[0].position = Vec3::new(0.0, -0.6, 0.0);

        cloth.step(1.0 / 60.0, Vec3::ZERO, 1, &[]);

        let node = cloth.nodes[0];
        assert!(
            (node.position.y - cloth.thickness).abs() < 1e-6,
            "pozisyon zemine kenetlenmeli"
        );
        // Doğru fix ile: gerçek (aşağı yönlü) hız yakalanıp sıfırlandığı için
        // prev_position.y tam olarak thickness olur. Buggy kodda kenetlenmiş
        // konumdan hesaplanan pozitif dikey hız prev_position.y'yi thickness'ın
        // ALTINA çeker.
        assert!(
            (node.prev_position.y - cloth.thickness).abs() < 1e-6,
            "prev_position.y kenetlenmiş y'ye eşit olmalı (sahte yukarı hız yok): {}",
            node.prev_position.y
        );
        let reconstructed_vel = (node.position - node.prev_position) / sub_dt;
        assert!(reconstructed_vel.is_finite());
        assert!(
            reconstructed_vel.y.abs() < 1e-6,
            "dikey hız sıfırlanmalı (sahte yukarı impuls yok): {}",
            reconstructed_vel.y
        );
    }

    /// A horizontal cloth sheet dropped onto a sphere must DRAPE over it — no node may
    /// penetrate the sphere, and the centre of the sheet should rest on top of it.
    #[test]
    fn cloth_drapes_over_sphere_without_penetrating() {
        let center = Vec3::new(0.0, 0.0, 0.0);
        let colliders = vec![(
            BodyHandle::from_id(1),
            Transform::new(center),
            Collider::sphere(1.0),
        )];

        // 7x7 sheet laid horizontally (XZ), centred 2 units above the sphere.
        let n = 7usize;
        let sp = 0.35;
        let mut cloth = Cloth::new(n, n, sp, 1.0);
        let half = (n as f32 - 1.0) * sp * 0.5;
        for (i, node) in cloth.nodes.iter_mut().enumerate() {
            let x = (i % n) as f32 * sp - half;
            let z = (i / n) as f32 * sp - half;
            node.position = Vec3::new(x, 2.0, z);
            node.prev_position = node.position;
        }

        for _ in 0..400 {
            cloth.step(1.0 / 60.0, Vec3::new(0.0, -9.81, 0.0), 10, &colliders);
        }

        let mut min_dist = f32::INFINITY;
        for node in &cloth.nodes {
            assert!(node.position.is_finite(), "cloth node went non-finite");
            min_dist = min_dist.min((node.position - center).length());
        }
        // No node penetrates the sphere surface (radius 1 + thickness 0.02 ≈ 1.02).
        assert!(min_dist >= 1.0 - 0.02, "cloth penetrated the sphere, min dist={min_dist}");

        let c = cloth.nodes[(n * n) / 2].position; // centre node
        assert!(c.y > 0.6, "cloth centre should drape on top of the sphere, y={}", c.y);
        // The centre must HUG the surface (~radius+thickness), NOT be inflated off it. A
        // too-loose bound here would hide the edge-pass over-damping/inflation regression
        // (the edge pass wrongly firing on every fine-cloth chord and pushing nodes outward).
        assert!(
            ((c - center).length() - 1.02).abs() < 0.06,
            "cloth centre should hug the sphere surface (inflated?), dist={}",
            (c - center).length()
        );
        // And it must SETTLE — over-damping stall or jitter would leave residual speed.
        let sub_dt = (1.0 / 60.0) / 10.0;
        let max_speed = cloth
            .nodes
            .iter()
            .map(|nd| ((nd.position - nd.prev_position) / sub_dt).length())
            .fold(0.0_f32, f32::max);
        assert!(max_speed < 0.5, "fine drape never settled, max node speed={max_speed}");
    }

    /// A COARSE cloth (nodes farther apart than the sphere) must not tunnel straight
    /// through it — the edge-midpoint collision catches the sphere between the nodes.
    #[test]
    fn coarse_cloth_does_not_tunnel_small_sphere() {
        let center = Vec3::new(0.0, 0.0, 0.0);
        let colliders = vec![(
            BodyHandle::from_id(1),
            Transform::new(center),
            Collider::sphere(0.6),
        )];
        // 2x2 sheet (a single quad), 3 units wide → nodes at ±1.5, far outside the r=0.6
        // sphere, but the sheet's centre passes right over it.
        let mut cloth = Cloth::new(2, 2, 3.0, 1.0);
        for (i, node) in cloth.nodes.iter_mut().enumerate() {
            let x = (i % 2) as f32 * 3.0 - 1.5;
            let z = (i / 2) as f32 * 3.0 - 1.5;
            node.position = Vec3::new(x, 2.0, z);
            node.prev_position = node.position;
        }
        let dt = 1.0 / 60.0;
        for _ in 0..300 {
            cloth.step(dt, Vec3::new(0.0, -9.81, 0.0), 10, &colliders);
        }
        // The sheet's centre (avg of the 4 nodes) must stay ON/above the sphere, not below it.
        let c = cloth.nodes.iter().fold(Vec3::ZERO, |a, n| a + n.position) / 4.0;
        assert!(c.y > 0.0, "coarse cloth tunnelled through the sphere, centre y={}", c.y);

        // …and it must SETTLE, not jitter: after this many steps the fastest node's
        // speed should have decayed. A velocity-preserving edge contact (the earlier bug)
        // kept bouncing the board and this speed stayed large. Run further and check.
        for _ in 0..400 {
            cloth.step(dt, Vec3::new(0.0, -9.81, 0.0), 10, &colliders);
        }
        let max_speed = cloth
            .nodes
            .iter()
            .map(|n| ((n.position - n.prev_position) / (dt / 10.0)).length())
            .fold(0.0_f32, f32::max);
        assert!(
            max_speed < 1.0,
            "coarse cloth never settled on the sphere (jitter), max node speed={max_speed}"
        );
        let c2 = cloth.nodes.iter().fold(Vec3::ZERO, |a, n| a + n.position) / 4.0;
        assert!(c2.y > 0.0, "coarse cloth slid off the sphere after settling, centre y={}", c2.y);
    }

    /// A cloth sheet must drape over a CAPSULE (axis-aligned along Y) without penetrating —
    /// the capsule was previously an unhandled shape (cloth fell straight through).
    #[test]
    fn cloth_drapes_over_capsule() {
        let center = Vec3::new(0.0, 0.0, 0.0);
        // Capsule radius 0.6, half-height 1.0 → total half-length 1.6 along Y.
        let colliders = vec![(
            BodyHandle::from_id(1),
            Transform::new(center),
            Collider::capsule(0.6, 1.0),
        )];
        let n = 9usize;
        let sp = 0.3;
        let mut cloth = Cloth::new(n, n, sp, 1.0);
        let half = (n as f32 - 1.0) * sp * 0.5;
        for (i, node) in cloth.nodes.iter_mut().enumerate() {
            // Lay the sheet across the TOP cap (y = +1.6) so it drapes over the dome.
            let x = (i % n) as f32 * sp - half;
            let z = (i / n) as f32 * sp - half;
            node.position = Vec3::new(x, 3.2, z);
            node.prev_position = node.position;
        }
        for _ in 0..500 {
            cloth.step(1.0 / 60.0, Vec3::new(0.0, -9.81, 0.0), 10, &colliders);
        }
        // No node may sit inside the capsule (distance to axis-segment < radius).
        for node in &cloth.nodes {
            assert!(node.position.is_finite(), "cloth node went non-finite on capsule");
            let ay = node.position.y.clamp(-1.0, 1.0);
            let axis_pt = Vec3::new(0.0, ay, 0.0);
            let d = (node.position - axis_pt).length();
            assert!(d >= 0.6 - 0.03, "cloth penetrated the capsule, dist-to-axis={d}");
        }
    }

    /// An OFF-CENTRE small collider sitting on an edge line but nearer ONE endpoint than the
    /// midpoint must still be caught — the old midpoint-only probe tunnelled here (the deepest
    /// segment point is not the midpoint). The multi-sample edge probe closes this gap.
    #[test]
    fn edge_catches_offcentre_collider_between_nodes() {
        // Sphere r=0.4 on the bottom-edge line (z=-1.5), at x=1.0 → 0.5 from node1 (x=1.5),
        // 2.5 from node0 (x=-1.5), 1.0 from the edge midpoint — all farther than r+thickness,
        // yet the segment's nearest point (x=1.0) sits at the sphere centre.
        let sphere_c = Vec3::new(1.0, 0.0, -1.5);
        let colliders = vec![(
            BodyHandle::from_id(1),
            Transform::new(sphere_c),
            Collider::sphere(0.4),
        )];
        let mut cloth = Cloth::new(2, 2, 3.0, 1.0);
        for (i, node) in cloth.nodes.iter_mut().enumerate() {
            let x = (i % 2) as f32 * 3.0 - 1.5;
            let z = (i / 2) as f32 * 3.0 - 1.5;
            node.position = Vec3::new(x, 2.0, z);
            node.prev_position = node.position;
        }
        for _ in 0..400 {
            cloth.step(1.0 / 60.0, Vec3::new(0.0, -9.81, 0.0), 10, &colliders);
        }
        // No point along any cloth EDGE may pass deep through the off-centre sphere.
        let node = |x: usize, z: usize| cloth.nodes[z * 2 + x].position;
        let edges = [
            (node(0, 0), node(1, 0)),
            (node(0, 1), node(1, 1)),
            (node(0, 0), node(0, 1)),
            (node(1, 0), node(1, 1)),
            (node(0, 0), node(1, 1)),
            (node(1, 0), node(0, 1)),
        ];
        let mut min_edge = f32::INFINITY;
        for (a, b) in edges {
            for k in 0..=16 {
                let t = k as f32 / 16.0;
                min_edge = min_edge.min((a + (b - a) * t - sphere_c).length());
            }
        }
        assert!(min_edge.is_finite());
        assert!(
            min_edge >= 0.4 - 0.06,
            "a cloth edge tunnelled the off-centre sphere, min edge dist={min_edge}"
        );
    }

    /// A coarse edge straddling a BOX (endpoints on opposite sides) must not be driven into
    /// the box by the shared contact normal → the sheet must stay finite and settle, never
    /// oscillate. Guards the straddling-box self-penetration jitter.
    #[test]
    fn box_straddle_edge_does_not_self_penetrate() {
        let colliders = vec![(
            BodyHandle::from_id(1),
            Transform::new(Vec3::ZERO),
            Collider::box_collider(Vec3::new(1.0, 1.0, 1.0)),
        )];
        let mut cloth = Cloth::new(2, 2, 4.0, 1.0); // nodes at ±2, straddling the ±1.02 box
        for (i, node) in cloth.nodes.iter_mut().enumerate() {
            let x = (i % 2) as f32 * 4.0 - 2.0;
            let z = (i / 2) as f32 * 4.0 - 2.0;
            node.position = Vec3::new(x, 1.5, z);
            node.prev_position = node.position;
        }
        let dt = 1.0 / 60.0;
        for _ in 0..600 {
            cloth.step(dt, Vec3::new(0.0, -9.81, 0.0), 10, &colliders);
        }
        // No node may be inside the box, and the sheet must settle (no oscillation).
        for node in &cloth.nodes {
            assert!(node.position.is_finite(), "box-straddle went non-finite");
            let p = node.position.abs();
            let inside = p.x < 1.02 && p.y < 1.02 && p.z < 1.02;
            assert!(!inside, "node was driven INSIDE the box: {:?}", node.position);
        }
        let max_speed = cloth
            .nodes
            .iter()
            .map(|n| ((n.position - n.prev_position) / (dt / 10.0)).length())
            .fold(0.0_f32, f32::max);
        assert!(max_speed < 1.0, "box-straddle never settled (jitter), max speed={max_speed}");
    }

    /// A constraint stretched past `tear_factor` is removed — the cloth rips.
    #[test]
    fn cloth_tears_when_overstretched() {
        let mut cloth = Cloth::new(4, 4, 0.5, 1.0);
        let before = cloth.constraints.len();
        cloth.tear_factor = 1.2; // tear at 20% stretch

        // Yank one corner far away and pin it there → its constraints overstretch massively.
        cloth.pin_node(15);
        cloth.nodes[15].position = Vec3::new(10.0, 0.0, 0.0);
        cloth.nodes[15].prev_position = cloth.nodes[15].position;

        cloth.step(1.0 / 60.0, Vec3::ZERO, 1, &[]);

        assert!(
            cloth.constraints.len() < before,
            "overstretched cloth must tear (lose constraints): {before} -> {}",
            cloth.constraints.len()
        );
    }

    // ---------------------------------------------------------------------
    // project_point_out — the static shape-projection math (pure geometry).
    // ---------------------------------------------------------------------

    /// A point INSIDE a sphere is pushed out to exactly `radius + thickness` along the
    /// outward normal; a point OUTSIDE returns `None`.
    #[test]
    fn project_point_out_sphere_pushes_to_surface_and_outside_returns_none() {
        let t = Transform::new(Vec3::new(0.0, 0.0, 0.0));
        let shape = Collider::sphere(1.0).shape;
        let thickness = 0.02;

        // Inside, off-centre along +X.
        let (p, n) = project_point_out(Vec3::new(0.5, 0.0, 0.0), thickness, &shape, &t)
            .expect("point inside the sphere must project out");
        assert!(n.abs_diff_eq(Vec3::X, 1e-5), "outward normal must be +X, got {n:?}");
        assert!(
            ((p - t.position).length() - (1.0 + thickness)).abs() < 1e-5,
            "must land on radius+thickness, got dist={}",
            (p - t.position).length()
        );

        // Outside → no contact.
        assert!(
            project_point_out(Vec3::new(2.0, 0.0, 0.0), thickness, &shape, &t).is_none(),
            "a point outside the sphere must not project"
        );
    }

    /// A point exactly at the sphere CENTRE is degenerate (zero direction); the code must
    /// fall back to the +Y normal instead of producing NaN.
    #[test]
    fn project_point_out_sphere_degenerate_center_uses_y_normal() {
        let t = Transform::new(Vec3::ZERO);
        let shape = Collider::sphere(1.0).shape;
        let (p, n) = project_point_out(Vec3::ZERO, 0.02, &shape, &t)
            .expect("centre point is inside → must project");
        assert!(n.abs_diff_eq(Vec3::Y, 1e-6), "degenerate normal must default to +Y");
        assert!(p.is_finite() && p.abs_diff_eq(Vec3::new(0.0, 1.02, 0.0), 1e-5));
    }

    /// A point inside a box is pushed out along the LEAST-penetrated face (here +X), landing
    /// on `half_extent + thickness` in that axis while the other axes are unchanged.
    #[test]
    fn project_point_out_box_pushes_out_least_penetrated_face() {
        let t = Transform::new(Vec3::ZERO);
        let shape = Collider::box_collider(Vec3::splat(1.0)).shape;
        // (0.9, 0.1, 0.1): closest to the +X face (penetration 0.12 vs 0.92 on y/z).
        let (p, n) = project_point_out(Vec3::new(0.9, 0.1, 0.1), 0.02, &shape, &t)
            .expect("interior point must project");
        assert!(n.abs_diff_eq(Vec3::X, 1e-6), "normal must be the +X face, got {n:?}");
        assert!(
            p.abs_diff_eq(Vec3::new(1.02, 0.1, 0.1), 1e-5),
            "must snap the X coord to half_extent+thickness, keeping y/z, got {p:?}"
        );
        // A point comfortably outside the (padded) box does not project.
        assert!(project_point_out(Vec3::new(3.0, 0.0, 0.0), 0.02, &shape, &t).is_none());
    }

    /// The box projection works in the collider's LOCAL frame: a box rotated 90° about Z
    /// pushes the point out along its rotated +X face (which points along world +Y).
    #[test]
    fn project_point_out_rotated_box_uses_local_frame() {
        use std::f32::consts::FRAC_PI_2;
        let t = Transform::new(Vec3::ZERO).with_rotation(gizmo_math::Quat::from_rotation_z(FRAC_PI_2));
        let shape = Collider::box_collider(Vec3::splat(1.0)).shape;
        let (p, n) = project_point_out(Vec3::new(0.1, 0.9, 0.1), 0.02, &shape, &t)
            .expect("interior point must project");
        // Rotated +X face points to world +Y; surface at world y = 1.02.
        assert!(n.abs_diff_eq(Vec3::Y, 1e-5), "rotated normal must be world +Y, got {n:?}");
        assert!(
            p.abs_diff_eq(Vec3::new(0.1, 1.02, 0.1), 1e-5),
            "rotated box must push along its local +X (world +Y), got {p:?}"
        );
    }

    /// The capsule reduces to a sphere test at the clamped axis point: a point beside the
    /// cylinder body pushes out radially, and a point above the top cap pushes out over the
    /// dome (clamped to +half_height).
    #[test]
    fn project_point_out_capsule_body_and_caps() {
        let t = Transform::new(Vec3::ZERO);
        let shape = Collider::capsule(0.5, 1.0).shape; // radius 0.5, half_height 1.0
        let thickness = 0.02;

        // Beside the cylinder body (y within [-1, 1]).
        let (pb, nb) = project_point_out(Vec3::new(0.2, 0.0, 0.0), thickness, &shape, &t)
            .expect("body point inside must project");
        assert!(nb.abs_diff_eq(Vec3::X, 1e-6));
        assert!(pb.abs_diff_eq(Vec3::new(0.52, 0.0, 0.0), 1e-5), "got {pb:?}");

        // Above the top cap: axis point clamps to y = +1, then pushes up over the dome.
        let (pc, nc) = project_point_out(Vec3::new(0.0, 1.4, 0.0), thickness, &shape, &t)
            .expect("cap point inside must project");
        assert!(nc.abs_diff_eq(Vec3::Y, 1e-6));
        assert!(pc.abs_diff_eq(Vec3::new(0.0, 1.52, 0.0), 1e-5), "got {pc:?}");

        // Well outside the capsule → no contact.
        assert!(project_point_out(Vec3::new(2.0, 0.0, 0.0), thickness, &shape, &t).is_none());
    }

    /// `collider_bound` returns the bounding-sphere radius for the shapes the cloth pass
    /// understands, and `None` for shapes it cannot resolve (plane).
    #[test]
    fn collider_bound_matches_shape_and_none_for_plane() {
        assert_eq!(collider_bound(&Collider::sphere(2.0).shape), Some(2.0));
        let boxb = collider_bound(&Collider::box_collider(Vec3::splat(1.0)).shape).unwrap();
        assert!((boxb - 3.0f32.sqrt()).abs() < 1e-6, "box bound must be |half_extents|");
        assert_eq!(collider_bound(&Collider::capsule(0.5, 1.0).shape), Some(1.5));
        assert_eq!(collider_bound(&Collider::plane(Vec3::Y, 0.0).shape), None);
    }

    /// `seg_point_dist_sq` clamps the projection parameter to `[0, 1]` (so points past an
    /// endpoint measure to that endpoint) and degrades gracefully for a zero-length segment.
    #[test]
    fn seg_point_dist_sq_clamps_and_handles_degenerate() {
        let a = Vec3::new(0.0, 0.0, 0.0);
        let b = Vec3::new(2.0, 0.0, 0.0);
        // Interior projection: closest point is the foot of the perpendicular.
        assert!((seg_point_dist_sq(a, b, Vec3::new(1.0, 1.0, 0.0)) - 1.0).abs() < 1e-6);
        // Past b → clamps to b (distance 3, squared 9), not the infinite-line distance.
        assert!((seg_point_dist_sq(a, b, Vec3::new(5.0, 0.0, 0.0)) - 9.0).abs() < 1e-6);
        // Before a → clamps to a.
        assert!((seg_point_dist_sq(a, b, Vec3::new(-3.0, 0.0, 0.0)) - 9.0).abs() < 1e-6);
        // Degenerate (a == b): distance to the single point, no divide-by-zero.
        let d = seg_point_dist_sq(a, a, Vec3::new(0.0, 4.0, 0.0));
        assert!((d - 16.0).abs() < 1e-6, "degenerate segment: {d}");
    }

    // ---------------------------------------------------------------------
    // Cloth construction & bookkeeping invariants.
    // ---------------------------------------------------------------------

    /// A `w x h` grid must lay out row-major with the right node/constraint counts, and the
    /// three constraint families must carry their expected rest lengths.
    #[test]
    fn cloth_new_builds_expected_grid_and_constraints() {
        let (w, h, sp) = (3usize, 2usize, 0.5f32);
        let cloth = Cloth::new(w, h, sp, 2.0);

        assert_eq!(cloth.nodes.len(), w * h, "node count = width*height");
        // Row-major layout: idx = y*w + x, positions on the XY plane.
        for y in 0..h {
            for x in 0..w {
                let node = cloth.nodes[y * w + x];
                assert!(node.position.abs_diff_eq(Vec3::new(x as f32 * sp, y as f32 * sp, 0.0), 1e-6));
                assert!((node.inv_mass - 0.5).abs() < 1e-6, "inv_mass = 1/mass");
                assert_eq!(node.mass, 2.0);
            }
        }

        // Hand-counted for a 3x2 grid: 7 structural + 2 bend + 4 shear = 13.
        let structural = (w - 1) * h + w * (h - 1);
        let bend = (w - 2) * h; // no vertical bend for h < 3
        let shear = 2 * (w - 1) * (h - 1);
        assert_eq!(structural, 7);
        assert_eq!(bend, 2);
        assert_eq!(shear, 4);
        assert_eq!(cloth.constraints.len(), structural + bend + shear);
        assert_eq!(cloth.constraints.len(), 13);

        // Every constraint's rest length is one of the three expected families.
        let diag = sp * std::f32::consts::SQRT_2;
        for c in &cloth.constraints {
            let ok = (c.rest_length - sp).abs() < 1e-5
                || (c.rest_length - (sp * 2.0)).abs() < 1e-5
                || (c.rest_length - diag).abs() < 1e-5;
            assert!(ok, "unexpected rest length {}", c.rest_length);
            assert_eq!(c.lambda, 0.0, "lambda must start at zero");
        }
    }

    /// `mass_per_node == 0` pins every node (inv_mass 0), so a zero-mass sheet never moves.
    #[test]
    fn cloth_new_zero_mass_pins_all_nodes() {
        let cloth = Cloth::new(4, 4, 0.5, 0.0);
        for node in &cloth.nodes {
            assert_eq!(node.inv_mass, 0.0);
            assert_eq!(node.mass, 0.0);
        }
    }

    /// `pin_node` clears the node's mass/inv_mass; an out-of-range index is a silent no-op
    /// (must not panic or resize the grid).
    #[test]
    fn pin_node_sets_and_out_of_range_is_noop() {
        let mut cloth = Cloth::new(2, 2, 1.0, 1.0);
        cloth.pin_node(1);
        assert_eq!(cloth.nodes[1].inv_mass, 0.0);
        assert_eq!(cloth.nodes[1].mass, 0.0);

        let len = cloth.nodes.len();
        cloth.pin_node(999); // out of range → no-op, no panic
        assert_eq!(cloth.nodes.len(), len);
    }

    /// Freshly built cloth carries the documented default material fields.
    #[test]
    fn cloth_defaults_are_set() {
        let cloth = Cloth::new(2, 2, 1.0, 1.0);
        assert!((cloth.thickness - 0.02).abs() < 1e-6);
        assert!((cloth.friction - 0.5).abs() < 1e-6);
        assert_eq!(cloth.tear_factor, f32::INFINITY);
    }

    // ---------------------------------------------------------------------
    // Integration & constraint dynamics (isolated, deterministic).
    // ---------------------------------------------------------------------

    /// A single free node falls under gravity; a pinned node in the same setup does not move.
    #[test]
    fn free_node_falls_under_gravity_pinned_stays() {
        // Free: 1x1 sheet lifted above the floor, no colliders.
        let mut free = Cloth::new(1, 1, 1.0, 1.0);
        free.nodes[0].position = Vec3::new(0.0, 5.0, 0.0);
        free.nodes[0].prev_position = free.nodes[0].position;
        free.step(0.1, Vec3::new(0.0, -10.0, 0.0), 1, &[]);
        assert!(free.nodes[0].position.y < 5.0, "free node must fall");
        assert!(free.nodes[0].position.is_finite());

        // Pinned: identical setup but pinned → immovable.
        let mut pinned = Cloth::new(1, 1, 1.0, 1.0);
        pinned.nodes[0].position = Vec3::new(0.0, 5.0, 0.0);
        pinned.nodes[0].prev_position = pinned.nodes[0].position;
        pinned.pin_node(0);
        pinned.step(0.1, Vec3::new(0.0, -10.0, 0.0), 1, &[]);
        assert_eq!(pinned.nodes[0].position, Vec3::new(0.0, 5.0, 0.0));
    }

    /// An overstretched structural constraint pulls its two endpoints back toward the rest
    /// length in a single step (XPBD), reducing the separation without overshooting.
    #[test]
    fn stretched_structural_constraint_pulls_nodes_together() {
        let mut cloth = Cloth::new(1, 2, 1.0, 1.0); // one vertical structural link, rest = 1
        assert_eq!(cloth.constraints.len(), 1);
        // Place the pair 2 units apart, well above the floor, with zero velocity.
        cloth.nodes[0].position = Vec3::new(0.0, 5.0, 0.0);
        cloth.nodes[0].prev_position = cloth.nodes[0].position;
        cloth.nodes[1].position = Vec3::new(0.0, 7.0, 0.0);
        cloth.nodes[1].prev_position = cloth.nodes[1].position;

        let before = (cloth.nodes[0].position - cloth.nodes[1].position).length();
        cloth.step(1.0 / 60.0, Vec3::ZERO, 1, &[]);
        let after = (cloth.nodes[0].position - cloth.nodes[1].position).length();

        assert!((before - 2.0).abs() < 1e-6);
        assert!(after < before - 0.1, "constraint must contract the pair: {before} -> {after}");
        assert!(after > 1.0, "must not overshoot past the rest length: {after}");
    }

    /// Stepping two identically-constructed cloths with identical inputs yields bit-identical
    /// state — the solver is deterministic (no RNG, no order-dependent float reduction).
    #[test]
    fn cloth_step_is_deterministic() {
        let mut a = Cloth::new(5, 5, 0.4, 1.0);
        let mut b = Cloth::new(5, 5, 0.4, 1.0);
        a.pin_node(0);
        b.pin_node(0);
        let g = Vec3::new(0.3, -9.81, -0.2);
        for _ in 0..40 {
            a.step(1.0 / 60.0, g, 4, &[]);
            b.step(1.0 / 60.0, g, 4, &[]);
        }
        assert_eq!(a, b, "identical cloths must evolve identically");
    }
}
