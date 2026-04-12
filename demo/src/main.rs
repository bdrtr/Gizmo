use gizmo::math::Quat;
use gizmo::prelude::*;

#[derive(Clone, Copy)]
pub struct OriginalRotation(pub Quat);

#[derive(Clone, Copy)]
pub struct Player;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct EntityName(pub String);

pub mod state;
pub use state::*;
pub mod scene_setup;
pub use scene_setup::*;
pub mod demo_car;
pub use demo_car::*;
pub mod ui;
pub use ui::*;
pub mod script_sys;
pub use script_sys::*;
pub mod render_pipeline;
pub use render_pipeline::*;
pub mod systems;
pub use systems::*;
pub mod gizmo_input;
pub use gizmo_input::*;
pub mod camera;
pub use camera::*;
pub mod hot_reload_sys;
pub use hot_reload_sys::*;
pub mod basic_scene;
pub mod components;
pub mod network;
pub mod race;
pub mod update;

fn main() {
    // Demo -> Sadece Voxygen (Client) Renderer olarak görev yapar!
    let mut app = App::<crate::state::GameState>::new("Gizmo Engine — Rust 3D Motor", 1280, 720)
        .add_system(crate::systems::vehicle_controller_system)
        .add_system(crate::systems::character_update_system)
        .add_system(crate::systems::free_camera_system)
        .add_system(crate::systems::chase_camera_system)
        .add_system(crate::systems::ccd_test_system)
        .add_system(crate::network::client_network_system)
        .add_event::<gizmo::physics::CollisionEvent>()
        .add_event::<crate::state::ShaderReloadEvent>()
        .add_event::<crate::state::SelectionEvent>()
        .add_event::<crate::state::TextureLoadEvent>()
        .add_event::<crate::state::SpawnDominoEvent>()
        .add_event::<crate::state::ReleaseDominoEvent>()
        .add_event::<crate::state::AssetSpawnEvent>();

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
    app = app.set_render(
        |world: &mut World,
         state: &crate::state::GameState,
         encoder: &mut wgpu::CommandEncoder,
         view: &wgpu::TextureView,
         renderer: &mut gizmo::renderer::Renderer,
         light_time: f32| {
            // Yeni Sahne Yükleme İsteği
            if let Some(req) = world.remove_resource::<crate::state::SceneLoadRequest>() {
                println!("[Demo Engine] Sahne değiştiriliyor: {}", req.0);
                if let Some(mut asset_manager) =
                    world.remove_resource::<gizmo::renderer::asset::AssetManager>()
                {
                    // Sahne yüklemesi sırasında mevcut nesneler, fizik rigid body'leri vs bozulabilir!
                    // Gerçek bir sahnede önce entity'leri temizlemek gerekebilir (demo world.clear yok)
                    // Şimdilik üst üste asset yükleyecektir.
                    let dummy_rgba = [255, 255, 255, 255];
                    let dummy_bg = renderer.create_texture(&dummy_rgba, 1, 1);

                    gizmo::scene::SceneData::load_into(
                        &req.0,
                        world,
                        &renderer.device,
                        &renderer.queue,
                        &renderer.scene.texture_bind_group_layout,
                        &mut asset_manager,
                        std::sync::Arc::new(dummy_bg),
                    );
                    world.insert_resource(asset_manager);
                }
            }

            // Post-process ayarlarını uygula
            {
                if let Some(pp) =
                    world.get_resource::<gizmo::renderer::renderer::PostProcessUniforms>()
                {
                    renderer.update_post_process(&renderer.queue, *pp);
                }
            }

            // Shader reload isteği
            if let Some(mut events) = world
                .get_resource_mut::<gizmo::core::event::Events<crate::state::ShaderReloadEvent>>()
            {
                if !events.is_empty() {
                    renderer.rebuild_shaders();
                    events.clear();
                }
            }

            // Texture reload isteği
            let mut texture_reloads = Vec::new();
            if let Some(mut events) = world
                .get_resource_mut::<gizmo::core::event::Events<crate::state::TextureLoadEvent>>()
            {
                texture_reloads.extend(events.drain());
            }

            if !texture_reloads.is_empty() {
                if let Some(mut asset_mgr) = world.get_resource_mut::<gizmo::renderer::asset::AssetManager>() {
                    for load_ev in texture_reloads {
                        let path = &load_ev.path;
                        println!("[Demo Engine] Texture GPU'ya yeniden yukleniyor: {}", path);
                        
                        if let Ok(new_bg) = asset_mgr.reload_material_texture(
                            &renderer.device,
                            &renderer.queue,
                            &renderer.scene.texture_bind_group_layout,
                            path,
                        ) {
                            if let Some(mut materials) = world.borrow_mut::<gizmo::renderer::components::Material>() {
                                if let Some(mat) = materials.get_mut(load_ev.entity_id) {
                                    mat.bind_group = new_bg;
                                }
                            }
                        }
                    }
                }
            }

            render_pipeline::execute_render_pipeline(
                world, state, encoder, view, renderer, light_time,
            );
        },
    );

    app.run();
}
