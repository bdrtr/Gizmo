use gizmo_core::query::{Query, Mut};
use gizmo_core::system::{Res, ResMut};
use gizmo_core::window::WindowInfo;
use gizmo_core::component::{Children, Parent};
use crate::components::{Style, Node};
use crate::layout::UiContext;
use taffy::{AvailableSpace, Size};

/// System that syncs UI entities into the taffy tree, computes layout for each
/// root, and writes the results back into the [`Node`] components.
pub fn ui_layout_system(
    mut ctx: ResMut<UiContext>,
    window: Res<WindowInfo>,
    styles: Query<&Style>,
    parents: Query<&Parent>,
    children: Query<&Children>,
    mut nodes: Query<Mut<Node>>,
) {
    // Track the real (resized) window size so root layout uses the actual available
    // space instead of the construction-time default (which was never updated).
    ctx.window_size = gizmo_math::Vec2::new(window.width, window.height);
    let mut current_entities = std::collections::HashSet::new();

    // 1. Ensure all entities with a Style have a Taffy node
    for (entity, style) in styles.iter() {
        current_entities.insert(entity);

        if !ctx.entity_to_node.contains_key(&entity) {
            // `new_leaf` returns a TaffyResult; on the rare allocation/insertion
            // failure we skip this entity for this frame instead of panicking the
            // whole frame loop. The entity will simply be retried next frame.
            // This keeps the policy consistent with the other taffy calls below
            // (`set_style`, `set_children`, `remove`, `compute_layout`) which all
            // swallow their errors.
            match ctx.taffy.new_leaf(style.0.clone()) {
                Ok(node_id) => {
                    ctx.entity_to_node.insert(entity, node_id);
                }
                Err(_) => {
                    // Skip this entity this frame; layout write-back below is
                    // guarded by `entity_to_node.get`, so a missing node is safe.
                    continue;
                }
            }
        } else {
            // Update style if it changed (always updating for simplicity here)
            let node_id = ctx.entity_to_node[&entity];
            let _ = ctx.taffy.set_style(node_id, style.0.clone());
        }
    }

    // 2. Remove deleted entities from Taffy
    let mut to_remove = Vec::new();
    for (&entity, &node_id) in ctx.entity_to_node.iter() {
        if !current_entities.contains(&entity) {
            to_remove.push((entity, node_id));
        }
    }
    for (entity, node_id) in to_remove {
        let _ = ctx.taffy.remove(node_id);
        ctx.entity_to_node.remove(&entity);
    }

    // 3. Update parent-child hierarchy in Taffy
    for (entity, _) in styles.iter() {
        if let Some(&node_id) = ctx.entity_to_node.get(&entity) {
            if let Some(children_comp) = children.get(entity) {
                let mut taffy_children = Vec::new();
                for &child_id in &children_comp.0 {
                    if let Some(&v) = ctx.entity_to_node.get(&child_id) {
                        taffy_children.push(v);
                    }
                }
                let _ = ctx.taffy.set_children(node_id, &taffy_children);
            } else {
                let _ = ctx.taffy.set_children(node_id, &[]);
            }
        }
    }

    // 4. Compute layout for roots
    // A root is any node without a Parent, or without a parent that has a Style
    let roots: Vec<_> = current_entities.iter()
        .filter_map(|&entity| {
            if parents.get(entity).is_none() {
                // Use `get` rather than indexing: an entity may be present in
                // `current_entities` but missing from `entity_to_node` if its
                // `new_leaf` allocation failed earlier this frame.
                ctx.entity_to_node.get(&entity).copied()
            } else {
                // Technically we should check if the parent also has a Style.
                // For simplicity, we assume the ECS hierarchy accurately represents the UI tree.
                None
            }
        }).collect();

    let available_space = Size {
        width: AvailableSpace::Definite(ctx.window_size.x),
        height: AvailableSpace::Definite(ctx.window_size.y),
    };

    for &root_node in &roots {
        let _ = ctx.taffy.compute_layout(root_node, available_space);
    }

    // 5. Write back ABSOLUTE layout positions. taffy's `layout.location` is
    //    PARENT-RELATIVE, but `Node.position` is documented (and hit-tested in
    //    `ui_interaction_system`) as ABSOLUTE window coordinates. Walk each root's
    //    subtree top-down, accumulating ancestor offsets, so a child laid out at
    //    parent-offset (10,10) under a root at (500,500) gets Node.position
    //    (510,510) — not (10,10) (which would hit-test at the window corner).
    let mut stack: Vec<(u32, gizmo_math::Vec2)> = current_entities
        .iter()
        .copied()
        .filter(|&e| parents.get(e).is_none())
        .map(|e| (e, gizmo_math::Vec2::ZERO))
        .collect();
    let mut visited = std::collections::HashSet::new();
    while let Some((entity, parent_origin)) = stack.pop() {
        if !visited.insert(entity) {
            continue; // guard against a Children cycle
        }
        let Some(&node_id) = ctx.entity_to_node.get(&entity) else {
            continue;
        };
        let (size, local) = match ctx.taffy.layout(node_id) {
            Ok(layout) => (
                gizmo_math::Vec2::new(layout.size.width, layout.size.height),
                gizmo_math::Vec2::new(layout.location.x, layout.location.y),
            ),
            Err(_) => continue,
        };
        let abs = parent_origin + local;
        if let Some(mut node) = nodes.get_mut(entity) {
            node.size = size;
            node.position = abs;
        }
        if let Some(children_comp) = children.get(entity) {
            for &child_id in &children_comp.0 {
                stack.push((child_id, abs));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::components::{Style, Node, Interaction};
    use crate::layout::UiContext;
    use gizmo_core::component::{Parent, Children};
    use gizmo_core::entity::Entity;
    use gizmo_core::input::Input;
    use gizmo_core::system::Schedule;
    use gizmo_core::window::WindowInfo;
    use gizmo_core::world::World;
    use gizmo_math::Vec2;
    use taffy::geometry::{Rect, Size};
    use taffy::style::{Dimension, LengthPercentage, Style as TaffyStyle};

    /// Builds a world with the UI systems registered exactly the way production
    /// does (via `crate::register`), plus the hierarchy component types and an
    /// `Input` resource the interaction system needs.
    fn make_world() -> (World, Schedule) {
        let mut world = World::new();
        let mut schedule = Schedule::new();
        crate::register(&mut world, &mut schedule);
        // The layout system queries Parent/Children; register them so the queries
        // resolve. `register_component_type` is idempotent, so this is safe even
        // if a future `register` starts registering them itself.
        world.register_component_type::<Parent>();
        world.register_component_type::<Children>();
        // register() does not insert an Input; the interaction system is skipped
        // without it, so provide one.
        world.insert_resource(Input::new());
        (world, schedule)
    }

    fn set_window(world: &mut World, w: f32, h: f32) {
        let mut info = world.get_resource_mut::<WindowInfo>().unwrap();
        *info = WindowInfo::new(w, h);
    }

    fn sized_style(w: f32, h: f32) -> Style {
        Style(TaffyStyle {
            size: Size { width: Dimension::length(w), height: Dimension::length(h) },
            ..Default::default()
        })
    }

    /// Style with an explicit size and left/top padding (right/bottom zero).
    fn padded_style(w: f32, h: f32, pad_left: f32, pad_top: f32) -> Style {
        Style(TaffyStyle {
            size: Size { width: Dimension::length(w), height: Dimension::length(h) },
            padding: Rect {
                left: LengthPercentage::length(pad_left),
                right: LengthPercentage::length(0.0),
                top: LengthPercentage::length(pad_top),
                bottom: LengthPercentage::length(0.0),
            },
            ..Default::default()
        })
    }

    fn node_of(world: &World, e: Entity) -> Node {
        *world.borrow::<Node>().get(e.id()).unwrap()
    }

    fn assert_vec2(got: Vec2, x: f32, y: f32) {
        assert!(
            (got.x - x).abs() < 1e-3 && (got.y - y).abs() < 1e-3,
            "expected ({x}, {y}), got ({}, {})",
            got.x,
            got.y
        );
    }

    #[test]
    fn root_is_sized_and_placed_at_window_origin() {
        let (mut world, mut schedule) = make_world();
        let e = world.spawn();
        world.add_component(e, sized_style(120.0, 80.0));
        world.add_component(e, Node::default());

        schedule.run(&mut world, 0.016);

        let n = node_of(&world, e);
        assert_vec2(n.size, 120.0, 80.0);
        // A root (no Parent) is laid out at the top-left of the available space.
        assert_vec2(n.position, 0.0, 0.0);
    }

    #[test]
    fn window_size_drives_percentage_root_layout() {
        let (mut world, mut schedule) = make_world();
        // The layout system copies WindowInfo into the taffy available space each
        // frame; a percentage-sized root must resolve against the real size.
        set_window(&mut world, 800.0, 600.0);
        let e = world.spawn();
        world.add_component(
            e,
            Style(TaffyStyle {
                size: Size {
                    width: Dimension::percent(1.0),
                    height: Dimension::percent(0.5),
                },
                ..Default::default()
            }),
        );
        world.add_component(e, Node::default());

        schedule.run(&mut world, 0.016);

        let n = node_of(&world, e);
        assert_vec2(n.size, 800.0, 300.0);
    }

    #[test]
    fn child_positions_accumulate_ancestor_offsets_into_absolute_coords() {
        // Three-level tree: root -> mid -> leaf. Padding on root and mid pushes
        // each descendant to a non-zero PARENT-RELATIVE offset. taffy stores
        // parent-relative locations; the write-back must accumulate them into
        // ABSOLUTE window coordinates (the contract the hit-test relies on).
        let (mut world, mut schedule) = make_world();

        let root = world.spawn();
        world.add_component(root, padded_style(400.0, 400.0, 100.0, 70.0));
        world.add_component(root, Node::default());

        let mid = world.spawn();
        world.add_component(mid, padded_style(200.0, 200.0, 20.0, 30.0));
        world.add_component(mid, Node::default());
        world.add_component(mid, Parent(root.id()));

        let leaf = world.spawn();
        world.add_component(leaf, sized_style(50.0, 50.0));
        world.add_component(leaf, Node::default());
        world.add_component(leaf, Parent(mid.id()));

        world.add_component(root, Children(vec![mid.id()]));
        world.add_component(mid, Children(vec![leaf.id()]));

        schedule.run(&mut world, 0.016);

        // Read taffy's own parent-relative locations from the shared context and
        // independently reconstruct the expected absolute positions. This keeps
        // the assertion agnostic to taffy's exact flexbox math while still
        // pinning down the accumulation logic under test.
        let (loc_root, loc_mid, loc_leaf) = {
            let ctx = world.get_resource::<UiContext>().unwrap();
            let rel = |e: Entity| {
                let id = ctx.entity_to_node[&e.id()];
                let l = ctx.taffy.layout(id).unwrap();
                Vec2::new(l.location.x, l.location.y)
            };
            (rel(root), rel(mid), rel(leaf))
        };

        let pr = node_of(&world, root).position;
        let pm = node_of(&world, mid).position;
        let pl = node_of(&world, leaf).position;

        // Root: relative == absolute (it has no ancestors).
        assert_vec2(pr, loc_root.x, loc_root.y);
        // Child absolute = parent absolute + child relative.
        let expect_mid = loc_root + loc_mid;
        assert_vec2(pm, expect_mid.x, expect_mid.y);
        // Grandchild accumulates the whole ancestor chain.
        let expect_leaf = loc_root + loc_mid + loc_leaf;
        assert_vec2(pl, expect_leaf.x, expect_leaf.y);

        // Regression guard for the exact bug the write-back comment warns about:
        // padding gave `mid` a non-zero relative offset, so the grandchild's
        // absolute position must be strictly greater than its parent-relative
        // location alone — proving the ancestor offset was added in, not dropped.
        assert!(loc_mid.x > 0.0 && loc_mid.y > 0.0, "padding should offset mid within root");
        assert!(
            pl.x > loc_leaf.x + 1.0 && pl.y > loc_leaf.y + 1.0,
            "grandchild absolute {pl:?} must exceed its parent-relative {loc_leaf:?}"
        );
    }

    #[test]
    fn taffy_node_is_created_then_reclaimed_when_style_removed() {
        let (mut world, mut schedule) = make_world();
        let e = world.spawn();
        world.add_component(e, sized_style(10.0, 10.0));
        world.add_component(e, Node::default());

        schedule.run(&mut world, 0.016);
        {
            let ctx = world.get_resource::<UiContext>().unwrap();
            assert!(ctx.entity_to_node.contains_key(&e.id()), "node mapped after first frame");
            assert_eq!(ctx.entity_to_node.len(), 1);
            assert_eq!(ctx.taffy.total_node_count(), 1);
        }

        // Dropping Style removes the entity from the styled set; the next frame's
        // cleanup pass must evict its taffy node and mapping (no leak).
        world.remove_component::<Style>(e);
        schedule.run(&mut world, 0.016);
        {
            let ctx = world.get_resource::<UiContext>().unwrap();
            assert!(!ctx.entity_to_node.contains_key(&e.id()), "mapping evicted");
            assert!(ctx.entity_to_node.is_empty());
            assert_eq!(ctx.taffy.total_node_count(), 0, "taffy node freed");
        }
    }

    #[test]
    fn interaction_state_machine_tracks_pointer_and_button() {
        let (mut world, mut schedule) = make_world();
        // Button with a directly-set Node (no Style, so the layout system leaves
        // its geometry untouched) spanning [100,150) x [100,130).
        let btn = world.spawn();
        world.add_component(
            btn,
            Node { position: Vec2::new(100.0, 100.0), size: Vec2::new(50.0, 30.0) },
        );
        world.add_component(btn, Interaction::None);

        let interaction = |world: &World| *world.borrow::<Interaction>().get(btn.id()).unwrap();

        // Pointer outside the box -> None.
        world.get_resource_mut::<Input>().unwrap().set_mouse_position(10.0, 10.0);
        schedule.run(&mut world, 0.016);
        assert_eq!(interaction(&world), Interaction::None);

        // Pointer inside, button up -> Hovered.
        world.get_resource_mut::<Input>().unwrap().set_mouse_position(120.0, 110.0);
        schedule.run(&mut world, 0.016);
        assert_eq!(interaction(&world), Interaction::Hovered);

        // Pointer inside, left button down -> Pressed.
        {
            let mut input = world.get_resource_mut::<Input>().unwrap();
            input.set_mouse_position(120.0, 110.0);
            input.on_mouse_button_pressed(0);
        }
        schedule.run(&mut world, 0.016);
        assert_eq!(interaction(&world), Interaction::Pressed);

        // Release and move out -> the state is recomputed from scratch, back to
        // None. (Reset the Input wholesale: without a per-frame begin_frame the
        // "just pressed" latch would otherwise keep the button marked pressed.)
        {
            let mut input = world.get_resource_mut::<Input>().unwrap();
            *input = Input::new();
            input.set_mouse_position(0.0, 0.0);
        }
        schedule.run(&mut world, 0.016);
        assert_eq!(interaction(&world), Interaction::None);
    }

    #[test]
    fn hovering_one_element_does_not_affect_a_disjoint_element() {
        let (mut world, mut schedule) = make_world();
        let a = world.spawn();
        world.add_component(
            a,
            Node { position: Vec2::new(0.0, 0.0), size: Vec2::new(50.0, 50.0) },
        );
        world.add_component(a, Interaction::None);

        let b = world.spawn();
        world.add_component(
            b,
            Node { position: Vec2::new(100.0, 0.0), size: Vec2::new(50.0, 50.0) },
        );
        world.add_component(b, Interaction::None);

        // Pointer over `a` only.
        world.get_resource_mut::<Input>().unwrap().set_mouse_position(25.0, 25.0);
        schedule.run(&mut world, 0.016);

        assert_eq!(*world.borrow::<Interaction>().get(a.id()).unwrap(), Interaction::Hovered);
        assert_eq!(*world.borrow::<Interaction>().get(b.id()).unwrap(), Interaction::None);
    }
}
