use gizmo_core::query::{Query, Mut};
use gizmo_core::system::{Res, ResMut};
use gizmo_core::window::WindowInfo;
use gizmo_core::component::{Children, Parent};
use crate::components::{Style, Node};
use crate::layout::UiContext;

/// System that syncs UI entities into the layout tree, computes layout for each
/// root, and writes the results back into the [`Node`] components.
///
/// No `taffy` type is named here: the tree lives behind [`UiContext`], which is
/// the crate's only boundary with the layout engine.
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

    // 1. Ensure every entity with a Style has a layout node carrying that style.
    for (entity, style) in styles.iter() {
        current_entities.insert(entity);
        ctx.sync_style(entity, style);
    }

    // 2. Reclaim the layout nodes of entities that lost their Style (or died).
    ctx.retain_entities(&current_entities);

    // 3. Mirror the ECS hierarchy into the layout tree.
    for (entity, _) in styles.iter() {
        match children.get(entity) {
            Some(children_comp) => ctx.set_children(entity, &children_comp.0),
            None => ctx.set_children(entity, &[]),
        }
    }

    // 4. Compute layout for roots.
    // A root is any node without a Parent, or without a parent that has a Style.
    let roots: Vec<u32> = current_entities
        .iter()
        .copied()
        .filter(|&entity| {
            // Technically we should check if the parent also has a Style.
            // For simplicity, we assume the ECS hierarchy accurately represents the UI tree.
            parents.get(entity).is_none()
        })
        .collect();

    for &root in &roots {
        ctx.compute_root_layout(root);
    }

    // 5. Write back ABSOLUTE layout positions. The layout engine's location is
    //    PARENT-RELATIVE, but `Node.position` is documented (and hit-tested in
    //    `ui_interaction_system`) as ABSOLUTE window coordinates. Walk each root's
    //    subtree top-down, accumulating ancestor offsets, so a child laid out at
    //    parent-offset (10,10) under a root at (500,500) gets Node.position
    //    (510,510) — not (10,10) (which would hit-test at the window corner).
    let mut stack: Vec<(u32, gizmo_math::Vec2)> =
        roots.iter().map(|&e| (e, gizmo_math::Vec2::ZERO)).collect();
    let mut visited = std::collections::HashSet::new();
    while let Some((entity, parent_origin)) = stack.pop() {
        if !visited.insert(entity) {
            continue; // guard against a Children cycle
        }
        let Some((size, local)) = ctx.relative_layout(entity) else {
            continue;
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
    use crate::components::{Interaction, Node, Style, UiRect, Val};
    use crate::layout::UiContext;
    use gizmo_core::component::{Parent, Children};
    use gizmo_core::entity::Entity;
    use gizmo_core::input::Input;
    use gizmo_core::system::Schedule;
    use gizmo_core::window::WindowInfo;
    use gizmo_core::world::World;
    use gizmo_math::Vec2;

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
        Style { width: Val::Px(w), height: Val::Px(h), ..Default::default() }
    }

    /// Style with an explicit size and left/top padding (right/bottom zero).
    fn padded_style(w: f32, h: f32, pad_left: f32, pad_top: f32) -> Style {
        Style {
            width: Val::Px(w),
            height: Val::Px(h),
            padding: UiRect::new(Val::Px(pad_left), Val::ZERO, Val::Px(pad_top), Val::ZERO),
            ..Default::default()
        }
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
        // The layout system copies WindowInfo into the available space each
        // frame; a percentage-sized root must resolve against the real size.
        set_window(&mut world, 800.0, 600.0);
        let e = world.spawn();
        world.add_component(
            e,
            Style {
                width: Val::Percent(100.0),
                height: Val::Percent(50.0),
                ..Default::default()
            },
        );
        world.add_component(e, Node::default());

        schedule.run(&mut world, 0.016);

        let n = node_of(&world, e);
        // `Val::Percent` is the CSS 0..=100 scale: 100% of 800 and 50% of 600.
        // A conversion that forwarded the number straight to taffy (which wants
        // a 0..=1 fraction) would give a 100x-too-large box here.
        assert_vec2(n.size, 800.0, 300.0);
    }

    #[test]
    fn px_percent_and_auto_widths_are_three_different_layouts() {
        // Pins the length distinction end-to-end, through the real system: the
        // same numeric value means three different things depending on the
        // `Val` variant. `auto` on a childless leaf collapses to zero width.
        let (mut world, mut schedule) = make_world();
        set_window(&mut world, 1000.0, 500.0);

        let spawn = |world: &mut World, width: Val| {
            let e = world.spawn();
            world.add_component(e, Style { width, height: Val::Px(10.0), ..Default::default() });
            world.add_component(e, Node::default());
            e
        };
        let px = spawn(&mut world, Val::Px(40.0));
        let percent = spawn(&mut world, Val::Percent(40.0));
        let auto = spawn(&mut world, Val::Auto);

        schedule.run(&mut world, 0.016);

        assert_vec2(node_of(&world, px).size, 40.0, 10.0);
        assert_vec2(node_of(&world, percent).size, 400.0, 10.0);
        assert_vec2(node_of(&world, auto).size, 0.0, 10.0);
    }

    #[test]
    fn child_positions_accumulate_ancestor_offsets_into_absolute_coords() {
        // Three-level tree: root -> mid -> leaf. Padding on root and mid pushes
        // each descendant to a non-zero PARENT-RELATIVE offset. The layout tree
        // stores parent-relative locations; the write-back must accumulate them
        // into ABSOLUTE window coordinates (the contract the hit-test relies on).
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

        // Read the engine's own parent-relative locations from the shared context
        // and independently reconstruct the expected absolute positions. This
        // keeps the assertion agnostic to the exact flexbox math while still
        // pinning down the accumulation logic under test.
        let (loc_root, loc_mid, loc_leaf) = {
            let ctx = world.get_resource::<UiContext>().unwrap();
            let rel = |e: Entity| ctx.relative_layout(e.id()).unwrap().1;
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
    fn layout_node_is_created_then_reclaimed_when_style_removed() {
        let (mut world, mut schedule) = make_world();
        let e = world.spawn();
        world.add_component(e, sized_style(10.0, 10.0));
        world.add_component(e, Node::default());

        schedule.run(&mut world, 0.016);
        {
            let ctx = world.get_resource::<UiContext>().unwrap();
            assert!(ctx.is_tracked(e.id()), "node mapped after first frame");
            assert_eq!(ctx.tracked_count(), 1);
            assert_eq!(ctx.node_count(), 1);
        }

        // Dropping Style removes the entity from the styled set; the next frame's
        // cleanup pass must evict its layout node and mapping (no leak).
        world.remove_component::<Style>(e);
        schedule.run(&mut world, 0.016);
        {
            let ctx = world.get_resource::<UiContext>().unwrap();
            assert!(!ctx.is_tracked(e.id()), "mapping evicted");
            assert_eq!(ctx.tracked_count(), 0);
            assert_eq!(ctx.node_count(), 0, "layout node freed");
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
