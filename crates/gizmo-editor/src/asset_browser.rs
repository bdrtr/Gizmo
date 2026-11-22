//! The asset browser — shows the project's files in the bottom panel.

use crate::editor_state::EditorState;
use egui;
use std::path::Path;

#[cfg(target_arch = "wasm32")]
use web_time::Instant;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

/// The file name of a prefab saved as `stem` — an entity's name, say. Spaces become `_`, and so does
/// every character a file system refuses: Windows' `<>:"/\|?*` and control characters. A slash would
/// have put the file in a folder of its own, and a colon, on NTFS, in an alternate data stream.
pub(crate) fn prefab_file_name(stem: &str) -> String {
    let clean: String = stem
        .chars()
        .map(|c| if c == ' ' || "<>:\"/\\|?*".contains(c) || c.is_control() { '_' } else { c })
        .collect();
    format!("{clean}.prefab")
}

/// Draws the asset browser tab.
pub fn ui_asset_browser(ui: &mut egui::Ui, state: &mut EditorState) {
    poll_workspace_pick(state);
    header(ui, state);
    kind_chips(ui, state);
    breadcrumbs(ui, state);
    quick_actions(ui, state);

    ui.separator();

    // The detail pane is a right-hand COLUMN, as in the prototype — not a strip under the grid.
    // Below the grid it simply never appeared: the thumbnails fill the dock and push it past the
    // bottom edge, which is what the first attempt did.
    if state.assets.selected.is_some() {
        egui::Panel::right("asset_detail")
            .exact_size(200.0)
            .show(ui, |ui| {
                draw_asset_detail(ui, state);
            });
    }

    // The folder tree is the prototype's LEFT column, and the third of the three.
    //
    // It is dropped when the dock is too narrow rather than squeezed: the three columns want
    // `TREE + grid + DETAIL`, and below that width the grid — the column the panel is actually
    // for — gets nothing. The detail pane already only appears when there is a selection, so the
    // usual layout is two columns wide, not three.
    state.assets.workspace_root =
        tree_root_for(&state.assets.workspace_root, &state.assets.root);
    if ui.available_width() > TREE_WIDTH + 260.0 {
        egui::Panel::left("asset_tree")
            .exact_size(TREE_WIDTH)
            .show(ui, |ui| {
                draw_folder_tree(ui, state);
            });
    }

    egui::ScrollArea::both().show(ui, |ui| {
        ui.horizontal_wrapped(|ui| asset_grid(ui, state));
    });
}

/// Takes the folder the workspace dialog chose, once it has answered.
fn poll_workspace_pick(state: &mut EditorState) {
    let mut finished = false;
    if let Some(rx) = &state.assets.workspace_rx {
        // Poison-recovery: mutex zehirlenmişse panik yerine iç değeri kurtar.
        match rx.lock().unwrap_or_else(|e| e.into_inner()).try_recv() {
            Ok(path) => {
                if !path.is_empty() {
                    state.assets.root = path;
                }
                finished = true;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                finished = true;
            }
            _ => {}
        }
    }

    if finished {
        state.assets.workspace_rx = None;
    }
}

/// The title row: back, the workspace picker and the text filter.
fn header(ui: &mut egui::Ui, state: &mut EditorState) {
    ui.horizontal(|ui| {
        ui.heading("🗂️ Asset Browser");
        ui.separator();

        // Geri git
        if ui.button("⬅").on_hover_text("Üst Dizin (Geri)").clicked() {
            if let Some(parent) = Path::new(&state.assets.root).parent() {
                state.assets.root = parent.to_string_lossy().to_string();
            }
        }

        #[cfg(not(target_arch = "wasm32"))]
        workspace_button(ui, state);

        ui.separator();

        ui.add(
            egui::TextEdit::singleline(&mut state.assets.filter)
                .desired_width(140.0)
                .hint_text("Filter assets…"),
        );
        ui.separator();

    });
}

/// "Open workspace": a native folder dialog, on a thread of its own so the editor keeps drawing.
///
/// Native only. A browser has no folder dialog, and wasm has no threads to wait on one: the button
/// was there on wasm too, and its `std::thread::spawn` panicked.
#[cfg(not(target_arch = "wasm32"))]
fn workspace_button(ui: &mut egui::Ui, state: &mut EditorState) {
    if state.assets.workspace_rx.is_none() {
        if ui
            .button("📁 Workspace Aç")
            .on_hover_text("Bilgisayardan bir çalışma dizini seçin")
            .clicked()
        {
            let (tx, rx) = std::sync::mpsc::channel();
            state.assets.workspace_rx = Some(std::sync::Mutex::new(rx));
            std::thread::spawn(move || {
                if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                    let _ = tx.send(folder.to_string_lossy().to_string());
                }
            });
        }
    } else {
        let _ = ui
            .add_enabled(false, egui::Button::new("📁 Workspace Aç"))
            .on_hover_text("Dizin seçimi bekleniyor...");
    }
}

/// The type chips.
///
/// They get their own row, as in the prototype — a 28 px filter strip under the header. Squeezed
/// onto the end of the header they simply ran off the right edge of the panel.
fn kind_chips(ui: &mut egui::Ui, state: &mut EditorState) {
    ui.horizontal(|ui| {
        for (kind, label) in AssetKind::CHIPS {
            let mut on = state.assets.kind_filter == kind;
            if crate::theme::toggle(ui, &mut on, label) {
                // Clicking the active chip turns it off, which lands back on All.
                state.assets.kind_filter = if on { kind } else { None };
            }
        }
    });
}

/// The current folder as a path of buttons, each going back to that folder.
fn breadcrumbs(ui: &mut egui::Ui, state: &mut EditorState) {
    ui.horizontal(|ui| {
        // Breadcrumb tarzında yol gösterimi
        let current_root = state.assets.root.clone();
        ui.horizontal(|ui| {
            let components: Vec<_> = Path::new(&current_root).components().collect();
            let mut current_path = std::path::PathBuf::new();

            for (i, comp) in components.iter().enumerate() {
                current_path.push(comp);
                let part_str = comp.as_os_str().to_string_lossy();

                if ui.add(egui::Button::new(part_str).frame(false)).clicked() {
                    state.assets.root = current_path.to_string_lossy().to_string();
                }

                if i < components.len() - 1 {
                    ui.label("›"); // Breadcrumb separator
                }
            }
        });
    });
}

/// The quick actions row.
fn quick_actions(ui: &mut egui::Ui, state: &mut EditorState) {
    ui.horizontal(|ui| {
        if ui.small_button("📦 Sahneden Prefab Oluştur").clicked() {
            save_selection_as_prefab(state);
        }
    });
}

/// Asks for the selection's primary — the one the inspector shows — to be saved as a prefab in the
/// folder on show.
///
/// It took any member of the selection set, and with several selected that was whichever one the
/// hash put first.
fn save_selection_as_prefab(state: &mut EditorState) {
    if let Some(selected) = state.primary_selection() {
        // From the entity's numbers: its `Display` (`Entity(3:0)`) holds a colon, which NTFS
        // takes for an alternate data stream.
        let stem = format!("prefab_{}v{}", selected.id(), selected.generation());
        let path = Path::new(&state.assets.root).join(prefab_file_name(&stem));
        state.prefab_save_request = Some((selected, path.to_string_lossy().to_string()));
    } else {
        state.log_warning("Önce bir entity seçin.");
    }
}

/// The tiles of `state.assets.root`'s entries that pass the filters.
fn asset_grid(ui: &mut egui::Ui, state: &mut EditorState) {
    let root = std::path::PathBuf::from(&state.assets.root);
    if !root.exists() || !root.is_dir() {
        ui.label(
            egui::RichText::new("⚠ Asset dizini bulunamadı").color(egui::Color32::YELLOW),
        );
        return;
    }

    let entries = match take_listing(&mut state.assets, &root) {
        Ok(entries) => entries,
        Err(e) => {
            ui.label(
                egui::RichText::new(format!("⚠ Klasör okunamadı: {e}")).color(egui::Color32::YELLOW),
            );
            return;
        }
    };

    let filter_lower = state.assets.filter.to_lowercase();

    for (path, name, is_dir) in &entries {
        // Filtre
        if !entry_passes(name, *is_dir, &filter_lower, state.assets.kind_filter) {
            continue;
        }
        asset_tile(ui, state, path, name, *is_dir);
    }

    // The listing goes back into the cache it was taken from. A tile may have moved the browser to
    // another folder meanwhile; then the cache is stale and the next frame reads the new one.
    if let Some((_, _, cache)) = &mut state.assets.cached_dir {
        if cache.is_empty() {
            *cache = entries;
        }
    }
}

/// `root`'s entries, folders first and then by name — from the cache while it is under a second
/// old, read again otherwise.
///
/// Taken OUT of the cache, so the grid can walk it while its tiles change the state; the grid puts
/// it back. It was cloned instead, every entry's path and name, every frame the browser was open.
///
/// A folder that cannot be read is an error, never the listing of another folder: a failed read
/// kept the cache, and an unreadable folder showed the files of the one before it under its own
/// path.
fn take_listing(
    assets: &mut crate::editor_state::AssetBrowserState,
    root: &Path,
) -> std::io::Result<Vec<(std::path::PathBuf, String, bool)>> {
    let now = Instant::now();
    let fresh = matches!(
        &assets.cached_dir,
        Some((cached_path, last_update, _))
            if cached_path == &assets.root && now.duration_since(*last_update).as_secs_f32() < 1.0
    );
    if !fresh {
        assets.cached_dir = None;
        let mut file_cache = Vec::new();
        for entry in std::fs::read_dir(root)?.filter_map(|e| e.ok()) {
            let is_dir = entry.path().is_dir();
            let name = entry.file_name().to_string_lossy().to_string();
            file_cache.push((entry.path(), name, is_dir));
        }
        file_cache.sort_by(|a, b| b.2.cmp(&a.2).then(a.1.cmp(&b.1)));
        assets.cached_dir = Some((assets.root.clone(), now, file_cache));
    }
    Ok(assets.cached_dir.as_mut().map(|(_, _, cache)| std::mem::take(cache)).unwrap_or_default())
}

/// One entry's tile: its icon and short name, and what a click, a double click, a drag and a right
/// click on it do.
fn asset_tile(ui: &mut egui::Ui, state: &mut EditorState, path: &Path, name: &str, is_dir: bool) {
    let kind = entry_kind(name, is_dir);
    let path_str = path.to_string_lossy().to_string();

    ui.vertical(|ui| {
        ui.set_width(82.0);

        let btn_text = egui::RichText::new(kind.icon()).size(30.0).color(icon_color(kind));
        let response = ui.add(
            egui::Button::new(btn_text)
                .min_size(egui::vec2(72.0, 52.0))
                .fill(egui::Color32::from_rgba_premultiplied(30, 30, 30, 180)),
        );

        // Tooltip (Reassign response because on_hover_text consumes it)
        let response = response.on_hover_text(format!("{}\n{}", name, tile_hint(kind)));

        // Sağ tık menüsü en sonda atanacak

        // Drag & Drop başlatma (viewport'ta yakalanır)
        let drag_id = egui::Id::new("drag_asset").with(path.as_os_str());
        let drag_response = ui.interact(response.rect, drag_id, egui::Sense::drag());

        if drag_response.drag_started() {
            state.dragged_asset = Some(path_str.clone());
        }

        if response.double_clicked() {
            if is_dir {
                // Klasöre gir (çift tık)
                state.assets.root = path_str.clone();
            }
        } else if response.clicked() {
            open_on_click(state, kind, &path_str, name);
            // Every click selects, whatever else it also did — the detail pane
            // describes what you last touched.
            state.assets.selected = Some(path.to_path_buf());
        }

        ui.label(
            egui::RichText::new(short_name(name))
                .small()
                .color(egui::Color32::from_rgb(200, 200, 200)),
        );

        // Context Menu tüketimini güvenli hale getirmek için scope'un en sonunda çağrılır
        response.context_menu(|ui| tile_menu(ui, state, kind, &path_str, name));
    });
}

/// The icon's colour: folders yellow, prefabs green, scenes blue, scripts orange, the rest grey.
fn icon_color(kind: AssetKind) -> egui::Color32 {
    match kind {
        AssetKind::Folder => egui::Color32::from_rgb(255, 220, 80),
        AssetKind::Prefab => egui::Color32::from_rgb(100, 255, 160),
        AssetKind::Scene => egui::Color32::from_rgb(100, 180, 255),
        AssetKind::Script => egui::Color32::from_rgb(255, 160, 80),
        _ => egui::Color32::from_rgb(200, 200, 200),
    }
}

/// The tooltip's second line: what clicking does.
fn tile_hint(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Prefab => "Tek tık: Sahneye ekle",
        AssetKind::Scene => "Tek tık: Sahneyi yükle",
        AssetKind::Folder => "Çift tık: Klasöre gir",
        _ => "Sağ tık: Seçenekler",
    }
}

/// A single click: a prefab goes into the scene and a scene opens; anything else is only named in
/// the status bar.
fn open_on_click(state: &mut EditorState, kind: AssetKind, path: &str, name: &str) {
    match kind {
        AssetKind::Prefab => {
            // ✅ TEK TIKLA prefab sahneye ekle
            state.add_asset_to_scene(path.to_string(), None);
            state.status_message = format!("Prefab eklendi: {}", name);
        }
        // ✅ TEK TIKLA sahneyi yükle
        AssetKind::Scene => state.request_scene_load(path.to_string()),
        AssetKind::Mesh => state.status_message = format!("Seçilen: {} (Model)", name),
        _ => state.status_message = format!("Seçilen: {}", name),
    }
}

/// A tile's right-click menu.
fn tile_menu(ui: &mut egui::Ui, state: &mut EditorState, kind: AssetKind, path: &str, name: &str) {
    if kind == AssetKind::Mesh && is_importable_model(name) && ui.button("⚙️ Sahneye Ekle").clicked() {
        state.add_asset_to_scene(path.to_string(), None);
        ui.close();
    }
    if kind == AssetKind::Prefab && ui.button("⚙️ Prefab Olarak Ekle").clicked() {
        state.add_asset_to_scene(path.to_string(), None);
        ui.close();
    }
    if kind == AssetKind::Scene && ui.button("📂 Bu Sahneyi Yükle").clicked() {
        state.request_scene_load(path.to_string());
        ui.close();
    }
    if ui.button("📋 Yolu Kopyala").clicked() {
        ui.output_mut(|o| o.commands.push(egui::OutputCommand::CopyText(path.to_string())));
        ui.close();
    }
}

/// The name under a tile: up to 11 characters whole, longer ones cut to 9 and an ellipsis.
fn short_name(name: &str) -> String {
    // Dosya adı (kısa gösterim)
    if name.chars().count() > 11 {
        let truncated: String = name.chars().take(9).collect();
        format!("{}...", truncated)
    } else {
        name.to_string()
    }
}

/// What an asset file is, decided from its extension.
///
/// One table, because the icon and the type filter are the same question asked twice. The icon
/// lookup used to own the extension list on its own; adding the prototype's `All / Mesh / Material
/// / Texture / Audio` filter beside it would have meant a second list that has to agree with the
/// first about whether `.tga` is a texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetKind {
    /// A directory.
    Folder,
    /// Geometry: `.obj`, `.gltf`, `.glb`.
    Mesh,
    /// An image the renderer can sample.
    Texture,
    /// A sound file.
    Audio,
    /// A Lua script.
    Script,
    /// A WGSL shader.
    Shader,
    /// A saved prefab.
    Prefab,
    /// A saved scene.
    Scene,
    /// Structured data — JSON, RON, TOML.
    Data,
    /// An asset's `.meta` sidecar. A kind of its own so the browser can leave the 22 of them out
    /// of the grid instead of rendering each as an anonymous grey tile beside the file it belongs
    /// to.
    Meta,
    /// Anything with an extension this table does not know.
    Other,
}

impl AssetKind {
    /// The glyph the browser shows for this kind.
    pub fn icon(self) -> &'static str {
        match self {
            Self::Folder => "📁",
            Self::Mesh => "🗿",
            Self::Texture => "🖼️",
            Self::Audio => "🔊",
            Self::Script => "📜",
            Self::Shader => "🎨",
            Self::Prefab => "📦",
            Self::Scene => "🎬",
            Self::Data => "📋",
            Self::Meta => "🏷",
            Self::Other => "📄",
        }
    }

    /// The filter chips, in the prototype's order. `None` is "All".
    ///
    /// Shader, Prefab, Scene and Data are deliberately absent: the prototype offers five chips and
    /// a row of ten would stop being a filter and start being a second navigation tree. They are
    /// still reachable through the text filter.
    pub const CHIPS: [(Option<AssetKind>, &'static str); 5] = [
        (None, "All"),
        (Some(AssetKind::Mesh), "Mesh"),
        (Some(AssetKind::Prefab), "Prefab"),
        (Some(AssetKind::Texture), "Texture"),
        (Some(AssetKind::Audio), "Audio"),
    ];
}

/// Classifies a file name. The single extension table.
///
/// A name alone never says "folder": a name without an extension is [`AssetKind::Other`], and only
/// [`entry_kind`], which is told, knows a folder.
pub fn asset_kind(filename: &str) -> AssetKind {
    let ext = filename.rsplit('.').next().unwrap_or("");
    let is = |c: &str| ext.eq_ignore_ascii_case(c);

    if is("obj") || is("glb") || is("gltf") || is("fbx") {
        AssetKind::Mesh
    } else if is("jpg") || is("jpeg") || is("png") || is("bmp") || is("tga") {
        AssetKind::Texture
    } else if is("wav") || is("ogg") || is("mp3") || is("flac") {
        AssetKind::Audio
    } else if is("lua") {
        AssetKind::Script
    } else if is("json") || is("toml") || is("ron") {
        AssetKind::Data
    } else if is("meta") {
        AssetKind::Meta
    } else if is("prefab") {
        AssetKind::Prefab
    } else if is("gizmo") || is("giz") {
        AssetKind::Scene
    } else if is("wgsl") || is("glsl") || is("hlsl") {
        AssetKind::Shader
    } else {
        AssetKind::Other
    }
}

/// What a directory entry is: a folder when it is one, whatever its name, and what its extension
/// says otherwise.
///
/// The browser asked the name alone. A folder called `v1.2` was a grey file tile, a file called
/// `LICENSE` a folder, and a single click on a folder called `door.prefab` instantiated it as a
/// prefab.
pub fn entry_kind(name: &str, is_dir: bool) -> AssetKind {
    if is_dir {
        AssetKind::Folder
    } else {
        asset_kind(name)
    }
}

/// Can the model importer read this file? `.obj`, `.gltf` and `.glb`.
///
/// Narrower than [`AssetKind::Mesh`] on purpose: an `.fbx` is geometry, and the browser shows it
/// with the meshes, but nothing in the engine reads one.
pub fn is_importable_model(filename: &str) -> bool {
    let ext = filename.rsplit('.').next().unwrap_or("");
    ["obj", "gltf", "glb"].iter().any(|e| ext.eq_ignore_ascii_case(e))
}

/// Does one directory entry survive the browser's two filters?
///
/// Extracted so it can be tested: the rules are small but each has a way of being wrong that only
/// shows up in use — a text filter that is case-sensitive, or a type filter that hides the folder
/// you need in order to leave the directory.
///
/// `text_filter` is expected already lowercased; the caller computes it once per frame rather than
/// per entry.
pub fn entry_passes(
    name: &str,
    is_dir: bool,
    text_filter: &str,
    kind_filter: Option<AssetKind>,
) -> bool {
    if !text_filter.is_empty() && !name.to_lowercase().contains(text_filter) {
        return false;
    }
    // Sidecars are never listed. They are bookkeeping that belongs to the file next to them, and
    // the detail pane shows their one field (the uuid) on that file's own row.
    if !is_dir && asset_kind(name) == AssetKind::Meta {
        return false;
    }
    match kind_filter {
        // Folders always survive the type chip. Filtering them out strands you in a directory with
        // no way back up, and the prototype keeps its folder tree visible under every chip.
        Some(want) => is_dir || asset_kind(name) == want,
        None => true,
    }
}

/// Width of the folder tree column.
const TREE_WIDTH: f32 = 150.0;
/// How long a directory listing in the tree is trusted before it is re-read.
const TREE_CACHE_SECS: f32 = 1.0;
/// How deep the tree will recurse. A symlink pointing at one of its own ancestors is a cycle, and
/// `read_dir` follows it happily; the panel would recurse until the stack ran out.
const TREE_MAX_DEPTH: usize = 12;
/// How many subfolders one directory contributes. `target/` in a Rust workspace holds thousands,
/// and a tree node is not a thing you want thousands of. What is dropped is *said* — see the
/// `… +N more` row — because a silently truncated tree reads as a complete one.
const TREE_MAX_CHILDREN: usize = 200;

/// Where the folder tree should be rooted, given the workspace and where the grid currently is.
///
/// Normally the workspace, unchanged. The one rule: the back button walks to the parent directory
/// with **no floor** — press it enough times and you are at `/` — so the grid can leave the
/// workspace entirely. A tree that did not follow would be showing a folder that no longer
/// contains you, with nothing highlighted and no way back down to where you are. So walking out
/// of the workspace makes wherever you landed the new one.
pub(crate) fn tree_root_for(workspace: &str, current: &str) -> String {
    if Path::new(current).starts_with(workspace) {
        workspace.to_string()
    } else {
        current.to_string()
    }
}

/// Is this directory one the tree should show?
///
/// Dot-directories are out: `.git` alone is thousands of nodes of plumbing, and no asset lives in
/// one. This is a rule about the leading dot rather than a list of names — a list would have to be
/// kept in step with every tool the user's project happens to use.
pub(crate) fn tree_dir_is_listable(name: &str) -> bool {
    !name.starts_with('.')
}

/// The tree's subfolders for one directory, through the cache.
fn cached_subfolders(
    state: &mut EditorState,
    dir: &Path,
) -> (Vec<(std::path::PathBuf, String)>, usize) {
    let now = Instant::now();
    if let Some((read_at, kids, total)) = state.assets.tree_cache.get(dir) {
        if now.duration_since(*read_at).as_secs_f32() < TREE_CACHE_SECS {
            return (kids.clone(), *total);
        }
    }

    let mut kids: Vec<(std::path::PathBuf, String)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if !path.is_dir() {
                continue; // files are the grid's job; this column is folders
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if !tree_dir_is_listable(&name) {
                continue;
            }
            kids.push((path, name));
        }
    }
    kids.sort_by(|a, b| a.1.cmp(&b.1));
    let total = kids.len();
    kids.truncate(TREE_MAX_CHILDREN);
    state
        .assets
        .tree_cache
        .insert(dir.to_path_buf(), (now, kids.clone(), total));
    (kids, total)
}

/// The prototype's folder tree — the browser's left column.
///
/// # What a click does
///
/// Navigates the grid *and* expands the node, both from the one click on the folder's name. The
/// two are the same intention here: you open a folder to see what is in it, and "what is in it" is
/// files (the grid) and folders (the branch). Making the arrow the only way to expand would mean
/// two targets 12 px apart that both mean "open this".
///
/// The folder the grid is showing is drawn in the accent, so the tree answers "where am I" without
/// being clicked.
fn draw_folder_tree(ui: &mut egui::Ui, state: &mut EditorState) {
    use crate::theme::palette::*;

    ui.label(
        egui::RichText::new("FOLDERS")
            .size(10.0)
            .color(TEXT_MUTED),
    );
    ui.separator();

    let root = std::path::PathBuf::from(state.assets.workspace_root.clone());
    let root_name = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| state.assets.workspace_root.clone());

    egui::ScrollArea::vertical().show(ui, |ui| {
        // The workspace itself is a row, so there is somewhere to click to get back to the top.
        let at_root = Path::new(&state.assets.root) == root;
        if ui
            .add(egui::Button::new(
                egui::RichText::new(format!("🗀 {root_name}"))
                    .size(11.0)
                    .color(if at_root { ACCENT } else { TEXT_BODY }),
            )
            .frame(false))
            .clicked()
        {
            state.assets.root = root.to_string_lossy().to_string();
        }
        tree_children(ui, state, &root, 0);
    });
}

/// One level of the tree. Recurses only into open nodes — `CollapsingHeader` does not run this
/// body while it is closed, which is what keeps an unexpanded `target/` from ever being read.
fn tree_children(ui: &mut egui::Ui, state: &mut EditorState, dir: &Path, depth: usize) {
    use crate::theme::palette::*;

    if depth >= TREE_MAX_DEPTH {
        ui.label(
            egui::RichText::new("… too deep")
                .size(10.0)
                .color(TEXT_DIM),
        );
        return;
    }

    let (kids, total) = cached_subfolders(state, dir);
    for (path, name) in kids {
        let current = Path::new(&state.assets.root) == path;
        let header = egui::RichText::new(&name)
            .size(11.0)
            .color(if current { ACCENT } else { TEXT_BODY });

        let response = egui::CollapsingHeader::new(header)
            .id_salt(&path)
            .default_open(false)
            .show(ui, |ui| {
                tree_children(ui, state, &path, depth + 1);
            });

        if response.header_response.clicked() {
            state.assets.root = path.to_string_lossy().to_string();
        }
    }

    if total > TREE_MAX_CHILDREN {
        ui.label(
            egui::RichText::new(format!("… +{} more", total - TREE_MAX_CHILDREN))
                .size(10.0)
                .color(TEXT_DIM),
        );
    }
}

/// The prototype's asset detail pane — four fields, and the fifth deliberately absent.
///
/// # Why four
///
/// The design shows `type / size / detail / folder / guid`. Four of those are measurable here and
/// one is not: `detail` means image dimensions or a triangle count, which needs a decode and a
/// dependency (`image`) this crate does not have. This project's rule is that a panel must not
/// print a value it did not measure, so the row is missing rather than blank-or-guessed.
///
/// # The guid row is a reader
///
/// It shows the `.meta` sidecar that already exists beside the file — 22 of them are committed —
/// and an em dash when there is none. It never creates one. `AssetManager::new` does mint sidecars
/// while scanning, but that is a scan of the `assets/` tree; minting identity because a user
/// clicked a thumbnail would stamp UUIDs onto files they merely looked at.
fn draw_asset_detail(ui: &mut egui::Ui, state: &mut EditorState) {
    use crate::theme::palette::*;

    let Some(path) = state.assets.selected.clone() else {
        return;
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    ui.label(
        egui::RichText::new(name.to_uppercase())
            .size(10.0)
            .color(TEXT_BRIGHT)
            .strong(),
    );

    let row = |ui: &mut egui::Ui, label: &str, value: String, dim: bool| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(label).size(10.0).color(TEXT_DIM));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(value)
                        .size(10.0)
                        .color(if dim { TEXT_DIM } else { TEXT_BODY }),
                );
            });
        });
    };

    row(ui, "type", format!("{:?}", entry_kind(&name, path.is_dir())).to_lowercase(), false);

    match std::fs::metadata(&path) {
        Ok(m) => row(ui, "size", format_size(m.len()), false),
        // The file was listed a moment ago and is not there now — say so rather than showing 0 B.
        Err(_) => row(ui, "size", "okunamadı".to_string(), true),
    }

    row(
        ui,
        "folder",
        path.parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| ".".to_string()),
        false,
    );

    match gizmo_renderer::asset::read_asset_meta(&path) {
        Some(meta) => row(ui, "guid", meta.uuid.to_string(), false),
        None => row(ui, "guid", "—".to_string(), true),
    }
}

/// Bytes as B / KB / MB, for the detail pane's `size` row.
fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    let b = bytes as f64;
    if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    /// A prefab's file name keeps what a name is made of and nothing a file system refuses: an
    /// entity's `Display` (`Entity(3:0)`) once went in, and on NTFS its colon opened a data stream.
    #[test]
    fn a_prefab_file_name_is_valid_everywhere() {
        use super::prefab_file_name;
        assert_eq!(prefab_file_name("Directional Light"), "Directional_Light.prefab");
        assert_eq!(prefab_file_name("Entity(3:0)"), "Entity(3_0).prefab");
        assert_eq!(prefab_file_name("a/b\\c?*"), "a_b_c__.prefab");
        assert_eq!(prefab_file_name("Kale Kapısı"), "Kale_Kapısı.prefab");
    }

    #[test]
    fn sidecars_are_a_kind_and_never_listed() {
        assert_eq!(asset_kind("grass.jpg.meta"), AssetKind::Meta);
        assert!(
            !entry_passes("grass.jpg.meta", false, "", None),
            "the 22 committed sidecars must not appear as tiles beside the files they belong to"
        );
        // Only the last extension counts, so the file itself is unaffected.
        assert_eq!(asset_kind("grass.jpg"), AssetKind::Texture);
        assert!(entry_passes("grass.jpg", false, "", None));
    }

    /// A folder called `meta` is a folder, not a sidecar.
    #[test]
    fn a_directory_named_meta_still_lists() {
        assert!(entry_passes("meta", true, "", None));
    }

    #[test]
    fn sizes_read_as_bytes_kilobytes_or_megabytes() {
        use super::format_size;
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(2048), "2 KB");
        assert_eq!(format_size(5 * 1024 * 1024), "5.0 MB");
    }

    use super::{asset_kind, entry_passes, AssetKind};

    /// The browser's "prefab from the scene" saves the primary — the entity the inspector shows —
    /// and not whichever member of the selection set the hash puts first.
    #[test]
    fn a_prefab_from_the_selection_is_the_primary() {
        use gizmo_core::entity::Entity;
        let mut state = crate::EditorState::default();
        let ids: Vec<Entity> = (1..=8).map(|i| Entity::new(i, 0)).collect();
        state.select_exclusive(ids[0]);
        for &e in &ids[1..] {
            state.toggle_selection(e);
        }
        let primary = state.selection.primary.expect("the last one clicked is the primary");
        super::save_selection_as_prefab(&mut state);
        let (saved, path) = state.prefab_save_request.take().expect("a prefab was asked for");
        assert_eq!(saved, primary, "with eight selected the prefab was of {saved:?}, not the primary");
        assert!(path.ends_with(&format!("prefab_{}v0.prefab", primary.id())), "{path}");
    }

    /// An asset goes into the scene where it was dropped, and one that was not dropped somewhere
    /// goes nowhere in particular — not to the spot the previous drop on the viewport left behind.
    #[test]
    fn an_asset_lands_where_it_was_dropped_and_not_where_the_last_one_was() {
        use gizmo_math::Vec3;
        let mut state = crate::EditorState::default();
        let here = Vec3::new(3.0, 0.0, -2.0);
        state.add_asset_to_scene("props/chair.glb".into(), Some(here));
        assert_eq!(state.gltf_load_request.take(), Some(("props/chair.glb".to_string(), Some(here))));
        // Dropped on the hierarchy, or added from the browser's menu.
        state.add_asset_to_scene("props/table.GLB".into(), None);
        assert_eq!(state.gltf_load_request.take(), Some(("props/table.GLB".to_string(), None)));
        state.add_asset_to_scene("door.prefab".into(), Some(here));
        assert_eq!(state.prefab_load_request.take(), Some(("door.prefab".to_string(), None, Some(here))));
        // Geometry the browser lists with the meshes, but that nothing in the engine reads.
        state.add_asset_to_scene("rig.fbx".into(), None);
        assert_eq!(state.gltf_load_request, None);
    }

    /// Every way of opening a scene asks first when there is unsaved work: the browser's context
    /// menu and a scene dropped on the viewport loaded it straight away and lost that work.
    #[test]
    fn every_way_of_opening_a_scene_asks_about_unsaved_work() {
        let mut state = crate::EditorState { has_unsaved_changes: true, ..Default::default() };
        state.add_asset_to_scene("levels/forest.gizmo".into(), None);
        assert_eq!(state.scene.load_confirm_dialog.as_deref(), Some("levels/forest.gizmo"));
        assert_eq!(state.scene.load_request, None);
        state.scene.load_confirm_dialog = None;
        state.request_scene_load("levels/forest.giz".into());
        assert_eq!(state.scene.load_confirm_dialog.as_deref(), Some("levels/forest.giz"));
        assert_eq!(state.scene.load_request, None);

        state.scene.load_confirm_dialog = None;
        state.has_unsaved_changes = false;
        state.request_scene_load("levels/forest.gizmo".into());
        assert_eq!(state.scene.load_request.as_deref(), Some("levels/forest.gizmo"));
        assert_eq!(state.status_message, "Sahne yükleniyor: forest.gizmo");
    }

    /// The texts a browser frame painted.
    fn painted_texts(state: &mut crate::EditorState) -> Vec<String> {
        let ctx = egui::Context::default();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let mut panel = ui.new_child(egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(900.0, 500.0),
            )));
            super::ui_asset_browser(&mut panel, state);
        });
        fn collect(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
                egui::Shape::Text(text) => out.push(text.galley.text().to_owned()),
                _ => {}
            }
        }
        let mut texts = Vec::new();
        output.shapes.iter().for_each(|s| collect(&s.shape, &mut texts));
        output.drop_without_applying_deltas();
        texts
    }

    /// A folder that cannot be read says so. It showed the files of the folder before it, under
    /// its own path: the failed read kept the old listing in the cache.
    #[cfg(unix)]
    #[test]
    fn an_unreadable_folder_does_not_show_the_previous_one() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("gizmo_unreadable_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (open, shut) = (dir.join("open"), dir.join("shut"));
        std::fs::create_dir_all(&open).unwrap();
        std::fs::create_dir_all(&shut).unwrap();
        std::fs::write(open.join("grass.png"), b"x").unwrap();
        std::fs::set_permissions(&shut, std::fs::Permissions::from_mode(0o000)).unwrap();
        let readable_anyway = std::fs::read_dir(&shut).is_ok(); // root reads everything

        let mut state = crate::EditorState::default();
        state.assets.root = open.to_string_lossy().to_string();
        state.assets.workspace_root = state.assets.root.clone();
        assert!(painted_texts(&mut state).iter().any(|t| t == "grass.png"), "the readable folder lists");
        state.assets.root = shut.to_string_lossy().to_string();
        let texts = painted_texts(&mut state);

        std::fs::set_permissions(&shut, std::fs::Permissions::from_mode(0o755)).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        if readable_anyway {
            return;
        }
        assert!(!texts.iter().any(|t| t == "grass.png"), "the unreadable folder showed the other one's files: {texts:?}");
        assert!(texts.iter().any(|t| t.starts_with("⚠ Klasör okunamadı")), "{texts:?}");
    }

    use super::{cached_subfolders, tree_dir_is_listable, tree_root_for, TREE_MAX_CHILDREN};

    /// The tree is rooted at the workspace — until the back button walks above it, which it can,
    /// because that button has no floor. Then the tree has to follow, or it is showing a folder
    /// that no longer contains you.
    #[test]
    fn the_tree_follows_you_out_of_the_workspace_and_not_into_it() {
        // Staying inside keeps the workspace, at every depth.
        assert_eq!(tree_root_for("demo/assets", "demo/assets"), "demo/assets");
        assert_eq!(tree_root_for("demo/assets", "demo/assets/textures"), "demo/assets");
        assert_eq!(
            tree_root_for("demo/assets", "demo/assets/textures/pbr/wood"),
            "demo/assets"
        );
        // Walking out of it re-roots, so the current folder is always in the tree.
        assert_eq!(tree_root_for("demo/assets", "demo"), "demo");
        // ...and so does picking an unrelated workspace, which is the same thing.
        assert_eq!(tree_root_for("demo/assets", "/srv/game"), "/srv/game");
    }

    /// A sibling whose name merely *starts with* the workspace's is not inside it. `starts_with`
    /// on a `Path` compares components, and the whole rule depends on that being true.
    #[test]
    fn a_sibling_with_a_shared_prefix_is_not_inside_the_workspace() {
        assert_eq!(
            tree_root_for("demo/assets", "demo/assets_old"),
            "demo/assets_old",
            "demo/assets_old is a sibling of demo/assets, not a child of it"
        );
    }

    #[test]
    fn dot_directories_stay_out_of_the_tree() {
        assert!(!tree_dir_is_listable(".git"));
        assert!(!tree_dir_is_listable(".cache"));
        assert!(tree_dir_is_listable("textures"));
        // The rule is the leading dot, not the dot: a folder can have one in the middle.
        assert!(tree_dir_is_listable("v1.2"));
    }

    /// The listing itself, against a real directory: folders only, sorted, dot-dirs dropped, and
    /// the cap counted rather than silently applied.
    #[test]
    fn the_listing_is_folders_only_sorted_and_counted() {
        let dir = std::env::temp_dir().join(format!("gizmo_tree_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("zebra")).unwrap();
        std::fs::create_dir_all(dir.join("apple")).unwrap();
        std::fs::create_dir_all(dir.join(".hidden")).unwrap();
        std::fs::write(dir.join("a_file.png"), b"not a folder").unwrap();

        let mut state = crate::EditorState::new();
        let (kids, total) = cached_subfolders(&mut state, &dir);

        let names: Vec<&str> = kids.iter().map(|(_, n)| n.as_str()).collect();
        assert_eq!(names, vec!["apple", "zebra"], "folders only, alphabetical");
        assert_eq!(total, 2, "the total is what survived the filter, before the cap");
        assert!(total <= TREE_MAX_CHILDREN, "nothing was dropped here");

        // A second call inside the cache window must not re-read the directory. Prove it by
        // changing the directory underneath and watching the answer NOT change.
        std::fs::create_dir_all(dir.join("mango")).unwrap();
        let (again, _) = cached_subfolders(&mut state, &dir);
        assert_eq!(again.len(), 2, "the second read came from the cache");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A directory that cannot be read is empty, not a panic. The tree walks whatever the user
    /// points it at, including folders they have no permission for.
    #[test]
    fn an_unreadable_directory_lists_as_empty() {
        let mut state = crate::EditorState::new();
        let (kids, total) = cached_subfolders(
            &mut state,
            std::path::Path::new("/definitely/not/a/directory/here"),
        );
        assert!(kids.is_empty());
        assert_eq!(total, 0);
    }


    #[test]
    fn the_type_chip_keeps_its_kind_and_every_folder() {
        assert!(entry_passes("hero.glb", false, "", Some(AssetKind::Mesh)));
        assert!(!entry_passes("hero.png", false, "", Some(AssetKind::Mesh)));
        assert!(
            entry_passes("textures", true, "", Some(AssetKind::Mesh)),
            "a folder must survive every type chip, or the filter traps you in the directory"
        );
        assert!(entry_passes("anything.xyz", false, "", None), "All filters nothing");
    }

    #[test]
    fn the_text_filter_is_case_insensitive_and_combines_with_the_chip() {
        assert!(entry_passes("Hero.GLB", false, "hero", None));
        assert!(!entry_passes("Hero.GLB", false, "villain", None));
        // Both filters have to pass.
        assert!(entry_passes("Hero.GLB", false, "hero", Some(AssetKind::Mesh)));
        assert!(!entry_passes("Hero.PNG", false, "hero", Some(AssetKind::Mesh)));
    }

    /// The chips cover kinds that `asset_kind` can actually produce.
    ///
    /// A chip for a kind nothing classifies as would filter every file away and look broken.
    #[test]
    fn every_chip_is_a_kind_something_can_be() {
        for (kind, label) in AssetKind::CHIPS {
            let Some(kind) = kind else { continue }; // "All"
            let sample = match kind {
                AssetKind::Mesh => "a.glb",
                AssetKind::Texture => "a.png",
                AssetKind::Audio => "a.wav",
                AssetKind::Prefab => "a.prefab",
                other => panic!("chip {label} is {other:?}, which this test has no sample for"),
            };
            assert_eq!(asset_kind(sample), kind, "chip {label} classifies nothing");
        }
    }

    fn get_file_icon(name: &str) -> &'static str {
        asset_kind(name).icon()
    }

    #[test]
    fn model_extensions_map_to_statue() {
        assert_eq!(get_file_icon("chair.obj"), "🗿");
        assert_eq!(get_file_icon("scene.glb"), "🗿");
        assert_eq!(get_file_icon("scene.gltf"), "🗿");
        assert_eq!(get_file_icon("rig.fbx"), "🗿");
    }

    #[test]
    fn image_audio_script_data_extensions() {
        assert_eq!(get_file_icon("albedo.png"), "🖼️");
        assert_eq!(get_file_icon("hit.wav"), "🔊");
        assert_eq!(get_file_icon("ai.lua"), "📜");
        assert_eq!(get_file_icon("prefs.toml"), "📋");
        assert_eq!(get_file_icon("data.json"), "📋");
        assert_eq!(get_file_icon("box.prefab"), "📦");
        assert_eq!(get_file_icon("level.gizmo"), "🎬");
        assert_eq!(get_file_icon("level.giz"), "🎬");
        assert_eq!(get_file_icon("pbr.wgsl"), "🎨");
    }

    /// Extension matching must be case-INSENSITIVE.
    #[test]
    fn extension_matching_is_case_insensitive() {
        assert_eq!(get_file_icon("MODEL.OBJ"), "🗿");
        assert_eq!(get_file_icon("Photo.PNG"), "🖼️");
        assert_eq!(get_file_icon("Sound.Wav"), "🔊");
        assert_eq!(get_file_icon("SHADER.WGSL"), "🎨");
    }

    /// A folder is known by being one, not by its name — and a file without an extension is a
    /// file. The name alone was asked: `LICENSE` was a folder, a folder `v1.2` a file, and a folder
    /// `door.prefab` a prefab that a click instantiated.
    #[test]
    fn a_folder_is_known_by_being_one_not_by_its_name() {
        use super::entry_kind;
        assert_eq!(entry_kind("assets", true), AssetKind::Folder);
        assert_eq!(entry_kind("v1.2", true), AssetKind::Folder);
        assert_eq!(entry_kind("door.prefab", true), AssetKind::Folder);
        assert_eq!(entry_kind("LICENSE", false), AssetKind::Other);
        assert_eq!(entry_kind("door.prefab", false), AssetKind::Prefab);
        assert_eq!(get_file_icon(""), "📄");
    }

    /// An unknown but dotted extension gets the generic file icon.
    #[test]
    fn unknown_extension_with_dot_is_generic_file() {
        assert_eq!(get_file_icon("notes.txt"), "📄");
        assert_eq!(get_file_icon("archive.xyz"), "📄");
    }

    /// In a multi-dotted name ONLY the last extension counts.
    #[test]
    fn only_last_extension_matters() {
        // son parça "gz" → bilinmeyen ama noktalı → jenerik dosya
        assert_eq!(get_file_icon("world.tar.gz"), "📄");
        // son parça "png" → görsel
        assert_eq!(get_file_icon("my.backup.png"), "🖼️");
    }

    /// A dotfile: rsplit takes the first piece as "gitignore", which is not a known extension,
    /// but the name does contain a dot → generic file icon.
    #[test]
    fn leading_dot_dotfile_is_generic_file() {
        assert_eq!(get_file_icon(".gitignore"), "📄");
    }
}
