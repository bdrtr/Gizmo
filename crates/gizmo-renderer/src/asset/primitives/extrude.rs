//! Turning a flat outline into a solid: extrusion, and the sweep around an axis.
//!
//! # Why one module and not fifteen constructors
//!
//! `docs/CAPABILITY_GAPS.md` §A4 counted a 26-shape catalogue against the engine's six ready-made
//! builders and found that **15 of the 20 missing ones were one absent capability** — eight
//! extrusions and seven ring extrusions. A prism, a pentagonal prism, a rounded slab and a torus of
//! square section are not four features; they are four *outlines* handed to one machine. So what is
//! here is the machine ([`extrude`], [`sweep`]) plus the outlines ([`outline`]), and a new shape is
//! a function returning `Vec<Vec2>` rather than another `create_*`.
//!
//! # The caps are the hard half
//!
//! Walls are trivial: every edge of the outline becomes a quad. The **caps** need the outline
//! triangulated, and a triangle fan — the obvious answer, and what the demo's hand-rolled pentagon
//! used — is only correct for a *convex* outline. A star, an L-shape or an arrow fanned from one
//! vertex produces triangles outside the shape. [`triangulate`] is ear clipping, so a concave
//! outline comes out right; what it does not do is holes, and it says so rather than producing
//! something plausible.
//!
//! # What it does not do
//!
//! No holes (an annulus is a sweep here, not an extrusion), no bevels or rounded edges, no
//! self-intersecting outlines — ear clipping assumes a *simple* polygon and a figure-of-eight is
//! not one. Normals are flat: each wall quad and each cap carries its own face normal, matching
//! `create_cube` and every other builder in this directory.

use gizmo_math::{Vec2, Vec3};

use crate::components::Mesh;
use crate::renderer::Vertex;

/// Closed outlines in the XY plane, wound **counter-clockwise**.
///
/// Counter-clockwise is not a preference: [`extrude`] gives the front cap a `+Z` normal and relies
/// on the winding to match, and the whole engine draws with `FrontFace::Ccw` plus back-face
/// culling. An outline wound the other way extrudes into a shape that is invisible from outside —
/// not a parse error, which is the worse kind of failure. [`extrude`] therefore *checks* rather
/// than assumes; see its docs.
pub mod outline {
    use super::Vec2;
    use std::f32::consts::TAU;

    /// A circle of `radius`, approximated with `segments` sides.
    ///
    /// Also the general regular-polygon case: `circle(r, 5)` is a pentagon and `circle(r, 3)` a
    /// triangle. There is no separate `regular_polygon` because there is no separate shape — the
    /// segment count is the only difference, and two names for it would drift.
    #[must_use]
    pub fn circle(radius: f32, segments: u32) -> Vec<Vec2> {
        let segments = segments.max(3);
        (0..segments)
            .map(|i| {
                let a = i as f32 / segments as f32 * TAU;
                Vec2::new(radius * a.cos(), radius * a.sin())
            })
            .collect()
    }

    /// An ellipse with the given semi-axes.
    #[must_use]
    pub fn ellipse(rx: f32, ry: f32, segments: u32) -> Vec<Vec2> {
        let segments = segments.max(3);
        (0..segments)
            .map(|i| {
                let a = i as f32 / segments as f32 * TAU;
                Vec2::new(rx * a.cos(), ry * a.sin())
            })
            .collect()
    }

    /// An axis-aligned rectangle centred on the origin.
    #[must_use]
    pub fn rectangle(width: f32, height: f32) -> Vec<Vec2> {
        let (w, h) = (width * 0.5, height * 0.5);
        vec![
            Vec2::new(-w, -h),
            Vec2::new(w, -h),
            Vec2::new(w, h),
            Vec2::new(-w, h),
        ]
    }

    /// A stadium: a `length`-long rectangle capped with semicircles of `radius`, along X.
    ///
    /// The 2-D capsule, and the outline a rounded slab or a lozenge extrudes from.
    #[must_use]
    pub fn stadium(radius: f32, length: f32, segments: u32) -> Vec<Vec2> {
        let segments = segments.max(2);
        let half = length * 0.5;
        let mut points = Vec::with_capacity((segments as usize + 1) * 2);
        // Right cap, from −90° up to +90°, then the left one continuing the turn: one sweep, so
        // the winding cannot disagree between the halves.
        for i in 0..=segments {
            let a = -std::f32::consts::FRAC_PI_2 + i as f32 / segments as f32 * std::f32::consts::PI;
            points.push(Vec2::new(half + radius * a.cos(), radius * a.sin()));
        }
        for i in 0..=segments {
            let a = std::f32::consts::FRAC_PI_2 + i as f32 / segments as f32 * std::f32::consts::PI;
            points.push(Vec2::new(-half + radius * a.cos(), radius * a.sin()));
        }
        points
    }

    /// A `points`-pointed star, alternating between `outer` and `inner` radius.
    ///
    /// Here because it is the cheapest **concave** outline: it is what shows the caps are ear
    /// clipped rather than fanned, and a fan produces visible triangles across its notches.
    #[must_use]
    pub fn star(outer: f32, inner: f32, points: u32) -> Vec<Vec2> {
        let points = points.max(3);
        (0..points * 2)
            .map(|i| {
                let a = i as f32 / (points * 2) as f32 * TAU;
                let r = if i % 2 == 0 { outer } else { inner };
                Vec2::new(r * a.cos(), r * a.sin())
            })
            .collect()
    }
}

/// Twice the signed area of a closed outline. Positive means counter-clockwise.
///
/// The shoelace formula, and it answers two questions at once: which way the outline is wound, and
/// whether it has any area at all. Both matter before triangulating one.
#[must_use]
pub fn signed_area2(outline: &[Vec2]) -> f32 {
    let n = outline.len();
    if n < 3 {
        return 0.0;
    }
    let mut sum = 0.0;
    for i in 0..n {
        let a = outline[i];
        let b = outline[(i + 1) % n];
        sum += a.x * b.y - b.x * a.y;
    }
    sum
}

/// Triangulates a **simple** closed outline into index triples, by ear clipping.
///
/// Returns `None` for anything it cannot honestly triangulate: fewer than three points, zero area,
/// or an outline it cannot reduce — which is what a self-intersecting one looks like from in here.
/// Returning nothing beats returning triangles that are not the shape.
///
/// The result always has `n - 2` triangles, and every one of them is wound counter-clockwise
/// regardless of how the input was wound: a clockwise outline is reversed first, so the caller does
/// not have to normalise and the two cap orientations in [`extrude`] stay a property of the
/// extruder rather than of the caller.
///
/// Ear clipping is `O(n²)`. For the outline sizes a shape primitive uses — tens of points, not
/// thousands — that is not the interesting number; correctness on a concave outline is.
#[must_use]
pub fn triangulate(outline: &[Vec2]) -> Option<Vec<[u32; 3]>> {
    let n = outline.len();
    if n < 3 {
        return None;
    }
    let area2 = signed_area2(outline);
    if area2.abs() < 1e-12 {
        return None;
    }
    // Work on indices into the ORIGINAL outline, so the caller's vertex order survives; the
    // reversal below only changes the order they are consumed in.
    let mut remaining: Vec<u32> = if area2 > 0.0 {
        (0..n as u32).collect()
    } else {
        (0..n as u32).rev().collect()
    };

    let mut out = Vec::with_capacity(n - 2);
    // Every successful clip removes one point, so the loop is bounded by `n`. `guard` bounds the
    // *unsuccessful* passes: without it a self-intersecting outline, which has no ear anywhere,
    // spins forever inside a mesh builder.
    let mut guard = 0;
    while remaining.len() > 3 {
        let count = remaining.len();
        let mut clipped = false;
        for i in 0..count {
            let (ia, ib, ic) = (
                remaining[(i + count - 1) % count],
                remaining[i],
                remaining[(i + 1) % count],
            );
            let (a, b, c) = (outline[ia as usize], outline[ib as usize], outline[ic as usize]);
            // Convex corner? A reflex one cannot be an ear.
            if cross(b - a, c - b) <= 0.0 {
                continue;
            }
            // …and no other point of the outline inside the candidate triangle.
            let blocked = remaining.iter().any(|&j| {
                j != ia && j != ib && j != ic && point_in_triangle(outline[j as usize], a, b, c)
            });
            if blocked {
                continue;
            }
            out.push([ia, ib, ic]);
            remaining.remove(i);
            clipped = true;
            break;
        }
        if !clipped {
            return None;
        }
        guard += 1;
        if guard > n {
            return None;
        }
    }
    out.push([remaining[0], remaining[1], remaining[2]]);
    Some(out)
}

fn cross(a: Vec2, b: Vec2) -> f32 {
    a.x * b.y - a.y * b.x
}

/// Inclusive of the edges, so a point exactly on a candidate ear's boundary blocks the clip.
/// Excluding it is what produces slivers and, on a shape with three collinear points, an ear that
/// covers a fourth.
fn point_in_triangle(p: Vec2, a: Vec2, b: Vec2, c: Vec2) -> bool {
    let d1 = cross(b - a, p - a);
    let d2 = cross(c - b, p - b);
    let d3 = cross(a - c, p - c);
    (d1 >= 0.0 && d2 >= 0.0 && d3 >= 0.0) || (d1 <= 0.0 && d2 <= 0.0 && d3 <= 0.0)
}

fn vertex(position: Vec3, normal: Vec3, uv: Vec2) -> Vertex {
    Vertex {
        position: position.to_array(),
        color: [1.0, 1.0, 1.0, 1.0],
        normal: normal.to_array(),
        tex_coords: uv.to_array(),
        joint_indices: [0; 4],
        joint_weights: [0.0; 4],
        ..Default::default()
    }
}

/// Extrudes a closed outline along **Z**, centred on the origin: two caps and a wall.
///
/// `None` when the outline cannot be triangulated — see [`triangulate`] for what that covers.
///
/// The caps are the outline at `±depth/2`, front facing `+Z` and back facing `−Z`; the wall is one
/// quad per edge, each with its own face normal. Cap UVs are the outline's own XY mapped onto its
/// bounding box, so a texture lands on the face undistorted only if the shape is square — which is
/// the honest default for an arbitrary outline. Wall UVs run `u` along the perimeter by arc length
/// (so a texture does not stretch over a long edge and bunch on a short one) and `v` across the
/// depth.
#[must_use]
pub fn extrude_data(outline: &[Vec2], depth: f32) -> Option<Vec<Vertex>> {
    let triangles = triangulate(outline)?;
    let half = depth * 0.5;

    // Cap UVs: normalise against the outline's bounds. A degenerate axis (a flat outline) would
    // divide by zero, and `triangulate` has already refused those, but the max() costs nothing and
    // makes that independent of the other function's behaviour.
    let (min, max) = outline.iter().fold(
        (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)),
        |(lo, hi), p| (lo.min(*p), hi.max(*p)),
    );
    let extent = (max - min).max(Vec2::splat(f32::MIN_POSITIVE));
    let uv_of = |p: Vec2| (p - min) / extent;

    let mut vertices = Vec::with_capacity(triangles.len() * 6 + outline.len() * 6);
    for [ia, ib, ic] in &triangles {
        let (a, b, c) = (
            outline[*ia as usize],
            outline[*ib as usize],
            outline[*ic as usize],
        );
        // Front cap: the triangulation's own winding, seen from +Z, is counter-clockwise.
        for p in [a, b, c] {
            vertices.push(vertex(p.extend(half), Vec3::Z, uv_of(p)));
        }
        // Back cap: reversed, so it is counter-clockwise seen from −Z.
        for p in [c, b, a] {
            vertices.push(vertex(p.extend(-half), Vec3::NEG_Z, Vec2::new(1.0 - uv_of(p).x, uv_of(p).y)));
        }
    }

    // The wall. `u` is arc length so the texture keeps its aspect around the perimeter.
    let n = outline.len();
    let perimeter: f32 = (0..n)
        .map(|i| (outline[(i + 1) % n] - outline[i]).length())
        .sum();
    let mut travelled = 0.0;
    for i in 0..n {
        let p0 = outline[i];
        let p1 = outline[(i + 1) % n];
        let edge = p1 - p0;
        let len = edge.length();
        // A zero-length edge (a repeated point) has no direction to build a normal from. Skipping
        // it drops nothing: the quad would have had no area.
        if len <= f32::EPSILON {
            continue;
        }
        // Outward normal of a counter-clockwise outline is the edge rotated −90°.
        let normal = Vec3::new(edge.y, -edge.x, 0.0).normalize();
        let (u0, u1) = (travelled / perimeter, (travelled + len) / perimeter);
        travelled += len;

        let (a, b) = (p0.extend(half), p1.extend(half));
        let (c, d) = (p1.extend(-half), p0.extend(-half));
        for (p, uv) in [
            (a, Vec2::new(u0, 0.0)),
            (d, Vec2::new(u0, 1.0)),
            (c, Vec2::new(u1, 1.0)),
            (a, Vec2::new(u0, 0.0)),
            (c, Vec2::new(u1, 1.0)),
            (b, Vec2::new(u1, 0.0)),
        ] {
            vertices.push(vertex(p, normal, uv));
        }
    }
    Some(vertices)
}

/// Sweeps a closed outline around the **Y** axis at `radius`: the ring extrusion.
///
/// The outline is read as a *profile* in the XY plane — its `x` becomes distance from the axis and
/// its `y` stays vertical — and is carried around `segments` steps of a full turn. A circular
/// profile gives a torus; a rectangle gives a square-section ring; a stadium gives a rounded one.
/// That is the other seven entries of §A4's fifteen.
///
/// There are no caps: a swept ring is closed by construction. The profile does not need
/// triangulating for the same reason, so unlike [`extrude_data`] this accepts any closed outline,
/// concave or not, and returns nothing only for one with fewer than three points.
#[must_use]
pub fn sweep_data(profile: &[Vec2], radius: f32, segments: u32) -> Option<Vec<Vertex>> {
    let n = profile.len();
    if n < 3 {
        return None;
    }
    let segments = segments.max(3);
    // A clockwise profile sweeps into a ring that is inside-out. Normalising here rather than
    // asking the caller keeps `outline`'s generators usable for both machines.
    let profile: Vec<Vec2> = if signed_area2(profile) > 0.0 {
        profile.to_vec()
    } else {
        profile.iter().rev().copied().collect()
    };

    let ring_point = |p: Vec2, a: f32| Vec3::new((radius + p.x) * a.cos(), p.y, (radius + p.x) * a.sin());
    let mut vertices = Vec::with_capacity(segments as usize * n * 6);
    for s in 0..segments {
        let a0 = s as f32 / segments as f32 * std::f32::consts::TAU;
        let a1 = (s + 1) as f32 / segments as f32 * std::f32::consts::TAU;
        let (v0, v1) = (s as f32 / segments as f32, (s + 1) as f32 / segments as f32);
        for i in 0..n {
            let p0 = profile[i];
            let p1 = profile[(i + 1) % n];
            let (u0, u1) = (i as f32 / n as f32, (i + 1) as f32 / n as f32);

            let q00 = ring_point(p0, a0);
            let q10 = ring_point(p1, a0);
            let q01 = ring_point(p0, a1);
            let q11 = ring_point(p1, a1);
            // Face normal from the quad itself: the profile may be concave, so a normal derived
            // from the profile's own outward direction would be wrong exactly where it matters.
            let normal = (q10 - q00).cross(q01 - q00);
            if normal.length_squared() <= f32::EPSILON {
                continue;
            }
            let normal = normal.normalize();
            for (p, uv) in [
                (q00, Vec2::new(u0, v0)),
                (q10, Vec2::new(u1, v0)),
                (q01, Vec2::new(u0, v1)),
                (q01, Vec2::new(u0, v1)),
                (q10, Vec2::new(u1, v0)),
                (q11, Vec2::new(u1, v1)),
            ] {
                vertices.push(vertex(p, normal, uv));
            }
        }
    }
    Some(vertices)
}

impl crate::asset::AssetManager {
    /// Extrudes a closed 2-D outline along Z — see [`extrude_data`].
    ///
    /// `None` for an outline that cannot be triangulated, rather than an empty or wrong mesh.
    #[must_use]
    pub fn create_extrusion(device: &wgpu::Device, outline: &[Vec2], depth: f32) -> Option<Mesh> {
        let vertices = extrude_data(outline, depth)?;
        Some(Mesh::new_indexed(device, &vertices, Vec3::ZERO, format!("extrusion_{}", outline.len())))
    }

    /// Sweeps a closed 2-D profile around the Y axis — see [`sweep_data`].
    #[must_use]
    pub fn create_sweep(
        device: &wgpu::Device,
        profile: &[Vec2],
        radius: f32,
        segments: u32,
    ) -> Option<Mesh> {
        let vertices = sweep_data(profile, radius, segments)?;
        Some(Mesh::new_indexed(
            device,
            &vertices,
            Vec3::ZERO,
            format!("sweep_{}_{segments}", profile.len()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shoelace area of the triangulation must equal the outline's own.
    ///
    /// This is the assertion that catches everything an eye would: a fan across a concave notch
    /// adds area outside the shape, a dropped ear subtracts it, and a triangle wound the wrong way
    /// subtracts twice. Equality to a tolerance is a much stronger statement than "n − 2 triangles
    /// came back", which a wrong triangulation also satisfies.
    fn assert_triangulation_covers(name: &str, outline: &[Vec2]) {
        let tris = triangulate(outline).unwrap_or_else(|| panic!("{name}: refused to triangulate"));
        assert_eq!(tris.len(), outline.len() - 2, "{name}: wrong triangle count");
        let total: f32 = tris
            .iter()
            .map(|[a, b, c]| {
                let (a, b, c) = (
                    outline[*a as usize],
                    outline[*b as usize],
                    outline[*c as usize],
                );
                cross(b - a, c - a) * 0.5
            })
            .sum();
        let expected = signed_area2(outline).abs() * 0.5;
        assert!(
            (total - expected).abs() < expected * 1e-3,
            "{name}: triangles cover {total}, the outline is {expected}",
        );
        // Every triangle counter-clockwise, whatever the input's winding was.
        for [a, b, c] in &tris {
            let (a, b, c) = (
                outline[*a as usize],
                outline[*b as usize],
                outline[*c as usize],
            );
            assert!(cross(b - a, c - a) > 0.0, "{name}: a triangle came back clockwise");
        }
    }

    #[test]
    fn convex_outlines_triangulate_to_their_own_area() {
        assert_triangulation_covers("rectangle", &outline::rectangle(2.0, 1.0));
        assert_triangulation_covers("triangle", &outline::circle(1.0, 3));
        assert_triangulation_covers("pentagon", &outline::circle(1.0, 5));
        assert_triangulation_covers("circle-32", &outline::circle(1.0, 32));
        assert_triangulation_covers("ellipse", &outline::ellipse(2.0, 0.5, 24));
        assert_triangulation_covers("stadium", &outline::stadium(0.4, 1.2, 8));
    }

    /// The concave case — the whole reason this is ear clipping and not a fan.
    ///
    /// A five-pointed star fanned from vertex 0 covers its notches; the area check above is what
    /// says so, and it fails by roughly the notch area rather than by a rounding error.
    #[test]
    fn a_concave_outline_triangulates_to_its_own_area() {
        assert_triangulation_covers("star-5", &outline::star(1.0, 0.4, 5));
        assert_triangulation_covers("star-8-deep", &outline::star(1.0, 0.15, 8));
        // An L, written out: the shape a fan gets most visibly wrong.
        let l = [
            Vec2::new(0.0, 0.0),
            Vec2::new(2.0, 0.0),
            Vec2::new(2.0, 0.6),
            Vec2::new(0.6, 0.6),
            Vec2::new(0.6, 2.0),
            Vec2::new(0.0, 2.0),
        ];
        assert_triangulation_covers("L", &l);
    }

    /// A clockwise outline triangulates the same way, and comes back counter-clockwise.
    #[test]
    fn winding_is_normalised_rather_than_demanded() {
        let ccw = outline::star(1.0, 0.4, 5);
        let cw: Vec<Vec2> = ccw.iter().rev().copied().collect();
        assert!(signed_area2(&ccw) > 0.0 && signed_area2(&cw) < 0.0, "the fixture is wrong");
        assert_triangulation_covers("star reversed", &cw);
    }

    /// What it refuses, and refuses rather than guessing.
    #[test]
    fn degenerate_outlines_produce_nothing() {
        assert!(triangulate(&[]).is_none());
        assert!(triangulate(&[Vec2::ZERO, Vec2::X]).is_none(), "two points are not a polygon");
        // Three collinear points: a real outline in the sense of having three vertices, and no
        // area, so nothing can be built from it.
        assert!(
            triangulate(&[Vec2::ZERO, Vec2::X, Vec2::new(2.0, 0.0)]).is_none(),
            "a zero-area outline was triangulated anyway",
        );
        // A figure-of-eight: simple-polygon algorithms have no answer for it, and the bound in
        // `triangulate` is what keeps that from being an infinite loop in a mesh builder.
        let bowtie = [
            Vec2::new(-1.0, -1.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(1.0, -1.0),
            Vec2::new(-1.0, 1.0),
        ];
        assert!(triangulate(&bowtie).is_none(), "a self-intersecting outline was accepted");
    }

    /// An extrusion is two caps and a wall, and the counts say which is which.
    #[test]
    fn an_extrusion_has_two_caps_and_one_quad_per_edge() {
        let outline = outline::circle(1.0, 6);
        let v = extrude_data(&outline, 2.0).expect("a hexagon extrudes");
        // 2 caps × (n−2) triangles × 3, plus n quads × 6.
        assert_eq!(v.len(), 2 * (6 - 2) * 3 + 6 * 6);
        // Both cap planes are present, at ±depth/2 and nowhere else.
        let front = v.iter().filter(|x| (x.position[2] - 1.0).abs() < 1e-5).count();
        let back = v.iter().filter(|x| (x.position[2] + 1.0).abs() < 1e-5).count();
        assert_eq!(front + back, v.len(), "a vertex landed off both cap planes");
        assert_eq!(front, back, "the two caps and the wall are not symmetric in z");
    }

    /// Depth scales the solid along z and leaves the outline alone.
    #[test]
    fn depth_only_changes_the_z_extent() {
        let outline = outline::rectangle(2.0, 1.0);
        let thin = extrude_data(&outline, 0.5).expect("extrudes");
        let thick = extrude_data(&outline, 4.0).expect("extrudes");
        let z_extent = |v: &[Vertex]| {
            let hi = v.iter().map(|x| x.position[2]).fold(f32::MIN, f32::max);
            let lo = v.iter().map(|x| x.position[2]).fold(f32::MAX, f32::min);
            hi - lo
        };
        let x_extent = |v: &[Vertex]| {
            let hi = v.iter().map(|x| x.position[0]).fold(f32::MIN, f32::max);
            let lo = v.iter().map(|x| x.position[0]).fold(f32::MAX, f32::min);
            hi - lo
        };
        assert!((z_extent(&thin) - 0.5).abs() < 1e-5);
        assert!((z_extent(&thick) - 4.0).abs() < 1e-5);
        assert!((x_extent(&thin) - x_extent(&thick)).abs() < 1e-5, "depth changed the outline");
    }

    /// A sweep of a circular profile is a torus, and its bounds say so.
    #[test]
    fn a_swept_circle_is_a_torus() {
        let profile = outline::circle(0.25, 12);
        let v = sweep_data(&profile, 1.0, 24).expect("sweeps");
        assert_eq!(v.len(), 24 * 12 * 6, "one quad per profile edge per segment");
        let radial = |x: &Vertex| (x.position[0].powi(2) + x.position[2].powi(2)).sqrt();
        let hi = v.iter().map(radial).fold(f32::MIN, f32::max);
        let lo = v.iter().map(radial).fold(f32::MAX, f32::min);
        assert!((hi - 1.25).abs() < 0.01, "outer radius {hi}, expected 1.25");
        assert!((lo - 0.75).abs() < 0.01, "inner radius {lo}, expected 0.75");
        // And it is a ring, not a disc: nothing near the axis.
        assert!(lo > 0.5, "the sweep filled its own hole");
    }

    /// A sweep needs no triangulation, so a concave profile is fine — unlike an extrusion's cap.
    #[test]
    fn a_concave_profile_sweeps() {
        let v = sweep_data(&outline::star(0.4, 0.15, 5), 1.2, 16).expect("a star profile sweeps");
        assert_eq!(v.len(), 16 * 10 * 6);
    }

    #[test]
    fn a_profile_with_too_few_points_sweeps_to_nothing() {
        assert!(sweep_data(&[Vec2::ZERO, Vec2::X], 1.0, 8).is_none());
    }
}
