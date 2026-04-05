use gizmo_math::Vec3;

#[derive(Debug, Clone)]
pub struct Wheel {
    pub connection_point: Vec3, // Gövde merkezine (Center of Mass) göre lokal pozisyonu
    pub direction: Vec3,        // Süspansiyonun yere uzanma yönü (genelde 0, -1, 0)
    pub axle: Vec3,             // Tekerleğin dönme ekseni (genelde -1, 0, 0 veya 1, 0, 0)
    
    pub suspension_rest_length: f32, // Normal şartlardaki boşluk mesafesi
    pub suspension_stiffness: f32,   // Yay sertliği
    pub suspension_damping: f32,     // Sönümleme (Geri fırlamayı önler)
    pub wheel_radius: f32,           // Tekerleğin yarıçapı
    
    // Geçici durumsal veriler (Sistem tarafından her frame güncellenir)
    pub is_grounded: bool,
    pub compression: f32,            // Yerdeyse yayın ne kadar sıkıştığı
    pub contact_point: Vec3,         // Çarpışma noktası
}

impl Wheel {
    pub fn new(connection_point: Vec3, rest_length: f32, stiffness: f32, damping: f32, radius: f32) -> Self {
        Self {
            connection_point,
            direction: Vec3::new(0.0, -1.0, 0.0),
            axle: Vec3::new(-1.0, 0.0, 0.0),
            suspension_rest_length: rest_length,
            suspension_stiffness: stiffness,
            suspension_damping: damping,
            wheel_radius: radius,
            is_grounded: false,
            compression: 0.0,
            contact_point: Vec3::ZERO,
        }
    }
}

/// Raycast Vehicle Controller. Araç gövdesine (Chassis) RigidBody ile birlikte eklenmelidir.
#[derive(Debug, Clone)]
pub struct VehicleController {
    pub wheels: Vec<Wheel>,
    pub engine_force: f32,    // Motor gücü (Newton). Pozitif = ileri, Negatif = geri
    pub steering_angle: f32,  // Direksiyon açısı (Radyan). Pozitif = sola, Negatif = sağa
    pub brake_force: f32,     // Fren kuvveti (Newton)
}

impl VehicleController {
    pub fn new() -> Self {
        Self {
            wheels: Vec::new(),
            engine_force: 0.0,
            steering_angle: 0.0,
            brake_force: 0.0,
        }
    }

    pub fn add_wheel(&mut self, wheel: Wheel) {
        self.wheels.push(wheel);
    }
}
