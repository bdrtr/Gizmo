//! Public functions nothing in the workspace mentions — a sweep, run by hand.
//!
//! # Why this is `#[ignore]`d rather than a gate
//!
//! Most of what it finds is **fine**: a builder like `Material::with_emissive` or
//! `App::add_update_system` exists for a *user* to call, and the engine calling its own builders
//! would be strange. Gating on the count would mean an exception list longer than the finding,
//! which is the shape of guard that gets muted rather than read.
//!
//! What it is for is the other kind. `RigidBody::force_accumulator` was frame-rate dependent by a
//! factor of four — the same push produced a quarter of the acceleration at 60 fps and an eighth
//! at 30 — and it survived because **nothing in the workspace wrote to it**: no caller, no test,
//! no demo, no golden image. `PhysicsWorld::raycast_all` ignored its own `max_distance` for the
//! same reason. Both were found by asking this question in August 2026, and both had been wrong
//! for as long as they had existed.
//!
//! So: a measurement to run when a crate grows a public surface, and to *read*, not a number to
//! keep at zero.
//!
//! ```text
//! cargo test -p gizmo-engine --test unmentioned_api -- --ignored --nocapture
//! ```
//!
//! # Three ways this lied before it was trustworthy
//!
//! Recorded because each cost time and each is a way the next such scanner will lie:
//!
//! 1. **A call may carry a turbofish.** `world.query_mut::<(A, B)>()` does not match
//!    `query_mut\s*\(`, so the first run reported `query_mut`, `get_resource` and
//!    `remove_component` as uncalled — i.e. it reported the whole ECS as dead code. A detector
//!    that says that is one nobody reads twice.
//! 2. **A function may be *passed* rather than called.** `schedule.add_di_system(
//!    ui_layout_system.into_config())` mentions the name with no parenthesis after it, and every
//!    ECS system looks like that — which is precisely the shape the `animation_update_system`
//!    bug had, the one this whole question exists to catch. The check is therefore "is the name
//!    mentioned at all", not "is it called".
//! 3. **A doc comment mentioning the name is not a use.** Comments are cut first, for the same
//!    reason four other guards in this repository had to learn it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Everything below `crates/` with a `src/`, plus the demo crate — scanned, not listed, so a crate
/// added tomorrow is swept the day it exists.
fn crate_sources(root: &Path) -> BTreeMap<String, Vec<PathBuf>> {
    let mut out = BTreeMap::new();
    let entries = std::fs::read_dir(root.join("crates")).expect("crates/ is readable");
    for entry in entries.flatten() {
        let src = entry.path().join("src");
        if !src.is_dir() {
            continue;
        }
        let mut files = Vec::new();
        collect_rs(&src, &mut files);
        files.sort();
        out.insert(entry.file_name().to_string_lossy().into_owned(), files);
    }
    out
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Source with `//` comments removed — see reason 3 in the module docs.
fn code_only(src: &str) -> String {
    src.lines()
        .map(|line| match line.find("//") {
            Some(i) => &line[..i],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every `pub fn` name declared in `files`, outside their test modules.
fn public_fns(files: &[PathBuf]) -> BTreeMap<String, (PathBuf, usize)> {
    let mut out = BTreeMap::new();
    for file in files {
        let raw = std::fs::read_to_string(file).unwrap_or_default();
        let code = code_only(raw.split("#[cfg(test)]").next().unwrap_or(""));
        for (i, line) in code.lines().enumerate() {
            let t = line.trim_start();
            let Some(rest) = t.strip_prefix("pub fn ").or_else(|| t.strip_prefix("pub async fn "))
            else {
                continue;
            };
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_')
                .collect();
            if !name.is_empty() {
                out.entry(name).or_insert_with(|| (file.clone(), i + 1));
            }
        }
    }
    out
}

#[test]
#[ignore = "measurement, not a gate — run with --ignored --nocapture"]
fn public_functions_nothing_mentions() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/gizmo sits two levels below the workspace root")
        .to_path_buf();

    // The haystack: every Rust file in the workspace, comments cut, split at its test module so a
    // name mentioned only by a test is reported separately.
    let mut haystack: Vec<(String, String, usize)> = Vec::new();
    for base in ["crates", "demo", "demo-web"] {
        let mut files = Vec::new();
        collect_rs(&root.join(base), &mut files);
        for file in files {
            let raw = std::fs::read_to_string(&file).unwrap_or_default();
            let test_at = raw.find("#[cfg(test)]").unwrap_or(raw.len());
            let is_test_file = file.components().any(|c| c.as_os_str() == "tests");
            haystack.push((
                file.to_string_lossy().into_owned(),
                code_only(&raw),
                if is_test_file { 0 } else { test_at },
            ));
        }
    }
    assert!(haystack.len() > 200, "only {} sources found", haystack.len());

    let sources = crate_sources(&root);
    assert!(sources.len() >= 15, "only {} crates found", sources.len());

    let mut total = 0usize;
    let mut findings: Vec<String> = Vec::new();
    for (crate_name, files) in &sources {
        for (name, (decl_file, decl_line)) in public_fns(files) {
            total += 1;
            let (prod, test) = mentions(&name, &haystack);
            if prod == 0 {
                findings.push(format!(
                    "{:<44} prod {prod:>3}  test {test:>3}   {}:{}",
                    name,
                    decl_file
                        .strip_prefix(&root)
                        .unwrap_or(&decl_file)
                        .display(),
                    decl_line
                ));
            }
            let _ = crate_name;
        }
    }

    println!("── public functions nothing in production mentions ──");
    for line in &findings {
        println!("{line}");
    }
    println!(
        "\n{} of {total} public fns are unmentioned in production; \
         the ones with `test 0` as well are the interesting half.",
        findings.len()
    );
}

/// How many times `name` appears outside a declaration, split by production vs test.
fn mentions(name: &str, haystack: &[(String, String, usize)]) -> (usize, usize) {
    let (mut prod, mut test) = (0, 0);
    for (path, code, test_at) in haystack {
        let mut from = 0;
        while let Some(i) = code[from..].find(name) {
            let at = from + i;
            from = at + name.len();
            // Whole word only.
            let before_ok = at == 0
                || !code.as_bytes()[at - 1].is_ascii_alphanumeric() && code.as_bytes()[at - 1] != b'_';
            let after = code.as_bytes().get(from);
            let after_ok = after.is_none_or(|c| !c.is_ascii_alphanumeric() && *c != b'_');
            if !before_ok || !after_ok {
                continue;
            }
            // Not the declaration itself.
            let line_start = code[..at].rfind('\n').map_or(0, |n| n + 1);
            let line_end = code[at..].find('\n').map_or(code.len(), |n| at + n);
            let line = &code[line_start..line_end];
            if line.contains(&format!("fn {name}")) {
                continue;
            }
            if *test_at == 0 || at > *test_at {
                test += 1;
            } else {
                prod += 1;
            }
            let _ = path;
        }
    }
    (prod, test)
}
