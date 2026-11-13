//! The shader graph tab: a node editor for [`gizmo_renderer::shader_graph::ShaderGraph`].
//!
//! The graph is drawn on a canvas of its own — nodes as boxes with their outputs on the right edge
//! and their inputs on the left, links as curves between them — and edited with the mouse:
//!
//! - **link**: drag from an output's dot onto an input's dot. An input takes one link; a new one
//!   replaces it. A link that would make a loop is refused, and the status line says so.
//! - **unlink**: drag a linked input's dot away. The link comes with the pointer, so dropping it on
//!   another input moves it and dropping it on empty canvas removes it.
//! - **add**: right-click the canvas, pick a node; it lands where the menu was opened.
//! - **remove**: right-click a node's title. The surface node cannot be removed.
//! - **move**: drag a title. **pan**: drag the empty canvas, with either button.
//! - **values**: an input nothing is linked to holds a value of its own, edited in place, as are
//!   the constants.
//!
//! # The graph and the scene
//!
//! "Apply" puts the graph on the selected entities as a
//! [`SurfaceGraph`](gizmo_renderer::shader_graph::SurfaceGraph) — which the scene saves, and the
//! engine draws — and links them to the tab: every later change that alters the WGSL reaches them
//! as it is made, so the viewport shows the graph being edited. "Edit selected" opens the selected
//! entity's graph and links that entity. A graph that does not compile is not pushed; the entities
//! keep the last one that did. [`apply_to_world`](crate::shader_graph_panel::apply_to_world) is what does this, once a frame, in the host.
//!
//! Every change that alters the WGSL bumps [`ShaderGraphPanelState::revision`](crate::shader_graph_panel::ShaderGraphPanelState::revision); moving a node does
//! not. The graph is saved on its own as JSON (`.shadergraph`), to reuse it in another scene.

use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};
use gizmo_renderer::shader_graph::{GraphError, InputDefault, NodeId, NodeKind, ShaderGraph, SlotType, ValueType};

use crate::theme::{self, palette};
use crate::EditorState;

/// A node's width on the canvas.
pub const NODE_WIDTH: f32 = 180.0;
/// Its title bar's height.
pub const HEADER_HEIGHT: f32 = 22.0;
/// One slot's row.
pub const ROW_HEIGHT: f32 = 20.0;
const PIN_RADIUS: f32 = 5.0;
/// How near a pin the pointer has to be to grab it or drop a link on it.
const PIN_REACH: f32 = 9.0;
const UNDO_DEPTH: usize = 100;

/// The node editor's state: the graph and what the editor is doing with it.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ShaderGraphPanelState {
    /// The graph being edited.
    pub graph: ShaderGraph,
    /// Where Save writes and Load reads.
    pub path: String,
    /// Bumped by every change that alters the graph's WGSL — a link, a node added or removed, a value
    /// edited — and not by moving a node or panning.
    pub revision: u64,
    /// Raised by "Apply": the host gives the selected entities this graph's surface, and lowers it.
    pub apply_request: bool,
    /// The canvas's offset, in points: where the graph's origin is drawn from the canvas's corner.
    pub pan: Vec2,
    /// The node last clicked.
    pub selected: Option<NodeId>,
    /// Whether the generated WGSL is shown under the canvas.
    pub show_wgsl: bool,
    /// What the last action said: a save, a refused link, a file that would not load.
    pub status: String,
    /// The link being dragged, from this output.
    dragging_link: Option<(NodeId, usize)>,
    /// Where the canvas's context menu was opened, in graph coordinates.
    menu_at: Option<Vec2>,
    /// The canvas, as drawn last frame — what pin positions are measured from.
    canvas: Option<Rect>,
    /// The compiled WGSL, and the revision it is of.
    compiled: Option<(u64, Result<String, GraphError>)>,
    undo: Vec<ShaderGraph>,
    /// The entities the graph being edited reaches, by "Apply" or "Edit selected".
    linked: Vec<gizmo_core::Entity>,
    /// The revision they hold.
    pushed_revision: u64,
    /// Raised by "Edit selected".
    edit_request: bool,
    redo: Vec<ShaderGraph>,
}

impl Default for ShaderGraphPanelState {
    fn default() -> Self {
        let mut graph = ShaderGraph::new();
        // The surface on the right, with room for what feeds it.
        if let Ok(output) = graph.output() {
            if let Some(node) = graph.node_mut(output) {
                node.position = [420.0, 40.0];
            }
        }
        Self {
            graph,
            path: "assets/surface.shadergraph".to_string(),
            revision: 0,
            apply_request: false,
            pan: Vec2::new(20.0, 20.0),
            selected: None,
            show_wgsl: false,
            status: String::new(),
            dragging_link: None,
            menu_at: None,
            canvas: None,
            compiled: None,
            undo: Vec::new(),
            redo: Vec::new(),
            linked: Vec::new(),
            pushed_revision: 0,
            edit_request: false,
        }
    }
}

impl ShaderGraphPanelState {
    /// The graph's WGSL, or why it has none, compiled once per revision.
    pub fn wgsl(&mut self) -> &Result<String, GraphError> {
        if self.compiled.as_ref().map(|(r, _)| *r) != Some(self.revision) {
            self.compiled = Some((self.revision, self.graph.to_wgsl()));
        }
        &self.compiled.as_ref().expect("just compiled").1
    }

    /// The entities the graph being edited reaches.
    #[must_use]
    pub fn linked(&self) -> &[gizmo_core::Entity] {
        &self.linked
    }

    /// The canvas as drawn last frame, `None` before the tab has been drawn.
    #[must_use]
    pub fn canvas(&self) -> Option<Rect> {
        self.canvas
    }

    /// Replaces the graph — a new one or one loaded — as one undoable change.
    pub fn set_graph(&mut self, graph: ShaderGraph) {
        self.record();
        self.graph = graph;
        self.changed();
    }

    /// Adds a node at a point of the graph, as one undoable change.
    pub fn add_node(&mut self, kind: NodeKind, at: [f32; 2]) -> NodeId {
        self.record();
        let id = self.graph.add(kind, at);
        self.selected = Some(id);
        self.changed();
        id
    }

    /// Takes the last change back.
    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.undo.pop() else { return false };
        self.redo.push(std::mem::replace(&mut self.graph, previous));
        self.changed();
        true
    }

    /// Puts back what [`undo`](Self::undo) took.
    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else { return false };
        self.undo.push(std::mem::replace(&mut self.graph, next));
        self.changed();
        true
    }

    /// Writes the graph to [`path`](Self::path) as JSON.
    ///
    /// # Errors
    ///
    /// The file could not be written, as text for the status line.
    pub fn save(&mut self) -> Result<(), String> {
        let text = serde_json::to_string_pretty(&self.graph).map_err(|e| e.to_string())?;
        if let Some(dir) = std::path::Path::new(&self.path).parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(&self.path, text).map_err(|e| format!("{}: {e}", self.path))
    }

    /// Reads the graph from [`path`](Self::path), as one undoable change. A graph that reads but is
    /// not one surface (a loop, a link to nothing) is loaded and its error shown, so it can be fixed.
    ///
    /// # Errors
    ///
    /// The file could not be read or is not a graph; the graph being edited is kept.
    pub fn load(&mut self) -> Result<(), String> {
        let text = std::fs::read_to_string(&self.path).map_err(|e| format!("{}: {e}", self.path))?;
        let graph: ShaderGraph = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", self.path))?;
        self.set_graph(graph);
        Ok(())
    }

    /// Where a pin is drawn: an output on a node's right edge, an input on its left, `None` for a
    /// node or slot that is not there or before the canvas has been drawn.
    #[must_use]
    pub fn pin_position(&self, node: NodeId, slot: usize, input: bool) -> Option<Pos2> {
        let canvas = self.canvas?;
        let n = self.graph.node(node)?;
        let row = if input {
            if slot >= n.kind.inputs().len() {
                return None;
            }
            input_row(&n.kind, slot)
        } else {
            if slot >= n.kind.outputs().len() {
                return None;
            }
            slot
        };
        let corner = self.node_corner(canvas, n.position);
        let x = if input { corner.x } else { corner.x + NODE_WIDTH };
        Some(Pos2::new(x, corner.y + HEADER_HEIGHT + row as f32 * ROW_HEIGHT + ROW_HEIGHT * 0.5))
    }

    /// Where a node's title bar is, `None` as for [`pin_position`](Self::pin_position).
    #[must_use]
    pub fn header_rect(&self, node: NodeId) -> Option<Rect> {
        let canvas = self.canvas?;
        let n = self.graph.node(node)?;
        Some(Rect::from_min_size(self.node_corner(canvas, n.position), Vec2::new(NODE_WIDTH, HEADER_HEIGHT)))
    }

    fn node_corner(&self, canvas: Rect, position: [f32; 2]) -> Pos2 {
        canvas.min + self.pan + Vec2::new(position[0], position[1])
    }

    fn record(&mut self) {
        self.undo.push(self.graph.clone());
        if self.undo.len() > UNDO_DEPTH {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn changed(&mut self) {
        self.revision += 1;
    }

    /// The input pin within reach of `pointer`, if any.
    fn input_at(&self, pointer: Pos2) -> Option<(NodeId, usize)> {
        self.graph.nodes().iter().find_map(|n| {
            (0..n.kind.inputs().len())
                .find(|&i| self.pin_position(n.id, i, true).is_some_and(|p| p.distance(pointer) <= PIN_REACH))
                .map(|i| (n.id, i))
        })
    }
}

/// Carries the tab's requests into the world, once a frame: "Edit selected" opens the primary
/// selection's graph, "Apply" puts the graph on every selected entity with a material (a `Material`
/// or the `MaterialDesc` one is built from), and a changed
/// graph that compiles reaches the entities linked to the tab. See the [module docs](self).
pub fn apply_to_world(world: &mut gizmo_core::World, state: &mut EditorState) {
    use gizmo_renderer::components::{Material, MaterialDesc};
    use gizmo_renderer::shader_graph::SurfaceGraph;

    let primary = state.selection.primary;
    let selected: Vec<gizmo_core::Entity> = state.selection.entities.iter().copied().collect();
    let panel = &mut state.shader_graph;

    if std::mem::take(&mut panel.edit_request) {
        let found = primary.and_then(|e| world.query::<&SurfaceGraph>().get(e).map(|g| (e, g.graph.clone())));
        if let Some((entity, graph)) = found {
            panel.set_graph(graph);
            panel.linked = vec![entity];
            panel.pushed_revision = panel.revision;
            panel.status = format!("Editing entity {}'s graph", entity.id());
        }
    }

    // Taken first, so a request on a graph that does not compile is dropped, not kept for later.
    if std::mem::take(&mut panel.apply_request) && panel.wgsl().is_ok() {
        let targets: Vec<gizmo_core::Entity> = {
            // A live material, or the description a loaded one starts from.
            let materials = world.query::<&Material>();
            let descs = world.query::<&MaterialDesc>();
            selected.into_iter().filter(|e| materials.get(*e).is_some() || descs.get(*e).is_some()).collect()
        };
        let mut changes = Vec::with_capacity(targets.len());
        for &entity in &targets {
            let before = world.query::<&SurfaceGraph>().get(entity).map(|g| g.graph.clone());
            world.add_component(entity, SurfaceGraph::new(panel.graph.clone()));
            changes.push((entity, before, Some(panel.graph.clone())));
        }
        if !changes.is_empty() {
            state.history.push(crate::history::EditorAction::SurfaceGraphsChanged { changes });
        }
        panel.status = match targets.len() {
            0 => "Nothing selected has a material".to_string(),
            n => format!("Applied to {n} entities"),
        };
        panel.linked = targets;
        panel.pushed_revision = panel.revision;
    }

    if panel.revision != panel.pushed_revision && panel.wgsl().is_ok() {
        panel.pushed_revision = panel.revision;
        // An entity despawned, or whose graph something else took off, is not written back.
        let still: Vec<gizmo_core::Entity> = {
            let graphs = world.query::<&SurfaceGraph>();
            panel.linked.iter().copied().filter(|e| graphs.get(*e).is_some()).collect()
        };
        for &entity in &still {
            world.add_component(entity, SurfaceGraph::new(panel.graph.clone()));
        }
        panel.linked = still;
    }
}

/// Which row an input sits in: after the outputs and, for a constant, its value.
fn input_row(kind: &NodeKind, slot: usize) -> usize {
    kind.outputs().len() + usize::from(has_value_row(kind)) + slot
}

fn has_value_row(kind: &NodeKind) -> bool {
    matches!(kind, NodeKind::Float(_) | NodeKind::Vector2(_) | NodeKind::Vector3(_) | NodeKind::Color(_))
}

fn node_height(kind: &NodeKind) -> f32 {
    let rows = kind.outputs().len() + usize::from(has_value_row(kind)) + kind.inputs().len();
    HEADER_HEIGHT + rows as f32 * ROW_HEIGHT + 4.0
}

fn type_color(ty: Option<ValueType>) -> Color32 {
    match ty {
        Some(ValueType::Float) => Color32::from_rgb(0xa0, 0xa0, 0xa0),
        Some(ValueType::Vec2) => Color32::from_rgb(0x63, 0xc7, 0x63),
        Some(ValueType::Vec3) => Color32::from_rgb(0xe8, 0xc5, 0x3d),
        Some(ValueType::Vec4) => Color32::from_rgb(0xb0, 0x7c, 0xe8),
        _ => palette::TEXT_DIM,
    }
}

/// Draws the shader graph tab.
pub fn ui_shader_graph(ui: &mut egui::Ui, world: &gizmo_core::World, state: &mut EditorState) {
    let has_selection = !state.selection.entities.is_empty();
    let editable = state
        .selection
        .primary
        .is_some_and(|e| world.query::<&gizmo_renderer::shader_graph::SurfaceGraph>().get(e).is_some());
    let panel = &mut state.shader_graph;
    toolbar(ui, panel, has_selection, editable);
    if panel.show_wgsl {
        let text = match panel.wgsl() {
            Ok(wgsl) => wgsl.clone(),
            Err(e) => format!("// {e}"),
        };
        egui::Panel::bottom(ui.id().with("shader-graph-wgsl"))
            .resizable(true)
            .default_size(160.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add(egui::Label::new(egui::RichText::new(text).monospace().size(11.0)).selectable(true));
                });
            });
    }
    canvas(ui, panel);
}

fn toolbar(ui: &mut egui::Ui, panel: &mut ShaderGraphPanelState, has_selection: bool, editable: bool) {
    ui.horizontal(|ui| {
        if ui.button("New").clicked() {
            panel.set_graph(ShaderGraphPanelState::default().graph);
            panel.status = "New graph".to_string();
        }
        ui.add(egui::TextEdit::singleline(&mut panel.path).desired_width(220.0));
        if ui.button("Load").clicked() {
            panel.status = match panel.load() {
                Ok(()) => format!("Loaded {}", panel.path),
                Err(e) => e,
            };
        }
        if ui.button("Save").clicked() {
            panel.status = match panel.save() {
                Ok(()) => format!("Saved {}", panel.path),
                Err(e) => e,
            };
        }
        ui.separator();
        if ui.add_enabled(!panel.undo.is_empty(), egui::Button::new("Undo")).clicked() {
            panel.undo();
        }
        if ui.add_enabled(!panel.redo.is_empty(), egui::Button::new("Redo")).clicked() {
            panel.redo();
        }
        ui.separator();
        let compiles = panel.wgsl().is_ok();
        if ui
            .add_enabled(compiles && has_selection, egui::Button::new("Apply"))
            .on_hover_text("Give the selected entities this surface")
            .clicked()
        {
            panel.apply_request = true;
        }
        if ui
            .add_enabled(editable, egui::Button::new("Edit selected"))
            .on_hover_text("Open the selected entity's graph here")
            .clicked()
        {
            panel.edit_request = true;
        }
        if !panel.linked.is_empty() {
            ui.label(egui::RichText::new(format!("→ {} linked", panel.linked.len())).color(palette::TEXT_DIM));
        }
        ui.toggle_value(&mut panel.show_wgsl, "WGSL");
    });
    ui.horizontal(|ui| match panel.wgsl() {
        Ok(_) => {
            ui.label(egui::RichText::new("✔ compiles").color(palette::TEXT_MUTED));
            if !panel.status.is_empty() {
                ui.label(egui::RichText::new(&panel.status).color(palette::TEXT_DIM));
            }
        }
        Err(e) => {
            ui.label(egui::RichText::new(format!("✖ {e}")).color(palette::DANGER));
        }
    });
}

fn canvas(ui: &mut egui::Ui, panel: &mut ShaderGraphPanelState) {
    let (rect, background) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
    panel.canvas = Some(rect);
    let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    ui.set_clip_rect(rect);
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 0.0, palette::VOID);

    // A grid that moves with the pan.
    let step = 32.0;
    let grid = Stroke::new(1.0, palette::CHROME);
    let start = rect.min + Vec2::new(panel.pan.x.rem_euclid(step), panel.pan.y.rem_euclid(step));
    let mut x = start.x;
    while x < rect.max.x {
        painter.line_segment([Pos2::new(x, rect.min.y), Pos2::new(x, rect.max.y)], grid);
        x += step;
    }
    let mut y = start.y;
    while y < rect.max.y {
        painter.line_segment([Pos2::new(rect.min.x, y), Pos2::new(rect.max.x, y)], grid);
        y += step;
    }

    if background.dragged() {
        panel.pan += background.drag_delta();
    }
    if background.secondary_clicked() {
        panel.menu_at = background.interact_pointer_pos().map(|p| p - rect.min - panel.pan);
    }
    background.context_menu(|ui| add_menu(ui, panel));

    let types = panel.graph.output_types().unwrap_or_default();

    // Links, under the nodes.
    for link in panel.graph.links() {
        if let (Some(a), Some(b)) = (panel.pin_position(link.from, link.output, false), panel.pin_position(link.to, link.input, true)) {
            let color = type_color(types.get(&(link.from, link.output)).copied());
            curve(&painter, a, b, color);
        }
    }

    let ids: Vec<NodeId> = panel.graph.nodes().iter().map(|n| n.id).collect();
    for id in ids {
        node_ui(&mut ui, &painter, panel, id, &types);
    }

    // The link being dragged follows the pointer, and lands on the input it is dropped on.
    if let Some((from, output)) = panel.dragging_link {
        let pointer = ui.input(|i| i.pointer.latest_pos());
        if let (Some(a), Some(b)) = (panel.pin_position(from, output, false), pointer) {
            curve(&painter, a, b, palette::ACCENT_LIGHT);
        }
        if ui.input(|i| i.pointer.any_released()) {
            panel.dragging_link = None;
            if let Some((to, input)) = pointer.and_then(|p| panel.input_at(p)) {
                let mut graph = panel.graph.clone();
                match graph.connect(from, output, to, input) {
                    Ok(()) => {
                        panel.record();
                        panel.graph = graph;
                        panel.changed();
                    }
                    Err(e) => panel.status = format!("Not linked: {e}"),
                }
            }
        }
    }
}

fn curve(painter: &egui::Painter, a: Pos2, b: Pos2, color: Color32) {
    let reach = ((b.x - a.x).abs() * 0.5).max(40.0);
    let shape = egui::epaint::CubicBezierShape::from_points_stroke(
        [a, a + Vec2::new(reach, 0.0), b - Vec2::new(reach, 0.0), b],
        false,
        Color32::TRANSPARENT,
        Stroke::new(2.0, color),
    );
    painter.add(shape);
}

fn add_menu(ui: &mut egui::Ui, panel: &mut ShaderGraphPanelState) {
    let catalogue = NodeKind::catalogue();
    for category in ["Input", "Constant", "Math", "Vector", "Pattern"] {
        ui.menu_button(category, |ui| {
            for kind in catalogue.iter().filter(|k| k.category() == category) {
                if ui.button(kind.name()).clicked() {
                    let at = panel.menu_at.unwrap_or(Vec2::new(40.0, 40.0));
                    panel.add_node(kind.clone(), [at.x, at.y]);
                    ui.close();
                }
            }
        });
    }
}

fn node_ui(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    panel: &mut ShaderGraphPanelState,
    id: NodeId,
    types: &std::collections::HashMap<(NodeId, usize), ValueType>,
) {
    let Some(canvas) = panel.canvas else { return };
    let Some(node) = panel.graph.node(id).cloned() else { return };
    let corner = panel.node_corner(canvas, node.position);
    let body = Rect::from_min_size(corner, Vec2::new(NODE_WIDTH, node_height(&node.kind)));
    let header = Rect::from_min_size(corner, Vec2::new(NODE_WIDTH, HEADER_HEIGHT));
    let selected = panel.selected == Some(id);
    let is_output = node.kind == NodeKind::Output;

    painter.rect_filled(body, 4.0, palette::SURFACE);
    painter.rect_filled(header, 4.0, if is_output { palette::ACCENT_DEEP } else { palette::CHROME });
    painter.rect_stroke(
        body,
        4.0,
        Stroke::new(1.0, if selected { palette::ACCENT } else { palette::BORDER }),
        egui::StrokeKind::Inside,
    );
    let font = egui::FontId::proportional(theme::TEXT_SIZE);
    painter.text(header.left_center() + Vec2::new(8.0, 0.0), egui::Align2::LEFT_CENTER, node.kind.name(), font.clone(), palette::TEXT_BRIGHT);

    let header_response = ui.interact(header, ui.id().with(("shader-graph-node", id.0)), Sense::click_and_drag());
    if header_response.clicked() || header_response.drag_started() {
        panel.selected = Some(id);
    }
    if header_response.dragged() {
        let delta = header_response.drag_delta();
        if let Some(n) = panel.graph.node_mut(id) {
            n.position[0] += delta.x;
            n.position[1] += delta.y;
        }
    }
    if !is_output {
        header_response.context_menu(|ui| {
            if ui.button("Delete node").clicked() {
                panel.record();
                if panel.graph.remove(id).is_ok() {
                    panel.changed();
                }
                ui.close();
            }
        });
    }

    let row_rect = |row: usize| {
        Rect::from_min_size(
            corner + Vec2::new(0.0, HEADER_HEIGHT + row as f32 * ROW_HEIGHT),
            Vec2::new(NODE_WIDTH, ROW_HEIGHT),
        )
    };

    // Outputs: name on the right, dot on the edge; dragging the dot starts a link.
    for (o, slot) in node.kind.outputs().iter().enumerate() {
        let r = row_rect(o);
        painter.text(r.right_center() - Vec2::new(10.0, 0.0), egui::Align2::RIGHT_CENTER, slot.name, font.clone(), palette::TEXT_BODY);
        let pin = Pos2::new(r.max.x, r.center().y);
        let ty = match slot.ty {
            SlotType::Fixed(t) => Some(t),
            _ => types.get(&(id, o)).copied(),
        };
        painter.circle_filled(pin, PIN_RADIUS, type_color(ty));
        let response = ui.interact(Rect::from_center_size(pin, Vec2::splat(PIN_REACH * 2.0)), ui.id().with(("shader-graph-out", id.0, o)), Sense::drag());
        if response.drag_started() {
            panel.dragging_link = Some((id, o));
        }
    }

    // A constant's value.
    if has_value_row(&node.kind) {
        let r = row_rect(node.kind.outputs().len()).shrink2(Vec2::new(8.0, 1.0));
        let mut kind = node.kind.clone();
        let changed = match &mut kind {
            NodeKind::Float(v) => ui.put(r, egui::DragValue::new(v).speed(0.01)).changed(),
            NodeKind::Vector2(v) => drag_row(ui, r, v),
            NodeKind::Vector3(v) => drag_row(ui, r, v),
            NodeKind::Color(c) => {
                let mut rgba = egui::Rgba::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
                let changed = ui
                    .put(Rect::from_min_size(r.min, Vec2::new(r.width(), r.height())), |ui: &mut egui::Ui| {
                        egui::color_picker::color_edit_button_rgba(ui, &mut rgba, egui::color_picker::Alpha::OnlyBlend)
                    })
                    .changed();
                if changed {
                    *c = rgba.to_rgba_unmultiplied();
                }
                changed
            }
            _ => false,
        };
        if changed {
            panel.record();
            if let Some(n) = panel.graph.node_mut(id) {
                n.kind = kind;
            }
            panel.changed();
        }
    }

    // Inputs: dot on the edge, name, and the value it holds while nothing is linked.
    for (i, slot) in node.kind.inputs().iter().enumerate() {
        let r = row_rect(input_row(&node.kind, i));
        let pin = Pos2::new(r.min.x, r.center().y);
        let link = panel.graph.link_into(id, i).copied();
        let ty = match slot.ty {
            SlotType::Fixed(t) => Some(t),
            _ => link.and_then(|l| types.get(&(l.from, l.output)).copied()),
        };
        painter.circle_filled(pin, PIN_RADIUS, type_color(ty));
        painter.text(r.left_center() + Vec2::new(10.0, 0.0), egui::Align2::LEFT_CENTER, slot.name, font.clone(), palette::TEXT_BODY);
        let response = ui.interact(Rect::from_center_size(pin, Vec2::splat(PIN_REACH * 2.0)), ui.id().with(("shader-graph-in", id.0, i)), Sense::drag());
        if let Some(link) = link {
            // Picking up a linked input carries its link away from it.
            if response.drag_started() {
                panel.record();
                panel.graph.disconnect(id, i);
                panel.changed();
                panel.dragging_link = Some((link.from, link.output));
            }
            continue;
        }
        let value_rect = Rect::from_min_max(Pos2::new(r.min.x + NODE_WIDTH * 0.45, r.min.y + 1.0), Pos2::new(r.max.x - 6.0, r.max.y - 1.0));
        match slot.default {
            InputDefault::Value(_) => {
                let mut value = node.value(i).unwrap_or([0.0; 4]);
                let width = match slot.ty {
                    SlotType::Fixed(t) => t.width(),
                    _ => 1,
                };
                if drag_row(ui, value_rect, &mut value[..width]) {
                    panel.record();
                    if let Some(n) = panel.graph.node_mut(id) {
                        if n.values.len() <= i {
                            n.values.resize(i + 1, [0.0; 4]);
                        }
                        n.values[i] = value;
                    }
                    panel.changed();
                }
            }
            default => {
                let hint = match default {
                    InputDefault::Uv => "mesh UV",
                    InputDefault::MaterialUv => "material UV",
                    InputDefault::WorldNormal => "normal",
                    InputDefault::WorldPosition => "position",
                    _ => "engine",
                };
                painter.text(value_rect.right_center(), egui::Align2::RIGHT_CENTER, hint, font.clone(), palette::TEXT_DIM);
            }
        }
    }
}

/// One drag value per component, side by side in `rect`; whether any changed.
fn drag_row(ui: &mut egui::Ui, rect: Rect, values: &mut [f32]) -> bool {
    let n = values.len().max(1) as f32;
    let w = rect.width() / n;
    let mut changed = false;
    for (k, v) in values.iter_mut().enumerate() {
        let r = Rect::from_min_size(rect.min + Vec2::new(w * k as f32, 0.0), Vec2::new(w - 2.0, rect.height()));
        changed |= ui.put(r, egui::DragValue::new(v).speed(0.01).max_decimals(3)).changed();
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, PointerButton, RawInput};

    fn frame(ctx: &egui::Context, state: &mut EditorState, events: Vec<Event>) -> egui::FullOutput {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 800.0))),
            events,
            ..Default::default()
        };
        ctx.run_ui(input, |ui| ui_shader_graph(ui, &gizmo_core::World::new(), state))
    }

    fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
        fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let mut out = Vec::new();
        for s in shapes {
            walk(&s.shape, &mut out);
        }
        out
    }

    fn state() -> EditorState {
        let mut state = EditorState::new();
        state.shader_graph = ShaderGraphPanelState::default();
        state
    }

    /// Presses at `from`, moves to `to` in steps, releases there: a drag as a hand makes one.
    fn drag(ctx: &egui::Context, state: &mut EditorState, from: Pos2, to: Pos2) {
        frame(ctx, state, vec![Event::PointerMoved(from)]).drop_without_applying_deltas();
        frame(ctx, state, vec![Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Default::default() }])
            .drop_without_applying_deltas();
        for k in 1..=4 {
            let p = from + (to - from) * (k as f32 / 4.0);
            frame(ctx, state, vec![Event::PointerMoved(p)]).drop_without_applying_deltas();
        }
        frame(ctx, state, vec![Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Default::default() }])
            .drop_without_applying_deltas();
        frame(ctx, state, vec![]).drop_without_applying_deltas();
    }

    /// The tab draws each node by name, each slot by name, and says the graph compiles.
    #[test]
    fn the_tab_draws_the_nodes_and_says_whether_the_graph_compiles() {
        let ctx = egui::Context::default();
        let mut state = state();
        state.shader_graph.add_node(NodeKind::Noise, [40.0, 40.0]);
        let output = frame(&ctx, &mut state, vec![]);
        let text = texts(&output.shapes);
        output.drop_without_applying_deltas();
        for want in ["Surface", "Noise", "albedo", "normal", "position", "scale", "✔ compiles"] {
            assert!(text.iter().any(|t| t == want), "{want:?} is not on screen: {text:?}");
        }
    }

    /// **Dragging an output's dot onto an input's dot links them**, as one change that bumps the
    /// revision and can be undone.
    #[test]
    fn dragging_from_an_output_to_an_input_links_them() {
        let ctx = egui::Context::default();
        let mut state = state();
        let color = state.shader_graph.add_node(NodeKind::Color([1.0, 0.0, 0.0, 1.0]), [40.0, 40.0]);
        let surface = state.shader_graph.graph.output().unwrap();
        frame(&ctx, &mut state, vec![]).drop_without_applying_deltas();
        let from = state.shader_graph.pin_position(color, 0, false).unwrap();
        let to = state.shader_graph.pin_position(surface, 0, true).unwrap();
        let revision = state.shader_graph.revision;

        drag(&ctx, &mut state, from, to);
        let link = state.shader_graph.graph.link_into(surface, 0).copied();
        assert_eq!(link.map(|l| (l.from, l.output)), Some((color, 0)), "no link after the drag: {}", state.shader_graph.status);
        assert!(state.shader_graph.revision > revision);
        assert!(state.shader_graph.wgsl().as_ref().unwrap().contains("surface.albedo ="));

        assert!(state.shader_graph.undo());
        assert!(state.shader_graph.graph.link_into(surface, 0).is_none(), "undo left the link");
        assert!(state.shader_graph.redo());
        assert!(state.shader_graph.graph.link_into(surface, 0).is_some(), "redo lost the link");
    }

    /// **Dragging a linked input away unlinks it**; dropped on another input, the link moves there.
    #[test]
    fn dragging_a_linked_input_away_carries_the_link() {
        let ctx = egui::Context::default();
        let mut state = state();
        let time = state.shader_graph.add_node(NodeKind::Time, [40.0, 40.0]);
        let surface = state.shader_graph.graph.output().unwrap();
        state.shader_graph.graph.connect(time, 0, surface, 2).unwrap();
        frame(&ctx, &mut state, vec![]).drop_without_applying_deltas();

        let roughness = state.shader_graph.pin_position(surface, 2, true).unwrap();
        let metallic = state.shader_graph.pin_position(surface, 3, true).unwrap();
        drag(&ctx, &mut state, roughness, metallic);
        assert!(state.shader_graph.graph.link_into(surface, 2).is_none(), "the link stayed");
        assert_eq!(state.shader_graph.graph.link_into(surface, 3).map(|l| l.from), Some(time), "the link did not move");

        drag(&ctx, &mut state, metallic, metallic + Vec2::new(-200.0, 150.0));
        assert!(state.shader_graph.graph.links().is_empty(), "dropped on nothing, the link is still there");
    }

    /// A link that would make a loop is refused, and the status line says why.
    #[test]
    fn a_link_that_makes_a_loop_is_refused() {
        let ctx = egui::Context::default();
        let mut state = state();
        let a = state.shader_graph.add_node(NodeKind::Sin, [40.0, 40.0]);
        let b = state.shader_graph.add_node(NodeKind::Cos, [40.0, 200.0]);
        state.shader_graph.graph.connect(a, 0, b, 0).unwrap();
        frame(&ctx, &mut state, vec![]).drop_without_applying_deltas();
        let from = state.shader_graph.pin_position(b, 0, false).unwrap();
        let to = state.shader_graph.pin_position(a, 0, true).unwrap();
        drag(&ctx, &mut state, from, to);
        assert!(state.shader_graph.graph.link_into(a, 0).is_none());
        assert!(state.shader_graph.status.contains("loop"), "{}", state.shader_graph.status);
    }

    /// Dragging a title moves the node and does not bump the revision: the WGSL is the same.
    #[test]
    fn moving_a_node_does_not_change_the_surface() {
        let ctx = egui::Context::default();
        let mut state = state();
        let noise = state.shader_graph.add_node(NodeKind::Noise, [40.0, 40.0]);
        frame(&ctx, &mut state, vec![]).drop_without_applying_deltas();
        let revision = state.shader_graph.revision;
        let title = state.shader_graph.header_rect(noise).unwrap().center();
        drag(&ctx, &mut state, title, title + Vec2::new(60.0, 30.0));
        let p = state.shader_graph.graph.node(noise).unwrap().position;
        assert!((p[0] - 100.0).abs() < 1.0 && (p[1] - 70.0).abs() < 1.0, "the node is at {p:?}");
        assert_eq!(state.shader_graph.revision, revision);
    }

    /// A graph that is not one surface says what is wrong, in the danger colour.
    #[test]
    fn a_broken_graph_shows_its_error() {
        let ctx = egui::Context::default();
        let mut state = state();
        let f = state.shader_graph.add_node(NodeKind::Float(1.0), [40.0, 40.0]);
        if let Some(n) = state.shader_graph.graph.node_mut(f) {
            n.kind = NodeKind::Float(f32::INFINITY);
        }
        let surface = state.shader_graph.graph.output().unwrap();
        state.shader_graph.graph.connect(f, 0, surface, 2).unwrap();
        state.shader_graph.revision += 1;
        let output = frame(&ctx, &mut state, vec![]);
        let text = texts(&output.shapes);
        output.drop_without_applying_deltas();
        assert!(text.iter().any(|t| t.starts_with('✖') && t.contains("not a finite number")), "{text:?}");
    }

    /// **"Apply" puts the graph on the selected entities that have a material, and links them**: an
    /// edit made afterwards reaches them, one that does not compile does not, and "Edit selected"
    /// opens an entity's graph.
    #[test]
    fn apply_links_the_selection_and_edits_reach_it() {
        use gizmo_renderer::components::MaterialDesc;
        use gizmo_renderer::shader_graph::SurfaceGraph;
        let mut world = gizmo_core::World::new();
        let with_material = world.spawn();
        world.add_component(with_material, MaterialDesc::default());
        let without = world.spawn();
        let mut state = state();
        state.selection.entities.extend([with_material, without]);
        state.selection.primary = Some(with_material);

        let color = state.shader_graph.add_node(NodeKind::Color([0.9, 0.05, 0.05, 1.0]), [40.0, 40.0]);
        let surface = state.shader_graph.graph.output().unwrap();
        state.shader_graph.graph.connect(color, 0, surface, 0).unwrap();
        state.shader_graph.apply_request = true;
        apply_to_world(&mut world, &mut state);
        let graph_of = |world: &gizmo_core::World, e| world.query::<&SurfaceGraph>().get(e).map(|g| g.graph.clone());
        assert_eq!(graph_of(&world, with_material), Some(state.shader_graph.graph.clone()));
        assert_eq!(graph_of(&world, without), None, "an entity with no material got a surface");
        assert_eq!(state.shader_graph.linked(), &[with_material]);

        // "Apply" is one step of the editor's own history: undone, the entity has no graph again.
        state.history.undo(&mut world);
        assert_eq!(graph_of(&world, with_material), None, "undo left the applied graph");
        state.history.redo(&mut world);
        assert_eq!(graph_of(&world, with_material), Some(state.shader_graph.graph.clone()), "redo did not put it back");

        // A later edit reaches it…
        let noise = state.shader_graph.add_node(NodeKind::Noise, [40.0, 200.0]);
        state.shader_graph.graph.connect(noise, 0, surface, 2).unwrap();
        apply_to_world(&mut world, &mut state);
        assert_eq!(graph_of(&world, with_material), Some(state.shader_graph.graph.clone()), "the edit did not reach the entity");
        // …one that does not compile does not.
        let good = state.shader_graph.graph.clone();
        if let Some(n) = state.shader_graph.graph.node_mut(color) {
            n.kind = NodeKind::Color([f32::NAN, 0.0, 0.0, 1.0]);
        }
        state.shader_graph.revision += 1;
        apply_to_world(&mut world, &mut state);
        assert_eq!(graph_of(&world, with_material), Some(good.clone()), "a broken graph was pushed");

        // "Edit selected" opens the entity's graph.
        state.shader_graph = ShaderGraphPanelState::default();
        state.shader_graph.edit_request = true;
        apply_to_world(&mut world, &mut state);
        assert_eq!(state.shader_graph.graph, good);
        assert_eq!(state.shader_graph.linked(), &[with_material]);
    }

    /// Saved and loaded, a graph comes back the same; a file that is not a graph keeps the one being
    /// edited.
    #[test]
    fn a_graph_saves_and_loads() {
        let dir = std::env::temp_dir().join(format!("gizmo-shader-graph-{}", std::process::id()));
        let mut panel = ShaderGraphPanelState { path: dir.join("g.shadergraph").to_string_lossy().into_owned(), ..Default::default() };
        let noise = panel.add_node(NodeKind::Noise, [10.0, 20.0]);
        panel.graph.connect(noise, 0, panel.graph.output().unwrap(), 2).unwrap();
        panel.save().expect("saves");
        let saved = panel.graph.clone();
        panel.set_graph(ShaderGraph::new());
        panel.load().expect("loads");
        assert_eq!(panel.graph, saved);

        std::fs::write(&panel.path, "not a graph").unwrap();
        assert!(panel.load().is_err());
        assert_eq!(panel.graph, saved);
        let _ = std::fs::remove_dir_all(dir);
    }
}
