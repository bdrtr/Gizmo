use super::physics::*;
use crate::core::World;
use crate::math::{Mat4, Vec3};
use crate::renderer::{
    components::{Camera, Material, Mesh, MeshRenderer},
    Renderer,
};
use bytemuck;
use wgpu;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WireframeConfig {
    pub global: bool,
}

#[derive(Default)]
pub struct RenderCache {
    pub(crate) batches: std::collections::HashMap<BatchKey, BatchData>,
    pub instances: Vec<crate::renderer::gpu_types::InstanceRaw>,
    pub draw_items: Vec<DrawItem>,
}

thread_local! {
    static RENDER_CACHE: std::cell::RefCell<RenderCache> = std::cell::RefCell::new(RenderCache::default());
}

pub fn clear_render_cache() {
    RENDER_CACHE.with(|rc| {
        let mut cache = rc.borrow_mut();
        cache.batches.clear();
        cache.instances.clear();
        cache.draw_items.clear();
    });
}

#[derive(Debug, Clone)]
pub struct DrawItem {
    vbuf: std::sync::Arc<wgpu::Buffer>,
    vertex_count: u32,
    bind_group: std::sync::Arc<wgpu::BindGroup>,
    unlit: bool,
    is_skybox: bool,
    skeleton_bind_group: Option<std::sync::Arc<wgpu::BindGroup>>,
    is_transparent: bool,
    first_instance: u32,
    /// Total instances in this batch's contiguous range: camera-visible ones FIRST,
    /// then shadow-only casters (outside the camera frustum but inside a cascade's light
    /// frustum). Shadow passes draw the whole range; main passes draw only `camera_count`.
    instance_count: u32,
    /// Number of leading instances visible to the CAMERA (== the old camera-culled set).
    camera_count: u32,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct BatchKey {
    vbuf_id: usize,
    mat_id: usize,
    skeleton_id: Option<usize>,
}

pub(crate) struct BatchData {
    vbuf: std::sync::Arc<wgpu::Buffer>,
    bind_group: std::sync::Arc<wgpu::BindGroup>,
    vertex_count: u32,
    unlit: bool,
    is_skybox: bool,
    skeleton_bind_group: Option<std::sync::Arc<wgpu::BindGroup>>,
    is_transparent: bool,
    instances: Vec<crate::renderer::gpu_types::InstanceRaw>,
    /// Casters outside the camera frustum but inside a shadow cascade's light frustum —
    /// must be drawn into the shadow maps so off-screen objects still cast visible shadows.
    shadow_instances: Vec<crate::renderer::gpu_types::InstanceRaw>,
}

/// Bevy'nin DefaultPlugins davranisini taklit eden, sadece modelleri
/// isiklandirip hizlica ekrana basmaya yarayan kutudan cikmis Render Motoru.
/// Yeni acilan `tut` gibi bos projelerde yuzlerce satir kod yazmamak icin kullanilir.
#[tracing::instrument(skip_all, name = "render_system")]
pub fn default_render_pass(
    world: &mut World,
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    renderer: &mut Renderer,
) {
    // Update post process parameters dynamically from renderer settings!
    renderer.update_post_process(
        &renderer.queue,
        crate::renderer::gpu_types::PostProcessUniforms {
            bloom_intensity: renderer.bloom_intensity,
            bloom_threshold: renderer.bloom_threshold,
            exposure: renderer.exposure,
            chromatic_aberration: renderer.chromatic_aberration,
            vignette_intensity: 0.25,
            film_grain_intensity: renderer.film_grain_intensity,
            dof_focus_dist: renderer.dof_focus_dist,
            dof_focus_range: renderer.dof_focus_range,
            dof_blur_size: if renderer.dof_enabled { renderer.dof_blur_size } else { 0.0 },
            _padding: [0.0; 3],
        },
    );

    let aspect = if renderer.size.height > 0 {
        renderer.size.width as f32 / renderer.size.height as f32
    } else {
        1.0
    };
    let mut proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, aspect, 0.1, 2000.0);
    let mut view_mat = Mat4::from_translation(Vec3::ZERO);
    let mut cam_pos = Vec3::ZERO;
    let mut cam_forward = Vec3::new(0.0, 0.0, -1.0);

    // TODO: Bütün nesnelerin (özellikle kamera ve çizilecek objelerin) global matrix'leri
    // bu pass çağrılmadan hemen önce bir `update_transforms(world)` sistemiyle güncellenmiş olmalıdır.

    // ECS veri GPU'ya basılır ve GPU verisi ECS'ye alınır
    gpu_physics_submit_system(world, renderer);
    gpu_physics_readback_system(world, renderer);

    let mut cam_exposure = 1.0;
    // Shadow cascades must follow the ACTIVE camera's near/far/fov, not hardcoded values
    // (otherwise splits/cascade matrices are wrong for any non-default camera).
    let mut cam_near = 0.1f32;
    let mut cam_far = 2000.0f32;
    let mut cam_fov = std::f32::consts::FRAC_PI_4;

    // KAMERALARI BUL VE MATRIX YARAT
    let cameras = world.borrow::<Camera>();
    let transforms = world.borrow::<gizmo_physics_core::components::GlobalTransform>();
    {
        // Pick the camera flagged `primary` — the convention maintained by
        // `spawn_camera`/`CameraBundle` (which keep a single primary) and used by
        // the audio listener. Fall back to the first camera if none is marked.
        // This makes selection deterministic instead of depending on the
        // (unstable) ECS iteration order.
        let active_cam = cameras
            .iter()
            .find(|(_, c)| c.primary)
            .or_else(|| cameras.iter().next())
            .map(|(id, _)| id);
        if let Some(active_cam) = active_cam {
            if let (Some(cam), Some(trans)) = (cameras.get(active_cam), transforms.get(active_cam))
            {
                let (_, _, pos) = trans.matrix.to_scale_rotation_translation();
                proj = cam.get_projection(aspect);
                view_mat = cam.get_view(pos);
                cam_pos = pos;
                cam_forward = cam.get_front();
                cam_exposure = cam.exposure;
                cam_near = cam.near;
                cam_far = cam.far;
                cam_fov = cam.fov;
            }
        }
    }

    // Save unjittered projection before applying TAA offset (needed for reprojection next frame).
    let unjittered_proj = proj;

    // ── TAA Halton jitter: subpixel offset applied via z-column of projection ──
    if let Some(ref taa) = renderer.taa {
        if taa.enabled {
            let jp = crate::renderer::taa::TaaState::get_jitter(taa.frame_index);
            // Convert pixel jitter [−0.5, 0.5] to NDC offset (2 / viewport_size per axis)
            let jx = jp[0] * 2.0 / renderer.size.width as f32;
            let jy = jp[1] * 2.0 / renderer.size.height as f32;
            // Adding jitter to NDC.x requires: new_clip.x = clip.x - jx*vz
            // ↔ subtract jx from proj.z_axis.x (the M[0][2] element, row0·col2)
            proj.z_axis.x -= jx;
            proj.z_axis.y -= jy;
        }
    }

    let view_proj = proj * view_mat; // jittered — used for SceneUniforms
    let unjittered_view_proj = unjittered_proj * view_mat; // clean    — stored in TaaState for next frame

    let mut sun_dir = gizmo_math::Vec3::new(0.0, -1.0, 0.0);
    let mut sun_col = gizmo_math::Vec4::new(0.0, 0.0, 0.0, 0.0); // W=0.0 means NO SUN by default!
    if let Some(q) = world.query::<(
        &crate::renderer::components::DirectionalLight,
        &gizmo_physics_core::components::GlobalTransform,
    )>() {
        for (_id, (light, transform)) in q.iter() {
            if light.role == crate::renderer::components::LightRole::Sun {
                let (_, rot, _) = transform.matrix.to_scale_rotation_translation();
                sun_dir = rot
                    .mul_vec3(gizmo_math::Vec3::new(0.0, 0.0, -1.0))
                    .normalize();
                sun_col = gizmo_math::Vec4::new(
                    light.color.x,
                    light.color.y,
                    light.color.z,
                    light.intensity,
                );
                break;
            }
        }
    }

    // Derive splits from the actual camera near/far (was hardcoded [20,80,250,2000],
    // which mismatched any camera whose far ≠ 2000 → fragments past the last split fell
    // into a cascade whose ortho matrix didn't cover them). Mirrors the studio path.
    let cascade_splits = crate::renderer::cascade_split_distances(cam_near, cam_far, 0.75);
    let cascade_vp = crate::renderer::directional_cascade_view_projs(
        cam_pos,
        cam_forward,
        aspect,
        cam_fov,
        cam_near,
        &cascade_splits,
        sun_dir,
        crate::renderer::SHADOW_MAP_RES,
    );
    let light_view_projs: [[[f32; 4]; 4]; 4] = cascade_vp.map(|m| m.to_cols_array_2d());

    // Dinamik Işıkları Bul
    let mut lights_data = [crate::renderer::gpu_types::LightData {
        position: [0.0; 4],
        color: [0.0; 4],
        direction: [0.0, -1.0, 0.0, 0.0],
        params: [0.0; 4],
    }; 10];
    let mut num_lights = 0;

    if let Some(q) = world.query::<(
        &crate::renderer::components::PointLight,
        &gizmo_physics_core::components::GlobalTransform,
    )>() {
        for (_id, (light, transform)) in q.iter() {
            if num_lights >= 10 {
                break;
            }
            let (_, _, pos) = transform.matrix.to_scale_rotation_translation();
            lights_data[num_lights as usize] = crate::renderer::gpu_types::LightData {
                position: [pos.x, pos.y, pos.z, light.intensity],
                color: [light.color.x, light.color.y, light.color.z, light.radius],
                direction: [0.0, -1.0, 0.0, 0.0],
                params: [0.0, 0.0, 0.0, 0.0], // y = 0 means PointLight
            };
            num_lights += 1;
        }
    }

    if let Some(q) = world.query::<(
        &crate::renderer::components::SpotLight,
        &gizmo_physics_core::components::GlobalTransform,
    )>() {
        for (_id, (light, transform)) in q.iter() {
            if num_lights >= 10 {
                break;
            }
            let (_, rot, pos) = transform.matrix.to_scale_rotation_translation();
            let dir = rot
                .mul_vec3(gizmo_math::Vec3::new(0.0, 0.0, -1.0))
                .normalize();
            lights_data[num_lights as usize] = crate::renderer::gpu_types::LightData {
                position: [pos.x, pos.y, pos.z, light.intensity],
                color: [light.color.x, light.color.y, light.color.z, light.radius],
                direction: [dir.x, dir.y, dir.z, light.inner_angle],
                params: [light.outer_angle, 1.0, 0.0, 0.0], // y = 1 means SpotLight
            };
            num_lights += 1;
        }
    }

    #[allow(unused_assignments)]
    let mut point_light_view_projs = [gizmo_math::Mat4::IDENTITY; 6];
    if renderer.point_shadows_enabled {
        if let Some(q) = world.query::<(
            &crate::renderer::components::PointLight,
            &gizmo_physics_core::components::GlobalTransform,
        )>() {
            if let Some((_id, (_light, transform))) = q.iter().next() {
                let (_, _, pos) = transform.matrix.to_scale_rotation_translation();
                let proj = gizmo_math::Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 1.0, 0.1, 100.0);
                point_light_view_projs = [
                    proj * gizmo_math::Mat4::look_to_rh(pos, gizmo_math::Vec3::X, -gizmo_math::Vec3::Y),
                    proj * gizmo_math::Mat4::look_to_rh(pos, gizmo_math::Vec3::NEG_X, -gizmo_math::Vec3::Y),
                    proj * gizmo_math::Mat4::look_to_rh(pos, gizmo_math::Vec3::Y, gizmo_math::Vec3::Z),
                    proj * gizmo_math::Mat4::look_to_rh(pos, gizmo_math::Vec3::NEG_Y, gizmo_math::Vec3::NEG_Z),
                    proj * gizmo_math::Mat4::look_to_rh(pos, gizmo_math::Vec3::Z, -gizmo_math::Vec3::Y),
                    proj * gizmo_math::Mat4::look_to_rh(pos, gizmo_math::Vec3::NEG_Z, -gizmo_math::Vec3::Y),
                ];
                
                for (i, view_proj) in point_light_view_projs.iter().enumerate() {
                    renderer.queue.write_buffer(
                        &renderer.scene.point_shadow_uniform_buffers[i],
                        0,
                        bytemuck::bytes_of(&crate::renderer::gpu_types::ShadowVsUniform {
                            light_view_proj: view_proj.to_cols_array_2d(),
                        }),
                    );
                }
            }
        }
    }


    // Elapsed time drives fluid caustics/wave animation in fluid_composite.wgsl
    // (it reads cascade_params.z); this slot was hardcoded to 0.0 → frozen water.
    let elapsed_time = world
        .get_resource::<gizmo_core::time::Time>()
        .map(|t| t.elapsed() as f32)
        .unwrap_or(0.0);
    let scene_uniform_data = crate::renderer::gpu_types::SceneUniforms {
        view_proj: view_proj.to_cols_array_2d(),
        camera_pos: [cam_pos.x, cam_pos.y, cam_pos.z, 1.0],
        sun_direction: [sun_dir.x, sun_dir.y, sun_dir.z, 1.0],
        sun_color: [sun_col.x, sun_col.y, sun_col.z, sun_col.w],
        lights: lights_data,
        light_view_proj: light_view_projs,
        cascade_splits,
        camera_forward: [cam_forward.x, cam_forward.y, cam_forward.z, 0.0],
        cascade_params: [0.1, 1.0 / crate::renderer::SHADOW_MAP_RES as f32, elapsed_time, 0.0],
        num_lights,
        exposure: cam_exposure,
        _pre_align_pad: [0; 2],
        _align_pad: [0; 3],
        environment_blend_t: renderer.environment_blend_t,
        environment_preset: renderer.environment_preset,
        point_shadows_enabled: renderer.point_shadows_enabled as u32,
        environment_preset_2: renderer.environment_preset_2,
        shading_mode: renderer.shading_mode,
    };
    renderer.queue.write_buffer(
        &renderer.scene.global_uniform_buffer,
        0,
        bytemuck::cast_slice(&[scene_uniform_data]),
    );
    for (i, light_view_proj) in light_view_projs.iter().enumerate() {
        renderer.queue.write_buffer(
            &renderer.scene.shadow_cascade_uniform_buffers[i],
            0,
            bytemuck::bytes_of(&crate::renderer::gpu_types::ShadowVsUniform {
                light_view_proj: *light_view_proj,
            }),
        );
    }

    // Upload TAA params (prev_vp from last frame, current jitter, blend alpha)
    if let Some(ref mut taa) = renderer.taa {
        if taa.enabled {
            let jp = crate::renderer::taa::TaaState::get_jitter(taa.frame_index);
            let jx = jp[0] * 2.0 / renderer.size.width as f32;
            let jy = jp[1] * 2.0 / renderer.size.height as f32;
            let alpha = if taa.frame_index == 0 { 1.0f32 } else { 0.1f32 };
            taa.update_params(&renderer.queue, [jx, jy], alpha);
            taa.store_prev_vp(unjittered_view_proj.to_cols_array_2d());
        }
    }

    // ... inside default_render_pass ...
    // ... before line 205 ...
    let renderers = world.borrow::<MeshRenderer>();

    // Get or create RenderCache
    let frustum = crate::math::Frustum::from_matrix(&unjittered_view_proj);
    // Per-cascade LIGHT frusta — shadow casters are culled against these, NOT the camera
    // frustum, so objects outside the view that cast shadows INTO it aren't dropped.
    let cascade_frusta: [crate::math::Frustum; 4] =
        cascade_vp.map(|m| crate::math::Frustum::from_matrix(&m));

    let (draw_items, uploaded_instances) = RENDER_CACHE.with(|rc| {
        let mut cache = rc.borrow_mut();
        
        // Clear instances but keep allocations
        for batch in cache.batches.values_mut() {
            batch.instances.clear();
        }
        cache.instances.clear();
        cache.draw_items.clear();

        let pooled_storage = world.borrow::<gizmo_core::pool::Pooled>();
        
        macro_rules! process_mesh {
            ($e:expr, $mesh:expr, $trans:expr, $mat:expr, $skeleton:expr) => {
                if renderers.get($e).is_none() {
                    continue;
                }
                
                // Pooled (havuzda pasif) nesneleri render etme
                if pooled_storage.get($e).is_some() {
                    continue;
                }

                let center_mat = Mat4::from_translation($mesh.center_offset);
                let model = $trans.matrix * center_mat;

                // CPU Frustum Culling
                let local_cx = ($mesh.bounds.min.x + $mesh.bounds.max.x) * 0.5;
                let local_cy = ($mesh.bounds.min.y + $mesh.bounds.max.y) * 0.5;
                let local_cz = ($mesh.bounds.min.z + $mesh.bounds.max.z) * 0.5;
                let world_c = model.transform_point3(Vec3::new(local_cx, local_cy, local_cz));
                let hx = ($mesh.bounds.max.x - $mesh.bounds.min.x) * 0.5;
                let hy = ($mesh.bounds.max.y - $mesh.bounds.min.y) * 0.5;
                let hz = ($mesh.bounds.max.z - $mesh.bounds.min.z) * 0.5;
                let local_r = (hx * hx + hy * hy + hz * hz).sqrt();
                let sx = model.x_axis.truncate().length();
                let sy = model.y_axis.truncate().length();
                let sz = model.z_axis.truncate().length();
                let world_r = local_r * sx.max(sy).max(sz);

                // Camera culling decides the MAIN passes (unchanged). But a shadow CASTER
                // outside the camera frustum can still cast a shadow into view, so keep it
                // if it falls in any cascade's LIGHT frustum — drawn into the shadow maps
                // only (main passes use `camera_count`, shadow passes the full range).
                let camera_visible = frustum.intersects_sphere(world_c, world_r);
                let is_caster = !(matches!(
                    $mat.material_type,
                    crate::renderer::components::MaterialType::Unlit
                        | crate::renderer::components::MaterialType::Skybox
                ) || $mat.is_transparent
                    || $mat.albedo.w < 0.99);
                if !camera_visible {
                    if !is_caster
                        || !cascade_frusta
                            .iter()
                            .any(|f| f.intersects_sphere(world_c, world_r))
                    {
                        continue;
                    }
                }

                // Auto-LOD (Level of Detail) Seçimi
                let dist_to_cam = (world_c - cam_pos).length();
                let use_lod1 = if !$mesh.lod_vbufs.is_empty() {
                    dist_to_cam > world_r * 15.0 // Nesne boyutuna göre uzaklaştıkça LOD1'e geç (örneğin 2m çapında bir nesne 30m uzaktayken geç)
                } else {
                    false
                };

                let active_vbuf = if use_lod1 {
                    $mesh.lod_vbufs[0].clone()
                } else {
                    $mesh.vbuf.clone()
                };
                let active_vertex_count = if use_lod1 {
                    $mesh.lod_vertex_counts[0]
                } else {
                    $mesh.vertex_count
                };

                let packed_params = (($mat.anisotropy * 1000.0).floor() + 1000.0 * ($mat.clear_coat * 1000.0).floor() + 1000000.0 * ($mat.subsurface * 100.0).floor()) as f32;

                let instance_data = crate::renderer::gpu_types::InstanceRaw {
                    model: model.to_cols_array_2d(),
                    albedo_color: [$mat.albedo.x, $mat.albedo.y, $mat.albedo.z, $mat.albedo.w],
                    roughness: $mat.roughness,
                    metallic: $mat.metallic,
                    unlit: match $mat.material_type {
                        crate::renderer::components::MaterialType::Skybox => 2.0,
                        crate::renderer::components::MaterialType::Unlit => 1.0,
                        _ => 0.0,
                    },
                    _padding: packed_params,
                };
                let skel_bg = $skeleton.map(|s: &crate::renderer::components::Skeleton| s.bind_group.clone());
                
                let key = BatchKey {
                    vbuf_id: std::sync::Arc::as_ptr(&active_vbuf) as usize,
                    mat_id: std::sync::Arc::as_ptr(&$mat.bind_group) as usize,
                    skeleton_id: skel_bg.as_ref().map(|bg| std::sync::Arc::as_ptr(bg) as usize),
                };

                let batch = cache.batches.entry(key).or_insert_with(|| BatchData {
                    vbuf: active_vbuf.clone(),
                    bind_group: $mat.bind_group.clone(),
                    vertex_count: active_vertex_count,
                    unlit: $mat.material_type == crate::renderer::components::MaterialType::Unlit
                        || $mat.material_type == crate::renderer::components::MaterialType::Skybox,
                    is_skybox: $mat.material_type == crate::renderer::components::MaterialType::Skybox,
                    skeleton_bind_group: skel_bg,
                    is_transparent: $mat.is_transparent || $mat.albedo.w < 0.99,
                    instances: Vec::new(),
                    shadow_instances: Vec::new(),
                });
                if camera_visible {
                    batch.instances.push(instance_data);
                } else {
                    // Off-screen caster kept above for shadow maps only.
                    batch.shadow_instances.push(instance_data);
                }
            };
        }

        let skeletons = world.borrow::<crate::renderer::components::Skeleton>();

        if let Some(mut q) = world.query::<(&Mesh, &gizmo_physics_core::components::GlobalTransform, &Material)>() {
            for (e, (mesh, trans, mat)) in q.iter_mut() {
                process_mesh!(e, mesh, trans, mat, skeletons.get(e));
            }
        }
        
        let meshes = world.try_get_resource::<gizmo_core::asset::Assets<Mesh>>().ok();
        let materials = world.try_get_resource::<gizmo_core::asset::Assets<Material>>().ok();
        
        if let (Some(meshes), Some(materials)) = (meshes, materials) {
            if let Some(mut q) = world.query::<(&gizmo_core::asset::Handle<Mesh>, &gizmo_physics_core::components::GlobalTransform, &gizmo_core::asset::Handle<Material>)>() {
                for (e, (h_mesh, trans, h_mat)) in q.iter_mut() {
                    if let (Some(mesh), Some(mat)) = (meshes.get(h_mesh), materials.get(h_mat)) {
                        process_mesh!(e, mesh, trans, mat, skeletons.get(e));
                    }
                }
            }
        }
        
        let mut local_instances: Vec<crate::renderer::gpu_types::InstanceRaw> = std::mem::take(&mut cache.instances);
        let mut local_draw_items: Vec<DrawItem> = std::mem::take(&mut cache.draw_items);

        for batch in cache.batches.values() {
            if batch.instances.is_empty() && batch.shadow_instances.is_empty() {
                continue;
            }
            let first_instance = local_instances.len() as u32;
            // Camera-visible instances FIRST (so `camera_count` == the old culled set),
            // then shadow-only casters — both contiguous under one DrawItem range.
            let camera_count = batch.instances.len() as u32;
            local_instances.extend(&batch.instances);
            local_instances.extend(&batch.shadow_instances);
            let instance_count = camera_count + batch.shadow_instances.len() as u32;

            local_draw_items.push(DrawItem {
                vbuf: batch.vbuf.clone(),
                vertex_count: batch.vertex_count,
                bind_group: batch.bind_group.clone(),
                unlit: batch.unlit,
                is_skybox: batch.is_skybox,
                skeleton_bind_group: batch.skeleton_bind_group.clone(),
                is_transparent: batch.is_transparent,
                first_instance,
                instance_count,
                camera_count,
            });
        }
        
        cache.instances = local_instances;
        cache.draw_items = local_draw_items;

        // Instance limiti kontrolü (Taşmaları önlemek için capaciteyi zorla)
        let max_instances = renderer.scene.instance_capacity;
        let instances_slice = if cache.instances.len() > max_instances {
            &cache.instances[..max_instances]
        } else {
            &cache.instances
        };

        if !instances_slice.is_empty() {
            renderer.queue.write_buffer(
                &renderer.scene.instance_buffer,
                0,
                bytemuck::cast_slice(instances_slice),
            );
        }
        
        // Pass draw_items to rendering logic by cloning the small struct (Arc clones are cheap).
        // Also return how many instances actually made it into the GPU buffer so draw ranges
        // can be clamped (shadow casters increase the count → guard against capacity truncation).
        (cache.draw_items.clone(), instances_slice.len() as u32)
    });
    // CPU Batched Instancing replaces GPU cull for draw_items

    if let Some(physics) = &renderer.gpu_physics {
        // Her frame başında sıradaki state'i çekmek için WGPU CommandEncoder'a asenkron mapping iste.
        physics.request_readback(encoder);

        physics.compute_pass(encoder);
        physics.debug_compute_pass(encoder);
        physics.cull_pass(encoder, &renderer.scene.global_bind_group);
    }

    // Compute LOD (Level of Detail) Scaling
    let fluid_pos = Vec3::new(0.0, 5.0, 0.0);
    let dist_to_fluid = (cam_pos - fluid_pos).length();
    let fluid_lod = if dist_to_fluid < 40.0 {
        1.0
    } else if dist_to_fluid < 80.0 {
        0.5
    } else if dist_to_fluid < 150.0 {
        0.1
    } else {
        0.0
    };

    let dist_to_origin = cam_pos.length();
    let particle_lod = if dist_to_origin < 50.0 {
        1.0
    } else if dist_to_origin < 100.0 {
        0.5
    } else if dist_to_origin < 200.0 {
        0.1
    } else {
        0.0
    };

    // Gpu Fluid Processing
    if let Some(fluid) = &renderer.gpu_fluid {
        let active_fluid = (fluid.num_particles as f32 * fluid_lod) as u32;
        fluid.compute_pass(encoder, &renderer.queue, true, active_fluid);
    }

    // Gpu Particles Processing
    if let Some(particles) = &renderer.gpu_particles {
        let active_parts = (particles.max_particles as f32 * particle_lod) as u32;
        let dt = world
            .get_resource::<gizmo_core::time::Time>()
            .map(|t| t.dt())
            .unwrap_or(0.016);
        particles.update_params(&renderer.queue, dt); // Scale based on time_scale
        particles.compute_pass(encoder, active_parts);
    }

    // GPU cull pass removed since we use CPU instancing

    // Resize deferred G-buffers if window changed; resize SSAO + TAA to match
    if let Some(ref mut def) = renderer.deferred {
        def.resize(&renderer.device, renderer.size.width, renderer.size.height);
    }
    {
        let w = renderer.size.width;
        let h = renderer.size.height;
        if let (Some(ssao), Some(def)) = (&mut renderer.ssao, &renderer.deferred) {
            if ssao.width != w || ssao.height != h {
                ssao.resize(&renderer.device, def, w, h);
            }
        }
        if let (Some(ssr), Some(def)) = (&mut renderer.ssr, &renderer.deferred) {
            if ssr.width != w || ssr.height != h {
                ssr.resize(&renderer.device, def, &renderer.post.hdr_texture_view, w, h);
            }
        }
        if let (Some(volumetric), Some(def)) = (&mut renderer.volumetric, &renderer.deferred) {
            if volumetric.width != w || volumetric.height != h {
                volumetric.resize(&renderer.device, def, w, h);
            }
        }
    }
    {
        let w = renderer.size.width;
        let h = renderer.size.height;
        if let (Some(taa), Some(def)) = (&mut renderer.taa, &renderer.deferred) {
            if taa.width != w || taa.height != h {
                taa.resize(
                    &renderer.device,
                    &renderer.post.hdr_texture_view,
                    &def.world_position_view,
                    w,
                    h,
                );
            }
        }
    }

    passes::record_shadow_passes(encoder, renderer, &draw_items, uploaded_instances);
    passes::record_deferred_geometry(encoder, renderer, world, &draw_items, uploaded_instances);
    passes::record_ssao(encoder, renderer);
    passes::record_forward_and_fluid(
        encoder, renderer, world, &draw_items, uploaded_instances, particle_lod, fluid_lod,
    );
    passes::record_screen_space_effects(encoder, renderer);
    passes::record_taa_and_overlays(encoder, renderer, world);

    renderer.run_post_processing(encoder, view);
}

// ============================================================
//  RenderContext Kolaylık Metodu
//  `ctx.default_render(world)` ile varsayılan pipeline çalışır.
// ============================================================

/// `RenderContext` üzerine eklenen kolaylık metodları.
/// `use gizmo::prelude::*;` ile otomatik olarak dahil edilir.
pub trait RenderContextExt {
    /// Motorun varsayılan render pipeline'ını çalıştırır.
    /// Deferred rendering, gölgeler, SSAO, SSR, TAA ve post-processing dahildir.
    ///
    /// ```ignore
    /// fn render(world: &mut World, _state: &GameState, ctx: &mut RenderContext) {
    ///     ctx.disable_gpu_compute();
    ///     ctx.default_render(world);
    /// }
    /// ```
    fn default_render(&mut self, world: &mut crate::core::World);
}

impl<'a> RenderContextExt for crate::renderer::RenderContext<'a> {
    fn default_render(&mut self, world: &mut crate::core::World) {
        let (encoder, view, renderer) = self.parts_mut();
        default_render_pass(world, encoder, view, renderer);
    }
}

mod passes;

/// Golden render test: drive the REAL [`default_render_pass`] over a minimal scene
/// (one lit cube + a camera + a sun) into an offscreen target and assert that geometry
/// actually reaches the framebuffer — a sizeable central region must differ from the
/// background. Unlike the renderer's clear-colour readback test, this exercises the full
/// pipeline (cull → batch → shadow/deferred/forward → post), so a regression in the
/// pass-recording split (or any pass) that drops geometry fails here instead of slipping
/// past CI. Needs a GPU adapter; runs in GPU-backed CI/dev.
#[cfg(test)]
mod golden_render_tests {
    use super::default_render_pass;
    use crate::bundles::{CameraBundle, DirectionalLightBundle};
    use crate::core::World;
    use crate::math::{Vec3, Vec4};
    use crate::physics::components::{GlobalTransform, Transform};
    use crate::renderer::asset::AssetManager;
    use crate::renderer::components::{Material, MeshRenderer};
    use crate::renderer::Renderer;

    #[test]
    fn default_render_pass_draws_a_cube_distinct_from_background() {
        if !pollster::block_on(Renderer::headless_adapter_available()) {
            eprintln!(
                "skipping default_render_pass_draws_a_cube_distinct_from_background: \
                 no GPU adapter available (headless render requires a GPU)"
            );
            return;
        }
        pollster::block_on(async {
            const W: u32 = 128;
            const H: u32 = 128;
            const BPP: u32 = 4; // every surface format used here is 4 bytes/pixel

            let mut renderer = Renderer::new_headless(W, H, None).await;
            let mut asset_manager = AssetManager::new();
            let mut world = World::new();

            // --- one cube at the origin (create_cube spans -1..1 → size 2) ---
            let mesh = AssetManager::create_cube(&renderer.device);
            let tex = asset_manager.create_white_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            );
            let mat = Material::new(tex).with_pbr(Vec4::new(0.9, 0.15, 0.15, 1.0), 0.0, 1.0);
            let cube = world.spawn();
            world.add_component(cube, Transform::new(Vec3::ZERO));
            world.add_component(cube, GlobalTransform::default()); // identity → cube at origin
            world.add_component(cube, mesh);
            world.add_component(cube, mat);
            world.add_component(cube, MeshRenderer::new());

            // --- camera on -X looking toward +X (yaw 0 → front = +X), framing the cube ---
            world.spawn_bundle(CameraBundle {
                position: Vec3::new(-6.0, 0.0, 0.0),
                yaw: 0.0,
                pitch: 0.0,
                primary: true,
                ..Default::default()
            });
            // --- a sun so the cube is lit (role = Sun by default) ---
            world.spawn_bundle(DirectionalLightBundle::default());

            // --- run the REAL pipeline into an offscreen target ---
            let format = renderer.config.format;
            let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("golden-target"),
                size: wgpu::Extent3d {
                    width: W,
                    height: H,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = target.create_view(&wgpu::TextureViewDescriptor::default());
            let mut encoder = renderer
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

            default_render_pass(&mut world, &mut encoder, &view, &mut renderer);

            // --- copy the result out (W*BPP = 512 → already 256-aligned) ---
            let staging = renderer.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("golden-readback"),
                size: (W * H * BPP) as u64,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &target,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(W * BPP),
                        rows_per_image: Some(H),
                    },
                },
                wgpu::Extent3d {
                    width: W,
                    height: H,
                    depth_or_array_layers: 1,
                },
            );
            renderer.queue.submit(Some(encoder.finish()));

            let slice = staging.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |v| tx.send(v).unwrap());
            let _ = renderer.device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            });
            rx.recv().unwrap().unwrap();
            let data = slice.get_mapped_range();

            let px = |x: u32, y: u32| -> [u8; 4] {
                let i = ((y * W + x) * BPP) as usize;
                [data[i], data[i + 1], data[i + 2], data[i + 3]]
            };
            let background = px(2, 2); // a corner — the cube never reaches here
            let centre = px(W / 2, H / 2);
            assert_ne!(
                centre, background,
                "centre pixel equals the corner/background — default_render_pass drew no geometry"
            );

            // the cube should cover a sizeable central region, not a stray pixel
            let mut differing = 0u32;
            for y in 0..H {
                for x in 0..W {
                    if px(x, y) != background {
                        differing += 1;
                    }
                }
            }
            let frac = differing as f32 / (W * H) as f32;
            assert!(
                frac > 0.05,
                "only {:.1}% of pixels differ from the background; the lit cube should fill a \
                 sizeable central region (regression dropping geometry?)",
                frac * 100.0
            );
        });
    }
}
