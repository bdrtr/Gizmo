//! Links the C++ runtime on Android, and says which render path this target takes.
//!
//! `meshopt` (the LOD simplifier) is C++, and on Android nothing links a C++ standard library by
//! default: the first APK loaded, then died in `dlopen` with `cannot locate symbol
//! "__gxx_personality_v0"`. The NDK's static `libc++` is linked into the app's library, so there
//! is no `libc++_shared.so` to package alongside it. Every other target links its C++ runtime on
//! its own.
//!
//! Plain `-l` rather than `static=`: rustc would look for a `static=` archive itself and does not
//! know the NDK's sysroot; the NDK's clang, which links the app, does.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    forward_only();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        println!("cargo:rustc-link-lib=c++_static");
        println!("cargo:rustc-link-lib=c++abi");
    }
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
