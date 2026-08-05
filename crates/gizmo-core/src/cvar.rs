//! Console variables (*cvars*) and the state behind an in-engine developer
//! console.
//!
//! A [`CVarRegistry`] is a flat, name-keyed table of runtime-tweakable values
//! ([`CVarValue`]) plus a minimal text command parser
//! ([`CVarRegistry::execute`]) that understands `set`, `get`, `list` and
//! `clear`. The registry is inert data: it owns no entities, reads no
//! components and runs no systems, and changing a cvar notifies nobody — code
//! that wants to honour one has to poll it with [`CVarRegistry::get`] and apply
//! the value itself.
//!
//! # Name matching
//!
//! Registration and every lookup lowercase the name with
//! [`str::to_lowercase`], so names are case-insensitive — but with Unicode, not
//! Turkish, casing rules: `Ilan` matches `ilan` (not `ılan`), and `İ` lowercases
//! to the two code points `i` + U+0307, which will *not* match a cvar registered
//! as `i`. ASCII names sidestep the question entirely.
//!
//! # Messages
//!
//! Every string the parser hands back is human-readable Turkish meant to be
//! printed verbatim. It is not a machine-readable protocol and the exact wording
//! is not part of the API; the sole exception is the `CLEAR_SCREEN_REQUEST`
//! sentinel documented on [`CVarRegistry::execute`].

use std::collections::HashMap;

/// The value of a console variable, dynamically typed at runtime.
///
/// The variant *is* the cvar's declared type: the console's `set` command parses
/// text against the variant stored in [`CVar::default_value`] and rejects
/// anything that does not parse. No range or sanity checking happens anywhere in
/// this module — a `Float` may be infinite or NaN, an `Int` may be any `i32`.
///
/// [`Display`](std::fmt::Display) prints the payload bare, with no type tag and
/// no quotes around strings, using Rust's own float formatting: `Float(1500.0)`
/// prints as `1500` (not `1500.0`), `Float(f32::NAN)` as `NaN`, `Bool` as
/// `true`/`false`. This is the form the `get` and `list` commands emit.
///
/// `#[non_exhaustive]`: downstream matches need a wildcard arm.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum CVarValue {
    /// A signed 32-bit integer. Console input must be a plain decimal literal
    /// with an optional sign (`-3`, `+7`); `5.0`, `0x10` and anything with
    /// embedded spaces are rejected.
    Int(i32),
    /// A 32-bit float. Console input is whatever `f32::from_str` accepts, which
    /// includes `1e3`, `inf`, `-inf` and `NaN`. Nothing clamps the result.
    Float(f32),
    /// A flag. Console input accepts exactly `true` and `false` (lowercase only
    /// — `True` is an error) plus the aliases `1` and `0`; `yes`/`on` are not
    /// recognised.
    Bool(bool),
    /// Arbitrary UTF-8 text. When set from the console the value is the rest of
    /// the command line with each run of whitespace collapsed to a single space;
    /// there is no quote or escape handling, so typed quotes end up inside the
    /// value.
    String(String),
}

impl std::fmt::Display for CVarValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CVarValue::Int(v) => write!(f, "{}", v),
            CVarValue::Float(v) => write!(f, "{}", v),
            CVarValue::Bool(v) => write!(f, "{}", v),
            CVarValue::String(v) => write!(f, "{}", v),
        }
    }
}

/// One registered console variable: its current value plus the metadata the
/// console needs in order to display it and to re-parse text into it.
///
/// Instances live in [`CVarRegistry::cvars`], keyed by the lowercased name.
#[derive(Debug, Clone)]
pub struct CVar {
    /// The name exactly as passed to [`CVarRegistry::register`], original casing
    /// preserved. Lookups never consult this field — they go through the
    /// lowercased map key — so it serves display only, and must be kept
    /// consistent with that key if the map is edited directly.
    pub name: String,
    /// Free-form one-line help text. It is shown by the `list` command and
    /// nowhere else, and may be empty.
    pub description: String,
    /// The value in effect right now. Its variant is normally the same as
    /// `default_value`'s, but [`CVarRegistry::set`] performs no type check, so
    /// readers should match on the variant rather than assume one.
    pub value: CVarValue,
    /// The value supplied at registration. Nothing in this module ever restores
    /// it — there is no `reset` command — so its only live role is as the *type*
    /// the console parses text against; replacing it changes how future
    /// `set <name> …` input is interpreted, without changing `value`.
    pub default_value: CVarValue,
}

/// Presentation state of the developer-console overlay: whether it is visible,
/// what the user has typed so far, and the scrollback.
///
/// Plain serializable state — it holds no cvars (those live in
/// [`CVarRegistry`]), and nothing in `gizmo-core` reads or mutates it. The UI
/// that draws the console owns every field's meaning. `Default` is closed, with
/// an empty input buffer and an empty log.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct DevConsoleState {
    /// Whether the overlay is currently shown. Flipping this flag is the entire
    /// open/close mechanism; there is no animation or transition state.
    pub is_open: bool,
    /// The line currently being edited, without the prompt. It is not yet part
    /// of [`CVarRegistry::command_history`] — a line only enters history when it
    /// is handed to [`CVarRegistry::execute`] — and nothing here clears it, so
    /// whoever submits the line owns that.
    pub input_buffer: String,
    /// Scrollback, one entry per displayed line. [`CVarRegistry::execute`] can
    /// return several lines joined by `\n` in a single `String`, so a caller
    /// that wants one entry per line has to split it first. Unbounded: nothing
    /// in this module trims the log, so a long-lived console grows it forever.
    pub output_log: Vec<String>,
}

/// A flat, case-insensitive table of console variables together with the log of
/// commands executed against it.
///
/// This is the whole cvar system: registration, lookup, mutation and the text
/// command parser. It does no change detection and sends no notifications, so a
/// system that wants to follow a cvar must read it each time it needs it.
#[derive(Debug)]
pub struct CVarRegistry {
    /// All registered cvars, keyed by `name.to_lowercase()`. Inserting into this
    /// map directly requires pre-lowercasing the key yourself, otherwise
    /// [`get`](Self::get), [`set`](Self::set) and the console will never find
    /// the entry.
    ///
    /// Iteration order is `HashMap` order: unspecified, and it differs from one
    /// process to the next because of the randomised hash seed. `list` therefore
    /// prints cvars in an arbitrary order, and anything that has to be
    /// deterministic must sort the keys instead of relying on this iteration.
    pub cvars: HashMap<String, CVar>,
    /// Every string ever passed to [`execute`](Self::execute), verbatim
    /// (original casing, untrimmed) and in call order — including blank input
    /// and commands that failed. Append-only: nothing in this module reads,
    /// trims or de-duplicates it and there is no history-recall command, so it
    /// grows for the lifetime of the registry.
    pub command_history: Vec<String>,
}

impl CVarRegistry {
    /// Creates an empty registry: no cvars are pre-registered and the history is
    /// empty. Everything the console can see must be added afterwards with
    /// [`register`](Self::register).
    pub fn new() -> Self {
        Self {
            cvars: HashMap::new(),
            command_history: Vec::new(),
        }
    }

    /// Registers a cvar under `name`, or replaces one that already exists.
    ///
    /// The map key is `name.to_lowercase()` while `name` itself is kept as given
    /// for display. `value` becomes both the current value and the default, and
    /// the default's variant fixes the type that later console input is parsed
    /// against.
    ///
    /// Re-registering a name that is already taken (compared case-insensitively)
    /// overwrites the entry outright — the previous current value, description
    /// and type are all discarded. This is silent: there is no error and no way
    /// to tell that it happened.
    pub fn register(&mut self, name: &str, description: &str, value: CVarValue) {
        self.cvars.insert(
            name.to_lowercase(),
            CVar {
                name: name.to_string(),
                description: description.to_string(),
                default_value: value.clone(),
                value,
            },
        );
    }

    /// Looks up a cvar's *current* value, case-insensitively.
    ///
    /// Returns `None` when nothing is registered under that name — a never
    /// registered cvar and a misspelled one are indistinguishable, so callers
    /// normally fall back to a hard-coded default rather than treat this as an
    /// error. The variant that comes back is not guaranteed to be the one the
    /// cvar was registered with (see [`set`](Self::set)), so match on it instead
    /// of assuming a single variant.
    pub fn get(&self, name: &str) -> Option<&CVarValue> {
        self.cvars.get(&name.to_lowercase()).map(|c| &c.value)
    }

    /// Overwrites the current value of an already-registered cvar, looked up
    /// case-insensitively.
    ///
    /// **No type checking is performed.** Any variant is accepted regardless of
    /// what the cvar was registered as, so this can leave a cvar whose `value`
    /// is a `String` while its `default_value` is a `Float`; readers matching a
    /// single variant then silently stop seeing it. The console's `set` command
    /// does enforce the type — this method is the escape hatch.
    ///
    /// Never creates a cvar: on an unknown name the registry is left completely
    /// untouched and an `Err` carrying a human-readable (Turkish) message is
    /// returned. [`CVar::default_value`] is never modified either way.
    pub fn set(&mut self, name: &str, value: CVarValue) -> Result<(), String> {
        if let Some(cvar) = self.cvars.get_mut(&name.to_lowercase()) {
            cvar.value = value;
            Ok(())
        } else {
            Err(format!("CVar '{}' bulunamadi.", name))
        }
    }

    /// Parses and runs one console command line, returning the text to print.
    ///
    /// The command word is the first whitespace-separated token, matched
    /// case-insensitively:
    ///
    /// - `set <cvar> <value…>` — parses `<value…>` as the type of the cvar's
    ///   [`CVar::default_value`] and assigns it. Everything after the name is
    ///   re-joined with single spaces, so runs of whitespace collapse and typed
    ///   quotes survive as part of the text; that only matters for string cvars,
    ///   since a multi-word value cannot parse as a number.
    /// - `get <cvar>` — prints `name = value`, using the cvar's registered
    ///   casing.
    /// - `list` — prints a Turkish header line, then one indented
    ///   `key = value (description)` line per cvar, using the lowercased key
    ///   rather than the registered casing, in the unspecified order described
    ///   on [`cvars`](Self::cvars). The exact layout (header text, indent,
    ///   trailing newline) is presentation, not a format to parse.
    /// - `clear` — see the sentinel below.
    ///
    /// Failures are not errors. An unknown command, an unknown cvar, a missing
    /// argument or a value of the wrong type all come back as an ordinary
    /// Turkish message inside the returned `String`; the method never panics and
    /// has no failure channel, so the only programmatic way to confirm a `set`
    /// took effect is to read the cvar back. Empty or whitespace-only input
    /// returns an empty string.
    ///
    /// `clear` returns the literal sentinel `CLEAR_SCREEN_REQUEST` in place of
    /// text to print; the caller is expected to recognise that exact string and
    /// wipe its own scrollback (the developer console in `gizmo-app` does this).
    ///
    /// Every call appends `cmd` verbatim to
    /// [`command_history`](Self::command_history) *before* parsing, so blank
    /// lines and rejected commands are recorded too.
    // Command parser (e.g., "set physics_gravity_y -10.5")
    pub fn execute(&mut self, cmd: &str) -> String {
        self.command_history.push(cmd.to_string());
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        if parts.is_empty() {
            return String::new();
        }

        let command = parts[0].to_lowercase();
        match command.as_str() {
            "set" => {
                if parts.len() < 3 {
                    return "Kullanim: set <cvar> <value>".to_string();
                }
                let cvar_name = parts[1].to_lowercase();
                let value_str = parts[2..].join(" ");

                if let Some(cvar) = self.cvars.get_mut(&cvar_name) {
                    // Try to parse based on existing type
                    match &cvar.default_value {
                        CVarValue::Int(_) => {
                            if let Ok(v) = value_str.parse::<i32>() {
                                cvar.value = CVarValue::Int(v);
                                format!("{} = {}", cvar_name, v)
                            } else {
                                "Hata: Beklenen tip Int".to_string()
                            }
                        }
                        CVarValue::Float(_) => {
                            if let Ok(v) = value_str.parse::<f32>() {
                                cvar.value = CVarValue::Float(v);
                                format!("{} = {}", cvar_name, v)
                            } else {
                                "Hata: Beklenen tip Float".to_string()
                            }
                        }
                        CVarValue::Bool(_) => {
                            if let Ok(v) = value_str.parse::<bool>() {
                                cvar.value = CVarValue::Bool(v);
                                format!("{} = {}", cvar_name, v)
                            } else if value_str == "1" {
                                cvar.value = CVarValue::Bool(true);
                                format!("{} = true", cvar_name)
                            } else if value_str == "0" {
                                cvar.value = CVarValue::Bool(false);
                                format!("{} = false", cvar_name)
                            } else {
                                "Hata: Beklenen tip Bool".to_string()
                            }
                        }
                        CVarValue::String(_) => {
                            cvar.value = CVarValue::String(value_str.clone());
                            format!("{} = \"{}\"", cvar_name, value_str)
                        }
                    }
                } else {
                    format!("Bilinmeyen cvar: {}", cvar_name)
                }
            }
            "get" => {
                if parts.len() < 2 {
                    return "Kullanim: get <cvar>".to_string();
                }
                let cvar_name = parts[1].to_lowercase();
                if let Some(cvar) = self.cvars.get(&cvar_name) {
                    format!("{} = {}", cvar.name, cvar.value)
                } else {
                    format!("Bilinmeyen cvar: {}", cvar_name)
                }
            }
            "list" => {
                let mut out = String::from("Kayitli CVar'lar:\n");
                for (name, cvar) in &self.cvars {
                    out.push_str(&format!(
                        "  {} = {} ({})\n",
                        name, cvar.value, cvar.description
                    ));
                }
                out
            }
            "clear" => {
                String::from("CLEAR_SCREEN_REQUEST") // Special signal
            }
            _ => {
                format!(
                    "Bilinmeyen komut: {}. Mevcut komutlar: set, get, list, clear",
                    command
                )
            }
        }
    }
}

impl Default for CVarRegistry {
    fn default() -> Self {
        Self::new()
    }
}
