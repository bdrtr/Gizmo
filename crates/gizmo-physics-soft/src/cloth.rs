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
}
