//! Says which render path this target takes: the render systems here pick their passes by it, the
//! same way `gizmo-renderer` picks its pipelines (that crate's `build.rs` has the same function, and
//! the two must agree).

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    forward_only();
}

/// `gizmo_forward_only`: the reduced render path — one forward pass, four bind groups, no shadows,
/// deferred lighting, SSAO or TAA, and a render-size budget. The browser needs it (WebGPU caps bind
/// groups at four) and so does Android: an Adreno 618's Vulkan driver allows four as well, and the
/// native path asks for six (`max_bind_groups: requested 6, allowed 4`, the first APK's crash). It
/// is a cfg of its own because "which render path" and "which platform API" are different
/// questions — Android keeps threads, files and the native timer, the browser has none of them.
fn forward_only() {
    println!("cargo::rustc-check-cfg=cfg(gizmo_forward_only)");
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if arch == "wasm32" || os == "android" {
        println!("cargo::rustc-cfg=gizmo_forward_only");
    }
}
