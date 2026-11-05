//! Planar reflections: the scene drawn again from the camera reflected in a mirror's plane.
//!
//! Each frame, for each [`PlanarReflection`](crate::components::PlanarReflection) in view and seen from
//! its front, the view is built here, the host draws the scene through it into the HDR target, and
//! the result is copied to the mirror's own texture. The mirror's pipeline then looks that texture
//! up by screen position (`planar_mirror.wgsl`). The capture is linear HDR from before the post chain,
//! so the reflection is exposed and tone-mapped once, with the rest of the frame.

use gizmo_math::{Mat4, Vec3, Vec4};
use std::collections::HashMap;

/// A point reflected in `plane` (`normal` unit length).
#[must_use]
pub fn reflect_point(point: Vec3, plane: Vec4) -> Vec3 {
    point - 2.0 * (plane.truncate().dot(point) + plane.w) * plane.truncate()
}

/// A direction reflected in a plane of unit `normal`.
#[must_use]
pub fn reflect_direction(direction: Vec3, normal: Vec3) -> Vec3 {
    direction - 2.0 * normal.dot(direction) * normal
}

/// `projection` with its near plane moved onto `plane` (world space, the side to keep positive) as
/// seen through `view`: whatever stands behind the mirror is clipped, or it would appear in the
/// reflection, in front of everything. Lengyel's oblique near plane, for a 0..1 depth range; it needs
/// no clip distance in any shader, so every pipeline in the frame obeys it.
#[must_use]
pub fn oblique_projection(projection: Mat4, view: Mat4, plane: Vec4) -> Mat4 {
    let clip_plane = view.inverse().transpose() * plane;
    // The corner of the frustum farthest along the plane: the new far plane passes through it, so
    // nothing the original frustum held is lost.
    let corner = projection.inverse()
        * Vec4::new(clip_plane.x.signum(), clip_plane.y.signum(), 1.0, 1.0);
    let scale = clip_plane.dot(corner);
    if !scale.is_finite() || scale.abs() < 1e-9 {
        return projection;
    }
    let row = clip_plane / scale;
    let mut oblique = projection;
    // Third row of the matrix; glam stores columns.
    oblique.x_axis.z = row.x;
    oblique.y_axis.z = row.y;
    oblique.z_axis.z = row.z;
    oblique.w_axis.z = row.w;
    oblique
}

struct Mirror {
    texture: wgpu::Texture,
    bind_group: std::sync::Arc<wgpu::BindGroup>,
    captured: bool,
}

/// Every mirror's texture. One per mirror, at the frame's size; rebuilt when that changes.
#[derive(Default)]
pub struct PlanarReflections {
    mirrors: HashMap<u32, Mirror>,
    size: (u32, u32),
    /// How many mirrors drew the scene again this frame: what they cost.
    pub captures: u32,
}

impl PlanarReflections {
    /// Forgets this frame's captures; a mirror draws plain until it has a new one.
    pub fn begin_frame(&mut self) {
        self.captures = 0;
        for mirror in self.mirrors.values_mut() {
            mirror.captured = false;
        }
    }

    /// The bind group of `owner`'s capture, if it has one this frame.
    #[must_use]
    pub fn captured(&self, owner: u32) -> Option<&wgpu::BindGroup> {
        self.mirrors.get(&owner).filter(|m| m.captured).map(|m| &*m.bind_group)
    }

    /// Drops the textures of mirrors no longer in `owners`.
    pub fn retain(&mut self, owners: &[u32]) {
        self.mirrors.retain(|id, _| owners.contains(id));
    }

    /// Whether `owner` has a texture of `size`; a different size drops them all.
    pub fn prepare(&mut self, owner: u32, size: (u32, u32)) -> bool {
        if self.size != size {
            self.mirrors.clear();
            self.size = size;
        }
        self.mirrors.contains_key(&owner)
    }

    /// Adds `owner`'s texture and the bind group that shows it.
    pub fn insert(&mut self, owner: u32, texture: wgpu::Texture, bind_group: std::sync::Arc<wgpu::BindGroup>) {
        self.mirrors.insert(owner, Mirror { texture, bind_group, captured: false });
    }

    /// Records copying the finished capture from `source` (the HDR target) into `owner`'s texture.
    pub fn record_copy(&mut self, encoder: &mut wgpu::CommandEncoder, source: &wgpu::Texture, owner: u32) {
        let Some(mirror) = self.mirrors.get_mut(&owner) else { return };
        encoder.copy_texture_to_texture(
            source.as_image_copy(),
            mirror.texture.as_image_copy(),
            wgpu::Extent3d { width: self.size.0, height: self.size.1, depth_or_array_layers: 1 },
        );
        mirror.captured = true;
        self.captures += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A floor at y = 1, reflecting upward.
    const FLOOR: Vec4 = Vec4::new(0.0, 1.0, 0.0, -1.0);

    #[test]
    fn a_reflection_mirrors_across_the_plane_and_fixes_points_on_it() {
        assert_eq!(reflect_point(Vec3::new(2.0, 5.0, -3.0), FLOOR), Vec3::new(2.0, -3.0, -3.0));
        assert_eq!(reflect_point(Vec3::new(9.0, 1.0, 9.0), FLOOR), Vec3::new(9.0, 1.0, 9.0));
        assert_eq!(reflect_direction(Vec3::new(1.0, -1.0, 0.0), Vec3::Y), Vec3::new(1.0, 1.0, 0.0));
    }

    /// The oblique near plane: on the mirror the depth is exactly 0, behind it negative (clipped),
    /// and everything the original frustum held in front stays within 0..=1.
    #[test]
    fn the_oblique_near_plane_lies_on_the_mirror_and_loses_nothing() {
        let eye = Vec3::new(0.0, -4.0, 6.0); // reflected, below the floor
        let view = Mat4::look_to_rh(eye, Vec3::new(0.0, 0.6, -0.8), Vec3::Y);
        let projection = Mat4::perspective_rh(1.0, 1.5, 0.1, 200.0);
        let oblique = oblique_projection(projection, view, FLOOR);
        let depth = |p: Vec3| {
            let c = oblique * view * p.extend(1.0);
            c.z / c.w
        };
        for p in [Vec3::new(0.0, 1.0, 0.0), Vec3::new(3.0, 1.0, -4.0), Vec3::new(-2.0, 1.0, 1.0)] {
            assert!(depth(p).abs() < 1e-3, "on the mirror: {p:?} has depth {}", depth(p));
        }
        assert!(depth(Vec3::new(0.0, 0.5, 0.0)) < 0.0, "behind the mirror must be clipped");
        // Above the mirror, near and far inside the original frustum.
        let original = projection * view;
        let mut kept = 0;
        for i in 1..60 {
            let p = eye + Vec3::new(0.0, 0.6, -0.8) * (i as f32 * 3.0);
            if p.y <= 1.0 {
                continue;
            }
            let c = original * p.extend(1.0);
            if c.z / c.w > 1.0 {
                continue;
            }
            let d = depth(p);
            assert!((0.0..=1.0).contains(&d), "{p:?} left the depth range: {d}");
            kept += 1;
        }
        assert!(kept > 10, "the setup kept {kept} points");
    }

    /// Picture from the reflected camera, mirrored left to right, is the picture a reflection shows:
    /// a point in front of the mirror lands, through the rotation-only camera and `1 - u`, where its
    /// mirror image lands through the real camera.
    #[test]
    fn the_flipped_capture_lines_up_with_the_mirror_image() {
        let eye = Vec3::new(2.0, 5.0, 8.0);
        let forward = Vec3::new(-0.2, -0.5, -0.8).normalize();
        let right = forward.cross(Vec3::Y).normalize();
        let up = right.cross(forward);
        let projection = Mat4::perspective_rh(1.0, 1.5, 0.1, 200.0);
        let real = projection * Mat4::look_to_rh(eye, forward, up);
        let captured = projection
            * Mat4::look_to_rh(
                reflect_point(eye, FLOOR),
                reflect_direction(forward, Vec3::Y),
                reflect_direction(up, Vec3::Y),
            );
        let ndc = |m: Mat4, p: Vec3| {
            let c = m * p.extend(1.0);
            (c.x / c.w, c.y / c.w)
        };
        for p in [Vec3::new(1.0, 2.0, -1.0), Vec3::new(-3.0, 4.0, -6.0), Vec3::new(0.5, 1.5, 2.0)] {
            let (x, y) = ndc(captured, p);
            let (rx, ry) = ndc(real, reflect_point(p, FLOOR));
            assert!((x + rx).abs() < 1e-3 && (y - ry).abs() < 1e-3, "{p:?}: ({x}, {y}) against ({rx}, {ry})");
        }
    }
}
