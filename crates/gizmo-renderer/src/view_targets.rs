//! A view's own **screen targets**: everything as big as the picture — the depth buffer, the G-buffer,
//! the HDR target, every screen-space effect's textures — at the size of the view rather than the
//! window's.
//!
//! A camera with a viewport (a minimap, one player of a split screen) or one that draws into a texture
//! was drawn at the window's size and its picture squeezed into its rectangle by the post chain: a map
//! in a sixth of the screen paid for every pixel of it — in `dunya` on a Radeon 780M, 3.5 ms of a
//! 17 ms frame for 6 % of its pixels (measured 2026-11-16), because the frame is bound by what it
//! writes per pixel.
//!
//! Each renderer state with such targets declares them with `screen_targets!`: the fields its
//! `resize` writes, gathered in a struct of their own, with a swap. `ViewTargets` holds a set at one
//! size; [`Renderer::with_view_targets`](crate::Renderer::with_view_targets) swaps it in for a view's
//! draw and back. Only the targets move: the states themselves — their settings, pipelines, the
//! irradiance volume, the fluid's particles — stay the renderer's, so nothing about them can differ
//! between the views. Several views of one size share one set: each draw writes all of it before it
//! reads any of it, and each view is its own submission.
//!
//! A set is made by resizing the renderer's own states to the view's size and taking what that made,
//! then putting the renderer's own back — the same code as a window resize, so a target the resize
//! makes cannot be missing from a view's. It is made again when the renderer has rebuilt one of the
//! states (a hot reload, `enable_ssr`): each set remembers a handle of each state it was made beside
//! (the `key`), one that `resize` does not replace.

/// Declares the screen targets of a renderer state: `State => Name { key k: K; field: Type, … }`
/// makes `Name`, holding those fields and the state's `k` as it was when they were taken, and on the
/// state `take_screen_targets` (copies of them — handles, so cheap), `swap_screen_targets` and
/// `built_beside` (whether a set was taken from this build of the state).
macro_rules! screen_targets {
    ($owner:ty => $name:ident { key $key:ident : $kty:ty; $($field:ident : $ty:ty),* $(,)? }) => {
        /// The screen targets of
        #[doc = concat!("[`", stringify!($owner), "`]")]
        /// — see [`crate::view_targets`].
        pub(crate) struct $name {
            key: $kty,
            $($field: $ty,)*
        }

        impl $owner {
            /// Its screen targets as they are — see [`crate::view_targets`].
            pub(crate) fn take_screen_targets(&self) -> $name {
                $name { key: self.$key.clone(), $($field: self.$field.clone(),)* }
            }

            /// Exchanges its screen targets with `targets`.
            pub(crate) fn swap_screen_targets(&mut self, targets: &mut $name) {
                $(std::mem::swap(&mut self.$field, &mut targets.$field);)*
            }

            /// Whether `targets` were taken from this build of it, and so fit it.
            pub(crate) fn built_beside(&self, targets: &$name) -> bool {
                self.$key == targets.key
            }
        }
    };
}
pub(crate) use screen_targets;

/// Swaps an optional state's targets with an optional set: both there, they swap; the set's presence
/// is checked by [`ViewTargets::fits`] before any swap.
macro_rules! swap_optional {
    ($state:expr, $targets:expr) => {
        if let (Some(state), Some(targets)) = ($state.as_mut(), $targets.as_mut()) {
            state.swap_screen_targets(targets);
        }
    };
}

/// Whether an optional state and an optional set agree: both absent, or the set taken from this build.
fn fits<S, T>(state: Option<&S>, targets: Option<&T>, beside: impl Fn(&S, &T) -> bool) -> bool {
    match (state, targets) {
        (None, None) => true,
        (Some(s), Some(t)) => beside(s, t),
        _ => false,
    }
}

/// One size's screen targets — see the [module](self).
pub(crate) struct ViewTargets {
    pub(crate) size: winit::dpi::PhysicalSize<u32>,
    depth: wgpu::TextureView,
    post: crate::post_process::PostTargets,
    deferred: Option<crate::deferred::DeferredTargets>,
    decal: Option<crate::decal::DecalTargets>,
    ssao: Option<crate::ssao::SsaoTargets>,
    ssr: Option<crate::ssr::SsrTargets>,
    volumetric: Option<crate::volumetric::VolumetricTargets>,
    fxaa: Option<crate::fxaa::FxaaTargets>,
    smaa: Option<crate::smaa::SmaaTargets>,
    cas: Option<crate::cas::CasTargets>,
    fluid: Option<crate::gpu_fluid::system::FluidTargets>,
    /// The frame under the glass, made at this size the first time a view of it has glass in sight.
    behind: Option<crate::transmission::BehindSurface>,
    /// The frame it was last drawn in, for [`Renderer::retain_view_targets`](crate::Renderer::retain_view_targets).
    pub(crate) last_used: u64,
}

impl ViewTargets {
    /// The renderer's own, taken: the other half of a swap, or the set to put back after making one.
    pub(crate) fn take(renderer: &crate::Renderer) -> Self {
        Self {
            size: renderer.size,
            depth: renderer.depth_texture_view.clone(),
            post: renderer.post.take_screen_targets(),
            deferred: renderer.deferred.as_ref().map(|s| s.take_screen_targets()),
            decal: renderer.decal.as_ref().map(|s| s.take_screen_targets()),
            ssao: renderer.ssao.as_ref().map(|s| s.take_screen_targets()),
            ssr: renderer.ssr.as_ref().map(|s| s.take_screen_targets()),
            volumetric: renderer.volumetric.as_ref().map(|s| s.take_screen_targets()),
            fxaa: renderer.fxaa.as_ref().map(|s| s.take_screen_targets()),
            smaa: renderer.smaa.as_ref().map(|s| s.take_screen_targets()),
            cas: renderer.cas.as_ref().map(|s| s.take_screen_targets()),
            fluid: renderer.gpu_fluid.as_ref().map(|s| s.take_screen_targets()),
            behind: None,
            last_used: 0,
        }
    }

    /// Whether this set fits the renderer's states as they are now: one set for each state there is,
    /// each taken from that build of it.
    pub(crate) fn fits(&self, renderer: &crate::Renderer) -> bool {
        renderer.post.built_beside(&self.post)
            && fits(renderer.deferred.as_ref(), self.deferred.as_ref(), |s, t| s.built_beside(t))
            && fits(renderer.decal.as_ref(), self.decal.as_ref(), |s, t| s.built_beside(t))
            && fits(renderer.ssao.as_ref(), self.ssao.as_ref(), |s, t| s.built_beside(t))
            && fits(renderer.ssr.as_ref(), self.ssr.as_ref(), |s, t| s.built_beside(t))
            && fits(renderer.volumetric.as_ref(), self.volumetric.as_ref(), |s, t| s.built_beside(t))
            && fits(renderer.fxaa.as_ref(), self.fxaa.as_ref(), |s, t| s.built_beside(t))
            && fits(renderer.smaa.as_ref(), self.smaa.as_ref(), |s, t| s.built_beside(t))
            && fits(renderer.cas.as_ref(), self.cas.as_ref(), |s, t| s.built_beside(t))
            && fits(renderer.gpu_fluid.as_ref(), self.fluid.as_ref(), |s, t| s.built_beside(t))
    }

    /// Exchanges this set with the renderer's targets, the frame under the glass and the size
    /// included. Call only on a set that [`fits`](Self::fits).
    pub(crate) fn swap(&mut self, renderer: &mut crate::Renderer) {
        std::mem::swap(&mut renderer.size, &mut self.size);
        std::mem::swap(&mut renderer.depth_texture_view, &mut self.depth);
        renderer.post.swap_screen_targets(&mut self.post);
        swap_optional!(renderer.deferred, self.deferred);
        swap_optional!(renderer.decal, self.decal);
        swap_optional!(renderer.ssao, self.ssao);
        swap_optional!(renderer.ssr, self.ssr);
        swap_optional!(renderer.volumetric, self.volumetric);
        swap_optional!(renderer.fxaa, self.fxaa);
        swap_optional!(renderer.smaa, self.smaa);
        swap_optional!(renderer.cas, self.cas);
        swap_optional!(renderer.gpu_fluid, self.fluid);
        // The frame under the glass is bound in the scene's group 0, so the group follows it — only
        // when either side has one: with neither, it holds the fallback already.
        if renderer.behind.is_some() || self.behind.is_some() {
            std::mem::swap(&mut renderer.behind, &mut self.behind);
            let behind = renderer.behind.as_ref().map(|b| (b.view(), b.depth_view()));
            renderer.scene.set_behind_view(&renderer.device, behind);
        }
    }
}
