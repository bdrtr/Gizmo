use gizmo_math::Vec3;

// En performanslı 3D çarpışma nesneleri: Küre, Kutu, Kapsül ve Konveks Gövde

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Sphere {
    pub radius: f32,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Aabb {
    pub half_extents: Vec3,
}

/// Kapsül şekli — İki yarıküre + silindir. Karakter kontrolcüsü için ideal.
/// Dikey eksen Y'dir: üst merkez (0, half_height, 0), alt merkez (0, -half_height, 0)
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Capsule {
    pub radius: f32,
    pub half_height: f32, // Yarıküre merkezleri arası mesafenin yarısı
}

/// Konveks Gövde — Rastgele vertex kümesinin konveks zarfı. Mesh collider için kullanılır.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConvexHull {
    pub vertices: Vec<Vec3>,
}

// ECS Component'i olmak üzere hazırlanmış Çarpışma Şekilleri listesi
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
pub enum ColliderShape {
    Sphere(Sphere),
    Aabb(Aabb),
    Capsule(Capsule),
    ConvexHull(ConvexHull),
    #[serde(skip)]
    Swept {
        base: Box<ColliderShape>,
        sweep_vector: Vec3,
    },
    // Yükseklik alanı tabanlı çarpışma yüzeyi (Arazi / Terrain)
    HeightField {
        heights: Vec<f32>,
        segments_x: u32,
        segments_z: u32,
        width: f32,
        depth: f32,
        max_height: f32,
    },
}

impl ColliderShape {
    /// GJK Algoritması için Destek Fonksiyonu (Support Function).
    /// Verilen bir 'direction' (yön) vektörü üzerinde, şeklin en uç noktasını döndürür.
    /// Transform (pozisyon ve rotasyon) dikkate alınarak Dünya Koordinatında (`World Space`) hesaplar.
    pub fn support_point(&self, pos: Vec3, rot: gizmo_math::Quat, mut dir: Vec3) -> Vec3 {
        if dir.length_squared() < 0.0001 {
            dir = Vec3::new(1.0, 0.0, 0.0);
        } else {
            dir = dir.normalize();
        }

        match self {
            ColliderShape::Sphere(s) => pos + dir * s.radius,
            ColliderShape::Aabb(aabb) => {
                // AABB'yi gerçek rotasyona tepki veren bir OBB gibi ele alıyoruz:
                let local_dir = rot.inverse().mul_vec3(dir);
                let lx = if local_dir.x >= 0.0 {
                    aabb.half_extents.x
                } else {
                    -aabb.half_extents.x
                };
                let ly = if local_dir.y >= 0.0 {
                    aabb.half_extents.y
                } else {
                    -aabb.half_extents.y
                };
                let lz = if local_dir.z >= 0.0 {
                    aabb.half_extents.z
                } else {
                    -aabb.half_extents.z
                };
                pos + rot.mul_vec3(Vec3::new(lx, ly, lz))
            }
            ColliderShape::Capsule(cap) => {
                // Arama vektörünü lokal uzaya çek (Böylece vertexlere döngüde rotasyon uygulamaktan kurtuluyoruz)
                let local_dir = rot.inverse().mul_vec3(dir);
                let local_top = Vec3::new(0.0, cap.half_height, 0.0);
                let local_bot = Vec3::new(0.0, -cap.half_height, 0.0);

                // Lokal yönü baz alıp seçimi yap
                let best_local = if local_dir.dot(local_top) >= local_dir.dot(local_bot) {
                    local_top
                } else {
                    local_bot
                };

                // Sadece seçili (1 adet) noktayı tekrar dünyaya çevirip küre yarıçapını ekle
                pos + rot.mul_vec3(best_local) + dir * cap.radius
            }
            ColliderShape::ConvexHull(hull) => {
                let local_dir = rot.inverse().mul_vec3(dir);
                let mut best_dot = f32::NEG_INFINITY;
                let mut best_local = Vec3::ZERO;

                for v in &hull.vertices {
                    let d = local_dir.dot(*v);
                    if d > best_dot {
                        best_dot = d;
                        best_local = *v;
                    }
                }
                pos + rot.mul_vec3(best_local)
            }
            ColliderShape::Swept { base, sweep_vector } => {
                // Sweep mantığı: Orijinal şeklin uç noktasına, eğer tarama yönü 'dir' ile aynı yöndeyse
                // sweep_vector (süpürme hareketi) eklenir. Bu sayede şekil hareket ettiği hacmi kapsar.
                let offset = if dir.dot(*sweep_vector) > 0.0 {
                    *sweep_vector
                } else {
                    Vec3::ZERO
                };
                base.support_point(pos, rot, dir) + offset
            }
            ColliderShape::HeightField {
                heights,
                segments_x,
                segments_z,
                width,
                depth,
                max_height,
            } => {
                let local_dir = rot.inverse().mul_vec3(dir);
                let half_w = *width * 0.5;
                let half_d = *depth * 0.5;

                // X ve Z eksenlerinde yön işaretine göre kenar seç
                let lx = if local_dir.x >= 0.0 { half_w } else { -half_w };
                let lz = if local_dir.z >= 0.0 { half_d } else { -half_d };

                // Y ekseni: yön aşağıya ise taban (y=0), yukarıya ise en yüksek grid noktası
                let ly = if local_dir.y >= 0.0 {
                    // Yukarı yön: 4 köşe grid noktasının yüksekliklerinden en büyüğü
                    // artı tüm grid'in max'ı da kontrol et
                    let sx = (*segments_x).saturating_sub(1) as usize;
                    let sz = (*segments_z).saturating_sub(1) as usize;
                    let h = |gx: usize, gz: usize| -> f32 {
                        let idx = gz * (*segments_x as usize) + gx;
                        if idx < heights.len() { heights[idx] * *max_height } else { 0.0 }
                    };
                    // Kenar köşelerin yükseklikleri
                    let h00 = h(0, 0);
                    let h10 = h(sx, 0);
                    let h01 = h(0, sz);
                    let h11 = h(sx, sz);
                    h00.max(h10).max(h01).max(h11)
                } else {
                    0.0 // Taban
                };

                // Seçilen kenar noktayı lokal uzaydan dünya uzayına çevir
                pos + rot.mul_vec3(Vec3::new(lx, ly, lz))
            }
        }
    }

    /// Tüm boyut şekillerini çevreleyen temel bir Bounding Box (AABB) üretir.
    /// Kesişim (Raycast/Broadphase) algoritmalarında ön ve hızlı test için gereklidir.
    pub fn bounding_box_half_extents(&self, rot: gizmo_math::Quat) -> Vec3 {
        match self {
            ColliderShape::Sphere(s) => Vec3::new(s.radius, s.radius, s.radius),
            ColliderShape::Aabb(a) => {
                let rx = rot.mul_vec3(Vec3::new(a.half_extents.x, 0.0, 0.0));
                let ry = rot.mul_vec3(Vec3::new(0.0, a.half_extents.y, 0.0));
                let rz = rot.mul_vec3(Vec3::new(0.0, 0.0, a.half_extents.z));
                Vec3::new(
                    rx.x.abs() + ry.x.abs() + rz.x.abs(),
                    rx.y.abs() + ry.y.abs() + rz.y.abs(),
                    rx.z.abs() + ry.z.abs() + rz.z.abs(),
                )
            }
            ColliderShape::Capsule(c) => {
                let r_top = rot.mul_vec3(Vec3::new(0.0, c.half_height, 0.0));
                Vec3::new(
                    r_top.x.abs() + c.radius,
                    r_top.y.abs() + c.radius,
                    r_top.z.abs() + c.radius,
                )
            }
            ColliderShape::ConvexHull(c) => {
                let mut max_x = 0.0_f32;
                let mut max_y = 0.0_f32;
                let mut max_z = 0.0_f32;
                for v in &c.vertices {
                    let v_rot = rot.mul_vec3(*v);
                    max_x = max_x.max(v_rot.x.abs());
                    max_y = max_y.max(v_rot.y.abs());
                    max_z = max_z.max(v_rot.z.abs());
                }
                Vec3::new(max_x, max_y, max_z)
            }
            ColliderShape::Swept { base, sweep_vector } => {
                let base_ext = base.bounding_box_half_extents(rot);
                Vec3::new(
                    base_ext.x + sweep_vector.x.abs() * 0.5,
                    base_ext.y + sweep_vector.y.abs() * 0.5,
                    base_ext.z + sweep_vector.z.abs() * 0.5,
                )
            }
            ColliderShape::HeightField {
                width,
                depth,
                max_height,
                ..
            } => {
                // Heightfield generally unrotated handling
                Vec3::new(*width * 0.5, *max_height * 0.5, *depth * 0.5)
            }
        }
    }
}

/// Fiziksel çarpışma bileşeni.
///
/// **ÖNEMLİ TASARIM KARARI — `Transform.scale` ETKİSİ:**
/// Collider boyutları (half_extents, radius, half_height) `Transform.scale` ile
/// otomatik olarak **çarpılmaz**. Şekil boyutlarını doğrudan oluştururken belirleyin.
///
/// Bu bilinçli bir karardır:
/// - Fizik deterministik kalır (scale animasyonu collision'ı bozmaz)
/// - Runtime'da her frame scale × collider çarpımı yapılmaz
/// - Non-uniform scale (2, 1, 3) küreyi elipse çevirirdi ki bu GJK/EPA'yı bozar
///
/// Doğru kullanım örneği:
/// ```
/// // Görsel: Transform scale (2, 2, 2) ile 2x büyütülmüş küp
/// // Collider: half_extents de 2x olmalı → new_aabb(2.0, 2.0, 2.0)
/// ```
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Collider {
    pub shape: ColliderShape,
}

impl Collider {
    pub fn new_sphere(radius: f32) -> Self {
        Self {
            shape: ColliderShape::Sphere(Sphere { radius }),
        }
    }

    pub fn new_aabb(hx: f32, hy: f32, hz: f32) -> Self {
        Self {
            shape: ColliderShape::Aabb(Aabb {
                half_extents: Vec3::new(hx, hy, hz),
            }),
        }
    }

    /// Kapsül collider oluşturur. Toplam yükseklik = 2*(half_height + radius)
    pub fn new_capsule(radius: f32, half_height: f32) -> Self {
        Self {
            shape: ColliderShape::Capsule(Capsule {
                radius,
                half_height,
            }),
        }
    }

    /// Konveks gövde collider oluşturur. Vertex'ler lokal koordinatlarda verilir.
    pub fn new_convex(vertices: Vec<Vec3>) -> Self {
        Self {
            shape: ColliderShape::ConvexHull(ConvexHull { vertices }),
        }
    }
}
