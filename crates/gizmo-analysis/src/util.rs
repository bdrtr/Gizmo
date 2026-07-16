//! Küçük paylaşılan yardımcılar — biçimlendirme + JSON/Chrome-trace serileştirme.
//! (report / panel / trace modülleri arasında tek kaynak olsun diye buradadır.)

use std::fmt::Write as _;

/// İnsan dostu bayt biçimi.
pub(crate) fn human_bytes(b: usize) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    let f = b as f64;
    if f >= MB {
        format!("{:.2} MB", f / MB)
    } else if f >= KB {
        format!("{:.1} KB", f / KB)
    } else {
        format!("{b} B")
    }
}

/// JSON string kaçışı (yalnız gerekli karakterler + kontrol karakterleri).
pub(crate) fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

/// Tek bir Chrome-trace "complete" (`ph:"X"`) olayını `out`'a yazar. `args` verilirse
/// ham JSON gövdesi olarak eklenir (ör. `"frame":3`). İsim ve kategori kaçışlanır.
pub(crate) fn write_trace_event(
    out: &mut String,
    name: &str,
    cat: &str,
    tid: u64,
    ts_us: f64,
    dur_us: f64,
    args: Option<&str>,
) {
    let _ = write!(
        out,
        "{{\"name\":\"{}\",\"cat\":\"{}\",\"ph\":\"X\",\"pid\":1,\"tid\":{},\"ts\":{:.3},\"dur\":{:.3}",
        json_escape(name),
        json_escape(cat),
        tid,
        ts_us,
        dur_us,
    );
    if let Some(a) = args {
        let _ = write!(out, ",\"args\":{{{a}}}");
    }
    out.push('}');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_bytes_selects_unit_at_boundaries() {
        // Sub-KB stays in raw bytes; exactly 1 KiB / 1 MiB flips the unit.
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1023), "1023 B");
        assert_eq!(human_bytes(1024), "1.0 KB");
        assert_eq!(human_bytes(1536), "1.5 KB"); // 1.5 KiB
        assert_eq!(human_bytes(1024 * 1024), "1.00 MB");
        assert_eq!(human_bytes(5 * 1024 * 1024 + 512 * 1024), "5.50 MB");
    }

    #[test]
    fn human_bytes_just_below_mb_stays_in_kb() {
        // One byte below a MiB must still render as KB, not MB.
        assert_eq!(human_bytes(1024 * 1024 - 1), "1024.0 KB");
    }

    #[test]
    fn json_escape_handles_all_special_and_control_chars() {
        assert_eq!(json_escape("plain"), "plain");
        assert_eq!(json_escape("a\"b"), "a\\\"b");
        assert_eq!(json_escape("a\\b"), "a\\\\b");
        assert_eq!(json_escape("l1\nl2"), "l1\\nl2");
        assert_eq!(json_escape("a\rb"), "a\\rb");
        assert_eq!(json_escape("a\tb"), "a\\tb");
        // Control char below 0x20 with no dedicated escape → \u00xx.
        assert_eq!(json_escape("\u{0001}"), "\\u0001");
        assert_eq!(json_escape("\u{001f}"), "\\u001f");
        // Non-ASCII printable passes through untouched (not escaped).
        assert_eq!(json_escape("ü→π"), "ü→π");
        assert_eq!(json_escape(""), "");
    }

    #[test]
    fn write_trace_event_without_args_is_exact() {
        let mut out = String::new();
        write_trace_event(&mut out, "sim", "gizmo", 7, 12.5, 3.25, None);
        assert_eq!(
            out,
            "{\"name\":\"sim\",\"cat\":\"gizmo\",\"ph\":\"X\",\"pid\":1,\"tid\":7,\"ts\":12.500,\"dur\":3.250}"
        );
    }

    #[test]
    fn write_trace_event_with_args_appends_raw_object() {
        let mut out = String::new();
        write_trace_event(&mut out, "sim", "gizmo", 1, 0.0, 1.0, Some("\"frame\":3"));
        assert!(
            out.ends_with(",\"args\":{\"frame\":3}}"),
            "args body must be wrapped in braces and closed: {out}"
        );
    }

    #[test]
    fn write_trace_event_escapes_name_and_category() {
        let mut out = String::new();
        write_trace_event(&mut out, "a\"b", "c\\d", 3, 0.0, 0.0, None);
        assert!(out.contains("\"name\":\"a\\\"b\""), "name must be escaped: {out}");
        assert!(out.contains("\"cat\":\"c\\\\d\""), "cat must be escaped: {out}");
    }

    #[test]
    fn write_trace_event_appends_without_clobbering() {
        // The helper must append; a pre-existing prefix survives.
        let mut out = String::from("PREFIX");
        write_trace_event(&mut out, "x", "y", 1, 0.0, 0.0, None);
        assert!(out.starts_with("PREFIX{\"name\":\"x\""));
    }
}
