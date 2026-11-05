//! Occlusion culling: what stands behind a wall is not drawn.
//!
//! A game marks its big opaque things with [`Occluder`](crate::components::Occluder). Each view,
//! their boxes are rasterised on the CPU into a small depth buffer, and every object the frustum let
//! through is tested against it before it costs an instance, a draw or a fragment.
//!
//! **Why on the CPU, and why not the GPU's last-frame depth.** A hierarchical depth buffer read back
//! from the GPU arrives a frame or two late: an object coming out from behind a corner pops in, and
//! a moving occluder hides what it no longer covers. This one is built from this frame's camera and
//! this frame's transforms, so it has no latency to hide; it needs no readback, which the browser
//! and a phone pay dearly for; and it runs the same on every backend.
//!
//! **It must never hide something visible, and may keep something hidden.** So both sides of the
//! comparison lean the same way:
//!
//! - an occluder writes only pixels it covers *completely*, at the *farthest* depth it has in them;
//! - an object reads every pixel its box *touches*, one more on each side for TAA's jitter, at the
//!   *nearest* depth of the box.
//!
//! Faces are rasterised as whole quads: two triangles would each leave the pixels along their shared
//! diagonal not completely covered, a hole across every wall. And only faces towards the camera,
//! because that is what the mesh draws: a camera inside a box sees out of it.
//!
//! The cost is opt-in: a scene with no `Occluder` builds no buffer and tests nothing.

use gizmo_math::{Aabb, Mat4, Vec3, Vec4};

/// How many pixels the buffer has, whatever the window's shape: what 256 × 144 costs. Cheap enough
/// to rebuild every view, and one cell is still only a few frame pixels wide.
const BUFFER_PIXELS: f32 = 256.0 * 144.0;

/// The depth buffer an occluder's box is rasterised into and an object's box is tested against.
#[derive(Clone, Debug)]
pub struct OcclusionBuffer {
    width: usize,
    height: usize,
    depth: Vec<f32>,
    view_proj: Mat4,
    drawn: bool,
}

/// The six faces of a box, as corners of [`Aabb::corners`], each wound so a face towards the camera
/// has positive area on screen.
const FACES: [[usize; 4]; 6] = [
    [0, 1, 3, 2], // x min
    [4, 6, 7, 5], // x max
    [0, 4, 5, 1], // y min
    [2, 3, 7, 6], // y max
    [0, 2, 6, 4], // z min
    [1, 5, 7, 3], // z max
];

impl OcclusionBuffer {
    /// An empty buffer for a view through `view_proj`, shaped like a `aspect` (width / height)
    /// window.
    #[must_use]
    pub fn new(view_proj: Mat4, aspect: f32) -> Self {
        let aspect = if aspect.is_finite() && aspect > 0.0 { aspect.clamp(0.1, 10.0) } else { 1.0 };
        let height = (BUFFER_PIXELS / aspect).sqrt().round().max(1.0);
        let width = (height * aspect).round().max(1.0);
        let (width, height) = (width as usize, height as usize);
        Self { width, height, depth: vec![f32::INFINITY; width * height], view_proj, drawn: false }
    }

    /// Whether any occluder wrote a pixel. Without one every test answers "visible".
    #[must_use]
    pub fn is_empty(&self) -> bool {
        !self.drawn
    }

    /// Rasterises the box `local`, placed by `model`, as a solid opaque occluder.
    pub fn add_box(&mut self, model: &Mat4, local: Aabb) {
        let to_clip = self.view_proj * *model;
        let corners = local.corners().map(|c| to_clip * Vec4::new(c.x, c.y, c.z, 1.0));
        // A mirroring transform turns every face inside out.
        let mirrored = model.determinant() < 0.0;
        for face in FACES {
            let mut quad = face.map(|i| corners[i]);
            if mirrored {
                quad.reverse();
            }
            self.add_face(&quad);
        }
    }

    fn add_face(&mut self, face: &[Vec4; 4]) {
        // Clipped to the near plane (z >= 0 in clip space, which also means w > 0 for a perspective
        // matrix): a wall the camera stands close to still covers what is behind it.
        let mut points = [Vec3::ZERO; 8];
        let mut count = 0;
        for i in 0..4 {
            let (a, b) = (face[i], face[(i + 1) % 4]);
            if a.z >= 0.0 && a.w > 1e-6 {
                points[count] = self.to_buffer(a);
                count += 1;
            }
            if (a.z >= 0.0) != (b.z >= 0.0) {
                let cut = a + (b - a) * (a.z / (a.z - b.z));
                if cut.w > 1e-6 {
                    points[count] = self.to_buffer(cut);
                    count += 1;
                }
            }
        }
        if count >= 3 {
            self.fill_convex(&points[..count]);
        }
    }

    /// Clip-space point to buffer pixels (x, y) and depth.
    fn to_buffer(&self, clip: Vec4) -> Vec3 {
        let ndc = clip.truncate() / clip.w;
        Vec3::new(
            (ndc.x * 0.5 + 0.5) * self.width as f32,
            (0.5 - ndc.y * 0.5) * self.height as f32,
            ndc.z,
        )
    }

    fn fill_convex(&mut self, points: &[Vec3]) {
        // Twice the signed area. Screen y is down, so a face towards the camera is positive here;
        // a back face, or one edge-on, draws nothing, as it does on the GPU.
        let mut area = 0.0;
        for i in 0..points.len() {
            let (a, b) = (points[i], points[(i + 1) % points.len()]);
            area += (b.x - a.x) * (b.y + a.y);
        }
        if !area.is_finite() || area <= 1e-3 {
            return;
        }

        // The face's plane, depth as a function of x and y, from its largest corner triangle.
        let (p0, mut p1, mut p2, mut best) = (points[0], points[1], points[2], 0.0f32);
        for i in 1..points.len() - 1 {
            let (a, b) = (points[i], points[i + 1]);
            let twice = (a.x - p0.x) * (b.y - p0.y) - (a.y - p0.y) * (b.x - p0.x);
            if twice.abs() > best {
                (p1, p2, best) = (a, b, twice.abs());
            }
        }
        let det = (p1.x - p0.x) * (p2.y - p0.y) - (p1.y - p0.y) * (p2.x - p0.x);
        if det == 0.0 {
            return;
        }
        let dz_dx = ((p1.z - p0.z) * (p2.y - p0.y) - (p1.y - p0.y) * (p2.z - p0.z)) / det;
        let dz_dy = ((p1.x - p0.x) * (p2.z - p0.z) - (p1.z - p0.z) * (p2.x - p0.x)) / det;
        // Half a pixel of slope towards the far corner, plus the rounding a plane this far from its
        // origin can carry.
        let lean = (dz_dx.abs() + dz_dy.abs()) * 0.5 + 1e-5;

        let (mut min_x, mut max_x, mut min_y, mut max_y) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for p in points {
            min_x = min_x.min(p.x);
            max_x = max_x.max(p.x);
            min_y = min_y.min(p.y);
            max_y = max_y.max(p.y);
        }
        let x0 = min_x.floor().max(0.0) as usize;
        let y0 = min_y.floor().max(0.0) as usize;
        let x1 = (max_x.ceil().max(0.0) as usize).min(self.width);
        let y1 = (max_y.ceil().max(0.0) as usize).min(self.height);

        // Each edge as a x + b y + c, positive inside, with the worst corner of a pixel centred on
        // (x, y) already taken off: at or above zero means the whole pixel is inside this edge.
        let mut edges = [(0.0f32, 0.0f32, 0.0f32); 8];
        let count = points.len().min(edges.len());
        for i in 0..count {
            let (s, e) = (points[i], points[(i + 1) % points.len()]);
            let (a, b) = (e.y - s.y, s.x - e.x);
            edges[i] = (a, b, -(a * s.x + b * s.y) - (a.abs() + b.abs()) * 0.5);
        }

        for y in y0..y1 {
            let cy = y as f32 + 0.5;
            'pixel: for x in x0..x1 {
                let cx = x as f32 + 0.5;
                for e in &edges[..count] {
                    if e.0 * cx + e.1 * cy + e.2 < 0.0 {
                        continue 'pixel;
                    }
                }
                let far = p0.z + dz_dx * (cx - p0.x) + dz_dy * (cy - p0.y) + lean;
                let cell = &mut self.depth[y * self.width + x];
                if far < *cell {
                    *cell = far;
                    self.drawn = true;
                }
            }
        }
    }

    /// Whether everything in the world-space box `bounds` is behind what has been drawn.
    #[must_use]
    pub fn is_occluded(&self, bounds: Aabb) -> bool {
        if !self.drawn {
            return false;
        }
        let (mut min_x, mut max_x, mut min_y, mut max_y) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        let mut nearest = f32::MAX;
        for c in bounds.corners() {
            let clip = self.view_proj * Vec4::new(c.x, c.y, c.z, 1.0);
            // Reaches the camera's plane: no rectangle on screen means anything.
            if clip.w <= 1e-6 || clip.z < 0.0 {
                return false;
            }
            let p = self.to_buffer(clip);
            min_x = min_x.min(p.x);
            max_x = max_x.max(p.x);
            min_y = min_y.min(p.y);
            max_y = max_y.max(p.y);
            nearest = nearest.min(p.z);
        }
        // One more pixel on each side: TAA draws through a jittered matrix this one does not have.
        let x0 = (min_x.floor() - 1.0).max(0.0) as usize;
        let y0 = (min_y.floor() - 1.0).max(0.0) as usize;
        let x1 = ((max_x.ceil() + 1.0).max(0.0) as usize).min(self.width);
        let y1 = ((max_y.ceil() + 1.0).max(0.0) as usize).min(self.height);
        if x0 >= x1 || y0 >= y1 {
            return false;
        }
        for y in y0..y1 {
            let row = &self.depth[y * self.width + x0..y * self.width + x1];
            if row.iter().any(|&d| d >= nearest) {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_math::Quat;

    fn view() -> Mat4 {
        let proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_3, 16.0 / 9.0, 0.1, 500.0);
        proj * Mat4::look_at_rh(Vec3::new(0.0, 0.0, 10.0), Vec3::ZERO, Vec3::Y)
    }

    fn unit() -> Aabb {
        Aabb::new(Vec3::splat(-1.0), Vec3::splat(1.0))
    }

    fn at(x: f32, y: f32, z: f32, half: f32) -> Aabb {
        Aabb::new(Vec3::new(x, y, z) - Vec3::splat(half), Vec3::new(x, y, z) + Vec3::splat(half))
    }

    /// A wall 12 wide, 8 high and thin, at z = 0, between the camera at z = 10 and -z.
    fn wall() -> OcclusionBuffer {
        let mut b = OcclusionBuffer::new(view(), 16.0 / 9.0);
        b.add_box(&Mat4::from_scale(Vec3::new(6.0, 4.0, 0.1)), unit());
        b
    }

    #[test]
    fn a_wall_hides_what_is_behind_it_and_nothing_else() {
        let b = wall();
        assert!(!b.is_empty());
        assert!(b.is_occluded(at(0.0, 0.0, -5.0, 0.5)), "right behind it");
        assert!(b.is_occluded(at(1.5, -1.0, -2.0, 0.5)), "behind it, off centre");
        assert!(!b.is_occluded(at(0.0, 0.0, 5.0, 0.5)), "in front of it");
        assert!(!b.is_occluded(at(30.0, 0.0, -30.0, 1.0)), "beside it");
        assert!(!b.is_occluded(at(0.0, 0.0, 0.0, 0.2)), "through it: nearer than its face");
    }

    #[test]
    fn something_sticking_out_from_behind_is_visible() {
        let b = wall();
        // Behind the wall, but its top passes the wall's edge as seen from the camera.
        assert!(!b.is_occluded(at(0.0, 5.0, -5.0, 2.0)));
        assert!(!b.is_occluded(at(0.0, 0.0, -5.0, 30.0)), "bigger than the wall");
    }

    /// Two triangles would leave the diagonal uncovered: a thin object behind the middle of a wall
    /// lies exactly across it.
    #[test]
    fn a_face_has_no_seam_along_its_diagonal() {
        let b = wall();
        for i in -5..=5 {
            let t = i as f32 * 0.4;
            assert!(b.is_occluded(at(t, t * 4.0 / 6.0, -1.0, 0.05)), "a hole at step {i}");
        }
    }

    /// The buffer's half of the promise, read directly: a cell the face only partly covers is not
    /// written, and a covered one holds the face's farthest depth in it, not a nearer one. (The pad
    /// on the testing side would hide a mistake here from any test that goes through it.)
    #[test]
    fn only_a_wholly_covered_cell_is_written_at_its_farthest_depth() {
        let mut b = OcclusionBuffer::new(Mat4::IDENTITY, 1.0);
        let (w, h) = (b.width as f32, b.height as f32);
        // In NDC: x from the middle of cell 10 to the middle of cell 20, y over the whole height.
        let x = |cell: f32| cell / w * 2.0 - 1.0;
        let face = [
            Vec4::new(x(10.5), -1.5, 0.4, 1.0),
            Vec4::new(x(20.5), -1.5, 0.6, 1.0),
            Vec4::new(x(20.5), 1.5, 0.6, 1.0),
            Vec4::new(x(10.5), 1.5, 0.4, 1.0),
        ];
        b.add_face(&face);
        let row = (h as usize / 2) * b.width;
        assert_eq!(b.depth[row + 10], f32::INFINITY, "half covered, and written");
        assert_eq!(b.depth[row + 20], f32::INFINITY, "half covered, and written");
        let slope = 0.2 / 10.0;
        for cell in 11..20 {
            let centre = 0.4 + (cell as f32 + 0.5 - 10.5) * slope;
            let written = b.depth[row + cell];
            assert!(written >= centre + slope * 0.5, "cell {cell} holds {written}, nearer than its far edge");
            assert!(written <= centre + slope * 0.5 + 1e-3, "cell {cell} holds {written}, needlessly far");
        }
    }

    /// The box's world AABB is bigger than a rotated wall; only the wall counts.
    #[test]
    fn a_rotated_wall_hides_only_what_it_covers() {
        let mut b = OcclusionBuffer::new(view(), 16.0 / 9.0);
        let model = Mat4::from_rotation_z(std::f32::consts::FRAC_PI_4) * Mat4::from_scale(Vec3::new(8.0, 0.5, 0.1));
        b.add_box(&model, unit());
        assert!(b.is_occluded(at(0.0, 0.0, -1.0, 0.1)), "on its long axis");
        assert!(!b.is_occluded(at(4.0, -4.0, -1.0, 0.1)), "inside its AABB, off the wall");
    }

    /// A mesh's back faces are culled, so a camera inside a box sees through it.
    #[test]
    fn a_camera_inside_a_box_is_not_blinded() {
        let mut b = OcclusionBuffer::new(view(), 16.0 / 9.0);
        b.add_box(&Mat4::from_scale(Vec3::splat(20.0)), unit());
        assert!(!b.is_occluded(at(0.0, 0.0, -50.0, 1.0)));
    }

    #[test]
    fn a_mirroring_transform_does_not_turn_it_inside_out() {
        let mut b = OcclusionBuffer::new(view(), 16.0 / 9.0);
        b.add_box(&Mat4::from_scale(Vec3::new(-6.0, 4.0, 0.1)), unit());
        assert!(b.is_occluded(at(0.0, 0.0, -5.0, 0.5)));
        assert!(!b.is_occluded(at(0.0, 0.0, 5.0, 0.5)));
    }

    /// A wall reaching past the camera's near plane still covers the rest of its span.
    #[test]
    fn a_wall_through_the_near_plane_still_hides() {
        let mut b = OcclusionBuffer::new(view(), 16.0 / 9.0);
        // Runs from far ahead along -z to behind the camera, off to the right, and faces left.
        let model = Mat4::from_translation(Vec3::new(2.0, 0.0, 0.0)) * Mat4::from_scale(Vec3::new(0.1, 8.0, 40.0));
        b.add_box(&model, unit());
        assert!(!b.is_empty(), "nothing was drawn");
        assert!(b.is_occluded(at(6.0, 0.0, -10.0, 0.5)), "behind it, to the right");
        assert!(!b.is_occluded(at(-4.0, 0.0, -10.0, 0.5)), "on the camera's side");
    }

    #[test]
    fn an_object_reaching_the_camera_is_visible() {
        assert!(!wall().is_occluded(at(0.0, 0.0, 10.0, 1.0)));
    }

    #[test]
    fn no_occluder_hides_nothing() {
        let b = OcclusionBuffer::new(view(), 1.0);
        assert!(b.is_empty());
        assert!(!b.is_occluded(at(0.0, 0.0, -5.0, 0.5)));
    }

    /// Ray against a box placed by `model`: the distance along `dir` it first enters, if any.
    fn ray_hits(origin: Vec3, dir: Vec3, model: &Mat4) -> Option<f32> {
        let inv = model.inverse();
        let o = inv.transform_point3(origin);
        let d = inv.transform_vector3(dir);
        let (mut near, mut far) = (f32::MIN, f32::MAX);
        for axis in 0..3 {
            if d[axis].abs() < 1e-7 {
                if o[axis].abs() > 1.0 {
                    return None;
                }
                continue;
            }
            let (a, b) = ((-1.0 - o[axis]) / d[axis], (1.0 - o[axis]) / d[axis]);
            near = near.max(a.min(b));
            far = far.min(a.max(b));
        }
        (near <= far && far > 0.0 && near > 0.0).then_some(near)
    }

    /// **The promise, against ground truth.** Random walls and random objects, seen from random
    /// places: whenever the buffer calls an object hidden, every ray from the camera to it must hit
    /// a wall before reaching it. And it must not be useless: a good share of the hidden ones found.
    #[test]
    fn what_it_calls_hidden_is_hidden_from_every_side() {
        let mut seed = 0x2545_f491_u32;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed as f32 / u32::MAX as f32) * 2.0 - 1.0
        };
        let (mut called_hidden, mut truly_hidden) = (0, 0);
        for _ in 0..40 {
            let eye = Vec3::new(next() * 15.0, next() * 6.0, 18.0 + next() * 6.0);
            let aspect = 1.0 + next().abs();
            let vp = Mat4::perspective_rh(std::f32::consts::FRAC_PI_3, aspect, 0.1, 500.0)
                * Mat4::look_at_rh(eye, Vec3::new(next() * 3.0, next() * 3.0, 0.0), Vec3::Y);
            let mut buffer = OcclusionBuffer::new(vp, aspect);
            let walls: Vec<Mat4> = (0..4)
                .map(|_| {
                    Mat4::from_translation(Vec3::new(next() * 10.0, next() * 4.0, next() * 6.0))
                        * Mat4::from_quat(Quat::from_rotation_y(next() * 1.5) * Quat::from_rotation_x(next() * 0.5))
                        * Mat4::from_scale(Vec3::new(2.0 + next().abs() * 6.0, 1.0 + next().abs() * 5.0, 0.2))
                })
                .collect();
            for w in &walls {
                buffer.add_box(w, unit());
            }
            for _ in 0..60 {
                let c = Vec3::new(next() * 14.0, next() * 8.0, -8.0 + next() * 10.0);
                let half = 0.1 + next().abs() * 1.2;
                let object = at(c.x, c.y, c.z, half);
                // A grid over the box; only what lands on screen can be seen, or be hidden.
                let mut on_screen = 0;
                let mut hidden_by_rays = true;
                for gx in 0..=6 {
                    for gy in 0..=6 {
                        for gz in 0..=6 {
                            let f = |g: i32| g as f32 / 6.0 * 2.0 - 1.0;
                            let p = c + Vec3::new(f(gx), f(gy), f(gz)) * half;
                            let clip = vp * Vec4::new(p.x, p.y, p.z, 1.0);
                            if clip.w <= 0.0 || clip.x.abs() > clip.w || clip.y.abs() > clip.w {
                                continue;
                            }
                            on_screen += 1;
                            let to = p - eye;
                            let (dir, len) = (to.normalize(), to.length());
                            if !walls.iter().any(|w| ray_hits(eye, dir, w).is_some_and(|t| t < len)) {
                                hidden_by_rays = false;
                            }
                        }
                    }
                }
                if on_screen == 0 {
                    continue;
                }
                truly_hidden += u32::from(hidden_by_rays);
                if buffer.is_occluded(object) {
                    called_hidden += 1;
                    assert!(hidden_by_rays, "called hidden, and a ray reaches it: {object:?} from {eye:?}");
                }
            }
        }
        assert!(truly_hidden > 40, "the scene never hid anything: {truly_hidden}");
        assert!(called_hidden * 3 > truly_hidden, "too timid: {called_hidden} of {truly_hidden} found");
    }
}
