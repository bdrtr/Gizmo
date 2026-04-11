use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warning,
    Error,
}

pub struct LogEntry {
    pub message: String,
    pub level: LogLevel,
}

// Global logger static variable. Use Mutex for thread safety.
pub static GLOBAL_LOGS: Mutex<Vec<LogEntry>> = Mutex::new(Vec::new());

/// Arkada Mutex'i güvenlice açan ve log yazan yardımcı bir fonksiyon
pub fn log_message(level: LogLevel, msg: String) {
    if let Ok(mut logs) = GLOBAL_LOGS.lock() {
        // Spam önleyici: Aynı mesaj art arda geliyorsa vektörü ve konsolu şişirme (Frame bazlı spam koruması)
        if let Some(last) = logs.last() {
            if last.message == msg && last.level == level {
                return;
            }
        }

        logs.push(LogEntry {
            message: msg.clone(),
            level,
        });
        // Aynı zamanda konsola da yaz (Opsiyonel ama hata takibi için terminalde durması iyidir)
        match level {
            LogLevel::Info => println!("[INFO] {}", msg),
            LogLevel::Warning => println!("[WARN] {}", msg),
            LogLevel::Error => eprintln!("[ERROR] {}", msg),
        }
    }
}

/// Global Logger Makrosu
/// Örnek Kullanım:
/// gizmo_log!(Info, "Sistem başlatıldı: {}", sistem_adi);
#[macro_export]
macro_rules! gizmo_log {
    (Info, $($arg:tt)*) => {
        $crate::logger::log_message($crate::logger::LogLevel::Info, format!($($arg)*))
    };
    (Warning, $($arg:tt)*) => {
        $crate::logger::log_message($crate::logger::LogLevel::Warning, format!($($arg)*))
    };
    (Error, $($arg:tt)*) => {
        $crate::logger::log_message($crate::logger::LogLevel::Error, format!($($arg)*))
    };
}
