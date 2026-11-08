//! A colour look-up table: a grade from a colour tool, or a view transform defined by a table.
//!
//! [`Renderer::set_color_lut`](crate::Renderer::set_color_lut) takes one. Read from a `.cube` file, the
//! format grading tools write, it is a cube of colours `size` texels on a side, and the post composite
//! looks a colour up in it with trilinear filtering. What goes in and what comes out depends on its
//! [`LutDomain`](crate::color_lut::LutDomain):
//!
//! - [`LutDomain::Grade`](crate::color_lut::LutDomain::Grade) is a look applied after the tone curve, display-encoded colour in and out. That
//!   is what a `.cube` from a grading tool is.
//! - [`LutDomain::Reinhard`](crate::color_lut::LutDomain::Reinhard) and [`LutDomain::Log2`](crate::color_lut::LutDomain::Log2) replace the tone curve. They take the scene's light,
//!   shaped to fit a cube, which is how view transforms that are defined by a table are made: Tony
//!   McMapface and SBDT take `x / (1 + x)` and give display light, and Blender's Filmic and AgX looks
//!   take `log2` between two exposures and give display-encoded colour.
//!
//! The engine ships none of those tables. Each comes with its own licence, and this is how to use one.

/// What a [`ColorLut`] takes and gives. See the [module docs](self).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
#[non_exhaustive]
pub enum LutDomain {
    /// After the tone curve: the display-encoded (sRGB) colour in, a display-encoded colour out.
    #[default]
    Grade,
    /// In place of the tone curve: the scene's light as `x / (1 + x)` in, display light out.
    Reinhard,
    /// In place of the tone curve: the scene's light as `log2` between `min_ev` and `max_ev` (stops
    /// around 1), display-encoded colour out.
    Log2 {
        /// The darkest stop the table covers.
        min_ev: f32,
        /// The brightest.
        max_ev: f32,
    },
}

impl LutDomain {
    /// `[mode, min_ev, max_ev]` as the post shader reads them: 1 a grade, 2 Reinhard, 3 log2; 0 is no table.
    pub(crate) fn params(self) -> [f32; 3] {
        match self {
            LutDomain::Grade => [1.0, 0.0, 0.0],
            LutDomain::Reinhard => [2.0, 0.0, 0.0],
            LutDomain::Log2 { min_ev, max_ev } => [3.0, min_ev, max_ev],
        }
    }
}

/// Why [`ColorLut::from_cube`] failed.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ColorLutError {
    /// No `LUT_3D_SIZE` line, or a size outside 2..=128.
    Size,
    /// A line that is neither a keyword nor three numbers.
    Line(usize),
    /// Another number of entries than `size³`.
    Entries {
        /// What the size said.
        expected: usize,
        /// What the file had.
        found: usize,
    },
    /// A one-dimensional table (`LUT_1D_SIZE`), which this does not read.
    OneDimensional,
}

impl std::fmt::Display for ColorLutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ColorLutError::Size => write!(f, "no LUT_3D_SIZE between 2 and 128"),
            ColorLutError::Line(n) => write!(f, "line {n} is neither a keyword nor three numbers"),
            ColorLutError::Entries { expected, found } => write!(f, "{found} entries where the size asks for {expected}"),
            ColorLutError::OneDimensional => write!(f, "a 1D table; only 3D ones are read"),
        }
    }
}

impl std::error::Error for ColorLutError {}

/// A cube of colours, red varying fastest. See the [module docs](self).
#[derive(Clone, Debug, PartialEq)]
pub struct ColorLut {
    size: u32,
    texels: Vec<[f32; 3]>,
    /// What it takes and gives.
    pub domain: LutDomain,
}

impl ColorLut {
    /// A table that changes nothing, `size` texels on a side.
    #[must_use]
    pub fn identity(size: u32) -> Self {
        let size = size.clamp(2, 128);
        let step = |i: u32| i as f32 / (size - 1) as f32;
        let texels = (0..size * size * size)
            .map(|i| [step(i % size), step(i / size % size), step(i / (size * size))])
            .collect();
        Self { size, texels, domain: LutDomain::Grade }
    }

    /// A table from the text of a `.cube` file (Adobe's and Resolve's format): `LUT_3D_SIZE`, an
    /// optional `DOMAIN_MIN` and `DOMAIN_MAX`, then `size³` lines of three numbers, red fastest. A domain
    /// other than 0..1 is folded into the entries' positions, not kept.
    pub fn from_cube(text: &str) -> Result<Self, ColorLutError> {
        let mut size = None;
        let mut texels = Vec::new();
        let (mut low, mut high) = ([0.0f32; 3], [1.0f32; 3]);
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut words = line.split_whitespace();
            let first = words.next().unwrap_or_default();
            let three = |words: std::str::SplitWhitespace<'_>| -> Option<[f32; 3]> {
                let v: Vec<f32> = words.map(str::parse).collect::<Result<_, _>>().ok()?;
                <[f32; 3]>::try_from(v).ok()
            };
            match first {
                "TITLE" => {}
                "LUT_1D_SIZE" => return Err(ColorLutError::OneDimensional),
                "LUT_3D_SIZE" => {
                    size = words.next().and_then(|w| w.parse::<u32>().ok()).filter(|s| (2..=128).contains(s));
                    if size.is_none() {
                        return Err(ColorLutError::Size);
                    }
                }
                "DOMAIN_MIN" => low = three(words).ok_or(ColorLutError::Line(n + 1))?,
                "DOMAIN_MAX" => high = three(words).ok_or(ColorLutError::Line(n + 1))?,
                _ => {
                    let entry = three(line.split_whitespace()).ok_or(ColorLutError::Line(n + 1))?;
                    texels.push(entry);
                }
            }
        }
        let size = size.ok_or(ColorLutError::Size)?;
        let expected = (size * size * size) as usize;
        if texels.len() != expected {
            return Err(ColorLutError::Entries { expected, found: texels.len() });
        }
        let lut = Self { size, texels, domain: LutDomain::Grade };
        if low == [0.0; 3] && high == [1.0; 3] {
            return Ok(lut);
        }
        // Resample onto 0..1, so the shader has one way to look a colour up.
        let step = |i: u32| i as f32 / (size - 1) as f32;
        let texels = (0..size * size * size)
            .map(|i| {
                let unit = [step(i % size), step(i / size % size), step(i / (size * size))];
                lut.sample(std::array::from_fn(|c| (unit[c] - low[c]) / (high[c] - low[c]).max(1e-6)))
            })
            .collect();
        Ok(Self { texels, ..lut })
    }

    /// Builder: what it takes and gives.
    #[must_use]
    pub fn with_domain(mut self, domain: LutDomain) -> Self {
        self.domain = domain;
        self
    }

    /// Texels on a side.
    #[must_use]
    pub fn size(&self) -> u32 {
        self.size
    }

    /// What it gives for `rgb`, each in 0..1, trilinearly filtered: what the shader does.
    #[must_use]
    pub fn sample(&self, rgb: [f32; 3]) -> [f32; 3] {
        let n = self.size as usize;
        let last = (n - 1) as f32;
        let pos: [f32; 3] = std::array::from_fn(|c| rgb[c].clamp(0.0, 1.0) * last);
        let base: [usize; 3] = std::array::from_fn(|c| (pos[c].floor() as usize).min(n - 2));
        let t: [f32; 3] = std::array::from_fn(|c| pos[c] - base[c] as f32);
        let at = |r: usize, g: usize, b: usize| self.texels[r + g * n + b * n * n];
        let mut out = [0.0; 3];
        for corner in 0..8 {
            let d = [corner & 1, (corner >> 1) & 1, (corner >> 2) & 1];
            let w: f32 = (0..3).map(|c| if d[c] == 1 { t[c] } else { 1.0 - t[c] }).product();
            let v = at(base[0] + d[0], base[1] + d[1], base[2] + d[2]);
            for c in 0..3 {
                out[c] += v[c] * w;
            }
        }
        out
    }

    /// The table as a 3D texture for the post composite.
    pub(crate) fn upload(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
        let size = wgpu::Extent3d { width: self.size, height: self.size, depth_or_array_layers: self.size };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("colour look-up table"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let half = |v: f32| half::f16::from_f32(v).to_bits();
        let data: Vec<u16> = self.texels.iter().flat_map(|&[r, g, b]| [half(r), half(g), half(b), half(1.0)]).collect();
        queue.write_texture(
            texture.as_image_copy(),
            bytemuck::cast_slice(&data),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(self.size * 8),
                rows_per_image: Some(self.size),
            },
            size,
        );
        texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("colour look-up table"),
            dimension: Some(wgpu::TextureViewDimension::D3),
            ..Default::default()
        })
    }
}

/// What the composite binds while no table is set: two empty texels on a side, which mode 0 never reads.
pub(crate) fn unset_view(device: &wgpu::Device) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("no colour look-up table"),
            size: wgpu::Extent3d { width: 2, height: 2, depth_or_array_layers: 2 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D3),
            ..Default::default()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file written by hand: a 2-texel table that swaps red and blue, with a title and a comment.
    const SWAP: &str = "# swaps red and blue\nTITLE \"swap\"\nLUT_3D_SIZE 2\n\
        0 0 0\n0 0 1\n0 1 0\n0 1 1\n1 0 0\n1 0 1\n1 1 0\n1 1 1\n";

    #[test]
    fn a_cube_file_reads_red_fastest() {
        let lut = ColorLut::from_cube(SWAP).expect("a valid file");
        assert_eq!(lut.size(), 2);
        assert_eq!(lut.sample([1.0, 0.0, 0.0]), [0.0, 0.0, 1.0], "red becomes blue");
        assert_eq!(lut.sample([0.0, 0.0, 1.0]), [1.0, 0.0, 0.0], "blue becomes red");
        assert_eq!(lut.sample([0.0, 1.0, 0.0]), [0.0, 1.0, 0.0], "green stays");
        let mid = lut.sample([0.25, 0.5, 0.75]);
        assert!((mid[0] - 0.75).abs() < 1e-6 && (mid[2] - 0.25).abs() < 1e-6, "trilinear in between: {mid:?}");
    }

    #[test]
    fn the_identity_changes_nothing() {
        let lut = ColorLut::identity(17);
        for rgb in [[0.0, 0.0, 0.0], [0.3, 0.6, 0.9], [1.0, 1.0, 1.0], [0.123, 0.456, 0.789]] {
            let out = lut.sample(rgb);
            assert!(out.iter().zip(rgb).all(|(o, i)| (o - i).abs() < 1e-5), "{rgb:?} gave {out:?}");
        }
    }

    /// A domain of 0..2 means the file's corners stand for 2: folded in, a colour of 0.5 finds the table's
    /// quarter, not its middle.
    #[test]
    fn a_domain_is_folded_into_the_table() {
        let text = "LUT_3D_SIZE 2\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 2 2 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
        let lut = ColorLut::from_cube(text).expect("a valid file");
        let out = lut.sample([1.0, 0.5, 0.0]);
        assert!((out[0] - 0.5).abs() < 1e-5 && (out[1] - 0.25).abs() < 1e-5, "{out:?}");
    }

    #[test]
    fn a_bad_file_says_what_is_wrong() {
        assert_eq!(ColorLut::from_cube("0 0 0\n"), Err(ColorLutError::Size));
        assert_eq!(ColorLut::from_cube("LUT_3D_SIZE 1\n"), Err(ColorLutError::Size));
        assert_eq!(ColorLut::from_cube("LUT_1D_SIZE 16\n"), Err(ColorLutError::OneDimensional));
        assert_eq!(ColorLut::from_cube("LUT_3D_SIZE 2\n0 0\n"), Err(ColorLutError::Line(2)));
        assert_eq!(ColorLut::from_cube("LUT_3D_SIZE 2\n0 0 0\n"), Err(ColorLutError::Entries { expected: 8, found: 1 }));
    }

    #[test]
    fn a_domain_reaches_the_shader_as_numbers() {
        assert_eq!(LutDomain::Grade.params(), [1.0, 0.0, 0.0]);
        assert_eq!(LutDomain::Reinhard.params(), [2.0, 0.0, 0.0]);
        assert_eq!(LutDomain::Log2 { min_ev: -12.5, max_ev: 12.5 }.params(), [3.0, -12.5, 12.5]);
    }
}
