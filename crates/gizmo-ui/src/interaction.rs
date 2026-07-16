use gizmo_core::query::{Query, Mut};
use gizmo_core::system::Res;
use gizmo_core::input::Input;
use crate::components::{Interaction, Node};

/// Returns whether `point` lies within a node's bounding box.
///
/// The box is treated as the half-open interval
/// `[position, position + size)` on each axis, so that a point on the shared
/// edge of two adjacent elements belongs to exactly one of them.
fn point_in_node(point: (f32, f32), node: &Node) -> bool {
    point.0 >= node.position.x
        && point.0 < node.position.x + node.size.x
        && point.1 >= node.position.y
        && point.1 < node.position.y + node.size.y
}

/// System that updates each element's [`Interaction`] state from the current
/// mouse position and button state.
pub fn ui_interaction_system(
    input: Res<Input>,
    mut interactions: Query<(&Node, Mut<Interaction>)>,
) {
    let mouse_pos = input.mouse_position();
    let is_clicked = input.is_mouse_button_pressed(0); // 0 is left click

    // Note: This is a simplified check that doesn't account for z-index or hierarchy properly yet.
    // In a real UI system, we would need to walk the tree from front to back.
    for (_, (node, mut interaction)) in interactions.iter_mut() {
        let is_hovered = point_in_node(mouse_pos, node);

        if is_hovered {
            if is_clicked {
                *interaction = Interaction::Pressed;
            } else {
                *interaction = Interaction::Hovered;
            }
        } else {
            *interaction = Interaction::None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_math::Vec2;

    fn node(pos: (f32, f32), size: (f32, f32)) -> Node {
        Node {
            position: Vec2::new(pos.0, pos.1),
            size: Vec2::new(size.0, size.1),
        }
    }

    #[test]
    fn shared_edge_belongs_to_exactly_one_node() {
        // Element A spans x=[0,100), element B spans x=[100,200); both share the
        // edge at x=100. The point on that edge must hover exactly one element.
        let a = node((0.0, 0.0), (100.0, 50.0));
        let b = node((100.0, 0.0), (100.0, 50.0));

        let point = (100.0, 25.0);
        assert!(!point_in_node(point, &a), "left node must not claim the shared right edge");
        assert!(point_in_node(point, &b), "right node owns its left edge");
    }

    #[test]
    fn interior_and_lower_bounds_are_inclusive() {
        let n = node((10.0, 20.0), (30.0, 40.0));
        // Top-left corner (inclusive lower bound) is inside.
        assert!(point_in_node((10.0, 20.0), &n));
        // Interior point is inside.
        assert!(point_in_node((25.0, 40.0), &n));
        // Exclusive upper bounds are outside.
        assert!(!point_in_node((40.0, 40.0), &n));
        assert!(!point_in_node((25.0, 60.0), &n));
    }

    #[test]
    fn hit_test_requires_both_axes_inside() {
        // A hit is the logical AND of the two per-axis intervals: being inside on
        // one axis but outside on the other must never register as a hit.
        let n = node((0.0, 0.0), (100.0, 100.0));
        assert!(!point_in_node((50.0, 150.0), &n), "x inside, y above upper");
        assert!(!point_in_node((150.0, 50.0), &n), "y inside, x past right");
        assert!(!point_in_node((-1.0, 50.0), &n), "y inside, x below left");
        assert!(!point_in_node((50.0, -1.0), &n), "x inside, y below top");
        // Only when both axes are inside is it a hit.
        assert!(point_in_node((50.0, 50.0), &n));
    }

    #[test]
    fn degenerate_boxes_contain_no_point() {
        // With the half-open [pos, pos+size) rule a zero-size box is empty: its
        // own corner is excluded because the upper bound equals the lower bound.
        let empty = node((30.0, 40.0), (0.0, 0.0));
        assert!(!point_in_node((30.0, 40.0), &empty));
        assert!(!point_in_node((29.999, 40.0), &empty));
        // A box degenerate on a single axis is empty too, even on-axis.
        let flat = node((0.0, 0.0), (100.0, 0.0)); // zero height
        assert!(!point_in_node((50.0, 0.0), &flat));
        let thin = node((0.0, 0.0), (0.0, 100.0)); // zero width
        assert!(!point_in_node((0.0, 50.0), &thin));
    }

    #[test]
    fn negative_positions_are_hit_tested_correctly() {
        // Node spanning [-50, 50) on both axes; the math must not assume the
        // origin is non-negative.
        let n = node((-50.0, -50.0), (100.0, 100.0));
        assert!(point_in_node((-50.0, -50.0), &n), "inclusive lower corner");
        assert!(point_in_node((-25.0, 0.0), &n));
        assert!(point_in_node((49.9, 49.9), &n));
        assert!(!point_in_node((50.0, 0.0), &n), "exclusive upper x");
        assert!(!point_in_node((0.0, 50.0), &n), "exclusive upper y");
        assert!(!point_in_node((-51.0, 0.0), &n), "below lower x");
    }

    #[test]
    fn adjacent_cells_tile_the_plane_as_a_partition() {
        // A 2x2 grid of 10x10 cells covering [0,20) x [0,20). The half-open
        // interval rule guarantees a *partition*: every point in the covered
        // region belongs to EXACTLY one cell — no gaps on shared edges, no
        // double-coverage. This is the core reason the interval is half-open.
        let cells = [
            node((0.0, 0.0), (10.0, 10.0)),
            node((10.0, 0.0), (10.0, 10.0)),
            node((0.0, 10.0), (10.0, 10.0)),
            node((10.0, 10.0), (10.0, 10.0)),
        ];
        for i in 0..20 {
            for j in 0..20 {
                let p = (i as f32, j as f32);
                let hits = cells.iter().filter(|c| point_in_node(p, c)).count();
                assert_eq!(hits, 1, "point {p:?} must land in exactly one cell (got {hits})");
            }
        }
        // Points just past the far edges belong to no cell (upper bound exclusive).
        assert_eq!(cells.iter().filter(|c| point_in_node((20.0, 5.0), c)).count(), 0);
        assert_eq!(cells.iter().filter(|c| point_in_node((5.0, 20.0), c)).count(), 0);
    }
}
