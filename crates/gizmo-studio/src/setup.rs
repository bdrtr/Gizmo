use crate::state::{DebugAssets, StudioState};
use gizmo::editor::EditorState;
use gizmo::math::{Quat, Vec3, Vec4};
use gizmo::physics::components::Transform;
use gizmo::prelude::*;

/// Builds the studio's starting scene — editor camera, grid, default light, debug assets — and
/// returns the state the update hook carries from then on.
pub fn setup_studio_scene(
    world: &mut World,
    renderer: &mut gizmo::renderer::Renderer,
) -> StudioState {
    // --- Setup Editor Scene (Grid & Axes) ---
    let mut asset_manager = gizmo::renderer::asset::AssetManager::new();
    let white_tex = asset_manager.create_white_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );

    spawn_sun(world, renderer, &white_tex);
    spawn_guidelines(world, renderer, &white_tex);
    spawn_default_cube(world, renderer, &white_tex);

    // Fizik sistemleri
    world.insert_resource(gizmo::physics::world::PhysicsWorld::new());

    register_asset_identities(&mut asset_manager);

    // Custom Skybox or proper horizon color
    world.insert_resource(asset_manager);
    world.insert_resource(gizmo::core::asset::Assets::<gizmo::renderer::components::Mesh>::default());

    let cam = spawn_editor_camera(world);
    let game_cam = spawn_game_camera(world);

    let mut editor_state = EditorState::new();
    editor_state.open = true; // Always open in Studio!
    world.insert_resource(editor_state);

    insert_debug_assets(world, renderer, &white_tex);
    world.insert_resource(gizmo::renderer::Gizmos::default());
    world.insert_resource(gizmo::core::FrameProfiler::new());
    world.insert_resource(component_registry());
    insert_script_engine(world);

    StudioState {
        current_fps: 0.0,
        actual_dt: 0.0,
        editor_camera: cam,
        game_camera: game_cam,
        do_raycast: false,
        play: gizmo::systems::PlayLoop::new(),
        // The asset browser's root, which the identity scan above reads too: three places that had
        // to agree on one directory, and this one was the string written out again.
        asset_watcher: gizmo::renderer::hot_reload::AssetWatcher::new(&[
            gizmo::editor::editor_state::AssetBrowserState::DEFAULT_WORKSPACE_ROOT,
            "scripts",
        ]),
        gc_timer: 0.0,
        autosave_timer: 0.0,
        visible_entity_count: 0,
        draw_call_count: 0,
        session: None,
        game: None,
    }
}

/// The default sun, with its icon: three crossed bars, editor furniture under it.
fn spawn_sun(world: &mut World, renderer: &gizmo::renderer::Renderer, white_tex: &std::sync::Arc<gizmo::wgpu::BindGroup>) {
    // Light (Güneş objesini görselleştir)
    let light = world.spawn();
    world.add_component(
        light,
        gizmo::core::component::EntityName("Directional Light".to_string()),
    );
    world.add_component(
        light,
        Transform::new(Vec3::new(3.0, 4.0, 3.0)).with_rotation(Quat::from_axis_angle(
            Vec3::new(1.0, 0.5, 0.0).normalize(),
            -std::f32::consts::FRAC_PI_4,
        )),
    );
    world.add_component(light, gizmo::physics::components::GlobalTransform::default());
    world.add_component(
        light,
        gizmo::renderer::components::DirectionalLight::new(
            Vec3::new(1.0, 0.95, 0.9),
            1.5,
            gizmo::renderer::components::LightRole::Sun,
        ),
    );

    // Light Icon (Kesişen prizmalar ile yıldız/güneş imgesi)
    let icons = [Vec3::new(0.04, 0.6, 0.04), Vec3::new(0.6, 0.04, 0.04), Vec3::new(0.04, 0.04, 0.6)]
        .into_iter()
        .zip(1..)
        .map(|(scale, n)| {
            let icon = world.spawn();
            world.add_component(icon, gizmo::core::component::EntityName(format!("Editor Light Icon {n}")));
            world.add_component(icon, Transform::new(Vec3::ZERO).with_scale(scale));
            world.add_component(icon, gizmo::physics::components::GlobalTransform::default());
            world.add_component(icon, gizmo::renderer::asset::AssetManager::create_cube(&renderer.device));
            world.add_component(
                icon,
                gizmo::prelude::Material::new(white_tex.clone()).with_unlit(Vec4::new(1.0, 0.8, 0.1, 1.0)),
            );
            world.add_component(icon, gizmo::renderer::components::MeshRenderer::new());
            world.add_component(icon, gizmo::core::component::Parent(light));
            world.add_component(icon, gizmo::core::component::EditorOnly);
            icon
        })
        .collect();

    world.add_component(light, gizmo::core::component::Children(icons));
}

/// The editor's guidelines: a root, and under it the grid.
fn spawn_guidelines(world: &mut World, renderer: &gizmo::renderer::Renderer, white_tex: &std::sync::Arc<gizmo::wgpu::BindGroup>) {
    // Editor Gizmos Root Node
    let gizmo_root = world.spawn();
    world.add_component(
        gizmo_root,
        gizmo::core::component::EntityName("Editor Guidelines".to_string()),
    );
    world.add_component(gizmo_root, Transform::new(Vec3::ZERO));
    world.add_component(gizmo_root, gizmo::physics::components::GlobalTransform::default());
    // Everything under this root is editor furniture. The marker is per-entity rather than
    // inherited because batching sees components, not ancestry — see `EditorOnly`.
    world.add_component(gizmo_root, gizmo::core::component::EditorOnly);

    // Procedural 3D Grid Lines and Infinite Axes
    // HDR uyumlu, hafif transparan çok şık ve ferah bir Grid materyali
    let grid_entity = world.spawn();
    world.add_component(
        grid_entity,
        gizmo::core::component::EntityName("Editor Grid".to_string()),
    );
    world.add_component(grid_entity, Transform::new(Vec3::ZERO));
    world.add_component(grid_entity, gizmo::physics::components::GlobalTransform::default());
    world.add_component(grid_entity, gizmo::core::component::Parent(gizmo_root));

    let grid_mesh =
        gizmo::renderer::asset::AssetManager::create_editor_grid_mesh(&renderer.device, 500.0);

    // Grid Material
    let mut grid_mat = gizmo::prelude::Material::new(white_tex.clone());
    grid_mat.albedo = gizmo::math::Vec4::new(1.0, 1.0, 1.0, 1.0);
    grid_mat.material_type = gizmo::renderer::components::MaterialType::Grid;

    // Editörün arka plan matris nesnesi olarak mesh ata. Pickable OLMASIN (fare engellemesin).
    world.add_component(grid_entity, grid_mesh);
    world.add_component(grid_entity, grid_mat);
    world.add_component(
        grid_entity,
        gizmo::renderer::components::MeshRenderer::new(),
    );
    world.add_component(grid_entity, gizmo::core::component::EditorOnly);

    // Merkez Eksenler (Kırmızı/Mavi Çizgiler) artık Grid Shader (grid.wgsl) tarafından
    // tam kalbinde 0 GPU sızıntısı ile çizilmektedir! Bu yüzden gereksiz Transform küplerini sildik.

    // Attach all children to root
    world.add_component(gizmo_root, gizmo::core::component::Children(vec![grid_entity]));
}

/// The default cube, to have something to interact with: solid, since a collider on its own is
/// static geometry.
fn spawn_default_cube(world: &mut World, renderer: &gizmo::renderer::Renderer, white_tex: &std::sync::Arc<gizmo::wgpu::BindGroup>) {
    let cube1 = world.spawn();
    world.add_component(
        cube1,
        gizmo::core::component::EntityName("Default Cube".to_string()),
    );
    world.add_component(cube1, Transform::new(Vec3::new(0.0, 0.0, 0.0))); // Tam Merkez!
    world.add_component(cube1, gizmo::physics::components::GlobalTransform::default());
    world.add_component(
        cube1,
        gizmo::renderer::asset::AssetManager::create_cube(&renderer.device),
    );
    world.add_component(
        cube1,
        gizmo::prelude::Material::new(white_tex.clone()).with_pbr(
            Vec4::new(0.21, 0.21, 0.21, 1.0),
            0.5,
            0.0,
        ),
    );
    world.add_component(cube1, gizmo::renderer::components::MeshRenderer::new());
    world.add_component(
        cube1,
        Collider::box_collider(gizmo::math::Vec3::new(1.0, 1.0, 1.0)),
    ); // Visual mesh is 2x2x2 (from -1 to +1)
}

/// Registers the identity of every asset in the project that already has one.
///
/// Read-only on purpose: `scan_assets_directory` skips an asset with no `.meta` sidecar rather
/// than stamping one, because writing into somebody's asset tree is an import action, not a
/// side effect of opening the editor (see `AssetManager::import_assets_directory`). Until this
/// call existed nothing anywhere populated the registry, so the path↔UUID machinery — the
/// scanner, the resolver, the sidecars on disk — was wired to nothing, and a scene could not
/// have been repaired by identity even in principle.
///
/// The directory is the asset browser's own root, so the editor cannot end up scanning one tree
/// while showing another.
fn register_asset_identities(asset_manager: &mut gizmo::renderer::asset::AssetManager) {
    let workspace_root = gizmo::editor::editor_state::AssetBrowserState::DEFAULT_WORKSPACE_ROOT;
    asset_manager.scan_assets_directory(std::path::Path::new(workspace_root));
    tracing::info!(
        root = workspace_root,
        registered = asset_manager.path_to_uuid.len(),
        "[Studio] varlık kimlik kaydı tarandı"
    );
}

/// The editor's own camera, the one the Scene panel draws through.
fn spawn_editor_camera(world: &mut World) -> Entity {
    let cam = world.spawn();
    world.add_component(
        cam,
        gizmo::core::component::EntityName("Editor Camera".to_string()),
    );
    world.add_component(
        cam,
        Transform::new(Vec3::new(0.0, 8.0, 18.0))
            .with_rotation(gizmo::renderer::components::Camera::rotation_from(-std::f32::consts::FRAC_PI_2, -0.4)),
    );
    world.add_component(cam, gizmo::physics::components::GlobalTransform::default());
    world.add_component(
        cam,
        gizmo::renderer::components::Camera::new(
            60.0_f32.to_radians(),
            0.1,
            1000.0,
            true),
    );
    cam
}

/// The game's camera, the one play mode draws through: twelve metres back, looking slightly down
/// at the origin.
fn spawn_game_camera(world: &mut World) -> Entity {
    // Game Camera (Play modunda kullanılacak oyun kamerası)
    let game_cam = world.spawn();
    world.add_component(
        game_cam,
        gizmo::core::component::EntityName("Game Camera".to_string()),
    );
    let game_cam_pos = Vec3::new(0.0, 3.0, 12.0);
    let look_target = Vec3::new(0.0, 1.0, 0.0);
    let game_cam_forward = look_target - game_cam_pos;
    let (game_cam_yaw, game_cam_pitch) =
        gizmo::renderer::components::Camera::yaw_pitch_from_forward(game_cam_forward, 0.0)
            .unwrap_or((0.0, 0.0));
    world.add_component(
        game_cam,
        Transform::new(game_cam_pos)
            .with_rotation(gizmo::renderer::components::Camera::rotation_from(game_cam_yaw, game_cam_pitch)),
    );
    world.add_component(game_cam, gizmo::physics::components::GlobalTransform::default());
    world.add_component(
        game_cam,
        gizmo::renderer::components::Camera::new(
            50.0_f32.to_radians(), // Biraz daha dar FOV (sinematik)
            0.1,
            500.0,
            false, // Editör kamerası DEĞİL
        ),
    );
    game_cam
}

/// The meshes the ➕ menu's primitives are spawned with.
fn insert_debug_assets(world: &mut World, renderer: &gizmo::renderer::Renderer, white_tex: &std::sync::Arc<gizmo::wgpu::BindGroup>) {
    // Every dimension here comes from `PrimitiveSize`, which the spawned colliders read too —
    // see the type's own note for what happened when they were two separate numbers.
    use crate::state::PrimitiveSize as PS;
    use gizmo::renderer::asset::AssetManager as AM;
    world.insert_resource(DebugAssets {
        cube: AM::create_cube(&renderer.device),
        sphere: AM::create_sphere(&renderer.device, PS::SPHERE_RADIUS, 16, 16),
        plane: AM::create_plane(&renderer.device, PS::PLANE_SIZE),
        cylinder: AM::create_cylinder(
            &renderer.device,
            PS::CYLINDER_RADIUS,
            PS::CYLINDER_HEIGHT,
            PS::CYLINDER_SEGMENTS,
        ),
        capsule: AM::create_capsule(
            &renderer.device,
            PS::CAPSULE_RADIUS,
            PS::CAPSULE_DEPTH,
            12,
            16,
        ),
        white_tex: white_tex.clone(),
    });
}

/// The components the editor can list, add and edit.
fn component_registry() -> gizmo::core::ComponentRegistry {
    let mut component_registry = gizmo::core::ComponentRegistry::new();
    let _ = component_registry.register::<gizmo::physics::components::Transform>("Transform");
    let _ = component_registry.register::<gizmo::physics::Velocity>("Velocity");
    let _ = component_registry.register::<gizmo::physics::RigidBody>("RigidBody");
    let _ = component_registry.register::<gizmo::physics::Collider>("Collider");
    let _ = component_registry.register::<gizmo::renderer::components::Camera>("Camera");
    let _ = component_registry.register::<gizmo::renderer::components::PointLight>("PointLight");
    // The other two light types, which the engine has drawn and the scene file has carried all
    // along with no way to create either one in the editor. The sun was the sharper case: only
    // this file's startup scene ever spawned one, so a scene arriving from Rust, from a
    // hand-written file or from another tool could never gain a sun — it stayed lit by point
    // lights for ever. Three entries rather than one "Light" section with a type switch, because
    // that is the shape the editor already has: `PointLight` is its own entry and
    // `inspector/light.rs` is already one section per type.
    let _ = component_registry.register::<gizmo::renderer::components::DirectionalLight>("DirectionalLight");
    let _ = component_registry.register::<gizmo::renderer::components::SpotLight>("SpotLight");
    let _ = component_registry.register::<gizmo::renderer::components::RectLight>("RectLight");
    // The authored look. Reachable from the ➕ menu as well as from the environment panel's own
    // button, because a scene can have more than one camera and only the panel's copy is about
    // "the" camera. Its default is the engine's no-component behaviour, so adding one is safe.
    let _ = component_registry.register::<gizmo::renderer::components::PostProcess>("PostProcess");
    let _ = component_registry.register::<gizmo::prelude::Material>("Material");
    // What a 2D game and a HUD are made of. Loaded from a scene and drawn all along, and neither
    // addable nor editable here until 2026-11-16 (`gizmo_editor::inspector::two_d`).
    let _ = component_registry.register::<gizmo::renderer::components::Sprite>("Sprite");
    let _ = component_registry.register::<gizmo::renderer::components::Text>("Text");
    let _ = component_registry.register::<gizmo::scripting::Script>("Script");
    let _ = component_registry.register::<gizmo::renderer::components::ParticleEmitter>("ParticleEmitter");
    let _ = component_registry.register::<gizmo::prelude::AudioSource>("AudioSource");
    let _ = component_registry.register::<gizmo::renderer::components::Terrain>("Terrain");
    let _ = component_registry.register::<gizmo::physics::components::Hitbox>("Hitbox");
    let _ = component_registry.register::<gizmo::physics::components::Hurtbox>("Hurtbox");
    let _ = component_registry.register::<gizmo::renderer::components::BoneAttachment>("BoneAttachment");
    let _ = component_registry.register::<gizmo::physics::components::fighter::FighterController>("FighterController");
    let _ = component_registry.register::<gizmo::ai::components::NavAgent>("NavAgent");
    let _ = component_registry.register::<gizmo::ai::BehaviorTree>("BehaviorTree");
    // The scene's fluid and what stands in it: both reach the GPU solver through the frame
    // preparation, so adding one here is what puts water in the scene.
    let _ = component_registry.register::<gizmo::physics::components::FluidSimulation>("FluidSimulation");
    let _ = component_registry.register::<gizmo::renderer::components::FluidInteractor>("FluidInteractor");
    // The limbs the engine bends: added from ➕ and edited in the inspector's generic rows — a
    // chain's tip bone by name, its target and pole as world points, its weight.
    let _ = component_registry.register::<gizmo::renderer::ik::IkChains>("IkChains");
    // Keyed transforms and fields — a door, a platform, a camera move, a pulsing lamp — which the
    // ANIMATION tab keys.
    let _ = component_registry.register::<gizmo::renderer::keyed_animation::KeyedAnimationPlayer>("KeyedAnimationPlayer");
    component_registry
}

/// The Lua engine the scripts run on. A failure to start it is an error, with its cause: it was
/// logged as `info` and the cause thrown away.
fn insert_script_engine(world: &mut World) {
    // --- SCRIPT ENGINE & ASSET WATCHER BİRLEŞİMİ ---
    match gizmo::scripting::ScriptEngine::new() {
        Ok(engine) => {
            tracing::info!("🚀 Gizmo Studio Script Motoru Başlatıldı.");
            world.insert_resource(engine);
        }
        Err(error) => tracing::error!(?error, "❌ Gizmo Studio Script Motoru Başlatılamadı — scriptler çalışmayacak"),
    }
}

#[cfg(test)]
mod tests {
    use gizmo::math::Vec3;
    use gizmo::renderer::components::Camera;

    /// Regression for the game-camera yaw/pitch initialization (setup.rs).
    /// The old code used `forward.x.atan2(-forward.z)` / `(-forward.y).asin()`,
    /// which does NOT invert `get_front()` and yields the wrong facing.
    ///
    /// Both this and the fight-camera test below used to carry their own copy of the inversion AND
    /// their own copy of `get_front` — so they measured a formula that no longer had to be the one
    /// in the source. They now call the real pair, which is the only version of it left.
    #[test]
    fn game_camera_yaw_pitch_inverts_get_front() {
        let game_cam_pos = Vec3::new(0.0, 3.0, 12.0);
        let look_target = Vec3::new(0.0, 1.0, 0.0);
        let forward = (look_target - game_cam_pos).normalize();

        let (yaw, pitch) =
            Camera::yaw_pitch_from_forward(forward, 0.0).expect("a real look direction");

        // yaw should point toward -Z (roughly -PI/2), NOT 0 as the buggy code produced.
        let expected_yaw = (-2.0_f32).atan2(0.0); // atan2(neg, 0) == -PI/2
        assert!((yaw - expected_yaw).abs() < 1e-4, "yaw = {yaw}");

        // Reconstructed forward must match the original direction.
        let reconstructed = Camera::forward_from(yaw, pitch);
        assert!((reconstructed.x - forward.x).abs() < 1e-4);
        assert!((reconstructed.y - forward.y).abs() < 1e-4);
        assert!((reconstructed.z - forward.z).abs() < 1e-4);
    }

    /// Regression for the fight-camera auto-focus look-at (systems/simulation.rs).
    /// Same inversion math; guards against a re-introduction of swapped atan2 args.
    #[test]
    fn fight_camera_lookat_yaw_pitch_inverts_get_front() {
        let cam_pos = Vec3::new(0.0, 3.0, 12.0);
        let look_target = Vec3::new(0.0, 1.0, 0.0);
        let dir = (look_target - cam_pos).normalize();

        let (yaw, pitch) = Camera::yaw_pitch_from_forward(dir, 0.0).expect("a real look direction");

        let reconstructed = Camera::forward_from(yaw, pitch);
        assert!((reconstructed.x - dir.x).abs() < 1e-4);
        assert!((reconstructed.y - dir.y).abs() < 1e-4);
        assert!((reconstructed.z - dir.z).abs() < 1e-4);
    }

    /// The fight camera lerps its position towards the target and then looks at it, so it can
    /// arrive exactly on top of it — and `(target - pos).normalize()` is NaN there, which used to
    /// be written straight into `cam.yaw` and take the view matrix with it.
    #[test]
    fn a_camera_sitting_on_its_target_keeps_its_angles() {
        assert!(Camera::yaw_pitch_from_forward(Vec3::ZERO, 0.7).is_none());
        // ...and glam hands us exactly that: normalizing a zero difference gives NaN.
        let degenerate = (Vec3::ONE - Vec3::ONE).normalize();
        assert!(degenerate.is_nan(), "the input this guards against");
        assert!(Camera::yaw_pitch_from_forward(degenerate, 0.7).is_none());
    }
}
