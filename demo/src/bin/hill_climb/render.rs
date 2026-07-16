use gizmo::prelude::*;
use super::DemoState;

pub(super) fn render(
    world: &mut World,
    state: &DemoState,
    encoder: &mut gizmo::wgpu::CommandEncoder,
    view: &gizmo::wgpu::TextureView,
    renderer: &mut Renderer,
    _light_time: f32,
) {
    renderer.update_post_process(&renderer.queue, state.post_process);

    let mut pending = state.pending_particles.borrow_mut();
    if !pending.is_empty() {
        if let Some(gpu_particles) = &renderer.gpu_particles {
            gpu_particles.spawn_particles(&renderer.queue, &pending);
        }
        pending.clear();
    }

    gizmo::systems::default_render_pass(world, encoder, view, renderer);
}
