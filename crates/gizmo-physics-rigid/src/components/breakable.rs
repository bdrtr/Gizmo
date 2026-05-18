#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Breakable {
    pub max_pieces: u32,
    pub threshold: f32, // Minimum impulse to deal damage
    pub current_health: f32,
    pub max_health: f32,
    pub debris_lifetime: f32,
    pub break_sound: Option<String>,
    pub piece_prefab: Option<String>,
    #[serde(skip)]
    pub is_broken: bool,
}

impl Default for Breakable {
    fn default() -> Self {
        Self {
            max_pieces: 10,
            threshold: 100.0,
            current_health: 100.0,
            max_health: 100.0,
            debris_lifetime: 5.0,
            break_sound: None,
            piece_prefab: None,
            is_broken: false,
        }
    }
}

gizmo_core::impl_component!(Breakable);
