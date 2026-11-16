//! [`LightOccluder2D`]: a shape that casts 2D shadows onto lit sprites.

use gizmo_math::{Mat4, Vec2};

/// A shape that stops the scene's point and spot lights from reaching the **lit sprites**
/// ([`Sprite::lit`](crate::components::Sprite::lit)) behind it — Godot's `LightOccluder2D`.
///
/// Its outline is [`points`](Self::points), in the entity's own plane (x right, y up), placed by
/// the entity's transform like a sprite; [`closed`](Self::closed) joins the last point back to the
/// first. Shadows are cast in the world's XY plane — the plane a 2D game is drawn in, its camera
/// looking down −Z: a light reaches a sprite's pixel unless the line between them, seen from above
/// that plane, crosses an occluder's outline. A pixel inside a closed occluder is not in that
/// occluder's own shadow, so the sprite drawn over a wall stays lit on the side facing the light.
///
/// Up to 256 edges and 32 occluders a frame shade the sprites; past that the rest cast nothing
/// (a log line says so). The sun and other directional lights cast none: they have no position to
/// cast from in the plane.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct LightOccluder2D {
    /// The outline, in the entity's plane.
    pub points: Vec<Vec2>,
    /// Whether the last point joins the first: a closed shape, or an open line (a fence).
    pub closed: bool,
}

impl LightOccluder2D {
    /// A closed outline through `points`.
    #[must_use]
    pub fn polygon(points: Vec<Vec2>) -> Self {
        Self { points, closed: true }
    }

    /// An open line through `points`.
    #[must_use]
    pub fn line(points: Vec<Vec2>) -> Self {
        Self { points, closed: false }
    }

    /// A `size` rectangle centred on the entity — the outline of a crate, a wall, a pillar.
    #[must_use]
    pub fn rectangle(size: Vec2) -> Self {
        let h = size * 0.5;
        Self::polygon(vec![Vec2::new(-h.x, -h.y), Vec2::new(h.x, -h.y), Vec2::new(h.x, h.y), Vec2::new(-h.x, h.y)])
    }

    /// Its edges in the world's XY plane, placed by `model`: `[a.x, a.y, b.x, b.y]` each.
    #[must_use]
    pub fn world_edges(&self, model: Mat4) -> Vec<[f32; 4]> {
        let world: Vec<Vec2> = self.points.iter().map(|p| model.transform_point3(p.extend(0.0)).truncate()).collect();
        let n = world.len();
        if n < 2 {
            return Vec::new();
        }
        let count = if self.closed && n > 2 { n } else { n - 1 };
        (0..count)
            .map(|i| {
                let (a, b) = (world[i], world[(i + 1) % n]);
                [a.x, a.y, b.x, b.y]
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_math::Vec3;

    /// A rectangle is four edges, closed; a line one fewer than its points; the transform places them.
    #[test]
    fn an_occluder_is_its_edges_placed_in_the_world() {
        let square = LightOccluder2D::rectangle(Vec2::splat(2.0));
        let edges = square.world_edges(Mat4::from_translation(Vec3::new(10.0, 0.0, 5.0)));
        assert_eq!(edges.len(), 4);
        assert_eq!(edges[0], [9.0, -1.0, 11.0, -1.0], "moved by the transform, z dropped");
        assert_eq!(edges[3], [9.0, 1.0, 9.0, -1.0], "and closed back to the first point");
        let fence = LightOccluder2D::line(vec![Vec2::ZERO, Vec2::X, Vec2::new(2.0, 0.0)]);
        assert_eq!(fence.world_edges(Mat4::IDENTITY).len(), 2);
        assert!(LightOccluder2D::line(vec![Vec2::ZERO]).world_edges(Mat4::IDENTITY).is_empty());
    }
}
