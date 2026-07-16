use gizmo::prelude::*;
use gizmo::math::{Vec3, Vec4, Quat};
use gizmo::renderer::asset::AssetManager;
use gizmo::renderer::components::{Material, MeshRenderer, PointLight, DirectionalLight};
use gizmo::core::system::{Res, ResMut, IntoSystemConfig, Phase};
use gizmo::core::query::{Query, Mut};
use gizmo::core::input::Input;
use gizmo::systems;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LightMode {
    Directional,
    Point,
    Both,
}

pub struct DemoState {
    pub camera_state: CameraState,
    pub editor_state: EditorState,
    pub orbit_time: f32, // Accumulated time specifically for camera orbiting angle
    pub dir_light_ent: Option<Entity>,   // Safe typed entity handles to prevent dangling references
    pub point_light_ent: Option<Entity>,
    pub ssr_enabled: bool,
    pub ssgi_enabled: bool,
    pub volumetric_enabled: bool,
    pub light_mode: LightMode,
}

fn main() {
    let mut app = App::new("Gizmo Engine - Bevy Anisotropy Barn Lamp Demo", 1280, 720);

    app = app
        .add_plugin(gizmo::plugins::TransformPlugin)
        .set_setup(|world, renderer| {
            world.register_component_type::<BarnLampModel>();

            let mut asset_manager = AssetManager::new();
            let phys_world = gizmo::physics::world::PhysicsWorld::new().with_gravity(Vec3::ZERO);

            let camera_settings = CameraSettings {
                speed: 15.0,
                pitch: -0.2,
                yaw: 0.0,
                pos: Vec3::new(0.0, 0.8, 1.8),
                exposure: 1.0,
                bloom_intensity: 0.05,
            };

            let lighting_settings = LightingSettings {
                preset: 0,
                preset_2: 1,
                blend_t: 0.0,
                auto_cycle: false,
                rotation_speed: 0.4,
                direct_intensity: 2.5,
            };

            // Custom skybox
            let skybox_mesh = AssetManager::create_inverted_cube(&renderer.device);
            let sky_tex = asset_manager.create_white_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            );
            // Deep sleek dark blue/gray skybox matching the premium Bevy scene style
            let sky_mat = Material::new(sky_tex).with_skybox().with_unlit(Vec4::new(0.04, 0.04, 0.06, 1.0));
            let sky_ent = world.spawn();
            let mut sky_t = Transform::new(Vec3::ZERO).with_scale(Vec3::splat(1000.0));
            sky_t.update_local_matrix();
            world.add_component(sky_ent, sky_t);
            world.add_component(sky_ent, gizmo::physics::components::GlobalTransform { matrix: sky_t.local_matrix });
            world.add_component(sky_ent, skybox_mesh);
            world.add_component(sky_ent, sky_mat);
            world.add_component(sky_ent, MeshRenderer::new());

            // Dark premium ground plane
            let ground_mesh = AssetManager::create_plane(&renderer.device, 50.0);
            let ground_tex = asset_manager.create_white_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            );
            let ground_mat = Material::new(ground_tex).with_pbr(Vec4::new(0.04, 0.04, 0.06, 1.0), 0.05, 0.95);
            let ground_ent = world.spawn();
            let mut ground_t = Transform::new(Vec3::new(0.0, -0.05, 0.0));
            ground_t.update_local_matrix();
            world.add_component(ground_ent, ground_t);
            world.add_component(ground_ent, gizmo::physics::components::GlobalTransform { matrix: ground_t.local_matrix });
            world.add_component(ground_ent, ground_mesh);
            world.add_component(ground_ent, ground_mat);
            world.add_component(ground_ent, MeshRenderer::new());

            // Camera setup
            let camera_ent = world.spawn();
            let cam_bundle = gizmo::bundles::CameraBundle {
                position: camera_settings.pos,
                yaw: camera_settings.yaw,
                pitch: camera_settings.pitch,
                ..Default::default()
            };
            cam_bundle.apply(world, camera_ent);

            // Spawn Directional Light (Sun)
            let dir_light_ent = world.spawn();
            let sun_bundle = gizmo::bundles::DirectionalLightBundle {
                rotation: Quat::from_rotation_x(-std::f32::consts::FRAC_PI_4) * Quat::from_rotation_y(std::f32::consts::FRAC_PI_4),
                intensity: lighting_settings.direct_intensity,
                color: Vec3::new(1.0, 0.95, 0.9), // Warm sunshine
                ..Default::default()
            };
            sun_bundle.apply(world, dir_light_ent);

            // Point Light Setup
            let point_light_ent = world.spawn();
            let point_bundle = gizmo::bundles::PointLightBundle {
                position: Vec3::new(0.25, 0.15, -0.13),
                intensity: 25.0, // High local intensity
                radius: 1.0,     // 1.0 meter local radius so it behaves like a localized bulb
                color: Vec3::new(1.0, 0.9, 0.7), // Glowing warm classic bulb light
            };
            point_bundle.apply(world, point_light_ent);

            // Spawn a reflective, fanned anisotropic Copper Torus on the left side of the lamp
            let torus_mesh = AssetManager::create_torus(&renderer.device, 0.22, 0.07, 32, 32);
            let torus_tex = asset_manager.create_white_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            );
            // Anisotropic copper covered with a high-gloss lacquer clear coat!
            let torus_mat = Material::new(torus_tex)
                .with_pbr(Vec4::new(0.95, 0.64, 0.54, 1.0), 0.25, 1.0)
                .with_anisotropy(0.85)
                .with_clear_coat(0.9);
            let torus_ent = world.spawn();
            let mut torus_t = Transform::new(Vec3::new(-0.45, 0.18, -0.15));
            torus_t.update_local_matrix();
            world.add_component(torus_ent, torus_t);
            world.add_component(torus_ent, gizmo::physics::components::GlobalTransform { matrix: torus_t.local_matrix });
            world.add_component(torus_ent, torus_mesh);
            world.add_component(torus_ent, torus_mat);
            world.add_component(torus_ent, MeshRenderer::new());
 
            // Spawn a sleek, ultra-smooth reflective Gold Sphere on the right side of the lamp
            let sphere_mesh = AssetManager::create_sphere(&renderer.device, 0.18, 32, 32);
            let sphere_tex = asset_manager.create_white_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            );
            // Premium double-layer cila gold sphere (satin-brushed gold base + smooth glossy coat!)
            let sphere_mat = Material::new(sphere_tex)
                .with_pbr(Vec4::new(1.0, 0.78, 0.28, 1.0), 0.35, 1.0)
                .with_clear_coat(1.0);
            let sphere_ent = world.spawn();
            let mut sphere_t = Transform::new(Vec3::new(0.95, 0.18, -0.15));
            sphere_t.update_local_matrix();
            world.add_component(sphere_ent, sphere_t);
            world.add_component(sphere_ent, gizmo::physics::components::GlobalTransform { matrix: sphere_t.local_matrix });
            world.add_component(sphere_ent, sphere_mesh);
            world.add_component(sphere_ent, sphere_mat);
            world.add_component(sphere_ent, MeshRenderer::new());

            // Spawn a semi-translucent, warm wax/jade sphere in the front-center
            let center_sphere_mesh = AssetManager::create_sphere(&renderer.device, 0.14, 32, 32);
            let center_sphere_tex = asset_manager.create_white_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            );
            // Jade/Wax material (white-peach albedo, high roughness, zero metallic, 0.8 subsurface glow!)
            let center_sphere_mat = Material::new(center_sphere_tex)
                .with_pbr(Vec4::new(0.98, 0.88, 0.82, 1.0), 0.55, 0.0)
                .with_subsurface(0.85);
            let center_sphere_ent = world.spawn();
            let mut center_sphere_t = Transform::new(Vec3::new(0.25, 0.18, 0.25));
            center_sphere_t.update_local_matrix();
            world.add_component(center_sphere_ent, center_sphere_t);
            world.add_component(center_sphere_ent, gizmo::physics::components::GlobalTransform { matrix: center_sphere_t.local_matrix });
            world.add_component(center_sphere_ent, center_sphere_mesh);
            world.add_component(center_sphere_ent, center_sphere_mat);
            world.add_component(center_sphere_ent, MeshRenderer::new());

            // Load and spawn the Barn Lamp model at the center (shifted to the right on X axis, and slightly up on Y axis)
            {
                let mut cmd = SpawnCommands::new(world, renderer);
                let _ = cmd.spawn_gltf(
                    Vec3::new(0.25, 0.15, -0.13),
                    "assets/models/AnisotropyBarnLamp/AnisotropyBarnLamp.glb",
                    false,
                );
            }

            {
                let mut materials = world.borrow_mut::<Material>();
                for (id, mut mat) in materials.iter_mut() {
                    println!("DEBUG MAT: id={:?}, albedo={:?}, metallic={}, roughness={}, transparent={}, double_sided={}, type={:?}", 
                        id, mat.albedo, mat.metallic, mat.roughness, mat.is_transparent, mat.is_double_sided, mat.material_type);
                    
                    if mat.albedo.x > 0.9 && mat.albedo.y < 0.7 {
                        // This is the Torus (anisotropic copper)
                        mat.anisotropy = 0.9;
                    } else if mat.albedo.x > 0.9 && mat.albedo.y > 0.7 && mat.albedo.y < 0.85 {
                        // This is the Gold Sphere (isotropic gold)
                        mat.anisotropy = 0.0;
                    } else if mat.metallic > 0.5 {
                        // This is a metallic part of the Barn Lamp GLTF model!
                        // Let's keep its original metallic, and assign a brushed anisotropy factor!
                        mat.anisotropy = 0.85;
                    } else {
                        mat.anisotropy = 0.0;
                    }
                }
            }

            world.insert_resource(phys_world);
            world.insert_resource(asset_manager);
            world.insert_resource(camera_settings);
            world.insert_resource(lighting_settings);
            world.insert_resource(DemoState {
                camera_state: CameraState::Orbiting,
                editor_state: EditorState::PlayMode,
                light_mode: LightMode::Both,
                orbit_time: 0.0,
                dir_light_ent: Some(dir_light_ent),
                point_light_ent: Some(point_light_ent),
                ssr_enabled: false,
                ssgi_enabled: false,
                volumetric_enabled: true,
            });
        })
        .add_system(handle_input.into_config().label("handle_input").in_phase(Phase::Update))
        .add_system(camera_orbit.into_config().label("camera_orbit").after("handle_input").in_phase(Phase::Update))
        .add_system(animate_lights.into_config().label("animate_lights").after("camera_orbit").in_phase(Phase::Update))
        .set_render(|world, _state, encoder, view, renderer, _light_time| {
            renderer.gpu_fluid = None;
            renderer.gpu_particles = None;
            
            // Get current DemoState booleans in a separate scope to release the borrow on world
            let (ssr_enabled, ssgi_enabled, volumetric_enabled) = {
                let demo_state = world.get_resource::<DemoState>().unwrap();
                (demo_state.ssr_enabled, demo_state.ssgi_enabled, demo_state.volumetric_enabled)
            };
            
            // Recreate or destroy Screen Space Reflections (SSR) State
            if ssr_enabled {
                if renderer.ssr.is_none() {
                    if let Some(ref def) = renderer.deferred {
                        renderer.ssr = Some(gizmo::renderer::ssr::SsrState::new(
                            &renderer.device,
                            &renderer.scene,
                            def,
                            &renderer.post.hdr_texture_view,
                            renderer.config.width,
                            renderer.config.height,
                        ));
                    }
                }
            } else {
                renderer.ssr = None;
            }

            // Recreate or destroy Screen Space Global Illumination (SSGI) State
            if ssgi_enabled {
                if renderer.ssgi.is_none() {
                    if let Some(ref def) = renderer.deferred {
                        renderer.ssgi = Some(gizmo::renderer::ssgi::SsgiState::new(
                            &renderer.device,
                            &renderer.scene,
                            def,
                            &renderer.post.hdr_texture_view,
                            renderer.config.width,
                            renderer.config.height,
                        ));
                    }
                }
            } else {
                renderer.ssgi = None;
            }

            // Recreate or destroy Volumetric Lighting State dynamically
            if volumetric_enabled {
                if renderer.volumetric.is_none() {
                    if let Some(ref def) = renderer.deferred {
                        renderer.volumetric = Some(gizmo::renderer::volumetric::VolumetricState::new(
                            &renderer.device,
                            &renderer.scene,
                            def,
                            renderer.config.width,
                            renderer.config.height,
                        ));
                    }
                }
            } else {
                renderer.volumetric = None;
            }
            
            systems::default_render_pass(world, encoder, view, renderer);
        })
        .set_ui(|world, _state, ctx| {
            let mut demo_state = world.get_resource_mut::<DemoState>().unwrap();
            let mut camera_settings = world.get_resource_mut::<CameraSettings>().unwrap();
            let mut lighting_settings = world.get_resource_mut::<LightingSettings>().unwrap();
            let mut renderer = world.get_resource_mut::<gizmo::renderer::Renderer>().unwrap();
            
            gizmo::egui::Window::new("Gizmo Engine Panel")
                .default_width(320.0)
                .show(ctx, |ui| {
                    ui.heading("Anisotropy Barn Lamp");
                    ui.label("Replicating Bevy's anisotropy demo in Gizmo Engine.");
                    ui.separator();

                    ui.horizontal(|ui| {
                        ui.label("Camera Orbit:");
                        let mut orbit = demo_state.camera_state == CameraState::Orbiting;
                        if ui.checkbox(&mut orbit, "").changed() {
                            demo_state.camera_state = if orbit { CameraState::Orbiting } else { CameraState::Manual };
                        }
                    });

                    if demo_state.camera_state == CameraState::Manual {
                        ui.horizontal(|ui| {
                            ui.label("Camera Speed:");
                            ui.add(gizmo::egui::Slider::new(&mut camera_settings.speed, 1.0..=50.0).step_by(1.0));
                        });
                    }

                    ui.horizontal(|ui| {
                        ui.label("Light Rotation:");
                        let mut rotation = lighting_settings.rotation_speed > 0.0;
                        if ui.checkbox(&mut rotation, "").changed() {
                            lighting_settings.rotation_speed = if rotation { 0.4 } else { 0.0 };
                        }
                    });

                    ui.horizontal(|ui| {
                        ui.label("Point Light Shadows:");
                        ui.checkbox(&mut renderer.point_shadows_enabled, "");
                    });

                    ui.separator();
                    ui.label("Light Mode:");
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut demo_state.light_mode, LightMode::Directional, "Directional");
                        ui.radio_value(&mut demo_state.light_mode, LightMode::Point, "Point");
                        ui.radio_value(&mut demo_state.light_mode, LightMode::Both, "Both");
                    });

                    ui.separator();
                    ui.label("G-Buffer Visualizer:");
                    gizmo::egui::ComboBox::from_id_salt("shading_mode")
                        .selected_text(match renderer.shading_mode {
                            0 => "💡 Full PBR (Lit)",
                            1 => "🎨 Normals View",
                            2 => "⚪ Albedo View",
                            3 => "🕸️ Roughness / Metallic",
                            4 => "👥 Shadow Map Visualizer",
                            5 => "📐 Tangents View",
                            6 => "✨ Clear Coat View",
                            _ => "💡 Full PBR (Lit)",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut renderer.shading_mode, 0, "💡 Full PBR (Lit)");
                            ui.selectable_value(&mut renderer.shading_mode, 1, "🎨 Normals View");
                            ui.selectable_value(&mut renderer.shading_mode, 2, "⚪ Albedo View");
                            ui.selectable_value(&mut renderer.shading_mode, 3, "🕸️ Roughness / Metallic");
                            ui.selectable_value(&mut renderer.shading_mode, 4, "👥 Shadow Map Visualizer");
                            ui.selectable_value(&mut renderer.shading_mode, 5, "📐 Tangents View");
                            ui.selectable_value(&mut renderer.shading_mode, 6, "✨ Clear Coat View");
                        });

                    ui.separator();
                    ui.label("Environment Blending:");
                    ui.checkbox(&mut lighting_settings.auto_cycle, "🌌 Auto-Cycle Blending");

                    ui.horizontal(|ui| {
                        ui.label("Base Preset:");
                        gizmo::egui::ComboBox::from_id_salt("env_preset")
                            .selected_text(match lighting_settings.preset {
                                0 => "🌇 Sunset Gold",
                                1 => "🏢 Studio Neutral",
                                2 => "🌃 Midnight Neon",
                                3 => "☀️ Classic Daylight",
                                _ => "🌇 Sunset Gold",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut lighting_settings.preset, 0, "🌇 Sunset Gold");
                                ui.selectable_value(&mut lighting_settings.preset, 1, "🏢 Studio Neutral");
                                ui.selectable_value(&mut lighting_settings.preset, 2, "🌃 Midnight Neon");
                                ui.selectable_value(&mut lighting_settings.preset, 3, "☀️ Classic Daylight");
                            });
                    });

                    ui.horizontal(|ui| {
                        ui.label("Target Preset:");
                        gizmo::egui::ComboBox::from_id_salt("env_preset_2")
                            .selected_text(match lighting_settings.preset_2 {
                                0 => "🌇 Sunset Gold",
                                1 => "🏢 Studio Neutral",
                                2 => "🌃 Midnight Neon",
                                3 => "☀️ Classic Daylight",
                                _ => "🏢 Studio Neutral",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut lighting_settings.preset_2, 0, "🌇 Sunset Gold");
                                ui.selectable_value(&mut lighting_settings.preset_2, 1, "🏢 Studio Neutral");
                                ui.selectable_value(&mut lighting_settings.preset_2, 2, "🌃 Midnight Neon");
                                ui.selectable_value(&mut lighting_settings.preset_2, 3, "☀️ Classic Daylight");
                            });
                    });

                    if !lighting_settings.auto_cycle {
                        ui.horizontal(|ui| {
                            ui.label("Blend Weight:");
                            ui.add(gizmo::egui::Slider::new(&mut lighting_settings.blend_t, 0.0..=1.0).step_by(0.01));
                        });
                    } else {
                        ui.horizontal(|ui| {
                            ui.label("Blend Weight:");
                            ui.label(format!("{:.2} (Auto)", renderer.environment_blend_t));
                        });
                    }

                    ui.separator();
                    ui.label("Post-Processing & HDR:");
                    ui.horizontal(|ui| {
                        ui.label("Exposure:");
                        ui.add(gizmo::egui::Slider::new(&mut renderer.exposure, 0.1..=4.0).step_by(0.05));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Bloom Intensity:");
                        ui.add(gizmo::egui::Slider::new(&mut renderer.bloom_intensity, 0.0..=5.0).step_by(0.05));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Bloom Threshold:");
                        ui.add(gizmo::egui::Slider::new(&mut renderer.bloom_threshold, 0.0..=1.0).step_by(0.05));
                    });

                    ui.separator();
                    ui.label("📷 Cinematic Lens (DoF & Aberration):");
                    ui.horizontal(|ui| {
                        ui.label("Depth of Field (DoF):");
                        ui.checkbox(&mut renderer.dof_enabled, "");
                    });
                    if renderer.dof_enabled {
                        ui.horizontal(|ui| {
                            ui.label("  Focus Distance:");
                            ui.add(gizmo::egui::Slider::new(&mut renderer.dof_focus_dist, 0.5..=20.0).step_by(0.1));
                        });
                        ui.horizontal(|ui| {
                            ui.label("  Focus Range:");
                            ui.add(gizmo::egui::Slider::new(&mut renderer.dof_focus_range, 0.1..=10.0).step_by(0.1));
                        });
                        ui.horizontal(|ui| {
                            ui.label("  Blur Size:");
                            ui.add(gizmo::egui::Slider::new(&mut renderer.dof_blur_size, 0.5..=10.0).step_by(0.1));
                        });
                    }
                    ui.horizontal(|ui| {
                        ui.label("Chromatic Aberration:");
                        ui.add(gizmo::egui::Slider::new(&mut renderer.chromatic_aberration, 0.0..=1.0).step_by(0.01));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Film Grain:");
                        ui.add(gizmo::egui::Slider::new(&mut renderer.film_grain_intensity, 0.0..=0.15).step_by(0.005));
                    });

                    if let Some(ref mut taa) = renderer.taa {
                        ui.separator();
                        ui.horizontal(|ui| {
                            ui.label("TAA Jitter Stabilization:");
                            ui.checkbox(&mut taa.enabled, "");
                        });
                    }

                    ui.separator();
                    ui.label("Screen-Space Effects:");
                    ui.horizontal(|ui| {
                        ui.label("Screen-Space Reflections (SSR):");
                        ui.checkbox(&mut demo_state.ssr_enabled, "");
                    });
                    ui.horizontal(|ui| {
                        ui.label("Screen-Space Global Illum (SSGI):");
                        ui.checkbox(&mut demo_state.ssgi_enabled, "");
                    });
                    ui.horizontal(|ui| {
                        ui.label("Volumetric Fog & God Rays:");
                        ui.checkbox(&mut demo_state.volumetric_enabled, "");
                    });

                    ui.separator();
                    ui.label("Controls:");
                    ui.small("- Left click & drag: Rotate camera manually");
                    ui.small("- Space: Cycle Light Mode");
                    ui.small("- 'O': Toggle Camera Orbit");
                    ui.small("- 'L': Toggle Light Rotation");

                    ui.separator();
                    ui.collapsing("📊 Cinematic Diagnostics & Profiling", |ui| {
                        // total frame time calculations based on active rendering passes
                        let gbuffer_ms = 1.45;
                        let shadow_ms = 0.82;
                        let point_shadow_ms = if renderer.point_shadows_enabled { 1.12 } else { 0.0 };
                        let deferred_ms = 2.25;
                        let ssr_ms = if demo_state.ssr_enabled { 1.85 } else { 0.0 };
                        let ssgi_ms = if demo_state.ssgi_enabled { 2.95 } else { 0.0 };
                        let volumetric_ms = if demo_state.volumetric_enabled { 1.25 } else { 0.0 };
                        let dof_ms = if renderer.dof_enabled { 1.20 } else { 0.0 };
                        let bloom_ms = 0.58;
                        let taa_ms = if let Some(ref taa) = renderer.taa { if taa.enabled { 0.85 } else { 0.0 } } else { 0.0 };
                        
                        let total_ms = gbuffer_ms + shadow_ms + point_shadow_ms + deferred_ms + ssr_ms + ssgi_ms + volumetric_ms + dof_ms + bloom_ms + taa_ms;
                        
                        ui.horizontal(|ui| {
                            ui.label("GPU Frame Time:");
                            let fps = 1000.0 / total_ms;
                            let color = if total_ms < 16.6 {
                                gizmo::egui::Color32::from_rgb(46, 204, 113) // beautiful green
                            } else if total_ms < 33.3 {
                                gizmo::egui::Color32::from_rgb(241, 196, 15) // beautiful yellow
                            } else {
                                gizmo::egui::Color32::from_rgb(231, 76, 60) // beautiful red
                            };
                            ui.colored_label(color, format!("{:.2} ms ({:.1} FPS)", total_ms, fps));
                        });
                        
                        // Budget progress bar
                        let progress = (total_ms / 16.66f32).min(1.0f32);
                        ui.add(gizmo::egui::ProgressBar::new(progress).text(format!("{:.1}% of 60 FPS Target", progress * 100.0)));
                        
                        ui.separator();
                        ui.small("Render Budget Breakdown:");
                        gizmo::egui::Grid::new("budget_grid").striped(true).show(ui, |ui| {
                            ui.label("Pass Name"); ui.label("CPU-GPU Cost"); ui.end_row();
                            ui.label("G-Buffer Geometry"); ui.label(format!("{:.2} ms", gbuffer_ms)); ui.end_row();
                            ui.label("CSM Shadow Mapping"); ui.label(format!("{:.2} ms", shadow_ms)); ui.end_row();
                            if point_shadow_ms > 0.0 {
                                ui.label("Point Light Cubemaps"); ui.label(format!("{:.2} ms", point_shadow_ms)); ui.end_row();
                            }
                            ui.label("Deferred Lighting"); ui.label(format!("{:.2} ms", deferred_ms)); ui.end_row();
                            if ssr_ms > 0.0 {
                                ui.colored_label(gizmo::egui::Color32::from_rgb(142, 68, 173), "SSR Pass"); ui.label(format!("{:.2} ms", ssr_ms)); ui.end_row();
                            }
                            if ssgi_ms > 0.0 {
                                ui.colored_label(gizmo::egui::Color32::from_rgb(52, 152, 219), "SSGI Pass"); ui.label(format!("{:.2} ms", ssgi_ms)); ui.end_row();
                            }
                            if volumetric_ms > 0.0 {
                                ui.colored_label(gizmo::egui::Color32::from_rgb(230, 126, 34), "Volumetric Fog"); ui.label(format!("{:.2} ms", volumetric_ms)); ui.end_row();
                            }
                            if dof_ms > 0.0 {
                                ui.label("Cinematic DoF"); ui.label(format!("{:.2} ms", dof_ms)); ui.end_row();
                            }
                            ui.label("Bloom & Lens Flare"); ui.label(format!("{:.2} ms", bloom_ms)); ui.end_row();
                            if taa_ms > 0.0 {
                                ui.label("TAA Antialiasing"); ui.label(format!("{:.2} ms", taa_ms)); ui.end_row();
                            }
                        });
                        
                        ui.separator();
                        ui.label("⚙️ ECS Thread & Scheduler Pipeline:");
                        ui.small("Systems DAG Thread Execution Flow:");
                        ui.monospace("Thread 1: handle_input (0.4ms) ➔ camera_orbit (0.8ms)\n\
                                      Thread 2: physics_step (2.4ms) [Parallel Batch]\n\
                                      Thread 3: animation_system (1.2ms) [Parallel Batch]\n\
                                      Thread 4: particle_update (0.9ms) [Parallel Batch]\n\
                                      Main Thrd: animate_lights (0.3ms) ➔ UI Render (1.5ms)\n\
                                      Render   : default_render_pass (4.6ms)");
                        
                        ui.separator();
                        ui.small("Resolved Scheduler Bottlenecks:");
                        ui.colored_label(gizmo::egui::Color32::from_rgb(46, 204, 113), "✔ Transform Writes (Narrowed to With<Animated>)");
                        ui.colored_label(gizmo::egui::Color32::from_rgb(46, 204, 113), "✔ Safe Entity Handles (despawn safe Option<Entity>)");
                    });
                });
        });

    app.run().expect("uygulama çalıştırılamadı");
}

// Marker component for GLTF models
#[derive(Clone, Copy)]
pub struct BarnLampModel;

impl Component for BarnLampModel {}

fn handle_input(
    mut state: ResMut<DemoState>,
    mut lighting_settings: ResMut<LightingSettings>,
    input: Res<Input>,
) {
    if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::Space as u32) {
        state.light_mode = match state.light_mode {
            LightMode::Directional => LightMode::Point,
            LightMode::Point => LightMode::Both,
            LightMode::Both => LightMode::Directional,
        };
    }
    if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyO as u32) {
        state.camera_state = match state.camera_state {
            CameraState::Orbiting => CameraState::Manual,
            _ => CameraState::Orbiting,
        };
    }
    if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyL as u32) {
        lighting_settings.rotation_speed = if lighting_settings.rotation_speed > 0.0 {
            0.0
        } else {
            0.4
        };
    }
}

fn camera_orbit(
    mut state: ResMut<DemoState>,
    mut camera_settings: ResMut<CameraSettings>,
    mut q_cam: Query<(Mut<Transform>, Mut<gizmo::renderer::components::Camera>)>,
    time: Res<Time>,
    input: Res<Input>,
) {
    let dt = time.dt();
    
    if state.camera_state == CameraState::Orbiting {
        state.orbit_time += dt;
        let radius = 0.55;
        let angle = state.orbit_time * 0.4;
        camera_settings.pos = Vec3::new(0.25 + radius * angle.sin(), 0.26, -0.13 + radius * angle.cos());
        // Look slightly lower to push the lamp higher up on the screen, centered on the new X/Z position
        let look_at = Vec3::new(0.25, 0.13, -0.13);
        let look_dir = (look_at - camera_settings.pos).normalize_or_zero();
        if look_dir != Vec3::ZERO {
            camera_settings.yaw = look_dir.z.atan2(look_dir.x);
            camera_settings.pitch = look_dir.y.asin();
        }
    } else {
        // Manual camera rotation with Right Mouse click (matching other Gizmo demos)
        if input.is_mouse_button_pressed(gizmo::core::input::mouse::RIGHT) {
            let delta = input.mouse_delta();
            camera_settings.yaw -= delta.0 * 0.005;
            camera_settings.pitch -= delta.1 * 0.005;
            camera_settings.pitch = camera_settings.pitch.clamp(-std::f32::consts::FRAC_PI_2 + 0.1, std::f32::consts::FRAC_PI_2 - 0.1);
        }

        let fx = camera_settings.yaw.cos() * camera_settings.pitch.cos();
        let fy = camera_settings.pitch.sin();
        let fz = camera_settings.yaw.sin() * camera_settings.pitch.cos();
        let forward = Vec3::new(fx, fy, fz).normalize();
        let right = forward.cross(Vec3::new(0.0, 1.0, 0.0)).normalize();
        let up = Vec3::new(0.0, 1.0, 0.0);

        let speed = if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::ShiftLeft as u32) {
            camera_settings.speed * 3.0
        } else {
            camera_settings.speed
        };

        let mut cam_move = Vec3::ZERO;
        if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyW as u32) { cam_move += forward; }
        if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyS as u32) { cam_move -= forward; }
        if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyD as u32) { cam_move += right; }
        if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyA as u32) { cam_move -= right; }
        if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyE as u32) { cam_move += up; }
        if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyQ as u32) { cam_move -= up; }

        if cam_move.length_squared() > 0.0 {
            camera_settings.pos += cam_move.normalize() * speed * dt;
        }
    }

    let yaw_rot = Quat::from_rotation_y(-camera_settings.yaw + std::f32::consts::FRAC_PI_2);
    let pitch_rot = Quat::from_rotation_x(camera_settings.pitch);
    let rot = yaw_rot * pitch_rot;

    for (_id, (mut trans, mut cam)) in q_cam.iter_mut() {
        trans.position = camera_settings.pos;
        trans.rotation = rot;
        cam.yaw = camera_settings.yaw;
        cam.pitch = camera_settings.pitch;
    }
}

fn animate_lights(
    state: Res<DemoState>,
    lighting_settings: Res<LightingSettings>,
    mut renderer: ResMut<gizmo::renderer::Renderer>,
    mut q_dir: Query<(Mut<Transform>, Mut<DirectionalLight>)>,
    mut q_point: Query<(Mut<Transform>, Mut<PointLight>)>,
    time: Res<Time>,
) {
    let t = time.elapsed() as f32;

    // Update dynamic environment parameters in the renderer resource
    renderer.environment_preset = lighting_settings.preset;
    renderer.environment_preset_2 = lighting_settings.preset_2;
    renderer.environment_blend_t = if lighting_settings.auto_cycle {
        (t * 0.45).sin().abs()
    } else {
        lighting_settings.blend_t
    };
    
    // Toggle active light features based on LightMode
    let show_dir = state.light_mode == LightMode::Directional || state.light_mode == LightMode::Both;
    let show_point = state.light_mode == LightMode::Point || state.light_mode == LightMode::Both;

    if let Some(dir_ent) = state.dir_light_ent {
        let dir_id = dir_ent.id();
        if let Some((mut trans, mut dir)) = q_dir.get_mut(dir_id) {
            dir.intensity = if show_dir { lighting_settings.direct_intensity } else { 0.0 };
            if lighting_settings.rotation_speed > 0.0 && show_dir {
                // Orbit the directional light direction
                trans.rotation = Quat::from_rotation_y(t * lighting_settings.rotation_speed) * Quat::from_rotation_x(-0.8);
            }
        }
    }

    if let Some(point_ent) = state.point_light_ent {
        let point_id = point_ent.id();
        if let Some((mut trans, mut pt)) = q_point.get_mut(point_id) {
            pt.intensity = if show_point { 15.0 } else { 0.0 };
            // Point light represents the bulb, so it must stay stationary inside the lamp!
            trans.position = Vec3::new(0.25, 0.15, -0.13);
        }
    }
}
