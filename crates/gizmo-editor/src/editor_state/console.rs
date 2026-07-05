//! EditorState — console logging helpers.
use super::*;

impl EditorState {
    pub fn log_info(&mut self, msg: &str) {
        gizmo_core::logger::log_message(
            gizmo_core::logger::LogLevel::Info,
            msg.to_string(),
            file!(),
            line!(),
        );
    }

    pub fn log_warning(&mut self, msg: &str) {
        gizmo_core::logger::log_message(
            gizmo_core::logger::LogLevel::Warning,
            msg.to_string(),
            file!(),
            line!(),
        );
    }

    pub fn log_error(&mut self, msg: &str) {
        gizmo_core::logger::log_message(
            gizmo_core::logger::LogLevel::Error,
            msg.to_string(),
            file!(),
            line!(),
        );
        self.last_error = Some(msg.to_string());
    }
}
