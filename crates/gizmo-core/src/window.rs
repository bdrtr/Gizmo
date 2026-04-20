#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowInfo {
    pub width: f32,
    pub height: f32,
}

impl WindowInfo {
    pub fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    pub fn size(&self) -> (f32, f32) {
        (self.width, self.height)
    }

    pub fn aspect_ratio(&self) -> f32 {
        if self.height > 0.0 {
            self.width / self.height
        } else {
            1.0
        }
    }
}

impl Default for WindowInfo {
    fn default() -> Self {
        Self {
            width: 1280.0,
            height: 720.0,
        }
    }
}
