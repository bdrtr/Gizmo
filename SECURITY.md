# Security Policy

## Supported versions

Gizmo is `0.x`. Only the latest published version on crates.io receives fixes; there are
no maintained release branches yet.

## Reporting a vulnerability

Please report privately via [GitHub's private vulnerability
reporting](https://github.com/bdrtr/Gizmo/security/advisories/new) rather than opening a
public issue.

The project is maintained by one person, so please allow a reasonable window before
public disclosure.

## What counts

Gizmo is a game engine, not a network service, so the realistic threat model is **hostile
content** and **hostile peers**, not remote attackers hitting a server:

- **Asset parsing** — glTF, OBJ, images, KTX2 and RON scene files are parsed at runtime.
  A crash, out-of-bounds read or arbitrary write triggered by a malformed asset is in
  scope. This matters for any game that loads user-supplied maps or mods.
- **Networking** — `gizmo-net` deserializes peer input and snapshots. A malicious peer
  causing memory unsafety or unbounded allocation is in scope.
- **Scripting** — `gizmo-scripting` runs Lua with the dangerous globals nilled out. A
  sandbox escape is in scope.
- **Soundness** — the workspace contains `unsafe`, concentrated in the ECS's type-erased
  storage. **Any way to trigger undefined behaviour from safe user code is a
  vulnerability**, even without a demonstrated exploit. The ECS unsafe surface is covered
  by a Miri job under Tree Borrows, and reports of gaps in that coverage are welcome.

## What does not count

- Panics on programmer error (wrong types, out-of-range indices from your own code).
- Denial of service from your own scene being too large.
- Advisories in dependencies that are already documented with a rationale in
  [`deny.toml`](deny.toml). If you think one of those rationales is wrong, that is a
  legitimate report — argue the reasoning, not the advisory ID.

## Supply chain

`cargo deny --all-features check` runs on every pull request and covers advisories,
licences, sources and duplicate versions. Every accepted exception carries a written
justification in `deny.toml`.
