//! The hatch rule, enforced instead of remembered.
//!
//! `docs/API_DEPTH.md` states the rule this file exists for:
//!
//! > **No convenience without a named hatch.** Every function, component or builder that decides
//! > something on the user's behalf must name, in its own documentation, what to reach for when
//! > that decision is wrong.
//!
//! A rule stated in a design document is a rule that is remembered, and `doc_language.rs` is this
//! repository's standing evidence that remembered rules go false silently — its own header records
//! a roadmap claiming a language sweep was finished while 462 lines said otherwise. So the rule
//! gets the same treatment: two things it asserts are checked here, and both of them are checks a
//! *document* would otherwise be trusted for.
//!
//! **1. Every API `API_DEPTH.md` names exists.** The document is where the plan's opened doors are
//! recorded, and a name written there that no longer exists in the tree is worse than no record:
//! it reads as a route. Checked by scanning the document's backticked identifiers and requiring
//! each to appear in some crate's `src/`.
//!
//! **2. Every hatch names the convenience it is a hatch for.** A door is only usable if the person
//! standing at the wall can find it, so each opened route's own documentation has to mention the
//! thing it replaces — `MaterialBuilder` has to say `assemble_material_bind_group`, `SceneView`
//! has to say `global_uniform_buffer`. That link is the rule's operative half, and it is the half
//! that rots first, because the hatch keeps working after the sentence pointing at it is deleted.
//!
//! What this file deliberately does **not** do is try to detect "a convenience" automatically.
//! Every heuristic for that — public functions with defaults, builders, `impl Default` — either
//! floods with false positives or misses the ones that matter, and a test that cries wolf gets
//! `#[ignore]`d within a month. The pairs below are written down, and the cost of writing one down
//! is one line.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// A hatch and the convenience it exists for.
///
/// `hatch` is the item's own name; `must_mention` is what its documentation has to point at, and is
/// deliberately the *old* name — the thing a reader already knows and is stuck behind.
const HATCHES: &[(&str, &str)] = &[
    // The 2026-08-23 API-depth pass. Each of these opened a door that had been shut by nothing.
    ("MaterialBuilder", "assemble_material_bind_group"),
    ("SceneView", "global_uniform_buffer"),
    ("add_update_hook", "set_update"),
    ("VolumetricParams", "shader"),
    ("Phase::User", "position"),
    ("MaterialType::Custom", "MaterialRegistry"),
    ("system_param", "get_access_info"),
    // `Option<P>`'s hatch is the other direction: the parameter is the hatch, and what it replaces
    // is the guard that skips the whole system.
    ("impl<P: SystemParam> SystemParam for Option<P>", "run_if"),
];

/// The line each hatch is declared on, as a prefix.
///
/// Needed because the check has to read the hatch's **own** doc block, not the file. The first
/// version searched the whole file and could not be made to fail: deleting the sentence that
/// points `SceneView` at `global_uniform_buffer` left six other mentions of that field in the
/// same file — the struct field, the selector, its docs — so the test passed while the rule was
/// broken. A test that cannot go red is not evidence.
const HATCH_DECLS: &[(&str, &str)] = &[
    ("MaterialBuilder", "pub struct MaterialBuilder"),
    ("SceneView", "pub struct SceneView"),
    ("add_update_hook", "pub fn add_update_hook"),
    ("VolumetricParams", "pub struct VolumetricParams"),
    ("Phase::User", "User(u16)"),
    ("MaterialType::Custom", "Custom(crate::custom_material::MaterialId)"),
    ("system_param", "macro_rules! system_param"),
    (
        "impl<P: SystemParam> SystemParam for Option<P>",
        "impl<P: SystemParam> SystemParam for Option<P>",
    ),
];

/// Where each hatch's documentation lives.
///
/// Written down rather than searched for, because "find the doc comment attached to this item"
/// needs a parser, and a grep that guesses would fail in exactly the case that matters: a hatch
/// whose docs were moved somewhere they cannot be found.
const HATCH_FILES: &[(&str, &str)] = &[
    ("MaterialBuilder", "crates/gizmo-renderer/src/asset/mod.rs"),
    ("SceneView", "crates/gizmo-renderer/src/pipeline/mod.rs"),
    ("add_update_hook", "crates/gizmo-app/src/windowed/builder.rs"),
    ("VolumetricParams", "crates/gizmo-renderer/src/volumetric.rs"),
    ("Phase::User", "crates/gizmo-core/src/system/mod.rs"),
    (
        "MaterialType::Custom",
        "crates/gizmo-renderer/src/components/material.rs",
    ),
    ("system_param", "crates/gizmo-core/src/system/params.rs"),
    (
        "impl<P: SystemParam> SystemParam for Option<P>",
        "crates/gizmo-core/src/system/params.rs",
    ),
];

/// APIs the plan **proposes** rather than records: named as future work, absent by design.
///
/// This list is the plan's remaining scope, in a form that cannot drift. When one of these lands
/// it comes out of the list and the scan starts requiring it — and if the plan stops proposing it,
/// the entry here is what makes that visible.
const PROPOSED: &[&str] = &[
    // Item 2's second half, alongside the `SystemParam` derive. The last entry in this list:
    // items 1 and 3–7 have all landed, and `MaterialId` left it on 2026-08-23.
    "iter_combinations",
];

/// Backticked names in `API_DEPTH.md` that are prose, not identifiers.
///
/// Each one is here because the scan flagged it and it turned out to be a word in code font rather
/// than something the tree should contain.
const NOT_IDENTIFIERS: &[&str] = &[
    "Custom", "params", "steps", "enabled", "views", "label", "normal", "sampler", "base",
    "emissive", "occlusion", "shader", "true", "false", "None", "Some", "Ok", "Err", "self", "dt",
    "world", "state", "input", "f32", "bool", "u16", "usize", "wgpu", "src", "tests", "crates",
];

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is crates/gizmo; the root is two up.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/gizmo has a grandparent")
        .to_path_buf()
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{} could not be read: {e}", path.display()))
        // Normalised for the same reason prose_counts.rs normalises: a CRLF checkout must not
        // change what a line looks like.
        .replace("\r\n", "\n")
}

/// Every `.rs` file in the workspace's own trees, concatenated.
///
/// `crates/` **and** `demo/` **and** each crate's `tests/`, because the plan names all three:
/// demos are where a capability is measured (`parallax_mapping`), and tests are where a rule is
/// enforced (`doc_language`). An earlier version scanned only `crates/*/src/` and reported eight
/// live demos as missing APIs — a scan whose subject is narrower than its claim.
///
/// Scanned, never listed, so a new crate or demo is covered the moment it exists.
fn all_engine_source(root: &Path) -> String {
    let mut out = String::new();
    let mut stack = vec![root.join("crates"), root.join("demo")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            // The file *name* is how the plan refers to a demo (`parallax_mapping`), so record it
            // before anything can move the path.
            if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                out.push_str(stem);
                out.push('\n');
            }
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push_str(&read(&p));
                out.push('\n');
            } else if p.extension().is_some_and(|x| x == "wgsl") {
                // Shader-side names count too: `VolumetricParams` is declared in both.
                out.push_str(&read(&p));
                out.push('\n');
            }
        }
    }
    out
}

#[test]
fn every_api_named_in_the_depth_plan_still_exists() {
    let root = repo_root();
    let plan = read(&root.join("docs/API_DEPTH.md"));
    let source = all_engine_source(&root);

    let mut skip: BTreeSet<&str> = NOT_IDENTIFIERS.iter().copied().collect();
    skip.extend(PROPOSED.iter().copied());
    let mut missing = Vec::new();

    for chunk in plan.split('`').skip(1).step_by(2) {
        // An identifier, not a phrase: no spaces, and it looks like Rust.
        let name = chunk.trim_end_matches("()").trim_end_matches('!');
        if name.is_empty()
            || name.contains(' ')
            || name.contains('/')
            || skip.contains(name)
            || !name
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == ':' || c == '<' || c == '>')
        {
            continue;
        }
        // A path like `Camera::exposure` is present if its last segment is; the qualifier is
        // there for the reader, and requiring the literal string would fail on the definition.
        // A path like `Camera::exposure` is present if its last segment is; a generic like
        // `Option<Res<T>>` if its outermost useful name is. Both are written for the reader,
        // and requiring the literal string would fail on the definition.
        let leaf = name
            .rsplit("::")
            .next()
            .unwrap_or(name)
            .split(['<', '>'])
            .find(|s| s.len() >= 4)
            .unwrap_or(name);
        if leaf.len() < 4 || skip.contains(leaf) {
            continue;
        }
        if !source.contains(leaf) {
            missing.push(name.to_string());
        }
    }
    missing.sort();
    missing.dedup();

    assert!(
        missing.is_empty(),
        "docs/API_DEPTH.md names {} API item(s) that no longer exist in any crate's src/: {:?}\n\
         A route recorded in the plan and absent from the tree reads as a door and is a wall.",
        missing.len(),
        missing,
    );
}

/// The contiguous run of `///` (or `//!`-free) doc lines immediately above `decl`.
///
/// Returns `None` if the declaration is not in the file at all, which the caller reports
/// separately — "moved" and "undocumented" are different failures.
fn doc_block_above<'a>(text: &'a str, decl: &str) -> Option<String> {
    let lines: Vec<&'a str> = text.lines().collect();
    // Trimmed: a method inside an `impl` block is indented, and the first version of this
    // compared against the raw line and reported `add_update_hook` as missing.
    let at = lines.iter().position(|l| l.trim_start().starts_with(decl))?;
    let mut block = Vec::new();
    for l in lines[..at].iter().rev() {
        let t = l.trim_start();
        // Attributes sit between the docs and the item; step over them.
        if t.starts_with("///") {
            block.push(t.trim_start_matches("///").trim());
        } else if t.starts_with("#[") || t.is_empty() && !block.is_empty() {
            continue;
        } else {
            break;
        }
    }
    Some(block.join(" "))
}

#[test]
fn every_hatch_names_the_convenience_it_is_a_hatch_for() {
    let root = repo_root();
    let mut failures = Vec::new();

    for (hatch, must_mention) in HATCHES {
        let file = HATCH_FILES
            .iter()
            .find(|(h, _)| h == hatch)
            .map(|(_, f)| *f)
            .unwrap_or_else(|| panic!("{hatch} has no entry in HATCH_FILES"));
        let decl = HATCH_DECLS
            .iter()
            .find(|(h, _)| h == hatch)
            .map(|(_, d)| *d)
            .unwrap_or_else(|| panic!("{hatch} has no entry in HATCH_DECLS"));
        let text = read(&root.join(file));

        match doc_block_above(&text, decl) {
            None => failures.push(format!(
                "{hatch} is not declared as `{decl}` in {file} — moved or renamed"
            )),
            Some(docs) if !docs.contains(must_mention) => failures.push(format!(
                "{hatch}'s own doc block ({file}) never mentions `{must_mention}`"
            )),
            Some(_) => {}
        }
    }

    assert!(
        failures.is_empty(),
        "the hatch rule is broken in {} place(s):\n  {}\n\
         Each hatch's own documentation has to name what it is a hatch for — a reader stuck behind \
         the convenience has to be able to find the door from where they are standing.",
        failures.len(),
        failures.join("\n  "),
    );
}

#[test]
fn every_hatch_in_the_table_is_a_real_item() {
    // The table is hand-written, so it can name something that was renamed away. This is what
    // stops it from becoming a list of good intentions.
    let root = repo_root();
    let mut missing = Vec::new();
    for (hatch, _) in HATCHES {
        let file = HATCH_FILES
            .iter()
            .find(|(h, _)| h == hatch)
            .map(|(_, f)| *f)
            .expect("checked by the test above");
        let text = read(&root.join(file));
        // `Phase::User` is declared as a bare `User(u16)` variant; check the leaf.
        let leaf = hatch.rsplit("::").next().unwrap_or(hatch);
        if !text.contains(leaf) {
            missing.push(format!("{hatch} is not in {file}"));
        }
    }
    assert!(
        missing.is_empty(),
        "HATCHES names {} item(s) that are not where the table says: {:?}",
        missing.len(),
        missing,
    );
}
