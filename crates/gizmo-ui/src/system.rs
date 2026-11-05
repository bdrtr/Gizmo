use gizmo_core::query::{Query, Mut};
use gizmo_core::system::{Res, ResMut};
use gizmo_core::window::WindowInfo;
use gizmo_core::component::{Children, Parent};
use crate::components::{Style, Node};
use crate::layout::UiContext;

/// Where a root node goes in the window: the origin, unless it is
/// [`PositionType::Absolute`](crate::components::PositionType::Absolute) — then its `left`/`top`
/// (or, when those are `Auto`, its `right`/`bottom` measured from the far edge), in pixels or as a
/// percentage of the window.
///
/// Before this an absolutely-positioned root ignored its `inset` and sat at the top-left like
/// every other root, so a dialog or a HUD corner panel could not be placed (`ui_layout`'s window,
/// 2026-11-02).
fn root_origin(style: &Style, size: gizmo_math::Vec2, window: gizmo_math::Vec2) -> gizmo_math::Vec2 {
    use crate::components::{PositionType, Val};
    if style.position_type != PositionType::Absolute {
        return gizmo_math::Vec2::ZERO;
    }
    let resolve = |v: Val, extent: f32| match v {
        Val::Px(p) => Some(p),
        Val::Percent(p) => Some(p / 100.0 * extent),
        _ => None,
    };
    let axis = |near: Val, far: Val, extent: f32, own: f32| {
        resolve(near, extent).or_else(|| resolve(far, extent).map(|f| extent - f - own)).unwrap_or(0.0)
    };
    gizmo_math::Vec2::new(
        axis(style.inset.left, style.inset.right, window.x, size.x),
        axis(style.inset.top, style.inset.bottom, window.y, size.y),
    )
}

/// An absolutely positioned root with both insets of an axis set and that axis's size `Auto`
/// spans the window between them, as an absolute box does inside a parent (CSS, and taffy for
/// every node but the root). taffy lays a root out with no containing block, so its insets never
/// sized it: `top: 0, bottom: 0` with no height came out **0 px tall** — a touch button meant to
/// cover half the screen covered none of it (`pinbol`, 2026-11-05). `root_origin` already placed
/// such a root by its insets; this gives it the size the same insets imply.
fn stretched_root(style: &Style, window: gizmo_math::Vec2) -> Style {
    use crate::components::Val;
    let resolve = |v: Val, extent: f32| match v {
        Val::Px(p) => Some(p),
        Val::Percent(p) => Some(p / 100.0 * extent),
        _ => None,
    };
    let span = |size: Val, near: Val, far: Val, extent: f32| match (size, resolve(near, extent), resolve(far, extent)) {
        (Val::Auto, Some(a), Some(b)) => Val::Px((extent - a - b).max(0.0)),
        _ => size,
    };
    Style {
        width: span(style.width, style.inset.left, style.inset.right, window.x),
        height: span(style.height, style.inset.top, style.inset.bottom, window.y),
        ..*style
    }
}

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
    mut scrolls: Query<Mut<crate::components::Scroll>>,
    scale: Option<ResMut<crate::scale::UiScale>>,
    moved: Query<&crate::animate::UiTransform>,
    display: Option<Res<gizmo_core::window::DisplayScale>>,
) {
    // Track the real (resized) window size so root layout uses the actual available
    // space instead of the construction-time default (which was never updated).
    //
    // In *layout* pixels: the window divided by the `UiScale`. Every `Val::Px` is a layout pixel,
    // so dividing the space once here and multiplying each box once on the way out (step 5)
    // scales them all without touching a single `Style`.
    let window_px = gizmo_math::Vec2::new(window.width, window.height);
    let density = display.map_or(1.0, |d| d.get());
    let s = match scale {
        Some(mut scale) => {
            let f = scale.factor_for(window_px);
            if scale.factor() != f {
                scale.set_factor(f);
            }
            scale.on_display(density)
        }
        None => density,
    };
    ctx.window_size = window_px / s;
    let mut current_entities = std::collections::HashSet::new();

    // 1. Ensure every entity with a Style has a layout node carrying that style.
    //    A child of a `Scroll` node never shrinks to fit it: overflowing is the point, and with
    //    flexbox's default `flex_shrink: 1` five 40 px rows in a 100 px list would come out
    //    20 px tall and there would be nothing to scroll. (CSS has the same trap and makes you
    //    write `flex-shrink: 0`; a list that has to be told not to squash its rows is no list.)
    for (entity, style) in styles.iter() {
        current_entities.insert(entity);
        // (`get_mut` only to ask: a `Mut` marks a change on write, not on fetch.)
        let in_scroll = match parents.get(entity) {
            Some(p) => scrolls.get_mut(p.0).is_some(),
            None => false,
        };
        if in_scroll && style.flex_shrink != 0.0 {
            ctx.sync_style(entity, &Style { flex_shrink: 0.0, ..*style });
        } else if parents.get(entity).is_none() && style.position_type == crate::components::PositionType::Absolute {
            let stretched = stretched_root(style, ctx.window_size);
            ctx.sync_style(entity, &stretched);
        } else {
            ctx.sync_style(entity, style);
        }
    }

    // 2. Reclaim the layout nodes of entities that lost their Style (or died).
    ctx.retain_entities(&current_entities);

    // 3. Decide the roots, then mirror the ECS hierarchy into the layout tree from them
    //    downward — CYCLE-SAFELY.
    //
    // A root is any node without a Parent, or without a parent that has a Style.
    // Technically we should check if the parent also has a Style.
    // For simplicity, we assume the ECS hierarchy accurately represents the UI tree.
    let roots: Vec<u32> = current_entities
        .iter()
        .copied()
        .filter(|&entity| parents.get(entity).is_none())
        .collect();

    // This pass used to hand every `Children` list to the layout engine verbatim, and a
    // `Children` loop reaching a root was then an ABORT rather than a hang: the recursion is
    // taffy's, inside `compute_root_layout` below, and nothing between the component and that
    // call checks for a cycle. `add_child` refuses to build one, but `Children` is an ordinary
    // component that `add_component` writes directly and that scene loading writes verbatim
    // out of a file. The write-back walk at the bottom of this function has carried a
    // `visited` set against exactly this since it was written; this pass, thirty lines above
    // it, had none.
    //
    // Descending from the roots with one shared `mirrored` set is what drops the back-edge:
    // an entity is given a parent by whoever reaches it first, so the edge that would close
    // the loop is simply never mirrored, and a diamond's shared child lands under one parent
    // instead of being reparented by whichever list was iterated last.
    //
    // Style'd entities no root reaches are CLEARED rather than left alone. Layout nodes
    // outlive a frame, so a child list mirrored before the cycle appeared would otherwise stay
    // in the tree and put it straight back.
    //
    // ONLY A TRACKED ENTITY MAY CLAIM A CHILD. `set_children` is a no-op for a parent with no
    // layout node (`UiContext::set_children` returns early), so letting an untracked entity
    // claim one would attach nothing while permanently taking that child away from a styled
    // parent that could actually hold it — the child would end the frame parented to nothing,
    // never laid out, and its `Node` overwritten from taffy's zero default. That is reachable
    // whenever an unstyled entity's `Children` happens to name a UI entity, which a scene file
    // is free to say. Before this pass descended the graph it could not happen: the loop was
    // over `styles.iter()`, so only a styled entity ever wrote a child list.
    let mut mirrored: std::collections::HashSet<u32> = roots.iter().copied().collect();
    // The adjacency the mirror actually handed taffy, pruned exactly as taffy received it.
    // Step 5 walks THIS rather than `Children` — see the comment there for why the two must
    // not be allowed to disagree.
    let mut mirrored_children: std::collections::HashMap<u32, Vec<u32>> =
        std::collections::HashMap::new();
    let mut to_mirror = roots.clone();
    while let Some(entity) = to_mirror.pop() {
        if !ctx.is_tracked(entity) {
            continue;
        }
        let kids: Vec<u32> = match children.get(entity) {
            Some(children_comp) => children_comp
                .0
                .iter()
                .copied()
                .filter(|&child| ctx.is_tracked(child) && mirrored.insert(child))
                .collect(),
            None => Vec::new(),
        };
        ctx.set_children(entity, &kids);
        to_mirror.extend(kids.iter().copied());
        mirrored_children.insert(entity, kids);
    }
    for (entity, _) in styles.iter() {
        if !mirrored.contains(&entity) {
            ctx.set_children(entity, &[]);
        }
    }

    // 4. Compute layout for roots.
    for &root in &roots {
        ctx.compute_root_layout(root);
    }

    // 5. Write back ABSOLUTE layout positions. The layout engine's location is
    //    PARENT-RELATIVE, but `Node.position` is documented (and hit-tested in
    //    `ui_interaction_system`) as ABSOLUTE window coordinates. Walk each root's
    //    subtree top-down, accumulating ancestor offsets, so a child laid out at
    //    parent-offset (10,10) under a root at (500,500) gets Node.position
    //    (510,510) — not (10,10) (which would hit-test at the window corner).
    //
    //    IT WALKS THE MIRRORED TREE, NOT `Children`. `local` is measured by taffy against the
    //    node taffy considers this one's parent, so adding it to any OTHER ancestor's origin
    //    produces a `Node.position` that is simply wrong — and `Node.position` is what
    //    `ui_interaction_system` hit-tests, so the box a user can click stops matching the box
    //    they can see. Reading `Children` here made that reliable rather than lucky once step 3
    //    started pruning: step 3 claims a shared child at PUSH time (shallowest reacher wins)
    //    while this walk dedups at POP time, so on `R.Children = [B, A]` with `A.Children = [B]`
    //    the mirror gives B to R and this walk would give B the origin of A.
    // A root sits at the window's top-left — unless it is positioned absolutely, in which case
    // its `inset` places it in the window (taffy always puts the root it lays out at the origin,
    // so this is the one place that can).
    let window_size = ctx.window_size;
    let mut stack: Vec<(u32, gizmo_math::Vec2)> = roots
        .iter()
        .map(|&e| {
            let origin = match (styles.get(e), ctx.relative_layout(e)) {
                (Some(style), Some((size, _))) => root_origin(style, size, window_size),
                _ => gizmo_math::Vec2::ZERO,
            };
            (e, origin)
        })
        .collect();
    while let Some((entity, parent_origin)) = stack.pop() {
        let Some((size, local)) = ctx.relative_layout(entity) else {
            continue;
        };
        // A `UiTransform` moves the box after layout, and its children with it.
        let abs = parent_origin + local + moved.get(entity).map_or(gizmo_math::Vec2::ZERO, |t| t.translation);
        if let Some(mut node) = nodes.get_mut(entity) {
            node.size = size * s;
            node.position = abs * s;
        }
        // No `visited` set: the mirror gives every node exactly one parent and never names a
        // root as anyone's child, so `mirrored_children` is a forest by construction and this
        // walk cannot revisit. The cycle guard that used to live here has moved to the pass
        // that builds the tree, which is the only place it can also keep taffy's own recursion
        // (inside `compute_root_layout`, thirty lines above) off the back-edge.
        if let Some(kids) = mirrored_children.get(&entity) {
            // A scrolling node moves its children by its offset, which is first clamped to how
            // far they reach past its box.
            let mut origin = abs;
            if let Some(mut scroll) = scrolls.get_mut(entity) {
                let reach = kids
                    .iter()
                    .filter_map(|&k| ctx.relative_layout(k))
                    .fold(gizmo_math::Vec2::ZERO, |m, (s, l)| m.max(l + s));
                // The offset is in window pixels (it is moved by wheels, fingers and thumbs, which
                // live there); the layout is in layout pixels.
                let max = (reach + ctx.inner_end(entity) - size).max(gizmo_math::Vec2::ZERO) * s;
                // Only write when something moves, so `Changed<Scroll>` means a real change.
                if scroll.max_offset() != max || scroll.offset != scroll.offset.clamp(gizmo_math::Vec2::ZERO, max) {
                    scroll.fit(max);
                }
                origin = abs - scroll.offset / s;
            }
            for &child_id in kids {
                stack.push((child_id, origin));
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

    /// A container that stacks its children vertically, so a sibling's offset is readable.
    fn column_style(w: f32, h: f32) -> Style {
        Style {
            width: Val::Px(w),
            height: Val::Px(h),
            flex_direction: crate::components::FlexDirection::Column,
            ..Default::default()
        }
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

    /// `Display::Grid`: tracks in pixels and shares, a gap, implicit rows of a fixed height, an
    /// item spanning two columns and one placed on a line. A 330 px container, columns
    /// `100px 1fr 2fr` and a 10 px gap leave 210 px for three shares: 70 and 140.
    #[test]
    fn a_grid_places_its_children_in_tracks() {
        use crate::components::{Display, GridPlacement, GridTrack, GridTracks};
        let (mut world, mut schedule) = make_world();
        set_window(&mut world, 800.0, 600.0);
        let grid = world.spawn();
        world.add_component(
            grid,
            Style {
                display: Display::Grid,
                width: Val::Px(330.0),
                height: Val::Px(300.0),
                grid_template_columns: GridTracks::new(&[GridTrack::Px(100.0), GridTrack::Fr(1.0), GridTrack::Fr(2.0)]),
                grid_auto_rows: GridTrack::Px(40.0),
                column_gap: Val::Px(10.0),
                row_gap: Val::Px(5.0),
                ..Default::default()
            },
        );
        world.add_component(grid, Node::default());
        let mut kids = Vec::new();
        // Cells 0..=3 auto-placed; 4 spans two columns; 5 sits on column line 3 of the next row.
        for i in 0..6 {
            let e = world.spawn();
            let mut style = Style::default();
            if i == 4 {
                style.grid_column = GridPlacement::Span(2);
            }
            if i == 5 {
                style.grid_column = GridPlacement::Line(3);
                style.grid_row = GridPlacement::Line(4);
            }
            world.add_component(e, style);
            world.add_component(e, Node::default());
            world.add_component(e, Parent(grid.id()));
            kids.push(e);
        }
        world.add_component(grid, Children(kids.iter().map(|e| e.id()).collect()));
        schedule.run(&mut world, 0.016);

        let share = (330.0 - 100.0 - 20.0) / 3.0;
        let col_x = [0.0, 110.0, 120.0 + share];
        let col_w = [100.0, share, 2.0 * share];
        let row_y = |r: usize| r as f32 * 45.0;
        // Row 1: 0 1 2; row 2: 3, then 4 spans columns 2–3.
        for (i, (c, r)) in [(0, 0), (1, 0), (2, 0), (0, 1)].into_iter().enumerate() {
            let n = node_of(&world, kids[i]);
            assert_vec2(n.position, col_x[c], row_y(r));
            assert_vec2(n.size, col_w[c], 40.0);
        }
        let span = node_of(&world, kids[4]);
        assert_vec2(span.position, col_x[1], row_y(1));
        assert_vec2(span.size, col_w[1] + 10.0 + col_w[2], 40.0);
        let placed = node_of(&world, kids[5]);
        assert_vec2(placed.position, col_x[2], row_y(3));
    }

    /// Explicit rows (pixels and a percentage of the container) and an item placed on a line and
    /// spanning from it: 200 px tall, rows `30px 50%` → 30 and 100; columns `25% 25% 50%` of 400.
    #[test]
    fn grid_rows_percentages_and_a_line_with_a_span() {
        use crate::components::{Display, GridPlacement, GridTrack, GridTracks};
        let (mut world, mut schedule) = make_world();
        set_window(&mut world, 800.0, 600.0);
        let grid = world.spawn();
        world.add_component(
            grid,
            Style {
                display: Display::Grid,
                width: Val::Px(400.0),
                height: Val::Px(200.0),
                grid_template_columns: GridTracks::new(&[GridTrack::Percent(25.0), GridTrack::Percent(25.0), GridTrack::Percent(50.0)]),
                grid_template_rows: GridTracks::new(&[GridTrack::Px(30.0), GridTrack::Percent(50.0)]),
                ..Default::default()
            },
        );
        world.add_component(grid, Node::default());
        let item = world.spawn();
        world.add_component(
            item,
            Style { grid_column: GridPlacement::LineSpan(2, 2), grid_row: GridPlacement::Line(2), ..Default::default() },
        );
        world.add_component(item, Node::default());
        world.add_component(item, Parent(grid.id()));
        world.add_component(grid, Children(vec![item.id()]));
        schedule.run(&mut world, 0.016);
        let n = node_of(&world, item);
        assert_vec2(n.position, 100.0, 30.0);
        assert_vec2(n.size, 300.0, 100.0);
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

    /// An absolutely-positioned root is placed by its inset — `left`/`top` in pixels or percent,
    /// or `right`/`bottom` from the far edge — and a static root still sits at the origin.
    #[test]
    fn an_absolute_root_is_placed_by_its_inset() {
        use crate::components::{PositionType, UiRect};
        let (mut world, mut schedule) = make_world();
        set_window(&mut world, 800.0, 600.0);
        let spawn = |world: &mut World, inset: UiRect, position_type: PositionType| {
            let e = world.spawn();
            world.add_component(
                e,
                Style { position_type, inset, width: Val::Px(100.0), height: Val::Px(50.0), ..Default::default() },
            );
            world.add_component(e, Node::default());
            e
        };
        let near = spawn(&mut world, UiRect { left: Val::Px(300.0), top: Val::Percent(10.0), right: Val::Auto, bottom: Val::Auto }, PositionType::Absolute);
        let far = spawn(&mut world, UiRect { left: Val::Auto, top: Val::Auto, right: Val::Px(20.0), bottom: Val::Px(30.0) }, PositionType::Absolute);
        let flow = spawn(&mut world, UiRect { left: Val::Px(300.0), top: Val::Px(300.0), right: Val::Auto, bottom: Val::Auto }, PositionType::Relative);
        schedule.run(&mut world, 0.016);
        assert_vec2(node_of(&world, near).position, 300.0, 60.0);
        assert_vec2(node_of(&world, far).position, 800.0 - 20.0 - 100.0, 600.0 - 30.0 - 50.0);
        assert_vec2(node_of(&world, flow).position, 0.0, 0.0);
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
    fn an_absolute_root_with_both_insets_spans_the_window_between_them() {
        let (mut world, mut schedule) = make_world();
        set_window(&mut world, 800.0, 600.0);
        let e = world.spawn();
        world.add_component(
            e,
            Style {
                position_type: crate::components::PositionType::Absolute,
                inset: UiRect { left: Val::Px(400.0), right: Val::Px(0.0), top: Val::Percent(10.0), bottom: Val::Px(0.0) },
                ..Default::default()
            },
        );
        world.add_component(e, Node::default());
        schedule.run(&mut world, 0.016);
        let n = node_of(&world, e);
        // Right half, below the top tenth: 400 wide, 540 tall, at (400, 60).
        assert_vec2(n.size, 400.0, 540.0);
        assert_vec2(n.position, 400.0, 60.0);
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

    /// A [`Scroll`](crate::components::Scroll) node moves its children up by its offset, clamped
    /// to how far they reach past its box — and its own box stays put.
    #[test]
    fn a_scroll_node_moves_its_children_by_its_clamped_offset() {
        use crate::components::Scroll;
        let (mut world, mut schedule) = make_world();
        let list = world.spawn();
        world.add_component(list, column_style(100.0, 100.0));
        world.add_component(list, Node::default());
        world.add_component(list, Scroll::default());
        let rows: Vec<Entity> = (0..5)
            .map(|_| {
                let r = world.spawn();
                world.add_component(r, sized_style(100.0, 40.0));
                world.add_component(r, Node::default());
                world.add_component(r, Parent(list.id()));
                r
            })
            .collect();
        world.add_component(list, Children(rows.iter().map(|r| r.id()).collect()));

        schedule.run(&mut world, 0.016);
        // Five 40 px rows in a 100 px box: they reach 200, so it can scroll 100.
        let max = world.borrow::<Scroll>().get(list.id()).unwrap().max_offset();
        assert_vec2(max, 0.0, 100.0);
        assert_vec2(node_of(&world, rows[1]).position, 0.0, 40.0);

        world.borrow_mut::<Scroll>().get_mut(list.id()).unwrap().offset.y = 30.0;
        schedule.run(&mut world, 0.016);
        assert_vec2(node_of(&world, rows[1]).position, 0.0, 10.0);
        assert_vec2(node_of(&world, rows[0]).position, 0.0, -30.0);
        assert_vec2(node_of(&world, list).position, 0.0, 0.0);

        // Past the end is clamped: the last row's bottom meets the box's bottom.
        world.borrow_mut::<Scroll>().get_mut(list.id()).unwrap().offset.y = 500.0;
        schedule.run(&mut world, 0.016);
        assert_eq!(world.borrow::<Scroll>().get(list.id()).unwrap().offset.y, 100.0);
        assert_vec2(node_of(&world, rows[4]).position, 0.0, 60.0);

        // With padding, the end keeps the bottom padding below the last row, as the start keeps
        // the top padding above the first: 10 + 200 + 10 in a 100 px box scrolls 120. (And the
        // 100 px rows now overflow the 80 px inside sideways, by 20.)
        world.borrow_mut::<Style>().get_mut(list.id()).unwrap().padding = UiRect::all(Val::Px(10.0));
        world.borrow_mut::<Scroll>().get_mut(list.id()).unwrap().offset.y = 500.0;
        schedule.run(&mut world, 0.016);
        assert_vec2(world.borrow::<Scroll>().get(list.id()).unwrap().max_offset(), 20.0, 120.0);
        assert_vec2(node_of(&world, rows[4]).position, 10.0, 50.0);
    }

    /// A `UiScale` multiplies every layout pixel — sizes, padding, position — and a `fit` scale
    /// follows the window; a percentage is still of the window. A scroll offset stays in window
    /// pixels, its extent scaled with the content.
    #[test]
    fn a_ui_scale_multiplies_the_layout_into_window_pixels() {
        use crate::components::Scroll;
        use crate::scale::UiScale;
        let (mut world, mut schedule) = make_world();
        set_window(&mut world, 1000.0, 1000.0);
        world.insert_resource(UiScale::fixed(2.0));
        let root = world.spawn();
        world.add_component(root, padded_style(100.0, 300.0, 10.0, 20.0));
        world.add_component(root, Node::default());
        let child = world.spawn();
        world.add_component(child, sized_style(30.0, 40.0));
        world.add_component(child, Node::default());
        world.add_component(child, Parent(root.id()));
        world.add_component(root, Children(vec![child.id()]));
        let wide = world.spawn();
        world.add_component(wide, Style { width: Val::Percent(100.0), height: Val::Px(5.0), ..Default::default() });
        world.add_component(wide, Node::default());
        schedule.run(&mut world, 0.016);
        assert_vec2(node_of(&world, root).size, 200.0, 600.0);
        assert_vec2(node_of(&world, child).position, 20.0, 40.0);
        assert_vec2(node_of(&world, child).size, 60.0, 80.0);
        assert_vec2(node_of(&world, wide).size, 1000.0, 10.0);

        // Designed for 1280×720, shown in 640×720: half size, and the factor is readable.
        world.insert_resource(UiScale::fit(1280.0, 720.0));
        set_window(&mut world, 640.0, 720.0);
        schedule.run(&mut world, 0.016);
        assert_eq!(world.get_resource::<UiScale>().unwrap().factor(), 0.5);
        assert_vec2(node_of(&world, root).size, 50.0, 150.0);
        assert_vec2(node_of(&world, child).position, 5.0, 10.0);
        assert_vec2(node_of(&world, wide).size, 640.0, 2.5);

        // A list at ×2: five 40 px rows in a 100 px box reach 200, so it scrolls 100 layout
        // pixels = 200 window pixels; an offset of 50 window pixels moves the rows by 50.
        world.insert_resource(UiScale::fixed(2.0));
        let list = world.spawn();
        world.add_component(list, column_style(100.0, 100.0));
        world.add_component(list, Node::default());
        world.add_component(list, Scroll::default());
        let rows: Vec<Entity> = (0..5)
            .map(|_| {
                let r = world.spawn();
                world.add_component(r, sized_style(100.0, 40.0));
                world.add_component(r, Node::default());
                world.add_component(r, Parent(list.id()));
                r
            })
            .collect();
        world.add_component(list, Children(rows.iter().map(|r| r.id()).collect()));
        schedule.run(&mut world, 0.016);
        assert_vec2(world.borrow::<Scroll>().get(list.id()).unwrap().max_offset(), 0.0, 200.0);
        world.borrow_mut::<Scroll>().get_mut(list.id()).unwrap().offset.y = 50.0;
        schedule.run(&mut world, 0.016);
        assert_vec2(node_of(&world, rows[1]).position, 0.0, 30.0);
    }

    /// A `UiTransform` moves a box after layout — its children with it, its siblings not, scaled
    /// like every layout pixel — and a `UiTween` drives it, giving a bare element its transform.
    #[test]
    fn a_ui_transform_moves_the_box_and_its_children_and_a_tween_drives_it() {
        use crate::animate::{UiTransform, UiTween};
        let (mut world, mut schedule) = make_world();
        let root = world.spawn();
        world.add_component(root, column_style(300.0, 300.0));
        world.add_component(root, Node::default());
        let card = world.spawn();
        world.add_component(card, padded_style(100.0, 100.0, 10.0, 10.0));
        world.add_component(card, Node::default());
        world.add_component(card, Parent(root.id()));
        let label = world.spawn();
        world.add_component(label, sized_style(20.0, 20.0));
        world.add_component(label, Node::default());
        world.add_component(label, Parent(card.id()));
        world.add_component(card, Children(vec![label.id()]));
        let next = world.spawn();
        world.add_component(next, sized_style(100.0, 50.0));
        world.add_component(next, Node::default());
        world.add_component(next, Parent(root.id()));
        world.add_component(root, Children(vec![card.id(), next.id()]));

        world.add_component(card, UiTransform { translation: Vec2::new(40.0, 5.0), opacity: 1.0 });
        schedule.run(&mut world, 0.016);
        assert_vec2(node_of(&world, card).position, 40.0, 5.0);
        assert_vec2(node_of(&world, label).position, 50.0, 15.0);
        assert_vec2(node_of(&world, next).position, 0.0, 100.0);

        world.insert_resource(crate::UiScale::fixed(2.0));
        schedule.run(&mut world, 0.016);
        assert_vec2(node_of(&world, label).position, 100.0, 30.0);
        world.insert_resource(crate::UiScale::fixed(1.0));

        // A tween on an element with no transform: it is given one, then moves it.
        let mut time = gizmo_core::time::Time::default();
        time.update(0.25);
        world.insert_resource(time);
        world.add_component(next, UiTween::translation(Vec2::new(100.0, 0.0), 0.5));
        schedule.run(&mut world, 0.016);
        assert!(world.borrow::<UiTransform>().get(next.id()).is_some(), "no transform was added");
        schedule.run(&mut world, 0.016);
        assert_vec2(node_of(&world, next).position, 50.0, 100.0);
        schedule.run(&mut world, 0.016);
        assert_vec2(node_of(&world, next).position, 100.0, 100.0);
        assert!(world.borrow::<UiTween>().get(next.id()).unwrap().is_finished());
    }

    /// A `Children` cycle must not reach the layout engine.
    ///
    /// The recursion this guards is not ours. Step 3 mirrors the ECS edges into the layout
    /// tree and `compute_root_layout` walks them, so before the fix a loop hanging off a root
    /// recursed inside taffy until the stack was gone — and a stack overflow in Rust is an
    /// immediate ABORT, with no unwind and no `catch_unwind`. That is also why this test needs
    /// no deadline harness: the pre-fix failure kills the test binary, and cargo reports that.
    ///
    /// The cycle is written straight into `Children`, because that is how one arrives:
    /// `add_child` refuses to build one, but scene loading writes a file's parent edges
    /// verbatim, and a UI tree is as loadable as any other.
    #[test]
    fn a_children_cycle_under_a_root_does_not_reach_the_layout_engine() {
        let (mut world, mut schedule) = make_world();
        set_window(&mut world, 800.0, 600.0);

        let root = world.spawn();
        let a = world.spawn();
        let b = world.spawn();
        for e in [root, a, b] {
            world.add_component(e, sized_style(100.0, 50.0));
            world.add_component(e, Node::default());
        }
        // root -> a -> b -> root. Only `root` is parentless, so it is the only root and the
        // descent has to reach the loop through it.
        world.add_component(a, Parent(root.id()));
        world.add_component(b, Parent(a.id()));
        world.add_component(root, Children(vec![a.id()]));
        world.add_component(a, Children(vec![b.id()]));
        world.add_component(b, Children(vec![root.id()]));

        schedule.run(&mut world, 0.016);

        // Terminating at all is the assertion. The sizes are the evidence that the tree was
        // still laid out rather than abandoned — and `b`, on the far side of the back-edge,
        // is the one a guard that stopped too early would leave at `Node::default()`.
        assert_vec2(node_of(&world, root).size, 100.0, 50.0);
        assert_vec2(node_of(&world, a).size, 100.0, 50.0);
        assert_vec2(node_of(&world, b).size, 100.0, 50.0);
    }

    /// `Node.position` must be built from the parent **taffy** gave the node, not from whichever
    /// ancestor a second walk happens to reach it through.
    ///
    /// Step 3 claims a shared child at PUSH time, so the shallowest reacher wins; the write-back
    /// used to read `Children` and dedup at POP time, which on this fixture resolves the same
    /// child to a different parent. `local` is measured by taffy against the parent it actually
    /// has, so adding it to the other ancestor's origin puts `Node.position` somewhere the box
    /// is not — and `Node.position` is what `ui_interaction_system` hit-tests, so the clickable
    /// rectangle stops matching the visible one.
    #[test]
    fn the_write_back_uses_the_parent_taffy_actually_gave_the_node() {
        let (mut world, mut schedule) = make_world();
        set_window(&mut world, 800.0, 600.0);

        let root = world.spawn();
        let a = world.spawn();
        let b = world.spawn();
        world.add_component(root, column_style(300.0, 300.0));
        for e in [a, b] {
            world.add_component(e, sized_style(100.0, 50.0));
        }
        for e in [root, a, b] {
            world.add_component(e, Node::default());
        }
        world.add_component(a, Parent(root.id()));
        world.add_component(b, Parent(a.id()));
        // `root` names BOTH of them and lists `b` first; `a` also names `b`. The mirror pops
        // `a` before `b` (LIFO), so the pre-fix write-back reached `b` through `a` and added
        // `b`'s root-relative offset to `a`'s origin.
        world.add_component(root, Children(vec![b.id(), a.id()]));
        world.add_component(a, Children(vec![b.id()]));

        schedule.run(&mut world, 0.016);

        // Column: taffy stacks the root's two children, `b` first.
        assert_vec2(node_of(&world, b).position, 0.0, 0.0);
        assert_vec2(node_of(&world, a).position, 0.0, 50.0);
    }

    /// An entity with no `Style` has no layout node, so it can parent nothing — and must not be
    /// able to take a child away from a styled parent that can.
    ///
    /// `UiContext::set_children` returns early for an untracked parent, so the claim attaches
    /// nothing; before the guard it still consumed the child, leaving it parented to nothing,
    /// never laid out, and its `Node` written from taffy's zero default. An unstyled entity
    /// whose `Children` names a UI entity is something a scene file is free to say.
    #[test]
    fn an_unstyled_entity_cannot_take_a_child_out_of_the_layout_tree() {
        let (mut world, mut schedule) = make_world();
        set_window(&mut world, 800.0, 600.0);

        let root = world.spawn();
        let styled_parent = world.spawn();
        let unstyled = world.spawn();
        let child = world.spawn();

        world.add_component(root, column_style(400.0, 400.0));
        world.add_component(styled_parent, padded_style(200.0, 100.0, 10.0, 20.0));
        world.add_component(child, sized_style(50.0, 25.0));
        for e in [root, styled_parent, child] {
            world.add_component(e, Node::default());
        }
        // `unstyled` gets NO Style — it never enters `current_entities` and has no layout node.
        world.add_component(styled_parent, Parent(root.id()));
        world.add_component(unstyled, Parent(root.id()));
        world.add_component(child, Parent(styled_parent.id()));

        // Both name `child`. `unstyled` is listed second, so the mirror pops it FIRST.
        world.add_component(root, Children(vec![styled_parent.id(), unstyled.id()]));
        world.add_component(styled_parent, Children(vec![child.id()]));
        world.add_component(unstyled, Children(vec![child.id()]));

        schedule.run(&mut world, 0.016);

        // A zero size is the pre-fix signature: taffy answers for a node it never computed, so
        // the orphaned child's `Node` is written from the default layout rather than left alone.
        assert_vec2(node_of(&world, child).size, 50.0, 25.0);
        assert_vec2(node_of(&world, child).position, 10.0, 20.0);
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
