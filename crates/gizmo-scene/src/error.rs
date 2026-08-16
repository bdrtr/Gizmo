//! Scene serialization error types.
//!
//! The RON parser is an implementation detail of the scene *format*, not of this crate's
//! *API*: the two RON-shaped failures are carried by [`ParseError`] and [`SerializeError`],
//! which are types this crate owns and whose payloads are private. A major release of the
//! parser therefore cannot force a major release of `gizmo-scene` (docs/ENGINE.md §4 — this
//! crate is Stage A and goes to 1.x, where 1.0 promises no breaking change without a 2.0).

/// A RON **parse** failure — the payload of [`SceneError::Parse`], produced when a scene or
/// prefab file/string is not valid RON or does not match the expected shape.
///
/// An opaque wrapper around the parser's own error. [`Display`](std::fmt::Display) and
/// [`Error::source`](std::error::Error::source) forward to it verbatim, so a printed error
/// chain reads exactly as it did when the payload was the parser's type — the message still
/// begins with the failure position, e.g. `4:12: Expected string`.
///
/// The main thing the transparent payload gave callers that a message string does not is the
/// failure position as numbers; [`line`](Self::line) and [`column`](Self::column) hand that
/// back.
///
/// # What the seal DOES cost — stated rather than glossed
///
/// Three smaller capabilities went with the transparent payload and are **not** replaced,
/// deliberately, because every replacement for them would put a parser type back on this
/// crate's surface and undo the seal:
///
/// - **Matching on the failure *kind*.** The old payload's `code` field was the parser's own
///   error enum, so a caller could tell "unexpected end of input" from "expected a string"
///   programmatically. That is a message string now. If a caller genuinely needs to branch on
///   the kind, the fix is a small owned enum of our own — not re-exposing the parser's.
/// - **The END of the error span.** Only the start is handed back; see [`line`](Self::line).
/// - **`Clone` / `PartialEq` on the payload.** The parser's error derives both; this wrapper
///   derives only `Debug`. (`SceneError` itself was never `Clone` — it carries an
///   `io::Error` — so this only affects a caller who had destructured the parse variant.)
///
/// Also worth knowing: `SceneError::source()` now yields a `&ParseError`, so a
/// `downcast_ref::<ron::error::SpannedError>()` on it no longer matches. Downcasting to
/// `ParseError` does.
///
/// # Evidence that the seal holds
///
/// The payload used to be the parser's `SpannedError` — a struct with public fields — so both
/// of these compiled before this type existed. They are the seal's regression tests.
///
/// ```compile_fail
/// // The payload is no longer the parser's type.
/// fn payload(e: &gizmo_scene::SceneError) {
///     if let gizmo_scene::SceneError::Parse(p) = e {
///         let _: &ron::error::SpannedError = p;
///     }
/// }
/// ```
///
/// ```compile_fail
/// // …and its fields are no longer reachable through it.
/// fn line(e: &gizmo_scene::SceneError) -> Option<usize> {
///     match e {
///         gizmo_scene::SceneError::Parse(p) => Some(p.span.start.line),
///         _ => None,
///     }
/// }
/// ```
///
/// What replaces the second one, and does compile:
///
/// ```
/// fn position(e: &gizmo_scene::SceneError) -> Option<(usize, usize)> {
///     match e {
///         gizmo_scene::SceneError::Parse(p) => Some((p.line(), p.column())),
///         _ => None,
///     }
/// }
/// ```
// The two `compile_fail` examples above are this seal's regression tests: both compiled
// before the wrapper existed (verified by running them unmarked against the old enum, where
// they passed as ordinary doc-tests). `compile_fail` on its own only asserts *some* error and
// this toolchain's rustdoc ignores a `compile_fail,E0nnn` code, so the codes are recorded
// here by hand — un-mark the blocks and read the diagnostics to get, in order:
//   1. E0308 mismatched types — expected `&SpannedError`, found `&ParseError`
//   2. E0609 no field `span` on type `&ParseError`
#[derive(Debug)]
pub struct ParseError(ron::error::SpannedError);

impl ParseError {
    /// 1-based line the parser was on when it gave up.
    ///
    /// The parser reports a *range* (it also knows where the cursor ended up); this is the
    /// start of it, which is the position an editor should jump to and what the old public
    /// payload exposed as `position.line`.
    pub fn line(&self) -> usize {
        self.0.span.start.line
    }

    /// 1-based column the parser was on when it gave up — the start of the error range, the
    /// counterpart of [`line`](Self::line).
    pub fn column(&self) -> usize {
        self.0.span.start.col
    }
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl std::error::Error for ParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        std::error::Error::source(&self.0)
    }
}

/// A RON **serialization** failure — the payload of [`SceneError::Serialize`], produced when
/// a scene or prefab cannot be encoded (a component whose `Serialize` impl fails, most often).
///
/// The write-side counterpart of [`ParseError`] and opaque for the same reason: it forwards
/// [`Display`](std::fmt::Display) and [`Error::source`](std::error::Error::source) to the
/// encoder's own error while keeping that error's type off this crate's public surface. There
/// is no position to expose here — a serializer failure is not tied to a spot in a file.
///
/// The costs listed on [`ParseError`] apply here too, minus the position ones: the encoder's
/// error kind is no longer matchable, and `Clone`/`PartialEq` are not forwarded.
///
/// # Evidence that the seal holds
///
/// The payload used to be the encoder's error type verbatim, so this compiled:
///
/// ```compile_fail
/// fn payload(e: &gizmo_scene::SceneError) {
///     if let gizmo_scene::SceneError::Serialize(s) = e {
///         let _: &ron::Error = s;
///     }
/// }
/// ```
// Verified the same way as `ParseError`'s: unmarked, this compiled against the old enum.
// Un-mark it now and the diagnostic is E0308 mismatched types — expected `&ron::Error`,
// found `&SerializeError`.
#[derive(Debug)]
pub struct SerializeError(ron::Error);

impl std::fmt::Display for SerializeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl std::error::Error for SerializeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        std::error::Error::source(&self.0)
    }
}

/// Errors that can occur while saving or loading scenes and prefabs.
#[derive(Debug)]
#[non_exhaustive]
pub enum SceneError {
    /// Filesystem I/O failure (reading or writing the scene file).
    Io(std::io::Error),
    /// RON deserialization (parse) failure when loading a scene/prefab.
    Parse(ParseError),
    /// RON serialization failure when saving a scene/prefab.
    Serialize(SerializeError),
    /// The file declares a format version this build does not know how to read.
    ///
    /// Loading it anyway would silently drop whatever the newer engine wrote, since the
    /// unknown fields are already gone by the time parsing succeeds — so this fails instead.
    UnsupportedVersion {
        /// Path of the offending file.
        path: String,
        /// Version the file declares.
        found: u32,
        /// Highest version this build understands.
        supported: u32,
    },
    /// The file predates the current component encoding, and no version field can say so.
    ///
    /// A component is stored today as a **string** holding its own RON
    /// (`"Transform": "(position:(0,0,0),…)"`). Older files wrote a nested map keyed by field
    /// name, with enums internally tagged (`"shape": {"type": "Aabb", …}`) — a `bevy_reflect`
    /// shape rather than a serde one. Such a file fails at the parser, *before*
    /// [`SceneData::migrate`](crate::scene::SceneData::migrate) can look at its version, and the
    /// raw failure is a column number and "Expected string", which tells the person holding an
    /// old scene nothing. This says what is actually wrong.
    LegacyComponentEncoding {
        /// Path of the offending file.
        path: String,
    },
}

impl std::fmt::Display for SceneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SceneError::Io(_) => write!(f, "scene file I/O error"),
            SceneError::Parse(_) => write!(f, "scene file parse error"),
            SceneError::Serialize(_) => write!(f, "scene serialization error"),
            SceneError::UnsupportedVersion {
                path,
                found,
                supported,
            } => write!(
                f,
                "scene file '{path}' is format version {found}, but this build understands \
                 at most {supported} — it was written by a newer version of the engine"
            ),
            SceneError::LegacyComponentEncoding { path } => write!(
                f,
                "scene file '{path}' stores its components in the old reflection format \
                 (nested maps, internally tagged enums); this build reads components written as \
                 RON strings. Re-save it from an engine version that can still open it, or \
                 rebuild the scene"
            ),
        }
    }
}

impl std::error::Error for SceneError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SceneError::Io(e) => Some(e),
            SceneError::Parse(e) => Some(e),
            SceneError::Serialize(e) => Some(e),
            // No inner error — the mismatch IS the failure in both of these.
            SceneError::UnsupportedVersion { .. } | SceneError::LegacyComponentEncoding { .. } => {
                None
            }
        }
    }
}

impl From<std::io::Error> for SceneError {
    fn from(e: std::io::Error) -> Self {
        SceneError::Io(e)
    }
}

// The two conversions below are the last place a parser type appears in this crate's public
// API, and they are kept deliberately: they are what makes `?` work on the parser calls in
// `scene.rs` (four sites). Unlike a public *payload*, they force nothing on a caller — no
// signature a caller writes has to name a parser type to receive, match or report a
// `SceneError` — so a parser major bump costs a re-write of these two `from` bodies, not of
// anybody's error handling.

impl From<ron::error::SpannedError> for SceneError {
    fn from(e: ron::error::SpannedError) -> Self {
        SceneError::Parse(ParseError(e))
    }
}

impl From<ron::Error> for SceneError {
    fn from(e: ron::Error) -> Self {
        SceneError::Serialize(SerializeError(e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    // A static assertion of the seal's shape: both RON-shaped payloads are types this crate
    // owns. This is a GUARD, not a proof — it names only our own types, so it says nothing
    // about what the payloads used to be; what it catches is someone widening a variant back
    // to a foreign type later. The actual proof that the payloads changed is the pair of
    // `compile_fail` doc-tests on `ParseError`/`SerializeError`, which compiled before.
    #[allow(dead_code)]
    fn scene_error_payloads_are_crate_owned(e: &SceneError) {
        match e {
            SceneError::Parse(p) => {
                let _: &ParseError = p;
            }
            SceneError::Serialize(s) => {
                let _: &SerializeError = s;
            }
            _ => {}
        }
    }

    // The one capability the seal takes away — reading the failure position off the payload —
    // must come back through `line()`/`column()`. The parse below fails on the THIRD line of
    // the input, so a wrapper that reported, say, a constant or the end of input would show up
    // here. The column is checked against the parser's own rendering rather than hard-coded,
    // since that is the number the accessor is supposed to be handing through.
    #[test]
    fn parse_error_reports_the_failure_position() {
        let src = "(\n    version: 1,\n    entities: notalist,\n)";
        let err: SceneError = ron::from_str::<crate::scene::SceneData>(src).unwrap_err().into();
        let SceneError::Parse(ref p) = err else {
            panic!("malformed RON must land in the Parse variant, got {err:?}");
        };
        assert_eq!(p.line(), 3, "error is on the third line of the input");
        // The parser prints `line:col: message` (or `line:col-line:col: message`), so the
        // accessors must agree with the start of its own message.
        let rendered = p.to_string();
        assert!(
            rendered.starts_with(&format!("{}:{}", p.line(), p.column())),
            "line()/column() disagree with the parser's own position in {rendered:?}",
        );
    }

    // Wrapping must not shorten the error chain or swallow the detail: `SceneError`'s own
    // Display stays the short classification, and the concrete reason (with its position) is
    // still one `source()` hop away, exactly as when the payload was the parser's own type.
    #[test]
    fn parse_error_display_is_forwarded_verbatim() {
        let inner: ron::error::SpannedError =
            ron::from_str::<i32>("definitely not ron").unwrap_err();
        let expected = inner.to_string();
        let err: SceneError = inner.into();
        assert_eq!(err.to_string(), "scene file parse error");
        let src = err.source().expect("Parse must expose its source");
        assert_eq!(src.to_string(), expected, "wrapper must Display like the parser error");
    }

    // Same for the write side.
    #[test]
    fn serialize_error_display_is_forwarded_verbatim() {
        let inner = ron::Error::Message("boom".to_string());
        let expected = inner.to_string();
        let err: SceneError = inner.into();
        assert_eq!(err.to_string(), "scene serialization error");
        let src = err.source().expect("Serialize must expose its source");
        assert_eq!(src.to_string(), expected);
    }

    // `?`/`From` glue: an `io::Error` bubbling out of a scene read/write must land in
    // the `Io` variant (not silently reclassified), keep a human-readable Display, and
    // expose the underlying io::Error via `source()` so callers can downcast/inspect it.
    #[test]
    fn io_error_converts_and_preserves_source() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "no such scene");
        let err: SceneError = io.into();
        assert!(matches!(err, SceneError::Io(_)), "io::Error must map to Io variant");
        assert_eq!(err.to_string(), "scene file I/O error");
        let src = err.source().expect("Io variant must expose its underlying source");
        // The wrapped source is the original io::Error, so its own message survives.
        assert!(src.to_string().contains("no such scene"));
    }

    // A RON PARSE failure (loading a malformed scene) is a `SpannedError` and must map to
    // the `Parse` variant — distinct from `Serialize`, so load vs save failures never blur.
    #[test]
    fn parse_error_converts_and_preserves_source() {
        let spanned: ron::error::SpannedError =
            ron::from_str::<i32>("definitely not ron").unwrap_err();
        let err: SceneError = spanned.into();
        assert!(matches!(err, SceneError::Parse(_)), "SpannedError must map to Parse variant");
        assert_eq!(err.to_string(), "scene file parse error");
        assert!(err.source().is_some(), "Parse variant must expose its source");
    }

    // A RON SERIALIZE failure (saving) is a bare `ron::Error` and must map to `Serialize`.
    #[test]
    fn serialize_error_converts_and_preserves_source() {
        let ron_err = ron::Error::Message("boom".to_string());
        let err: SceneError = ron_err.into();
        assert!(matches!(err, SceneError::Serialize(_)), "ron::Error must map to Serialize variant");
        assert_eq!(err.to_string(), "scene serialization error");
        assert!(err.source().is_some(), "Serialize variant must expose its source");
    }

    // Each variant's Display must be distinct so a log/UI message tells the three failure
    // classes (I/O vs parse vs serialize) apart at a glance.
    #[test]
    fn each_variant_has_a_distinct_display() {
        let io: SceneError = std::io::Error::other("x").into();
        let parse: SceneError = ron::from_str::<i32>("zz").unwrap_err().into();
        let ser: SceneError = ron::Error::Message("y".to_string()).into();

        let msgs = [io.to_string(), parse.to_string(), ser.to_string()];
        // All three must differ from one another.
        assert_ne!(msgs[0], msgs[1]);
        assert_ne!(msgs[1], msgs[2]);
        assert_ne!(msgs[0], msgs[2]);
        // And every variant must expose a source (the `?`-chain is never dropped).
        assert!(io.source().is_some() && parse.source().is_some() && ser.source().is_some());
    }
}
