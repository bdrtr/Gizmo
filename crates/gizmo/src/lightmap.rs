//! Baking a scene's lightmaps from its [`World`] in one call: [`bake_lightmaps`].
//!
//! ```ignore
//! world.add_component(floor, LightmapReceiver::new(256, 256));
//! let bake = LightmapBake::new().with_sky(Sky::uniform([0.3, 0.35, 0.4]));
//! let baked = gizmo::lightmap::bake_lightmaps(&mut world, bake, "assets/lightmaps")?;
//! ```
//!
//! The engine's lightmapper ([`gizmo_renderer::lightmap_bake`]) does the tracing; this takes the
//! scene to it. See [`bake_lightmaps`] for what it reads and what it writes.

use std::path::{Path, PathBuf};

use gizmo_core::{Entity, World};
use gizmo_math::{Mat4, Vec3};
use gizmo_physics_core::components::GlobalTransform;
use gizmo_renderer::components::{
    DirectionalLight, LightmapReceiver, Material, MaterialDesc, Mesh, MeshRenderer, PointLight, ShadowCasting, SpotLight,
};
use gizmo_renderer::lightmap_bake::{BakeLight, BakeMesh, LightmapBake, LightmapError};

/// One receiver [`bake_lightmaps`] baked: the entity, the picture it wrote, and the intensity it
/// set beside it.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct BakedReceiver {
    /// The entity with the [`LightmapReceiver`].
    pub entity: Entity,
    /// The PNG written, now the material's `maps.lightmap`.
    pub path: PathBuf,
    /// The material's `maps.lightmap_intensity`: the 8-bit image's scale.
    pub intensity: f32,
}

/// Bakes the lightmap of every entity with a [`LightmapReceiver`], writes each into `dir` as a PNG,
/// and sets its material to draw it ([`MaterialMaps::lightmap`](gizmo_renderer::components::MaterialMaps::lightmap)
/// and its intensity, on the `Material` and on the `MaterialDesc`, whichever it has).
///
/// `bake` brings the sky, the settings and any lights only the bake has
/// ([`BakeLight::baked_only`]); the world adds the rest:
///
/// * **What blocks and bounces light:** every mesh that is drawn and casts — a [`Mesh`] with a
///   [`MeshRenderer`] whose shadows are not `Off`, and not hidden — in its material's albedo and
///   emissive (a glowing material is an area light). Under the `physics` feature a body that moves
///   (a `RigidBody` that is not static) is left out: its shadow would stay baked where it was. The
///   material's textures are not read; its constant colour is.
/// * **The lights:** every `DirectionalLight`, `PointLight` and `SpotLight`, placed by their
///   transforms. They are in the scene at run time, so their direct light is added there, every
///   frame — the bake takes only their **bounces**.
/// * **The receivers:** a receiver's mesh needs its second UV set (`Vertex::lightmap_uv`); without
///   one the bake stops with [`LightmapError::NothingCovered`] and writes nothing. Its lightmap is
///   `lightmap_<entity>_<hash>.png` — the hash of the picture, so a bake that changes the light is a
///   new file the renderer loads rather than the one it has cached.
///
/// Placement is where each entity is now, through its parents (`gizmo::systems::world_matrix`), so a
/// scene baked before its first frame is where it will be drawn.
pub fn bake_lightmaps(world: &mut World, mut bake: LightmapBake, dir: impl AsRef<Path>) -> Result<Vec<BakedReceiver>, LightmapError> {
    let dir = dir.as_ref();
    let mut receivers = Vec::new();
    {
        // Where it is now, through its parents: a scene baked before its first frame has propagated
        // no `GlobalTransform` yet. An entity with no `Transform` is placed by its `GlobalTransform`.
        let matrix = |e: Entity| -> Option<Mat4> {
            crate::systems::world_matrix(world, e).or_else(|| world.query::<&GlobalTransform>().get(e).map(|g| g.matrix))
        };
        let materials = world.query::<&Material>();
        let descs = world.query::<&MaterialDesc>();
        let hidden = world.query::<&gizmo_core::component::IsHidden>();
        let receiving = world.query::<&LightmapReceiver>();
        #[cfg(feature = "physics")]
        let bodies = world.query::<&gizmo_physics_rigid::components::RigidBody>();
        let mut meshes: Vec<(Entity, Mesh, MeshRenderer)> =
            world.query::<(Entity, &Mesh, &MeshRenderer)>().iter().map(|(e, m, r)| (e, m.clone(), r.clone())).collect();
        // Entity order, so the bake — and the files it writes — are the same on every run.
        meshes.sort_by_key(|(e, _, _)| e.id());
        for (entity, mesh, renderer) in meshes {
            if renderer.shadows == ShadowCasting::Off || hidden.get(entity).is_some() {
                continue;
            }
            #[cfg(feature = "physics")]
            if bodies.get(entity).is_some_and(|b| !b.is_static()) {
                continue;
            }
            let Some(at) = matrix(entity) else { continue };
            let (albedo, emissive) = match (materials.get(entity), descs.get(entity)) {
                (Some(m), _) => (m.albedo.truncate(), m.emissive),
                (None, Some(d)) => (d.albedo.truncate(), d.emissive),
                (None, None) => (Vec3::splat(0.8), Vec3::ZERO),
            };
            let mut baked = BakeMesh::from_mesh(&mesh, at).with_albedo(albedo).with_emissive(emissive);
            if let Some(r) = receiving.get(entity) {
                baked = baked.receiving(r.width, r.height);
            }
            let id = bake.add_mesh(baked);
            if receiving.get(entity).is_some() {
                receivers.push((entity, id));
            }
        }

        let place = |e: Entity| matrix(e).map(|m| m.to_scale_rotation_translation());
        for (e, light) in world.query::<(Entity, &DirectionalLight)>().iter() {
            if let Some((_, rotation, _)) = place(e) {
                bake.add_light(BakeLight::from_directional(light, rotation * Vec3::NEG_Z));
            }
        }
        for (e, light) in world.query::<(Entity, &PointLight)>().iter() {
            if let Some((_, _, position)) = place(e) {
                bake.add_light(BakeLight::from_point(light, position));
            }
        }
        for (e, light) in world.query::<(Entity, &SpotLight)>().iter() {
            if let Some((_, rotation, position)) = place(e) {
                bake.add_light(BakeLight::from_spot(light, position, rotation * Vec3::NEG_Z));
            }
        }
    }

    let baked = bake.run()?;
    std::fs::create_dir_all(dir).map_err(|e| LightmapError::Write(format!("{}: {e}", dir.display())))?;
    let mut out = Vec::with_capacity(receivers.len());
    for (entity, id) in receivers {
        let Some(map) = baked.get(id) else { continue };
        let (bytes, _) = map.to_rgba8();
        let path = dir.join(format!("lightmap_{}_{:016x}.png", entity.id(), fnv1a(&bytes)));
        let intensity = map.save_png(&path)?;
        let name = path.to_string_lossy().into_owned();
        if let Some(mut m) = world.query_mut::<&mut Material>().get_mut(entity) {
            m.maps.lightmap = Some(name.clone());
            m.maps.lightmap_intensity = intensity;
        }
        if let Some(mut d) = world.query_mut::<&mut MaterialDesc>().get_mut(entity) {
            d.maps.lightmap = Some(name.clone());
            d.maps.lightmap_intensity = intensity;
        }
        out.push(BakedReceiver { entity, path, intensity });
    }
    Ok(out)
}

/// FNV-1a, 64 bits: names a picture by its contents.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_physics_core::components::Transform;
    use gizmo_renderer::frame_uniforms::Sky;
    use gizmo_renderer::gpu_types::Vertex;
    use gizmo_renderer::lightmap_bake::BakeSettings;
    use std::sync::Arc;

    struct Gpu {
        device: wgpu::Device,
        layout: wgpu::BindGroupLayout,
    }

    /// wgpu's noop backend: buffers and bind groups with no GPU.
    fn gpu() -> Gpu {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::NOOP,
            backend_options: wgpu::BackendOptions {
                noop: wgpu::NoopBackendOptions { enable: true, ..Default::default() },
                ..Default::default()
            },
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("the noop adapter");
        let (device, _queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("a noop device");
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &[] });
        Gpu { device, layout }
    }

    impl Gpu {
        fn material(&self) -> Material {
            Material::new(Arc::new(self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &self.layout, entries: &[] })))
        }
    }

    /// A floor 8 m across, facing up, its second UV set over the whole of it.
    fn floor(with_uv2: bool) -> Vec<Vertex> {
        let corner = |u: f32, v: f32| Vertex {
            position: [-4.0 + u * 8.0, 0.0, -4.0 + v * 8.0],
            normal: [0.0, 1.0, 0.0],
            tex_coords: [u, v],
            lightmap_uv: if with_uv2 { [u, v] } else { [0.0, 0.0] },
            ..Default::default()
        };
        let (a, b, c, d) = (corner(0.0, 0.0), corner(0.0, 1.0), corner(1.0, 1.0), corner(1.0, 0.0));
        vec![a, b, c, a, c, d]
    }

    fn spawn(world: &mut World, gpu: &Gpu, mesh: Mesh, at: Transform) -> Entity {
        world.spawn_bundle((at, GlobalTransform::default(), mesh, gpu.material(), MeshRenderer::new()))
    }

    fn bake() -> LightmapBake {
        LightmapBake::new().with_sky(Sky::uniform([1.0, 1.0, 1.0])).with_settings(BakeSettings::default().with_samples(64).with_bounces(0))
    }

    fn texel(path: &Path, x: u32, y: u32, intensity: f32) -> f32 {
        let image = image::open(path).expect("the baked lightmap").to_rgba8();
        f32::from(image.get_pixel(x, y)[0]) / 255.0 * intensity
    }

    /// A floor with a cube standing on it, both spawned this frame (no propagation has run): the
    /// receiver's material now names a PNG in which the floor under the cube is dark and the open
    /// floor has the sky — the cube placed by its `Transform`, as it will be drawn.
    #[test]
    fn a_world_bakes_its_receivers_and_their_materials_draw_the_map() {
        let gpu = gpu();
        let dir = std::env::temp_dir().join(format!("gizmo-lightmap-test-{}", std::process::id()));
        let mut world = World::new();
        let ground = spawn(&mut world, &gpu, Mesh::from_vertices(&gpu.device, &floor(true), "floor"), Transform::new(Vec3::ZERO));
        world.add_component(ground, LightmapReceiver::new(32, 32));
        // `create_cube` is ±1: half a metre each way, standing on the floor at (2, 0, 2).
        spawn(
            &mut world,
            &gpu,
            gizmo_renderer::asset::AssetManager::create_cube(&gpu.device),
            Transform::new(Vec3::new(2.0, 0.5, 2.0)).with_scale(Vec3::splat(0.5)),
        );
        let baked = bake_lightmaps(&mut world, bake(), &dir).expect("the bake");
        assert_eq!(baked.len(), 1);
        let b = &baked[0];
        assert_eq!(b.entity, ground);
        let material = world.get::<Material>(ground).unwrap();
        assert_eq!(material.maps.lightmap.as_deref(), Some(b.path.to_string_lossy().as_ref()));
        assert_eq!(material.maps.lightmap_intensity, b.intensity);
        // (2, 2) on the floor is texel (24, 24); the far corner (-4, -4) is texel (0, 0).
        let (under, open) = (texel(&b.path, 24, 24, b.intensity), texel(&b.path, 0, 0, b.intensity));
        eprintln!("under the cube {under}, open floor {open}");
        assert!(under < 0.05, "the floor under the cube saw the sky: {under}");
        assert!(open > 0.9, "the open floor lost its sky: {open}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// What is not drawn or does not cast is not in the bake: a hidden cube and one whose shadows are
    /// `Off` leave the floor under them lit.
    #[test]
    fn what_casts_no_shadow_bakes_none() {
        let gpu = gpu();
        let dir = std::env::temp_dir().join(format!("gizmo-lightmap-test-nocast-{}", std::process::id()));
        let mut world = World::new();
        let ground = spawn(&mut world, &gpu, Mesh::from_vertices(&gpu.device, &floor(true), "floor"), Transform::new(Vec3::ZERO));
        world.add_component(ground, LightmapReceiver::new(32, 32));
        let cube = |world: &mut World, x: f32| {
            spawn(world, &gpu, gizmo_renderer::asset::AssetManager::create_cube(&gpu.device), Transform::new(Vec3::new(x, 0.5, 2.0)).with_scale(Vec3::splat(0.5)))
        };
        let hidden = cube(&mut world, 2.0);
        world.add_component(hidden, gizmo_core::component::IsHidden);
        let no_shadow = cube(&mut world, -2.0);
        world.add_component(no_shadow, MeshRenderer::new().with_shadows(ShadowCasting::Off));
        let b = bake_lightmaps(&mut world, bake(), &dir).unwrap().remove(0);
        for x in [24, 8] {
            let t = texel(&b.path, x, 24, b.intensity);
            assert!(t > 0.9, "texel ({x}, 24) is shadowed by a cube that casts none: {t}");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A body that moves would leave its shadow baked where it was: a dynamic cube is left out, a
    /// static one is baked.
    #[cfg(feature = "physics")]
    #[test]
    fn a_moving_body_is_not_baked() {
        use gizmo_physics_rigid::components::RigidBody;
        let gpu = gpu();
        let dir = std::env::temp_dir().join(format!("gizmo-lightmap-test-bodies-{}", std::process::id()));
        let mut world = World::new();
        let ground = spawn(&mut world, &gpu, Mesh::from_vertices(&gpu.device, &floor(true), "floor"), Transform::new(Vec3::ZERO));
        world.add_component(ground, LightmapReceiver::new(32, 32));
        for (x, body) in [(2.0, RigidBody::new(1.0, true)), (-2.0, RigidBody::new_static())] {
            let cube = spawn(&mut world, &gpu, gizmo_renderer::asset::AssetManager::create_cube(&gpu.device), Transform::new(Vec3::new(x, 0.5, 2.0)).with_scale(Vec3::splat(0.5)));
            world.add_component(cube, body);
        }
        let b = bake_lightmaps(&mut world, bake(), &dir).unwrap().remove(0);
        let (dynamic, fixed) = (texel(&b.path, 24, 24, b.intensity), texel(&b.path, 8, 24, b.intensity));
        assert!(dynamic > 0.9, "the dynamic cube's shadow was baked: {dynamic}");
        assert!(fixed < 0.05, "the static cube's shadow was not: {fixed}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A receiver whose mesh has no second UV set stops the bake with the error that says so.
    #[test]
    fn a_receiver_without_a_second_uv_set_is_an_error() {
        let gpu = gpu();
        let mut world = World::new();
        let ground = spawn(&mut world, &gpu, Mesh::from_vertices(&gpu.device, &floor(false), "floor"), Transform::new(Vec3::ZERO));
        world.add_component(ground, LightmapReceiver::new(8, 8));
        let err = bake_lightmaps(&mut world, bake(), std::env::temp_dir().join("gizmo-lightmap-unused")).unwrap_err();
        assert!(matches!(err, LightmapError::NothingCovered { .. }), "{err:?}");
        assert!(world.get::<Material>(ground).unwrap().maps.lightmap.is_none());
    }

    /// The scene's lights are placed by their transforms and only bounce: a point light over a
    /// white wall's foot lights the floor beside it through the wall, not directly.
    #[test]
    fn the_scenes_lights_bake_only_their_bounces() {
        let gpu = gpu();
        let dir = std::env::temp_dir().join(format!("gizmo-lightmap-test-lights-{}", std::process::id()));
        let lit = |with_wall: bool| {
            let mut world = World::new();
            let ground = spawn(&mut world, &gpu, Mesh::from_vertices(&gpu.device, &floor(true), "floor"), Transform::new(Vec3::ZERO));
            world.add_component(ground, LightmapReceiver::new(16, 16));
            if with_wall {
                spawn(
                    &mut world,
                    &gpu,
                    gizmo_renderer::asset::AssetManager::create_cube(&gpu.device),
                    Transform::new(Vec3::new(0.0, 2.0, 4.5)).with_scale(Vec3::new(4.0, 2.0, 0.5)),
                );
            }
            world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 1.0, 2.5)),
                GlobalTransform::default(),
                PointLight::new(Vec3::ONE, 20.0, 10.0),
            ));
            let bake = LightmapBake::new().with_settings(BakeSettings::default().with_samples(64).with_bounces(1));
            let b = bake_lightmaps(&mut world, bake, &dir).unwrap().remove(0);
            let image = image::open(&b.path).unwrap().to_rgba8();
            image.pixels().map(|p| f32::from(p[0])).sum::<f32>() / 255.0 * b.intensity / image.pixels().len() as f32
        };
        let (without, with) = (lit(false), lit(true));
        eprintln!("floor without a wall {without}, with one {with}");
        assert!(without < 1e-3, "the scene's own light was baked directly: {without}");
        assert!(with > 0.01, "the light did not bounce off the wall: {with}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
