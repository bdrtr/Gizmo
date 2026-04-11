use gizmo::prelude::*;
use gizmo::math::{Quat};

#[derive(Clone, Copy)]
pub struct OriginalRotation(pub Quat);

#[derive(Clone, Copy)]
pub struct Player;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct EntityName(pub String);

pub mod state;       pub use state::*;
pub mod scene_setup; pub use scene_setup::*;
pub mod demo_car;    pub use demo_car::*;
pub mod ui;          pub use ui::*;
pub mod script_sys;  pub use script_sys::*;
pub mod render_pipeline; pub use render_pipeline::*;
pub mod systems;     pub use systems::*;
pub mod gizmo_input; pub use gizmo_input::*;
pub mod camera;      pub use camera::*;
pub mod hot_reload_sys; pub use hot_reload_sys::*;
pub mod components;
pub mod race;
pub mod basic_scene;
pub mod network;
pub mod update;

fn main() {
    // Demo -> Sadece Voxygen (Client) Renderer olarak görev yapar!
    let mut app = App::<crate::state::GameState>::new("Gizmo Engine — Rust 3D Motor", 1280, 720)
        .add_system(crate::systems::vehicle_controller_system)
        .add_system(crate::systems::character_update_system)
        .add_system(crate::systems::free_camera_system)
        .add_system(crate::systems::chase_camera_system)
        .add_system(crate::systems::ccd_test_system)
        .add_system(crate::network::client_network_system);

    app = app.set_setup(|world, renderer| {
        use gizmo::core::input::ActionMap;
        use gizmo::winit::keyboard::KeyCode;

        let mut action_map = ActionMap::new();
        action_map.bind_action("Accelerate", KeyCode::ArrowUp as u32);
        action_map.bind_action("Reverse", KeyCode::ArrowDown as u32);
        action_map.bind_action("SteerLeft", KeyCode::ArrowLeft as u32);
        action_map.bind_action("SteerRight", KeyCode::ArrowRight as u32);
        action_map.bind_action("Brake", KeyCode::Space as u32);
        world.insert_resource(action_map);

        let mut state = scene_setup::setup_empty_scene(world, renderer);
        
        // Yarış yerine Basic Scene modunu başlat
        let basic_state = crate::basic_scene::setup_basic_scene(world, renderer);
        state.player_id = basic_state.player_entity; 
        state.camera_follow_target = Some(basic_state.player_entity);
        state.basic_scene = Some(basic_state);
        state.free_cam = false;
        
        state
    });

    // ── UPDATE ─────────────────────────────────────────────────────────────
    app = app.set_update(|world, state, dt, input| {
        update::update_demo(world, state, dt, input);
    });

    // ── UI ─────────────────────────────────────────────────────────────────
    app = app.set_ui(|world, state, ctx| {
        state.egui_wants_pointer = ctx.is_pointer_over_area();
        render_ui(ctx, state, world);
    });

    // ── RENDER ─────────────────────────────────────────────────────────────
    app = app.set_render(|world: &mut World, state: &crate::state::GameState, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, renderer: &mut gizmo::renderer::Renderer, light_time: f32| {
        // Post-process ayarlarını uygula
        {
            if let Some(pp) = world.get_resource::<gizmo::renderer::renderer::PostProcessUniforms>() {
                renderer.update_post_process(&renderer.queue, *pp);
            }
        }
        
        // Shader reload isteği
        if let Some(mut events) = world.get_resource_mut::<gizmo::core::event::Events<crate::state::ShaderReloadEvent>>() {
            if !events.is_empty() {
                renderer.rebuild_shaders();
                events.clear();
            }
        }

        render_pipeline::execute_render_pipeline(world, state, encoder, view, renderer, light_time);
    });

    app.run();
}
