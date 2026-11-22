//! Vehicle simulation: the per-frame `update_vehicle` step (suspension raycast, Pacejka tire
//! forces, drivetrain/steering, aero + ground-effect + weather grip) and its helpers, plus the
//! behaviour tests. Extracted verbatim from the former 1562-line `vehicle.rs` (pure move); the
//! data types (Wheel/Drivetrain/VehicleController/PacejkaParams/…) stay in the parent module and
//! arrive via `use super::*`.

use super::*;

// ============================================================
// ARAÇ GÜNCELLEME FONKSİYONU
// ============================================================

/// Per-wheel Ackermann steering angle.
///
/// `turn_radius = wheelbase / tan(steer_angle)` is *signed*: with `+Y` up, `-Z`
/// forward and `+X` right, a positive `steer_angle` steers the wheels left, giving
/// a positive `turn_radius` and putting the turn centre on the car's left, so the
/// **left** wheel is the inner one. The inner wheel traces the smaller radius
/// (`turn_radius - track/2`) and therefore steers *more*; the outer wheel uses
/// `turn_radius + track/2`. Beyond a near-straight threshold the nominal angle is
/// returned unchanged.
#[inline]
fn ackermann_steering_angle(
    steer_angle: f32,
    turn_radius: f32,
    wheelbase: f32,
    track_width: f32,
    is_left: bool,
) -> f32 {
    if turn_radius.abs() < 1e4 {
        // Inner wheel (left on a left turn) subtracts half-track → tighter angle.
        let sign = if is_left { -1.0 } else { 1.0 };
        (wheelbase / (turn_radius + sign * track_width * 0.5)).atan()
    } else {
        steer_angle
    }
}

/// Anti-roll bar (sway bar) corrective vertical force for one wheel on an axle.
///
/// `diff = travel_left - travel_right`, where `travel` is suspension
/// *compression*. A positive `diff` means the left corner is more compressed
/// (lower) than the right — i.e. the body is rolling toward the left. The
/// returned value is *added* into `suspension_force`, which pushes the chassis
/// **up** at that corner. To RESIST the roll, the bar must add up-force on the
/// lower (more-compressed) corner and remove it from the higher corner, so the
/// left wheel gets `+diff` and the right `-diff`.
///
/// The previous code had both signs flipped (`-diff` on the left, `+diff` on the
/// right), which *amplified* roll: it pushed the already-high inner corner up and
/// the already-low outer corner down, so raising `anti_roll_stiffness` — which a
/// user does to reduce roll — made cornering roll and stability worse.
#[inline]
fn anti_roll_force(is_left: bool, diff: f32, stiffness: f32) -> f32 {
    let signed = if is_left { diff } else { -diff };
    signed * stiffness
}

/// The "dynamic pressure" the lift term uses: `½ρ·v^n` for [`AeroPackage::downforce_speed_exponent`]
/// `n`. For `n == 2` it returns `q` — the drag term's own `½ρ·v·v` — unchanged, because `powf(2.0)`
/// is not guaranteed to be bit-identical to `v·v`, and the default must not move a single bit.
fn lift_pressure(exponent: f32, speed: f32, q: f32) -> f32 {
    if exponent == 2.0 {
        q
    } else {
        0.5 * 1.225 * speed.powf(exponent)
    }
}

/// The ground-effect downforce multiplier: as the chassis `clearance` (the vertical gap between
/// the body's underside and the ground, in metres) shrinks, it ramps SMOOTHLY from `1.0` to
/// `multiplier`. `clearance ≥ height` → 1.0 (no effect); `clearance = 0` → `multiplier`; linear
/// in between. Negative clearance (the body inside the ground) clamps to `multiplier`.
fn ground_effect_factor(clearance: f32, height: f32, multiplier: f32) -> f32 {
    let t = (1.0 - clearance.max(0.0) / height.max(1e-3)).clamp(0.0, 1.0);
    1.0 + (multiplier - 1.0) * t
}

/// The weather grip multiplier — a port of the logic from the rigid vehicle system (which is
/// being deleted). Sunny → 1.0, Snow → 0.3, Rain → a 50 % floor with a gradual fall-off above
/// 20 m/s as it starts to aquaplane. `speed_mps` is the vehicle's speed in m/s. Applied as a
/// multiplier on the friction-circle limit (see update_vehicle, second pass).
pub fn weather_grip_factor(weather: Weather, speed_mps: f32) -> f32 {
    match weather {
        Weather::Sunny => 1.0,
        Weather::Rain => {
            if speed_mps > 20.0 {
                (0.5 - (speed_mps - 20.0) * 0.01).max(0.1)
            } else {
                0.5
            }
        }
        Weather::Snow => 0.3,
        _ => 1.0, // Weather #[non_exhaustive] — bilinmeyen/ileride eklenen → cezasız (tam grip).
    }
}

// 8 argüman: all_colliders + weather_grip + dt hepsi per-step ÇEVRE girdisi. CI zaten
// `too_many_arguments`'ı `-A` ile muaf tutuyor; alternatif küçük bir `VehicleStepCtx` struct'ı
// (colliders+weather_grip+dt) — ileride çevre girdisi artarsa (rüzgâr, yüzey sıcaklığı) tercih.
/// Advances one vehicle by a single fixed step of `dt` seconds.
///
/// Runs the whole model in order: drivetrain and automatic shifting, aerodynamics, Ackermann
/// steering, one suspension raycast per wheel, then per-corner spring/damper/bump-stop/
/// anti-roll forces and combined-slip Pacejka tyre forces.
///
/// # What it writes
///
/// Every force is summed as `force · inv_mass · dt` (and the matching angular term) and added to
/// `vehicle_vel` once, at the end, so every wheel reads the body's velocity at the start of the
/// step and the result does not depend on the wheels' order; it is an explicit force-to-velocity
/// step and `dt`-dependent. Two jobs are left to the surrounding pipeline, NOT to the
/// immediate caller: integrating `vehicle_transform` from the resulting velocity — this
/// function takes the transform by shared reference and never moves the car — and applying
/// **gravity**, which is not applied here. In this engine both happen later in the schedule,
/// in `gizmo-physics-rigid`'s integrator; `vehicle_controller_system` does neither, so do not
/// add gravity in a system of your own or it is applied twice.
///
/// `vehicle` is mutated in place: `engine_rpm`, `current_speed_kmh` and `current_gear` are
/// recomputed, and on every wheel the `drive_torque`, `brake_torque`, front-wheel
/// `steering_angle` and all the output state (`is_grounded`, `ground_hit`,
/// `suspension_length`, `suspension_force`, `surface_friction`, `angular_velocity`,
/// `rotation_angle`) are overwritten.
///
/// `vehicle_rb` is taken by `&mut` **only** to wake the body; its mass, inertia and centre of
/// mass are read, never modified.
///
/// # Parameters
///
/// `all_colliders` is what the suspension rays are cast against — anything absent from it is
/// invisible to the wheels, including terrain. Entries whose handle equals `vehicle_entity`
/// are skipped, as are triggers, so the car cannot raycast itself; that also means a stale
/// transform for the vehicle's own entry is harmless, while a stale transform for the ground
/// is not. Each wheel scans the whole slice, AABB-rejecting first, so cost is
/// `wheels × colliders` — which is why
/// [`update_vehicle_with_query`] exists and why the ECS driver
/// [`vehicle_controller_system`](crate::systems::vehicle_controller_system) no longer calls
/// this entry point when it has a broadphase to query. This one is unchanged, and stays: it
/// is the ECS-free API, and the only way to put a wheel on a collider the rigid pipeline does
/// not simulate.
///
/// `weather_grip` multiplies every tyre force (1.0 = dry, no penalty); see
/// [`weather_grip_factor`] for the mapping this is normally fed from. It stacks with the
/// per-wheel surface friction sampled from whatever the ray hit.
///
/// # Sleep and early exits
///
/// Returns immediately, doing nothing at all, if the body is static.
///
/// A sleeping body is woken first, but only when there is throttle, brake or steering input —
/// waking unconditionally would mean a parked car never sleeps. If the body is *still* asleep
/// after that check the function returns before touching any force — no suspension, drive or
/// aero force is accumulated into `vehicle_vel` while the body sleeps.
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(
    skip_all,
    level = "trace",
    name = "update_vehicle",
    fields(entity = ?vehicle_entity, weather_grip)
)]
pub fn update_vehicle(
    vehicle_entity: BodyHandle,
    vehicle: &mut VehicleController,
    vehicle_rb: &mut RigidBody,
    vehicle_transform: &Transform,
    vehicle_vel: &mut Velocity,
    all_colliders: &[(BodyHandle, Transform, Collider)],
    weather_grip: f32,
    dt: f32,
) {
    update_vehicle_with_query(
        vehicle_entity,
        vehicle,
        vehicle_rb,
        vehicle_transform,
        vehicle_vel,
        &ColliderListQuery::new(all_colliders),
        weather_grip,
        dt,
    );
}

/// [`update_vehicle`], with the suspension rays answered by an arbitrary
/// [`WheelGroundQuery`] instead of a collider list.
///
/// This is the same simulation — identical drivetrain, aero, suspension and tyre maths, and
/// identical wheel state written back. The *only* difference is who answers "what is under
/// this wheel": `update_vehicle` scans a slice, this asks `ground`, and a [`PhysicsWorld`] is
/// a [`WheelGroundQuery`] that answers through the broadphase.
///
/// One call per wheel, per step. `ground` sees a ray whose length already includes the
/// half-metre origin offset the suspension model adds (see the body), so an implementation
/// must not add its own margin.
///
/// [`PhysicsWorld`]: gizmo_physics_rigid::world::PhysicsWorld
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(
    skip_all,
    level = "trace",
    name = "update_vehicle_with_query",
    fields(entity = ?vehicle_entity, weather_grip)
)]
pub fn update_vehicle_with_query(
    vehicle_entity: BodyHandle,
    vehicle: &mut VehicleController,
    vehicle_rb: &mut RigidBody,
    vehicle_transform: &Transform,
    vehicle_vel: &mut Velocity,
    ground: &dyn WheelGroundQuery,
    weather_grip: f32,
    dt: f32,
) {
    if vehicle_rb.is_static() {
        tracing::trace!(entity = ?vehicle_entity, "[Vehicle] static rigid body — skipping update");
        return;
    }

    // Bir sürücü girdisi varsa gövdeyi UYANDIR.
    //
    // Duran bir araba ~1 saniye sonra uyur ve `integrator` uyuyan gövdeyi hiç entegre etmez. Bu
    // sistem ise uyusun uyumasın çalışmaya devam eder: tork üretir, `current_speed_kmh` yazar,
    // tekerlekleri döndürür. Sonuç, dışarıdan bakınca fizik motorunun bozulmuş gibi görünmesidir —
    // gaza basarsın, devir yükselir, hız göstergesi 54 km/h yazar, tekerler döner ve araba
    // milimetre kıpırdamaz. (Tam olarak böyle bir hata raporuyla bulundu.)
    //
    // Uyandırmayı *girdiye* bağlamak önemli: koşulsuz uyandırmak park etmiş bir aracın asla
    // uyumaması demek olurdu ve uyku sistemi tam da bunun için var. Frenin de sayılması, el freni
    // çekili bir araca çarpıldığında değil, sürücü frene bastığında gövdenin uyanması içindir.
    let driver_input = vehicle.throttle_input.abs() > f32::EPSILON
        || vehicle.brake_input.abs() > f32::EPSILON
        || vehicle.steering_input.abs() > f32::EPSILON;
    if driver_input && vehicle_rb.is_sleeping {
        tracing::debug!(entity = ?vehicle_entity, "[Vehicle] driver input woke a sleeping chassis");
        vehicle_rb.wake_up();
    }

    // Hâlâ uyuyorsa: HİÇBİR ŞEY yapma.
    //
    // Uyandırma tek başına yetmiyor, ve yetmediği çok net bir şekilde görüldü. Bu fonksiyon
    // eskiden uyuyan bir gövde için de sonuna kadar koşuyordu: süspansiyon kuvvetini, yerçekimini
    // ve tahrik torkunu `vehicle_vel`'e **biriktiriyordu**. Entegratör uyuyan gövdeyi taşımadığı
    // için o hız hiç harcanmıyor, sadece büyüyordu — ölçüldü, bir dakika park eden bir arabada
    // 15 m/s'ye ulaşmıştı. Uyandırma eklendiğinde o birikim tek karede serbest kaldı ve araba
    // havalanıp takla attı. İlk hata "gaz veriyorum gitmiyor", ikincisi "araba uçtu": aynı hatanın
    // iki yarısı.
    //
    // Erken çıkış ikisini birden kapatıyor: uyuyan araca kuvvet uygulanmaz, uyanan araç durgun
    // halden başlar. `is_grounded` gibi okunan alanlar son uyanık karenin değerinde kalır, ki bir
    // park etmiş araba için doğru olan da budur.
    if vehicle_rb.is_sleeping {
        return;
    }

    // Yerel eksenler
    let up = vehicle_transform
        .rotation
        .mul_vec3(Vec3::new(0.0, 1.0, 0.0));
    let forward = vehicle_transform
        .rotation
        .mul_vec3(Vec3::new(0.0, 0.0, -1.0));
    let right = vehicle_transform
        .rotation
        .mul_vec3(Vec3::new(1.0, 0.0, 0.0));

    let v_com = vehicle_vel.linear;
    // **Every force of the step is summed here and applied once, at the end** (Jacobi). They used
    // to go straight into `vehicle_vel`, wheel after wheel, and the next wheel read its contact
    // velocity from what the one before had just changed (Gauss-Seidel): the result depended on the
    // wheels' order, no order of four corners is its own mirror, and a symmetric car turned left
    // and right differently — 5 % of lateral grip on a front-driven car
    // (`left_and_right_turns_are_mirror_images`). Every wheel now reads the body's velocity at the
    // start of the step.
    let mut kick = Velocity::default();
    let forward_speed = v_com.dot(forward);
    vehicle.current_speed_kmh = forward_speed * 3.6;

    // --------------------------------------------------------
    // 1. GÜÇ AKTARMA ORGANı (1.5 otomatik vites dahil) — `drive_for_step`
    // --------------------------------------------------------
    let drive = drive_for_step(vehicle, dt);

    // --------------------------------------------------------
    // 2. AERODİNAMİK (fiziksel — ½ρCdAv²) — `apply_aerodynamics`
    // --------------------------------------------------------
    let com = vehicle_transform.position
        + vehicle_transform
            .rotation
            .mul_vec3(vehicle_rb.center_of_mass);
    let ge_factor = apply_aerodynamics(vehicle, vehicle_rb, vehicle_transform, v_com, up, com, &mut kick, dt);

    // --------------------------------------------------------
    // 3. ACKERMANN DİREKSİYON
    // --------------------------------------------------------
    // The input's positive is right (what a stick or the D key gives); the geometry below works
    // in yaw, where positive turns the wheels left about +Y — hence the sign.
    let steer_angle = -vehicle.steering_input * vehicle.max_steering_angle;
    let turn_radius = if steer_angle.abs() > 0.01 {
        vehicle.tuning.wheelbase / steer_angle.tan()
    } else {
        f32::MAX
    };

    // --------------------------------------------------------
    // 4. TEKERLEK DÖNGÜSÜ — 1. geçiş: Raycast + Süspansiyon setup — `place_wheels`
    // --------------------------------------------------------
    place_wheels(vehicle, vehicle_entity, vehicle_transform, ground, &drive, steer_angle, turn_radius);

    // --------------------------------------------------------
    // 5. Anti-roll bar farkları — `anti_roll_diffs`
    // --------------------------------------------------------
    let (front_diff, rear_diff) = anti_roll_diffs(&vehicle.wheels);

    // --------------------------------------------------------
    // 6. TEKERLEK DÖNGÜSÜ — 2. geçiş: Kuvvetler + Tekerlek integrasyon
    // --------------------------------------------------------
    // How many contacts share stopping the body (the tyre-force limit below); the rays above have
    // set `is_grounded`.
    let step = WheelStep {
        vehicle_entity,
        vehicle_transform,
        vehicle_vel: *vehicle_vel,
        up,
        forward,
        right,
        com,
        front_diff,
        rear_diff,
        tuning: &vehicle.tuning,
        brake_input: vehicle.brake_input,
        drive: &drive,
        // Motor/volan ataleti — sürülen tekerlekte gear² ile yansır (`step_wheel`).
        flywheel_inertia: vehicle.flywheel_inertia,
        // How many contacts share stopping the body (the tyre-force limit); the rays above have
        // set `is_grounded`.
        grounded_share: vehicle.wheels.iter().filter(|w| w.is_grounded).count().max(1) as f32,
        weather_grip,
        dt,
    };
    for wheel in &mut vehicle.wheels {
        step_wheel(wheel, &step, vehicle_rb, &mut kick);
    }

    // Per-step özet (yalnız trace etkinken alanlar hesaplanır — tracing makro seviye-kapılıdır).
    vehicle_vel.linear += kick.linear;
    vehicle_vel.angular += kick.angular;

    tracing::trace!(
        entity = ?vehicle_entity,
        speed_kmh = vehicle.current_speed_kmh,
        engine_rpm = vehicle.engine_rpm,
        gear = vehicle.current_gear,
        grounded_wheels = vehicle.wheels.iter().filter(|w| w.is_grounded).count(),
        wheel_count = vehicle.wheels.len(),
        ge_factor,
        "[Vehicle] step complete"
    );
}

/// What the drivetrain hands the wheels for one step.
struct Drive {
    drivetrain: Drivetrain,
    /// The gear's ratio times the final drive's (negative in reverse).
    total_ratio: f32,
    /// How many wheels are driven, at least 1: they share the torque, the engine brake and the
    /// reflected flywheel inertia.
    driven_count_f: f32,
    drive_torque_total: f32,
    engine_brake_at_wheels: f32,
}

/// The drivetrain for one step (section 1): the engine's revs from the driven wheels, the torque
/// and the engine brake at the wheels, then the automatic gearbox's tick.
fn drive_for_step(vehicle: &mut VehicleController, dt: f32) -> Drive {
    let drivetrain = vehicle.drivetrain;

    // --------------------------------------------------------
    // 1. GÜÇ AKTARMA ORGANı
    // --------------------------------------------------------
    let gear_ratio = vehicle
        .tuning
        .gear_ratios
        .get(vehicle.current_gear)
        .copied()
        .unwrap_or(0.0);
    let total_ratio = gear_ratio * vehicle.tuning.final_drive_ratio;

    // RPM ← TAHRİKLİ tekerlek angular_velocity ortalamasından (FWD/RWD/AWD)
    let mut avg_driven_ω = 0.0f32;
    let mut driven_count = 0.0f32;
    for w in &vehicle.wheels {
        if drivetrain.drives(&w.axle_type) {
            avg_driven_ω += w.angular_velocity;
            driven_count += 1.0;
        }
    }
    if driven_count > 0.0 {
        avg_driven_ω /= driven_count;
    }

    let wheel_rpm = avg_driven_ω.abs() * 9.549; // rad/s → rpm
    vehicle.engine_rpm =
        (wheel_rpm * total_ratio.abs()).clamp(vehicle.tuning.idle_rpm, vehicle.tuning.max_rpm);

    vehicle.shift_remaining = (vehicle.shift_remaining - dt).max(0.0);
    let engine_torque = vehicle.engine_torque();
    // Engine braking at the driven wheels, against their spin like a brake (it never drives a
    // stopped car backwards): the crank's drag through the gearbox, shared out like the torque.
    let engine_brake_at_wheels = vehicle.engine_brake_torque() * total_ratio.abs();
    // Geri viteste tork yönü ters
    let torque_sign = if total_ratio < 0.0 { -1.0 } else { 1.0 };
    let drive_torque_total = engine_torque * total_ratio.abs() * torque_sign;

    // --------------------------------------------------------
    // 1.5 Otomatik vites
    // --------------------------------------------------------
    vehicle.auto_shift_tick(dt);
    Drive {
        drivetrain,
        total_ratio,
        driven_count_f: driven_count.max(1.0),
        drive_torque_total,
        engine_brake_at_wheels,
    }
}

/// The aerodynamic drag and lift (section 2: `½ρCdAv²`, with ground effect from the suspension's
/// compression), applied at the centre of pressure into `kick`. Returns the ground-effect factor.
#[allow(clippy::too_many_arguments)]
fn apply_aerodynamics(
    vehicle: &VehicleController,
    vehicle_rb: &RigidBody,
    vehicle_transform: &Transform,
    v_com: Vec3,
    up: Vec3,
    com: Vec3,
    kick: &mut Velocity,
    dt: f32,
) -> f32 {
    const AIR_DENSITY: f32 = 1.225; // kg/m³
    let spd = v_com.length();
    let spd_sq = spd * spd;
    let a = &vehicle.tuning.aero;
    let q = 0.5 * AIR_DENSITY * spd_sq; // dinamik basınç

    // Zemin etkisi: araç gövdesi yere yaklaştıkça downforce artar. Referans = SÜSPANSİYON
    // SIKIŞMASI (gerçek dinamik ride-height). ESKİDEN şasi collider AABB tabanı kullanılıyordu;
    // o değer collider yarı-yüksekliğine hâkim olup yalnız cm-ölçekte oynadığından ge_factor
    // collider boyutuna kilitleniyor, yüke DUYARSIZ kalıyordu (ya hep max ya hep 1.0). Süspansiyon
    // sıkışma oranı (0 = rest, 1 = tam sıkışma) yükü/downforce'u doğrudan yansıtır: araç yüklenip
    // çöktükçe clearance düşer → ge_factor rampalanır (max_travel'a kırpılı → sınırlı geri besleme).
    let (sum_frac, n_grounded) = vehicle.wheels.iter().fold((0.0f32, 0.0f32), |(s, n), w| {
        if w.is_grounded {
            let travel = (w.suspension_rest_length - w.suspension_length).max(0.0);
            let frac = (travel / w.suspension_max_travel.max(1e-3)).clamp(0.0, 1.0);
            (s + frac, n + 1.0)
        } else {
            (s, n)
        }
    });
    let ge_factor = if n_grounded > 0.0 {
        let avg_frac = sum_frac / n_grounded; // 0 (rest) .. 1 (tam sıkışma)
        // clearance: rest → ge_height (etki yok), tam sıkışma → 0 (max etki)
        let clearance = a.ground_effect_height * (1.0 - avg_frac);
        ground_effect_factor(clearance, a.ground_effect_height, a.ground_effect_multiplier)
    } else {
        1.0
    };

    let drag_dir = if spd > 0.1 { -v_com / spd } else { Vec3::ZERO };
    let drag_coefficient = match a.drag_coefficient_off_throttle {
        Some(closed) => closed + (a.drag_coefficient - closed) * vehicle.throttle_input.abs().min(1.0),
        None => a.drag_coefficient,
    };
    let drag_force = drag_dir * (drag_coefficient * a.frontal_area * q);
    let q_lift = lift_pressure(a.downforce_speed_exponent, spd, q);
    let lift_force = up * (a.lift_coefficient * a.frontal_area * q_lift * ge_factor);

    // Aero kuvvetini basınç merkezinden uygula (tork üretir)
    let cop_world =
        vehicle_transform.position + vehicle_transform.rotation.mul_vec3(a.center_of_pressure);
    apply_force_at_point(
        vehicle_rb,
        kick,
        com,
        vehicle_transform.rotation,
        drag_force + lift_force,
        cop_world,
        dt,
    );
    ge_factor
}

/// Each wheel's ray to the ground and its suspension's length, its Ackermann angle, and its share
/// of the drive torque and the brake (section 4).
fn place_wheels(
    vehicle: &mut VehicleController,
    vehicle_entity: BodyHandle,
    vehicle_transform: &Transform,
    ground: &dyn WheelGroundQuery,
    drive: &Drive,
    steer_angle: f32,
    turn_radius: f32,
) {
    let Drive { drivetrain, driven_count_f, drive_torque_total, engine_brake_at_wheels, .. } = *drive;
    for wheel in &mut vehicle.wheels {
        let attach_world = vehicle_transform.position
            + vehicle_transform
                .rotation
                .mul_vec3(wheel.attachment_local_pos);
        let ray_dir = vehicle_transform
            .rotation
            .mul_vec3(wheel.direction_local)
            .normalize();

        // Ray origin'i attach_world'den biraz geriye al (yukarıya) ki araç yere tam oturduğunda
        // raycast origin'i yerin içinde kalıp çarpışmayı kaçırmasın!
        let ray_origin_offset = 0.5;
        let ray_start = attach_world - ray_dir * ray_origin_offset;
        let ray_max = wheel.suspension_rest_length
            + wheel.radius
            + wheel.suspension_max_travel
            + ray_origin_offset;
        let ray = Ray::new(ray_start, ray_dir);

        // Süspansiyon raycast'i — TEK sorgu, tekerlek başına. `ground` bir `PhysicsWorld` ise
        // bu, BVH üzerinden `raycast_filtered`; bir `ColliderListQuery` ise eski lineer tarama.
        // İki durumda da kasa (`vehicle_entity`) ve trigger'lar hariç, en yakın isabet döner.
        // Çarpılan zeminin sürtünmesi de aynı sorgudan gelir (grip çarpanı için): `RaycastHit`
        // lastik-agnostik olduğundan materyal ayrı taşınır.
        let was_grounded = wheel.is_grounded;
        if let Some(ground_hit) = ground.cast_wheel_ray(&ray, ray_max, vehicle_entity) {
            wheel.is_grounded = true;
            wheel.surface_friction = ground_hit.dynamic_friction;

            // Gerçek mesafe için eklediğimiz offseti çıkarıyoruz
            let actual_dist = ground_hit.hit.distance - ray_origin_offset;
            wheel.ground_hit = Some(ground_hit.hit);

            // Süspansiyon sıkışması: yay uzunluğu = çarpma mesafesi - tekerlek yarıçapı
            let raw_len = (actual_dist - wheel.radius).clamp(
                wheel.suspension_rest_length - wheel.suspension_max_travel,
                wheel.suspension_rest_length + wheel.suspension_max_travel,
            );
            wheel.suspension_length = raw_len;
        } else {
            wheel.is_grounded = false;
            wheel.ground_hit = None;
            wheel.suspension_length = wheel.suspension_rest_length;
            wheel.suspension_force = 0.0;
            wheel.surface_friction = PhysicsMaterial::ASPHALT.dynamic_friction;
        }
        if was_grounded != wheel.is_grounded {
            tracing::debug!(
                entity = ?vehicle_entity,
                axle = ?wheel.axle_type,
                is_left = wheel.is_left,
                grounded = wheel.is_grounded,
                suspension_length = wheel.suspension_length,
                surface_friction = wheel.surface_friction,
                "[Vehicle] wheel ground contact changed"
            );
        }

        // Ackermann açısı (ön tekerlek)
        if wheel.axle_type == Axle::Front {
            wheel.steering_angle = ackermann_steering_angle(
                steer_angle,
                turn_radius,
                vehicle.tuning.wheelbase,
                vehicle.tuning.track_width,
                wheel.is_left,
            );
        }

        // Tork dağıtımı — tahrikli akslar (FWD/RWD/AWD)
        wheel.drive_torque = if drivetrain.drives(&wheel.axle_type) {
            drive_torque_total / driven_count_f
        } else {
            0.0
        };

        // Fren dağıtımı — varsayılan %60 ön / %40 arka (`brake_bias_front/rear`).
        let bias = if wheel.axle_type == Axle::Front {
            vehicle.tuning.brake_bias_front
        } else {
            vehicle.tuning.brake_bias_rear
        };
        wheel.brake_torque = vehicle.brake_input * vehicle.tuning.max_brake_torque * bias
            + if drivetrain.drives(&wheel.axle_type) { engine_brake_at_wheels / driven_count_f } else { 0.0 };
    }
}

/// The anti-roll bars' inputs (section 5): each axle's difference in suspension travel, left minus
/// right.
fn anti_roll_diffs(wheels: &[Wheel]) -> (f32, f32) {
    let (mut fl, mut fr, mut rl, mut rr) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for w in wheels {
        let travel = w.suspension_rest_length - w.suspension_length;
        match (&w.axle_type, w.is_left) {
            (Axle::Front, true) => fl = travel,
            (Axle::Front, false) => fr = travel,
            (Axle::Rear, true) => rl = travel,
            (Axle::Rear, false) => rr = travel,
        }
    }
    let front_diff = fl - fr;
    let rear_diff = rl - rr;
    (front_diff, rear_diff)
}

/// What every wheel of one step reads: the body as the step began, its axes and centre of mass,
/// the anti-roll differences, the tuning and the driver's brake, the drivetrain's share-out.
struct WheelStep<'a> {
    vehicle_entity: BodyHandle,
    vehicle_transform: &'a Transform,
    /// The body's velocity at the start of the step: every wheel reads this one (Jacobi).
    vehicle_vel: Velocity,
    up: Vec3,
    forward: Vec3,
    right: Vec3,
    com: Vec3,
    front_diff: f32,
    rear_diff: f32,
    tuning: &'a VehicleTuning,
    brake_input: f32,
    drive: &'a Drive,
    flywheel_inertia: f32,
    grounded_share: f32,
    weather_grip: f32,
    dt: f32,
}

/// One wheel's step (section 6): its suspension and tyre forces into `kick` (6.1, 6.2), its spin
/// (6.3), the traction control and the brake's lock, and the airborne wheel's own rules.
fn step_wheel(wheel: &mut Wheel, step: &WheelStep, vehicle_rb: &RigidBody, kick: &mut Velocity) {
    let WheelStep {
        vehicle_transform,
        brake_input,
        drive,
        flywheel_inertia,
        dt,
        ..
    } = *step;
    let Drive { drivetrain, total_ratio, driven_count_f, .. } = *drive;
    let attach_world = vehicle_transform.position
        + vehicle_transform
            .rotation
            .mul_vec3(wheel.attachment_local_pos);
    let ray_dir = vehicle_transform
        .rotation
        .mul_vec3(wheel.direction_local)
        .normalize();

    // --- YAY KUVVET ENTEGRASYONu (her zaman, grounded veya değil) ---
    // Tekerlek ataletini (I = 0.5 m r²) hesapla.
    // Guard: wheel_mass=0 veya radius=0 ise inertia=0 → net_torque/wheel_inertia
    // = inf/NaN ve tüm simülasyon sessizce bozulur. Epsilon-clamp ile tek
    // noktada koruyarak aşağıdaki tüm bölmeleri (690/693/710/713) güvene al.
    let wheel_inertia = (0.5 * wheel.wheel_mass * wheel.radius.powi(2)).max(1e-6);

    // Sürülen (Rear) tekerlek motora şanzımanla kenetli → motor+volan ataleti
    // gear² ile YANSIR (I_eff = I_wheel + I_flywheel·ratio²). Bunu katmadan tekerlek
    // yalnız kendi minik ataletiyle (~0.8) anında redline'a fırlıyordu = kalkışta
    // patinaj + tüm vitesleri geçme. Yansıyan atalet (düşük viteste ~30 kg·m²) motoru
    // gerçekçi biçimde kademeli döndürür; yüksek viteste ratio küçülür → daha çevik.
    //
    // Shared, like the torque: one engine turns every driven wheel, so each carries its
    // `1/driven` share of the reflected inertia — the engine's `½·I·R²·ω²` split over the
    // wheels. Each carried all of it until 2026-11-15: a two-wheel-drive car spun up two
    // engines, ~31 kg·m² too much on each rear wheel in first gear, ~1.3 kN of the launch.
    let effective_inertia = if drivetrain.drives(&wheel.axle_type) {
        wheel_inertia + flywheel_inertia * total_ratio * total_ratio / driven_count_f
    } else {
        wheel_inertia
    };

    if wheel.is_grounded {
        if let Some(hit_point) = wheel.ground_hit.as_ref().map(|hit| hit.point) {
            step_grounded_wheel(wheel, step, vehicle_rb, kick, attach_world, ray_dir, hit_point, effective_inertia);
        }
    } else {
        step_airborne_wheel(wheel, brake_input, effective_inertia, dt);
    }

    // Viskoz spin sönümü SADECE havadaki tekerleğe (hava/rulman sürtünmesi serbest
    // dönen tekerleği yavaşça durdurur). YERDEKİ tekerlekte uygulamak, lastiğin
    // yuvarlanma-kısıtını korumak için sürekli negatif slip üretip şasiye büyük,
    // hıza-orantılı FANTOM geri-sürükleme (~600 N/tekerlek) biniyordu → düz/coast
    // sürüşte ve dönüşte hız kaybı. Yerde: lastik/yuvarlanma zaten yönetir.
    if !wheel.is_grounded {
        let damping_coeff = 2.0; // rad/s² / (rad/s)
        wheel.angular_velocity *= (1.0 - damping_coeff * dt).max(0.0);
    }

    // A slow, unpowered wheel in the AIR stops. Only in the air: on the ground the tyre owns ω
    // (it settles at ≈ v/r), and zeroing it there turned every crawling undriven wheel into a
    // locked one — a heavy car driven by one axle could not pull away against its own
    // standing wheels (`a_heavy_car_driven_by_one_axle_pulls_away`).
    if !wheel.is_grounded
        && wheel.angular_velocity.abs() < 0.05
        && wheel.drive_torque.abs() < 1.0
        && wheel.brake_torque < 1.0
    {
        wheel.angular_velocity = 0.0;
    }

    // Görsel rotasyon
    wheel.rotation_angle += wheel.angular_velocity * dt;
    wheel.rotation_angle %= std::f32::consts::TAU;
}

/// A wheel on the ground, for one step: its suspension force (6.1) and its tyre forces (6.2) into
/// `kick`, and its spin through them (6.3), the traction control and the brake's lock. `hit_point`
/// is where its ray met the ground.
#[allow(clippy::too_many_arguments)]
fn step_grounded_wheel(
    wheel: &mut Wheel,
    step: &WheelStep,
    vehicle_rb: &RigidBody,
    kick: &mut Velocity,
    attach_world: Vec3,
    ray_dir: Vec3,
    hit_point: Vec3,
    effective_inertia: f32,
) {
    let WheelStep {
        vehicle_entity,
        vehicle_transform,
        vehicle_vel,
        com,
        brake_input,
        dt,
        ..
    } = *step;
    // 6.1 Gelişmiş Süspansiyon: baskı/geri dönüş ayrı damper
    let point_rel = attach_world - vehicle_transform.position;
    let point_vel = vehicle_vel.linear + vehicle_vel.angular.cross(point_rel);
    let susp_vel = point_vel.dot(ray_dir); // pozitif = yay sıkışıyor
    wheel.suspension_force = suspension_force(wheel, step, susp_vel);
    let susp_impulse = (-ray_dir) * wheel.suspension_force;
    apply_force_at_point(
        vehicle_rb,
        kick,
        com,
        vehicle_transform.rotation,
        susp_impulse,
        attach_world,
        dt,
    );

    // 6.2 Pacejka Kuvvetleri
    let tyre = Tyre::at(wheel, step, point_vel);

    // 6.3 Tekerlek angular_velocity entegrasyonu (Semi-implicit Euler)
    let (solved_force, solved_spin) = solve_spin(wheel, &tyre, effective_inertia, dt);

    traction_control(wheel, vehicle_entity, &tyre);

    lock_braked_wheel(wheel, brake_input, effective_inertia, dt);

    let (long_force, final_lat) = settled_tyre_forces(wheel, &tyre, solved_force, solved_spin);
    let tire_force = contact_force(wheel, &tyre, step, vehicle_rb, hit_point, long_force, final_lat);
    apply_force_at_point(
        vehicle_rb,
        kick,
        com,
        vehicle_transform.rotation,
        tire_force,
        hit_point,
        dt,
    );
}

/// The suspension's push (6.1) at a compressing speed of `susp_vel`: the spring, the damper —
/// compression and rebound apart — the bump stop and the anti-roll bar, never pulling.
fn suspension_force(wheel: &Wheel, step: &WheelStep, susp_vel: f32) -> f32 {
    let WheelStep { vehicle_entity, front_diff, rear_diff, tuning, .. } = *step;
    let compression = wheel.suspension_rest_length - wheel.suspension_length;

    let spring_force = wheel.suspension_stiffness * compression;

    // Baskı: damping_compression, geri dönüş: damping_rebound (genelde 2-3x baskı)
    let damper_coeff = if susp_vel > 0.0 {
        wheel.suspension_damping // baskı katsayısı
    } else {
        // rebound — ayrı verildiyse o, yoksa eski kural: 2.5× baskı
        wheel.suspension_rebound_damping.unwrap_or(wheel.suspension_damping * 2.5)
    };
    let mut damper_force = damper_coeff * susp_vel;
    // Rebound damper'ı (2.5×) statik yay yükünü İPTAL etmesin. Açık (explicit)
    // entegratörde sert rebound damping + aşağıdaki `.max(0.0)` kırpması +
    // baskı/rebound anahtarlaması, DURAĞAN araçta küçük bir dikey limit-cycle
    // doğuruyordu: yay pozitif sıkışmada olsa bile (compression>0) rebound
    // fazında damper yayı tam götürüp `suspension_force`'u 0'a düşürüyordu.
    // Normal yük 0 → Pacejka lastik kuvveti (∝ Fz) 0 → sürülen tekerlek grip'siz
    // → tam gazda yerinde patinaj (rpm redline'a fırlar, araç KALKMAZ). Damper'ı
    // yay kuvvetinin yarısıyla tabanlayarak tekerleğin zemine basılı kalmasını
    // (Fz>0) garanti et; baskı fazını (pozitif) etkilemez, yalnız rebound'u sınırlar.
    damper_force = damper_force.max(-spring_force * 0.5);

    // Bump stop: max seyahat sonunda sert non-linear yay
    let bump_stop_travel = wheel.suspension_max_travel * 0.1;
    let bump_excess = compression - (wheel.suspension_max_travel - bump_stop_travel);
    let bump_stop_force = if bump_excess > 0.0 {
        tracing::trace!(
            entity = ?vehicle_entity,
            axle = ?wheel.axle_type,
            is_left = wheel.is_left,
            bump_excess,
            compression,
            max_travel = wheel.suspension_max_travel,
            "[Vehicle] suspension bottoming out — bump-stop engaged"
        );
        bump_excess * wheel.suspension_stiffness * 8.0
    } else {
        0.0
    };

    // Anti-roll bar — adds up-force to the more-compressed corner to
    // resist roll (see `anti_roll_force`; the sign was previously
    // inverted, amplifying roll).
    let axle_diff = match wheel.axle_type {
        Axle::Front => front_diff,
        Axle::Rear => rear_diff,
    };
    let t = tuning;
    let arb_stiffness = match wheel.axle_type {
        Axle::Front => t.anti_roll_front,
        Axle::Rear => t.anti_roll_rear,
    }
    .unwrap_or(t.anti_roll_stiffness);
    let arb_force = anti_roll_force(wheel.is_left, axle_diff, arb_stiffness);

    (spring_force + damper_force + bump_stop_force + arb_force).max(0.0)
}

/// The tyre at its contact this step (6.2): its axes, the contact's velocity along them, the slips'
/// reference speed, its slip angle and load, and the grip the surface and the weather leave it.
struct Tyre {
    /// The wheel's forward, turned by its steering.
    forward: Vec3,
    /// The wheel's right, turned by its steering.
    right: Vec3,
    /// The contact's velocity along `forward`.
    v_long: f32,
    /// The contact's velocity along `right`.
    v_lat: f32,
    /// The slips' denominator: `|v_long|`, never under 0.5 m/s.
    ref_vel: f32,
    /// The lateral slip angle, radians.
    slip_angle: f32,
    /// The suspension's force, which is the tyre's load.
    normal_load: f32,
    /// The traction weight while driven forward (`traction_circle_weight`), 1 otherwise.
    fx_weight: f32,
    /// The surface's and the weather's grip, against asphalt's.
    grip: f32,
}

impl Tyre {
    fn at(wheel: &Wheel, step: &WheelStep, point_vel: Vec3) -> Self {
        let WheelStep { up, forward, right, weather_grip, .. } = *step;
        let steering_rot = Quat::from_axis_angle(up, wheel.steering_angle);
        let wheel_forward = steering_rot.mul_vec3(forward).normalize();
        let wheel_right = steering_rot.mul_vec3(right).normalize();

        let v_long = point_vel.dot(wheel_forward);
        let v_lat = point_vel.dot(wheel_right);

        // Denom: düşük hızda sıfır bölünmeyi önle
        let ref_vel = v_long.abs().max(0.5);

        // Lateral slip angle [rad]
        let slip_angle = -(v_lat / ref_vel).atan();

        // The traction weight counts while the wheel is driven forward (`traction_circle_weight`).
        let fx_weight = if wheel.drive_torque > 1.0 { wheel.traction_circle_weight } else { 1.0 };

        // Yüzey materyali (buz/kum/asfalt) + hava durumu (yağmur/kar) grip çarpanı.
        // Sürtünme çemberi Pacejka içinde uygulandı; tüm vektörü ölçeklemek yönü korur,
        // etkin μ'yü orantılı ölçekler (buzda düşük tutuş → redline patinaj vb.).
        let grip = (wheel.surface_friction / PhysicsMaterial::ASPHALT.dynamic_friction)
            * weather_grip;

        Tyre {
            forward: wheel_forward,
            right: wheel_right,
            v_long,
            v_lat,
            ref_vel,
            slip_angle,
            normal_load: wheel.suspension_force,
            fx_weight,
            grip,
        }
    }

    /// The combined curve's forces at a longitudinal slip ratio of `slip_ratio`, before `grip`.
    fn forces_at(&self, wheel: &Wheel, slip_ratio: f32) -> (f32, f32) {
        pacejka_combined_weighted(
            &wheel.pacejka_long,
            &wheel.pacejka_lat,
            slip_ratio,
            self.slip_angle,
            self.normal_load,
            wheel.sliding_grip_floor,
            self.fx_weight,
        )
    }

    /// The longitudinal slip ratio of a wheel spinning at `wheel`'s current speed.
    fn slip_ratio(&self, wheel: &Wheel) -> f32 {
        (wheel.angular_velocity * wheel.radius - self.v_long) / self.ref_vel
    }
}

/// The wheel's spin for the step (6.3), solved together with the tyre's force on it. Returns that
/// force — what the chassis is pushed with too — and the spin it left.
///
/// **The wheel and the chassis take the same force.** The spin update is implicit
/// in the tyre's stiffness at zero slip, `k = B·C·D·Fz / ref` per m/s of slip
/// velocity — the steepest the curve gets, so it is stable wherever the slip is —
/// which to first order is the wheel balancing against
///   F = (F₀ + k·dt·r·T/I) / (1 + k·dt·r²/I),
/// and that `F` is what the chassis is pushed with too. The chassis used to be pushed
/// with the curve's force at the slip the update left instead: equal to `F` only on the
/// curve's straight part, and at a crawl, where the slip is tiny and the stiffness
/// ~10⁵ N per m/s, the two parted by enough to pump — a parked car fell into a slow
/// spin on the spot, left and right wheels rolling opposite ways for good, and a heavy
/// car could not pull away gently (`a_parked_car_comes_to_rest_and_stays`,
/// `a_heavy_car_driven_by_one_axle_pulls_away`).
fn solve_spin(wheel: &mut Wheel, tyre: &Tyre, effective_inertia: f32, dt: f32) -> (f32, f32) {
    // Longitudinal slip ratio
    let wheel_linear_vel = wheel.angular_velocity * wheel.radius;
    let slip_ratio = (wheel_linear_vel - tyre.v_long) / tyre.ref_vel;

    // Kombine Pacejka MF — sürtünme çemberi dahilinde
    let (final_long, _) = tyre.forces_at(wheel, slip_ratio);
    let final_long = final_long * tyre.grip;

    // Reaksiyon torku lastikten gelen geri tepme
    let effective_brake = brake_against_spin(wheel);

    let external = wheel.drive_torque + effective_brake;

    let k = wheel.pacejka_long.b * wheel.pacejka_long.c * wheel.pacejka_long.d * tyre.normal_load / tyre.ref_vel;
    let radius = wheel.radius;
    let solved_force = (final_long + k * dt * radius * external / effective_inertia)
        / (1.0 + k * dt * radius * radius / effective_inertia);
    wheel.angular_velocity += (external - solved_force * radius) / effective_inertia * dt;
    (solved_force, wheel.angular_velocity)
}

/// The traction control: a driven wheel's spin held within the target slip of the contact's
/// speed.
///
/// A driven wheel's `drive_torque` can exceed the reaction torque the tyre's grip gives back
/// (at most `μ·Fz·r`) many times over, and then ω does not settle — it RUNS AWAY (`ω·r ≫ v_long`,
/// 109 m/s once). A slip ratio like that fills the friction circle LENGTHWAYS, and through the
/// Lorentzian cross-weight (`gy`) takes the tyre's SIDEWAYS grip to almost nothing: the driven
/// axle makes no lateral force, the smallest yaw disturbance grows, and the car spins by itself
/// at full throttle in a straight line (and wheelspins on the spot at a launch). Holding ω at the
/// target slip keeps the longitudinal force near the curve's peak (good acceleration) and keeps
/// some lateral grip — about 45 % of it at the target for the default tyre, not all of it: floored
/// with the wheel turned, a rear-driven car still power-oversteers. Only DRIVEN wheels are held,
/// so the front's steering and the brake/ABS path (`drive_torque ≈ 0`) are untouched; the
/// allowed ω grows with `ref_vel`, so the car accelerates normally.
fn traction_control(wheel: &mut Wheel, vehicle_entity: BodyHandle, tyre: &Tyre) {
    if wheel.drive_torque.abs() > 1.0 && wheel.radius > 1e-4 {
        const TC_TARGET_SLIP: f32 = 0.2; // ~Pacejka tepe slip'i
        let spin_margin = TC_TARGET_SLIP * tyre.ref_vel;
        let hi = (tyre.v_long + spin_margin) / wheel.radius;
        let lo = (tyre.v_long - spin_margin) / wheel.radius;
        let pre_clamp_ω = wheel.angular_velocity;
        wheel.angular_velocity = wheel.angular_velocity.clamp(lo, hi);
        if (wheel.angular_velocity - pre_clamp_ω).abs() > 1e-3 {
            tracing::trace!(
                entity = ?vehicle_entity,
                axle = ?wheel.axle_type,
                is_left = wheel.is_left,
                omega_before = pre_clamp_ω,
                omega_after = wheel.angular_velocity,
                v_long = tyre.v_long,
                "[Vehicle] traction control — clamped driven-wheel spin to target slip"
            );
        }
    }
}

/// The tyre's longitudinal force for the step, and its lateral force before `grip`, at the slip the
/// wheel has now — after its update and the traction control's and the brake's say.
///
/// The lateral force is taken at that slip, and it caps the solved force: a first-order step that
/// ran past the curve's peak (a big torque on a light wheel), or a spin the clamps changed after
/// the solve, leaves the tyre with what the curve gives there and no more.
fn settled_tyre_forces(wheel: &Wheel, tyre: &Tyre, solved_force: f32, solved_spin: f32) -> (f32, f32) {
    let slip_ratio = tyre.slip_ratio(wheel);
    let (at_slip, final_lat) = tyre.forces_at(wheel, slip_ratio);
    let at_slip = at_slip * tyre.grip;
    // A spin the traction control or the brake's lock set after the solve is held there,
    // not free to take the slip up as the solve assumed (`r²/I`): the tyre then gives what
    // the curve gives at it — a locked wheel slides at its full sliding grip.
    let held = wheel.angular_velocity != solved_spin;
    let long_force = if held || solved_force * at_slip < 0.0 || solved_force.abs() > at_slip.abs() {
        at_slip
    } else {
        solved_force
    };
    (long_force, final_lat)
}

/// The force the tyre puts on the chassis at the contact: `long_force` plus the rolling resistance
/// along the wheel, and the lateral force across it.
///
/// **The tyre never pushes the contact past standing still sideways in one step**:
/// the lateral half of the same stiffness, limited to what stops the contact's
/// sideways velocity through its effective mass, shared among the grounded wheels.
/// At speed the limit is far above any tyre force and nothing changes.
fn contact_force(
    wheel: &Wheel,
    tyre: &Tyre,
    step: &WheelStep,
    vehicle_rb: &RigidBody,
    hit_point: Vec3,
    long_force: f32,
    final_lat: f32,
) -> Vec3 {
    let WheelStep { vehicle_transform, com, grounded_share, dt, .. } = *step;
    let r_contact = hit_point - com;
    let inv_inertia = vehicle_rb.inv_world_inertia_tensor(vehicle_transform.rotation);
    let inv_mass_along = |dir: Vec3| {
        let rn = r_contact.cross(dir);
        vehicle_rb.inv_mass() + rn.dot(inv_inertia * rn)
    };
    let at_most = |dir: Vec3, force: f32, v: f32| -> f32 {
        if force * v >= 0.0 {
            return force;
        }
        let inv_mass = inv_mass_along(dir);
        if inv_mass <= 0.0 {
            return force;
        }
        let stop = v.abs() / (inv_mass * dt * grounded_share);
        force.clamp(-stop, stop)
    };
    // Rolling resistance (`Wheel::rolling_resistance`): what a free-rolling tyre's
    // `Crr·Fz·r` comes to at the contact — a force against the contact's motion of
    // at most `Crr·Fz`, and at most what stops it. As a torque on the wheel it was a
    // light wheel crossing zero and being caught there every other step, and the car
    // crept on at a fraction of a millimetre a second.
    let v_long = tyre.v_long;
    let rolling = if v_long == 0.0 {
        0.0
    } else {
        at_most(tyre.forward, -v_long.signum() * wheel.rolling_resistance * tyre.normal_load, v_long)
    };
    tyre.forward * (long_force + rolling) + tyre.right * at_most(tyre.right, final_lat * tyre.grip, tyre.v_lat)
}

/// A wheel in the air, for one step: the engine and the brake turn it, and no spring holds it.
fn step_airborne_wheel(wheel: &mut Wheel, brake_input: f32, effective_inertia: f32, dt: f32) {
    // Havada: sadece motor + fren, yay kuvveti yok
    wheel.suspension_force = 0.0;

    let effective_brake = brake_against_spin(wheel);
    let net_torque = wheel.drive_torque + effective_brake;
    wheel.angular_velocity += (net_torque / effective_inertia) * dt;

    lock_braked_wheel(wheel, brake_input, effective_inertia, dt);
}

/// The brake's torque on `wheel`, against its spin; none on a wheel all but stopped, which a brake
/// does not drive backwards.
fn brake_against_spin(wheel: &Wheel) -> f32 {
    let brake_dir = if wheel.angular_velocity.abs() > 0.01 {
        -wheel.angular_velocity.signum()
    } else {
        0.0
    };
    wheel.brake_torque * brake_dir
}

/// Fren kilitleme: a braked wheel slower than one step of its brake's deceleration stops.
fn lock_braked_wheel(wheel: &mut Wheel, brake_input: f32, effective_inertia: f32, dt: f32) {
    let max_brake_decel = wheel.brake_torque / effective_inertia * dt;
    if brake_input > 0.01 && wheel.angular_velocity.abs() < max_brake_decel {
        wheel.angular_velocity = 0.0;
    }
}

// ============================================================
// YARDIMCI FONKSİYONLAR
// ============================================================

/// Applies a force at a point, which produces a torque.
fn apply_force_at_point(
    rb: &RigidBody,
    vel: &mut Velocity,
    center_of_mass: Vec3,
    rotation: Quat,
    force: Vec3,
    point: Vec3,
    dt: f32,
) {
    if rb.is_static() {
        return;
    }
    vel.linear += (force * rb.inv_mass()) * dt;
    let torque = (point - center_of_mass).cross(force);
    vel.angular += (rb.inv_world_inertia_tensor(rotation) * torque) * dt;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The axle of a wheel at `z` along the body. The vehicle drives along its local −Z (see
    /// `update_vehicle`'s axes), so the front axle is at negative `z`.
    ///
    /// The harnesses here built every car the other way round: front axle at `+z`, behind the
    /// centre of mass, and `is_left` on the right. Every test of a "front-driven" or steered car
    /// was testing a car that steered with its back wheels. The engine's own cars
    /// (`gizmo-app`'s gameplay, `simple`, `systems`) put the front at `z = −1.4`.
    fn axle_at(z: f32) -> Axle {
        if z < 0.0 {
            Axle::Front
        } else {
            Axle::Rear
        }
    }

    /// Is a wheel at `x` across the body on the left? Right is local +X.
    fn is_left_at(x: f32) -> bool {
        x < 0.0
    }

    /// Minimal headless harness: build a 4-wheel VehicleController on flat ground and
    /// drive `update_vehicle` with a hand-rolled semi-implicit integrator (the ECS
    /// rigid integrator is not a dep here). Returns the final state after `steps`.
    fn sim_vehicle(
        throttle: f32,
        steer: f32,
        steps: usize,
        seed: Option<(RigidBody, Transform, Velocity, VehicleController)>,
    ) -> (RigidBody, Transform, Velocity, VehicleController) {
        let veh_id = BodyHandle::from_id(2);
        let ground_id = BodyHandle::from_id(1);
        let ground = Collider::box_collider(Vec3::new(200.0, 1.0, 200.0)); // top at y=0
        let ground_t = Transform::new(Vec3::new(0.0, -1.0, 0.0));

        let (mut rb, mut t, mut vel, mut vc) = seed.unwrap_or_else(|| {
            let mut rb = RigidBody::new(1200.0, true);
            rb.calculate_box_inertia(1.4, 0.7, 2.4);
            rb.center_of_mass = Vec3::new(0.0, 0.2, 0.0);
            let t = Transform::new(Vec3::new(0.0, 1.0, 0.0)); // start above ground
            let mut vc = VehicleController::new();
            for (x, z) in [(0.7_f32, 1.0_f32), (-0.7, 1.0), (0.7, -1.0), (-0.7, -1.0)] {
                vc.add_wheel(Wheel {
                    attachment_local_pos: Vec3::new(x, 0.2, z),
                    radius: 0.3,
                    axle_type: axle_at(z),
                    is_left: is_left_at(x),
                    suspension_rest_length: 0.15,
                    suspension_max_travel: 0.15,
                    suspension_stiffness: 40000.0,
                    suspension_damping: 3000.0,
                    wheel_mass: 25.0,
                    ..Default::default()
                });
            }
            (rb, t, Velocity::default(), vc)
        });

        let dt = 1.0 / 60.0;
        let gravity = Vec3::new(0.0, -9.81, 0.0);
        let colliders = [
            (ground_id, ground_t, ground.clone()),
            (veh_id, t, Collider::box_collider(Vec3::new(0.7, 0.35, 1.4))),
        ];
        for _ in 0..steps {
            vc.throttle_input = throttle.abs();
            vc.set_reverse(throttle < 0.0);
            vc.steering_input = steer;
            vel.linear += gravity * dt; // gravity (rigid integrator would do this)
            update_vehicle(veh_id, &mut vc, &mut rb, &t, &mut vel, &colliders, 1.0, dt);
            // Semi-implicit integrate pose from the mutated velocity.
            t.position += vel.linear * dt;
            if vel.angular.length() > 1e-6 {
                t.rotation = (Quat::from_scaled_axis(vel.angular * dt) * t.rotation).normalize();
            }
            assert!(
                t.position.is_finite() && vel.linear.is_finite() && vel.angular.is_finite(),
                "vehicle sim produced NaN/Inf"
            );
        }
        (rb, t, vel, vc)
    }

    /// **The traction weight takes cornering grip at the limit, and only there** (#24): with the
    /// pair inside the circle nothing changes; at the limit, a weight of 1.5 leaves less lateral
    /// force for the same slips.
    #[test]
    fn traction_weight_costs_lateral_grip_only_at_the_limit() {
        let p = PacejkaParams::default();
        let fz = 4000.0;
        let (gentle_x, gentle_y) = pacejka_combined_weighted(&p, &p, 0.01, 0.01, fz, DEFAULT_SLIDING_GRIP_FLOOR, 1.5);
        assert_eq!((gentle_x, gentle_y), pacejka_combined_with_floor(&p, &p, 0.01, 0.01, fz, DEFAULT_SLIDING_GRIP_FLOOR));
        let (_, plain) = pacejka_combined_with_floor(&p, &p, 0.1, 0.05, fz, DEFAULT_SLIDING_GRIP_FLOOR);
        let (_, weighted) = pacejka_combined_weighted(&p, &p, 0.1, 0.05, fz, DEFAULT_SLIDING_GRIP_FLOOR, 1.5);
        assert!(weighted.abs() < plain.abs() * 0.95, "at the limit: {weighted} against {plain}");
    }

    /// **The powertrain's knobs, each off by default** (`nfsu2-gizmo` MOTOR-NOTLARI #21, #23): a
    /// shift that takes time and shares torque, a fuel cut, torque added at the crank.
    #[test]
    fn shift_time_fuel_cut_and_extra_torque_shape_the_crank_torque() {
        let mut vc = VehicleController::new();
        vc.tuning.torque_curve = vec![(0.0, 300.0), (9000.0, 300.0)];
        vc.throttle_input = 1.0;
        vc.engine_rpm = 4000.0;
        assert_eq!(vc.engine_torque(), 300.0, "defaults: the curve times the throttle");
        vc.extra_crank_torque = 50.0;
        assert_eq!(vc.engine_torque(), 350.0, "extra torque is added, not scaled by the throttle");
        vc.tuning.shift_time = 0.5;
        vc.tuning.shift_torque_share = 0.7;
        vc.shift_to(3);
        assert_eq!((vc.current_gear, vc.shift_remaining), (3, 0.5));
        assert!((vc.engine_torque() - 245.0).abs() < 1e-3, "70 % during the shift: {}", vc.engine_torque());
        vc.shift_remaining = 0.0;
        vc.tuning.fuel_cut_rpm = Some(3975.0);
        assert_eq!(vc.engine_torque(), 0.0, "fuel cut above 3975 rpm");
        vc.engine_rpm = 3900.0;
        assert_eq!(vc.engine_torque(), 350.0);
        let mut auto = VehicleController::new();
        auto.tuning.shift_time = 0.4;
        auto.engine_rpm = auto.tuning.upshift_rpm + 1.0;
        auto.auto_shift_tick(0.016);
        assert_eq!((auto.current_gear, auto.shift_remaining), (3, 0.4), "the automatic box arms the shift too");
    }

    /// **Engine braking slows a coasting car and never drives a stopped one backwards**, and drag
    /// can be higher off the throttle (#21 c, d).
    #[test]
    fn engine_braking_and_off_throttle_drag_slow_a_coasting_car() {
        let settled = sim_vehicle(0.0, 0.0, 120, None);
        let rolling = sim_vehicle(1.0, 0.0, 240, Some(settled.clone()));
        let coast = |tune: &dyn Fn(&mut VehicleController)| {
            let (rb, t, v, mut vc) = rolling.clone();
            tune(&mut vc);
            let (_, _, v2, _) = sim_vehicle(0.0, 0.0, 120, Some((rb, t, v, vc)));
            v.linear.length() - v2.linear.length()
        };
        let plain = coast(&|_| {});
        let braked = coast(&|vc| vc.tuning.engine_brake_curve = vec![(800.0, 20.0), (7000.0, 80.0)]);
        let draggy = coast(&|vc| vc.tuning.aero.drag_coefficient_off_throttle = Some(1.5));
        assert!(braked > plain + 0.5, "engine braking: lost {braked} m/s against {plain}");
        assert!(draggy > plain + 0.05, "off-throttle drag: lost {draggy} m/s against {plain}");

        let (rb, t, v, mut vc) = settled;
        vc.tuning.engine_brake_curve = vec![(800.0, 200.0), (7000.0, 200.0)];
        let (_, parked, still, _) = sim_vehicle(0.0, 0.0, 180, Some((rb, t, v, vc)));
        assert!(still.linear.length() < 1e-3 && (parked.position - t.position).length() < 5e-3, "engine braking moved a stopped car");
    }

    /// **A parked car stays parked.** Dropped onto flat ground with no input, it settles and stops:
    /// no creep (it rolled on at 1.4 cm/s for good with nothing resisting rolling, then at 0.6 mm/s
    /// with the resistance as a wheel torque), no rocking in yaw (±0.2 rad/s, the low-speed tyre
    /// overshooting the contact velocity each step) and no slow spin on the spot (left and right
    /// wheels rolling opposite ways, the longitudinal half of the same overshoot).
    #[test]
    fn a_parked_car_comes_to_rest_and_stays() {
        // With and without downforce: the drop lands a little differently, and the yaw mode came
        // out of one landing and not the other.
        for lift in [0.0, -0.8] {
            let (rb, t, v, mut vc) = sim_vehicle(0.0, 0.0, 1, None);
            vc.tuning.aero.lift_coefficient = lift;
            let settled = sim_vehicle(0.0, 0.0, 120, Some((rb, t, v, vc)));
            let (_, before, _, _) = settled.clone();
            let (_, after, vel, vc) = sim_vehicle(0.0, 0.0, 180, Some(settled));
            let crept = (after.position - before.position).length();
            assert!(crept < 5e-3, "lift {lift}: a parked car crept {crept} m in 3 s");
            assert!(vel.linear.length() < 1e-3 && vel.angular.length() < 1e-3, "lift {lift}: still moving: {vel:?}");
            assert!(vc.wheels.iter().all(|w| w.angular_velocity.abs() < 1e-3), "lift {lift}: wheels turning while parked");
        }
    }

    /// **The harness's car is a car: it steers with its front wheels and turns the way it is
    /// steered.** Steered right at speed, it goes right (+X, a clockwise yaw from above), the
    /// steered wheels are ahead of the centre of mass, and the inner — right — one steers more.
    /// The harnesses built every car facing backwards, so every steering test here was testing a
    /// car that steered with its rear wheels and turned left when steered right.
    #[test]
    fn the_harness_car_steers_with_its_front_wheels_and_turns_the_way_it_is_steered() {
        let settled = sim_vehicle(0.0, 0.0, 120, None);
        let rolling = sim_vehicle(1.0, 0.0, 120, Some(settled));
        let (_, t, v, vc) = sim_vehicle(0.3, 0.5, 60, Some(rolling));
        assert!(t.position.x > 0.5, "steered right, the car went to x {}", t.position.x);
        assert!(v.angular.y < 0.0, "steered right, the yaw rate is {}", v.angular.y);
        let steered: Vec<&Wheel> = vc.wheels.iter().filter(|w| w.steering_angle != 0.0).collect();
        assert_eq!(steered.len(), 2);
        for w in &steered {
            assert!(w.attachment_local_pos.z < 0.0, "a steered wheel at z {} is behind the centre", w.attachment_local_pos.z);
        }
        let angle = |left: bool| steered.iter().find(|w| w.is_left == left).map(|w| w.steering_angle.abs());
        let (inner, outer) = (angle(false).expect("a right front wheel"), angle(true).expect("a left front wheel"));
        assert!(inner > outer, "turning right, the right wheel is the inner one: {inner} against {outer}");
    }

    /// **Left and right turns are mirror images.** A symmetric car at speed, steered half left and
    /// half right for two seconds, must end mirrored: the same lateral offset with the opposite
    /// sign, the same yaw rate negated. With the wheels' forces applied one after another (each
    /// wheel reading the velocity the one before had changed) they were not — 5 % of lateral grip
    /// on a front-driven car, and swapping the wheels' order swapped the two numbers exactly
    /// (`nfsu2-gizmo` MOTOR-NOTLARI #19).
    #[test]
    fn left_and_right_turns_are_mirror_images() {
        let settled = sim_vehicle(0.0, 0.0, 120, None);
        let rolling = sim_vehicle(1.0, 0.0, 120, Some(settled));
        let (_, left_t, left_v, _) = sim_vehicle(0.3, 0.5, 120, Some(rolling.clone()));
        let (_, right_t, right_v, _) = sim_vehicle(0.3, -0.5, 120, Some(rolling));
        let (lx, rx) = (left_t.position.x, right_t.position.x);
        let (ly, ry) = (left_v.angular.y, right_v.angular.y);
        assert!(lx.abs() > 1.0, "the car did not turn: x {lx}");
        assert!((lx + rx).abs() < 1e-3 * lx.abs(), "offsets {lx} and {rx} are not mirrored");
        assert!((ly + ry).abs() < 1e-3 * ly.abs(), "yaw rates {ly} and {ry} are not mirrored");
    }

    /// **A heavy car driven by one axle pulls away, however gently.** The step-end rule that stops a
    /// slow wheel with no input used to stop *grounded* undriven wheels too: rolling at a crawl, their ω (≈ v/r) fell
    /// under 0.05 rad/s every step and was zeroed, and a zero-ω wheel on a moving body brakes with
    /// the tyre's full slip force. A 2.9 t rear-driven car at full throttle sat at 0 km/h for a
    /// minute with its front wheels at exactly ω = 0 (`nfsu2-gizmo` MOTOR-NOTLARI #17); the same
    /// torque on all four wheels moved it.
    #[test]
    fn a_heavy_car_driven_by_one_axle_pulls_away() {
        let mut rb = RigidBody::new(2900.0, true);
        rb.calculate_box_inertia(1.8, 0.9, 4.6);
        rb.center_of_mass = Vec3::new(0.0, 0.2, 0.0);
        let (_, _, _, light) = sim_vehicle(0.0, 0.0, 1, None);
        let mut vc = light.clone();
        for w in &mut vc.wheels {
            w.suspension_stiffness = 90000.0;
            w.suspension_damping = 7000.0;
        }
        // 200 N·m at the driven wheels in first gear, flat over the rev range: ≈ 670 N of push
        // against ≈ 340 N of rolling resistance — a gentle pull-away that spends seconds at a crawl,
        // where the old rule held the undriven wheels at ω = 0 (0.005 m/s after 10 s, against
        // 0.43). The report's car stalled harder (≈ 6 kN against ≈ 14 kN); part of that was the
        // low-speed tyre overshoot `a_parked_car_comes_to_rest_and_stays` pins.
        let t = &mut vc.tuning;
        // `gear_ratios[2]` is first gear (0 is reverse, 1 neutral).
        let at_wheels = 200.0 / (t.gear_ratios[2] * t.final_drive_ratio);
        t.torque_curve = vec![(0.0, at_wheels), (20000.0, at_wheels)];
        let seed = (rb, Transform::new(Vec3::new(0.0, 1.0, 0.0)), Velocity::default(), vc);
        let settled = sim_vehicle(0.0, 0.0, 120, Some(seed));
        let (_, _, vel, vc) = sim_vehicle(1.0, 0.0, 600, Some(settled));
        let front_still = vc.wheels.iter().filter(|w| w.axle_type == Axle::Front && w.angular_velocity == 0.0).count();
        assert!(
            vel.linear.length() > 0.25,
            "10 s at full throttle and the car moves at {:.2} m/s; undriven wheels at ω = 0: {front_still}",
            vel.linear.length()
        );
    }

    /// E2E: the vehicle must SETTLE on the ground (all wheels grounded, sane ride height),
    /// ACCELERATE under throttle, and — the regression that motivated wiring/fixing the
    /// model — NOT lose all its speed coasting through a turn (the explicit-Euler spin
    /// instability + phantom spin-damper drag used to bleed ~70% of speed and oscillate).
    #[test]
    fn e2e_vehicle_settles_accelerates_and_holds_speed_in_turn() {
        // 1. Settle from a drop, no throttle (~2s).
        let (rb, t, vel, vc) = sim_vehicle(0.0, 0.0, 120, None);
        let grounded = vc.wheels.iter().filter(|w| w.is_grounded).count();
        assert_eq!(grounded, 4, "all 4 wheels must ground after settling, got {grounded}");
        assert!(
            t.position.y > 0.0 && t.position.y < 0.7,
            "chassis rests at a sane height, got {}",
            t.position.y
        );
        assert!(vel.linear.length() < 1.0, "settled car should be ~stationary");

        // 2. Full throttle straight (~3s) → must build real speed.
        let (rb, t, vel, vc) = sim_vehicle(1.0, 0.0, 180, Some((rb, t, vel, vc)));
        let cruise = vel.linear.length();
        assert!(cruise > 3.0, "throttle must accelerate the car, got {cruise} m/s");
        assert!(vc.engine_rpm >= vc.tuning.idle_rpm, "engine rpm must be sane");
        assert!(
            vc.wheels.iter().filter(|w| w.is_grounded).count() == 4,
            "must stay grounded while driving"
        );

        // 3. Coast (no throttle) + full steer (~2.5s) → speed must NOT collapse.
        //    Regression guard: before the implicit-spin + damper-gate fixes this lost ~70%.
        let speed_in = vel.linear.length();
        let (_rb, _t, vel2, _vc) = sim_vehicle(0.0, 1.0, 150, Some((rb, t, vel, vc)));
        let speed_out = vel2.linear.length();
        assert!(
            speed_out > speed_in * 0.55,
            "coasting turn must not scrub away most of the speed: {speed_in:.1} -> {speed_out:.1} m/s"
        );
    }

    /// Aerodynamic drag = ½·ρ·Cd·A·v², OPPOSING the velocity. Steps the vehicle once high in
    /// the air (no wheel touches the ground → the only horizontal force is aero), throttle OFF,
    /// at a known speed, and checks that the horizontal velocity lost matches the analytic
    /// ½ρCdAv²/m·dt. That shows drag is both APPLIED and applied with the right
    /// formula/sign/magnitude.
    #[test]
    fn aero_drag_matches_half_rho_cd_a_v_squared() {
        const MASS: f32 = 1200.0;
        const V0: f32 = 50.0; // m/s ileri (-Z)
        const RHO: f32 = 1.225;
        let dt = 1.0 / 60.0;

        let veh_id = BodyHandle::from_id(2);
        let mut rb = RigidBody::new(MASS, true);
        rb.calculate_box_inertia(1.4, 0.7, 2.4);
        rb.center_of_mass = Vec3::new(0.0, 0.2, 0.0);
        let t = Transform::new(Vec3::new(0.0, 100.0, 0.0)); // havada
        let mut vc = VehicleController::new();
        for (x, z) in [(0.7_f32, 1.0_f32), (-0.7, 1.0), (0.7, -1.0), (-0.7, -1.0)] {
            vc.add_wheel(Wheel {
                attachment_local_pos: Vec3::new(x, 0.2, z),
                radius: 0.3,
                axle_type: axle_at(z),
                is_left: is_left_at(x),
                suspension_rest_length: 0.15,
                suspension_max_travel: 0.15,
                ..Default::default()
            });
        }
        // Zemin ÇOK aşağıda → süspansiyon ışını ona ulaşmaz → hiçbir tekerlek grounded olmaz.
        let colliders = [(
            BodyHandle::from_id(1),
            Transform::new(Vec3::new(0.0, -1000.0, 0.0)),
            Collider::box_collider(Vec3::new(200.0, 1.0, 200.0)),
        )];

        let mut vel = Velocity::new(Vec3::new(0.0, 0.0, -V0));
        vc.throttle_input = 0.0;
        vc.brake_input = 0.0;
        vc.steering_input = 0.0;

        update_vehicle(veh_id, &mut vc, &mut rb, &t, &mut vel, &colliders, 1.0, dt);

        // İzolasyon garantisi: airborne → tek yatay kuvvet aero drag.
        assert!(
            vc.wheels.iter().all(|w| !w.is_grounded),
            "araç havada olmalı (hiçbir tekerlek grounded değil)"
        );

        // Beklenen: F = ½ρ·Cd·A·v² (+Z, yani hareketin tersi); Δv = F·dt/m.
        let aero = &vc.tuning.aero;
        let f_drag = 0.5 * RHO * V0 * V0 * aero.drag_coefficient * aero.frontal_area;
        let expected_dv = f_drag / MASS * dt;
        let actual_dv = vel.linear.z - (-V0); // -50'den +Z'ye ne kadar arttı (yavaşlama)

        assert!(
            actual_dv > 0.0,
            "drag hareketin TERSİNE olmalı (+Z), bulundu Δvz={actual_dv}"
        );
        assert!(
            (actual_dv - expected_dv).abs() < expected_dv * 0.02,
            "drag Δv {actual_dv:.6} ≈ ½ρCdAv²/m·dt = {expected_dv:.6} olmalı (±%2)"
        );
        assert!(
            vel.linear.length() < V0,
            "drag toplam hızı düşürmeli: {} < {V0}",
            vel.linear.length()
        );
    }

    /// Ground effect: downforce must only rise once the car is REALLY low; at normal ride
    /// height the effect is 1.0 (behaviour preserved). A smooth, monotonic ramp.
    #[test]
    fn ground_effect_ramps_downforce_only_when_low() {
        let (height, mult) = (0.15_f32, 1.8_f32);
        // Normal ride height (clearance ≥ height) → NO effect (davranış aynen korunur).
        assert_eq!(ground_effect_factor(0.36, height, mult), 1.0);
        assert_eq!(ground_effect_factor(height, height, mult), 1.0);
        // Gövde yere değerken → tam multiplier.
        assert!((ground_effect_factor(0.0, height, mult) - mult).abs() < 1e-5);
        // Yarı yükseklik → yumuşak rampanın tam ortası.
        let mid = ground_effect_factor(height * 0.5, height, mult);
        assert!((mid - (1.0 + (mult - 1.0) * 0.5)).abs() < 1e-5, "mid={mid}");
        // Monoton: alçaldıkça downforce artar.
        assert!(
            ground_effect_factor(0.05, height, mult) > ground_effect_factor(0.10, height, mult)
        );
        // Negatif clearance (gövde yerin içinde) → multiplier'a kırpılır (patlamaz).
        assert!((ground_effect_factor(-0.1, height, mult) - mult).abs() < 1e-5);
    }

    /// Pacejka combined-slip must stay inside the friction circle: the resultant of the
    /// longitudinal + lateral tire force cannot exceed mu_peak * normal_load.
    #[test]
    fn pacejka_combined_respects_friction_circle() {
        let long = PacejkaParams::default();
        let lat = PacejkaParams::default();
        let fz = 3000.0_f32;
        // Large slip in both axes → both axes saturate; resultant must be clamped.
        let (fx, fy) = pacejka_combined(&long, &lat, 0.6, 0.6, fz);
        let mag = (fx * fx + fy * fy).sqrt();
        let limit = long.d.max(lat.d) * 1.2 * fz;
        assert!(
            mag <= limit + 1.0,
            "combined force {mag} must not exceed friction-circle limit {limit}"
        );
        // Zero slip → zero force.
        let (fx0, fy0) = pacejka_combined(&long, &lat, 0.0, 0.0, fz);
        assert!(fx0.abs() < 1e-3 && fy0.abs() < 1e-3, "no slip → no force");
    }

    /// Ackermann geometry: the inner wheel (nearer the turn centre) must steer
    /// more sharply than the outer wheel, for turns in both directions. The old
    /// code had the inner/outer half-track sign flipped (reverse-Ackermann), so
    /// the inner wheel came out *shallower* — this asserts the corrected mapping.
    #[test]
    fn ackermann_inner_wheel_steers_more_than_outer() {
        let wheelbase = 2.8_f32;
        let track = 1.6_f32;

        // Left turn: positive steer → positive turn radius → left wheel is inner.
        let steer = 0.3_f32;
        let turn_radius = wheelbase / steer.tan();
        let left = ackermann_steering_angle(steer, turn_radius, wheelbase, track, true);
        let right = ackermann_steering_angle(steer, turn_radius, wheelbase, track, false);
        assert!(
            left > right,
            "left(inner) {left} must steer more than right(outer) {right} on a left turn"
        );

        // Right turn: mirror — right wheel is inner and steers more (in magnitude).
        let steer = -0.3_f32;
        let turn_radius = wheelbase / steer.tan();
        let left = ackermann_steering_angle(steer, turn_radius, wheelbase, track, true);
        let right = ackermann_steering_angle(steer, turn_radius, wheelbase, track, false);
        assert!(
            right.abs() > left.abs(),
            "right(inner) {right} must steer more than left(outer) {left} on a right turn"
        );
    }

    /// Near-straight guard: when the turn radius is effectively infinite (|r| ≥ 1e4,
    /// i.e. steering ~0), `ackermann_steering_angle` must return the nominal steer
    /// angle UNCHANGED for both wheels, rather than running the geometry formula —
    /// which for a huge radius collapses to ~0 and would erase the commanded angle.
    #[test]
    fn ackermann_returns_nominal_angle_when_near_straight() {
        let wheelbase = 2.8_f32;
        let track = 1.6_f32;
        let steer = 0.2_f32;

        // Infinite radius (dead straight) → passthrough for both wheels.
        let left = ackermann_steering_angle(steer, f32::MAX, wheelbase, track, true);
        let right = ackermann_steering_angle(steer, f32::MAX, wheelbase, track, false);
        assert_eq!(left, steer, "near-straight left wheel keeps the nominal angle");
        assert_eq!(right, steer, "near-straight right wheel keeps the nominal angle");

        // Just over the threshold is still passthrough...
        assert_eq!(ackermann_steering_angle(steer, 2.0e4, wheelbase, track, true), steer);
        // ...whereas an actual (tight) turn engages the geometry and diverges from nominal.
        let tight = ackermann_steering_angle(steer, 6.0, wheelbase, track, true);
        assert!(
            (tight - steer).abs() > 1e-4,
            "a real turn radius must compute a geometry angle ≠ nominal, got {tight}"
        );
    }

    #[test]
    fn test_suspension_spring_and_damper_math() {
        // "Kuvvet Testi: Bir yaya 10cm sıkışma uygulandığında, sönümleme katsayısı X iken..."
        let stiffness = 25000.0; // N/m (Süspansiyon yay sertliği)
        let compression = 0.1; // 0.1 m (10 cm sıkışma)
        let spring_force = stiffness * compression;

        // Yay tam 0.1 metre sıkıştığında, Hooke Kanunu'na göre (F = k*x) 2500N kuvvet üretmeli.
        assert_eq!(spring_force, 2500.0, "Hooke's Law spring force failed");

        // Sönümleme (Damper) Testi
        let damping_compression = 3000.0; // N*s/m (Sönümleme katsayısı)
        let susp_vel_compressing = 1.0; // 1 m/s hızla sıkışıyor (amortisör direnci)

        // Baskı sırasında damper kuvveti hıza zıt (dirençli) ve pozitif olmalı (F = c*v)
        let damper_force = damping_compression * susp_vel_compressing;
        assert_eq!(damper_force, 3000.0, "Damper force calculation failed");

        // Toplam Süspansiyon Kuvveti (Yay + Amortisör)
        let total_suspension_force = spring_force + damper_force;
        assert_eq!(
            total_suspension_force, 5500.0,
            "Total suspension force calculation failed"
        );
    }

    /// Anti-roll bar must RESIST body roll, not amplify it. With the left corner
    /// more compressed than the right (`diff > 0` → body rolling left), the bar
    /// must add up-force to the lower (left) corner and remove it from the higher
    /// (right) corner. The old code flipped both signs (pro-roll), so `left < 0`
    /// and `right > 0` — this locks the corrected convention.
    #[test]
    fn lift_pressure_is_quadratic_by_default_and_follows_the_exponent_otherwise() {
        let v = 30.0_f32;
        let q = 0.5 * 1.225 * v * v;
        assert_eq!(lift_pressure(2.0, v, q).to_bits(), q.to_bits(), "the default must be the old q, bit for bit");
        assert!((lift_pressure(1.0, v, q) - 0.5 * 1.225 * v).abs() < 1e-4, "n = 1 is linear in speed");
        let ratio = lift_pressure(1.0, 2.0 * v, q) / lift_pressure(1.0, v, q);
        assert!((ratio - 2.0).abs() < 1e-5, "doubling speed doubles linear downforce, got ×{ratio}");
    }

    /// The default floor reproduces `pacejka_combined` bit for bit; a higher floor keeps more of
    /// one axis' grip while the other slides hard, and never pushes past the friction circle.
    #[test]
    fn sliding_grip_floor_defaults_to_the_old_value_and_raises_sliding_grip() {
        let p = PacejkaParams::default();
        let fz = 4000.0;
        for (sr, sa) in [(0.0, 0.0), (0.15, 0.1), (5.0, 0.12), (0.3, 0.8)] {
            let a = pacejka_combined(&p, &p, sr, sa, fz);
            let b = pacejka_combined_with_floor(&p, &p, sr, sa, fz, DEFAULT_SLIDING_GRIP_FLOOR);
            assert_eq!((a.0.to_bits(), a.1.to_bits()), (b.0.to_bits(), b.1.to_bits()));
        }
        // Wheel spinning (slip ratio 5) while cornering: lateral grip is what the floor keeps.
        let (_, low) = pacejka_combined_with_floor(&p, &p, 5.0, 0.12, fz, 0.35);
        let (_, high) = pacejka_combined_with_floor(&p, &p, 5.0, 0.12, fz, 0.8);
        assert!(high.abs() > low.abs() * 1.5, "floor 0.8 kept {high:.0} N vs {low:.0} N at 0.35");
        let (fx, fy) = pacejka_combined_with_floor(&p, &p, 5.0, 0.12, fz, 1.0);
        assert!((fx * fx + fy * fy).sqrt() <= fz * p.d * 1.2 + 1e-2, "past the friction circle");
    }

    /// Brake torque per wheel is `brake_input · max_brake_torque · bias` with each axle's own
    /// bias — and the default biases are exactly 0.6 / 0.4.
    #[test]
    fn brake_bias_sets_each_axles_share() {
        let t = VehicleTuning::default();
        assert_eq!((t.brake_bias_front, t.brake_bias_rear), (0.6, 0.4));
        let (mut rb, t0, mut vel, mut vc) = probe_car();
        vc.tuning.brake_bias_front = 0.7;
        vc.tuning.brake_bias_rear = 0.3;
        vc.brake_input = 1.0;
        let cols = probe_ground();
        update_vehicle(BodyHandle::from_id(2), &mut vc, &mut rb, &t0, &mut vel, &cols, 1.0, 1.0 / 120.0);
        for w in &vc.wheels {
            let want = vc.tuning.max_brake_torque * if w.axle_type == Axle::Front { 0.7 } else { 0.3 };
            assert_eq!(w.brake_torque, want, "{:?} wheel", w.axle_type);
        }
    }

    /// `suspension_rebound_damping: None` is the old 2.5× rule bit for bit; a value of its own
    /// changes the result.
    #[test]
    fn rebound_damping_defaults_to_two_and_a_half_times_bump() {
        assert_eq!(Wheel::default().suspension_rebound_damping, None);
        let run = |rebound: Option<f32>| {
            let (mut rb, mut t, mut vel, mut vc) = probe_car();
            for w in &mut vc.wheels {
                w.suspension_rebound_damping = rebound.map(|r| r * w.suspension_damping);
            }
            let cols = probe_ground();
            for _ in 0..240 {
                update_vehicle(BodyHandle::from_id(2), &mut vc, &mut rb, &t, &mut vel, &cols, 1.0, 1.0 / 120.0);
                vel.linear.y -= 9.81 / 120.0;
                t.position += vel.linear / 120.0;
            }
            t.position.y.to_bits()
        };
        assert_eq!(run(None), run(Some(2.5)), "None must be exactly 2.5 × bump");
        assert_ne!(run(None), run(Some(1.0)), "a separate rebound rate changed nothing");
    }

    /// A per-axle anti-roll rate overrides the shared one on that axle only; `None` keeps it.
    #[test]
    fn anti_roll_can_be_set_per_axle() {
        let t = VehicleTuning::default();
        assert_eq!((t.anti_roll_front, t.anti_roll_rear), (None, None));
        let roll_after_turn = |front: Option<f32>, rear: Option<f32>| {
            let (mut rb, mut t, mut vel, mut vc) = probe_car();
            vc.tuning.anti_roll_front = front;
            vc.tuning.anti_roll_rear = rear;
            vc.tuning.anti_roll_stiffness = 20000.0;
            let cols = probe_ground();
            for step in 0..600 {
                vc.throttle_input = 1.0;
                vc.steering_input = if step > 240 { 1.0 } else { 0.0 };
                update_vehicle(BodyHandle::from_id(2), &mut vc, &mut rb, &t, &mut vel, &cols, 1.0, 1.0 / 120.0);
                vel.linear.y -= 9.81 / 120.0;
                t.position += vel.linear / 120.0;
                let w = vel.angular / 120.0;
                if w.length() > 0.0 {
                    t.rotation = (gizmo_math::Quat::from_scaled_axis(w) * t.rotation).normalize();
                }
            }
            t.position.to_array().map(f32::to_bits)
        };
        let shared = roll_after_turn(None, None);
        assert_eq!(shared, roll_after_turn(Some(20000.0), Some(20000.0)), "explicit = shared must be identical");
        assert_ne!(shared, roll_after_turn(Some(0.0), None), "a front-only override changed nothing");
    }

    /// `four_wheeled` builds a car that drives the way the controller's conventions say: the
    /// throttle takes it toward −Z, and a positive steering input turns it right (+X) — with the
    /// tuning's track and wheelbase matching where the wheels are.
    #[test]
    fn a_four_wheeled_car_drives_forward_and_steers_right() {
        let run = |steer: f32| {
            let mut rb = RigidBody::new(1200.0, true);
            rb.calculate_box_inertia(1.6, 0.6, 4.0);
            let mut t = Transform::new(Vec3::new(0.0, 0.9, 0.0));
            let mut vel = Velocity::default();
            let mut vc = VehicleController::four_wheeled(1.6, 2.8, 0.35);
            let cols = probe_ground();
            for step in 0..720 {
                vc.throttle_input = 1.0;
                vc.steering_input = if step > 360 { steer } else { 0.0 };
                update_vehicle(BodyHandle::from_id(2), &mut vc, &mut rb, &t, &mut vel, &cols, 1.0, 1.0 / 120.0);
                vel.linear.y -= 9.81 / 120.0;
                t.position += vel.linear / 120.0;
                let w = vel.angular / 120.0;
                if w.length() > 0.0 {
                    t.rotation = (gizmo_math::Quat::from_scaled_axis(w) * t.rotation).normalize();
                }
            }
            t.position
        };
        let straight = run(0.0);
        let right = run(1.0);
        let left = run(-1.0);
        // Not exactly straight: the corners update the chassis one after another (the wheel
        // vector's documented order dependence) — measured 0.55 m of drift over 51 m.
        assert!(straight.z < -10.0 && straight.x.abs() < 1.5, "straight ahead is −Z: {straight:?}");
        assert!(right.x > straight.x + 2.0, "positive steering turns right (+X): {right:?} vs {straight:?}");
        assert!(left.x < straight.x - 2.0, "negative steering turns left (−X): {left:?} vs {straight:?}");
    }

    /// **A car pulls away at the grip its driven wheels have.** Two things held a 900 kg car back
    /// off the line, both drivetrain bookkeeping and neither the tyre: the force on the chassis was
    /// the tyre's at the slip from *before* the step's spin update, a one-step lag that acted as
    /// ~30 kg·m² of extra inertia on every free-rolling wheel at launch; and each driven wheel
    /// carried the whole engine's reflected inertia instead of its share. Measured on `dunya`'s
    /// car, 3 s of full throttle from rest: 32 km/h before, 40 after (0–100 km/h ~7.5 s → ~6.3 s).
    #[test]
    fn a_car_pulls_away_at_the_grip_it_has() {
        let collider = Collider::box_collider(Vec3::new(0.9, 0.35, 2.0));
        let mut rb = RigidBody::new(900.0, true);
        rb.update_inertia_from_collider(&collider);
        rb.wake_up();
        let mut vc = VehicleController::four_wheeled(1.7, 2.8, 0.38);
        for w in &mut vc.wheels {
            w.suspension_rest_length = 0.45;
            w.suspension_max_travel = 0.35;
        }
        let ground = probe_ground();
        let mut t = Transform::new(Vec3::new(0.0, 0.9, 0.0));
        let mut vel = Velocity::default();
        let dt = 1.0 / 120.0;
        for _ in 0..(120 * 3) {
            vc.throttle_input = 1.0;
            update_vehicle(BodyHandle::from_id(2), &mut vc, &mut rb, &t, &mut vel, &ground, 1.0, dt);
            vel.linear.y -= 9.81 * dt;
            t.position += vel.linear * dt;
            let w = vel.angular * dt;
            if w.length() > 0.0 {
                t.rotation = (gizmo_math::Quat::from_scaled_axis(w) * t.rotation).normalize();
            }
        }
        let kmh = Vec3::new(vel.linear.x, 0.0, vel.linear.z).length() * 3.6;
        assert!(kmh > 37.0, "3 s of full throttle reached only {kmh:.1} km/h");
        assert!(kmh < 60.0, "faster than its tyres can push it: {kmh:.1} km/h");
    }

    /// **One axis drives, brakes and reverses.** Held back from rest, `drive(-1)` backs the car up;
    /// held back while it rolls ahead it brakes first and then backs up; pushed ahead again it
    /// brakes the reverse out and drives off. (A car that only braked on S never went back.)
    #[test]
    fn one_axis_drives_brakes_and_reverses() {
        let collider = Collider::box_collider(Vec3::new(0.9, 0.35, 2.0));
        let mut rb = RigidBody::new(900.0, true);
        rb.update_inertia_from_collider(&collider);
        rb.wake_up();
        let mut vc = VehicleController::four_wheeled(1.7, 2.8, 0.38);
        let ground = probe_ground();
        let mut t = Transform::new(Vec3::new(0.0, 0.9, 0.0));
        let mut vel = Velocity::default();
        let dt = 1.0 / 120.0;
        let mut run = |vc: &mut VehicleController, axis: f32, seconds: f32| {
            for _ in 0..(seconds * 120.0) as usize {
                vc.drive(axis);
                update_vehicle(BodyHandle::from_id(2), vc, &mut rb, &t, &mut vel, &ground, 1.0, dt);
                vel.linear.y -= 9.81 * dt;
                t.position += vel.linear * dt;
                let w = vel.angular * dt;
                if w.length() > 0.0 {
                    t.rotation = (gizmo_math::Quat::from_scaled_axis(w) * t.rotation).normalize();
                }
            }
            (t.position.z, vc.current_speed_kmh)
        };
        let (_, settled) = run(&mut vc, 0.0, 0.5);
        assert!(settled.abs() < 1.0);
        let (z, kmh) = run(&mut vc, -1.0, 3.0);
        assert!(z > 3.0 && kmh < -5.0, "held back from rest it reverses (+Z): z {z}, {kmh} km/h");
        assert!(vc.current_gear == 0);
        // Ahead while rolling back: brakes, then drives off forwards (−Z).
        let (_, kmh) = run(&mut vc, 1.0, 4.0);
        assert!(kmh > 10.0, "pushed ahead it stops the reverse and drives: {kmh} km/h");
        assert!(vc.current_gear >= 2);
        // Back while rolling ahead: the brake, not reverse.
        vc.drive(-1.0);
        assert!(vc.brake_input > 0.9 && vc.throttle_input == 0.0 && vc.current_gear >= 2);
    }

    fn probe_ground() -> Vec<(BodyHandle, Transform, Collider)> {
        vec![(
            BodyHandle::from_id(1),
            Transform::new(Vec3::new(0.0, -1.0, 0.0)),
            Collider::box_collider(Vec3::new(2000.0, 1.0, 2000.0)),
        )]
    }

    /// A 1200 kg four-wheeled car sitting on `probe_ground`, awake.
    fn probe_car() -> (RigidBody, Transform, Velocity, VehicleController) {
        let mut rb = RigidBody::new(1200.0, true);
        rb.calculate_box_inertia(1.4, 0.7, 2.4);
        rb.wake_up();
        let mut vc = VehicleController::new();
        for (x, z) in [(0.7_f32, 1.2_f32), (-0.7, 1.2), (0.7, -1.2), (-0.7, -1.2)] {
            vc.add_wheel(Wheel {
                attachment_local_pos: Vec3::new(x, 0.2, z),
                radius: 0.3,
                axle_type: axle_at(z),
                is_left: is_left_at(x),
                suspension_rest_length: 0.2,
                suspension_max_travel: 0.15,
                suspension_stiffness: 40000.0,
                suspension_damping: 3000.0,
                wheel_mass: 25.0,
                ..Default::default()
            });
        }
        (rb, Transform::new(Vec3::new(0.0, 0.8, 0.0)), Velocity::default(), vc)
    }

    #[test]
    fn anti_roll_bar_resists_roll_not_amplifies() {
        let k = 3000.0;
        let diff = 0.05; // left suspension 5 cm more compressed than right

        let left = anti_roll_force(true, diff, k);
        let right = anti_roll_force(false, diff, k);

        assert!(
            left > 0.0,
            "ARB must add up-force to the more-compressed (lower) left corner, got {left}"
        );
        assert!(
            right < 0.0,
            "ARB must remove up-force from the less-compressed (higher) right corner, got {right}"
        );
        // A torsion bar transfers load — equal and opposite, zero net vertical force.
        assert!(
            (left + right).abs() < 1e-6,
            "ARB forces must be equal and opposite (load transfer), got {left} and {right}"
        );
        // A level car (no compression difference) produces no ARB force.
        assert_eq!(anti_roll_force(true, 0.0, k), 0.0);
        assert_eq!(anti_roll_force(false, 0.0, k), 0.0);
    }

    #[test]
    fn test_pacejka_combined_slip() {
        let long = PacejkaParams::default();
        let lat = PacejkaLat::default();
        let normal_load = 5000.0; // 500 kg tekerlek yükü (Fz)

        // 1. Durum: Sıfır Slip (Kayma Yok)
        let (fx1, fy1) = pacejka_combined(&long, &lat, 0.0, 0.0, normal_load);
        assert!(
            fx1.abs() < 1e-4,
            "Expected zero longitudinal force at zero slip"
        );
        assert!(fy1.abs() < 1e-4, "Expected zero lateral force at zero slip");

        // 2. Durum: Sadece İleri Kayma (Burnout/Frenleme)
        let (fx2, fy2) = pacejka_combined(&long, &lat, 0.15, 0.0, normal_load);
        let expected_fx2 = long.calculate_force(0.15, normal_load);
        assert!(
            (fx2 - expected_fx2).abs() < 1e-4,
            "Expected combined force to match pure force when no lateral slip is present"
        );
        assert!(
            fy2.abs() < 1e-4,
            "Expected zero lateral force when purely accelerating straight"
        );

        // 3. Durum: Kombine Kayma (Virajda Gazlama - Friction Circle Test)
        // Her iki yönde kayma olduğunda (Drift durumu), eksenler birbirinin tutuşunu düşürmeli (Weighting)
        let (fx3, fy3) = pacejka_combined(&long, &lat, 0.15, 0.15, normal_load);

        // fx3, fx2'den (sadece düz gitmekten) çok daha düşük olmalıdır çünkü yanal kuvvet (fy3) de yol tutuşundan pay alıyor
        assert!(
            fx3 < fx2,
            "Combined slip should reduce longitudinal grip (Friction Circle violated)"
        );
        assert!(
            fy3 > 1000.0,
            "Expected significant lateral force during cornering"
        );
    }

    /// A flexible harness for the surface-material and weather grip wiring (Track C) and for
    /// FWD/reverse (Track E): it drops and drives the vehicle with the given ground material,
    /// weather_grip, drivetrain and axle layout, and returns the final forward (−Z) speed
    /// together with the final VehicleController.
    /// A parked car that has fallen asleep must **move when the driver presses the accelerator**.
    ///
    /// The integrator skips a sleeping body entirely, but this system runs regardless: it makes
    /// torque, writes `current_speed_kmh` and turns the wheels. Nothing woke the chassis, so the
    /// visible result was a car whose rev counter climbed, whose speedometer read 54 km/h, whose
    /// wheels spun — and which did not move a millimetre. It reached a bug report as "the wheels
    /// turn but it doesn't go", and it only ever happened to a car that had been left standing:
    /// anything driven from the first frame never slept and so never showed it.
    ///
    /// The test asserts the wake, and then that the wake is **earned** — a sleeping car with no
    /// input stays asleep, because unconditional waking would mean the sleep system does nothing
    /// for parked cars, which is the whole reason it exists.
    #[test]
    fn driver_input_wakes_a_sleeping_chassis() {
        let veh_id = BodyHandle::from_id(2);
        let ground_id = BodyHandle::from_id(1);
        let ground = Collider::box_collider(Vec3::new(200.0, 1.0, 200.0));
        let ground_t = Transform::new(Vec3::new(0.0, -1.0, 0.0));
        let mut rb = RigidBody::new(1200.0, true);
        rb.calculate_box_inertia(1.4, 0.7, 2.4);
        let t = Transform::new(Vec3::new(0.0, 1.0, 0.0));
        let mut vel = Velocity::default();
        let mut vc = VehicleController::new();
        vc.add_wheel(Wheel { attachment_local_pos: Vec3::new(0.0, 0.2, 0.0), ..Default::default() });
        let cols = [(ground_id, ground_t, ground), (veh_id, t, Collider::box_collider(Vec3::new(0.7, 0.35, 1.4)))];

        // Parked and asleep, exactly as `update_sleep_state` leaves a car that has stood still.
        rb.is_sleeping = true;
        rb.sleep_counter = 60;

        // No input: it stays asleep.
        vc.throttle_input = 0.0;
        vc.brake_input = 0.0;
        vc.steering_input = 0.0;
        update_vehicle(veh_id, &mut vc, &mut rb, &t, &mut vel, &cols, 1.0, 1.0 / 60.0);
        assert!(rb.is_sleeping, "a parked car with no input must not be woken by its own controller");

        // Throttle wakes it, and clears the counter so it does not doze straight off again.
        vc.throttle_input = 1.0;
        update_vehicle(veh_id, &mut vc, &mut rb, &t, &mut vel, &cols, 1.0, 1.0 / 60.0);
        assert!(!rb.is_sleeping, "throttle must wake the chassis");
        assert_eq!(rb.sleep_counter, 0);

        // Brake and steering count too — a driver touching either is a driver.
        for (throttle, brake, steer) in [(0.0, 1.0, 0.0), (0.0, 0.0, 0.6)] {
            rb.is_sleeping = true;
            vc.throttle_input = throttle;
            vc.brake_input = brake;
            vc.steering_input = steer;
            update_vehicle(veh_id, &mut vc, &mut rb, &t, &mut vel, &cols, 1.0, 1.0 / 60.0);
            assert!(!rb.is_sleeping, "brake {brake} / steering {steer} must wake the chassis");
        }
    }

    /// A sleeping chassis accumulates **no velocity**, and that is the other half of the same bug.
    ///
    /// Waking on input, on its own, made things worse rather than better. This function used to run
    /// to completion for a sleeping body — suspension, gravity and drive torque all summed into
    /// `Velocity` — while the integrator, which skips sleeping bodies, never spent any of it. The
    /// velocity only grew: measured at 15 m/s on a car that had been standing. Adding the wake
    /// released all of it in one frame and the car took off and flipped. First report: "I press the
    /// accelerator and it doesn't move." Second: "the car flew and did a backflip." One bug, two
    /// halves, and the early return closes both.
    #[test]
    fn a_sleeping_chassis_accumulates_nothing() {
        let veh_id = BodyHandle::from_id(2);
        let ground_id = BodyHandle::from_id(1);
        let ground = Collider::box_collider(Vec3::new(200.0, 1.0, 200.0));
        let ground_t = Transform::new(Vec3::new(0.0, -1.0, 0.0));
        let mut rb = RigidBody::new(1200.0, true);
        rb.calculate_box_inertia(1.4, 0.7, 2.4);
        // **Resting on the ground**, which the earlier draft of this test got wrong: it put the
        // car at y = 1.0 over a ground whose top is y = 0, so the wheels hung in the air, no
        // suspension force was ever produced and the test passed with the fix removed. A test for
        // "a parked car accumulates nothing" has to park the car. Ground top is 0; attachment sits
        // 0.2 above the chassis origin, rest 0.15, radius 0.3 — so y = 0.25 puts the tyres on it
        // and anything lower **compresses the springs**, which is the state a parked car is
        // actually in: they are holding its weight. That compression is the whole mechanism — the
        // springs push up every frame, and for a sleeping body the integrator skips gravity too,
        // so there is nothing pushing back.
        let t = Transform::new(Vec3::new(0.0, 0.17, 0.0));
        let mut vel = Velocity::default();
        let mut vc = VehicleController::new();
        for (x, z) in [(0.7_f32, 1.0_f32), (-0.7, 1.0), (0.7, -1.0), (-0.7, -1.0)] {
            vc.add_wheel(Wheel {
                attachment_local_pos: Vec3::new(x, 0.2, z),
                radius: 0.3,
                suspension_rest_length: 0.15,
                suspension_max_travel: 0.15,
                suspension_stiffness: 40000.0,
                suspension_damping: 3000.0,
                ..Default::default()
            });
        }
        let cols = [
            (ground_id, ground_t, ground),
            (veh_id, t, Collider::box_collider(Vec3::new(0.7, 0.35, 1.4))),
        ];

        rb.is_sleeping = true;
        rb.sleep_counter = 60;
        // Ten seconds of standing still, with no driver anywhere near it.
        for _ in 0..600 {
            update_vehicle(veh_id, &mut vc, &mut rb, &t, &mut vel, &cols, 1.0, 1.0 / 60.0);
        }
        assert!(rb.is_sleeping, "nothing woke it");
        assert!(
            vel.linear.length() < 1e-6 && vel.angular.length() < 1e-6,
            "a parked car built up {:?} / {:?} in ten seconds; on waking that is a launch",
            vel.linear,
            vel.angular
        );

        // And the first frame after waking starts from rest rather than from a stored-up shove.
        vc.throttle_input = 1.0;
        update_vehicle(veh_id, &mut vc, &mut rb, &t, &mut vel, &cols, 1.0, 1.0 / 60.0);
        assert!(!rb.is_sleeping);
        assert!(vel.linear.y.abs() < 1.0, "woke with {:.2} m/s upward — that is the flip", vel.linear.y);
    }

    fn sim_forward_speed(
        throttle: f32,
        reverse: bool,
        ground_material: PhysicsMaterial,
        weather_grip: f32,
        drivetrain: Drivetrain,
        all_front: bool,
        steps: usize,
    ) -> (f32, VehicleController) {
        let veh_id = BodyHandle::from_id(2);
        let ground_id = BodyHandle::from_id(1);
        let ground =
            Collider::box_collider(Vec3::new(200.0, 1.0, 200.0)).with_material(ground_material);
        let ground_t = Transform::new(Vec3::new(0.0, -1.0, 0.0));

        let mut rb = RigidBody::new(1200.0, true);
        rb.calculate_box_inertia(1.4, 0.7, 2.4);
        rb.center_of_mass = Vec3::new(0.0, 0.2, 0.0);
        let mut t = Transform::new(Vec3::new(0.0, 1.0, 0.0));
        let mut vc = VehicleController::new();
        vc.drivetrain = drivetrain;
        for (x, z) in [(0.7_f32, 1.0_f32), (-0.7, 1.0), (0.7, -1.0), (-0.7, -1.0)] {
            vc.add_wheel(Wheel {
                attachment_local_pos: Vec3::new(x, 0.2, z),
                radius: 0.3,
                axle_type: if all_front { Axle::Front } else { axle_at(z) },
                is_left: is_left_at(x),
                suspension_rest_length: 0.15,
                suspension_max_travel: 0.15,
                suspension_stiffness: 40000.0,
                suspension_damping: 3000.0,
                wheel_mass: 25.0,
                ..Default::default()
            });
        }
        let mut vel = Velocity::default();
        let dt = 1.0 / 60.0;
        let gravity = Vec3::new(0.0, -9.81, 0.0);
        let veh_col = Collider::box_collider(Vec3::new(0.7, 0.35, 1.4));

        for _ in 0..steps {
            vc.throttle_input = throttle.abs();
            vc.set_reverse(reverse);
            vc.steering_input = 0.0;
            vel.linear += gravity * dt;
            // Aracın collider'ı GÜNCEL transform ile yeniden kurulur (E2: eskiden harness
            // bayat spawn transformunu geçiriyordu).
            let colliders = [
                (ground_id, ground_t, ground.clone()),
                (veh_id, t, veh_col.clone()),
            ];
            update_vehicle(veh_id, &mut vc, &mut rb, &t, &mut vel, &colliders, weather_grip, dt);
            t.position += vel.linear * dt;
            if vel.angular.length() > 1e-6 {
                t.rotation = (Quat::from_scaled_axis(vel.angular * dt) * t.rotation).normalize();
            }
            assert!(
                t.position.is_finite() && vel.linear.is_finite() && vel.angular.is_finite(),
                "vehicle sim produced NaN/Inf (material {:?}, wg {weather_grip})",
                ground_material.dynamic_friction
            );
        }
        let forward = t.rotation.mul_vec3(Vec3::new(0.0, 0.0, -1.0));
        (vel.linear.dot(forward), vc)
    }

    /// Track C: the surface material decides the real grip. Ice (μ=0.03) must accelerate far
    /// less than asphalt (μ=0.65). The dynamics tyre model used to IGNORE PhysicsMaterial
    /// entirely.
    #[test]
    fn surface_material_scales_tire_grip() {
        let (asphalt, _) =
            sim_forward_speed(1.0, false, PhysicsMaterial::ASPHALT, 1.0, Drivetrain::Rwd, false, 300);
        let (ice, _) =
            sim_forward_speed(1.0, false, PhysicsMaterial::ICE, 1.0, Drivetrain::Rwd, false, 300);
        assert!(asphalt > 3.0, "asfaltta hızlanmalı, bulundu {asphalt}");
        // grip_mult = 0.03/0.65 ≈ 0.046 → fiziksel oran ~20×; eşiği gerçek orana yaklaştır.
        assert!(
            ice < asphalt * 0.2,
            "buz (μ=0.03) asfalttan çok daha az tutmalı: buz {ice} vs asfalt {asphalt}"
        );
    }

    /// Track C: the weather grip multiplier really is applied. Snow (weather_grip 0.3) must
    /// produce less speed than sunny (1.0).
    #[test]
    fn weather_grip_reduces_traction() {
        let (sunny, _) =
            sim_forward_speed(1.0, false, PhysicsMaterial::ASPHALT, 1.0, Drivetrain::Rwd, false, 300);
        let (snow, _) =
            sim_forward_speed(1.0, false, PhysicsMaterial::ASPHALT, 0.3, Drivetrain::Rwd, false, 300);
        assert!(sunny > 3.0, "sunny'de gerçekten hızlanmalı, bulundu {sunny}");
        // weather_grip 0.3 → kuvvet 0.3× ölçekleniyor; büyüklüğü de pinle (yalnız < değil).
        assert!(
            snow < sunny * 0.7,
            "kar (weather_grip 0.3) sunny'den belirgin az hız vermeli: kar {snow} vs sunny {sunny}"
        );
    }

    /// Track E: FWD (every wheel on the front axle + Drivetrain::Fwd) must accelerate. With the
    /// old hardcoded RWD an all-front layout never moved at all and the RPM stuck at idle.
    #[test]
    fn fwd_layout_accelerates() {
        let (fwd, vc) =
            sim_forward_speed(1.0, false, PhysicsMaterial::ASPHALT, 1.0, Drivetrain::Fwd, true, 300);
        assert!(fwd > 3.0, "FWD araç hızlanmalı, bulundu {fwd}");
        assert!(
            vc.engine_rpm > vc.tuning.idle_rpm,
            "FWD motor RPM idle üstüne çıkmalı, bulundu {}",
            vc.engine_rpm
        );
    }

    /// Track E: reverse gear must really produce backwards (+Z) motion (negative forward
    /// speed). The reverse path was previously untested.
    #[test]
    fn reverse_produces_backward_motion() {
        let (fwd, vc) =
            sim_forward_speed(1.0, true, PhysicsMaterial::ASPHALT, 1.0, Drivetrain::Rwd, false, 300);
        assert!(vc.reverse_input, "reverse bayrağı set olmalı");
        assert!(
            fwd < -0.5,
            "geri vites aracı geri (−ileri) götürmeli, ileri hız {fwd} olmalı < -0.5"
        );
    }

    /// Track C: pin weather_grip_factor's Weather→grip mapping and its aquaplaning ramp
    /// DIRECTLY (the simulation tests pass a raw scalar and so skipped this function).
    #[test]
    fn weather_grip_factor_maps_weather_and_aquaplanes() {
        assert_eq!(weather_grip_factor(Weather::Sunny, 0.0), 1.0);
        assert_eq!(weather_grip_factor(Weather::Sunny, 100.0), 1.0);
        assert_eq!(weather_grip_factor(Weather::Snow, 0.0), 0.3);
        assert_eq!(weather_grip_factor(Weather::Snow, 100.0), 0.3);
        // Rain: 20 m/s'ye kadar 0.5 taban; üstünde aquaplaning ile kademeli düşüş, 0.1'e kırpılı.
        assert_eq!(weather_grip_factor(Weather::Rain, 0.0), 0.5);
        assert_eq!(weather_grip_factor(Weather::Rain, 20.0), 0.5);
        assert!((weather_grip_factor(Weather::Rain, 30.0) - 0.4).abs() < 1e-4);
        // 0.5-0.4≈0.1 tabana kırpılır (float: 0.5-40*0.01 tam 0.1 değil → yaklaşık karşılaştır).
        assert!((weather_grip_factor(Weather::Rain, 60.0) - 0.1).abs() < 1e-4);
        assert_eq!(weather_grip_factor(Weather::Rain, 200.0), 0.1); // sert clamp → tam taban
        // Yağmurda hızla monoton azalmalı.
        assert!(weather_grip_factor(Weather::Rain, 40.0) < weather_grip_factor(Weather::Rain, 25.0));
    }

    /// Track E: the Drivetrain::drives mapping table (torque distribution and RPM derivation
    /// both rest on it).
    #[test]
    fn drivetrain_drives_table() {
        assert!(Drivetrain::Fwd.drives(&Axle::Front) && !Drivetrain::Fwd.drives(&Axle::Rear));
        assert!(!Drivetrain::Rwd.drives(&Axle::Front) && Drivetrain::Rwd.drives(&Axle::Rear));
        assert!(Drivetrain::Awd.drives(&Axle::Front) && Drivetrain::Awd.drives(&Axle::Rear));
    }

    /// Track C: when a wheel leaves the ground, surface_friction must reset to ASPHALT (which
    /// stops a stale ice value from carrying for a frame on landing). While grounded it is
    /// captured from the material that was hit.
    #[test]
    fn surface_friction_captures_material_and_resets_when_airborne() {
        let veh_id = BodyHandle::from_id(2);
        let ground_id = BodyHandle::from_id(1);
        let ice =
            Collider::box_collider(Vec3::new(50.0, 1.0, 50.0)).with_material(PhysicsMaterial::ICE);
        let ground_t = Transform::new(Vec3::new(0.0, -1.0, 0.0));

        let mut rb = RigidBody::new(1000.0, true);
        rb.calculate_box_inertia(1.4, 0.7, 2.4);
        let mut t = Transform::new(Vec3::new(0.0, 0.4, 0.0));
        let mut vc = VehicleController::new();
        for (x, z) in [(0.6_f32, 1.0_f32), (-0.6, 1.0), (0.6, -1.0), (-0.6, -1.0)] {
            vc.add_wheel(Wheel {
                attachment_local_pos: Vec3::new(x, 0.1, z),
                radius: 0.3,
                suspension_rest_length: 0.2,
                suspension_max_travel: 0.2,
                ..Default::default()
            });
        }
        let mut vel = Velocity::default();
        let dt = 1.0 / 60.0;
        let veh_col = Collider::box_collider(Vec3::new(0.6, 0.35, 1.4));

        // Buz üzerinde grounded → surface_friction = ICE.
        let colliders = [(ground_id, ground_t, ice.clone()), (veh_id, t, veh_col.clone())];
        update_vehicle(veh_id, &mut vc, &mut rb, &t, &mut vel, &colliders, 1.0, dt);
        assert!(vc.wheels.iter().any(|w| w.is_grounded), "araç buzda grounded olmalı");
        for w in vc.wheels.iter().filter(|w| w.is_grounded) {
            assert!(
                (w.surface_friction - PhysicsMaterial::ICE.dynamic_friction).abs() < 1e-4,
                "buzda grounded → surface_friction=ICE(0.03), bulundu {}",
                w.surface_friction
            );
        }

        // Aracı yukarı taşı → havada; surface_friction ASPHALT'a resetlenmeli.
        t.position.y = 100.0;
        let colliders2 = [(ground_id, ground_t, ice.clone()), (veh_id, t, veh_col.clone())];
        update_vehicle(veh_id, &mut vc, &mut rb, &t, &mut vel, &colliders2, 1.0, dt);
        for w in &vc.wheels {
            assert!(!w.is_grounded, "araç havada olmalı");
            assert!(
                (w.surface_friction - PhysicsMaterial::ASPHALT.dynamic_friction).abs() < 1e-4,
                "havada → surface_friction ASPHALT'a resetlenmeli, bulundu {}",
                w.surface_friction
            );
        }
    }

    /// Track E1: ground-effect artik SUSPANSIYON sikismasindan turetiliyor → downforce YUKE
    /// DUYARLI. Yuksek hizda aero downforce (lift_coefficient<0) normal yuku ARTIRMALI (dinlenen
    /// araca gore). Isaret ters cevrilse (downforce yuku azaltsa) bu test duser.
    #[test]
    fn ground_effect_downforce_adds_normal_load_at_speed() {
        let sum_load = |vc: &VehicleController| -> f32 {
            vc.wheels.iter().map(|w| w.suspension_force).sum()
        };
        // 1) Dinlen (3s), grounded taban yuk = agirlik.
        let (rb, t, vel, vc) = sim_vehicle(0.0, 0.0, 180, None);
        assert_eq!(vc.wheels.iter().filter(|w| w.is_grounded).count(), 4);
        let rest_load = sum_load(&vc);
        assert!(rest_load > 5000.0, "dinlenen araç ağırlığını taşımalı, bulundu {rest_load}");

        // 2) Yuksek ileri hiz (-Z) enjekte et, birkac adim kos → aero downforce suspansiyonu
        //    daha cok sikistirir, ge_factor rampalanir, normal yuk artar.
        let mut fast_vel = vel;
        fast_vel.linear.z = -40.0;
        // A car with downforce: the default has none.
        let mut vc = vc;
        vc.tuning.aero.lift_coefficient = -0.8;
        let (_rb2, _t2, _vel2, vc_fast) = sim_vehicle(0.0, 0.0, 20, Some((rb, t, fast_vel, vc)));
        let fast_load = sum_load(&vc_fast);
        assert!(
            fast_load > rest_load * 1.05,
            "yüksek hızda downforce normal yükü ARTIRMALI (ge işaret/yük-duyarlılık): dinlenme {rest_load:.0} → hız {fast_load:.0}"
        );
    }
}
