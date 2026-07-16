//! Kumaş bir cismin üstüne serilir — CPU XPBD `Cloth`'un rijit çarpışması ile (küre/kapsül/
//! kutu). Kumaş boşluksuz KATI çift-yüzlü üçgen çarşaf olarak render edilir (her kare
//! düğümlerden yeniden kurulur).
//!
//! **YUKARI / AŞAGI ok** = kumaşın SEGMENT sayısını (çözünürlüğünü) değiştir:
//!   1 = tek parça, sert TAHTA gibi · büyük N = çok segment, akıcı kumaş gibi kıvrılır.
//! **C** = altdaki cismi değiştir (küre → kapsül → kutu).
//! **R** = yeniden düşür.
//! (Değerler terminale yazılır.)

use std::f32::consts::{FRAC_PI_2, FRAC_PI_3};
use std::sync::Arc;

use gizmo::physics::cloth::Cloth;
use gizmo::physics::components::Collider;
use gizmo::physics::{BodyHandle, Transform as PhysTransform};
use gizmo::prelude::*;
use gizmo::renderer::asset::AssetManager;
use gizmo::renderer::components::{Camera, DirectionalLight, LightRole, Material, Mesh, MeshRenderer};
use gizmo::renderer::gpu_types::Vertex;

const SIZE: f32 = 3.4; // kumaşın fiziksel kenar uzunluğu (segment sayısından bağımsız)
const OBJ_C: Vec3 = Vec3::new(0.0, 1.3, 0.0);
const START_Y: f32 = 3.6;
const MIN_N: usize = 1;
const MAX_N: usize = 48; // çözünürlük üst sınırı (performans)

const SPHERE_R: f32 = 1.3;
const CAP_R: f32 = 0.85;
const CAP_HH: f32 = 0.85; // kapsül silindir yarı-yüksekliği (eksen Y)
const BOX_HE: Vec3 = Vec3::new(1.15, 1.15, 1.15);

const SHAPE_NAMES: [&str; 3] = ["küre", "kapsül", "kutu"];

/// `shape` indeksine karşılık fizik collider'ı (0=küre, 1=kapsül, 2=kutu).
fn shape_collider(shape: usize) -> Collider {
    match shape {
        0 => Collider::sphere(SPHERE_R),
        1 => Collider::capsule(CAP_R, CAP_HH),
        _ => Collider::box_collider(BOX_HE),
    }
}

/// Görsel mesh'in Transform ölçeği. Küre/kapsül mesh'i gerçek boyutta pişirildi (ölçek 1);
/// küp mesh'i yarı-kenar 1 birim olduğundan half-extents ile ölçeklenir.
fn shape_scale(shape: usize) -> Vec3 {
    match shape {
        2 => BOX_HE,
        _ => Vec3::ONE,
    }
}

struct ClothDemo {
    cloth: Cloth,
    grid: usize, // kenar başına düğüm = segment + 1
    segments: usize,
    cloth_entity: u32,
    obj_entity: u32,
    obj_meshes: Vec<Mesh>, // [küre, kapsül, kutu]
    shape: usize,
    colliders: Vec<(BodyHandle, PhysTransform, Collider)>,
    accum: f32,
    cooldown: f32,
    prev_r: bool,
    prev_c: bool,
}

/// `segments` segmentlik (kenar başına) bir kumaş kur — fiziksel boyu SIZE sabit.
fn make_cloth(segments: usize) -> (Cloth, usize) {
    let seg = segments.clamp(MIN_N, MAX_N);
    let grid = seg + 1; // kenar başına düğüm
    let spacing = SIZE / seg as f32;
    let mut cloth = Cloth::new(grid, grid, spacing, 1.0);
    let half = SIZE * 0.5;
    for (i, node) in cloth.nodes.iter_mut().enumerate() {
        let x = (i % grid) as f32 * spacing - half;
        let z = (i / grid) as f32 * spacing - half;
        node.position = Vec3::new(x, START_Y, z);
        node.prev_position = node.position;
    }
    cloth.friction = 0.4;
    (cloth, grid)
}

fn push_tri(verts: &mut Vec<Vertex>, a: Vec3, b: Vec3, c: Vec3, col: [f32; 3]) {
    let n = (b - a).cross(c - a).normalize_or_zero();
    let mk = |p: Vec3, nn: Vec3| Vertex {
        position: [p.x, p.y, p.z],
        color: col,
        normal: [nn.x, nn.y, nn.z],
        tangent: [0.0, 0.0, 0.0, 1.0],
        tex_coords: [0.0, 0.0],
        joint_indices: [0; 4],
        joint_weights: [0.0; 4],
    };
    verts.push(mk(a, n));
    verts.push(mk(b, n));
    verts.push(mk(c, n));
    verts.push(mk(a, -n));
    verts.push(mk(c, -n));
    verts.push(mk(b, -n));
}

/// Kumaş düğümlerinden katı, çift-yüzlü, dama-desenli üçgen mesh kurar.
fn build_cloth_mesh(device: &gizmo::wgpu::Device, cloth: &Cloth, grid: usize) -> Mesh {
    use gizmo::wgpu::util::DeviceExt;
    let node = |x: usize, z: usize| cloth.nodes[z * grid + x].position;
    let mut verts: Vec<Vertex> = Vec::with_capacity((grid - 1) * (grid - 1) * 12);
    for z in 0..grid - 1 {
        for x in 0..grid - 1 {
            let col = if (x + z).is_multiple_of(2) {
                [0.85, 0.22, 0.24]
            } else {
                [0.96, 0.92, 0.90]
            };
            let (p00, p10, p11, p01) = (node(x, z), node(x + 1, z), node(x + 1, z + 1), node(x, z + 1));
            push_tri(&mut verts, p00, p11, p10, col);
            push_tri(&mut verts, p00, p01, p11, col);
        }
    }
    let vbuf = device.create_buffer_init(&gizmo::wgpu::util::BufferInitDescriptor {
        label: Some("Cloth VBuf"),
        contents: gizmo::bytemuck::cast_slice(&verts),
        usage: gizmo::wgpu::BufferUsages::VERTEX,
    });
    Mesh::new(device, Arc::new(vbuf), &verts, Vec3::ZERO, "cloth".to_string())
}

fn setup(world: &mut World, renderer: &Renderer) -> ClothDemo {
    let mut assets = AssetManager::new();
    let tex = assets.create_white_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );
    let cube = AssetManager::create_cube(&renderer.device);

    // Üç collider cismi için mesh'ler (küre/kapsül gerçek boyutta, kutu birim küp).
    let obj_meshes = vec![
        AssetManager::create_sphere(&renderer.device, SPHERE_R, 32, 32),
        AssetManager::create_capsule(&renderer.device, CAP_R, 2.0 * CAP_HH, 16, 24),
        AssetManager::create_cube(&renderer.device),
    ];

    world.spawn_bundle((
        Transform::new(Vec3::new(12.0, 30.0, 14.0)).with_rotation(Quat::from_rotation_x(-0.9)),
        DirectionalLight::new(Vec3::new(1.0, 0.97, 0.9), 3.2, LightRole::Sun),
    ));
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, 3.2, 7.0)),
        Camera::new(FRAC_PI_3, 0.1, 500.0, -FRAC_PI_2, -0.28, true),
    ));
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, -0.25, 0.0)).with_scale(Vec3::new(20.0, 0.25, 20.0)),
        cube.clone(),
        Material::new(tex.clone()).with_pbr(Vec4::new(0.28, 0.30, 0.34, 1.0), 0.9, 0.05),
        MeshRenderer::new(),
    ));

    // Serilecek cisim (başlangıç: küre).
    let shape = 0usize;
    let obj_entity = world
        .spawn_bundle((
            Transform::new(OBJ_C).with_scale(shape_scale(shape)),
            obj_meshes[shape].clone(),
            Material::new(tex.clone()).with_pbr(Vec4::new(0.35, 0.55, 0.9, 1.0), 0.3, 0.5),
            MeshRenderer::new(),
        ))
        .id();

    let colliders = vec![
        (BodyHandle::from_id(1), PhysTransform::new(OBJ_C), shape_collider(shape)),
        (
            BodyHandle::from_id(2),
            PhysTransform::new(Vec3::new(0.0, -0.25, 0.0)),
            Collider::box_collider(Vec3::new(20.0, 0.25, 20.0)),
        ),
    ];

    let segments = 6;
    let (cloth, grid) = make_cloth(segments);
    let cloth_entity = world
        .spawn_bundle((
            Transform::new(Vec3::ZERO),
            build_cloth_mesh(&renderer.device, &cloth, grid),
            Material::new(tex.clone()).with_pbr(Vec4::new(1.0, 1.0, 1.0, 1.0), 0.7, 0.15),
            MeshRenderer::new(),
        ))
        .id();

    eprintln!("cisim: {}  ·  segment: {segments}  (↑↓ segment, C cisim, R yeniden)", SHAPE_NAMES[shape]);
    ClothDemo {
        cloth,
        grid,
        segments,
        cloth_entity,
        obj_entity,
        obj_meshes,
        shape,
        colliders,
        accum: 0.0,
        cooldown: 0.0,
        prev_r: false,
        prev_c: false,
    }
}

fn update(world: &mut World, state: &mut ClothDemo, dt: f32, input: &gizmo::core::input::Input) {
    // Yukarı/Aşağı ok → segment sayısını değiştir (basılı tutunca hızlanır).
    state.cooldown -= dt;
    if state.cooldown <= 0.0 {
        let up = input.is_key_pressed(KeyCode::ArrowUp as u32);
        let down = input.is_key_pressed(KeyCode::ArrowDown as u32);
        let new = if up && state.segments < MAX_N {
            state.segments + 1
        } else if down && state.segments > MIN_N {
            state.segments - 1
        } else {
            state.segments
        };
        if new != state.segments {
            state.segments = new;
            let (cloth, grid) = make_cloth(new);
            state.cloth = cloth;
            state.grid = grid;
            state.accum = 0.0;
            state.cooldown = 0.07;
            eprintln!("segment: {new}");
        }
    }

    // C = altdaki cismi değiştir (küre → kapsül → kutu) + kumaşı yeniden düşür.
    let c = input.is_key_pressed(KeyCode::KeyC as u32);
    if c && !state.prev_c {
        state.shape = (state.shape + 1) % 3;
        state.colliders[0].2 = shape_collider(state.shape);
        {
            let mut ts = world.borrow_mut::<Transform>();
            if let Some(mut t) = ts.get_mut(state.obj_entity) {
                t.scale = shape_scale(state.shape);
                t.update_local_matrix();
            }
        }
        {
            let mut ms = world.borrow_mut::<Mesh>();
            if let Some(mut m) = ms.get_mut(state.obj_entity) {
                *m = state.obj_meshes[state.shape].clone();
            }
        }
        let (cloth, grid) = make_cloth(state.segments);
        state.cloth = cloth;
        state.grid = grid;
        state.accum = 0.0;
        eprintln!("cisim: {}", SHAPE_NAMES[state.shape]);
    }
    state.prev_c = c;

    // R = yeniden düşür.
    let r = input.is_key_pressed(KeyCode::KeyR as u32);
    if r && !state.prev_r {
        let (cloth, grid) = make_cloth(state.segments);
        state.cloth = cloth;
        state.grid = grid;
        state.accum = 0.0;
    }
    state.prev_r = r;

    // SABİT dt biriktirici (değişken kare-dt XPBD'de enerji enjekte eder → kumaş uçar).
    const FIXED: f32 = 1.0 / 60.0;
    state.accum = (state.accum + dt).min(0.1);
    let mut n = 0;
    while state.accum >= FIXED && n < 5 {
        state.cloth.step(FIXED, Vec3::new(0.0, -9.81, 0.0), 10, &state.colliders);
        state.accum -= FIXED;
        n += 1;
    }
}

fn render(
    world: &mut World,
    s: &ClothDemo,
    encoder: &mut gizmo::wgpu::CommandEncoder,
    view: &gizmo::wgpu::TextureView,
    renderer: &mut Renderer,
    _light_time: f32,
) {
    let mesh = build_cloth_mesh(&renderer.device, &s.cloth, s.grid);
    {
        let mut ms = world.borrow_mut::<Mesh>();
        if let Some(mut m) = ms.get_mut(s.cloth_entity) {
            *m = mesh;
        }
    }
    gizmo::systems::default_render_pass(world, encoder, view, renderer);
}

fn main() {
    App::<ClothDemo>::new("Gizmo — Kumaş (↑↓ çözünürlük · C cisim küre/kapsül/kutu · R yeniden)", 1280, 720)
        .set_setup(setup)
        .set_update(update)
        .set_render(render)
        .run()
        .expect("uygulama çalıştırılamadı");
}
