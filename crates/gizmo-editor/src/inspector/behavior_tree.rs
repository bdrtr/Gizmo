//! The behaviour tree section: an agent's tree, shown as the tree it is and edited in place.
//!
//! A tree is data (`gizmo_ai::BehaviorTree`, decided 2026-11-14), so the inspector can do for it
//! what it does for any component: show it, change it, add it from ➕ and remove it. Each node is
//! one row — its kind, what it holds (a leaf's name, a wait's seconds, a repeat's count) and,
//! while the game runs, where it is (a sequence's step, a wait's progress). A leaf naming something
//! the world's `BtLibrary` does not hold is marked, because that leaf will fail.
//!
//! Changing a value (a name, a duration) leaves the running state alone; changing the shape
//! (adding, removing, moving or converting a node) starts the tree afresh (`BtNode::reset`),
//! because a saved position may no longer point at the child it meant.

use crate::editor_state::EditorState;
use gizmo_ai::{BehaviorTree, BtLibrary, BtNode};
use gizmo_core::World;

/// What kind of node a row is — the inspector's choice list, and what a conversion turns into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    /// [`BtNode::Sequence`].
    Sequence,
    /// [`BtNode::Selector`].
    Selector,
    /// [`BtNode::Inverter`].
    Inverter,
    /// [`BtNode::Succeeder`].
    Succeeder,
    /// [`BtNode::Repeat`].
    Repeat,
    /// [`BtNode::Wait`].
    Wait,
    /// [`BtNode::Action`].
    Action,
    /// [`BtNode::Condition`].
    Condition,
}

impl NodeKind {
    /// Every kind, in the order the menus list them.
    pub const ALL: [NodeKind; 8] = [
        Self::Sequence,
        Self::Selector,
        Self::Inverter,
        Self::Succeeder,
        Self::Repeat,
        Self::Wait,
        Self::Action,
        Self::Condition,
    ];

    /// The row's label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Sequence => "→ Sıra",
            Self::Selector => "? Seçici",
            Self::Inverter => "! Ters",
            Self::Succeeder => "✓ Başarılı say",
            Self::Repeat => "↻ Tekrar",
            Self::Wait => "⏱ Bekle",
            Self::Action => "▶ Eylem",
            Self::Condition => "◆ Koşul",
        }
    }

    /// The kind of `node`.
    pub fn of(node: &BtNode) -> Self {
        match node {
            BtNode::Sequence { .. } => Self::Sequence,
            BtNode::Selector { .. } => Self::Selector,
            BtNode::Inverter(_) => Self::Inverter,
            BtNode::Succeeder(_) => Self::Succeeder,
            BtNode::Repeat { .. } => Self::Repeat,
            BtNode::Wait { .. } => Self::Wait,
            BtNode::Condition(_) => Self::Condition,
            // `BtNode` is `#[non_exhaustive]`; a kind this editor does not know is shown as a leaf.
            _ => Self::Action,
        }
    }

    /// A fresh node of this kind: composites empty, decorators over an unnamed action, a one-second
    /// wait, leaves unnamed.
    pub fn fresh(self) -> BtNode {
        match self {
            Self::Sequence => BtNode::sequence([]),
            Self::Selector => BtNode::selector([]),
            Self::Inverter => BtNode::inverter(BtNode::action("")),
            Self::Succeeder => BtNode::succeeder(BtNode::action("")),
            Self::Repeat => BtNode::repeat(BtNode::action(""), 2),
            Self::Wait => BtNode::wait(1.0),
            Self::Action => BtNode::action(""),
            Self::Condition => BtNode::condition(""),
        }
    }
}

/// One change to a tree, raised by a row and applied after the tree is drawn. A path is the child
/// index at each level from the root; a decorator's child is index 0.
#[derive(Debug, Clone, PartialEq)]
pub enum TreeEdit {
    /// Set the root (or clear it).
    SetRoot(Option<BtNode>),
    /// Append `node` to the children of the composite at the path.
    Append(Vec<usize>, BtNode),
    /// Remove the node at the path. A decorator left without a child goes with it; removing the
    /// root empties the tree.
    Remove(Vec<usize>),
    /// Move the node at the path one place earlier (`true`) or later among its siblings.
    Move(Vec<usize>, bool),
    /// Turn the node at the path into another kind, keeping what carries over: a composite's
    /// children, a decorator's child, a leaf's name.
    Convert(Vec<usize>, NodeKind),
}

fn children_mut(node: &mut BtNode) -> Option<&mut Vec<BtNode>> {
    match node {
        BtNode::Sequence { children, .. } | BtNode::Selector { children, .. } => Some(children),
        _ => None,
    }
}

fn child_mut(node: &mut BtNode) -> Option<&mut BtNode> {
    match node {
        BtNode::Inverter(child) | BtNode::Succeeder(child) | BtNode::Repeat { child, .. } => Some(child),
        _ => None,
    }
}

fn is_decorator(node: &BtNode) -> bool {
    matches!(node, BtNode::Inverter(_) | BtNode::Succeeder(_) | BtNode::Repeat { .. })
}

fn node_at_mut<'a>(node: &'a mut BtNode, path: &[usize]) -> Option<&'a mut BtNode> {
    let Some((&first, rest)) = path.split_first() else {
        return Some(node);
    };
    let next = if is_decorator(node) {
        if first != 0 {
            return None;
        }
        child_mut(node)?
    } else {
        children_mut(node)?.get_mut(first)?
    };
    node_at_mut(next, rest)
}

/// `node` turned into `kind`, keeping what carries over.
pub fn convert(node: BtNode, kind: NodeKind) -> BtNode {
    if NodeKind::of(&node) == kind {
        return node;
    }
    // What the old node holds, in the shapes the new one can take it in.
    let (children, name): (Vec<BtNode>, Option<String>) = match node {
        BtNode::Sequence { children, .. } | BtNode::Selector { children, .. } => (children, None),
        BtNode::Inverter(child) | BtNode::Succeeder(child) | BtNode::Repeat { child, .. } => (vec![*child], None),
        BtNode::Action(name) | BtNode::Condition(name) => (Vec::new(), Some(name)),
        _ => (Vec::new(), None),
    };
    let one = |children: Vec<BtNode>| -> BtNode {
        let mut children = children.into_iter();
        match (children.next(), children.next()) {
            (Some(only), None) => only,
            (None, _) => BtNode::action(""),
            // More than one child under a one-child decorator: they stay together, in a sequence.
            (Some(first), Some(second)) => BtNode::sequence([first, second].into_iter().chain(children)),
        }
    };
    match kind {
        NodeKind::Sequence => BtNode::sequence(children),
        NodeKind::Selector => BtNode::selector(children),
        NodeKind::Inverter => BtNode::inverter(one(children)),
        NodeKind::Succeeder => BtNode::succeeder(one(children)),
        NodeKind::Repeat => BtNode::repeat(one(children), 2),
        NodeKind::Wait => BtNode::wait(1.0),
        NodeKind::Action => BtNode::action(name.unwrap_or_default()),
        NodeKind::Condition => BtNode::condition(name.unwrap_or_default()),
    }
}

/// Applies `edit` to `tree`. Every edit changes the shape, so the tree's running state is reset.
/// An edit whose path no longer leads anywhere does nothing.
pub fn apply_edit(tree: &mut BehaviorTree, edit: TreeEdit) {
    match edit {
        TreeEdit::SetRoot(root) => tree.root = root,
        TreeEdit::Append(path, node) => {
            if let Some(children) = tree.root.as_mut().and_then(|r| node_at_mut(r, &path)).and_then(children_mut) {
                children.push(node);
            }
        }
        TreeEdit::Remove(mut path) => {
            let Some(root) = tree.root.as_mut() else { return };
            // Climb out of decorators: one cannot stand without its child.
            loop {
                let Some((_, parent)) = path.split_last() else {
                    tree.root = None;
                    return;
                };
                let parent_is_decorator = node_at_mut(root, parent).is_some_and(|p| is_decorator(p));
                if !parent_is_decorator {
                    break;
                }
                path.pop();
            }
            let (&index, parent) = path.split_last().expect("not the root");
            if let Some(children) = node_at_mut(root, parent).and_then(children_mut) {
                if index < children.len() {
                    children.remove(index);
                }
            }
        }
        TreeEdit::Move(path, earlier) => {
            let Some((&index, parent)) = path.split_last() else { return };
            if let Some(children) = tree.root.as_mut().and_then(|r| node_at_mut(r, parent)).and_then(children_mut) {
                let to = if earlier { index.checked_sub(1) } else { Some(index + 1) };
                if let Some(to) = to.filter(|&to| to < children.len() && index < children.len()) {
                    children.swap(index, to);
                }
            }
        }
        TreeEdit::Convert(path, kind) => {
            if let Some(node) = tree.root.as_mut().and_then(|r| node_at_mut(r, &path)) {
                let old = std::mem::replace(node, BtNode::action(""));
                *node = convert(old, kind);
            }
        }
    }
    if let Some(root) = tree.root.as_mut() {
        root.reset();
    }
}

/// What a row needs besides its node.
struct RowContext<'a> {
    library: Option<&'a BtLibrary>,
    id: u64,
    edit: Option<TreeEdit>,
}

fn kind_menu(ui: &mut egui::Ui, label: &str, mut on_pick: impl FnMut(NodeKind)) {
    ui.menu_button(label, |ui| {
        for kind in NodeKind::ALL {
            if ui.button(kind.label()).clicked() {
                on_pick(kind);
                ui.close();
            }
        }
    });
}

fn leaf_name_row(ui: &mut egui::Ui, name: &mut String, names: &[&str], known: Option<bool>, salt: String) {
    egui::ComboBox::from_id_salt(salt)
        .selected_text(if name.is_empty() { "(seç)" } else { name.as_str() })
        .show_ui(ui, |ui| {
            for candidate in names {
                if ui.selectable_label(name == candidate, *candidate).clicked() {
                    *name = (*candidate).to_string();
                }
            }
        });
    ui.add(egui::TextEdit::singleline(name).desired_width(90.0).hint_text("ad"));
    if known == Some(false) {
        ui.label(egui::RichText::new("⚠ kayıtlı değil").color(egui::Color32::from_rgb(230, 90, 70)))
            .on_hover_text("Bu adı oyunun BtLibrary'si tutmuyor: yaprak her seferinde başarısız olur.");
    }
}

fn draw_node(ui: &mut egui::Ui, node: &mut BtNode, path: &mut Vec<usize>, siblings: usize, cx: &mut RowContext) {
    let depth = path.len();
    let kind = NodeKind::of(node);
    ui.horizontal(|ui| {
        ui.add_space(depth as f32 * 14.0);
        let mut picked = kind;
        egui::ComboBox::from_id_salt(("bt_kind", cx.id, path.clone()))
            .selected_text(kind.label())
            .width(110.0)
            .show_ui(ui, |ui| {
                for k in NodeKind::ALL {
                    ui.selectable_value(&mut picked, k, k.label());
                }
            });
        if picked != kind {
            cx.edit = Some(TreeEdit::Convert(path.clone(), picked));
        }

        match node {
            BtNode::Sequence { children, current } | BtNode::Selector { children, current } => {
                if *current > 0 {
                    ui.label(format!("adım {}/{}", *current + 1, children.len()));
                }
                let at = path.clone();
                kind_menu(ui, "+", |k| cx.edit = Some(TreeEdit::Append(at.clone(), k.fresh())));
            }
            BtNode::Repeat { times, done, .. } => {
                let mut forever = times.is_none();
                if ui.checkbox(&mut forever, "sonsuz").changed() {
                    *times = if forever { None } else { Some(2) };
                }
                if let Some(n) = times {
                    ui.add(egui::DragValue::new(n).range(1..=10_000).suffix(" kez"));
                }
                if *done > 0 {
                    ui.label(format!("{done} oldu"));
                }
            }
            BtNode::Wait { secs, elapsed } => {
                ui.add(egui::DragValue::new(secs).speed(0.05).range(0.0..=3600.0).suffix(" sn"));
                if *elapsed > 0.0 {
                    ui.label(format!("{elapsed:.2} sn geçti"));
                }
            }
            BtNode::Action(name) => {
                let names = cx.library.map(|l| l.action_names()).unwrap_or_default();
                let known = cx.library.map(|l| l.action(name).is_some());
                leaf_name_row(ui, name, &names, known, format!("bt_name_{}_{path:?}", cx.id));
            }
            BtNode::Condition(name) => {
                let names = cx.library.map(|l| l.condition_names()).unwrap_or_default();
                let known = cx.library.map(|l| l.condition(name).is_some());
                leaf_name_row(ui, name, &names, known, format!("bt_name_{}_{path:?}", cx.id));
            }
            _ => {}
        }

        if depth > 0 {
            let index = *path.last().expect("not the root");
            if siblings > 1 {
                if ui.small_button("↑").clicked() && index > 0 {
                    cx.edit = Some(TreeEdit::Move(path.clone(), true));
                }
                if ui.small_button("↓").clicked() && index + 1 < siblings {
                    cx.edit = Some(TreeEdit::Move(path.clone(), false));
                }
            }
        }
        if ui.small_button("🗑").clicked() {
            cx.edit = Some(TreeEdit::Remove(path.clone()));
        }
    });

    match node {
        BtNode::Sequence { children, .. } | BtNode::Selector { children, .. } => {
            let count = children.len();
            for (i, child) in children.iter_mut().enumerate() {
                path.push(i);
                draw_node(ui, child, path, count, cx);
                path.pop();
            }
        }
        BtNode::Inverter(child) | BtNode::Succeeder(child) | BtNode::Repeat { child, .. } => {
            path.push(0);
            draw_node(ui, child, path, 1, cx);
            path.pop();
        }
        _ => {}
    }
}

/// The behaviour tree rows of the selected entity, when it has a tree.
pub fn draw_behavior_tree_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    state: &mut EditorState,
) {
    let library = world.get_resource::<BtLibrary>().map(|l| l.clone());
    // SAFETY: editor UI runs single-threaded in the egui draw; no concurrent World access.
    let mut trees = unsafe { world.query_unchecked::<&mut BehaviorTree>() };
    let Some(mut tree) = trees.get_mut(entity_id) else { return };
    let mut remove = false;
    egui::CollapsingHeader::new(crate::theme::section_title("Davranış Ağacı"))
        .id_salt(("behavior_tree", entity_id.id()))
        .default_open(true)
        .show(ui, |ui| {
            let mut cx = RowContext { library: library.as_ref(), id: u64::from(entity_id.id()), edit: None };
            match tree.root.as_mut() {
                Some(root) => draw_node(ui, root, &mut Vec::new(), 1, &mut cx),
                None => {
                    ui.horizontal(|ui| {
                        ui.label("Boş ağaç — her tick başarısız.");
                        kind_menu(ui, "Kök ekle", |k| cx.edit = Some(TreeEdit::SetRoot(Some(k.fresh()))));
                    });
                }
            }
            if let Some(edit) = cx.edit.take() {
                apply_edit(&mut tree, edit);
            }

            match &library {
                Some(lib) => {
                    let missing = tree.unresolved(lib);
                    if !missing.is_empty() {
                        ui.label(
                            egui::RichText::new(format!("⚠ Kayıtlı olmayan: {}", missing.join(", ")))
                                .color(egui::Color32::from_rgb(230, 90, 70)),
                        );
                    }
                }
                None => {
                    ui.label(
                        egui::RichText::new("Bu dünyada BtLibrary yok: adları oyun kurulumunda kaydeder.")
                            .weak(),
                    );
                }
            }
            ui.separator();
            if ui.button("🗑 Bileşeni Sil").clicked() {
                remove = true;
            }
        });
    ui.separator();
    if remove {
        state.remove_component_request = Some((entity_id, "BehaviorTree".to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_ai::BtStatus;

    fn guard() -> BehaviorTree {
        BehaviorTree::new(BtNode::selector([
            BtNode::sequence([BtNode::condition("sees_player"), BtNode::action("chase")]),
            BtNode::inverter(BtNode::action("rest")),
            BtNode::action("patrol"),
        ]))
    }

    #[test]
    fn appending_moving_and_removing_children() {
        let mut tree = guard();
        apply_edit(&mut tree, TreeEdit::Append(vec![0], BtNode::wait(2.0)));
        apply_edit(&mut tree, TreeEdit::Move(vec![2], true));
        let expected = BehaviorTree::new(BtNode::selector([
            BtNode::sequence([BtNode::condition("sees_player"), BtNode::action("chase"), BtNode::wait(2.0)]),
            BtNode::action("patrol"),
            BtNode::inverter(BtNode::action("rest")),
        ]));
        assert_eq!(tree, expected);
        apply_edit(&mut tree, TreeEdit::Remove(vec![0, 1]));
        apply_edit(&mut tree, TreeEdit::Move(vec![0], true));
        assert_eq!(
            tree,
            BehaviorTree::new(BtNode::selector([
                BtNode::sequence([BtNode::condition("sees_player"), BtNode::wait(2.0)]),
                BtNode::action("patrol"),
                BtNode::inverter(BtNode::action("rest")),
            ])),
            "moving the first child earlier does nothing"
        );
    }

    /// A decorator cannot stand without its child, so removing the child removes the decorator.
    #[test]
    fn removing_a_decorators_child_removes_the_decorator() {
        let mut tree = guard();
        apply_edit(&mut tree, TreeEdit::Remove(vec![1, 0]));
        let BtNode::Selector { children, .. } = tree.root.as_ref().unwrap() else { panic!() };
        assert_eq!(children.len(), 2);
        assert!(!children.iter().any(|c| matches!(c, BtNode::Inverter(_))));
        apply_edit(&mut tree, TreeEdit::Remove(vec![]));
        assert_eq!(tree.root, None, "removing the root empties the tree");
    }

    #[test]
    fn converting_keeps_what_carries_over() {
        let mut tree = guard();
        apply_edit(&mut tree, TreeEdit::Convert(vec![0], NodeKind::Selector));
        apply_edit(&mut tree, TreeEdit::Convert(vec![2], NodeKind::Condition));
        apply_edit(&mut tree, TreeEdit::Convert(vec![1], NodeKind::Succeeder));
        assert_eq!(
            tree,
            BehaviorTree::new(BtNode::selector([
                BtNode::selector([BtNode::condition("sees_player"), BtNode::action("chase")]),
                BtNode::succeeder(BtNode::action("rest")),
                BtNode::condition("patrol"),
            ]))
        );
        // Two children under a one-child decorator stay together, in a sequence.
        let mut node = convert(BtNode::sequence([BtNode::action("a"), BtNode::action("b")]), NodeKind::Inverter);
        assert_eq!(node, BtNode::inverter(BtNode::sequence([BtNode::action("a"), BtNode::action("b")])));
        node = convert(node, NodeKind::Selector);
        assert_eq!(node, BtNode::selector([BtNode::sequence([BtNode::action("a"), BtNode::action("b")])]));
    }

    #[test]
    fn a_shape_change_starts_the_tree_afresh_and_a_stale_path_does_nothing() {
        let lib = BtLibrary::new().with_condition("sees_player", |_, _| true).with_action("chase", |_, _, _| BtStatus::Running);
        let mut tree = guard();
        let mut world = World::new();
        world.insert_resource(lib);
        tree.tick(gizmo_core::Entity::new(0, 0), &mut world, 0.1);
        assert_ne!(tree, guard(), "the sequence is on its second child");
        apply_edit(&mut tree, TreeEdit::Remove(vec![7, 3]));
        assert_eq!(tree, guard(), "a path to nothing removes nothing, and the running state is reset");
    }

    fn painted(world: &World, entity: gizmo_core::entity::Entity) -> Vec<(String, egui::Color32)> {
        fn scan(shape: &egui::Shape, out: &mut Vec<(String, egui::Color32)>) {
            match shape {
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| scan(s, out)),
                egui::Shape::Text(t) => {
                    let colour = t
                        .override_text_color
                        .or_else(|| t.galley.job.sections.first().map(|s| s.format.color))
                        .unwrap_or_default();
                    out.push((t.galley.text().to_string(), colour));
                }
                _ => {}
            }
        }
        let mut state = EditorState::default();
        let ctx = egui::Context::default();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.set_clip_rect(egui::Rect::EVERYTHING);
            draw_behavior_tree_section(ui, world, entity, &mut state);
        });
        let mut out = Vec::new();
        output.shapes.iter().for_each(|s| scan(&s.shape, &mut out));
        output.drop_without_applying_deltas();
        out
    }

    /// The section paints the tree — every node's kind and every leaf's name — and marks, in red,
    /// the leaves the world's library does not know.
    #[test]
    fn the_section_paints_the_tree_and_marks_what_the_library_lacks() {
        let mut world = World::new();
        world.insert_resource(
            BtLibrary::new()
                .with_condition("sees_player", |_, _| true)
                .with_action("chase", |_, _, _| BtStatus::Running)
                .with_action("patrol", |_, _, _| BtStatus::Running),
        );
        let e = world.spawn();
        world.add_component(e, guard());
        let painted = painted(&world, e);
        let text: Vec<&str> = painted.iter().map(|(t, _)| t.as_str()).collect();
        for expected in ["? Seçici", "→ Sıra", "◆ Koşul", "! Ters", "sees_player", "chase", "rest", "patrol"] {
            assert!(text.contains(&expected), "{expected:?} is not on screen: {text:?}");
        }
        let marked: Vec<&(String, egui::Color32)> = painted.iter().filter(|(t, _)| t.contains("ayıtlı")).collect();
        assert!(
            marked.iter().any(|(t, _)| t.contains("action rest")),
            "the summary names the one leaf the library lacks: {marked:?}"
        );
        assert_eq!(marked.iter().filter(|(t, _)| t == "⚠ kayıtlı değil").count(), 1, "and only that row is marked");
        assert!(marked.iter().all(|(_, c)| c.r() > 200 && c.g() < 120), "in red: {marked:?}");
    }

    /// While the game runs, the section shows where the tree is: the sequence's step, the wait's
    /// progress.
    #[test]
    fn the_section_shows_where_a_running_tree_is() {
        let mut world = World::new();
        world.insert_resource(BtLibrary::new().with_action("go", |_, _, _| BtStatus::Success));
        let e = world.spawn();
        world.add_component(e, BehaviorTree::new(BtNode::sequence([BtNode::action("go"), BtNode::wait(2.0)])));
        gizmo_ai::behavior_tree_system(&mut world, 0.5);
        let text: Vec<String> = painted(&world, e).into_iter().map(|(t, _)| t).collect();
        assert!(text.iter().any(|t| t == "adım 2/2"), "the step is not shown: {text:?}");
        assert!(text.iter().any(|t| t == "0.50 sn geçti"), "the wait's progress is not shown: {text:?}");
    }
}
