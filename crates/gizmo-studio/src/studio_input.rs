//! Studio Input — Viewport raycast ve rubber band seçim mantığı
//!
//! Bu dosya şu sorumlulukları taşır:
//! 1. Fare ışını (ray) ile sahnedeki objelere tıklama (raycast/OBB seçimi)
//! 2. Rubber band (kutu) ile çoklu seçim
//! 3. Kamera ışını (ray) oluşturma
//!
//! Seçim akışı:
//! ```text
//! scene_view.rs  →  do_raycast = true
//! systems/input.rs → build_ray() → handle_studio_input()
//! studio_input.rs  → perform_raycast() → state.select_exclusive()
//! update.rs        → OBB highlight çizimi
//! ```

use gizmo::editor::EditorState;
use gizmo::math::{Ray, Vec3};
use gizmo::prelude::*;

// ═══════════════════════════════════════════════════════════════
// ANA GİRİŞ NOKTASI
// ═══════════════════════════════════════════════════════════════

/// Her frame çağrılır. Raycast ve rubber band seçimlerini yürütür.
pub fn handle_studio_input(
    world: &mut World,
    state: &mut EditorState,
    ray: Ray,
    player_id: u32,
    do_raycast: bool,
    ctrl_pressed: bool,
) {
    if do_raycast {
        perform_raycast(world, state, ray, player_id, ctrl_pressed);
    }

    if let Some((start, end)) = state.selection.rubber_band_request.take() {
        perform_rubber_band_selection(world, state, start, end, player_id, ctrl_pressed);
    }
}

// ═══════════════════════════════════════════════════════════════
// RAYCAST SEÇİMİ
// ═══════════════════════════════════════════════════════════════

/// Editör ismi olan objeleri seçimden çıkarır (Grid, Directional Light vb.)
const EDITOR_ENTITY_NAMES: &[&str] = &["Editor Grid", "Editor Guidelines", "Directional Light"];
const EDITOR_ENTITY_PREFIXES: &[&str] = &["Editor Light Icon"];

/// Bir entity'nin editör objesi (seçilemez) olup olmadığını kontrol eder.
fn is_editor_entity(world: &World, id: u32) -> bool {
    let names = world.borrow::<gizmo::core::component::EntityName>();
    if let Some(name) = names.get(id) {
        if EDITOR_ENTITY_NAMES.contains(&name.0.as_str()) {
            return true;
        }
        for prefix in EDITOR_ENTITY_PREFIXES {
            if name.0.starts_with(prefix) {
                return true;
            }
        }
    }
    false
}

/// Fare ışınıyla sahnedeki en yakın objeye tıklama.
/// OBB (Oriented Bounding Box) testi ile çalışır.
fn perform_raycast(
    world: &mut World,
    state: &mut EditorState,
    ray: Ray,
    player_id: u32,
    ctrl_pressed: bool,
) {
    state.do_raycast = false;

    let hit = find_closest_hit(world, &ray, player_id);

    match hit {
        Some(hit_id) => {
            // Parent zincirini yukarı takip et → root entity'yi bul (Blender davranışı)
            let root_id = find_selection_root(world, hit_id);

            let name = world
                .borrow::<gizmo::core::component::EntityName>()
                .get(root_id)
                .map(|n| n.0.clone())
                .unwrap_or_else(|| format!("Entity {}", root_id));
            state.log_info(&format!("Seçildi: {}", name));

            if let Some(entity) = world.get_entity(root_id) {
                if ctrl_pressed {
                    state.toggle_selection(entity);
                } else {
                    state.select_exclusive(entity);
                }
            }
        }
        None => {
            state.log_info("Boşluğa tıklandı, seçim temizlendi.");
            state.clear_selection();
        }
    }
}

/// Bir entity'nin en üstteki seçilebilir atasını (root) bulur.
/// GLTF modelleri için: mesh child'ından → GLTF root'a yürür.
/// Sonsuz döngüyü engellemek için maksimum 32 seviye derinlik limiti var.
fn find_selection_root(world: &World, start_id: u32) -> u32 {
    let parents = world.borrow::<gizmo::core::component::Parent>();
    let mut current = start_id;

    for _ in 0..32 {
        match parents.get(current) {
            Some(parent) => current = parent.0,
            None => break, // Root'a ulaştık
        }
    }

    current
}

/// Işın ile sahnedeki tüm objeleri tarar, en yakın isabet eden entity id'sini döner.
fn find_closest_hit(world: &World, ray: &Ray, player_id: u32) -> Option<u32> {
    let mut closest_t = f32::MAX;
    let mut hit_entity = None;

    let transforms = world.borrow::<Transform>();
    let global_transforms = world.borrow::<gizmo::physics::components::GlobalTransform>();
    let colliders = world.borrow::<Collider>();
    let meshes = world.borrow::<gizmo::renderer::components::Mesh>();
    let is_hidden = world.borrow::<gizmo::core::component::IsHidden>();

    for (id, t) in transforms.iter() {
        // Filtreleme: Kamera, gizli, editör objeleri
        if id == player_id || is_hidden.contains(id) || is_editor_entity(world, id) {
            continue;
        }

        // Objenin OBB bilgilerini topla
        let obb = match compute_entity_obb(id, t, &meshes, &colliders, &global_transforms) {
            Some(obb) => obb,
            None => continue, // Hacmi olmayan objeleri atla
        };

        // Işın testi
        if let Some(hit_t) = ray.intersect_obb(obb.center, obb.half_extents, obb.rotation) {
            if hit_t > 0.0 && hit_t < closest_t {
                closest_t = hit_t;
                hit_entity = Some(id);
            }
        }
    }

    hit_entity
}

/// Bir entity'nin OBB (Oriented Bounding Box) bilgilerini hesaplar.
struct ObbInfo {
    center: Vec3,
    half_extents: Vec3,
    rotation: gizmo::math::Quat,
}

fn compute_entity_obb(
    id: u32,
    local_transform: &Transform,
    meshes: &gizmo::core::storage::StorageView<gizmo::renderer::components::Mesh>,
    colliders: &gizmo::core::storage::StorageView<Collider>,
    global_transforms: &gizmo::core::storage::StorageView<gizmo::physics::components::GlobalTransform>,
) -> Option<ObbInfo> {
    let mut extents;
    let mut local_offset = Vec3::ZERO;

    if let Some(mesh) = meshes.get(id) {
        let (min, max) = (mesh.bounds.min, mesh.bounds.max);
        extents = Vec3::new(
            (max.x - min.x).abs() * 0.5,
            (max.y - min.y).abs() * 0.5,
            (max.z - min.z).abs() * 0.5,
        );
        local_offset = Vec3::new(
            (max.x + min.x) * 0.5,
            (max.y + min.y) * 0.5,
            (max.z + min.z) * 0.5,
        );
        // Çok ince mesh'ler için minimum tıklanabilir alan
        extents = extents.max(Vec3::splat(0.1));
    } else if let Some(col) = colliders.get(id) {
        let aabb = col.compute_aabb(Vec3::ZERO, gizmo::math::Quat::IDENTITY);
        extents = Vec3::new(aabb.half_extents().x, aabb.half_extents().y, aabb.half_extents().z);
    } else {
        return None; // Hacmi yok — seçilemez
    }

    // Global transform üzerinden pozisyon, rotasyon, scale al
    let (g_pos, g_rot, g_scale) = if let Some(gt) = global_transforms.get(id) {
        gizmo::renderer::decompose_mat4(gt.matrix)
    } else {
        (local_transform.position, local_transform.rotation, local_transform.scale)
    };

    // Renderer: model = global_transform * center_offset
    // center_offset'i OBB merkezine ekle (visual mesh ile aynı pozisyon)
    let mesh_center_offset = meshes.get(id).map(|m| m.center_offset).unwrap_or(Vec3::ZERO);
    let total_local_offset = local_offset + mesh_center_offset;

    Some(ObbInfo {
        center: g_pos + (g_rot * (total_local_offset * g_scale)),
        half_extents: Vec3::new(
            extents.x * g_scale.x,
            extents.y * g_scale.y,
            extents.z * g_scale.z,
        ),
        rotation: g_rot,
    })
}

// ═══════════════════════════════════════════════════════════════
// RUBBER BAND (KUTU) SEÇİMİ
// ═══════════════════════════════════════════════════════════════

/// Kutu ile çoklu seçim — ekran koordinatlarında dikdörtgen çizer ve içindeki objeleri seçer.
fn perform_rubber_band_selection(
    world: &mut World,
    state: &mut EditorState,
    start: gizmo::math::Vec2,
    end: gizmo::math::Vec2,
    player_id: u32,
    ctrl_pressed: bool,
) {
    let (view_mat, proj_mat, scene_rect) =
        match (state.camera.view, state.camera.proj, state.scene_view_rect) {
            (Some(v), Some(p), Some(r)) => (v, p, r),
            _ => return,
        };

    let vp_mat = proj_mat * view_mat;
    let rect_left = scene_rect.min.x;
    let rect_top = scene_rect.min.y;
    let rect_width = scene_rect.max.x - rect_left;
    let rect_height = scene_rect.max.y - rect_top;

    let (min_x, max_x) = (start.x.min(end.x), start.x.max(end.x));
    let (min_y, max_y) = (start.y.min(end.y), start.y.max(end.y));

    if !ctrl_pressed {
        state.selection.entities.clear();
    }

    let transforms = world.borrow::<Transform>();
    let global_transforms = world.borrow::<gizmo::physics::components::GlobalTransform>();
    let is_hidden = world.borrow::<gizmo::core::component::IsHidden>();
    let colliders = world.borrow::<Collider>();
    let meshes = world.borrow::<gizmo::renderer::components::Mesh>();

    for (id, t) in transforms.iter() {
        if id == player_id || is_hidden.contains(id) || is_editor_entity(world, id) {
            continue;
        }

        // Hacmi olmayanları atla
        if colliders.get(id).is_none() && meshes.get(id).is_none() {
            continue;
        }

        // Global pozisyon hesapla
        let (g_pos, g_rot, g_scale) = if let Some(gt) = global_transforms.get(id) {
            gizmo::renderer::decompose_mat4(gt.matrix)
        } else {
            (t.position, t.rotation, t.scale)
        };

        let mut local_offset = Vec3::ZERO;
        if let Some(mesh) = meshes.get(id) {
            let (min, max) = (mesh.bounds.min, mesh.bounds.max);
            local_offset = Vec3::new(
                (max.x + min.x) * 0.5,
                (max.y + min.y) * 0.5,
                (max.z + min.z) * 0.5,
            );
            // Renderer ile aynı: center_offset'i ekle
            local_offset = local_offset + mesh.center_offset;
        }

        let center = g_pos + (g_rot * (local_offset * g_scale));

        // World → Screen dönüşümü
        let clip = vp_mat * gizmo::math::Vec4::new(center.x, center.y, center.z, 1.0);
        if clip.w <= 0.0 {
            continue; // Kamera arkasında
        }

        let ndc = gizmo::math::Vec3::new(clip.x, clip.y, clip.z) / clip.w;
        let screen_x = ((ndc.x + 1.0) / 2.0) * rect_width + rect_left;
        let screen_y = ((1.0 - ndc.y) / 2.0) * rect_height + rect_top;

        if screen_x >= min_x && screen_x <= max_x && screen_y >= min_y && screen_y <= max_y {
            // Child mesh yerine root entity'yi seç (raycast ile aynı davranış)
            let root_id = find_selection_root(world, id);
            if let Some(entity) = world.get_entity(root_id) {
                state.selection.entities.insert(entity);
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// KAMERA IŞINI OLUŞTURMA
// ═══════════════════════════════════════════════════════════════

/// NDC koordinatlarından kamera ışını (ray) oluşturur.
/// `ndc_x`, `ndc_y`: -1..1 aralığında normalize edilmiş ekran koordinatları.
pub fn build_ray(
    world: &World,
    player_id: u32,
    ndc_x: f32,
    ndc_y: f32,
    aspect: f32,
    _wh: f32,
) -> Option<Ray> {
    let transforms = world.borrow::<Transform>();
    let cameras = world.borrow::<gizmo::renderer::components::Camera>();

    let cam_t = transforms.get(player_id)?;
    let cam = cameras.get(player_id)?;

    let view = cam.get_view(cam_t.position);
    let proj = cam.get_projection(aspect);
    let inv_vp = (proj * view).inverse();

    // WGPU: Z=0 yakın düzlem, Z=1 uzak düzlem
    let near = inv_vp.project_point3(gizmo::math::Vec3::new(ndc_x, ndc_y, 0.0));
    let far = inv_vp.project_point3(gizmo::math::Vec3::new(ndc_x, ndc_y, 1.0));

    Some(Ray {
        origin: near.into(),
        direction: (far - near).normalize().into(),
    })
}
