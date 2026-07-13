//! Render-pass recorders extracted from `default_render_pass` (Tier 3 round-2: mega-fn
//! split, then one-file-per-pass). Every recorder is side-effect-only: it records commands
//! into the encoder and returns nothing. Splitting the former 931-line `passes.rs` into one
//! file per pass keeps the render path navigable — pure moves, no behaviour change.

mod forward;
mod geometry;
mod screen_space;
#[cfg(not(target_arch = "wasm32"))]
mod shadow;
mod ssao;
mod taa;

pub(super) use self::forward::record_forward_and_fluid;
pub(super) use self::geometry::record_deferred_geometry;
pub(super) use self::screen_space::record_screen_space_effects;
#[cfg(not(target_arch = "wasm32"))]
pub(super) use self::shadow::record_shadow_passes;
pub(super) use self::ssao::record_ssao;
pub(super) use self::taa::record_taa_and_overlays;
