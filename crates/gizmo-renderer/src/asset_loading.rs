use gizmo_core::query::Without;
use gizmo_core::Entity;
use gizmo_core::World;
use gizmo_core::component::{MeshSource, MaterialSource};
use crate::components::{Mesh, Material, MeshRenderer};
use crate::asset::AssetManager;
use wgpu::Device;
use wgpu::Queue;
use wgpu::BindGroupLayout;

/// The [`MeshData`](crate::components::MeshData) an entity's mesh was uploaded from, so a replaced
/// `MeshData` is uploaded again. Internal bookkeeping of the asset loader.
///
/// It holds the data, not the data's address. It held the address, and an address outlives its
/// allocation: a `MeshData` taken off an entity was freed once nothing else drew it, a new one of
/// the same size could be given the same address, and added to that entity it was taken for the
/// one already uploaded — the entity drew the old shape. Held here, the allocation lives as long as
/// the entity remembers it, so its address is never another's while it does.
#[derive(Clone, Debug)]
pub(crate) struct UploadedFrom(crate::components::MeshData);
gizmo_core::impl_component!(UploadedFrom);

/// Continuously scans the scene for newly added `MeshSource` and `MaterialSource` components and
/// loads the `Mesh` and `Material` GPU components they are missing.
///
/// **A [`MaterialDesc`](crate::components::MaterialDesc) outranks a `MaterialSource`.** Both can
/// build a material and they do not carry the same thing: a source holds albedo, roughness,
/// metallic, an unlit flag and a texture path, while a description holds every field a `Material`
/// has. A scene row saved after descriptions existed carries both — the source because it was
/// authored before, the description because a save writes one for every live material — and if
/// this system won, the richer record would be discarded and then *overwritten* by the poorer one
/// on the next save. So an entity that has a description is left for
/// `material_sync::resolve_material_descriptions`; it still gets its `MeshRenderer` here, because
/// that pairing is this system's to guarantee.
#[tracing::instrument(skip_all, level = "trace")]
pub fn run_asset_loading_system(
    world: &mut World,
    device: &Device,
    queue: &Queue,
    texture_bind_group_layout: &BindGroupLayout,
    asset_manager: &mut AssetManager,
) {
    let mut missing_meshes = Vec::new();
    // Lightmap receivers whose mesh needs its second UV set: (entity, source, receiver, has no mesh).
    let mut receivers: Vec<(Entity, String, crate::components::LightmapReceiver, bool)> = Vec::new();
    // Geometry made in code (`MeshData`) waiting for its upload — first time, or because the
    // entity's `MeshData` was replaced since.
    let mut missing_data: Vec<(u32, crate::components::MeshData)> = Vec::new();
    let mut missing_materials = Vec::new();
    // Entities whose material a DESCRIPTION will build (see this function's docs) but whose
    // `MeshRenderer` is still this system's to add.
    let mut renderer_only = Vec::new();
    // Every `MeshData` allocation an entity carries: the cached uploads worth keeping.
    let mut live_data = std::collections::HashSet::new();

    // Hangi Entity'lerin Mesh/Material'ı eksik bul — **only among the entities that carry what
    // the loader loads from.** It used to walk every live entity and ask each for seven components,
    // so its cost grew with the world, not with the work: an app with no source at all paid for
    // the whole scan every frame, which is why the loop ran it only when the game had put an
    // `AssetManager` in the world — and a game that had not, but spawned `MeshData`, drew nothing.
    // Each query below visits only the archetypes holding its component, and the filters skip an
    // entity whose mesh or material is already there.
    {
        let meshes = world.query::<&Mesh>();
        let uploaded = world.query::<&UploadedFrom>();
        // Data made in code, not uploaded yet. A `MeshSource` on the same entity loses: the data
        // is the more specific of the two.
        for (id, data) in world.query::<(Entity, &crate::components::MeshData)>().iter() {
            live_data.insert(data.key());
            // Not uploaded yet, or uploaded from another allocation: the game replaced the data (a
            // vision cone rebuilt each frame). It used to be read once — the first frame the
            // entity had no `Mesh` — so every later change drew nothing new.
            let stale = match uploaded.get(id) {
                Some(from) => !from.0.same_allocation(data),
                None => meshes.get(id).is_none(),
            };
            if stale {
                missing_data.push((id.id(), data.clone()));
            }
        }

        // A lightmap receiver drawn from a source with no second UV set — a file without
        // `TEXCOORD_1`, a primitive — draws a copy laid out for its lightmap (`receiver_mesh`), so the
        // map baked for it is read where it was baked. Before its first mesh, or once its plain one
        // has been loaded (the receiver added later).
        for (id, src, receiver) in world
            .query_filtered::<(Entity, &MeshSource, &crate::components::LightmapReceiver), Without<crate::components::MeshData>>()
            .iter()
        {
            if meshes.get(id).is_none_or(|m| m.cpu_lightmap.is_none()) {
                receivers.push((id, src.0.clone(), *receiver, meshes.get(id).is_none()));
            }
        }

        // MeshSource var ama GPU Mesh yok mu? (Not when data is supplying the mesh.)
        let receiving = world.query::<&crate::components::LightmapReceiver>();
        for (id, src) in world
            .query_filtered::<(Entity, &MeshSource), (Without<Mesh>, Without<crate::components::MeshData>)>()
            .iter()
        {
            if receiving.get(id).is_none() {
                missing_meshes.push((id, src.0.clone()));
            }
        }

        // MaterialSource var ama GPU Material yok mu?
        let descs = world.query::<&crate::components::MaterialDesc>();
        for (id, src) in world.query_filtered::<(Entity, &MaterialSource), Without<Material>>().iter() {
            if descs.get(id).is_some() {
                renderer_only.push(id);
            } else {
                missing_materials.push((id, src.clone()));
            }
        }
    }

    // Idle frames find nothing missing and stay silent; only log when there is
    // actual GPU upload work this frame (avoids per-frame noise on a steady scene).
    if !missing_meshes.is_empty() || !missing_materials.is_empty() {
        tracing::debug!(
            meshes = missing_meshes.len(),
            materials = missing_materials.len(),
            "[AssetLoading] uploading missing GPU components"
        );
    }

    for (id, data) in missing_data {
        let Some(entity) = world.get_entity(id) else { continue };
        match asset_manager.mesh_from_data(device, &data) {
            Some(mesh) => world.add_component(entity, mesh),
            // Emptied: nothing to draw, so no mesh — not the last shape it had.
            None => world.remove_component::<Mesh>(entity),
        }
        world.add_component(entity, UploadedFrom(data));
    }
    for (id, src, receiver, meshless) in receivers {
        let Some(entity) = world.live(id) else { continue };
        match asset_manager.receiver_mesh(device, &src, receiver.width, receiver.height) {
            Some(mesh) => world.add_component(entity, mesh),
            // Warned about in `receiver_mesh`: drawn as it is, without the lightmap's UVs.
            None if meshless => missing_meshes.push((entity, src)),
            None => {}
        }
    }

    // The uploads no entity's data names any more — replaced (a vision cone, a cloth rebuilt
    // every frame) or despawned. The game path never runs `garbage_collect`, so without this each
    // replaced frame's vertices and GPU buffers were kept for the rest of the run.
    asset_manager.retain_data_meshes(&live_data);

    // Default beyaz doku (Texture yüklenemezse veya yoksa kullanılır) — only when something is
    // missing: on a fresh `AssetManager` it is the first GPU work the loader does, and a frame with
    // nothing to load does none.
    if missing_meshes.is_empty() && missing_materials.is_empty() {
        add_mesh_renderers(world, renderer_only);
        return;
    }
    let default_texture_bind_group = asset_manager.create_white_texture(device, queue, texture_bind_group_layout);

    // Eksik Mesh'leri yükle ve world'e ekle
    for (id, mesh_src) in missing_meshes {
        if let Some(entity) = world.live(id) {
            let mesh = asset_manager.mesh_from_source(device, queue, texture_bind_group_layout, &mesh_src);

            world.add_component(entity, mesh);
        }
    }

    // Eksik Material'ları yükle ve world'e ekle
    for (id, mat_data) in missing_materials {
        if let Some(entity) = world.live(id) {
            let bind_group = if let Some(tex_path) = &mat_data.texture_source {
                asset_manager
                    .load_material_texture(device, queue, texture_bind_group_layout, tex_path)
                    .unwrap_or_else(|e| {
                        // Recoverable: falls back to the default white texture, but the
                        // requested texture is missing/undecodable — a real problem to surface.
                        tracing::warn!(
                            texture = %tex_path,
                            error = %e,
                            "[AssetLoading] material texture load failed; using default white texture"
                        );
                        default_texture_bind_group.clone()
                    })
            } else {
                default_texture_bind_group.clone()
            };

            let mut mat = Material::new(bind_group);
            mat.albedo = gizmo_math::Vec4::from(mat_data.albedo);
            mat.roughness = mat_data.roughness;
            mat.metallic = mat_data.metallic;
            mat.material_type = if mat_data.unlit > 1.5 {
                crate::components::MaterialType::Skybox
            } else if mat_data.unlit > 0.5 {
                crate::components::MaterialType::Unlit
            } else {
                crate::components::MaterialType::Pbr
            };
            mat.texture_source = mat_data.texture_source;

            world.add_component(entity, mat);
            // Her Material'ın yanında bir MeshRenderer olmalıdır (Render Pipeline için)
            world.add_component(entity, MeshRenderer::new());
        }
    }

    add_mesh_renderers(world, renderer_only);
}

/// The description-owned entities: their material arrives from `material_sync`, but the
/// `MeshRenderer` that has to sit beside it is still the loader's guarantee. Added only when
/// missing, so an entity that already carries one keeps whatever flags it was given.
fn add_mesh_renderers(world: &mut World, renderer_only: Vec<Entity>) {
    if renderer_only.is_empty() {
        return;
    }
    let needed: Vec<Entity> = {
        let renderers = world.query::<&MeshRenderer>();
        renderer_only.into_iter().filter(|id| renderers.get(*id).is_none()).collect()
    };
    for id in needed {
        if let Some(e) = world.live(id) {
            world.add_component(e, MeshRenderer::new());
        }
    }
}

#[cfg(test)]
mod tests {
    /// Source with its comments removed — a `contains` over raw source is satisfied by the
    /// paragraph explaining the rule, which is exactly what this must not accept.
    fn code_only(src: &str) -> String {
        src.lines()
            .map(|line| {
                let bytes = line.as_bytes();
                let mut end = line.len();
                let mut i = 0;
                while i + 1 < bytes.len() {
                    if bytes[i] == b'/' && bytes[i + 1] == b'/' && (i == 0 || bytes[i - 1] != b':') {
                        end = i;
                        break;
                    }
                    i += 1;
                }
                &line[..end]
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// **Built-in primitives go through the shared cache, not a fresh upload per entity.** A
    /// source-shape guard for the same reason as the one below — the loader needs a material
    /// layout to run at all; `AssetManager::primitive_mesh`'s own GPU test covers the sharing.
    #[test]
    fn primitive_keys_are_loaded_through_the_shared_cache() {
        let code: String = code_only(include_str!("asset_loading.rs"))
            .split("#[cfg(test)]")
            .next()
            .unwrap_or("")
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        // The loader builds a key's mesh with `AssetManager::mesh_from_source` (shared, since
        // 2026-11-14, with what builds a `LodGroupDesc` back), and that asks the primitive cache first.
        assert!(code.contains("asset_manager.mesh_from_source(device,queue,texture_bind_group_layout,&mesh_src)"), "the loader no longer builds a key's mesh through `mesh_from_source`");
        let builder: String = code_only(include_str!("asset/mod.rs")).chars().filter(|c| !c.is_whitespace()).collect();
        let body = builder.split("pubfnmesh_from_source(").nth(1).expect("`mesh_from_source` exists");
        let body = body.split("pubfnrebuilds_mesh(").next().unwrap_or(body);
        assert!(body.contains("self.primitive_mesh(device,key)"), "`mesh_from_source` no longer asks the primitive cache");
        for direct in ["AssetManager::create_sphere(", "AssetManager::create_cube(", "AssetManager::create_plane("] {
            assert!(!code.contains(direct) && !body.contains(direct), "a key's mesh is built with `{direct}..)` per entity again");
        }
    }

    /// **This system must consult `MaterialDesc` before building a material from a source.**
    ///
    /// Two records can build one material and they do not carry the same thing: a `MaterialSource`
    /// holds albedo, roughness, metallic, an unlit flag and a texture path; a `MaterialDesc` holds
    /// every field a `Material` has. A scene row saved after descriptions existed carries both, and
    /// this system runs *before* the draw pass that resolves descriptions — so without the check a
    /// user's emissive, transparency, double-sidedness and the rest are rebuilt from the poorer
    /// record and then written back over the richer one by the next save.
    ///
    /// A source-shape guard because the behavioural version needs a GPU: building a `Material`
    /// means building a bind group. Comments are cut first.
    #[test]
    fn a_description_outranks_a_source_when_building_a_material() {
        let code: String = code_only(include_str!("asset_loading.rs"))
            .split("#[cfg(test)]")
            .next()
            .unwrap_or("")
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();

        assert!(
            code.contains("MaterialDesc"),
            "the asset-loading scan no longer looks at `MaterialDesc` — a scene authored after \
             2026-08-19 will have its material rebuilt from the older, poorer `MaterialSource` and \
             then overwritten by it on the next save"
        );
        assert!(
            code.contains("descs.get(id).is_some()"),
            "the check that defers to a description is gone; a `MaterialDesc` mentioned but not \
             consulted is the same defect with a comment on top"
        );
    }
}

/// Lightmap receivers drawn from a `MeshSource`: the mesh read on the CPU, laid out, uploaded.
#[cfg(test)]
mod receiver_tests {
    use super::*;
    use crate::components::LightmapReceiver;

    struct Gpu {
        device: wgpu::Device,
        queue: wgpu::Queue,
        layout: wgpu::BindGroupLayout,
    }

    /// wgpu's noop backend: buffers with no GPU, so these run in CI.
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
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("a noop device");
        let layout = crate::pipeline::layouts::build_layouts(&device).texture;
        Gpu { device, queue, layout }
    }

    /// A one-UV-set glTF quad (no `TEXCOORD_1`) — a model a lightmap could not be read on.
    fn write_one_uv_gltf() -> String {
        let dir = std::env::temp_dir().join(format!("gizmo-gltf-uv1-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let mut bin: Vec<u8> = Vec::new();
        let f32s = |bin: &mut Vec<u8>, v: &[f32]| v.iter().for_each(|x| bin.extend_from_slice(&x.to_le_bytes()));
        for p in [[-1.5f32, -1.5, 0.0], [1.5, -1.5, 0.0], [1.5, 1.5, 0.0], [-1.5, 1.5, 0.0]] {
            f32s(&mut bin, &p);
        }
        for _ in 0..4 {
            f32s(&mut bin, &[0.0, 0.0, 1.0]);
        }
        for uv in [[0.0f32, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]] {
            f32s(&mut bin, &uv);
        }
        for i in [0u16, 1, 2, 0, 2, 3] {
            bin.extend_from_slice(&i.to_le_bytes());
        }
        std::fs::write(dir.join("quad.bin"), &bin).expect("bin");
        let view = |offset: usize, length: usize| format!(r#"{{"buffer":0,"byteOffset":{offset},"byteLength":{length}}}"#);
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],"nodes":[{{"mesh":0,"name":"Quad"}}],
            "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0,"NORMAL":1,"TEXCOORD_0":2}},"indices":3}}]}}],
            "buffers":[{{"uri":"quad.bin","byteLength":{len}}}],
            "bufferViews":[{v0},{v1},{v2},{v3}],
            "accessors":[{{"bufferView":0,"componentType":5126,"count":4,"type":"VEC3","min":[-1.5,-1.5,0.0],"max":[1.5,1.5,0.0]}},
                         {{"bufferView":1,"componentType":5126,"count":4,"type":"VEC3"}},
                         {{"bufferView":2,"componentType":5126,"count":4,"type":"VEC2"}},
                         {{"bufferView":3,"componentType":5123,"count":6,"type":"SCALAR"}}]}}"#,
            len = bin.len(),
            v0 = view(0, 48), v1 = view(48, 48), v2 = view(96, 32), v3 = view(128, 12),
        );
        let path = dir.join("quad.gltf");
        std::fs::write(&path, json).expect("gltf");
        path.to_string_lossy().into_owned()
    }

    /// What `source_vertices` reads on the CPU is what the loader uploads, corner for corner: every
    /// primitive key (bare and with numbers), a glTF sub-mesh, and nothing for a key that names
    /// nothing.
    #[test]
    fn a_sources_vertices_on_the_cpu_are_the_ones_uploaded() {
        let gpu = gpu();
        let mut assets = AssetManager::new();
        let model = write_one_uv_gltf();
        let white = assets.create_white_texture(&gpu.device, &gpu.queue, &gpu.layout);
        assets.load_gltf_scene(&gpu.device, &gpu.queue, &gpu.layout, white, &model).expect("the quad imports");
        let mut keys: Vec<String> = AssetManager::PRIMITIVE_MESH_KEYS.iter().map(|k| k.to_string()).collect();
        keys.extend(
            ["plane_3", "plane_2_4", "circle_1_12", "sphere_0.5_8_12", "cylinder_1_2_8", "cone_1_2_8", "torus_1_0.3_8_12", "capsule_0.5_1_4_8", "sprite_quad_2_1", "tetrahedron_1", "conical_frustum_1_0.5_2_8"]
                .map(String::from),
        );
        // The key as the import spells it: the path normalised (`/`, not Windows' `\`), as every key a
        // scene holds came from the import.
        keys.push(format!("gltf_mesh_{}_Quad_p0", AssetManager::normalize_path(&model)));
        for key in &keys {
            let uploaded = assets.mesh_from_source(&gpu.device, &gpu.queue, &gpu.layout, key);
            assert_ne!(uploaded.source, "__async_loading__", "{key} loaded the placeholder, not the mesh");
            let read = assets.source_vertices(key).unwrap_or_else(|| panic!("{key} read nothing"));
            let positions: Vec<gizmo_math::Vec3> = read.iter().map(|v| gizmo_math::Vec3::from(v.position)).collect();
            assert_eq!(*uploaded.cpu_vertices, positions, "{key}");
        }
        assert!(assets.source_vertices("no_such_mesh.obj").is_none());
    }

    /// A receiver drawn from a primitive or from a glTF with no second UV set draws a copy laid out
    /// for its lightmap: before its first mesh, and once a plain one is there (the receiver added
    /// after). The UVs are the ones `receiver_vertices` gives — what a bake reads.
    #[test]
    fn a_receiver_is_drawn_with_the_uvs_its_lightmap_is_baked_at() {
        let gpu = gpu();
        let mut assets = AssetManager::new();
        let model = write_one_uv_gltf();
        let white = assets.create_white_texture(&gpu.device, &gpu.queue, &gpu.layout);
        assets.load_gltf_scene(&gpu.device, &gpu.queue, &gpu.layout, white, &model).expect("the quad imports");
        let gltf_key = format!("gltf_mesh_{}_Quad_p0", AssetManager::normalize_path(&model));

        let mut world = World::new();
        let cube = world.spawn_bundle((MeshSource("standard_cube".into()), LightmapReceiver::new(32, 32)));
        let quad = world.spawn_bundle((MeshSource(gltf_key.clone()),));
        let plain = world.spawn_bundle((MeshSource("sphere".into()),));
        run_asset_loading_system(&mut world, &gpu.device, &gpu.queue, &gpu.layout, &mut assets);
        assert!(world.get::<Mesh>(quad).unwrap().cpu_lightmap.is_none(), "a model that is not a receiver was laid out");
        assert!(world.get::<Mesh>(plain).unwrap().cpu_lightmap.is_none());

        // Receivers added after their plain mesh was loaded.
        world.add_component(quad, LightmapReceiver::new(64, 64));
        world.add_component(plain, LightmapReceiver::new(16, 16));
        run_asset_loading_system(&mut world, &gpu.device, &gpu.queue, &gpu.layout, &mut assets);

        for (entity, key, resolution) in [(cube, "standard_cube", 32), (quad, gltf_key.as_str(), 64), (plain, "sphere", 16)] {
            let mesh = world.get::<Mesh>(entity).unwrap();
            let drawn = mesh.lightmap_vertices().unwrap_or_else(|| panic!("{key}: drawn without a second UV set"));
            let baked = AssetManager::receiver_vertices(assets.source_vertices(key), resolution).unwrap();
            let uv = |v: &[crate::gpu_types::Vertex]| v.iter().map(|v| v.lightmap_uv).collect::<Vec<_>>();
            assert_eq!(uv(&drawn), uv(&baked), "{key}: drawn at other UVs than it bakes at");
            assert!(drawn.iter().any(|v| v.lightmap_uv != [0.0, 0.0]));
        }
        // Once laid out, left alone: the same upload the next frame.
        let before = world.get::<Mesh>(cube).unwrap().vbuf.clone();
        run_asset_loading_system(&mut world, &gpu.device, &gpu.queue, &gpu.layout, &mut assets);
        assert!(std::sync::Arc::ptr_eq(&before, &world.get::<Mesh>(cube).unwrap().vbuf));
    }
}
