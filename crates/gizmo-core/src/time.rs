#[derive(Debug, Clone, Copy)]
pub struct Time {
    pub dt: f32,
    pub elapsed_seconds: f64,
}

impl Default for Time {
    fn default() -> Self {
        Self {
            dt: 0.0,
            elapsed_seconds: 0.0,
        }
    }
}
