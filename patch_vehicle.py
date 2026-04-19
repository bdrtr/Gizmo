import re

with open("crates/gizmo-physics/src/vehicle.rs", "r") as f:
    content = f.read()

# Replace Wheel and VehicleController structs
structs_pattern = re.compile(
    r"pub struct Wheel \{.*?\n\}\n\nimpl Wheel \{.*?\n\}\n\n/// Raycast Vehicle Controller.*?pub struct VehicleController \{.*?pub wheels: Vec<Wheel>,.*?\}\n\nimpl Default for VehicleController \{.*?\n\}\n\nimpl VehicleController \{.*?pub fn new\(\) -> Self \{.*?\}\n\n    pub fn add_wheel.*?\n    \}\n\}",
    re.DOTALL
)

new_structs = """pub struct WheelComponent {
    pub direction: Vec3,        // Süspansiyonun yere uzanma yönü (genelde 0, -1, 0)
    pub axle: Vec3,             // Tekerleğin dönme ekseni (genelde -1, 0, 0 veya 1, 0, 0)

    pub suspension_rest_length: f32, // Normal şartlardaki boşluk mesafesi
    pub suspension_stiffness: f32,   // Yay sertliği
    pub suspension_damping: f32,     // Sönümleme (Geri fırlamayı önler)
    pub wheel_radius: f32,           // Tekerleğin yarıçapı

    pub is_drive_wheel: bool, // Bu tekerlek motor gücü alıyor mu (FWD/RWD/4WD)

    #[serde(skip)] pub is_grounded: bool,
    #[serde(skip)] pub compression: f32,    // Yerdeyse yayın ne kadar sıkıştığı
    #[serde(skip)] pub contact_point: Vec3, // Çarpışma noktası
    #[serde(skip)] pub rotation_angle: f32, // Animasyon için görsel dönüş açısı
}

impl Default for WheelComponent {
    fn default() -> Self {
        Self {
            direction: Vec3::new(0.0, -1.0, 0.0),
            axle: Vec3::new(-1.0, 0.0, 0.0),
            suspension_rest_length: 1.0,
            suspension_stiffness: 20000.0,
            suspension_damping: 2000.0,
            wheel_radius: 0.5,
            is_drive_wheel: false,
            is_grounded: false,
            compression: 0.0,
            contact_point: Vec3::ZERO,
            rotation_angle: 0.0,
        }
    }
}

impl WheelComponent {
    pub fn new(
        rest_length: f32,
        stiffness: f32,
        damping: f32,
        radius: f32,
    ) -> Self {
        Self {
            suspension_rest_length: rest_length,
            suspension_stiffness: stiffness,
            suspension_damping: damping,
            wheel_radius: radius,
            ..Default::default()
        }
    }

    /// Motorlu tekerlek olarak ayarla
    pub fn with_drive(mut self) -> Self {
        self.is_drive_wheel = true;
        self
    }
}

/// Raycast Vehicle Controller. Araç gövdesine (Chassis) RigidBody ile birlikte eklenmelidir.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VehicleController {
    pub engine_force: f32,   // Motor gücü (Newton). Pozitif = ileri, Negatif = geri
    pub steering_angle: f32, // Direksiyon açısı (Radyan). Pozitif = sola, Negatif = sağa
    pub brake_force: f32,    // Fren kuvveti (Newton)
    // Konfigüre edilebilir fizik sabitleri
    pub steering_force_mult: f32, // Direksiyon kuvvet çarpanı
    pub lateral_grip: f32,        // Yanal tutuş kuvveti
    pub anti_slide_force: f32,    // Kayma önleme kuvveti
    /// Aerodinamik sürükleme katsayısı — F = ½ · Cd · ρA · v²
    pub drag_coefficient: f32,
}

impl Default for VehicleController {
    fn default() -> Self {
        Self::new()
    }
}

impl VehicleController {
    pub fn new() -> Self {
        Self {
            engine_force: 0.0,
            steering_angle: 0.0,
            brake_force: 0.0,
            steering_force_mult: 8000.0,
            lateral_grip: 5000.0,
            anti_slide_force: 3000.0,
            drag_coefficient: 0.3,
        }
    }
}"""

content = structs_pattern.sub(new_structs, content)

# Second half logic replacement
content = content.replace("    if let (Some(trans_storage), Some(mut vel_storage), Some(mut rbs), Some(mut vehicles)) = (\n        world.borrow::<Transform>(),     // Sadece okuma — borrow_mut gerekmiyor (runtime panic önlenir)\n        world.borrow_mut::<Velocity>(),\n        world.borrow_mut::<RigidBody>(),\n        world.borrow_mut::<VehicleController>(),\n    ) {",
"""    if let (Some(mut trans_storage), Some(mut vel_storage), Some(mut rbs), Some(vehicles), Some(children_storage), Some(mut wheel_storage)) = (
        world.borrow_mut::<Transform>(),
        world.borrow_mut::<Velocity>(),
        world.borrow_mut::<RigidBody>(),
        world.borrow::<VehicleController>(),
        world.borrow::<gizmo_core::component::Children>(),
        world.borrow_mut::<WheelComponent>(),
    ) {""")

middle_target = """            let vehicle = vehicles.get_mut(entity).unwrap();

            rb.wake_up();

            let inv_mass = if rb.mass > 0.0 { 1.0 / rb.mass } else { 0.0 };
            let inv_inertia = rb.inverse_inertia_local;

            let mut total_linear_impulse = Vec3::ZERO;
            let mut total_angular_impulse = Vec3::ZERO;

            let forward = t.rotation.mul_vec3(Vec3::new(0.0, 0.0, 1.0)).normalize();
            let right = t.rotation.mul_vec3(Vec3::new(1.0, 0.0, 0.0)).normalize();

            let num_wheels = vehicle.wheels.len() as f32;
            let engine_force = vehicle.engine_force;
            let steering_angle = vehicle.steering_angle;
            let brake_force = vehicle.brake_force;
            let steer_mult = vehicle.steering_force_mult;
            let lat_grip = vehicle.lateral_grip;
            let anti_slide_k = vehicle.anti_slide_force;
            let drive_wheel_count = vehicle
                .wheels
                .iter()
                .filter(|w| w.is_drive_wheel)
                .count()
                .max(1) as f32;

            for wheel in vehicle.wheels.iter_mut() {
                // Lokal bağlantı noktasını dünya haritasına çevir (Scale dahil)
                let scaled_conn = wheel.connection_point * t.scale;
                let r_ws = t.rotation.mul_vec3(scaled_conn);
                let origin = t.position + r_ws;
                let dir = t.rotation.mul_vec3(wheel.direction).normalize();"""

middle_replacement = """            let vehicle = vehicles.get(entity).unwrap();

            rb.wake_up();

            let inv_mass = if rb.mass > 0.0 { 1.0 / rb.mass } else { 0.0 };
            let inv_inertia = rb.inverse_inertia_local;

            let mut total_linear_impulse = Vec3::ZERO;
            let mut total_angular_impulse = Vec3::ZERO;

            let forward = t.rotation.mul_vec3(Vec3::new(0.0, 0.0, 1.0)).normalize();
            let right = t.rotation.mul_vec3(Vec3::new(1.0, 0.0, 0.0)).normalize();

            let mut wheel_entities = Vec::new();
            if let Some(children) = children_storage.get(entity) {
                for &child_id in &children.0 {
                    if wheel_storage.contains(child_id) {
                        wheel_entities.push(child_id);
                    }
                }
            }

            let num_wheels = wheel_entities.len() as f32;
            if num_wheels < 1.0 { continue; }

            let engine_force = vehicle.engine_force;
            let steering_angle = vehicle.steering_angle;
            let brake_force = vehicle.brake_force;
            let steer_mult = vehicle.steering_force_mult;
            let lat_grip = vehicle.lateral_grip;
            let anti_slide_k = vehicle.anti_slide_force;
            
            let mut drive_wheel_count = 0.0;
            for &c in &wheel_entities {
                if let Some(w) = wheel_storage.get(c) {
                    if w.is_drive_wheel {
                        drive_wheel_count += 1.0;
                    }
                }
            }
            let drive_wheel_count = drive_wheel_count.max(1.0);

            for &wheel_entity in &wheel_entities {
                let wheel_trans = match trans_storage.get(wheel_entity) {
                    Some(wt) => *wt, // clone the current transform state
                    None => continue,
                };
                let mut wheel = wheel_storage.get_mut(wheel_entity).unwrap();
                
                let wheel_mat = wheel_trans.global_matrix;
                let origin = Vec3::new(wheel_mat.w_axis.x, wheel_mat.w_axis.y, wheel_mat.w_axis.z);
                
                let r_ws = origin - t.position;
                let dir = t.rotation.mul_vec3(wheel.direction).normalize();"""

content = content.replace(middle_target, middle_replacement)

# Finally update the rotation logic to visually rotate the Transform
target_rot = """                    // Görsel tekerlek dönmesi (hıza göre tekerlek çevresini hesaplayarak döndür)
                    let speed = v.linear.dot(forward);
                    wheel.rotation_angle += (speed / scaled_radius) * dt;
                } else {
                    wheel.is_grounded = false;"""

replacement_rot = """                    // Görsel tekerlek dönmesi (hıza göre tekerlek çevresini hesaplayarak döndür)
                    let speed = v.linear.dot(forward);
                    wheel.rotation_angle += (speed / scaled_radius) * dt;
                    
                    // Fiziksel bileşen üzerinde Transform rotation'ı doğrudan uygula!
                    if let Some(mut wt) = trans_storage.get_mut(wheel_entity) {
                        let base_rot = gizmo_math::Quat::from_axis_angle(wheel.axle, wheel.rotation_angle);
                        
                        // Direksiyon açısı ön tekerler için eklenir
                        let steer_rot = if !wheel.is_drive_wheel {
                            gizmo_math::Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), steering_angle)
                        } else {
                            gizmo_math::Quat::IDENTITY
                        };
                        
                        wt.rotation = steer_rot * base_rot;
                        wt.update_local_matrix();
                    }
                } else {
                    wheel.is_grounded = false;"""

content = content.replace(target_rot, replacement_rot)

with open("crates/gizmo-physics/src/vehicle.rs", "w") as f:
    f.write(content)
