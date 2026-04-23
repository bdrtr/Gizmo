use gizmo_math::Vec4;

/// Bevy benzeri renk tipi. RGBA float değerleri (0.0 - 1.0) tutar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color(pub Vec4);

impl Color {
    // ─── Temel Renkler ────────────────────────────────────────────────────────
    pub const RED: Color = Color(Vec4::new(1.0, 0.0, 0.0, 1.0));
    pub const GREEN: Color = Color(Vec4::new(0.0, 1.0, 0.0, 1.0));
    pub const BLUE: Color = Color(Vec4::new(0.0, 0.0, 1.0, 1.0));
    pub const WHITE: Color = Color(Vec4::new(1.0, 1.0, 1.0, 1.0));
    pub const BLACK: Color = Color(Vec4::new(0.0, 0.0, 0.0, 1.0));
    pub const YELLOW: Color = Color(Vec4::new(1.0, 1.0, 0.0, 1.0));
    pub const CYAN: Color = Color(Vec4::new(0.0, 1.0, 1.0, 1.0));
    pub const MAGENTA: Color = Color(Vec4::new(1.0, 0.0, 1.0, 1.0));
    pub const ORANGE: Color = Color(Vec4::new(1.0, 0.5, 0.0, 1.0));
    pub const GRAY: Color = Color(Vec4::new(0.5, 0.5, 0.5, 1.0));
    pub const DARK_GRAY: Color = Color(Vec4::new(0.2, 0.2, 0.2, 1.0));
    pub const TRANSPARENT: Color = Color(Vec4::new(0.0, 0.0, 0.0, 0.0));

    // ─── Yapıcılar ────────────────────────────────────────────────────────────

    /// RGB float değerleriyle oluştur (0.0 - 1.0 arası).
    #[inline]
    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Color(Vec4::new(r, g, b, 1.0))
    }

    /// RGBA float değerleriyle oluştur (0.0 - 1.0 arası).
    #[inline]
    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Color(Vec4::new(r, g, b, a))
    }

    /// 0-255 arasında RGB tam sayı değerleriyle oluştur.
    #[inline]
    pub fn rgb8(r: u8, g: u8, b: u8) -> Self {
        Color(Vec4::new(
            r as f32 / 255.0,
            g as f32 / 255.0,
            b as f32 / 255.0,
            1.0,
        ))
    }

    /// Hex string ile oluştur. Örnek: `Color::hex("#FF5733")`, `Color::hex("FF5733")` veya RGBA `#FF5733AA`.
    pub fn hex(s: &str) -> Self {
        let s = s.trim_start_matches('#');
        let bytes = s.as_bytes();
        let parse = |i: usize| -> f32 {
            let slice = std::str::from_utf8(&bytes[i..i + 2]).unwrap_or("ff");
            u8::from_str_radix(slice, 16).unwrap_or(255) as f32 / 255.0
        };
        if bytes.len() >= 8 {
            Color(Vec4::new(parse(0), parse(2), parse(4), parse(6)))
        } else if bytes.len() >= 6 {
            Color(Vec4::new(parse(0), parse(2), parse(4), 1.0))
        } else {
            Color::WHITE
        }
    }

    /// Rengi Hex formatına dönüştürüp döndürür. Örnek: "#FF5733"
    pub fn to_hex(self) -> String {
        let r = (self.0.x.clamp(0.0, 1.0) * 255.0).round() as u8;
        let g = (self.0.y.clamp(0.0, 1.0) * 255.0).round() as u8;
        let b = (self.0.z.clamp(0.0, 1.0) * 255.0).round() as u8;

        if self.0.w < 0.999 {
            let a = (self.0.w.clamp(0.0, 1.0) * 255.0).round() as u8;
            format!("#{:02X}{:02X}{:02X}{:02X}", r, g, b, a)
        } else {
            format!("#{:02X}{:02X}{:02X}", r, g, b)
        }
    }

    /// İki renk arasında lineer interpolasyon (karıştırma) yapar.
    pub fn lerp(self, other: Color, t: f32) -> Color {
        let t = t.clamp(0.0, 1.0);
        Color(Vec4::new(
            self.0.x + (other.0.x - self.0.x) * t,
            self.0.y + (other.0.y - self.0.y) * t,
            self.0.z + (other.0.z - self.0.z) * t,
            self.0.w + (other.0.w - self.0.w) * t,
        ))
    }

    /// Alfa (şeffaflık) ile birleştir.
    #[inline]
    pub fn with_alpha(mut self, a: f32) -> Self {
        self.0.w = a;
        self
    }

    /// İç Vec4 değerini döndür.
    #[inline]
    pub fn to_vec4(self) -> Vec4 {
        self.0
    }
}

impl From<Color> for Vec4 {
    fn from(c: Color) -> Vec4 {
        c.0
    }
}

impl From<Vec4> for Color {
    fn from(v: Vec4) -> Color {
        Color(v)
    }
}
