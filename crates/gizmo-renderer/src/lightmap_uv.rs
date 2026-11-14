//! Lightmap UVs made for a mesh: [`unwrap`] lays its triangles out flat, without overlap, in the
//! second UV set ([`Vertex::lightmap_uv`]) a lightmap is read at and baked into
//! ([`crate::lightmap_bake`]).
//!
//! What it does, in order:
//!
//! 1. **Charts.** Triangles that share an edge (corners matched by position) and face within
//!    [`UnwrapSettings::max_angle`] of the chart's first triangle grow into one chart — a cube's
//!    faces are six charts, a sphere's a handful of caps and bands.
//! 2. **Flattening.** Each chart is projected along its first triangle's normal. Every triangle in
//!    it faces within the angle of that direction, so none is turned over; a chart that still lies
//!    over itself (a surface that curls back under the projection) is split again at half the angle,
//!    down to single triangles, which cannot.
//! 3. **Turning.** Each chart is turned to the smallest rectangle around it (rotating calipers over
//!    its convex hull), which wastes the least of the atlas.
//! 4. **Packing.** Charts go into rows, tallest first, [`UnwrapSettings::padding`] texels apart and
//!    off the edges at the resolution the lightmap will be baked at; the scale — one for the whole
//!    mesh, so a texel is the same size everywhere on it — is the largest that fits.
//!
//! A chart lands on whole texels, so the bake's padding around each island
//! ([`crate::lightmap_bake::BakeSettings::padding`]) has room and bilinear filtering never mixes two
//! charts. Unwrap for the resolution the lightmap will have.

use gizmo_math::{Vec2, Vec3};

use crate::gpu_types::Vertex;

/// How [`unwrap`] cuts and packs.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct UnwrapSettings {
    /// The lightmap's size in texels, square — the one the mesh will be baked at. Default 128.
    pub resolution: u32,
    /// Texels kept empty around every chart. Default 2: what the bake fills (its own padding) plus
    /// one so bilinear filtering of one chart never reaches the next.
    pub padding: u32,
    /// The most a triangle may turn from its chart's first one and stay in it, in radians. Lower
    /// cuts more charts and stretches less. Default 66° (1.15).
    pub max_angle: f32,
}

impl Default for UnwrapSettings {
    fn default() -> Self {
        Self { resolution: 128, padding: 2, max_angle: 1.15 }
    }
}

impl UnwrapSettings {
    /// These settings for a `resolution × resolution` lightmap.
    #[must_use]
    pub fn with_resolution(mut self, resolution: u32) -> Self {
        self.resolution = resolution;
        self
    }

    /// These settings with `padding` texels around every chart.
    #[must_use]
    pub fn with_padding(mut self, padding: u32) -> Self {
        self.padding = padding;
        self
    }

    /// These settings cutting charts at `radians`.
    #[must_use]
    pub fn with_max_angle(mut self, radians: f32) -> Self {
        self.max_angle = radians;
        self
    }
}

/// What [`unwrap`] made.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct Unwrapped {
    /// How many charts the mesh was cut into.
    pub charts: usize,
    /// Texels per metre (of the mesh's own space) at the settings' resolution.
    pub texels_per_unit: f32,
}

/// Why [`unwrap`] could not lay a mesh out.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum UnwrapError {
    /// The vertices are not a triangle list with at least one triangle of some area.
    NoTriangles,
    /// The charts and their padding do not fit at this resolution even a texel each — more charts
    /// than the lightmap has room for.
    DoesNotFit {
        /// How many charts there were.
        charts: usize,
    },
    /// A resolution of zero, or a padding that leaves no texel inside it.
    BadSettings,
}

impl std::fmt::Display for UnwrapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoTriangles => write!(f, "the mesh has no triangle with any area to lay out"),
            Self::DoesNotFit { charts } => {
                write!(f, "{charts} charts and their padding do not fit in the lightmap: raise the resolution or lower the padding")
            }
            Self::BadSettings => write!(f, "the resolution leaves no texel inside the padding"),
        }
    }
}

impl std::error::Error for UnwrapError {}

/// Writes a second UV set into `vertices`, a flat counter-clockwise triangle list: charts that do not
/// overlap, packed into `[0, 1]²` for a lightmap of `settings.resolution`. See the [module](self).
/// Degenerate triangles are given a point inside the first chart.
pub fn unwrap(vertices: &mut [Vertex], settings: &UnwrapSettings) -> Result<Unwrapped, UnwrapError> {
    let res = settings.resolution;
    if res == 0 || 2 * settings.padding + 1 > res {
        return Err(UnwrapError::BadSettings);
    }
    let tris = vertices.len() / 3;
    let pos = |i: usize| Vec3::from(vertices[i].position);
    let normals: Vec<Option<Vec3>> =
        (0..tris).map(|t| (pos(3 * t + 1) - pos(3 * t)).cross(pos(3 * t + 2) - pos(3 * t)).try_normalize()).collect();
    if normals.iter().all(Option::is_none) {
        return Err(UnwrapError::NoTriangles);
    }

    // Corners welded by exact position, then edges to the triangles on them.
    let mut corner_id = std::collections::HashMap::new();
    let ids: Vec<u32> = (0..tris * 3)
        .map(|i| {
            let key = vertices[i].position.map(f32::to_bits);
            let next = corner_id.len() as u32;
            *corner_id.entry(key).or_insert(next)
        })
        .collect();
    let mut edges: std::collections::HashMap<(u32, u32), Vec<u32>> = std::collections::HashMap::new();
    for t in 0..tris {
        if normals[t].is_none() {
            continue;
        }
        for k in 0..3 {
            let (a, b) = (ids[3 * t + k], ids[3 * t + (k + 1) % 3]);
            edges.entry((a.min(b), a.max(b))).or_default().push(t as u32);
        }
    }

    // Charts: grown from each unclaimed triangle in order; folded ones are cut again.
    let mut chart_of = vec![u32::MAX; tris];
    let mut charts: Vec<Chart> = Vec::new();
    for seed in 0..tris {
        if chart_of[seed] != u32::MAX || normals[seed].is_none() {
            continue;
        }
        let members = grow(seed, settings.max_angle.cos(), &normals, &ids, &edges, &mut chart_of, u32::MAX, charts.len() as u32);
        let mut pending = vec![(members, settings.max_angle)];
        while let Some((members, angle)) = pending.pop() {
            let chart = flatten(&members, vertices);
            if members.len() == 1 || !chart.overlaps() {
                let index = charts.len() as u32;
                for &t in &members {
                    chart_of[t as usize] = index;
                }
                charts.push(chart);
                continue;
            }
            // Cut again at half the angle, among these triangles only.
            let half = angle * 0.5;
            let mark = u32::MAX - 1;
            for &t in &members {
                chart_of[t as usize] = mark;
            }
            let mut subs = Vec::new();
            for &t in &members {
                if chart_of[t as usize] == mark {
                    subs.push(grow(t as usize, half.cos(), &normals, &ids, &edges, &mut chart_of, mark, u32::MAX - 2));
                }
            }
            if subs.len() == 1 {
                // The angle no longer cuts it — a surface that lies over itself facing one way, a
                // flat spiral: one triangle a chart, which cannot overlap.
                pending.extend(members.iter().map(|&t| (vec![t], half)));
            } else {
                pending.extend(subs.into_iter().map(|sub| (sub, half)));
            }
        }
    }

    let Some(scale) = fit(&charts, settings) else {
        return Err(UnwrapError::DoesNotFit { charts: charts.len() });
    };
    let placed = pack(&charts, scale, settings).ok_or(UnwrapError::DoesNotFit { charts: charts.len() })?;

    let r = res as f32;
    let mut first_point = None;
    for (chart, offset) in charts.iter().zip(&placed) {
        for (k, &t) in chart.members.iter().enumerate() {
            for c in 0..3 {
                let local = (chart.points[3 * k + c] - chart.min) * scale;
                let uv = (*offset + local) / r;
                vertices[3 * t as usize + c].lightmap_uv = uv.to_array();
                first_point.get_or_insert(uv);
            }
        }
    }
    let point = first_point.unwrap_or(Vec2::splat(0.5)).to_array();
    for (t, n) in normals.iter().enumerate() {
        if n.is_none() {
            for c in 0..3 {
                vertices[3 * t + c].lightmap_uv = point;
            }
        }
    }
    Ok(Unwrapped { charts: charts.len(), texels_per_unit: scale })
}

/// One chart: its triangles and their corners flattened (turned to the smallest rectangle), with
/// that rectangle's corner and size.
struct Chart {
    members: Vec<u32>,
    points: Vec<Vec2>,
    min: Vec2,
    size: Vec2,
}

impl Chart {
    /// Whether two of its triangles cover the same place: a grid over the chart, each cell's centre
    /// tested strictly inside each triangle (so triangles meeting at an edge do not count).
    fn overlaps(&self) -> bool {
        let long = self.size.max_element();
        if long <= 0.0 {
            return false;
        }
        const CELLS: f32 = 48.0;
        let step = long / CELLS;
        let (nx, ny) = ((self.size.x / step).ceil().max(1.0) as usize, (self.size.y / step).ceil().max(1.0) as usize);
        let mut hits = vec![0u8; nx * ny];
        for tri in self.points.as_chunks::<3>().0 {
            let (a, b, c) = (tri[0], tri[1], tri[2]);
            let d = (b - a).perp_dot(c - a);
            if d.abs() <= f32::EPSILON * long * long {
                continue;
            }
            let lo = a.min(b).min(c) - self.min;
            let hi = a.max(b).max(c) - self.min;
            let (x0, x1) = ((lo.x / step).floor().max(0.0) as usize, ((hi.x / step).ceil() as usize).min(nx));
            let (y0, y1) = ((lo.y / step).floor().max(0.0) as usize, ((hi.y / step).ceil() as usize).min(ny));
            for y in y0..y1 {
                for x in x0..x1 {
                    let p = self.min + Vec2::new(x as f32 + 0.5, y as f32 + 0.5) * step;
                    let u = (p - a).perp_dot(c - a) / d;
                    let v = (b - a).perp_dot(p - a) / d;
                    const E: f32 = 1e-4;
                    if u > E && v > E && u + v < 1.0 - E {
                        let h = &mut hits[y * nx + x];
                        *h += 1;
                        if *h > 1 {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
}

/// The triangles reachable from `seed` across shared edges, among those marked `free`, that face
/// within `cos_max` of it; each is marked `taken`.
fn grow(
    seed: usize,
    cos_max: f32,
    normals: &[Option<Vec3>],
    ids: &[u32],
    edges: &std::collections::HashMap<(u32, u32), Vec<u32>>,
    chart_of: &mut [u32],
    free: u32,
    taken: u32,
) -> Vec<u32> {
    let Some(axis) = normals[seed] else { return Vec::new() };
    let mut members = vec![seed as u32];
    chart_of[seed] = taken;
    let mut at = 0;
    while at < members.len() {
        let t = members[at] as usize;
        at += 1;
        for k in 0..3 {
            let (a, b) = (ids[3 * t + k], ids[3 * t + (k + 1) % 3]);
            for &n in &edges[&(a.min(b), a.max(b))] {
                let n = n as usize;
                if chart_of[n] == free && normals[n].is_some_and(|nn| nn.dot(axis) >= cos_max) {
                    chart_of[n] = taken;
                    members.push(n as u32);
                }
            }
        }
    }
    members
}

/// The chart's corners projected along its first triangle's normal and turned to the smallest
/// rectangle around them.
fn flatten(members: &[u32], vertices: &[Vertex]) -> Chart {
    let pos = |i: usize| Vec3::from(vertices[i].position);
    let t0 = members[0] as usize;
    let axis = (pos(3 * t0 + 1) - pos(3 * t0)).cross(pos(3 * t0 + 2) - pos(3 * t0)).normalize();
    let (u, v) = axis.any_orthonormal_pair();
    let mut points: Vec<Vec2> =
        members.iter().flat_map(|&t| (0..3).map(move |c| 3 * t as usize + c)).map(|i| Vec2::new(pos(i).dot(u), pos(i).dot(v))).collect();
    let turn = smallest_rectangle(&points);
    for p in &mut points {
        *p = turn.rotate(*p);
    }
    let min = points.iter().fold(Vec2::splat(f32::MAX), |m, p| m.min(*p));
    let max = points.iter().fold(Vec2::splat(f32::MIN), |m, p| m.max(*p));
    let mut size = max - min;
    // Taller than wide, so packing rows by height wastes less.
    if size.x > size.y {
        let quarter = Vec2::new(0.0, 1.0);
        for p in &mut points {
            *p = quarter.rotate(*p);
        }
        let min = points.iter().fold(Vec2::splat(f32::MAX), |m, p| m.min(*p));
        let max = points.iter().fold(Vec2::splat(f32::MIN), |m, p| m.max(*p));
        size = max - min;
        return Chart { members: members.to_vec(), points, min, size };
    }
    Chart { members: members.to_vec(), points, min, size }
}

/// The rotation (as a unit complex number) that puts `points` in the smallest-area rectangle with
/// axis-aligned sides: one side of the best rectangle lies along an edge of the convex hull.
fn smallest_rectangle(points: &[Vec2]) -> Vec2 {
    let hull = convex_hull(points);
    let mut best = (f32::MAX, Vec2::X);
    for i in 0..hull.len() {
        let Some(edge) = (hull[(i + 1) % hull.len()] - hull[i]).try_normalize() else { continue };
        // Turn the edge onto +X: multiply by its conjugate.
        let turn = Vec2::new(edge.x, -edge.y);
        let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for p in &hull {
            let q = turn.rotate(*p);
            lo = lo.min(q);
            hi = hi.max(q);
        }
        let area = (hi - lo).x * (hi - lo).y;
        if area < best.0 - 1e-9 {
            best = (area, turn);
        }
    }
    best.1
}

/// Andrew's monotone chain, counter-clockwise.
fn convex_hull(points: &[Vec2]) -> Vec<Vec2> {
    let mut p = points.to_vec();
    p.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    p.dedup();
    if p.len() < 3 {
        return p;
    }
    let mut hull: Vec<Vec2> = Vec::with_capacity(p.len() * 2);
    for pass in 0..2 {
        let start = hull.len();
        let iter: Box<dyn Iterator<Item = &Vec2>> = if pass == 0 { Box::new(p.iter()) } else { Box::new(p.iter().rev()) };
        for &q in iter {
            while hull.len() >= start + 2 && (hull[hull.len() - 1] - hull[hull.len() - 2]).perp_dot(q - hull[hull.len() - 2]) <= 0.0 {
                hull.pop();
            }
            hull.push(q);
        }
        hull.pop();
    }
    hull
}

/// The largest scale (texels per unit) at which [`pack`] places every chart.
fn fit(charts: &[Chart], settings: &UnwrapSettings) -> Option<f32> {
    let longest = charts.iter().map(|c| c.size.max_element()).fold(0.0f32, f32::max);
    if longest <= 0.0 {
        return None;
    }
    let mut lo = 0.0f32;
    let mut hi = settings.resolution as f32 / longest;
    if pack(charts, hi, settings).is_some() {
        return Some(hi);
    }
    for _ in 0..40 {
        let mid = 0.5 * (lo + hi);
        if pack(charts, mid, settings).is_some() {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    // A scale so small every chart is under a texel still needs a texel and its padding each.
    (lo > 0.0 && pack(charts, lo, settings).is_some()).then_some(lo)
}

/// Where each chart's rectangle corner goes, in texels, at `scale`: rows tallest first, `padding`
/// texels between charts and from the edges, every chart on whole texels. `None` if they do not fit.
fn pack(charts: &[Chart], scale: f32, settings: &UnwrapSettings) -> Option<Vec<Vec2>> {
    let res = settings.resolution;
    let pad = settings.padding;
    let cells = |c: &Chart| ((c.size * scale).ceil().max(Vec2::ONE)).as_uvec2();
    let mut order: Vec<usize> = (0..charts.len()).collect();
    order.sort_by_key(|&i| (std::cmp::Reverse(cells(&charts[i]).y), i));
    let mut out = vec![Vec2::ZERO; charts.len()];
    let (mut x, mut y, mut row) = (pad, pad, 0u32);
    for i in order {
        let size = cells(&charts[i]);
        if x + size.x + pad > res {
            x = pad;
            y += row + pad;
            row = 0;
        }
        if x + size.x + pad > res || y + size.y + pad > res {
            return None;
        }
        out[i] = Vec2::new(x as f32, y as f32);
        x += size.x + pad;
        row = row.max(size.y);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube() -> Vec<Vertex> {
        crate::asset::AssetManager::cube_data()
    }

    /// Every triangle's second-UV-set corners, at `res` texels.
    fn uv_tris(vertices: &[Vertex], res: u32) -> Vec<[Vec2; 3]> {
        vertices.as_chunks::<3>().0.iter().map(|t| [0, 1, 2].map(|c| Vec2::from(t[c].lightmap_uv) * res as f32)).collect()
    }

    /// Texel centres strictly inside more than one triangle.
    fn overlapping_texels(tris: &[[Vec2; 3]], res: u32) -> usize {
        let mut hits = vec![0u32; (res * res) as usize];
        for [a, b, c] in tris {
            let d = (*b - *a).perp_dot(*c - *a);
            if d.abs() < 1e-9 {
                continue;
            }
            for y in 0..res {
                for x in 0..res {
                    let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                    let u = (p - *a).perp_dot(*c - *a) / d;
                    let v = (*b - *a).perp_dot(p - *a) / d;
                    if u > 1e-4 && v > 1e-4 && u + v < 1.0 - 1e-4 {
                        hits[(y * res + x) as usize] += 1;
                    }
                }
            }
        }
        hits.iter().filter(|&&h| h > 1).count()
    }

    /// A cube is six charts, inside the unit square and the padding, overlapping nowhere, at one
    /// texel density: every face the same area in the lightmap.
    #[test]
    fn a_cube_is_six_charts_that_do_not_overlap() {
        let mut v = cube();
        let settings = UnwrapSettings::default().with_resolution(64);
        let done = unwrap(&mut v, &settings).unwrap();
        assert_eq!(done.charts, 6);
        let tris = uv_tris(&v, 64);
        for t in &tris {
            for p in t {
                assert!(p.x >= 2.0 - 1e-3 && p.y >= 2.0 - 1e-3 && p.x <= 62.0 + 1e-3 && p.y <= 62.0 + 1e-3, "{p} outside the padding");
            }
        }
        assert_eq!(overlapping_texels(&tris, 64), 0);
        let areas: Vec<f32> = tris.chunks(2).map(|f| f.iter().map(|[a, b, c]| (*b - *a).perp_dot(*c - *a).abs() * 0.5).sum()).collect();
        for a in &areas {
            assert!((a - areas[0]).abs() < 1e-2 * areas[0], "faces of different sizes: {areas:?}");
        }
        // A face 2 units across at this density fills most of a third of the map.
        assert!(done.texels_per_unit * 2.0 > 15.0, "{}", done.texels_per_unit);
    }

    /// Charts are `padding` texels apart: no texel within it of one chart is in another's rectangle.
    #[test]
    fn charts_are_kept_apart_by_the_padding() {
        let mut v = crate::asset::AssetManager::cylinder_data(0.5, 2.0, 12);
        unwrap(&mut v, &UnwrapSettings::default().with_resolution(128).with_padding(3)).unwrap();
        // Group corners by chart: triangles connected in UV space share corners exactly.
        let tris = uv_tris(&v, 128);
        let boxes = chart_boxes(&tris);
        for (i, a) in boxes.iter().enumerate() {
            for b in &boxes[i + 1..] {
                let gap_x = (b.0.x - a.1.x).max(a.0.x - b.1.x);
                let gap_y = (b.0.y - a.1.y).max(a.0.y - b.1.y);
                assert!(gap_x.max(gap_y) >= 3.0 - 1e-3, "charts {a:?} and {b:?} are closer than the padding");
            }
        }
    }

    /// The bounding boxes of the groups of triangles that share UV corners.
    fn chart_boxes(tris: &[[Vec2; 3]]) -> Vec<(Vec2, Vec2)> {
        let n = tris.len();
        let mut group: Vec<usize> = (0..n).collect();
        fn find(g: &mut [usize], i: usize) -> usize {
            if g[i] != i {
                let r = find(g, g[i]);
                g[i] = r;
            }
            g[i]
        }
        for i in 0..n {
            for j in i + 1..n {
                if tris[i].iter().any(|p| tris[j].iter().any(|q| p.distance(*q) < 1e-4)) {
                    let (a, b) = (find(&mut group, i), find(&mut group, j));
                    group[a] = b;
                }
            }
        }
        let mut boxes: std::collections::BTreeMap<usize, (Vec2, Vec2)> = std::collections::BTreeMap::new();
        for (i, t) in tris.iter().enumerate() {
            let e = boxes.entry(find(&mut group, i)).or_insert((Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)));
            for p in t {
                e.0 = e.0.min(*p);
                e.1 = e.1.max(*p);
            }
        }
        boxes.into_values().collect()
    }

    /// A sphere is cut into charts that do not overlap, every corner inside the map.
    #[test]
    fn a_sphere_unwraps_without_overlap() {
        let mut v = crate::asset::AssetManager::sphere_data(1.0, 12, 16);
        let done = unwrap(&mut v, &UnwrapSettings::default().with_resolution(128)).unwrap();
        assert!(done.charts > 1 && done.charts < v.len() / 3, "{} charts", done.charts);
        assert_eq!(overlapping_texels(&uv_tris(&v, 128), 128), 0);
    }

    /// A strip that curls over itself within the angle — a flat ring, every triangle facing up — is
    /// one chart by its normals, but would overlap where it comes round: it is cut until it does not.
    #[test]
    fn a_chart_that_lies_over_itself_is_cut() {
        // A spiral ramp: a ring of quads that climbs and comes round over its own start.
        let mut v = Vec::new();
        let steps = 40;
        for i in 0..steps {
            let at = |k: usize, r: f32| {
                let a = k as f32 / 30.0 * std::f32::consts::TAU;
                Vertex { position: [a.cos() * r, k as f32 * 0.01, a.sin() * r], ..Default::default() }
            };
            let (a, b, c, d) = (at(i, 1.0), at(i, 2.0), at(i + 1, 2.0), at(i + 1, 1.0));
            v.extend_from_slice(&[a, c, b, a, d, c]);
        }
        let done = unwrap(&mut v, &UnwrapSettings::default().with_resolution(128)).unwrap();
        assert!(done.charts >= 2, "the spiral was left as one chart");
        assert_eq!(overlapping_texels(&uv_tris(&v, 128), 128), 0);
    }

    /// Too many charts for the map is an error, not a layout that overlaps.
    #[test]
    fn too_many_charts_is_an_error() {
        let mut v = crate::asset::AssetManager::sphere_data(1.0, 24, 32);
        let err = unwrap(&mut v, &UnwrapSettings::default().with_resolution(8).with_padding(2).with_max_angle(0.05)).unwrap_err();
        assert!(matches!(err, UnwrapError::DoesNotFit { .. }), "{err:?}");
        assert_eq!(unwrap(&mut [], &UnwrapSettings::default()).unwrap_err(), UnwrapError::NoTriangles);
        assert_eq!(unwrap(&mut cube(), &UnwrapSettings::default().with_resolution(4)).unwrap_err(), UnwrapError::BadSettings);
    }

    /// An unwrapped sphere bakes as a receiver: alone under a uniform sky every texel on it sees a
    /// whole hemisphere of sky, so the whole lightmap is the sky's colour — every chart covered,
    /// no texel left black between them that the padding should have filled.
    #[test]
    fn an_unwrapped_sphere_bakes_the_sky_all_over() {
        use crate::lightmap_bake::{BakeMesh, BakeSettings, LightmapBake};
        let mut v = crate::asset::AssetManager::sphere_data(1.0, 12, 16);
        unwrap(&mut v, &UnwrapSettings::default().with_resolution(64)).unwrap();
        let mut bake = LightmapBake::new()
            .with_sky(crate::frame_uniforms::Sky::uniform([1.0, 1.0, 1.0]))
            .with_settings(BakeSettings::default().with_samples(16).with_padding(2));
        let id = bake.add_mesh(BakeMesh::new(v.clone(), gizmo_math::Mat4::IDENTITY).receiving(64, 64));
        let map = bake.run().unwrap().get(id).unwrap().clone();
        // Every texel a triangle's corner rounds to is lit.
        for [a, b, c] in uv_tris(&v, 64) {
            let centre = (a + b + c) / 3.0;
            let t = map.texel(centre.x as u32, centre.y as u32);
            assert!(t.x > 0.95, "texel at {centre} is {t}");
        }
    }
}
