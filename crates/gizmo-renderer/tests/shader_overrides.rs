//! Every shader's disk override is where hot reload looks, and is the same file as the copy compiled in.
//!
//! A renderer shader is compiled in with `include_str!` and first tries a copy on disk at
//! `demo/assets/shaders/<file>`, which the studio watches and rebuilds the pipelines from (the
//! `shader-hot-reload` skill). Until 2026-11-23 eight of them looked at
//! `crates/gizmo-renderer/src/shaders/<file>` instead:
//! - decal ×2, fluid ×2, particles, GPU physics ×2, smoke;
//! - every demo started from the repository's root read those eight from the source tree at run
//!   time rather than from its own build;
//! - the studio never rebuilt them, as nothing under `demo/assets` had changed.
//!
//! The paths are string literals at each call, so this test reads them: every `"….wgsl"` handed to
//! a `load_shader*` call or to `std::fs::read_to_string` must be `demo/assets/shaders/` and the
//! file name of the `include_str!` beside it.

use std::path::Path;

fn sources(dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in std::fs::read_dir(dir).expect("the source directory") {
        let path = entry.expect("an entry").path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push((path.display().to_string(), std::fs::read_to_string(&path).expect("a source file")));
        }
    }
}

/// The `"…"` literals on `line` that end in `.wgsl`, each with the text before it on the line.
fn wgsl_literals(line: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut at = 0;
    while let Some(open) = line[at..].find('"').map(|o| at + o) {
        let Some(close) = line[open + 1..].find('"').map(|c| open + 1 + c) else { break };
        let literal = &line[open + 1..close];
        if literal.ends_with(".wgsl") {
            found.push((literal.to_string(), line[..open].to_string()));
        }
        at = close + 1;
    }
    found
}

fn is_include(before: &str) -> bool {
    before.trim_end().ends_with("include_str!(")
}

#[test]
fn every_shader_override_is_under_demo_assets_and_names_its_own_file() {
    let mut files = Vec::new();
    sources(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
    let mut wrong = Vec::new();
    let mut checked = 0;
    for (path, text) in &files {
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            for (literal, before) in wgsl_literals(line) {
                // A path handed to a shader loader — on the call's line or as an argument on its own
                // line under it — or read from disk directly. Not the copy compiled in, nor a name
                // naga_oil composes by (`file_path`), nor a test's source path.
                let loader_above = before.trim().is_empty() && lines[i.saturating_sub(3)..i].iter().any(|l| l.contains("load_shader"));
                let read = before.trim_end().ends_with("read_to_string(");
                if is_include(&before) || !(before.contains("load_shader") || loader_above || read) {
                    continue;
                }
                checked += 1;
                let name = literal.rsplit('/').next().unwrap_or_default();
                if literal != format!("demo/assets/shaders/{name}") {
                    wrong.push(format!("{path}:{}: `{literal}` is not under demo/assets/shaders/", i + 1));
                    continue;
                }
                // The compiled-in copy is the next `include_str!` of a `.wgsl`, at the same call.
                let beside = lines[i..lines.len().min(i + 4)]
                    .iter()
                    .flat_map(|l| wgsl_literals(l))
                    .find(|(_, before)| is_include(before))
                    .map(|(l, _)| l);
                match beside {
                    Some(copy) if copy.rsplit('/').next() == Some(name) => {}
                    Some(copy) => wrong.push(format!("{path}:{}: `{literal}` overrides `{copy}`", i + 1)),
                    None => {} // read on its own (`std::fs::read_to_string`), its copy further down
                }
            }
        }
    }
    assert!(checked >= 30, "the scan found only {checked} overrides; it has stopped reading them");
    assert!(wrong.is_empty(), "shader overrides out of place:\n{}", wrong.join("\n"));
}
