//! Göller ve kayık.
//!
//! Arazinin yüksekliğinde geniş, alçak çanaklar var (`basin`); su seviyesinin (`WATER`) altında
//! kalan her yer göl. Su tek bir motor `FluidZone`'u: dünyanın her yerinde `WATER`'a kadar — kuru
//! yerde zaten kimse o yükseklikte değil, ve çukur nerede olursa olsun su orada. Suyun taşıdığı her
//! şey ona göre: kayık yüzüyor, oyuncu yüzüyor (motorun karakter denetleyicisinin yüzme kipi), suya
//! giren araba da yüzüyor.
//!
//! Gölü olan her hücrenin kıyısında, ilk yüklendiğinde bir kayık: motorun `BoatController`'ı —
//! kıçta bir pervane, dümen, omurga — `kayik` oyununun teknesi. Arabalar gibi **E** ile binilir,
//! **W/S** gaz, **A/D** dümen. Kayık bir hücreye ait değil: uzaklaşınca silinmiyor, hücresi
//! yüklü değilken motorun hücre sistemi onu yerinde bekletiyor.

use super::*;
use gizmo::physics::world::{FluidZone, ZoneShape};
use gizmo::physics::BoatController;

/// Su seviyesi, metre.
pub(super) const WATER: f32 = 3.5;

/// Kayığın gövdesi (yarı boyutlar) ve yoğunluğu: modelin (Kenney'nin sürat teknesi, 1,15 kat)
/// teknesi — 2,05 m eninde, 3,9 m boyunda.
const HULL: Vec3 = Vec3::new(0.95, 0.3, 1.9);
const HULL_DENSITY: f32 = 350.0;
const THRUST: f32 = 9000.0;

/// Göl çanağının derinliği bu noktada (yüksekliğe eksi olarak eklenir): geniş, yumuşak kenarlı
/// çukurlar, başlangıcın çevresinde hiç.
pub(super) fn basin(x: f32, z: f32) -> f32 {
    let mask = fbm(x / 300.0 + 17.3, z / 300.0 - 4.1, 3, 55);
    let t = ((mask - 0.5) / 0.12).clamp(0.0, 1.0);
    let smooth = t * t * (3.0 - 2.0 * t);
    let away = ((Vec2::new(x, z).length() - 70.0) / 60.0).clamp(0.0, 1.0);
    smooth * 10.0 * away
}

/// Dünyanın suyu.
pub(super) fn install(physics: &mut PhysicsWorld) {
    physics.fluid_zones.push(FluidZone {
        shape: ZoneShape::Box { min: Vec3::new(-1.0e6, WATER - 200.0, -1.0e6), max: Vec3::new(1.0e6, WATER, 1.0e6) },
        density: 1000.0,
        linear_drag: 150.0,
        quadratic_drag: 150.0,
        fog_color: [0.1, 0.25, 0.3],
        fog_density: 0.15,
        ..Default::default()
    });
}

/// Bu yer suyun altında mı (kıyıdan bir pay ile).
pub(super) fn wet(x: f32, z: f32, margin: f32) -> bool {
    height(x, z) < WATER + margin
}

/// Hücrenin su yüzeyi, hücrede su varsa: hücreyle gelip giden yarı saydam bir düzlem.
pub(super) fn surface(world: &mut World, coord: ChunkCoord, lowest: f32, visual: bool) -> Option<Entity> {
    if !visual || lowest >= WATER {
        return None;
    }
    let centre = ChunkManager::chunk_to_world_pos(coord);
    Some(world.spawn_bundle((
        Transform::new(Vec3::new(centre.x, WATER, centre.z)),
        GlobalTransform::default(),
        MeshSource("plane_100".into()),
        // Dalgacıklı, saydam su (`su.wgsl`): pencereli oyun gölgelendiricisini kurarken kaydetti.
        world.get_resource::<super::modeller::WaterLook>().map(|w| w.0.clone()).expect("the windowed game registers the water's surface first"),
        MeshRenderer::new(),
    )))
}

/// Kıyıda bir kayık yeri: hücrenin ızgarasında suyu 0,7–2 m derin, kıyıya en yakın nokta.
pub(super) fn boat_spot(coord: ChunkCoord) -> Option<Vec3> {
    boat_berth(coord).map(|(at, _)| at)
}

/// [`boat_spot`] ve kayığın yönü: burnu kıyıdan uzağa, açık suya (yön `−Z`'den, `R_y(yaw)`).
pub(super) fn boat_berth(coord: ChunkCoord) -> Option<(Vec3, f32)> {
    let centre = ChunkManager::chunk_to_world_pos(coord);
    let mut best: Option<(f32, Vec3, f32)> = None;
    for i in 0..20 {
        for j in 0..20 {
            let (x, z) = (centre.x - 47.5 + i as f32 * 5.0, centre.z - 47.5 + j as f32 * 5.0);
            let depth = WATER - height(x, z);
            if !(0.7..2.0).contains(&depth) {
                continue;
            }
            // Kıyıya yakın: dört yanından biri kuru.
            let sides = [(4.0f32, 0.0f32), (-4.0, 0.0), (0.0, 4.0), (0.0, -4.0)];
            let dry: Vec<Vec2> = sides.iter().filter(|(dx, dz)| !wet(x + dx, z + dz, 0.0)).map(|(dx, dz)| Vec2::new(*dx, *dz)).collect();
            if dry.is_empty() {
                continue;
            }
            // Açık su: kuru yanların tersi, ve o yönde 12 m ileride de su olsun.
            let away = -dry.iter().copied().sum::<Vec2>().normalize_or_zero();
            if away == Vec2::ZERO || !wet(x + away.x * 12.0, z + away.y * 12.0, -0.5) {
                continue;
            }
            let score = depth - dry.len() as f32;
            if best.is_none_or(|(b, _, _)| score < b) {
                best = Some((score, Vec3::new(x, WATER, z), (-away.x).atan2(-away.y)));
            }
        }
    }
    best.map(|(_, p, yaw)| (p, yaw))
}

pub(super) fn spawn_boat(world: &mut World, at: Vec3, yaw: f32, visual: bool) -> Entity {
    let collider = Collider::box_collider(HULL);
    let mut body = RigidBody::new(HULL_DENSITY * 8.0 * HULL.x * HULL.y * HULL.z, true);
    body.update_inertia_from_collider(&collider);
    let boat = world.spawn_bundle((
        Transform::new(at + Vec3::Y * 0.1).with_rotation(Quat::from_rotation_y(yaw)),
        GlobalTransform::default(),
        body,
        Velocity::default(),
        collider,
        BoatController::new(THRUST, HULL.z),
    ));
    if visual {
        // Modelin dibi gövdenin dibinde; burnu modelde +Z, kayığın önü −Z.
        super::modeller::place(world, boat, super::modeller::BOAT, Vec3::new(0.0, -HULL.y, 0.0), Quat::from_rotation_y(std::f32::consts::PI), Vec3::splat(1.15));
    }
    boat
}

/// Yeni yüklenen hücrelerde, daha önce kayık konmadıysa, kıyıya bir kayık.
pub(super) fn place_boats(world: &mut World, game: &mut Game) {
    let active: Vec<ChunkCoord> = world.get_resource::<ChunkManager>().map(|m| m.active_chunks.iter().copied().collect()).unwrap_or_default();
    for coord in active {
        if !game.boat_chunks.insert(coord) {
            continue;
        }
        if let Some((at, yaw)) = boat_berth(coord) {
            let boat = spawn_boat(world, at, yaw, game.visual);
            game.boats.push(boat);
        }
    }
}

/// Bir kayığın sürülüşü: W/S gaz, A/D dümen (artısı sağa, motorun kuralı).
pub(super) fn drive(world: &mut World, boat: Entity, throttle: f32, rudder: f32) -> bool {
    let mut boats = world.query_mut::<&mut BoatController>();
    let Some(mut b) = boats.get_mut(boat) else { return false };
    b.throttle_input = throttle;
    b.rudder_input = rudder;
    true
}

/// Kayık mı.
pub(super) fn is_boat(world: &World, e: Entity) -> bool {
    world.get::<BoatController>(e).is_some()
}
