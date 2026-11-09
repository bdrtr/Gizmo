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
/// The engine's own stage is the facade's `RenderWorld`, which copies the world the draw reads.
pub trait RenderStage: Send + 'static {
    /// At the sync point: everything the frame does with the game world and the renderer before
    /// the draw — including keeping what the draw will need.
    fn prepare(&mut self, game: &mut World, renderer: &mut Renderer);

    /// The draw, from what [`prepare`](Self::prepare) kept, into `view`.
    fn draw(&mut self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, renderer: &mut Renderer);
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
    /// the draw's published cost. The acquire needs a real swapchain, so this pins the order.
    #[test]
    fn the_stage_is_prepared_before_the_backbuffer_is_acquired() {
        let code: String = include_str!("event.rs")
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let prepare = code.find("stage.prepare(&mut self.world, &mut renderer)").expect("the loop prepares the stage");
        let acquire = code.find("self.acquire_backbuffer(&mut renderer)").expect("the loop acquires a backbuffer");
        let draw = code.find("stage.draw(&mut encoder, &view, &mut renderer)").expect("the loop draws the stage");
        assert!(prepare < acquire, "the stage is prepared only on a frame that got a backbuffer");
        assert!(acquire < draw);
    }
}
