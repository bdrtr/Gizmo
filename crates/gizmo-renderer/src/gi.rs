//! Spherical Harmonics (SH) Probe — Global Illumination altyapısı
//!
//! ## Mimari
//! 1. **SH Probe**: Sahnedeki bir noktada 2. derece SH katsayılarını saklar (L0, L1, L2 = 9 katsayı)
//! 2. **Probe Grid**: Sahnede düzenli aralıklarla yerleştirilmiş probe ızgarası
//! 3. **Irradiance Baking**: Her probe için ambient aydınlatma hesaplanır
//! 4. **Runtime Lookup**: Shader'da en yakın probe'lardan trilinear interpolasyon
//!
//! Bevy/Unity/Unreal'deki Light Probe sistemiyle eşdeğerdir.

use gizmo_math::Vec3;

/// 2. derece Spherical Harmonics katsayıları (9 katsayı × 3 kanal = 27 float)
///
/// Band 0 (L=0): 1 katsayı  → ortam ışığı (DC bileşen)
/// Band 1 (L=1): 3 katsayı  → yönlü gradient
/// Band 2 (L=2): 5 katsayı  → detay
#[derive(Debug, Clone, Copy)]
pub struct SHCoeffs {
    /// L0 band (ambient/constant term) — RGB
    pub l0: Vec3,
    /// L1 band (directional gradients) — 3 yön × RGB
    pub l1: [Vec3; 3],
    /// L2 band (quadratic detail) — 5 katsayı × RGB
    pub l2: [Vec3; 5],
}

impl Default for SHCoeffs {
    fn default() -> Self {
        Self {
            l0: Vec3::ZERO,
            l1: [Vec3::ZERO; 3],
            l2: [Vec3::ZERO; 5],
        }
    }
}

impl SHCoeffs {
    /// Verilen yönden gelen ışığı SH katsayılarına ekler
    pub fn add_directional_light(&mut self, direction: Vec3, color: Vec3) {
        let d = direction.normalize();

        // SH basis fonksiyonları (2. derece)
        // Band 0
        let y0 = 0.282095; // 1 / (2 * sqrt(pi))
        self.l0 += color * y0;

        // Band 1
        let y1_neg1 = 0.488603 * d.y; // sqrt(3) / (2*sqrt(pi)) * y
        let y1_0 = 0.488603 * d.z; // sqrt(3) / (2*sqrt(pi)) * z
        let y1_pos1 = 0.488603 * d.x; // sqrt(3) / (2*sqrt(pi)) * x
        self.l1[0] += color * y1_neg1;
        self.l1[1] += color * y1_0;
        self.l1[2] += color * y1_pos1;

        // Band 2
        let y2_neg2 = 1.092548 * d.x * d.y; // sqrt(15)/(2*sqrt(pi)) * xy
        let y2_neg1 = 1.092548 * d.y * d.z; // sqrt(15)/(2*sqrt(pi)) * yz
        let y2_0 = 0.315392 * (3.0 * d.z * d.z - 1.0); // sqrt(5)/(4*sqrt(pi)) * (3z²-1)
        let y2_pos1 = 1.092548 * d.x * d.z; // sqrt(15)/(2*sqrt(pi)) * xz
        let y2_pos2 = 0.546274 * (d.x * d.x - d.y * d.y); // sqrt(15)/(4*sqrt(pi)) * (x²-y²)
        self.l2[0] += color * y2_neg2;
        self.l2[1] += color * y2_neg1;
        self.l2[2] += color * y2_0;
        self.l2[3] += color * y2_pos1;
        self.l2[4] += color * y2_pos2;
    }

    /// Verilen yöndeki ışık radyansını (irradiance) hesaplar
    pub fn evaluate(&self, direction: Vec3) -> Vec3 {
        let d = direction.normalize();

        let y0 = 0.282095;
        let y1_neg1 = 0.488603 * d.y;
        let y1_0 = 0.488603 * d.z;
        let y1_pos1 = 0.488603 * d.x;
        let y2_neg2 = 1.092548 * d.x * d.y;
        let y2_neg1 = 1.092548 * d.y * d.z;
        let y2_0 = 0.315392 * (3.0 * d.z * d.z - 1.0);
        let y2_pos1 = 1.092548 * d.x * d.z;
        let y2_pos2 = 0.546274 * (d.x * d.x - d.y * d.y);

        let mut result = self.l0 * y0;
        result += self.l1[0] * y1_neg1;
        result += self.l1[1] * y1_0;
        result += self.l1[2] * y1_pos1;
        result += self.l2[0] * y2_neg2;
        result += self.l2[1] * y2_neg1;
        result += self.l2[2] * y2_0;
        result += self.l2[3] * y2_pos1;
        result += self.l2[4] * y2_pos2;

        // Negatif ışık olamaz
        Vec3::new(result.x.max(0.0), result.y.max(0.0), result.z.max(0.0))
    }

    /// İki SH katsayısını lineer interpolasyon yapar
    pub fn lerp(&self, other: &Self, t: f32) -> Self {
        let lerp_v = |a: Vec3, b: Vec3| -> Vec3 {
            Vec3::new(
                a.x + (b.x - a.x) * t,
                a.y + (b.y - a.y) * t,
                a.z + (b.z - a.z) * t,
            )
        };

        Self {
            l0: lerp_v(self.l0, other.l0),
            l1: [
                lerp_v(self.l1[0], other.l1[0]),
                lerp_v(self.l1[1], other.l1[1]),
                lerp_v(self.l1[2], other.l1[2]),
            ],
            l2: [
                lerp_v(self.l2[0], other.l2[0]),
                lerp_v(self.l2[1], other.l2[1]),
                lerp_v(self.l2[2], other.l2[2]),
                lerp_v(self.l2[3], other.l2[3]),
                lerp_v(self.l2[4], other.l2[4]),
            ],
        }
    }

    /// GPU'ya gönderilebilecek flat float dizisi (27 float)
    pub fn to_gpu_data(&self) -> [f32; 28] {
        let mut data = [0.0f32; 28]; // 28 = 27 + 1 padding (16-byte aligned)
        data[0] = self.l0.x;
        data[1] = self.l0.y;
        data[2] = self.l0.z;
        data[3] = self.l1[0].x;
        data[4] = self.l1[0].y;
        data[5] = self.l1[0].z;
        data[6] = self.l1[1].x;
        data[7] = self.l1[1].y;
        data[8] = self.l1[1].z;
        data[9] = self.l1[2].x;
        data[10] = self.l1[2].y;
        data[11] = self.l1[2].z;
        data[12] = self.l2[0].x;
        data[13] = self.l2[0].y;
        data[14] = self.l2[0].z;
        data[15] = self.l2[1].x;
        data[16] = self.l2[1].y;
        data[17] = self.l2[1].z;
        data[18] = self.l2[2].x;
        data[19] = self.l2[2].y;
        data[20] = self.l2[2].z;
        data[21] = self.l2[3].x;
        data[22] = self.l2[3].y;
        data[23] = self.l2[3].z;
        data[24] = self.l2[4].x;
        data[25] = self.l2[4].y;
        data[26] = self.l2[4].z;
        data[27] = 0.0; // padding
        data
    }
}

/// Tek bir Light Probe — sahne içindeki belirli bir pozisyonda SH katsayılarını tutar
#[derive(Debug, Clone)]
pub struct LightProbe {
    pub position: Vec3,
    pub coeffs: SHCoeffs,
    pub is_baked: bool,
}

impl LightProbe {
    pub fn new(position: Vec3) -> Self {
        Self {
            position,
            coeffs: SHCoeffs::default(),
            is_baked: false,
        }
    }
}

/// Probe Grid — Sahnede düzenli aralıklarla yerleştirilmiş probe ızgarası
pub struct ProbeGrid {
    pub probes: Vec<LightProbe>,
    pub grid_min: Vec3,
    pub grid_max: Vec3,
    pub resolution: [u32; 3], // (x, y, z)
    pub cell_size: Vec3,
}

impl ProbeGrid {
    /// Verilen sınırlar içinde düzenli bir probe ızgarası oluşturur
    pub fn new(min: Vec3, max: Vec3, resolution: [u32; 3]) -> Self {
        let extent = max - min;
        let cell_size = Vec3::new(
            extent.x / resolution[0].max(1) as f32,
            extent.y / resolution[1].max(1) as f32,
            extent.z / resolution[2].max(1) as f32,
        );

        let mut probes = Vec::new();
        for z in 0..resolution[2] {
            for y in 0..resolution[1] {
                for x in 0..resolution[0] {
                    let pos = Vec3::new(
                        min.x + (x as f32 + 0.5) * cell_size.x,
                        min.y + (y as f32 + 0.5) * cell_size.y,
                        min.z + (z as f32 + 0.5) * cell_size.z,
                    );
                    probes.push(LightProbe::new(pos));
                }
            }
        }

        Self {
            probes,
            grid_min: min,
            grid_max: max,
            resolution,
            cell_size,
        }
    }

    /// Basit baking: Sahne ışıklarından her probe için SH katsayılarını hesaplar
    ///
    /// Gerçek bir path-tracer yerine, mevcut DirectionalLight ve PointLight'ları
    /// analitik olarak SH'ya projekte ederiz.
    pub fn bake_from_lights(
        &mut self,
        directional_lights: &[(Vec3, Vec3, f32)], // (direction, color, intensity)
        point_lights: &[(Vec3, Vec3, f32, f32)],  // (position, color, intensity, radius)
        ambient_color: Vec3,
    ) {
        let start = std::time::Instant::now();

        for probe in &mut self.probes {
            let mut coeffs = SHCoeffs::default();

            // Ambient (sabit terim)
            coeffs.l0 = ambient_color * 0.282095;

            // Directional lights
            for &(dir, color, intensity) in directional_lights {
                coeffs.add_directional_light(dir, color * intensity);
            }

            // Point lights (probe'a göre yön ve uzaklık hesapla)
            for &(light_pos, color, intensity, radius) in point_lights {
                let to_light = light_pos - probe.position;
                let dist = to_light.length();
                if dist < 0.001 || dist > radius {
                    continue;
                }

                let direction = to_light / dist;
                let attenuation = 1.0 - (dist / radius).min(1.0);
                let attenuation = attenuation * attenuation; // quadratic falloff

                coeffs.add_directional_light(direction, color * intensity * attenuation);
            }

            probe.coeffs = coeffs;
            probe.is_baked = true;
        }

        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        println!(
            "[GI] {} probe bake edildi ({:.1}ms)",
            self.probes.len(),
            elapsed
        );
    }

    /// Dünya pozisyonundaki noktadan trilineer interpolasyon ile SH değeri okur
    pub fn sample(&self, world_pos: Vec3) -> SHCoeffs {
        // Grid koordinatlarına dönüştür
        let local = world_pos - self.grid_min;
        let fx = (local.x / self.cell_size.x - 0.5).max(0.0);
        let fy = (local.y / self.cell_size.y - 0.5).max(0.0);
        let fz = (local.z / self.cell_size.z - 0.5).max(0.0);

        let ix = (fx as u32).min(self.resolution[0].saturating_sub(2));
        let iy = (fy as u32).min(self.resolution[1].saturating_sub(2));
        let iz = (fz as u32).min(self.resolution[2].saturating_sub(2));

        let tx = fx - ix as f32;
        let ty = fy - iy as f32;
        let tz = fz - iz as f32;

        // 8 köşe probe'u oku
        let idx = |x: u32, y: u32, z: u32| -> usize {
            (z * self.resolution[1] * self.resolution[0] + y * self.resolution[0] + x) as usize
        };

        let get = |x: u32, y: u32, z: u32| -> &SHCoeffs {
            let i = idx(x, y, z).min(self.probes.len() - 1);
            &self.probes[i].coeffs
        };

        // Trilineer interpolasyon
        let c000 = get(ix, iy, iz);
        let c100 = get(ix + 1, iy, iz);
        let c010 = get(ix, iy + 1, iz);
        let c110 = get(ix + 1, iy + 1, iz);
        let c001 = get(ix, iy, iz + 1);
        let c101 = get(ix + 1, iy, iz + 1);
        let c011 = get(ix, iy + 1, iz + 1);
        let c111 = get(ix + 1, iy + 1, iz + 1);

        let c00 = c000.lerp(c100, tx);
        let c01 = c001.lerp(c101, tx);
        let c10 = c010.lerp(c110, tx);
        let c11 = c011.lerp(c111, tx);

        let c0 = c00.lerp(&c10, ty);
        let c1 = c01.lerp(&c11, ty);

        c0.lerp(&c1, tz)
    }

    /// Probe sayısı
    pub fn probe_count(&self) -> usize {
        self.probes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sh_directional_light() {
        let mut sh = SHCoeffs::default();
        sh.add_directional_light(Vec3::new(0.0, -1.0, 0.0), Vec3::new(1.0, 1.0, 1.0));

        // Aşağı bakan yüzey ışık almalı
        let irradiance_down = sh.evaluate(Vec3::new(0.0, -1.0, 0.0));
        assert!(irradiance_down.x > 0.0, "Aşağıda ışık olmalı");

        // Yukarı bakan yüzey ışık almamalı (veya çok az)
        let irradiance_up = sh.evaluate(Vec3::new(0.0, 1.0, 0.0));
        assert!(
            irradiance_down.x > irradiance_up.x,
            "Işık kaynağı yönünde daha fazla ışık olmalı"
        );
    }

    #[test]
    fn test_sh_evaluate_symmetry() {
        let mut sh = SHCoeffs::default();
        // Saf ambient (tüm yönlerden eşit)
        sh.l0 = Vec3::new(1.0, 1.0, 1.0);

        let up = sh.evaluate(Vec3::new(0.0, 1.0, 0.0));
        let down = sh.evaluate(Vec3::new(0.0, -1.0, 0.0));
        let right = sh.evaluate(Vec3::new(1.0, 0.0, 0.0));

        // L0 sadece ambient — tüm yönler yaklaşık eşit olmalı
        assert!((up.x - down.x).abs() < 0.01, "Simetri: yukarı ≈ aşağı");
        assert!((up.x - right.x).abs() < 0.01, "Simetri: yukarı ≈ sağ");
    }

    #[test]
    fn test_probe_grid_creation() {
        let grid = ProbeGrid::new(
            Vec3::new(-10.0, 0.0, -10.0),
            Vec3::new(10.0, 5.0, 10.0),
            [4, 2, 4],
        );

        assert_eq!(grid.probe_count(), 4 * 2 * 4);
        assert_eq!(grid.probes.len(), 32);
    }

    #[test]
    fn test_probe_grid_bake_and_sample() {
        let mut grid = ProbeGrid::new(
            Vec3::new(-5.0, 0.0, -5.0),
            Vec3::new(5.0, 5.0, 5.0),
            [2, 2, 2],
        );

        // Güneş ışığı
        let dir_lights = vec![(
            Vec3::new(0.0, -1.0, 0.0), // Yukarıdan aşağı
            Vec3::new(1.0, 0.9, 0.7),  // Sıcak beyaz
            2.0,                       // Yoğunluk
        )];

        grid.bake_from_lights(&dir_lights, &[], Vec3::new(0.1, 0.1, 0.15));

        // Tüm probe'lar bake edilmiş olmalı
        assert!(grid.probes.iter().all(|p| p.is_baked));

        // Sample al
        let sampled = grid.sample(Vec3::new(0.0, 2.5, 0.0));
        let down_irr = sampled.evaluate(Vec3::new(0.0, -1.0, 0.0));
        assert!(down_irr.x > 0.0, "Bake sonrası irradiance pozitif olmalı");
    }

    #[test]
    fn test_sh_lerp() {
        let a = SHCoeffs::default();
        let mut b = SHCoeffs::default();
        b.l0 = Vec3::new(2.0, 2.0, 2.0);

        let mid = a.lerp(&b, 0.5);
        assert!((mid.l0.x - 1.0).abs() < 0.001, "Lerp ortası 1.0 olmalı");
    }

    #[test]
    fn test_sh_gpu_data() {
        let mut sh = SHCoeffs::default();
        sh.l0 = Vec3::new(0.5, 0.6, 0.7);
        sh.l1[0] = Vec3::new(0.1, 0.2, 0.3);

        let data = sh.to_gpu_data();
        assert_eq!(data[0], 0.5);
        assert_eq!(data[1], 0.6);
        assert_eq!(data[2], 0.7);
        assert_eq!(data[3], 0.1);
        assert_eq!(data.len(), 28);
    }
}
