//! # BeamNG tarzı FEM yumuşak-gövde demosu (temiz sürüm)
//!
//! GPU üzerinde tetrahedral **FEM** (sonlu-eleman) simülasyonu ile deforme olan bir "araba
//! gövdesi" zemine düşer ve ezilir. Fizik TAMAMEN özel bir GPU compute boru hattında koşar —
//! motorun rigid-body `PhysicsPlugin`'i burada YOK. Bu yüzden bilinçli idiom seçimleri:
//!
//!   * **`Prefab` / `DespawnAfter` / `DespawnBelowY` YOK** — tekrar eden kutu-collider'lı rigid
//!     nesne yok, ömür-döngüsü çalıştıran bir fizik/lifetime schedule de kayıtlı değil. Bu
//!     komponentleri eklemek işlevsiz olurdu (statik sahne). Sahne varlıkları (skybox, güneş,
//!     kamera, zemin) tek çağrılık `world.spawn_bundle((...))` ile kurulur — eski `spawn()` +
//!     tekrar tekrar `add_component()` kalıbı yerine.
//!   * **Kamera yönü paylaşılan yardımcılardan** — `Camera::forward_from` / `Camera::right_from`
//!     (nişan/ileri-sağ matematiği elle yeniden yazılmaz; pitch her zaman clamp'li → davranış
//!     birebir aynı).
//!   * **`is_key_just_pressed`** — R (reset) için kenar-tespiti motordan (elle prev_* takibi yok).
//!   * **Render = `default_render_pass` + ÖZEL FEM geçişleri KORUNUR** — motorun tek-satır
//!     `with_scene_render()` kısayolu VAR ama onu kullanamayız: bu demo render-hook içinde GERÇEK
//!     özel iş yapar (FEM compute substep'leri + kendi render pipeline'ı ile yumuşak gövdeyi çizme).
//!     Bu mantık aynen korunur.
//!
//! ## Kontroller
//!   * **Sağ-tık + fare** — kamera orbit · **WASD / QE** — kamera hareket · **Shift** — hızlı
//!   * **R** — yumuşak gövdeyi başlangıç durumuna sıfırla

use gizmo::prelude::*;
use gizmo::renderer::gpu_physics::fem::{
    GpuFemCollider, GpuFemParams, GpuFemSystem, GpuSoftBodyNode, GpuTetrahedron,
};
use gizmo::renderer::gpu_types::Vertex;
use std::f32::consts::{FRAC_PI_2, FRAC_PI_3, PI};
use wgpu::util::DeviceExt;

struct BeamNGState {
    camera_speed: f32,
    camera_pitch: f32,
    camera_yaw: f32,
    camera_pos: Vec3,

    fem_system: GpuFemSystem,
    fem_render_pipeline: wgpu::RenderPipeline,
    fem_bind_group: wgpu::BindGroup,
    fem_index_buffer: wgpu::Buffer,
    fem_index_count: u32,

    // FPS-bağımsız sim: gerçek dt biriktirilir, substep sayısı ondan türetilir (update'te
    // hesaplanır, render'da tüketilir). ESKİDEN her frame SABİT 40 substep vardı → sim hızı = FPS.
    sim_accum: f32,
    pending_substeps: u32,
    // R ile başlangıç durumuna sıfırlama (bir-frame bayrağı: update'te set, render'da tüketilir).
    reset_requested: bool,
    initial_nodes: Vec<GpuSoftBodyNode>,
}

/// Kutu şeklinde tetrahedral FEM ağı üretir (düğümler + elemanlar + yüzey index tamponu).
/// Motorun rigid-body sistemleriyle ilgisi yok — tamamen GPU-FEM sim için özel geometri.
fn create_tetra_box(
    device: &wgpu::Device,
    pos: Vec3,
    size: Vec3,
    res_x: u32,
    res_y: u32,
    res_z: u32,
    mass: f32,
) -> (Vec<GpuSoftBodyNode>, Vec<GpuTetrahedron>, wgpu::Buffer, u32) {
    let mut nodes = Vec::new();
    let mut elements = Vec::new();

    let dx = size.x / res_x as f32;
    let dy = size.y / res_y as f32;
    let dz = size.z / res_z as f32;

    let total_nodes = (res_x + 1) * (res_y + 1) * (res_z + 1);
    let node_mass = mass / total_nodes as f32;

    // Düğümleri kur
    for z in 0..=res_z {
        for y in 0..=res_y {
            for x in 0..=res_x {
                let px = pos.x - size.x / 2.0 + x as f32 * dx;
                let py = pos.y - size.y / 2.0 + y as f32 * dy;
                let pz = pos.z - size.z / 2.0 + z as f32 * dz;

                nodes.push(GpuSoftBodyNode {
                    position_mass: [px, py, pz, node_mass],
                    velocity_fixed: [0.0, 0.0, 0.0, 0.0],
                    forces: [0, 0, 0, 0],
                });
            }
        }
    }

    let get_idx = |x, y, z| -> u32 { z * ((res_y + 1) * (res_x + 1)) + y * (res_x + 1) + x };

    let mut surface_indices = Vec::new();

    // Elemanları (tetrahedra) kur
    for z in 0..res_z {
        for y in 0..res_y {
            for x in 0..res_x {
                let i000 = get_idx(x, y, z);
                let i100 = get_idx(x + 1, y, z);
                let i010 = get_idx(x, y + 1, z);
                let i110 = get_idx(x + 1, y + 1, z);
                let i001 = get_idx(x, y, z + 1);
                let i101 = get_idx(x + 1, y, z + 1);
                let i011 = get_idx(x, y + 1, z + 1);
                let i111 = get_idx(x + 1, y + 1, z + 1);

                // Dönüşümlü 5-tetrahedron ayrıştırması
                let flip = (x + y + z) % 2 == 1;

                let tets = if flip {
                    vec![
                        [i001, i100, i010, i111],
                        [i100, i010, i000, i001],
                        [i100, i111, i010, i110],
                        [i100, i001, i111, i101],
                        [i010, i111, i001, i011],
                    ]
                } else {
                    vec![
                        [i000, i101, i011, i110],
                        [i000, i110, i100, i101],
                        [i000, i011, i010, i110],
                        [i000, i101, i001, i011],
                        [i110, i011, i101, i111],
                    ]
                };

                for t in tets {
                    let p0 = nodes[t[0] as usize].position_mass;
                    let p1 = nodes[t[1] as usize].position_mass;
                    let p2 = nodes[t[2] as usize].position_mass;
                    let p3 = nodes[t[3] as usize].position_mass;

                    let e1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
                    let e2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
                    let e3 = [p3[0] - p0[0], p3[1] - p0[1], p3[2] - p0[2]];

                    let dm = gizmo::math::Mat3::from_cols(
                        Vec3::new(e1[0], e1[1], e1[2]),
                        Vec3::new(e2[0], e2[1], e2[2]),
                        Vec3::new(e3[0], e3[1], e3[2]),
                    );

                    let det = dm.determinant();
                    // Dejenere tetrahedron (det≈0) → dm.inverse() NaN üretir ve TÜM sim'i zehirler.
                    // Düzgün ızgarada oluşmaz ama savunmacı guard (0 boyut/çözünürlük verilirse).
                    if det.abs() < 1e-9 {
                        continue;
                    }
                    let volume = (det / 6.0).abs();
                    let inv_dm = dm.inverse();

                    elements.push(GpuTetrahedron {
                        indices: t,
                        inv_rest_col0: [inv_dm.x_axis.x, inv_dm.x_axis.y, inv_dm.x_axis.z, 0.0],
                        inv_rest_col1: [inv_dm.y_axis.x, inv_dm.y_axis.y, inv_dm.y_axis.z, 0.0],
                        inv_rest_col2: [inv_dm.z_axis.x, inv_dm.z_axis.y, inv_dm.z_axis.z, 0.0],
                        rest_volume_pad: [volume, 0.0, 0.0, 0.0],
                    });
                }
            }
        }
    }

    // Dış yüzey index'lerini çıkar (çizim için)
    let mut add_quad = |i0, i1, i2, i3| {
        surface_indices.push(i0);
        surface_indices.push(i1);
        surface_indices.push(i2);
        surface_indices.push(i0);
        surface_indices.push(i2);
        surface_indices.push(i3);
    };

    for z in 0..res_z {
        for y in 0..res_y {
            // Sol (x=0) ve Sağ (x=res_x)
            add_quad(
                get_idx(0, y, z),
                get_idx(0, y + 1, z),
                get_idx(0, y + 1, z + 1),
                get_idx(0, y, z + 1),
            );
            add_quad(
                get_idx(res_x, y, z + 1),
                get_idx(res_x, y + 1, z + 1),
                get_idx(res_x, y + 1, z),
                get_idx(res_x, y, z),
            );
        }
    }
    for z in 0..res_z {
        for x in 0..res_x {
            // Alt (y=0) ve Üst (y=res_y)
            add_quad(
                get_idx(x, 0, z + 1),
                get_idx(x + 1, 0, z + 1),
                get_idx(x + 1, 0, z),
                get_idx(x, 0, z),
            );
            add_quad(
                get_idx(x, res_y, z),
                get_idx(x + 1, res_y, z),
                get_idx(x + 1, res_y, z + 1),
                get_idx(x, res_y, z + 1),
            );
        }
    }
    for y in 0..res_y {
        for x in 0..res_x {
            // Ön (z=0) ve Arka (z=res_z)
            add_quad(
                get_idx(x, y, 0),
                get_idx(x + 1, y, 0),
                get_idx(x + 1, y + 1, 0),
                get_idx(x, y + 1, 0),
            );
            add_quad(
                get_idx(x, y + 1, res_z),
                get_idx(x + 1, y + 1, res_z),
                get_idx(x + 1, y, res_z),
                get_idx(x, y, res_z),
            );
        }
    }

    let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("FEM Index Buffer"),
        contents: bytemuck::cast_slice(&surface_indices),
        usage: wgpu::BufferUsages::INDEX,
    });

    (nodes, elements, index_buffer, surface_indices.len() as u32)
}

fn setup(world: &mut World, renderer: &Renderer) -> BeamNGState {
    let mut asset_manager = AssetManager::new();

    // Gökyüzü (skybox) — ters küp, tek varlık
    let sky_tex = asset_manager
        .load_material_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
            "tut/assets/sky.jpg",
        )
        .unwrap();
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_scale(Vec3::splat(2000.0)),
        AssetManager::create_inverted_cube(&renderer.device),
        Material::new(sky_tex).with_skybox(),
        MeshRenderer::new(),
    ));

    // Güneş (yönlü ışık)
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_x(-PI / 4.0)),
        DirectionalLight::new(Vec3::new(1.0, 0.95, 0.9), 4.0, LightRole::Sun),
    ));

    // Kamera (serbest uçuş)
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, 5.0, 15.0)),
        Camera::new(FRAC_PI_3, 0.1, 5000.0, 0.0, 0.0, true),
    ));

    // Zemin (tek büyük quad, elle kurulu vertex tamponu + damalı doku)
    let mut ground_vertices = Vec::new();
    let r = 500.0;
    let uvs = 300.0;
    let v0 = Vertex {
        position: [-r, 0.0, r],
        tex_coords: [0.0, uvs],
        color: [1.0, 1.0, 1.0],
        normal: [0.0, 1.0, 0.0],
        tangent: [0.0, 0.0, 0.0, 1.0],
        joint_indices: [0; 4],
        joint_weights: [0.0; 4],
    };
    let v1 = Vertex {
        position: [r, 0.0, r],
        tex_coords: [uvs, uvs],
        color: [1.0, 1.0, 1.0],
        normal: [0.0, 1.0, 0.0],
        tangent: [0.0, 0.0, 0.0, 1.0],
        joint_indices: [0; 4],
        joint_weights: [0.0; 4],
    };
    let v2 = Vertex {
        position: [r, 0.0, -r],
        tex_coords: [uvs, 0.0],
        color: [1.0, 1.0, 1.0],
        normal: [0.0, 1.0, 0.0],
        tangent: [0.0, 0.0, 0.0, 1.0],
        joint_indices: [0; 4],
        joint_weights: [0.0; 4],
    };
    let v3 = Vertex {
        position: [-r, 0.0, -r],
        tex_coords: [0.0, 0.0],
        color: [1.0, 1.0, 1.0],
        normal: [0.0, 1.0, 0.0],
        tangent: [0.0, 0.0, 0.0, 1.0],
        joint_indices: [0; 4],
        joint_weights: [0.0; 4],
    };

    ground_vertices.push(v0);
    ground_vertices.push(v1);
    ground_vertices.push(v2);
    ground_vertices.push(v0);
    ground_vertices.push(v2);
    ground_vertices.push(v3);

    let ground_vbuf = renderer
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Ground VBuf"),
            contents: bytemuck::cast_slice(&ground_vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

    let ground_mesh = Mesh::new(
        &renderer.device,
        std::sync::Arc::new(ground_vbuf),
        &ground_vertices,
        Vec3::ZERO,
        "ground_mesh".to_string(),
    );

    let grass_tex = asset_manager.create_checkerboard_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );
    let grass_mat = Material::new(grass_tex).with_pbr(Vec4::new(0.5, 0.8, 0.3, 1.0), 0.9, 0.1);

    world.spawn_bundle((
        Transform::new(Vec3::ZERO),
        ground_mesh,
        grass_mat,
        MeshRenderer::new(),
    ));

    // FEM araba (chassis) kurulumu — özel tetrahedral yumuşak gövde.
    let (nodes, elements, index_buffer, index_count) = create_tetra_box(
        &renderer.device,
        Vec3::new(0.0, 15.0, 0.0), // Konum
        Vec3::new(2.0, 1.0, 4.0),  // Boyut
        8,
        4,
        16,     // Çözünürlük
        1000.0, // Kütle
    );

    let params = GpuFemParams {
        properties: [0.0005, 10000.0, 10000.0, 0.999], // dt, mu, lambda, damping
        gravity: [0.0, -9.81, 0.0, 0.0],
        counts: [nodes.len() as u32, elements.len() as u32, 1, 0],
    };

    let ground_collider = GpuFemCollider {
        shape_type: 0, // Düzlem
        radius: 0.0,
        _pad0: 0,
        _pad1: 0,
        position: [0.0, 0.0, 0.0, 0.0],
        normal: [0.0, 1.0, 0.0, 0.0],
    };

    let fem_system = GpuFemSystem::new(
        &renderer.device,
        &nodes,
        &elements,
        &[ground_collider],
        &params,
    );

    // FEM yumuşak gövdesi için render pipeline (pozisyonları storage buffer'dan çeker).
    let render_shader = renderer
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("FEM Render Shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../../../crates/gizmo-renderer/src/shaders/fem_render.wgsl").into(),
            ),
        });

    let fem_bind_group_layout =
        renderer
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
                label: Some("fem_render_bind_group_layout"),
            });

    let fem_bind_group = renderer
        .device
        .create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &fem_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: fem_system.nodes_buffer.as_entire_binding(),
            }],
            label: Some("fem_render_bind_group"),
        });

    let render_pipeline_layout =
        renderer
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("FEM Render Pipeline Layout"),
                bind_group_layouts: &[
                    Some(&renderer.scene.global_bind_group_layout),
                    Some(&fem_bind_group_layout),
                ],
                immediate_size: 0,
            });

    let fem_render_pipeline =
        renderer
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("FEM Render Pipeline"),
                layout: Some(&render_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &render_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[], // Vertex verisi yerine storage buffer'dan çekilir
                },
                fragment: Some(wgpu::FragmentState {
                    module: &render_shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: renderer.config.format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: Some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

    world.insert_resource(asset_manager);

    println!(
        "BeamNG FEM demo hazır. Kontroller: R = yumuşak gövdeyi sıfırla, sağ-tık = kamera orbit, \
         WASD/QE = kamera hareket, Shift = hızlı. Sim FPS-bağımsız (substep gerçek dt'den)."
    );

    BeamNGState {
        camera_speed: 15.0,
        camera_pitch: 0.0,
        camera_yaw: -PI / 2.0,
        camera_pos: Vec3::new(0.0, 5.0, 15.0),
        fem_system,
        fem_render_pipeline,
        fem_bind_group,
        fem_index_buffer: index_buffer,
        fem_index_count: index_count,
        sim_accum: 0.0,
        pending_substeps: 40,
        reset_requested: false,
        initial_nodes: nodes,
    }
}

fn update(world: &mut World, state: &mut BeamNGState, dt: f32, input: &Input) {
    // Önceki frame'de set edilen reset bayrağı render'da tüketildi → temizle.
    state.reset_requested = false;
    // R: yumuşak gövdeyi başlangıç durumuna sıfırla (kenar-tespiti motorun is_key_just_* API'sinden).
    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        state.reset_requested = true;
    }

    // FEM substep sayısını GERÇEK dt'den türet (FPS-bağımsız). ESKİDEN her frame sabit 40 substep
    // × dt=0.0005 = 0.02 s/frame işleniyordu, gerçek frame süresinden bağımsız → sim hızı = FPS.
    const SUB_DT: f32 = 0.0005; // params.dt ile aynı fiziksel substep
    state.sim_accum += dt.min(0.1);
    let n = (state.sim_accum / SUB_DT).floor();
    state.pending_substeps = (n as u32).min(80); // spiral-of-death guard
    state.sim_accum -= n * SUB_DT;

    // Sağ-tık: kamera orbit (fare-look).
    if input.is_mouse_button_pressed(1) {
        let delta = input.mouse_delta();
        state.camera_yaw -= delta.0 * 0.005;
        state.camera_pitch -= delta.1 * 0.005;
        state.camera_pitch = state.camera_pitch.clamp(-PI / 2.0 + 0.1, PI / 2.0 - 0.1);
    }

    // İleri/sağ yönler paylaşılan kamera yardımcılarından (elle yeniden yazılmaz).
    let forward = Camera::forward_from(state.camera_yaw, state.camera_pitch);
    let right = Camera::right_from(state.camera_yaw);
    let up = Vec3::new(0.0, 1.0, 0.0);

    let speed = if input.is_key_pressed(KeyCode::ShiftLeft as u32) {
        state.camera_speed * 3.0
    } else {
        state.camera_speed
    };

    let mut cam_move = Vec3::ZERO;
    if input.is_key_pressed(KeyCode::KeyW as u32) {
        cam_move += forward;
    }
    if input.is_key_pressed(KeyCode::KeyS as u32) {
        cam_move -= forward;
    }
    if input.is_key_pressed(KeyCode::KeyD as u32) {
        cam_move += right;
    }
    if input.is_key_pressed(KeyCode::KeyA as u32) {
        cam_move -= right;
    }
    if input.is_key_pressed(KeyCode::KeyE as u32) {
        cam_move += up;
    }
    if input.is_key_pressed(KeyCode::KeyQ as u32) {
        cam_move -= up;
    }

    if cam_move.length_squared() > 0.0 {
        state.camera_pos += cam_move.normalize() * speed * dt;
    }

    if let Some(mut q) = world.query_mut::<(Mut<Transform>, Mut<Camera>)>() {
        let yaw_rot = Quat::from_rotation_y(-state.camera_yaw + FRAC_PI_2);
        let pitch_rot = Quat::from_rotation_x(state.camera_pitch);
        let rot = yaw_rot * pitch_rot;

        for (_, (mut trans, mut cam)) in q.iter_mut() {
            trans.position = state.camera_pos;
            trans.rotation = rot;
            cam.yaw = state.camera_yaw;
            cam.pitch = state.camera_pitch;
        }
    }
}

// Render-hook GERÇEK özel iş yapar (FEM compute substep'leri + kendi pipeline'ıyla yumuşak gövdeyi
// çizme). Bu yüzden `with_scene_render()` tek-satır kısayoluna ÇEVRİLMEZ; standart sahne için
// `default_render_pass`, ardından özel FEM geçişi elle sürülür.
fn render(
    world: &mut World,
    state: &BeamNGState,
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    renderer: &mut Renderer,
    _light_time: f32,
) {
    // R ile sıfırlama: başlangıç düğüm-tamponunu GPU'ya geri yaz (pozisyon+hız+kuvvet sıfırlanır).
    if state.reset_requested {
        renderer.queue.write_buffer(
            &state.fem_system.nodes_buffer,
            0,
            bytemuck::cast_slice(&state.initial_nodes),
        );
    }

    // 1. FEM compute — substep sayısı update'te GERÇEK dt'den hesaplandı (FPS-bağımsız).
    for _ in 0..state.pending_substeps {
        state.fem_system.compute_pass(encoder);
    }

    // 2. Standart varlıkları çiz (skybox, zemin)
    default_render_pass(world, encoder, view, renderer);

    // 3. FEM geçişi için derinlik tamponunu hazırla
    let depth_view = &renderer.depth_texture_view;

    // 4. FEM nesnesini çiz
    {
        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("FEM Render Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        rpass.set_pipeline(&state.fem_render_pipeline);
        rpass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
        rpass.set_bind_group(1, &state.fem_bind_group, &[]);

        rpass.set_index_buffer(state.fem_index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        // Not: index buffer'ı kullanarak pozisyonu Vertex Shader içinde storage buffer'dan çekiyoruz
        rpass.draw_indexed(0..state.fem_index_count, 0, 0..1);
    }
}

fn main() {
    App::<BeamNGState>::new("Gizmo Engine - BeamNG Style FEM Soft Body", 1280, 720)
        .set_setup(setup)
        .set_update(update)
        .set_render(render)
        .run()
        .expect("uygulama çalıştırılamadı");
}
