
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Breakable {
    pub max_pieces: u32,
    pub threshold: f32, // required impulse/force to break
    pub is_broken: bool,
}

impl Default for Breakable {
    fn default() -> Self {
        Self {
            max_pieces: 10,
            threshold: 100.0,
            is_broken: false,
        }
    }
}


gizmo_core::impl_component!(Breakable);
