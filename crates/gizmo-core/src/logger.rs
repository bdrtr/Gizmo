use std::sync::Mutex;

/// Log seviyesi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LogLevel {
    Info,
    Warning,
    Error,
}

/// Tek bir log kaydı.
pub struct LogEntry {
    pub message: String,
    pub level: LogLevel,
    /// Kaynak dosya yolu (compile-time).
    pub file: &'static str,
    /// Kaynak satır numarası (compile-time).
    pub line: u32,
}

/// Maksimum log kapasitesi — ring buffer gibi davranır.
const MAX_LOG_ENTRIES: usize = 2048;

/// Minimum log seviyesi — bu seviyenin altındaki loglar kaydedilmez.
/// Release build'de Info loglarını bastırmak için bu değer değiştirilebilir.
static MIN_LOG_LEVEL: Mutex<LogLevel> = Mutex::new(LogLevel::Info);

// Global logger. Mutex poisoning durumunda into_inner() ile kurtarma yapılır.
static GLOBAL_LOGS: Mutex<Vec<LogEntry>> = Mutex::new(Vec::new());

/// Mutex lock'u güvenli şekilde alan yardımcı — poisoned olsa bile veriyi kurtarır.
fn lock_logs() -> std::sync::MutexGuard<'static, Vec<LogEntry>> {
    match GLOBAL_LOGS.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            // Bir thread panic yaptıysa bile log verisini kurtar
            eprintln!("[Logger] Mutex poisoned — veri kurtarılıyor");
            poisoned.into_inner()
        }
    }
}

fn lock_min_level() -> std::sync::MutexGuard<'static, LogLevel> {
    match MIN_LOG_LEVEL.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Log kaydı ekler. **Doğrudan çağırmayın** — `gizmo_log!` makrosunu kullanın.
#[doc(hidden)]
pub fn log_message(level: LogLevel, msg: String, file: &'static str, line: u32) {
    // Seviye filtresi
    let min_level = *lock_min_level();
    if (level as u8) < (min_level as u8) {
        return;
    }

    let mut logs = lock_logs();

    // Ring buffer: kapasiteyi aşarsa en eski log silinir
    if logs.len() >= MAX_LOG_ENTRIES {
        logs.remove(0);
    }

    logs.push(LogEntry {
        message: msg.clone(),
        level,
        file,
        line,
    });

    // Konsol çıktısı — Warning ve Error stderr'e gider
    match level {
        LogLevel::Info    => println!("[INFO]  {}:{} — {}", file, line, msg),
        LogLevel::Warning => eprintln!("[WARN]  {}:{} — {}", file, line, msg),
        LogLevel::Error   => eprintln!("[ERROR] {}:{} — {}", file, line, msg),
    }
}

// ──── Public API ────

/// Tüm logların snapshot'ını alır (okuma için).
/// Editor console gibi tüketiciler bu fonksiyonu kullanmalıdır.
pub fn get_logs<F, R>(f: F) -> R
where
    F: FnOnce(&[LogEntry]) -> R,
{
    let logs = lock_logs();
    f(&logs)
}

/// Tüm log kayıtlarını temizler.
pub fn clear_logs() {
    lock_logs().clear();
}

/// Tüm log kayıtlarını alır ve kuyruktan siler (drain).
pub fn drain_logs() -> Vec<LogEntry> {
    lock_logs().drain(..).collect()
}

/// Log entry sayısını döndürür.
pub fn log_count() -> usize {
    lock_logs().len()
}

/// Minimum log seviyesini ayarlar.
/// Bu seviyenin altındaki loglar kaydedilmez ve konsola yazılmaz.
pub fn set_min_log_level(level: LogLevel) {
    *lock_min_level() = level;
}

/// Global Logger Makrosu — kaynak konum bilgisi otomatik eklenir.
///
/// # Kullanım
/// ```rust,ignore
/// gizmo_log!(Info, "Sistem başlatıldı: {}", sistem_adi);
/// gizmo_log!(Warning, "FPS düşük: {:.1}", fps);
/// gizmo_log!(Error, "Dosya bulunamadı: {}", path);
/// ```
#[macro_export]
macro_rules! gizmo_log {
    (Info, $($arg:tt)*) => {
        $crate::logger::log_message(
            $crate::logger::LogLevel::Info,
            format!($($arg)*),
            file!(), line!()
        )
    };
    (Warning, $($arg:tt)*) => {
        $crate::logger::log_message(
            $crate::logger::LogLevel::Warning,
            format!($($arg)*),
            file!(), line!()
        )
    };
    (Error, $($arg:tt)*) => {
        $crate::logger::log_message(
            $crate::logger::LogLevel::Error,
            format!($($arg)*),
            file!(), line!()
        )
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Her test öncesi logları temizle
    fn setup() {
        clear_logs();
        set_min_log_level(LogLevel::Info);
    }

    #[test]
    fn test_log_and_read() {
        setup();
        log_message(LogLevel::Info, "test mesajı".into(), "test.rs", 1);

        get_logs(|logs| {
            assert_eq!(logs.len(), 1);
            assert_eq!(logs[0].message, "test mesajı");
            assert_eq!(logs[0].level, LogLevel::Info);
            assert_eq!(logs[0].file, "test.rs");
            assert_eq!(logs[0].line, 1);
        });
    }

    #[test]
    fn test_drain_clears() {
        setup();
        log_message(LogLevel::Warning, "w1".into(), "test.rs", 10);
        log_message(LogLevel::Error, "e1".into(), "test.rs", 20);

        let drained = drain_logs();
        assert_eq!(drained.len(), 2);
        assert_eq!(log_count(), 0);
    }

    #[test]
    fn test_clear_logs() {
        setup();
        log_message(LogLevel::Info, "clear me".into(), "test.rs", 1);
        assert_eq!(log_count(), 1);

        clear_logs();
        assert_eq!(log_count(), 0);
    }

    #[test]
    fn test_ring_buffer_capacity() {
        setup();
        // Kapasiteyi aşacak kadar log yaz
        for i in 0..MAX_LOG_ENTRIES + 500 {
            log_message(LogLevel::Info, format!("cap_test_{}", i), "test.rs", i as u32);
        }

        let count = log_count();
        // Paralel testler de log ekleyebilir, bu yüzden tam MAX_LOG_ENTRIES olmayabilir
        // ama asla aşmamalı
        assert!(
            count <= MAX_LOG_ENTRIES,
            "ring buffer kapasitesi aşıldı: {} > {}", count, MAX_LOG_ENTRIES
        );
    }

    #[test]
    fn test_min_level_filter() {
        setup();
        set_min_log_level(LogLevel::Warning);

        log_message(LogLevel::Info, "filtered".into(), "test.rs", 1);
        assert_eq!(log_count(), 0, "Info filtrelenmeli");

        log_message(LogLevel::Warning, "kept".into(), "test.rs", 2);
        assert_eq!(log_count(), 1, "Warning geçmeli");

        log_message(LogLevel::Error, "also kept".into(), "test.rs", 3);
        assert_eq!(log_count(), 2, "Error geçmeli");
    }

    #[test]
    fn test_log_level_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(LogLevel::Info);
        set.insert(LogLevel::Warning);
        set.insert(LogLevel::Error);
        assert_eq!(set.len(), 3);
    }
}
