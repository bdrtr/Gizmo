//! A scene has to come back the way it went out.
//!
//! Both ends of this are load-bearing now: the editor writes the file, and the exported game's
//! runtime (`demo/src/bin/gizmo_runtime.rs`) is the only thing that reads it — nothing else in the
//! repository opens a `.scene` at startup. A drop between the two is invisible from either side.
//!
//! The registry is the part worth pinning. `gizmo::full_scene_registry()` exists because the
//! physics-only `default_scene_registry` was what the save path used, and a scene therefore came
//! back with its bodies intact and its lights and cameras gone — no error, an unlit empty world.
//! This drives the real one, and it drives it through the facade, which is what a game links.
//!
//! Known and deliberate gap, so nobody reads this test as promising more than it does: `Material`
//! is not registered (it owns a live wgpu bind group), so PBR maps do not survive a round trip.
//! `MaterialSource` is what carries a material through a file, and that is what is asserted here.

use gizmo::core::component::{EntityName, MaterialSource, MeshSource};
use gizmo::prelude::*;
use gizmo::renderer::components::{Camera, DirectionalLight};
use gizmo::scene::scene::SceneData;

/// A floor, a cube, a sphere, a sun and a camera — the smallest scene that is still a scene.
fn sample_world() -> World {
    let mut world = World::new();

    let floor = world.spawn();
    world.add_component(floor, EntityName::new("Zemin"));
    world.add_component(
        floor,
        Transform::new(Vec3::new(0.0, -1.0, 0.0)).with_scale(Vec3::new(20.0, 0.4, 20.0)),
    );
    world.add_component(floor, MeshSource("standard_cube".to_string()));
    world.add_component(
        floor,
        MaterialSource {
            albedo: [0.75, 0.75, 0.78, 1.0],
            roughness: 0.9,
            metallic: 0.0,
            unlit: 0.0,
            texture_source: None,
        },
    );

    let cube = world.spawn();
    world.add_component(cube, EntityName::new("Kutu"));
    world.add_component(cube, Transform::new(Vec3::ZERO));
    world.add_component(cube, MeshSource("standard_cube".to_string()));
    world.add_component(
        cube,
        MaterialSource {
            albedo: [0.85, 0.2, 0.15, 1.0],
            roughness: 0.5,
            metallic: 0.0,
            unlit: 0.0,
            texture_source: None,
        },
    );

    let ball = world.spawn();
    world.add_component(ball, EntityName::new("Küre"));
    world.add_component(
        ball,
        Transform::new(Vec3::new(2.5, 0.5, 1.5)).with_scale(Vec3::splat(1.2)),
    );
    world.add_component(ball, MeshSource("sphere".to_string()));

    world.spawn_bundle(DirectionalLightBundle::default());
    world.spawn_bundle(CameraBundle {
        position: Vec3::new(-7.0, 3.0, 0.0),
        yaw: 0.0,
        pitch: -0.30,
        primary: true,
        ..Default::default()
    });

    world
}

#[test]
fn a_saved_scene_comes_back_with_its_meshes_lights_and_camera() {
    let world = sample_world();
    let registry = gizmo::full_scene_registry();

    let path = std::env::temp_dir().join(format!("gizmo_round_trip_{}.scene", std::process::id()));
    let path = path.to_string_lossy().to_string();
    SceneData::save(&world, &path, &registry).expect("sahne kaydedilemedi");

    let mut loaded = World::new();
    let result = SceneData::load_into(&path, &mut loaded, &registry);
    let _ = std::fs::remove_file(&path);
    result.expect("kaydedilen sahne geri yüklenemedi");

    // What the picture is made of.
    {
        let meshes = loaded.borrow::<MeshSource>();
        let mut sources: Vec<String> = meshes.iter().map(|(_, m)| m.0.clone()).collect();
        sources.sort();
        assert_eq!(
            sources,
            vec![
                "sphere".to_string(),
                "standard_cube".to_string(),
                "standard_cube".to_string()
            ],
            "mesh sources are how a loaded entity becomes visible at all"
        );
    }

    // What it is lit by, and what looks at it: the pair that used to vanish silently.
    {
        let lights = loaded.borrow::<DirectionalLight>();
        let found: Vec<f32> = lights.iter().map(|(_, l)| l.intensity).collect();
        assert_eq!(found, vec![3.0], "the sun must survive the file");

        let cameras = loaded.borrow::<Camera>();
        let primaries = cameras.iter().filter(|(_, c)| c.primary).count();
        assert_eq!(primaries, 1, "a scene with no primary camera draws nothing");
    }

    // Values, not just presence — a component that comes back with default contents is the same
    // bug wearing the shape of a pass.
    {
        let names = loaded.borrow::<EntityName>();
        let transforms = loaded.borrow::<Transform>();
        let materials = loaded.borrow::<MaterialSource>();

        let floor = names
            .iter()
            .find(|(_, n)| n.0 == "Zemin")
            .map(|(id, _)| id)
            .expect("adlar da dosyadan geliyor");
        let t = transforms.get(floor).expect("zeminin transform'u");
        assert_eq!(t.position, Vec3::new(0.0, -1.0, 0.0));
        assert_eq!(t.scale, Vec3::new(20.0, 0.4, 20.0));

        let m = materials.get(floor).expect("zeminin materyali");
        assert_eq!(m.albedo, [0.75, 0.75, 0.78, 1.0]);
        assert_eq!(m.roughness, 0.9);
    }
}
