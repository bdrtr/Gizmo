//! Spherical-harmonics (SH) probes — the global-illumination machinery.
//!
//! ## Architecture
//! 1. **SH probe**: stores second-order SH coefficients at a point in the scene (L0, L1, L2 = 9
//!    coefficients)
//! 2. **Probe grid**: a regularly spaced grid of probes across the scene
//! 3. **Baking**: an [`IrradianceVolume`](crate::irradiance::IrradianceVolume) captures the lit
//!    scene from each probe and projects it into the probe's coefficients, on the GPU
//! 4. **Runtime lookup**: trilinear interpolation from the nearest probes, in the shader
//!
//! This is the standard light-probe approach to static indirect lighting: bake once, look up
//! cheaply, and pay nothing per frame for light that never moves.
//!
//! ## What the coefficients hold
//!
//! **Irradiance over π**, convolved with the clamped cosine already: [`SHCoeffs::evaluate`]`(n)` is
//! the light a white matte surface facing `n` sends back, so one of albedo `a` sends `a × evaluate(n)`
//! — what the lighting pass adds as indirect light. Projecting radiance and evaluating the
//! projection as it stands, as this did until 2026-11-12, read a light from straight ahead at
//! `9/4π` of its irradiance where a matte surface takes `1/π` of it (2.25 times as much), and an
//! ambient of 1 at `1/4π`.

use gizmo_math::Vec3;

/// Second-order spherical-harmonics coefficients (9 coefficients × 3 channels = 27 floats).
///
/// Band 0 (L=0): 1 coefficient  → ambient light (the DC component)
/// Band 1 (L=1): 3 coefficients → the directional gradient
/// Band 2 (L=2): 5 coefficients → detail
#[derive(Debug, Clone, Copy)]
pub struct SHCoeffs {
    /// L0 band (ambient/constant term) — RGB
    pub l0: Vec3,
    /// The L1 band (directional gradients) — 3 directions × RGB
    pub l1: [Vec3; 3],
    /// The L2 band (quadratic detail) — 5 coefficients × RGB
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

/// The clamped cosine's SH convolution over π, band by band (`Â_l / π`): what turns projected
/// radiance into irradiance over π. Ramamoorthi & Hanrahan's π, 2π/3, π/4.
pub const COSINE_BANDS: [f32; 3] = [1.0, 2.0 / 3.0, 0.25];

impl SHCoeffs {
    /// Adds a distant light from `direction` (unit, towards the light) of irradiance `color` — what
    /// the lighting pass calls a directional light's colour times its intensity — as a white matte
    /// surface takes it: `evaluate(direction)` ≈ `color / π`, nothing from behind, within the ringing
    /// a second-order projection has.
    pub fn add_directional_light(&mut self, direction: Vec3, color: Vec3) {
        let d = direction.normalize();
        let [b0, b1, b2] = COSINE_BANDS;

        self.l0 += color * (0.282095 * b0);

        self.l1[0] += color * (0.488603 * d.y * b1);
        self.l1[1] += color * (0.488603 * d.z * b1);
        self.l1[2] += color * (0.488603 * d.x * b1);

        self.l2[0] += color * (1.092548 * d.x * d.y * b2);
        self.l2[1] += color * (1.092548 * d.y * d.z * b2);
        self.l2[2] += color * (0.315392 * (3.0 * d.z * d.z - 1.0) * b2);
        self.l2[3] += color * (1.092548 * d.x * d.z * b2);
        self.l2[4] += color * (0.546274 * (d.x * d.x - d.y * d.y) * b2);
    }

    /// Adds light of `radiance` from every direction alike — a uniform sky: `evaluate` reads it
    /// back as it is, in every direction.
    pub fn add_ambient(&mut self, radiance: Vec3) {
        // The projection of a constant is √(4π) of it; the cosine's band 0 is 1.
        self.l0 += radiance * (4.0 * std::f32::consts::PI).sqrt();
    }

    /// The light a white matte surface facing `direction` sends back: the irradiance there over π
    /// (see the module docs). Never negative.
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

    /// Linearly interpolates two sets of SH coefficients.
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

    /// The coefficients back from [`to_gpu_data`](Self::to_gpu_data)'s layout.
    pub fn from_gpu_data(data: &[f32; 28]) -> Self {
        let v = |i: usize| Vec3::new(data[i], data[i + 1], data[i + 2]);
        Self { l0: v(0), l1: [v(3), v(6), v(9)], l2: [v(12), v(15), v(18), v(21), v(24)] }
    }

    /// A flat float array ready to be uploaded to the GPU (27 floats).
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

/// A single light probe — holds the SH coefficients at one position in the scene.
#[derive(Debug, Clone)]
pub struct LightProbe {
    /// Where the probe sits, in world space.
    pub position: Vec3,
    /// The light around it, as irradiance over π in spherical harmonics (see the module docs).
    pub coeffs: SHCoeffs,
    /// Whether [`Self::coeffs`] has been baked yet. An unbaked probe contributes nothing rather
    /// than contributing black.
    pub is_baked: bool,
}

impl LightProbe {
    /// An unbaked probe at `position`, with zero coefficients.
    pub fn new(position: Vec3) -> Self {
        Self {
            position,
            coeffs: SHCoeffs::default(),
            is_baked: false,
        }
    }
}

/// The probe grid — probes placed at regular intervals across the scene.
pub struct ProbeGrid {
    /// The probes, in x-major grid order.
    pub probes: Vec<LightProbe>,
    /// The grid's minimum corner, in world space.
    pub grid_min: Vec3,
    /// Its maximum corner.
    pub grid_max: Vec3,
    /// How many probes along each axis, `(x, y, z)`.
    pub resolution: [u32; 3],
    /// The spacing between neighbouring probes, derived from the bounds and the resolution.
    pub cell_size: Vec3,
}

impl ProbeGrid {
    /// Builds a regular grid of probes within the given bounds.
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

    /// Reads the SH value at a world position by trilinear interpolation.
    pub fn sample(&self, world_pos: Vec3) -> SHCoeffs {
        // An empty grid (zero-resolution axis) would make `probes.len() - 1` in the
        // corner fetch underflow and panic; return a neutral coefficient set instead.
        if self.probes.is_empty() {
            return SHCoeffs::default();
        }

        // Grid koordinatlarına dönüştür
        let local = world_pos - self.grid_min;
        let fx = (local.x / self.cell_size.x - 0.5).max(0.0);
        let fy = (local.y / self.cell_size.y - 0.5).max(0.0);
        let fz = (local.z / self.cell_size.z - 0.5).max(0.0);

        let ix = (fx as u32).min(self.resolution[0].saturating_sub(2));
        let iy = (fy as u32).min(self.resolution[1].saturating_sub(2));
        let iz = (fz as u32).min(self.resolution[2].saturating_sub(2));

        // Clamp interpolation weights to [0,1]. For queries outside the probe
        // grid, `fx - ix` can exceed 1.0 (e.g. fx=1.3 with ix clamped to 0),
        // which would extrapolate instead of clamping at the boundary.
        let tx = (fx - ix as f32).clamp(0.0, 1.0);
        let ty = (fy - iy as f32).clamp(0.0, 1.0);
        let tz = (fz - iz as f32).clamp(0.0, 1.0);

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

    /// How many probes there are.
    pub fn probe_count(&self) -> usize {
        self.probes.len()
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)] // testlerde Default sonrası alan atama okunabilirlik için
mod tests {
    use super::*;

    /// **The coefficients read back what a white matte surface sends back.** A distant light of
    /// irradiance 2 from straight ahead: a surface facing it sends 2/π back (Lambert), and evaluating
    /// the coefficients reads that within the 6 % a second-order projection overshoots by; side-on
    /// and from behind, close to nothing. A uniform sky of radiance 0.3 reads 0.3 every way. The
    /// coefficients were the light's projection as it stands until 2026-11-12: 2.25 times Lambert
    /// head-on (`9/4π` against `1/π`), and an ambient of 0.3 read 0.024.
    #[test]
    fn the_coefficients_read_what_a_matte_surface_sends_back() {
        let ahead = Vec3::new(0.3, -0.8, 0.52).normalize();
        let mut sh = SHCoeffs::default();
        sh.add_directional_light(ahead, Vec3::splat(2.0));
        let lambert = 2.0 / std::f32::consts::PI;
        let facing = sh.evaluate(ahead).x;
        assert!((facing / lambert - 1.0).abs() < 0.07, "head-on {facing} against Lambert's {lambert}");
        let side = ahead.cross(Vec3::X).normalize();
        assert!(sh.evaluate(side).x < 0.1 * lambert, "side-on {}", sh.evaluate(side).x);
        assert!(sh.evaluate(-ahead).x < 0.07 * lambert, "behind {}", sh.evaluate(-ahead).x);

        let mut sky = SHCoeffs::default();
        sky.add_ambient(Vec3::new(0.3, 0.2, 0.1));
        for n in [Vec3::X, Vec3::NEG_Y, ahead] {
            assert!((sky.evaluate(n) - Vec3::new(0.3, 0.2, 0.1)).abs().max_element() < 1e-4, "{n}: {}", sky.evaluate(n));
        }
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

        // Light from overhead, warm, and a faint sky, into every probe.
        for probe in &mut grid.probes {
            probe.coeffs.add_directional_light(Vec3::Y, Vec3::new(1.0, 0.9, 0.7) * 2.0);
            probe.coeffs.add_ambient(Vec3::new(0.1, 0.1, 0.15));
            probe.is_baked = true;
        }

        // An upward-facing surface faces the light: brighter than a downward one.
        let sampled = grid.sample(Vec3::new(0.0, 2.5, 0.0));
        let up_irr = sampled.evaluate(Vec3::new(0.0, 1.0, 0.0));
        let down_irr = sampled.evaluate(Vec3::new(0.0, -1.0, 0.0));
        assert!(up_irr.x > 0.0, "Bake sonrası yukarı bakan yüzey aydınlık olmalı");
        assert!(
            up_irr.x > down_irr.x,
            "Yukarıdan gelen güneş: yukarı bakan yüzey ({}) aşağıdan ({}) daha aydınlık olmalı",
            up_irr.x,
            down_irr.x
        );
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
    fn test_sample_out_of_bounds_clamps_no_extrapolation() {
        // 1x1x1 cell grid; two probes per axis at cell centers.
        let mut grid = ProbeGrid::new(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(2.0, 2.0, 2.0),
            [2, 2, 2],
        );
        // Ramp l0.x by x-index: probes at x=0 get 0.0, at x=1 get 1.0. A correctly
        // clamped trilinear sample can never exceed the max probe value of 1.0.
        let res = grid.resolution;
        for z in 0..res[2] {
            for y in 0..res[1] {
                for x in 0..res[0] {
                    let i = (z * res[1] * res[0] + y * res[0] + x) as usize;
                    grid.probes[i].coeffs.l0 = Vec3::new(x as f32, 0.0, 0.0);
                }
            }
        }

        // Sample far outside the grid max. Before the clamp fix, the fractional
        // coordinate tx exceeded 1.0 and trilinear interpolation extrapolated,
        // pushing l0.x above the max probe value of 1.0.
        let sampled = grid.sample(Vec3::new(100.0, 100.0, 100.0));
        assert!(
            sampled.l0.x <= 1.0 + 1e-4,
            "Out-of-bounds sample must clamp, not extrapolate; got {}",
            sampled.l0.x
        );
        assert!(
            (sampled.l0.x - 1.0).abs() < 1e-4,
            "Out-of-bounds sample must clamp to boundary value 1.0, got {}",
            sampled.l0.x
        );
    }

    #[test]
    fn test_sh_gpu_data() {
        let mut sh = SHCoeffs::default();
        sh.l0 = Vec3::new(0.5, 0.6, 0.7);
        sh.l1[0] = Vec3::new(0.1, 0.2, 0.3);

        let data = sh.to_gpu_data();
        let back = SHCoeffs::from_gpu_data(&data);
        assert_eq!((back.l0, back.l1[0], back.l2[4]), (sh.l0, sh.l1[0], sh.l2[4]));
        assert_eq!(data[0], 0.5);
        assert_eq!(data[1], 0.6);
        assert_eq!(data[2], 0.7);
        assert_eq!(data[3], 0.1);
        assert_eq!(data.len(), 28);
    }
}
