//! The SCRIPT section's property rows.
//!
//! # The rule
//!
//! **Every value stored on the component gets a row, because every value reaches the script.**
//! `update_entity` takes `Script::properties` whole and the studio hands it
//! `script.properties.clone()` — nothing between the scene file and Lua filters it
//! (`every_stored_property_reaches_the_script_declared_or_not`, in `gizmo-scripting`).
//!
//! This section used to draw the script's `properties = { … }` declaration instead, and only that.
//! Two kinds of stored value fell through the gap and both are ordinary:
//!
//! - **A stale override.** A script edited from `locked = false` to `locked = "no"` leaves a bool
//!   behind on every entity that had touched it. The old code filtered it out of the display and
//!   showed the *declared default* — while the script kept receiving the bool. The inspector said
//!   one thing and the running game did another, which is the worst shape a panel can have.
//! - **An undeclared key.** Never listed at all, still handed to the script, still serialized into
//!   the scene, and no way to see or remove it from the editor.
//!
//! So the rows are the union, and the odd ones are *marked* rather than hidden. Nothing here
//! deletes an override on its own: a script file saved mid-edit would otherwise throw away values
//! the user spent time on.
//!
//! # What the declaration is still for
//!
//! It is the schema and the defaults. A property with no override is not in `props` at all — the
//! script reads its own `properties` table for the fallback — so a `Declared` row is showing what
//! the script will fall back to, not something this entity stores.

use gizmo_scripting::{Script, ScriptEngine, ScriptValue};
use std::collections::BTreeMap;

/// Why a property row looks the way it does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PropertyStatus {
    /// Declared by the script; this entity stores nothing, so the row shows the declared default
    /// and the script falls back to it.
    Declared,
    /// Declared, and this entity overrides it with a value of the declared kind.
    Overridden,
    /// Declared, but this entity's stored value is of a **different** kind than the declaration.
    ///
    /// The stored value is what the script receives — it is not coerced and it is not dropped.
    /// The row shows it, and says what the script now declares instead.
    TypeMismatch {
        /// The kind the script declares the property as, which the override disagrees with.
        declared_kind: &'static str,
    },
    /// Stored on this entity but absent from the declaration: either a leftover from an older
    /// version of the script, or data added here on purpose. Either way the script gets it.
    Undeclared,
}

/// One row of the SCRIPT section.
#[derive(Clone, Debug)]
pub struct PropertyRow {
    /// The property's name, as the script declares it.
    pub name: String,
    /// What the row shows and edits. For [`PropertyStatus::Declared`] this is the declared
    /// default (nothing is stored); for every other status it is the stored value, which is
    /// exactly what the script receives.
    pub value: ScriptValue,
    /// Whether this entity overrides it, and whether that override still type-checks.
    pub status: PropertyStatus,
}

/// The union of what the script declares and what this entity stores.
///
/// Declared names first, then the undeclared leftovers — both in `BTreeMap` order, so the list is
/// stable between frames and the schema reads before the exceptions.
pub fn property_rows(
    declared: &BTreeMap<String, ScriptValue>,
    stored: &BTreeMap<String, ScriptValue>,
) -> Vec<PropertyRow> {
    let mut rows = Vec::with_capacity(declared.len() + stored.len());

    for (name, default) in declared {
        let (value, status) = match stored.get(name) {
            None => (default.clone(), PropertyStatus::Declared),
            Some(v) if v.kind() == default.kind() => (v.clone(), PropertyStatus::Overridden),
            Some(v) => (
                v.clone(),
                PropertyStatus::TypeMismatch { declared_kind: default.kind() },
            ),
        };
        rows.push(PropertyRow { name: name.clone(), value, status });
    }

    for (name, v) in stored {
        if !declared.contains_key(name) {
            rows.push(PropertyRow {
                name: name.clone(),
                value: v.clone(),
                status: PropertyStatus::Undeclared,
            });
        }
    }

    rows
}

/// The kind a newly added property starts as. `ScriptValue` carries a value, and the picker needs
/// something to hold before there is one.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum NewPropertyKind {
    #[default]
    /// A number.
    Num,
    /// A flag.
    Bool,
    /// A string.
    Text,
}

impl NewPropertyKind {
    /// The value a property of this kind is created with.
    pub fn blank(self) -> ScriptValue {
        match self {
            Self::Num => ScriptValue::Num(0.0),
            Self::Bool => ScriptValue::Bool(false),
            Self::Text => ScriptValue::Text(String::new()),
        }
    }
}

/// Why an add was refused, or `None` when it is fine to add.
///
/// Split out from the button so the reason can be *shown*: a disabled control with no explanation
/// is a control that looks broken.
pub fn add_refusal<'a>(
    name: &str,
    declared: &BTreeMap<String, ScriptValue>,
    stored: &BTreeMap<String, ScriptValue>,
) -> Option<&'a str> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Some("Bir isim yaz.");
    }
    // Lua reads these as `props.<name>`, so anything that is not an identifier is only reachable
    // as `props["..."]` — legal, but almost always a typo rather than an intention.
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
        || trimmed.starts_with(|c: char| c.is_ascii_digit())
    {
        return Some("Yalnız harf, rakam ve _ (rakamla başlayamaz).");
    }
    if stored.contains_key(trimmed) {
        return Some("Bu isim bu varlıkta zaten var.");
    }
    if declared.contains_key(trimmed) {
        return Some("Script bunu zaten bildiriyor — satırı yukarıda.");
    }
    None
}

/// The properties a script declares, with this entity's values — and everything else this entity
/// stores, because the script gets that too.
pub fn draw_script_properties(
    ui: &mut egui::Ui,
    world: &gizmo_core::World,
    script: &mut Script,
    state: &mut crate::editor_state::EditorState,
) {
    use crate::theme::palette::*;

    let declared = match world.get_resource::<ScriptEngine>() {
        Some(engine) => engine.declared_properties(&script.file_path),
        // No engine in this configuration. The stored values still reach a script that does run,
        // so they are still listed — with nothing to compare against, all of them read as
        // undeclared, which is the truth here rather than a bug.
        None => BTreeMap::new(),
    };

    let rows = property_rows(&declared, &script.properties);

    ui.separator();
    if rows.is_empty() {
        ui.label(
            egui::RichText::new("Bu script özellik bildirmiyor (properties = { … }).")
                .size(10.0)
                .color(TEXT_DIM),
        );
    }

    // Collected and applied after the loop: the rows borrow `script.properties`, and a row's
    // buttons decide to change it.
    let mut set: Option<(String, ScriptValue)> = None;
    let mut clear: Option<String> = None;

    for row in &rows {
        ui.horizontal(|ui| {
            let (mark, tip, colour) = match row.status {
                PropertyStatus::Declared => ("", "", TEXT_DIM),
                PropertyStatus::Overridden => {
                    ("●", "Bu varlıkta script'in varsayılanından farklı", ACCENT)
                }
                PropertyStatus::TypeMismatch { declared_kind } => (
                    "⚠",
                    match declared_kind {
                        "number" => "Script bunu artık number bildiriyor — script'e giden değer bu satırdaki",
                        "bool" => "Script bunu artık bool bildiriyor — script'e giden değer bu satırdaki",
                        _ => "Script bunu artık text bildiriyor — script'e giden değer bu satırdaki",
                    },
                    ACCENT_LIGHT,
                ),
                PropertyStatus::Undeclared => (
                    "+",
                    "Script bunu bildirmiyor, ama script'e gidiyor",
                    TEXT_MUTED,
                ),
            };
            if !mark.is_empty() {
                ui.label(egui::RichText::new(mark).size(10.0).color(colour))
                    .on_hover_text(tip);
            }
            ui.label(egui::RichText::new(&row.name).size(11.0));

            match row.value.clone() {
                ScriptValue::Num(mut n) => {
                    if ui.add(egui::DragValue::new(&mut n).speed(0.1)).changed() {
                        set = Some((row.name.clone(), ScriptValue::Num(n)));
                    }
                }
                ScriptValue::Bool(mut b) => {
                    if crate::theme::segmented(ui, &mut b, &[(true, "true"), (false, "false")]) {
                        set = Some((row.name.clone(), ScriptValue::Bool(b)));
                    }
                }
                ScriptValue::Text(mut t) => {
                    if ui.text_edit_singleline(&mut t).changed() {
                        set = Some((row.name.clone(), ScriptValue::Text(t)));
                    }
                }
            }

            match row.status {
                // Back to the script's default: drop the stored value. For a mismatch this is
                // also the fix, since the declared default is by definition the declared kind.
                PropertyStatus::Overridden | PropertyStatus::TypeMismatch { .. } => {
                    if ui
                        .small_button(egui::RichText::new("↺").color(ACCENT_LIGHT))
                        .on_hover_text("Script'teki varsayılana dön")
                        .clicked()
                    {
                        clear = Some(row.name.clone());
                    }
                }
                // Nothing declares this, so there is no default to fall back to — removing it
                // removes it.
                PropertyStatus::Undeclared => {
                    if ui
                        .small_button(egui::RichText::new("🗑").color(TEXT_MUTED))
                        .on_hover_text("Bu varlıktan sil")
                        .clicked()
                    {
                        clear = Some(row.name.clone());
                    }
                }
                PropertyStatus::Declared => {}
            }
        });
    }

    if let Some((name, value)) = set {
        script.properties.insert(name, value);
    }
    if let Some(name) = clear {
        script.properties.remove(&name);
    }

    // ── Add a property the script does not declare ───────────────────────────────────────────
    //
    // Worth having precisely because the runtime passes the whole map: a script can read
    // `props.foo` without declaring `foo`, and a scene can carry per-entity data for it. It is
    // also the only way to put back a value whose declaration was removed.
    ui.separator();
    let refusal = add_refusal(&state.script.new_property_name, &declared, &script.properties);
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut state.script.new_property_name)
                .desired_width(96.0)
                .hint_text("yeni özellik"),
        );
        crate::theme::segmented(
            ui,
            &mut state.script.new_property_kind,
            &[
                (NewPropertyKind::Num, "num"),
                (NewPropertyKind::Bool, "bool"),
                (NewPropertyKind::Text, "text"),
            ],
        );
        if ui
            .add_enabled(refusal.is_none(), egui::Button::new("Ekle"))
            .clicked()
        {
            let name = state.script.new_property_name.trim().to_string();
            script
                .properties
                .insert(name, state.script.new_property_kind.blank());
            state.script.new_property_name.clear();
        }
    });
    if let Some(why) = refusal {
        // Only once the user has started typing: an empty field is not a mistake yet.
        if !state.script.new_property_name.trim().is_empty() {
            ui.label(egui::RichText::new(why).size(10.0).color(TEXT_DIM));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, ScriptValue)]) -> BTreeMap<String, ScriptValue> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    fn row<'a>(rows: &'a [PropertyRow], name: &str) -> &'a PropertyRow {
        rows.iter().find(|r| r.name == name).expect("a row for that name")
    }

    /// A property with no override shows the declared default — that is what the script falls
    /// back to, because a name absent from `props` is nil on the Lua side.
    #[test]
    fn a_property_with_no_override_shows_the_declared_default() {
        let rows = property_rows(&map(&[("speed", ScriptValue::Num(2.4))]), &BTreeMap::new());
        assert_eq!(rows.len(), 1);
        assert_eq!(row(&rows, "speed").status, PropertyStatus::Declared);
        assert_eq!(row(&rows, "speed").value, ScriptValue::Num(2.4));
    }

    #[test]
    fn an_override_of_the_declared_kind_is_marked_and_shows_its_own_value() {
        let rows = property_rows(
            &map(&[("speed", ScriptValue::Num(2.4))]),
            &map(&[("speed", ScriptValue::Num(9.0))]),
        );
        assert_eq!(row(&rows, "speed").status, PropertyStatus::Overridden);
        assert_eq!(row(&rows, "speed").value, ScriptValue::Num(9.0));
    }

    /// The defect this module exists for. A stale override was filtered out of the display and
    /// the declared default was shown in its place — while the script went on receiving the stale
    /// value. The row must show what the script gets, and say what changed under it.
    #[test]
    fn a_stale_override_is_shown_not_hidden() {
        let rows = property_rows(
            &map(&[("locked", ScriptValue::Bool(false))]),
            &map(&[("locked", ScriptValue::Text("yes".to_string()))]),
        );
        assert_eq!(
            row(&rows, "locked").status,
            PropertyStatus::TypeMismatch { declared_kind: "bool" }
        );
        assert_eq!(
            row(&rows, "locked").value,
            ScriptValue::Text("yes".to_string()),
            "the row must show the value the SCRIPT receives, not the declared default"
        );
    }

    /// An undeclared key is invisible in the declaration and very much visible to the script.
    #[test]
    fn an_undeclared_value_gets_a_row_of_its_own() {
        let rows = property_rows(
            &map(&[("speed", ScriptValue::Num(1.0))]),
            &map(&[("leftover", ScriptValue::Num(42.0))]),
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(row(&rows, "leftover").status, PropertyStatus::Undeclared);
        assert_eq!(row(&rows, "leftover").value, ScriptValue::Num(42.0));
    }

    /// Declared names first, then the leftovers — so the schema reads before the exceptions, and
    /// the list does not reshuffle between frames.
    #[test]
    fn declared_rows_come_first_and_the_order_is_stable() {
        let rows = property_rows(
            &map(&[("b_declared", ScriptValue::Num(1.0)), ("a_declared", ScriptValue::Num(1.0))]),
            &map(&[("z_extra", ScriptValue::Num(1.0)), ("a_extra", ScriptValue::Num(1.0))]),
        );
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["a_declared", "b_declared", "a_extra", "z_extra"]);
    }

    /// With no engine loaded there is no declaration to compare against. Every stored value is
    /// then undeclared — which is the truth in that configuration, and still listed, because the
    /// values still reach whatever runs the script.
    #[test]
    fn with_no_declaration_everything_stored_still_gets_a_row() {
        let rows = property_rows(
            &BTreeMap::new(),
            &map(&[("a", ScriptValue::Num(1.0)), ("b", ScriptValue::Bool(true))]),
        );
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.status == PropertyStatus::Undeclared));
    }

    #[test]
    fn adding_refuses_the_names_that_would_do_nothing_useful() {
        let declared = map(&[("speed", ScriptValue::Num(1.0))]);
        let stored = map(&[("extra", ScriptValue::Num(1.0))]);

        assert!(add_refusal("", &declared, &stored).is_some(), "empty");
        assert!(add_refusal("   ", &declared, &stored).is_some(), "blank");
        assert!(add_refusal("speed", &declared, &stored).is_some(), "already declared");
        assert!(add_refusal("extra", &declared, &stored).is_some(), "already stored");
        // Lua reads these as `props.<name>`; anything else is only reachable as `props["..."]`.
        assert!(add_refusal("has space", &declared, &stored).is_some());
        assert!(add_refusal("has-dash", &declared, &stored).is_some());
        assert!(add_refusal("2fast", &declared, &stored).is_some(), "leading digit");

        assert!(add_refusal("open_speed2", &declared, &stored).is_none());
        assert!(add_refusal("  padded  ", &declared, &stored).is_none(), "trimmed before judging");
    }

    #[test]
    fn a_new_property_starts_blank_in_the_chosen_kind() {
        assert_eq!(NewPropertyKind::Num.blank(), ScriptValue::Num(0.0));
        assert_eq!(NewPropertyKind::Bool.blank(), ScriptValue::Bool(false));
        assert_eq!(NewPropertyKind::Text.blank(), ScriptValue::Text(String::new()));
    }
}
