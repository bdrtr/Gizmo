//! The staging split is a promise about the dependency graph, so the graph is what gets checked.
//!
//! §4 divides the workspace in two: Stage A crates may go 1.x, Stage B stays 0.y until wgpu,
//! winit and egui settle. That is not a labelling exercise — the whole content of "may go 1.x" is
//! that the crate's public surface can hold still, and a Stage A crate that depends on a Stage B
//! one inherits every breaking change the fast-moving layer makes. One such edge would quietly
//! disqualify a crate from the release it is listed for, and the list is prose in a document.
//!
//! Both facts this file needs — which crates exist, and what they depend on — are read from the
//! manifests rather than written down here. Only the *classification* is written down, because
//! that is the decision; and a crate that is in neither list fails, so the decision cannot be
//! skipped by adding a crate.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Crates whose surface we own, listed in ENGINE.md §4 as candidates for 1.x.
const STAGE_A: &[&str] = &[
    "gizmo-math",
    "gizmo-core",
    "gizmo-physics-core",
    "gizmo-physics-rigid",
    "gizmo-physics-dynamics",
    "gizmo-physics-soft",
    "gizmo-scene",
    "gizmo-net",
    "gizmo-audio",
    "gizmo-ai",
    "gizmo-animation",
];

/// The graphics and integration layer, pinned to 0.y while its dependencies move.
const STAGE_B: &[&str] = &[
    "gizmo-renderer",
    "gizmo-window",
    "gizmo-editor",
    "gizmo-ui",
    "gizmo-app",
    "gizmo-scripting",
    // The facade. Its package name is `gizmo-engine` while its directory and its path-dependency
    // key are both `gizmo`; the mapping below is what reconciles the two. It re-exports both
    // stages, so it moves with the faster one by construction.
    "gizmo-engine",
    // Tooling on top of the app layer, not part of the staged surface at all.
    "gizmo-analysis",
    // The editor application. `publish = false`; it is here so the coverage check below sees
    // every directory rather than needing an "ignore" list of its own.
    "gizmo-studio",
];

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/gizmo sits two levels below the workspace root")
        .to_path_buf()
}

/// `crate name -> its in-workspace dependencies`, read from the manifests.
///
/// Deliberately naive parsing: a `[dependencies]`-section line whose key starts with `gizmo`. It
/// is enough because every in-workspace dependency in this repo is written on one line with a
/// `path = "../…"`, and being naive is what keeps this test from needing a TOML crate to check a
/// property about crate dependencies.
fn graph() -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    let dir = workspace().join("crates");
    for entry in std::fs::read_dir(&dir).expect("crates/ is readable").flatten() {
        let manifest = entry.path().join("Cargo.toml");
        let Ok(text) = std::fs::read_to_string(&manifest) else { continue };

        let mut name = String::new();
        let mut deps = Vec::new();
        let mut section = String::new();
        for line in text.lines() {
            let t = line.trim();
            if t.starts_with('[') {
                section = t.to_string();
                continue;
            }
            if section == "[package]" {
                if let Some(rest) = t.strip_prefix("name") {
                    name = rest.trim_start_matches([' ', '=']).trim().trim_matches('"').to_string();
                }
            }
            // Only real dependencies: dev-dependencies are not part of the published surface, and
            // a Stage A crate testing against a Stage B one breaks no promise.
            if section == "[dependencies]" && t.starts_with("gizmo") {
                let dep = t.split(['=', ' ', '.']).next().unwrap_or("").trim();
                if !dep.is_empty() {
                    deps.push(dep.to_string());
                }
            }
        }
        if !name.is_empty() {
            deps.sort();
            deps.dedup();
            out.insert(name, deps);
        }
    }
    assert!(out.len() > 15, "only {} manifests parsed", out.len());
    out
}

/// The promise itself: nothing in Stage A may reach into Stage B.
#[test]
fn no_stage_a_crate_depends_on_a_stage_b_crate() {
    let graph = graph();
    let mut violations = Vec::new();

    for (name, deps) in &graph {
        if !STAGE_A.contains(&name.as_str()) {
            continue;
        }
        for dep in deps {
            // A path dependency on the facade is spelled `gizmo`; the package is `gizmo-engine`.
            let dep_name = if dep == "gizmo" { "gizmo-engine" } else { dep.as_str() };
            if STAGE_B.contains(&dep_name) {
                violations.push(format!(
                    "{name} (Stage A, may go 1.x) depends on {dep} (Stage B, stays 0.y) — it \
                     would inherit every breaking change that layer makes"
                ));
            }
        }
    }

    assert!(violations.is_empty(), "staging violated:\n  {}", violations.join("\n  "));
}

/// Every crate is classified. A new one has to be put in a stage, which is the moment to think
/// about it — not after it has grown a public surface.
#[test]
fn every_crate_is_in_exactly_one_stage() {
    let graph = graph();
    let mut unclassified = Vec::new();
    let mut both = Vec::new();
    for name in graph.keys() {
        let a = STAGE_A.contains(&name.as_str());
        let b = STAGE_B.contains(&name.as_str());
        if !a && !b {
            unclassified.push(name.clone());
        }
        if a && b {
            both.push(name.clone());
        }
    }
    assert!(
        unclassified.is_empty(),
        "these crates are in neither stage — add them to STAGE_A or STAGE_B in this file, and to \
         ENGINE.md §4:\n  {}",
        unclassified.join("\n  ")
    );
    assert!(both.is_empty(), "listed in both stages: {}", both.join(", "));

    // And the lists do not name crates that no longer exist.
    for name in STAGE_A.iter().chain(STAGE_B.iter()) {
        assert!(graph.contains_key(*name), "`{name}` is staged here and has no manifest");
    }
}

/// `gizmo-core` and `gizmo-math` are the floor. If either grows an in-workspace dependency the
/// layering has inverted somewhere, and every "bottom-up, no cycles" claim in the docs goes with
/// it.
#[test]
fn the_two_root_crates_depend_on_nothing_of_ours() {
    let graph = graph();
    for root in ["gizmo-math", "gizmo-core"] {
        let deps = graph.get(root).unwrap_or_else(|| panic!("{root} has no manifest"));
        assert!(deps.is_empty(), "{root} now depends on {deps:?} — it is supposed to be the floor");
    }
}
