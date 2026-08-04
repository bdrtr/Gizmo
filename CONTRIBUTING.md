# Contributing to Gizmo Engine

Thanks for looking. This document exists because an audit pointed out that the
project's only engineering document was written in a language most potential
contributors do not read, and that there was no way in at all — no contribution
guide, no issue templates, nothing under `.github/` but a CI file. The effective
bus factor was one. This is the first step out of that.

## Getting a build

```sh
git clone https://github.com/bdrtr/Gizmo && cd Gizmo
cargo build --workspace
cargo test --workspace
```

A fresh clone runs everything. Large `.glb` models are not committed; the two demos
that showcase one fall back to procedural geometry and say so — see
[`assets/README.md`](assets/README.md).

Physics demos need `--release`. In a debug build the broad/narrow-phase is slow enough
to look broken:

```sh
cargo run --release -p demo --bin bevy_3d_scene   # default scene
cargo run --release -p demo --bin car_demo        # vehicle dynamics
```

The machine this engine was written on has limited RAM, so `.cargo/config.toml` caps
`jobs = 4`. If you have more headroom, override it locally rather than editing the
committed file.

## What CI checks

Everything below runs on every pull request. Running them locally first is faster than
finding out from a red badge.

```sh
# The lint gate, exactly as CI invokes it. `-D warnings` is real; the two `-A`s are a
# shrinking grandfathered list, not an invitation to add more.
cargo clippy --workspace --all-features --all-targets -- -D warnings \
  -A clippy::too_many_arguments -A clippy::type_complexity

# Determinism and stability: a tower collapse run three times, hashes must match.
cargo run --release -p demo --bin headless_stress_test

# Feature composition. The facade used to compile in exactly one configuration; this
# is what stops that recurring.
cargo hack check -p gizmo-engine --feature-powerset --depth 2 --no-dev-deps

# Supply chain. Every exception in deny.toml carries a written reason.
cargo deny --all-features check
```

CI additionally runs a three-OS test matrix, an MSRV check (Rust 1.92), a WASM build,
and Miri over the ECS's unsafe surface. `rustfmt` is report-only — the tree is not yet
fmt-clean, so a formatting diff will not fail your PR, but please don't reflow code you
aren't otherwise touching.

## How changes are expected to land

The working rule, from `docs/ENGINE.md` §8:

> fix → write a regression test → build/test/clippy → done.

Two things that follow from it and are worth stating outright:

**A test that cannot fail is worse than no test.** A recent example from this repo: a
guard written as `assert!(cfg!(not(feature = "taffy/calc")))` looks like it protects an
`unsafe impl`, but a dependency's feature flags are not visible as `cfg` to the dependent
crate, so it was always true. It was deleted rather than kept for the green tick.

**Pick a soak horizon past the onset of the bug.** A 600-frame stability test once
shipped green while hiding an explosion at frame ~853.

If you change physics behaviour, say so and show `headless_stress_test`. The determinism
hash changing is not automatically wrong — but it must be intentional, and the reason
belongs in the commit message.

## Where things are

```
gizmo-math ─┬─ gizmo-core ─┬─ gizmo-physics-{core,rigid,dynamics,soft}
            │              ├─ gizmo-renderer ─ gizmo-{window,ui,editor}
            │              ├─ gizmo-{scene,net,ai,animation,audio,scripting}
            └──────────────┴─ gizmo-app ─ gizmo-engine ─ demo / cradle / server
```

- [`docs/ENGINE.md`](docs/ENGINE.md) — architecture, roadmap, the determinism contract,
  and a list of bugs already investigated and refuted. **Read §7 before reporting
  something as a bug**; several plausible-looking findings have been chased more than once.
- [`docs/AUDIT-2026-08.md`](docs/AUDIT-2026-08.md) — an external review, every finding
  pinned to `file:line`.
- [`docs/FIXPLAN.md`](docs/FIXPLAN.md) — what is being worked on and what is deliberately
  deferred. A good place to find something to pick up.

## Language

`docs/ENGINE.md` and roughly 40% of the inline comments are in Turkish, for historical
reasons — the project was written solo. New code should carry **English** doc comments
(`///`, `//!`), because they end up on docs.rs. Inline `//` comments in either language
are fine; translating existing ones as you touch surrounding code is welcome and tracked
as a roadmap item.

Commit messages, issues and pull requests: English please, so the archive stays
searchable for everyone.

## Reporting a bug

Include the engine version or commit, your OS and GPU, and — if it is physics — the
smallest scene that reproduces it. Determinism means a reproducer that works for you
should work identically for a maintainer on the same platform, which makes physics bugs
unusually tractable here. Please use that.

## Scope and stability

The engine is `0.x`; no API is frozen. `docs/ENGINE.md` §4 describes the staged path to
1.0, where the dependency-light crates can stabilise ahead of the graphics layer. If your
change breaks a public API, that is allowed — but say so in the commit message and update
`CHANGELOG.md`.

## Licence

By contributing you agree that your work is dual-licensed under MIT and Apache-2.0, matching
the rest of the repository.
