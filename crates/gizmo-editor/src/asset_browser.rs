//! The asset browser — shows the project's files in the bottom panel.

use crate::editor_state::EditorState;
use egui;
use std::path::Path;

#[cfg(target_arch = "wasm32")]
use web_time::Instant;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

/// Draws the asset browser tab.
pub fn ui_asset_browser(ui: &mut egui::Ui, state: &mut EditorState) {
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

    ui.horizontal(|ui| {
        ui.heading("🗂️ Asset Browser");
        ui.separator();

        // Geri git
        if ui.button("⬅").on_hover_text("Üst Dizin (Geri)").clicked() {
            if let Some(parent) = Path::new(&state.assets.root).parent() {
                state.assets.root = parent.to_string_lossy().to_string();
            }
        }

        // Workspace seçici
        if state.assets.workspace_rx.is_none() {
            if ui
                .button("📁 Workspace Aç")
                .on_hover_text("Bilgisayardan bir çalışma dizini seçin")
                .clicked()
            {
                let (tx, rx) = std::sync::mpsc::channel();
                state.assets.workspace_rx = Some(std::sync::Mutex::new(rx));
                std::thread::spawn(move || {
                    #[cfg(not(target_arch = "wasm32"))]
                    if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                        let _ = tx.send(folder.to_string_lossy().to_string());
                    }
                    #[cfg(target_arch = "wasm32")]
                    let _ = tx.send("".to_string());
                });
            }
        } else {
            let _ = ui
                .add_enabled(false, egui::Button::new("📁 Workspace Aç"))
                .on_hover_text("Dizin seçimi bekleniyor...");
        }

        ui.separator();

        ui.add(
            egui::TextEdit::singleline(&mut state.assets.filter)
                .desired_width(140.0)
                .hint_text("Filter assets…"),
        );
        ui.separator();

    });

    // The type chips get their own row, as in the prototype — a 28 px filter strip under the
    // header. Squeezed onto the end of the header they simply ran off the right edge of the panel.
    ui.horizontal(|ui| {
        for (kind, label) in AssetKind::CHIPS {
            let mut on = state.assets.kind_filter == kind;
            if crate::theme::toggle(ui, &mut on, label) {
                // Clicking the active chip turns it off, which lands back on All.
                state.assets.kind_filter = if on { kind } else { None };
            }
        }
    });

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

    // Hızlı aksiyon butonu satırı
    ui.horizontal(|ui| {
        if ui.small_button("📦 Sahneden Prefab Oluştur").clicked() {
            if let Some(&selected) = state.selection.entities.iter().next() {
                let path =
                    Path::new(&state.assets.root).join(format!("prefab_{}.prefab", selected));
                state.prefab_save_request = Some((selected, path.to_string_lossy().to_string()));
            } else {
                state.log_warning("Önce bir entity seçin.");
            }
        }
    });

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
        ui.horizontal_wrapped(|ui| {
            let root = Path::new(&state.assets.root);
            if !root.exists() || !root.is_dir() {
                ui.label(
                    egui::RichText::new("⚠ Asset dizini bulunamadı").color(egui::Color32::YELLOW),
                );
                return;
            }

            let now = Instant::now();
            let mut need_refresh = true;
            if let Some((cached_path, last_update, _)) = &state.assets.cached_dir {
                if cached_path == &state.assets.root
                    && now.duration_since(*last_update).as_secs_f32() < 1.0
                {
                    need_refresh = false;
                }
            }

            if need_refresh {
                if let Ok(entries) = std::fs::read_dir(root) {
                    let mut file_cache = Vec::new();
                    for entry in entries.filter_map(|e| e.ok()) {
                        let is_dir = entry.path().is_dir();
                        let name = entry.file_name().to_string_lossy().to_string();
                        file_cache.push((entry.path(), name, is_dir));
                    }
                    file_cache.sort_by(|a, b| b.2.cmp(&a.2).then(a.1.cmp(&b.1)));
                    state.assets.cached_dir = Some((state.assets.root.clone(), now, file_cache));
                }
            }

            let file_entries = if let Some((_, _, cache)) = &state.assets.cached_dir {
                cache.clone()
            } else {
                return;
            };

            let filter_lower = state.assets.filter.to_lowercase();

            for (path, name, is_dir) in file_entries {
                // Filtre
                if !entry_passes(&name, is_dir, &filter_lower, state.assets.kind_filter) {
                    continue;
                }

                let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
                let is_prefab = ext == "prefab";
                let is_scene = ext == "gizmo" || ext == "giz";
                let is_model = ext == "glb" || ext == "gltf" || ext == "obj";
                let is_script = ext == "lua";

                let icon = get_file_icon(&name);
                let path_str = path.to_string_lossy().to_string();

                ui.vertical(|ui| {
                    ui.set_width(82.0);

                    // Renk: prefab=yeşilimsi, scene=mavimsi, dir=sarı, script=turuncu
                    let icon_color = if is_dir {
                        egui::Color32::from_rgb(255, 220, 80)
                    } else if is_prefab {
                        egui::Color32::from_rgb(100, 255, 160)
                    } else if is_scene {
                        egui::Color32::from_rgb(100, 180, 255)
                    } else if is_script {
                        egui::Color32::from_rgb(255, 160, 80)
                    } else {
                        egui::Color32::from_rgb(200, 200, 200)
                    };

                    let btn_text = egui::RichText::new(icon).size(30.0).color(icon_color);
                    let response = ui.add(
                        egui::Button::new(btn_text)
                            .min_size(egui::vec2(72.0, 52.0))
                            .fill(egui::Color32::from_rgba_premultiplied(30, 30, 30, 180)),
                    );

                    // Tooltip (Reassign response because on_hover_text consumes it)
                    let response = response.on_hover_text(format!(
                        "{}\n{}",
                        name,
                        if is_prefab {
                            "Tek tık: Sahneye ekle"
                        } else if is_scene {
                            "Tek tık: Sahneyi yükle"
                        } else if is_dir {
                            "Çift tık: Klasöre gir"
                        } else {
                            "Sağ tık: Seçenekler"
                        }
                    ));

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
                        if is_prefab {
                            // ✅ TEK TIKLA prefab sahneye ekle
                            state.prefab_load_request = Some((path_str.clone(), None, None));
                            state.status_message = format!("Prefab eklendi: {}", name);
                        } else if is_scene {
                            // ✅ TEK TIKLA sahneyi yükle
                            if state.has_unsaved_changes {
                                state.scene.load_confirm_dialog = Some(path_str.clone());
                            } else {
                                state.scene.load_request = Some(path_str.clone());
                                state.status_message = format!("Sahne yükleniyor: {}", name);
                            }
                        } else if is_model {
                            state.status_message = format!("Seçilen: {} (Model)", name);
                        } else {
                            state.status_message = format!("Seçilen: {}", name);
                        }
                        // Every click selects, whatever else it also did — the detail pane
                        // describes what you last touched.
                        state.assets.selected = Some(path.clone());
                    }

                    // Dosya adı (kısa gösterim)
                    let char_count = name.chars().count();
                    let short_name = if char_count > 11 {
                        let truncated: String = name.chars().take(9).collect();
                        format!("{}...", truncated)
                    } else {
                        name.clone()
                    };
                    ui.label(
                        egui::RichText::new(short_name)
                            .small()
                            .color(egui::Color32::from_rgb(200, 200, 200)),
                    );

                    // Context Menu tüketimini güvenli hale getirmek için scope'un en sonunda çağrılır
                    response.context_menu(|ui| {
                        if is_model && ui.button("⚙️ Sahneye Ekle").clicked() {
                            state.spawn_asset_request = Some(path_str.clone());
                            ui.close();
                        }
                        if is_prefab && ui.button("⚙️ Prefab Olarak Ekle").clicked() {
                            state.prefab_load_request = Some((path_str.clone(), None, None));
                            ui.close();
                        }
                        if is_scene && ui.button("📂 Bu Sahneyi Yükle").clicked() {
                            state.scene.load_request = Some(path_str.clone());
                            ui.close();
                        }
                        if ui.button("📋 Yolu Kopyala").clicked() {
                            ui.output_mut(|o| o.commands.push(egui::OutputCommand::CopyText(path_str.clone())));
                            ui.close();
                        }
                    });
                });
            }
        });
    });
}

/// What an asset file is, decided from its extension.
///
/// One table, because the icon and the type filter are the same question asked twice. The icon
/// lookup used to own the extension list on its own; adding the prototype's `All / Mesh / Material
/// / Texture / Audio` filter beside it would have meant a second list that has to agree with the
/// first about whether `.tga` is a texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetKind {
    Folder,
    Mesh,
    Texture,
    Audio,
    Script,
    Shader,
    Prefab,
    Scene,
    Data,
    /// An asset's `.meta` sidecar. A kind of its own so the browser can leave the 22 of them out
    /// of the grid instead of rendering each as an anonymous grey tile beside the file it belongs
    /// to.
    Meta,
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
    } else if filename.contains('.') {
        AssetKind::Other
    } else {
        AssetKind::Folder
    }
}

fn get_file_icon(filename: &str) -> &'static str {
    asset_kind(filename).icon()
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

    row(ui, "type", format!("{:?}", asset_kind(&name)).to_lowercase(), false);

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

    use super::get_file_icon;

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

    /// A name with no extension (no dot) gets the folder icon.
    #[test]
    fn no_extension_is_folder() {
        assert_eq!(get_file_icon("assets"), "📁");
        assert_eq!(get_file_icon(""), "📁");
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
