//! [`Locale`](crate::locale::Locale) — a game's strings in more than one language, and
//! [`LocalizedText`](crate::locale::LocalizedText), a label that follows the language.
//!
//! Every demo here writes its menus in Turkish, in the source. A game that ships in a second
//! language had nowhere to put the strings: no table, no lookup, no way to switch at run time.
//!
//! ```
//! use gizmo_app::locale::Locale;
//!
//! let mut locale = Locale::new("de");
//! locale.load_json("de", r#"{ "menu": { "start": "Starten", "hello": "Hallo, {name}!" } }"#).unwrap();
//! locale.load_json("en", r#"{ "menu": { "start": "Start", "hello": "Hello, {name}!" } }"#).unwrap();
//! assert_eq!(locale.get("menu.start"), "Starten");
//! locale.set_language("en");
//! assert_eq!(locale.format("menu.hello", &[("name", "Ada")]), "Hello, Ada!");
//! assert_eq!(locale.get("menu.quit"), "menu.quit", "a missing string shows its key, not nothing");
//! ```
//!
//! Insert it as a resource and put a [`LocalizedText`](crate::locale::LocalizedText) beside a `Text`: under `gizmo-engine` the
//! label shows the current language's string, and changes the frame the language does.

use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;

/// Why a string table did not load.
#[derive(Debug)]
#[non_exhaustive]
pub enum LocaleError {
    /// Not JSON.
    Json(serde_json::Error),
    /// JSON, but not an object of strings (and objects of strings): the offending key.
    NotAString(String),
}

impl std::fmt::Display for LocaleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(e) => write!(f, "string table is not JSON: {e}"),
            Self::NotAString(k) => write!(f, "string table entry `{k}` is neither a string nor an object"),
        }
    }
}

impl std::error::Error for LocaleError {}

/// String tables per language, the language in use and the one to fall back to — a resource.
#[derive(Debug, Default)]
pub struct Locale {
    language: String,
    fallback: Option<String>,
    tables: HashMap<String, HashMap<String, String>>,
    /// Keys asked for and found in no table, for [`missing`](Self::missing). Behind a lock because
    /// lookups take `&self` — they happen at draw time, from a shared borrow.
    missing: Mutex<BTreeSet<String>>,
}

impl Locale {
    /// No strings yet, `language` in use, falling back to whichever language was loaded first.
    #[must_use]
    pub fn new(language: &str) -> Self {
        Self { language: normalise(language), ..Self::default() }
    }

    /// Adds `language`'s strings from a JSON object. Nested objects become dotted keys
    /// (`{"menu": {"start": "…"}}` is `menu.start`); a later load of the same key wins. Returns how
    /// many strings it added.
    ///
    /// # Errors
    ///
    /// [`LocaleError::Json`] for text that is not JSON, [`LocaleError::NotAString`] for a value
    /// that is neither a string nor an object. Nothing is added from a table that fails.
    pub fn load_json(&mut self, language: &str, json: &str) -> Result<usize, LocaleError> {
        let value: serde_json::Value = serde_json::from_str(json).map_err(LocaleError::Json)?;
        let mut flat = HashMap::new();
        flatten("", &value, &mut flat)?;
        let n = flat.len();
        let language = normalise(language);
        if self.fallback.is_none() {
            self.fallback = Some(language.clone());
        }
        self.tables.entry(language).or_default().extend(flat);
        Ok(n)
    }

    /// Adds one string.
    pub fn insert(&mut self, language: &str, key: &str, text: &str) {
        let language = normalise(language);
        if self.fallback.is_none() {
            self.fallback = Some(language.clone());
        }
        self.tables.entry(language).or_default().insert(key.to_string(), text.to_string());
    }

    /// Switches language; every [`LocalizedText`] follows on the next frame drawn.
    pub fn set_language(&mut self, language: &str) {
        self.language = normalise(language);
    }

    /// The language in use (`"tr"`, `"en"`, … — lowercased, region dropped).
    #[must_use]
    pub fn language(&self) -> &str {
        &self.language
    }

    /// The language a missing string falls back to.
    pub fn set_fallback(&mut self, language: &str) {
        self.fallback = Some(normalise(language));
    }

    /// The languages with a table, sorted.
    #[must_use]
    pub fn languages(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.tables.keys().map(String::as_str).collect();
        v.sort_unstable();
        v
    }

    /// `key` in the current language, else in the fallback, else the key itself — a missing
    /// string shows as its key on screen, where it is noticed, rather than as nothing. Missing ones
    /// are remembered for [`missing`](Self::missing).
    #[must_use]
    pub fn get<'a>(&'a self, key: &'a str) -> &'a str {
        if let Some(s) = self.tables.get(&self.language).and_then(|t| t.get(key)) {
            return s;
        }
        if let Some(s) = self.fallback.as_ref().and_then(|f| self.tables.get(f)).and_then(|t| t.get(key)) {
            return s;
        }
        if let Ok(mut m) = self.missing.lock() {
            m.insert(key.to_string());
        }
        key
    }

    /// [`get`](Self::get) with each `{name}` replaced by its value from `args`. A placeholder with
    /// no value is left as it is.
    #[must_use]
    pub fn format(&self, key: &str, args: &[(&str, &str)]) -> String {
        let mut s = self.get(key).to_string();
        for (name, value) in args {
            s = s.replace(&format!("{{{name}}}"), value);
        }
        s
    }

    /// A string that depends on a count: `"{key}.one"` when `n` is 1 and the table has one, else
    /// `"{key}.other"`, else `key` itself — with `{n}` (and `args`) filled in.
    ///
    /// ```text
    /// "coins": { "one": "{n} coin", "other": "{n} coins" }   // en
    /// "coins": { "other": "{n} altın" }                       // tr: no plural after a number
    /// ```
    ///
    /// Two forms cover the languages this engine's games ship in so far: English and its kin say
    /// "1 coin, 2 coins", and Turkish keeps the noun singular after any number, so it writes only
    /// `other`. Languages with more forms (Russian, Arabic, Polish) need CLDR's full rules, which
    /// this does not implement — their `other` is used for every count except 1.
    #[must_use]
    pub fn format_count(&self, key: &str, n: i64, args: &[(&str, &str)]) -> String {
        let one = format!("{key}.one");
        let other = format!("{key}.other");
        // The forms come from ONE language: the current one if it has either, else the fallback.
        // Mixing them is how a Turkish `1` would come out as the English "1 coin".
        let table = [Some(&self.language), self.fallback.as_ref()]
            .into_iter()
            .flatten()
            .filter_map(|l| self.tables.get(l))
            .find(|t| t.contains_key(&one) || t.contains_key(&other));
        let pick = match table {
            Some(t) if n == 1 && t.contains_key(&one) => t[&one].clone(),
            Some(t) if t.contains_key(&other) => t[&other].clone(),
            Some(t) => t[&one].clone(),
            None => self.get(key).to_string(),
        };
        let mut s = pick.replace("{n}", &n.to_string());
        for (name, value) in args {
            s = s.replace(&format!("{{{name}}}"), value);
        }
        s
    }

    /// Every key asked for that no table had, sorted — the list a translator needs.
    #[must_use]
    pub fn missing(&self) -> Vec<String> {
        self.missing.lock().map(|m| m.iter().cloned().collect()).unwrap_or_default()
    }

    /// The system's language, if it says: `LC_ALL`, `LC_MESSAGES` or `LANG` on Linux and macOS
    /// (`"tr_TR.UTF-8"` → `"tr"`), the browser's `navigator.language` on the web. `None` where
    /// neither is available (Windows without those variables), so a game picks its own default.
    #[must_use]
    pub fn system_language() -> Option<String> {
        #[cfg(target_arch = "wasm32")]
        {
            web_sys::window().and_then(|w| w.navigator().language()).map(|l| normalise(&l))
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            ["LC_ALL", "LC_MESSAGES", "LANG"]
                .iter()
                .filter_map(|v| std::env::var(v).ok())
                .find(|v| !v.is_empty() && v != "C" && v != "POSIX")
                .map(|v| normalise(&v))
        }
    }
}

/// `"tr_TR.UTF-8"`, `"tr-TR"`, `"TR"` → `"tr"`.
fn normalise(language: &str) -> String {
    language
        .split(['_', '-', '.', '@'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn flatten(prefix: &str, value: &serde_json::Value, out: &mut HashMap<String, String>) -> Result<(), LocaleError> {
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                let key = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
                flatten(&key, v, out)?;
            }
            Ok(())
        }
        serde_json::Value::String(s) => {
            out.insert(prefix.to_string(), s.clone());
            Ok(())
        }
        _ => Err(LocaleError::NotAString(prefix.to_string())),
    }
}

/// A label in the current language: beside a `Text`, it replaces the text's content with
/// [`Locale::format`] of `key` and `args`, the frame it is drawn — so switching
/// [`Locale::set_language`] relabels every one at once. Without a [`Locale`] resource the `Text`
/// shows as authored.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct LocalizedText {
    /// The string's key, dotted (`"menu.start"`).
    pub key: String,
    /// `{name}` placeholders and their values.
    pub args: Vec<(String, String)>,
    /// A count, which picks the string's form ([`Locale::format_count`]) and fills `{n}`.
    pub count: Option<i64>,
}

impl LocalizedText {
    /// The string `key`, with no placeholders.
    #[must_use]
    pub fn new(key: &str) -> Self {
        Self { key: key.to_string(), args: Vec::new(), count: None }
    }

    /// With `{name}` filled by `value`.
    #[must_use]
    pub fn with_arg(mut self, name: &str, value: impl ToString) -> Self {
        self.args.push((name.to_string(), value.to_string()));
        self
    }

    /// With a count: the string's one/other form by `n`, and `{n}` filled.
    #[must_use]
    pub fn with_count(mut self, n: i64) -> Self {
        self.count = Some(n);
        self
    }

    /// What it shows under `locale`.
    #[must_use]
    pub fn resolve(&self, locale: &Locale) -> String {
        let args: Vec<(&str, &str)> = self.args.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        match self.count {
            Some(n) => locale.format_count(&self.key, n, &args),
            None => locale.format(&self.key, &args),
        }
    }
}

gizmo_core::impl_component!(LocalizedText);

#[cfg(test)]
mod tests {
    use super::*;

    fn two_languages() -> Locale {
        let mut l = Locale::new("tr_TR.UTF-8");
        l.load_json("tr", r#"{"menu": {"start": "Başla", "score": "Skor: {n}"}, "only_tr": "yalnız"}"#).unwrap();
        l.load_json("en-GB", r#"{"menu": {"start": "Start", "score": "Score: {n}"}}"#).unwrap();
        l
    }

    #[test]
    fn nested_keys_are_dotted_and_the_language_picks_the_table() {
        let mut l = two_languages();
        assert_eq!(l.language(), "tr");
        assert_eq!(l.get("menu.start"), "Başla");
        l.set_language("EN");
        assert_eq!(l.get("menu.start"), "Start");
        assert_eq!(l.languages(), vec!["en", "tr"]);
    }

    /// A string missing in the language falls back to the first one loaded, then to its key —
    /// and every key no table had is listed for the translator.
    #[test]
    fn missing_strings_fall_back_then_show_their_key_and_are_listed() {
        let mut l = two_languages();
        l.set_language("en");
        assert_eq!(l.get("only_tr"), "yalnız", "the fallback is the first language loaded");
        assert_eq!(l.get("menu.quit"), "menu.quit");
        assert_eq!(l.get("menu.quit"), "menu.quit");
        l.set_fallback("en");
        l.set_language("de");
        assert_eq!(l.get("menu.start"), "Start");
        assert_eq!(l.missing(), vec!["menu.quit".to_string()]);
    }

    /// English picks one/other by the count; Turkish, writing only `other`, uses it for 1 too;
    /// a key with neither form is used as it is; the count fills `{n}` beside other arguments.
    #[test]
    fn a_count_picks_its_form() {
        let mut l = Locale::new("en");
        l.load_json("en", r#"{"coins": {"one": "{n} coin", "other": "{n} coins"}, "left": "{n} left of {total}"}"#).unwrap();
        l.load_json("tr", r#"{"coins": {"other": "{n} altın"}}"#).unwrap();
        assert_eq!(l.format_count("coins", 1, &[]), "1 coin");
        assert_eq!(l.format_count("coins", 3, &[]), "3 coins");
        assert_eq!(l.format_count("coins", 0, &[]), "0 coins");
        assert_eq!(l.format_count("left", 2, &[("total", "5")]), "2 left of 5");
        l.set_language("tr");
        assert_eq!(l.format_count("coins", 1, &[]), "1 altın");
        assert_eq!(l.format_count("coins", 4, &[]), "4 altın");
        assert!(l.missing().is_empty(), "probing the forms reported them missing: {:?}", l.missing());
        assert_eq!(LocalizedText::new("coins").with_count(2).resolve(&l), "2 altın");
    }

    #[test]
    fn placeholders_are_filled_and_unknown_ones_kept() {
        let l = two_languages();
        assert_eq!(l.format("menu.score", &[("n", "12")]), "Skor: 12");
        assert_eq!(l.format("menu.score", &[]), "Skor: {n}");
        let label = LocalizedText::new("menu.score").with_arg("n", 7);
        assert_eq!(label.resolve(&l), "Skor: 7");
    }

    /// A bad table adds nothing — not the strings before the bad entry either.
    #[test]
    fn a_bad_table_is_refused_whole() {
        let mut l = Locale::new("tr");
        assert!(matches!(l.load_json("tr", "{not json"), Err(LocaleError::Json(_))));
        let e = l.load_json("tr", r#"{"a": "fine", "b": 3}"#).unwrap_err();
        assert!(matches!(e, LocaleError::NotAString(ref k) if k == "b"), "{e}");
        assert_eq!(l.get("a"), "a", "half a table was loaded");
    }

    #[test]
    fn language_tags_are_normalised() {
        for (raw, want) in [("tr_TR.UTF-8", "tr"), ("en-US", "en"), ("DE", "de"), ("sr@latin", "sr")] {
            assert_eq!(normalise(raw), want);
        }
    }
}
