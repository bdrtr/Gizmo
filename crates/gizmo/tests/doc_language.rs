//! The doc-language rule, enforced instead of remembered.
//!
//! docs/ENGINE.md's D2 item says the engine's documentation is written in English, and the reason
//! is bus factor: a `///` line only a Turkish speaker can read is a line only one person can
//! maintain, and it is also what a stranger reads on docs.rs. The rule was carried out by hand
//! in one campaign and the roadmap then claimed it was done for the Stage A crates — on
//! 2026-08-17 a scan found **462 Turkish doc lines still in Stage A's `src/`**, so the claim had
//! been false for as long as anyone had believed it.
//!
//! That is the class of failure a written status marker always has, so this file replaces the
//! marker. It counts what is actually there and compares it against [`BUDGET`]:
//!
//! - a crate absent from the table must be at **zero**, so a cleaned crate cannot quietly refill;
//! - a crate in the table must match its number **exactly** — cleaning lines without lowering the
//!   budget fails too, which is what stops the table from drifting into fiction;
//! - the subject list is *scanned* (`crates/*/src/**/*.rs`), never written down, so a new crate is
//!   covered the moment it exists.
//!
//! It deliberately does not police `tests/`, `benches/` or plain `//` comments: the rule is about
//! the documentation surface, and CLAUDE.md already records that inline comments are still
//! Turkish in places.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Turkish doc lines still present under each crate's `src/`, by crate.
///
/// **Empty, as of 2026-08-17** — every crate's documentation surface is English. It stays in the
/// file because that is the shape the rule needs: work that lands mid-campaign gets a number here
/// instead of an exemption, and the number may only ever be edited downwards. A crate that
/// reaches zero leaves the table entirely.
const BUDGET: &[(&str, usize)] = &[];

/// Files whose flagged lines are English prose *about* Turkish, with the reason.
///
/// Each one must still contain a flagged line: an exception that has stopped applying fails, so
/// the list cannot outlive what it excuses.
/// Currently empty, and that is a result rather than an oversight: the one file that needed an
/// entry (`gizmo-core/src/cvar.rs`, which documents how `to_lowercase` handles `İ`) stopped being
/// flagged once [`is_turkish`] learned to ignore citations. The mechanism stays for the case that
/// citation-stripping cannot answer.
const EXCEPTIONS: &[(&str, &str)] = &[];

/// Letters that exist in Turkish and not in English. One is enough to settle a line.
const TURKISH_LETTERS: [char; 6] = ['ğ', 'Ğ', 'ş', 'Ş', 'ı', 'İ'];

/// Common Turkish function words, for lines that happen to carry no distinctive letter.
///
/// Two of them together is the threshold. Measured over the whole workspace this rule adds 24
/// lines and none of them is a false positive; none of these words is an English word, and the
/// ones that could collide with an identifier (`var`, `ise`) need a second word to fire.
const TURKISH_WORDS: [&str; 24] = [
    "ve", "bir", "bu", "için", "ile", "değil", "var", "yok", "olarak", "ama", "çünkü", "göre",
    "sonra", "önce", "daha", "yani", "her", "gibi", "kadar", "ise", "olan", "tüm", "ayrı", "zaten",
];

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/gizmo sits two levels below the workspace root")
        .to_path_buf()
}

/// Is this doc line's text Turkish?
///
/// Code spans and quoted strings are removed first, because a citation is not prose: an English
/// sentence explaining that the engine logs `"Obje çoğaltıldı."`, or how `to_lowercase` treats
/// `İ`, is written in English *about* Turkish, and counting it would push the rule towards
/// rewriting quotes that have to keep matching the code.
fn is_turkish(text: &str) -> bool {
    let text = &strip_citations(text);
    if text.chars().any(|c| TURKISH_LETTERS.contains(&c)) {
        return true;
    }
    let lowered = text.to_lowercase();
    let words: Vec<&str> = lowered
        .split(|c: char| !c.is_alphabetic())
        .filter(|w| !w.is_empty())
        .collect();
    words.iter().filter(|w| TURKISH_WORDS.contains(w)).count() >= 2
}

/// Removes `` `code spans` `` and `"quoted strings"` — the parts of a doc line that quote
/// something rather than say it.
fn strip_citations(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut delimiter: Option<char> = None;
    for c in text.chars() {
        match delimiter {
            Some(open) => {
                if c == open {
                    delimiter = None;
                }
            }
            None if c == '`' || c == '"' => delimiter = Some(c),
            None => out.push(c),
        }
    }
    out
}

/// Every `.rs` file under a directory.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Turkish doc lines in one file: `(line number, text)`.
fn turkish_doc_lines(path: &Path) -> Vec<(usize, String)> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let trimmed = line.trim_start();
            let body = trimmed
                .strip_prefix("///")
                .or_else(|| trimmed.strip_prefix("//!"))?;
            is_turkish(body).then(|| (i + 1, body.trim().to_string()))
        })
        .collect()
}

#[test]
fn documentation_is_in_english_outside_the_recorded_budget() {
    let root = workspace();
    let excepted: Vec<PathBuf> = EXCEPTIONS
        .iter()
        .map(|(rel, _)| root.join("crates").join(rel))
        .collect();

    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut samples: BTreeMap<String, (PathBuf, usize, String)> = BTreeMap::new();
    for entry in std::fs::read_dir(root.join("crates"))
        .expect("crates/ is readable")
        .flatten()
    {
        let src = entry.path().join("src");
        if !src.is_dir() {
            continue;
        }
        let crate_name = entry.file_name().to_string_lossy().into_owned();
        let mut files = Vec::new();
        rust_files(&src, &mut files);
        files.sort();
        for file in files {
            if excepted.contains(&file) {
                continue;
            }
            for (line, text) in turkish_doc_lines(&file) {
                *counts.entry(crate_name.clone()).or_default() += 1;
                samples
                    .entry(crate_name.clone())
                    .or_insert_with(|| (file.clone(), line, text));
            }
        }
    }

    let budget: BTreeMap<&str, usize> = BUDGET.iter().copied().collect();
    let mut failures = Vec::new();

    for (crate_name, count) in &counts {
        match budget.get(crate_name.as_str()) {
            None => {
                let (file, line, text) = &samples[crate_name];
                failures.push(format!(
                    "{crate_name}: {count} Turkish doc line(s), budget 0. First: {}:{line}\n    {text}",
                    file.strip_prefix(&root).unwrap_or(file).display()
                ));
            }
            Some(&allowed) if *count > allowed => {
                let (file, line, text) = &samples[crate_name];
                failures.push(format!(
                    "{crate_name}: {count} Turkish doc line(s), budget {allowed} — the budget only \
                     goes down. First: {}:{line}\n    {text}",
                    file.strip_prefix(&root).unwrap_or(file).display()
                ));
            }
            Some(&allowed) if *count < allowed => failures.push(format!(
                "{crate_name}: {count} Turkish doc line(s) but the budget still says {allowed} — \
                 lower it in BUDGET, or the table becomes fiction"
            )),
            Some(_) => {}
        }
    }
    for (name, _) in BUDGET {
        if !counts.contains_key(*name) {
            failures.push(format!(
                "{name}: no Turkish doc lines left — remove it from BUDGET entirely"
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "the doc-language rule (docs/ENGINE.md D2) is not holding:\n  {}",
        failures.join("\n  ")
    );
}

/// An exception that no longer excuses anything is a note about the past pretending to be a rule.
#[test]
fn every_documented_exception_still_applies() {
    let root = workspace();
    for (rel, reason) in EXCEPTIONS {
        let path = root.join("crates").join(rel);
        assert!(path.is_file(), "excepted file {rel} does not exist");
        assert!(
            !turkish_doc_lines(&path).is_empty(),
            "{rel} is excepted ({reason}) but no longer has a line that would be flagged — drop \
             the exception"
        );
    }
}

/// The detector itself, because a scan that flags nothing passes every budget.
#[test]
fn the_detector_tells_the_two_languages_apart() {
    assert!(is_turkish("Bir noktadaki su örneği: yüzey yüksekliği ve derinlik."));
    assert!(is_turkish("Çözücü geçişi başına birikmiş λ."), "distinctive letters");
    assert!(
        is_turkish("Regression: RigidBody var ama Velocity yoksa sessizce"),
        "no distinctive letters, but two function words — a real line from this workspace"
    );
    assert!(!is_turkish(
        "The solver's accumulated λ per pass, and what it is for."
    ));
    assert!(
        !is_turkish("Every value is derived from the camera, not from a constant."),
        "English must not trip the word rule"
    );
    assert!(
        !is_turkish("The console prints `\"Obje çoğaltıldı.\"` when a duplicate lands."),
        "an English sentence quoting a Turkish string is English"
    );
    assert!(
        !is_turkish("`to_lowercase` maps `İ` to two code points."),
        "and so is one quoting a Turkish letter"
    );
    assert!(
        is_turkish("var files, ise blocks"),
        "a line carrying two Turkish function words fires even when they could be identifiers — \
         the false positive lands on the side of asking a human, and EXCEPTIONS is where the \
         answer goes"
    );
}
