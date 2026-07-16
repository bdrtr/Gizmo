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

impl Default for Color {
    /// Varsayılan renk: opak beyaz (`Color::WHITE`).
    fn default() -> Self {
        Color::WHITE
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

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-6, "{a} != {b}");
    }

    #[test]
    fn rgb_sets_opaque_alpha() {
        let c = Color::rgb(0.1, 0.2, 0.3);
        assert_eq!(c.0, Vec4::new(0.1, 0.2, 0.3, 1.0));
    }

    #[test]
    fn rgba_passes_alpha_through() {
        let c = Color::rgba(0.1, 0.2, 0.3, 0.4);
        assert_eq!(c.0, Vec4::new(0.1, 0.2, 0.3, 0.4));
    }

    #[test]
    fn rgb8_maps_255_to_one_and_0_to_zero() {
        assert_eq!(Color::rgb8(255, 0, 128).0, Vec4::new(1.0, 0.0, 128.0 / 255.0, 1.0));
        // sınır: 0 ve 255 tam uçlar
        assert_eq!(Color::rgb8(0, 0, 0), Color::BLACK);
        assert_eq!(Color::rgb8(255, 255, 255), Color::WHITE);
    }

    #[test]
    fn hex_parses_with_and_without_hash() {
        let a = Color::hex("#FF5733");
        let b = Color::hex("FF5733");
        assert_eq!(a, b);
        approx(a.0.x, 1.0);
        approx(a.0.y, 0x57 as f32 / 255.0);
        approx(a.0.z, 0x33 as f32 / 255.0);
        approx(a.0.w, 1.0);
    }

    #[test]
    fn hex_parses_rgba_8_digits() {
        let c = Color::hex("#FF573380");
        approx(c.0.x, 1.0);
        approx(c.0.w, 0x80 as f32 / 255.0);
    }

    #[test]
    fn hex_too_short_falls_back_to_white() {
        assert_eq!(Color::hex("#abc"), Color::WHITE);
        assert_eq!(Color::hex(""), Color::WHITE);
        assert_eq!(Color::hex("#"), Color::WHITE);
    }

    #[test]
    fn hex_invalid_digits_default_to_full_channel() {
        // u8::from_str_radix hatası → unwrap_or(255) → 1.0 (panik YOK)
        let c = Color::hex("ZZZZZZ");
        assert_eq!(c, Color::WHITE);
    }

    #[test]
    fn to_hex_opaque_omits_alpha() {
        assert_eq!(Color::RED.to_hex(), "#FF0000");
        assert_eq!(Color::WHITE.to_hex(), "#FFFFFF");
        assert_eq!(Color::BLACK.to_hex(), "#000000");
    }

    #[test]
    fn to_hex_includes_alpha_when_translucent() {
        // 0.5 * 255 = 127.5 → round = 128 = 0x80
        assert_eq!(Color::rgba(1.0, 0.0, 0.0, 0.5).to_hex(), "#FF000080");
    }

    #[test]
    fn to_hex_clamps_out_of_range_channels() {
        // negatif/1'in üstü kanallar [0,1]'e kırpılır
        assert_eq!(Color::rgba(2.0, -1.0, 0.5, 1.0).to_hex(), "#FF0080");
    }

    #[test]
    fn hex_round_trips_within_one_lsb() {
        for c in [Color::RED, Color::ORANGE, Color::rgb8(17, 200, 99), Color::rgba(0.3, 0.6, 0.9, 0.4)] {
            let back = Color::hex(&c.to_hex());
            for (a, b) in [(c.0.x, back.0.x), (c.0.y, back.0.y), (c.0.z, back.0.z), (c.0.w, back.0.w)] {
                assert!((a - b).abs() <= 1.0 / 255.0 + 1e-6, "{a} vs {b}");
            }
        }
    }

    #[test]
    fn lerp_endpoints_and_midpoint() {
        assert_eq!(Color::BLACK.lerp(Color::WHITE, 0.0), Color::BLACK);
        assert_eq!(Color::BLACK.lerp(Color::WHITE, 1.0), Color::WHITE);
        let mid = Color::BLACK.lerp(Color::WHITE, 0.5);
        assert_eq!(mid.0, Vec4::new(0.5, 0.5, 0.5, 1.0));
    }

    #[test]
    fn lerp_clamps_t_outside_unit_range() {
        assert_eq!(Color::BLACK.lerp(Color::WHITE, -3.0), Color::BLACK);
        assert_eq!(Color::BLACK.lerp(Color::WHITE, 5.0), Color::WHITE);
    }

    #[test]
    fn with_alpha_only_touches_w() {
        let c = Color::RED.with_alpha(0.25);
        assert_eq!(c.0, Vec4::new(1.0, 0.0, 0.0, 0.25));
    }

    #[test]
    fn default_is_opaque_white() {
        assert_eq!(Color::default(), Color::WHITE);
    }

    #[test]
    fn vec4_conversions_round_trip() {
        let v = Vec4::new(0.2, 0.4, 0.6, 0.8);
        let c: Color = v.into();
        let back: Vec4 = c.into();
        assert_eq!(v, back);
        assert_eq!(c.to_vec4(), v);
    }
}
