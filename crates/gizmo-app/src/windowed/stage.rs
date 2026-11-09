//! What a frame draws, and in which half of the frame each part of it runs.

use gizmo_core::world::World;
use gizmo_renderer::renderer::Renderer;
use gizmo_renderer::RenderContext;

/// A frame's render work, split where a render thread splits it.
///
/// A frame has two halves. **The sync point** comes after the game's update: the game world is
/// between frames and the renderer is in hand. [`prepare`](Self::prepare) runs there, and it is the
/// only part that sees the game world — so it does everything that writes the world or reads it:
/// advancing render-side clocks, building meshes, publishing what the last draw cost, and copying
/// what the draw needs out of the world into the stage itself. **The draw** comes after it.
/// [`draw`](Self::draw) records the frame from what `prepare` kept, and is handed no world at all.
///
/// That shape is what lets the draw run somewhere else. While it records, nothing it reads can
/// change under it — the game world is not among its inputs — so the next frame's update can run
/// at the same time. A hook that writes the game world from inside the draw
/// ([`App::set_render`](crate::App::set_render), [`App::set_simple_render`](crate::App::set_simple_render))
/// cannot be split this way, and runs whole at the sync point.
///
/// `prepare` runs on every frame the loop runs, including one whose swapchain image could not be
/// acquired and so is never drawn: what it does to the game world is part of the game's frame, not
/// of the picture.
///
/// With [`App::with_render_thread`](crate::App::with_render_thread) the draw runs on a render
/// thread, overlapping the next frame's update; otherwise both halves run on the loop's thread, one
/// after the other. A stage is the same either way, which is why it must be [`ConditionalSend`].
///
/// The engine's own stage is the facade's `RenderWorld`, which copies the world the draw reads.
pub trait RenderStage: ConditionalSend + 'static {
    /// At the sync point: everything the frame does with the game world and the renderer before
    /// the draw — including keeping what the draw will need.
    fn prepare(&mut self, game: &mut World, renderer: &mut Renderer);

    /// The draw, from what [`prepare`](Self::prepare) kept, into `view`.
    fn draw(&mut self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, renderer: &mut Renderer);
}

/// `Send` where a render thread can exist, and nothing at all on `wasm32`, where the app has one
/// thread and GPU objects cannot cross threads anyway. Implemented for every type that qualifies.
#[cfg(not(target_arch = "wasm32"))]
pub trait ConditionalSend: Send {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send> ConditionalSend for T {}
/// `Send` where a render thread can exist, and nothing at all on `wasm32`, where the app has one
/// thread and GPU objects cannot cross threads anyway. Implemented for every type that qualifies.
#[cfg(target_arch = "wasm32")]
pub trait ConditionalSend {}
#[cfg(target_arch = "wasm32")]
impl<T> ConditionalSend for T {}

/// The draw's side of the app: what a frame's draw owns, and what a render thread holds while it
/// draws. The game's side reaches none of it but through the sync point.
pub(crate) struct DrawSide {
    pub(crate) renderer: Renderer,
    /// The overlay UI's painter: it holds GPU textures and paints in the draw.
    #[cfg(feature = "egui")]
    pub(crate) painter: crate::egui_ctx::EguiPainter,
    /// The app's [`RenderStage`], when its render hook is one.
    pub(crate) stage: Option<Box<dyn RenderStage>>,
}

/// The low-level render hook's signature — see [`App::set_render`](crate::App::set_render).
pub(crate) type RawRenderFn<State> =
    dyn FnMut(&mut World, &State, &mut wgpu::CommandEncoder, &wgpu::TextureView, &mut Renderer, f32);

/// The render hook's signature through a [`RenderContext`] — see
/// [`App::set_simple_render`](crate::App::set_simple_render).
pub(crate) type SimpleRenderFn<State> = dyn for<'a> FnMut(&mut World, &State, &mut RenderContext<'a>);

/// The one way an app draws its frame. One field rather than three, so that installing a hook
/// replaces the last one — with three, a second hook was silently outranked by the first.
pub(crate) enum RenderHook<State: 'static> {
    /// Nothing draws the scene.
    None,
    /// [`App::set_render`](crate::App::set_render).
    Raw(Box<RawRenderFn<State>>),
    /// [`App::set_simple_render`](crate::App::set_simple_render).
    Simple(Box<SimpleRenderFn<State>>),
    /// [`App::set_render_stage`](crate::App::set_render_stage).
    Stage(Box<dyn RenderStage>),
}

impl<State: 'static> RenderHook<State> {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Raw(_) => "set_render",
            Self::Simple(_) => "set_simple_render",
            Self::Stage(_) => "set_render_stage",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::App;

    struct Nothing;

    impl RenderStage for Nothing {
        fn prepare(&mut self, _: &mut World, _: &mut Renderer) {}
        fn draw(&mut self, _: &mut wgpu::CommandEncoder, _: &wgpu::TextureView, _: &mut Renderer) {}
    }

    /// The last render hook installed is the one that draws. With a field per kind, a raw hook
    /// silently outranked a stage or a simple hook installed after it.
    #[test]
    fn installing_a_render_hook_replaces_the_last_one() {
        let app = App::<()>::new("t", 8, 8).set_render(|_, _, _, _, _, _| {}).set_render_stage(Nothing);
        assert_eq!(app.render.name(), "set_render_stage");
        let app = app.set_simple_render(|_, _, _| {});
        assert_eq!(app.render.name(), "set_simple_render");
        let app = app.set_render(|_, _, _, _, _, _| {});
        assert_eq!(app.render.name(), "set_render");
    }

    /// The stage is prepared at the sync point, before the swapchain is asked for an image: a frame
    /// whose image cannot be acquired (occluded, resized, timed out — and the first frame of a
    /// freshly mapped window) is still a frame of the game, and skipping `prepare` with the
    /// picture dropped what it does to the game world — the render clocks, the particle spawns,
    /// the draw's published cost. The acquire needs a real swapchain, so this pins the shape: the
    /// image is acquired only inside the draw (`draw_and_present`), and the loop prepares the stage
    /// before it hands any frame to a draw.
    #[test]
    fn the_stage_is_prepared_before_the_backbuffer_is_acquired() {
        let code: String = include_str!("event.rs")
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let draw_fn = code.find("fn draw_and_present(").expect("the draw is one function");
        let draw_body = &code[draw_fn..code[draw_fn..].find("\n}").map(|e| draw_fn + e).expect("its body ends")];
        assert_eq!(code.matches("acquire_backbuffer(renderer)").count(), 1, "the image is acquired somewhere else too");
        assert!(draw_body.contains("acquire_backbuffer(renderer)"), "the image is acquired outside the draw");

        let prepare = code.find("stage.prepare(&mut self.world, &mut side.renderer)").expect("the loop prepares the stage");
        let first_draw = [code.find("gpu.draw("), code.find("draw_and_present(\n")]
            .into_iter()
            .flatten()
            .min()
            .expect("the loop draws");
        assert!(prepare < first_draw, "a frame is handed to the draw before the stage is prepared");
        assert!(code.contains("stage.draw(encoder, view, renderer)"), "the stage's draw is not what the draw runs");
    }

    /// A frame that is not drawn still ends: the rumble its systems asked for goes to the pads, and
    /// `Input::begin_frame` clears its key edges and mouse delta. The skipped frame used to return
    /// from the loop straight out of the acquire, before either — and the next update saw the same
    /// key presses and the same mouse motion a second time. Nothing between the sync point and the
    /// end of the frame leaves the loop now; the skip is the draw's business (`draw_and_present`).
    #[test]
    fn a_frame_that_is_not_drawn_still_ends() {
        let code: String = include_str!("event.rs")
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let sync = code.find("let side = gpu.sync();").expect("the frame has a sync point");
        let end = code.find("self.input.begin_frame();").expect("the frame clears its input");
        assert!(sync < end);
        assert!(!code[sync..end].contains("return"), "the frame can leave the loop before its input is cleared");
    }
}
