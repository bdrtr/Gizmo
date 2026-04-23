#[derive(Clone)]
pub struct Sprite {
    pub width: f32,
    pub height: f32,
    pub uv_min: [f32; 2],
    pub uv_max: [f32; 2],
    pub layer: i32,
    pub flip_x: bool,
    pub flip_y: bool,
}

impl Sprite {
    pub fn new(width: f32, height: f32) -> Self {
        Self { width, height, uv_min: [0.0, 0.0], uv_max: [1.0, 1.0], layer: 0, flip_x: false, flip_y: false }
    }

    pub fn with_uv_region(mut self, min: [f32; 2], max: [f32; 2]) -> Self {
        self.uv_min = min; self.uv_max = max; self
    }

    pub fn with_layer(mut self, layer: i32) -> Self {
        self.layer = layer; self
    }
}
