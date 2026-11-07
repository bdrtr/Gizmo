//! An image as the sky: a panorama lights the scene, is reflected in it, and stands behind it.
//!
//! [`Renderer::set_environment_map`](crate::Renderer::set_environment_map) takes an equirectangular
//! image, the usual form of an HDRI. It goes into the reflection probes' prefiltered cube array as the
//! background slot, so specular, screen-space reflections and transparent surfaces read it the way
//! they read any probe with no box, and the array's blurriest mip is the diffuse. A probe with no box
//! of the scene's own takes the reflections instead, and the image still lights and backs the scene.
//!
//! Native only, as reflection probes are: the web's forward path has no bind group to read them through.

use crate::reflection::FACES;

/// An equirectangular panorama, linear RGB. See the [module docs](self).
#[derive(Clone, Debug, PartialEq)]
pub struct EnvironmentMap {
    pixels: Vec<[f32; 3]>,
    width: u32,
    height: u32,
    /// What its light is multiplied by, in the scene and behind it.
    pub intensity: f32,
    /// Turned about the vertical axis, in radians.
    pub rotation: f32,
    /// Edge length of the cube it is drawn into, in texels. 256 unless set.
    pub resolution: u32,
}

/// Why [`EnvironmentMap::from_hdr`] failed.
#[derive(Debug)]
#[non_exhaustive]
pub enum EnvironmentMapError {
    /// The bytes are not a Radiance `.hdr` image.
    NotAnHdrImage(String),
}

impl std::fmt::Display for EnvironmentMapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAnHdrImage(why) => write!(f, "not a Radiance HDR image: {why}"),
        }
    }
}

impl std::error::Error for EnvironmentMapError {}

impl EnvironmentMap {
    /// A panorama from its pixels, row by row from the top: `width` × `height` linear RGB.
    ///
    /// # Panics
    ///
    /// If `pixels` does not hold `width × height` of them, or either is zero.
    #[must_use]
    pub fn from_equirectangular(pixels: Vec<[f32; 3]>, width: u32, height: u32) -> Self {
        assert!(width > 0 && height > 0, "an environment map has at least one pixel");
        assert_eq!(pixels.len(), (width * height) as usize, "{width} × {height} pixels");
        Self { pixels, width, height, intensity: 1.0, rotation: 0.0, resolution: 256 }
    }

    /// A panorama from the bytes of a Radiance `.hdr` file.
    pub fn from_hdr(bytes: &[u8]) -> Result<Self, EnvironmentMapError> {
        let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Hdr)
            .map_err(|e| EnvironmentMapError::NotAnHdrImage(e.to_string()))?
            .to_rgb32f();
        let (width, height) = image.dimensions();
        let pixels = image.pixels().map(|p| p.0).collect();
        Ok(Self::from_equirectangular(pixels, width, height))
    }

    /// Builder: what its light is multiplied by.
    #[must_use]
    pub fn with_intensity(mut self, intensity: f32) -> Self {
        self.intensity = intensity.max(0.0);
        self
    }

    /// Builder: turned about the vertical axis, in radians.
    #[must_use]
    pub fn with_rotation(mut self, radians: f32) -> Self {
        self.rotation = radians;
        self
    }

    /// Builder: edge length of the cube it is drawn into, rounded up to a power of two.
    #[must_use]
    pub fn with_resolution(mut self, texels: u32) -> Self {
        self.resolution = texels.clamp(8, 1024).next_power_of_two();
        self
    }

    /// The panorama's light along `direction`, bilinearly filtered and wrapping around the seam.
    /// `+X` is the image's middle column turned by [`rotation`](Self::rotation), `+Y` its top row.
    #[must_use]
    pub fn sample(&self, direction: [f32; 3]) -> [f32; 3] {
        let [x, y, z] = direction;
        let length = (x * x + y * y + z * z).sqrt().max(1e-6);
        let (x, y, z) = (x / length, y / length, z / length);
        let u = (z.atan2(x) - self.rotation) / std::f32::consts::TAU + 0.5;
        let v = y.clamp(-1.0, 1.0).acos() / std::f32::consts::PI;
        let (fx, fy) = (u.rem_euclid(1.0) * self.width as f32 - 0.5, v * self.height as f32 - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let at = |column: f32, row: f32| {
            let column = (column as i64).rem_euclid(i64::from(self.width)) as usize;
            let row = (row as i64).clamp(0, i64::from(self.height) - 1) as usize;
            self.pixels[row * self.width as usize + column]
        };
        let (a, b, c, d) = (at(x0, y0), at(x0 + 1.0, y0), at(x0, y0 + 1.0), at(x0 + 1.0, y0 + 1.0));
        std::array::from_fn(|i| {
            let top = a[i] + (b[i] - a[i]) * tx;
            let bottom = c[i] + (d[i] - c[i]) * tx;
            top + (bottom - top) * ty
        })
    }

    /// The six faces of a cube `size` texels across, as `Rgba16Float` texels, in WebGPU's layer order.
    pub(crate) fn cube_faces(&self, size: u32) -> Vec<Vec<[u16; 4]>> {
        (0..FACES)
            .map(|face| {
                let mut texels = Vec::with_capacity((size * size) as usize);
                for row in 0..size {
                    for column in 0..size {
                        let s = (column as f32 + 0.5) / size as f32 * 2.0 - 1.0;
                        let t = (row as f32 + 0.5) / size as f32 * 2.0 - 1.0;
                        let [r, g, b] = self.sample(cube_direction(face, s, t));
                        let half = |v: f32| half::f16::from_f32(v.max(0.0)).to_bits();
                        texels.push([half(r), half(g), half(b), half(1.0)]);
                    }
                }
                texels
            })
            .collect()
    }
}

/// Where a texel of a cube face points. `s` runs along the face's rows and `t` down its columns, both
/// in `[-1, 1]`: the sampling rule of a WebGPU cube texture.
fn cube_direction(face: usize, s: f32, t: f32) -> [f32; 3] {
    match face {
        0 => [1.0, -t, -s],
        1 => [-1.0, -t, s],
        2 => [s, 1.0, t],
        3 => [s, -1.0, -t],
        4 => [s, -t, 1.0],
        _ => [-s, -t, -1.0],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A panorama painted by direction reads back by direction: red on +X, blue on −X, wrapping
    /// around the seam, white at the top row and black at the bottom.
    #[test]
    fn a_direction_finds_its_part_of_the_panorama() {
        let (w, h) = (64u32, 32u32);
        let pixels = (0..h)
            .flat_map(|row| {
                (0..w).map(move |column| {
                    let u = (column as f32 + 0.5) / w as f32;
                    if row == 0 {
                        [1.0, 1.0, 1.0]
                    } else if row == h - 1 {
                        [0.0, 0.0, 0.0]
                    } else if (0.25..0.75).contains(&u) {
                        [1.0, 0.0, 0.0]
                    } else {
                        [0.0, 0.0, 1.0]
                    }
                })
            })
            .collect();
        let map = EnvironmentMap::from_equirectangular(pixels, w, h);
        assert_eq!(map.sample([1.0, 0.0, 0.0]), [1.0, 0.0, 0.0]);
        assert_eq!(map.sample([-1.0, 0.0, 0.0]), [0.0, 0.0, 1.0], "across the seam");
        assert_eq!(map.sample([0.0, 1.0, 0.0]), [1.0, 1.0, 1.0]);
        assert_eq!(map.sample([0.0, -1.0, 0.0]), [0.0, 0.0, 0.0]);
        let turned = map.clone().with_rotation(std::f32::consts::PI);
        assert_eq!(turned.sample([-1.0, 0.0, 0.0]), [1.0, 0.0, 0.0], "half a turn swaps them");
    }

    /// Each face's centre looks along its axis, and each is right-handed in the rule's way: going right
    /// on +X goes towards −Z, and going down goes towards −Y.
    #[test]
    fn a_cube_face_points_where_webgpu_samples_it() {
        assert_eq!(cube_direction(0, 0.0, 0.0), [1.0, 0.0, 0.0]);
        assert_eq!(cube_direction(1, 0.0, 0.0), [-1.0, 0.0, 0.0]);
        assert_eq!(cube_direction(2, 0.0, 0.0), [0.0, 1.0, 0.0]);
        assert_eq!(cube_direction(3, 0.0, 0.0), [0.0, -1.0, 0.0]);
        assert_eq!(cube_direction(4, 0.0, 0.0), [0.0, 0.0, 1.0]);
        assert_eq!(cube_direction(5, 0.0, 0.0), [0.0, 0.0, -1.0]);
        assert_eq!(cube_direction(0, 1.0, 1.0), [1.0, -1.0, -1.0]);
    }

    /// A Radiance file written by hand: flat scanlines, RGBE. (128, 0, 0, 129) is 128/256 × 2^(129−128) = 1.
    #[test]
    fn a_radiance_file_reads_as_linear_light() {
        let mut bytes = b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 1 +X 2\n".to_vec();
        bytes.extend_from_slice(&[128, 0, 0, 129, 0, 128, 0, 130]);
        let map = EnvironmentMap::from_hdr(&bytes).expect("a valid file");
        assert_eq!((map.width, map.height), (2, 1));
        assert!((map.pixels[0][0] - 1.0).abs() < 0.01, "{:?}", map.pixels[0]);
        assert!((map.pixels[1][1] - 2.0).abs() < 0.02, "{:?}", map.pixels[1]);
    }

    #[test]
    fn not_an_hdr_file_is_an_error() {
        assert!(EnvironmentMap::from_hdr(b"plainly not").is_err());
    }
}
