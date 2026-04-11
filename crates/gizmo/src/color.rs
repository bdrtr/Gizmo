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

    /// Hex string ile oluştur. Örnek: `Color::hex("#FF5733")` veya `Color::hex("FF5733")`.
    pub fn hex(s: &str) -> Self {
        let s = s.trim_start_matches('#');
        let parse = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).unwrap_or(255) as f32 / 255.0;
        if s.len() >= 6 {
            Color(Vec4::new(parse(0), parse(2), parse(4), 1.0))
        } else {
            Color::WHITE
        }
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
