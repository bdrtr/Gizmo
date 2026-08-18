//! Counts written into prose, checked against the code they count.
//!
//! §8's rule is "a count written into prose is a count the code will walk away from", and the fix
//! it prescribes is not diligence: **where the number carries a decision, compute it.** This is
//! that, for the one family of counts that has drifted most.
//!
//! `ScriptCommand`'s vocabulary is described in two places — the `apply_host_commands` doc in
//! `crates/gizmo/src/systems/play.rs` and the Scripting section of `docs/ENGINE.md` — and both say
//! how many variants there are, how many the scripting crate applies itself and how many are
//! handed back. Those numbers are the shape of the argument around them ("the remaining twelve are
//! scene load/save, dialogue, cutscenes and the race subsystem"), so deleting them would cost
//! something real. On 2026-08-19 alone they went stale twice in one session, both times from work
//! in that same session: one commit added a variant and another deleted one.
//!
//! So the number stays and this test holds it to the enum. It reads the sources rather than
//! carrying its own copy — a test that restated the count would be the third place to update.

use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

/// Every variant name in `ScriptCommand`, read from the enum body.
///
/// Struct-variant bodies are skipped by brace depth, so a field called `id` is never mistaken for
/// a variant, and doc comments are skipped because a variant name has to start the line.
fn script_command_variants(root: &Path) -> Vec<String> {
    let src = std::fs::read_to_string(root.join("crates/gizmo-scripting/src/commands.rs"))
        .expect("commands.rs");
    let start = src.find("pub enum ScriptCommand {").expect("the enum");
    let body_start = src[start..].find('{').expect("its body") + start;

    // Flatten the body to depth 1: a struct variant's fields are dropped, and the `{` that opens
    // them becomes a line break so the variant's own name still terminates. Without that the ten
    // struct variants vanish — measured, the first version of this counted 32 of 42.
    let mut depth = 0usize;
    let mut flat = String::new();
    for ch in src[body_start..].chars() {
        match ch {
            '{' => {
                depth += 1;
                if depth == 2 {
                    flat.push('\n');
                }
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ if depth == 1 => flat.push(ch),
            _ => {}
        }
    }

    let mut variants: Vec<String> = flat
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with("//"))
        .map(|l| {
            l.chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect::<String>()
        })
        .filter(|n| n.starts_with(|c: char| c.is_ascii_uppercase()))
        .collect();
    variants.sort();
    variants.dedup();
    variants
}

/// The body of one `fn`, from its signature to the matching closing brace.
fn fn_body<'a>(src: &'a str, signature: &str) -> &'a str {
    let start = src.find(signature).unwrap_or_else(|| panic!("{signature} not found"));
    let open = src[start..].find('{').expect("a body") + start;
    let mut depth = 0usize;
    for (i, ch) in src[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &src[open..open + i];
                }
            }
            _ => {}
        }
    }
    &src[open..]
}

/// The number that follows `needle` in `text`, as a decimal integer.
fn number_after(text: &str, needle: &str) -> Option<usize> {
    let rest = &text[text.find(needle)? + needle.len()..];
    let digits: String = rest
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

/// **The variant count in both prose sites must be the enum's.**
///
/// It has been wrong twice in a day: `SetFighterHealth` landed and made 42 read 43, then
/// `SetFightCamera` was deleted and made 43 read 42. Neither was caught by anything — the numbers
/// sit in a doc comment and a Turkish paragraph, and nothing reads them but a person.
#[test]
fn the_script_command_counts_in_prose_match_the_enum() {
    let root = workspace_root();
    let variants = script_command_variants(&root);
    let measured = variants.len();
    assert!(
        measured > 30,
        "the parser found only {measured} variants — it has stopped matching the enum's shape, \
         which would make this test pass by seeing nothing: {variants:?}"
    );

    let play = std::fs::read_to_string(root.join("crates/gizmo/src/systems/play.rs")).expect("play.rs");
    let in_play = number_after(&play, "Of `ScriptCommand`'s")
        .expect("play.rs's `apply_host_commands` doc must still state the variant count");
    assert_eq!(
        in_play, measured,
        "crates/gizmo/src/systems/play.rs says {in_play} ScriptCommand variants; the enum has \
         {measured}"
    );

    let engine_md =
        std::fs::read_to_string(root.join("docs/ENGINE.md")).expect("docs/ENGINE.md");
    let in_doc = number_after(&engine_md, "`ScriptCommand`'ın\n**")
        .expect("ENGINE.md's Scripting section must still state the variant count");
    assert_eq!(
        in_doc, measured,
        "docs/ENGINE.md says {in_doc} ScriptCommand variants; the enum has {measured}"
    );
}

/// The arithmetic the same two paragraphs assert: everything the scripting crate does not apply
/// itself is handed back, and the host's chain takes seven of those.
///
/// Checked against the code, not against itself: the applied count is the number of variants
/// matched inside `flush_commands`, and the returned count is the rest.
#[test]
fn the_applied_and_returned_split_matches_flush_commands() {
    let root = workspace_root();
    let total = script_command_variants(&root).len();

    let engine_rs = std::fs::read_to_string(root.join("crates/gizmo-scripting/src/engine.rs"))
        .expect("engine.rs");
    let body = fn_body(&engine_rs, "pub fn flush_commands");
    let applied = script_command_variants(&root)
        .iter()
        .filter(|v| body.contains(&format!("ScriptCommand::{v}")))
        .count();

    let play =
        std::fs::read_to_string(root.join("crates/gizmo/src/systems/play.rs")).expect("play.rs");
    let claimed_applied = number_after(&play, "variants, ").expect("the applied count");
    let claimed_returned = number_after(&play, "the scripting crate and ").expect("the returned count");

    assert_eq!(
        claimed_applied, applied,
        "play.rs says {claimed_applied} variants are applied inside the scripting crate; \
         `flush_commands` matches {applied}"
    );
    assert_eq!(
        claimed_returned,
        total - applied,
        "play.rs says {claimed_returned} come back to the host; {total} - {applied} = {}",
        total - applied
    );
}
