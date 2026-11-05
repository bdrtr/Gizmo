//! Games written once and hosted twice: by a desktop binary in `demo/src/bin/` and by the browser
//! build in `demo-web/`. Each game module exposes `app()` — the whole game as an `App`, input,
//! scene and update — and `self_test()`, its headless `--test`.
//!
//! A game lives here rather than in `demo/src/bin/` once it has to run in the browser too:
//! `demo-web` is a library crate and cannot reach into another crate's binaries.

pub mod nisanci;
pub mod pinbol;
