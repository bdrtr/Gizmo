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
        if id == player_id || is_hidden.get(id).is_some() || is_editor_entity(world, id) {
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
    meshes: &gizmo::core::Query<'_, &gizmo::renderer::components::Mesh>,
    colliders: &gizmo::core::Query<'_, &Collider>,
    global_transforms: &gizmo::core::Query<'_, &gizmo::physics::components::GlobalTransform>,
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
    } else {
        let col = colliders.get(id)?;
        let aabb = col.compute_aabb(Vec3::ZERO, gizmo::math::Quat::IDENTITY);
        extents = Vec3::new(aabb.half_extents().x, aabb.half_extents().y, aabb.half_extents().z);
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
        if id == player_id || is_hidden.get(id).is_some() || is_editor_entity(world, id) {
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
            local_offset += mesh.center_offset;
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

    // WGPU: Z=0 yakın düzlem, Z=1 uzak düzlem. Ray::from_ndc kanonik guard'lı
    // yolu kullanır: tekil VP-inverse veya dejenere (far==near) yön ham
    // .normalize() ile NaN üretmek yerine güvenli varsayılana düşer.
    Some(Ray::from_ndc(gizmo::math::Vec2::new(ndc_x, ndc_y), inv_vp))
}

#[cfg(test)]
mod tests {
    // Real production functions exercised against a HEADLESS World (no GPU, no
    // window). Transform / Camera / Collider are plain-data components, so we can
    // spawn entities and drive the actual picking/selection logic. Private helpers
    // (`find_selection_root`, `is_editor_entity`, `compute_entity_obb`) are reachable
    // because this module is a descendant of the one that defines them.
    use super::{build_ray, compute_entity_obb, find_selection_root, is_editor_entity};
    use gizmo::core::component::{EntityName, Parent};
    use gizmo::math::{Quat, Vec3};
    use gizmo::physics::components::{GlobalTransform, Transform};
    use gizmo::physics::Collider;
    use gizmo::prelude::World;
    use gizmo::renderer::components::Camera;

    const FRAC_PI_2: f32 = std::f32::consts::FRAC_PI_2;

    fn make_camera(yaw: f32, pitch: f32) -> Camera {
        Camera::new(90.0_f32.to_radians(), 0.1, 100.0, yaw, pitch, true)
    }

    fn spawn_camera(world: &mut World, pos: Vec3, yaw: f32, pitch: f32) -> u32 {
        let e = world.spawn();
        world.add_component(e, Transform::new(pos));
        world.add_component(e, make_camera(yaw, pitch));
        e.id()
    }

    // ── build_ray ──────────────────────────────────────────────────────────

    /// Center-of-screen ray must originate a finite point and aim exactly along the
    /// camera's forward vector — the invariant the whole picking pipeline relies on.
    #[test]
    fn build_ray_center_points_along_camera_forward() {
        let mut world = World::new();
        // yaw=-PI/2 → forward ≈ (0,0,-1) per Camera::forward_from.
        let cam_id = spawn_camera(&mut world, Vec3::new(2.0, 3.0, 4.0), -FRAC_PI_2, 0.0);

        let ray = build_ray(&world, cam_id, 0.0, 0.0, 1.0, 1.0).expect("ray built");
        assert!(ray.is_valid(), "ray direction must be unit-length and finite");
        assert!(ray.origin.is_finite());

        let forward = make_camera(-FRAC_PI_2, 0.0).get_front();
        let d = ray.direction;
        assert!((d.x - forward.x).abs() < 1e-3, "dir.x={} fwd.x={}", d.x, forward.x);
        assert!((d.y - forward.y).abs() < 1e-3, "dir.y={} fwd.y={}", d.y, forward.y);
        assert!((d.z - forward.z).abs() < 1e-3, "dir.z={} fwd.z={}", d.z, forward.z);
    }

    /// Screen top vs bottom must tilt the ray up vs down (NDC +Y is up in WGPU),
    /// and screen right vs left must tilt along the camera's right axis. These
    /// monotonic invariants catch a flipped/ swapped unproject axis.
    #[test]
    fn build_ray_screen_edges_tilt_monotonically() {
        let mut world = World::new();
        let cam_id = spawn_camera(&mut world, Vec3::ZERO, -FRAC_PI_2, 0.0);

        let up = build_ray(&world, cam_id, 0.0, 0.5, 1.0, 1.0).unwrap().direction;
        let down = build_ray(&world, cam_id, 0.0, -0.5, 1.0, 1.0).unwrap().direction;
        assert!(up.y > down.y, "top-of-screen ray must aim higher: {} vs {}", up.y, down.y);

        // For yaw=-PI/2 the camera-right axis is +X, so screen-right tilts +X.
        let right = build_ray(&world, cam_id, 0.5, 0.0, 1.0, 1.0).unwrap().direction;
        let left = build_ray(&world, cam_id, -0.5, 0.0, 1.0, 1.0).unwrap().direction;
        assert!(right.x > left.x, "screen-right ray must aim +X: {} vs {}", right.x, left.x);
    }

    /// build_ray short-circuits to None when the entity lacks a Camera OR a Transform
    /// (both `?` early-returns), instead of fabricating a bogus ray.
    #[test]
    fn build_ray_none_without_camera_or_transform() {
        let mut world = World::new();

        let no_cam = world.spawn();
        world.add_component(no_cam, Transform::new(Vec3::ZERO));
        assert!(build_ray(&world, no_cam.id(), 0.0, 0.0, 1.0, 1.0).is_none());

        let no_tf = world.spawn();
        world.add_component(no_tf, make_camera(0.0, 0.0));
        assert!(build_ray(&world, no_tf.id(), 0.0, 0.0, 1.0, 1.0).is_none());

        // Entirely unknown id.
        assert!(build_ray(&world, 987_654, 0.0, 0.0, 1.0, 1.0).is_none());
    }

    // ── find_selection_root (Blender-style parent walk) ─────────────────────

    #[test]
    fn find_selection_root_walks_to_topmost_ancestor() {
        let mut world = World::new();
        let a = world.spawn(); // root
        let b = world.spawn();
        let c = world.spawn();
        world.add_component(b, Parent(a.id()));
        world.add_component(c, Parent(b.id()));

        // A root resolves to itself.
        assert_eq!(find_selection_root(&world, a.id()), a.id());
        // A direct child resolves to its parent.
        assert_eq!(find_selection_root(&world, b.id()), a.id());
        // A grandchild walks the whole chain up to the root.
        assert_eq!(find_selection_root(&world, c.id()), a.id());
    }

    /// A pathological Parent cycle must NOT hang: the 32-level cap bounds the walk.
    /// The test completing at all proves termination; the result is one of the two.
    #[test]
    fn find_selection_root_cycle_is_bounded_no_hang() {
        let mut world = World::new();
        let a = world.spawn();
        let b = world.spawn();
        world.add_component(a, Parent(b.id()));
        world.add_component(b, Parent(a.id()));

        let root = find_selection_root(&world, a.id());
        assert!(root == a.id() || root == b.id());
    }

    // ── is_editor_entity (unpickable-object whitelist) ──────────────────────

    #[test]
    fn is_editor_entity_matches_names_and_prefixes() {
        let mut world = World::new();
        let named = |world: &mut World, name: &str| {
            let e = world.spawn();
            world.add_component(e, EntityName(name.to_string()));
            e.id()
        };

        for exact in ["Editor Grid", "Editor Guidelines", "Directional Light"] {
            let id = named(&mut world, exact);
            assert!(is_editor_entity(&world, id), "'{exact}' should be an editor entity");
        }
        // Prefix rule: any "Editor Light Icon*".
        let icon = named(&mut world, "Editor Light Icon 3");
        assert!(is_editor_entity(&world, icon));

        // Regular user objects are selectable.
        let cube = named(&mut world, "Default Cube");
        assert!(!is_editor_entity(&world, cube));

        // An entity with no name is never an editor entity.
        let anon = world.spawn();
        assert!(!is_editor_entity(&world, anon.id()));
    }

    // ── compute_entity_obb (collider fallback path — no GPU mesh needed) ─────

    fn obb_of(world: &World, id: u32) -> Option<super::ObbInfo> {
        let transforms = world.borrow::<Transform>();
        let meshes = world.borrow::<gizmo::renderer::components::Mesh>();
        let colliders = world.borrow::<Collider>();
        let gts = world.borrow::<GlobalTransform>();
        let t = transforms.get(id).unwrap();
        compute_entity_obb(id, t, &meshes, &colliders, &gts)
    }

    #[test]
    fn compute_obb_from_box_collider_uses_transform() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::new(5.0, 0.0, 0.0)));
        world.add_component(e, Collider::box_collider(Vec3::new(0.5, 0.5, 0.5)));

        let obb = obb_of(&world, e.id()).expect("collider yields an OBB");
        assert!((obb.center - Vec3::new(5.0, 0.0, 0.0)).length() < 1e-4);
        assert!((obb.half_extents - Vec3::new(0.5, 0.5, 0.5)).length() < 1e-4);
        // No GlobalTransform present → OBB inherits the identity local rotation.
        let dot = obb.rotation.dot(Quat::IDENTITY).abs();
        assert!((dot - 1.0).abs() < 1e-4, "rotation should be identity, dot={dot}");
    }

    /// Non-uniform scale must multiply the half-extents component-wise while leaving
    /// the (zero-offset) collider center anchored at the entity position.
    #[test]
    fn compute_obb_scales_half_extents() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(
            e,
            Transform::new(Vec3::new(5.0, 0.0, 0.0)).with_scale(Vec3::new(2.0, 3.0, 4.0)),
        );
        world.add_component(e, Collider::box_collider(Vec3::new(0.5, 0.5, 0.5)));

        let obb = obb_of(&world, e.id()).unwrap();
        assert!((obb.half_extents - Vec3::new(1.0, 1.5, 2.0)).length() < 1e-4);
        // Center unaffected by scale because the collider offset is zero.
        assert!((obb.center - Vec3::new(5.0, 0.0, 0.0)).length() < 1e-4);
    }

    #[test]
    fn compute_obb_none_without_mesh_or_collider() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::ZERO));
        assert!(obb_of(&world, e.id()).is_none());
    }

    // ── Rubber-band world→screen projection (mirror of
    //    perform_rubber_band_selection). Guards the NDC→pixel mapping, whose Y axis
    //    is FLIPPED (top-left origin). Kept as a formula mirror because the real fn
    //    is welded to World + camera matrices; the arithmetic is the fragile part.
    fn ndc_to_screen(ndc_x: f32, ndc_y: f32, rect: (f32, f32, f32, f32)) -> (f32, f32) {
        let (left, top, width, height) = rect;
        let sx = ((ndc_x + 1.0) / 2.0) * width + left;
        let sy = ((1.0 - ndc_y) / 2.0) * height + top;
        (sx, sy)
    }

    #[test]
    fn rubber_band_projection_center_and_flipped_y() {
        // Scene rect: origin (100,50), size 800x600.
        let rect = (100.0, 50.0, 800.0, 600.0);

        // NDC center → rect center.
        let (cx, cy) = ndc_to_screen(0.0, 0.0, rect);
        assert!((cx - 500.0).abs() < 1e-3);
        assert!((cy - 350.0).abs() < 1e-3);

        // NDC top (+1 y) maps to the TOP pixel row (min y) because y is flipped.
        let (_, top_y) = ndc_to_screen(0.0, 1.0, rect);
        assert!((top_y - 50.0).abs() < 1e-3, "top of NDC must map to top pixel: {top_y}");
        // NDC bottom (-1 y) maps to the BOTTOM pixel row.
        let (_, bot_y) = ndc_to_screen(0.0, -1.0, rect);
        assert!((bot_y - 650.0).abs() < 1e-3, "bottom of NDC must map to bottom pixel: {bot_y}");

        // NDC left/right map to the rect's left/right edges (x is NOT flipped).
        assert!((ndc_to_screen(-1.0, 0.0, rect).0 - 100.0).abs() < 1e-3);
        assert!((ndc_to_screen(1.0, 0.0, rect).0 - 900.0).abs() < 1e-3);
    }
}
