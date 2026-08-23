# Tombstone crates

Three names on crates.io outlived the move from the old self-hosted repository to this one.
They are not part of the workspace and nothing here is built by CI — each is a single
`lib.rs` whose only job is to be the last thing published under its name, so that anyone
who reaches for the old name lands on a signpost instead of code from May 2026.

| old name | last real release | superseded by |
|---|---|---|
| `gizmo-physics` | 0.1.2, 2026-05-13 | `gizmo-physics-core`, `-rigid`, `-dynamics`, `-soft` |
| `gizmo-network` | 0.1.7, 2026-06-02 | `gizmo-net` |
| `gizmo-studio` | 0.1.7, 2026-06-02 | nothing published — the editor ships with the repository |

## Why they were a hazard

All three carried the workspace's blurb, *"A custom ECS and physics engine aimed for realistic
simulations"* — word for word what the live crates say. Someone searching for `gizmo-physics`
could not tell from the description that they were looking at an abandoned line, and
`cargo add gizmo-physics` would have given them a 0.1.2 from before the split.

The cleanup after the move was done for the names that survived it — `gizmo-engine` and
`gizmo-core` both have their pre-move versions yanked. These three were simply missed.

## What was done

Each got a final release whose description names its successor and whose `repository` points
here, and the older versions were yanked. The final release stays unyanked on purpose: a crate
with every version yanked still appears in search results but has nothing to say, whereas this
one answers the question the searcher actually has.

## Excluded from the workspace

`Cargo.toml`'s `exclude` list keeps them out, for the same reason `benchmarks` is excluded:
they must never enter the engine's dependency graph or its build.
