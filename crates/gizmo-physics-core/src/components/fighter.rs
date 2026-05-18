use serde::{Deserialize, Serialize};
use gizmo_core::input::FighterInputBuffer;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FrameData {
    pub startup: u32,
    pub active: u32,
    pub recovery: u32,
    pub damage: f32,
    pub hitstun: u32,
    pub hitstop: u32,
}

impl Default for FrameData {
    fn default() -> Self {
        Self {
            startup: 10,
            active: 5,
            recovery: 15,
            damage: 10.0,
            hitstun: 20,
            hitstop: 5,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct CombatMove {
    pub name: String,
    pub frame_data: FrameData,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FighterController {
    pub player_id: u8,
    pub health: f32,
    pub max_health: f32,
    pub is_blocking: bool,
    pub is_crouching: bool,
    
    // Aktif saldırı durumu ve Frame Data takibi
    pub active_move: Option<CombatMove>,
    pub current_move_frame: u32,
    
    // Combo / Input handling
    #[serde(skip)]
    pub input_buffer: FighterInputBuffer,
    
    // Hitstop / Hitstun (Kare cinsinden bekleme süresi)
    #[serde(skip)]
    pub hitstop_frames: u32,
    #[serde(skip)]
    pub hitstun_frames: u32,

    pub walk_speed: f32,
    pub dash_speed: f32,
}

impl Default for FighterController {
    fn default() -> Self {
        Self {
            player_id: 1,
            health: 100.0,
            max_health: 100.0,
            is_blocking: false,
            is_crouching: false,
            active_move: None,
            current_move_frame: 0,
            input_buffer: FighterInputBuffer::new(60), // 1 saniyelik buffer (60fps)
            hitstop_frames: 0,
            hitstun_frames: 0,
            walk_speed: 3.0,
            dash_speed: 10.0,
        }
    }
}

impl FighterController {
    pub fn new(player_id: u8) -> Self {
        Self {
            player_id,
            ..Default::default()
        }
    }
    
    /// Karakter hasar yediğinde veya blokladığında hitstop (donma) uygula
    pub fn apply_hitstop(&mut self, frames: u32) {
        self.hitstop_frames = frames;
    }

    /// Sersemletme uygula
    pub fn apply_hitstun(&mut self, frames: u32) {
        self.hitstun_frames = frames;
        self.active_move = None;
        self.current_move_frame = 0;
        self.is_blocking = false;
    }

    /// Karakter şu an kilitli mi (animasyon donmuş veya sersemlemiş)
    pub fn is_locked(&self) -> bool {
        self.hitstop_frames > 0 || self.hitstun_frames > 0
    }

    /// Aktif saldırının 'Hasar Veren' (Active) kareleri içinde miyiz?
    pub fn is_in_active_window(&self) -> bool {
        if let Some(move_data) = &self.active_move {
            let fd = &move_data.frame_data;
            self.current_move_frame >= fd.startup && self.current_move_frame < (fd.startup + fd.active)
        } else {
            false
        }
    }
}

gizmo_core::impl_component!(FighterController);
