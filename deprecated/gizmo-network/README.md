# `gizmo-network` is deprecated

This name belongs to an earlier generation of the engine, published from a self-hosted
repository that no longer backs it. Its last real release was 0.1.7 (2026-06-02).

It was renamed: the successor is [`gizmo-net`](https://crates.io/crates/gizmo-net), which
carries the client/server and rollback work and is versioned with the rest of the engine.

```toml
# before
gizmo-network = "0.1"
# now
gizmo-net = "0.10"
```

The rename is not a drop-in: the crate was rewritten between the two lines, and the network
backends are behind the `client-server` and `rollback` features rather than always on.

The engine now lives at <https://github.com/bdrtr/Gizmo>.
