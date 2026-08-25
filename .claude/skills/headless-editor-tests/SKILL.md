---
name: headless-editor-tests
description: Write or debug tests that drive a real gizmo-editor/gizmo-studio egui frame headlessly — no window, no wgpu, no display — and assert what the frame actually painted (textures, text, colours, content width). Use when adding tests for editor panels, the inspector, the console, the hierarchy or the Game viewport, or when a UI bug needs a number instead of a screenshot.
---

# Testing the editor UI without a window or a GPU

`egui::Context::run_ui` drives a **real** editor frame headlessly — no window, no wgpu, no display.
That makes UI facts assertable instead of screenshot-guessable, and it is fast enough for ordinary
unit tests — measured here, a module driving several full editor frames finishes in 0.04–0.22 s,
most of which is egui building its font atlas once:

```rust
let ctx = egui::Context::default();
let output = ctx.run_ui(egui::RawInput::default(), |ui| {
    draw_editor(ui, &world, &mut state);       // or one panel: ui_console(ui, &mut state)
});
// …inspect `output.shapes`…
output.drop_without_applying_deltas();          // REQUIRED: TexturesDelta panics if just dropped
```

`output.shapes` is what the frame actually painted, and it answers the questions screenshots
can't:

- **Was a texture painted?** `Shape::Mesh(m) => m.texture_id == id` — used to prove the Game panel
  displays its render target (`the_game_panel_paints_the_texture_it_was_given`).
- **What colour was that row?** `Shape::Text(t)` → `t.override_text_color` or the first section's
  `format.color` (`the_console_paints_warnings_and_errors_in_those_colours`).
- **How wide is the content really?** `shape.visual_bounding_rect().max.x` against the panel width.
  Set `ui.set_clip_rect(Rect::EVERYTHING)` first — clipping is what *hides* overflow on screen, so
  measuring unclipped is what turns "looks bitten off" into a number (`inspector_width_tests`).
- **What text is on screen?** collect `Shape::Text` and read `t.galley.text()`
  (`hierarchy_count_tests` reads the header's digits back out).

`Shape::Vec` nests, so every scan needs to recurse or it silently misses most of the frame.

Two things to know when building the state for such a test:

- `EditorState::default()` calls `EditorPrefs::load()` **and** `load_layout()`, which read
  `editor_prefs.toml` and `editor_layout.json` from the config dir and the working directory. A
  test that cares about the dock should overwrite `state.dock_state` with
  `editor_state::create_default_dock_state()` rather than inherit whatever is on disk; a test that
  cares about prefs I/O should use the `*_to(path)` variants instead of pointing `XDG_CONFIG_HOME`
  somewhere (that is process-global, and other tests in the same binary read it in parallel).
- To choose which viewport is on top: `dock_state.find_tab(&tab)` then `set_active_tab(..)`.
