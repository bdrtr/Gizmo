use crate::commands::{CommandQueue, ScriptCommand};
use gizmo_core::input::Input;
use gizmo_core::World;
use std::sync::Arc;
use std::sync::Mutex;

pub struct ScriptEngine {
    command_queue: Arc<CommandQueue>,
    pub log_queue: Arc<Mutex<Vec<(String, String)>>>,
}

unsafe impl Send for ScriptEngine {}
unsafe impl Sync for ScriptEngine {}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Script {
    pub file_path: String,
    #[serde(default, skip)]
    pub initialized: bool,
}

impl Script {
    pub fn new(path: &str) -> Self {
        Self {
            file_path: path.to_string(),
            initialized: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ScriptContext {
    pub entity_id: u32,
    pub dt: f32,
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub key_w: bool,
    pub key_a: bool,
    pub key_s: bool,
    pub key_d: bool,
    pub key_space: bool,
    pub key_up: bool,
    pub key_down: bool,
    pub key_left: bool,
    pub key_right: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ScriptResult {
    pub new_position: Option<[f32; 3]>,
    pub new_velocity: Option<[f32; 3]>,
}

impl ScriptEngine {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            command_queue: Arc::new(CommandQueue::new()),
            log_queue: Arc::new(Mutex::new(Vec::new())),
        })
    }
    pub fn load_script(&mut self, _path: &str) -> Result<(), String> {
        Ok(())
    }
    pub fn update(&mut self, _world: &World, _input: &Input, _dt: f32) -> Result<(), String> {
        Ok(())
    }
    pub fn update_entity(
        &mut self,
        _entity_id: u32,
        _script_path: &str,
        _dt: f32,
    ) -> Result<(), String> {
        Ok(())
    }
    pub fn flush_commands(&self, _world: &mut World, _dt: f32) -> Vec<ScriptCommand> {
        Vec::new()
    }
    pub fn get_pending_audio_scene_commands(&self) -> Vec<ScriptCommand> {
        Vec::new()
    }
    pub fn reload_if_changed(&mut self, _path: &str) -> Result<bool, String> {
        Ok(false)
    }
    pub fn has_function(&self, _path: &str, _name: &str) -> bool {
        false
    }
    pub fn run_entity_update(
        &self,
        _path: &str,
        _func_name: &str,
        _ctx: &ScriptContext,
    ) -> Result<ScriptResult, String> {
        Ok(ScriptResult::default())
    }
    pub fn command_queue(&self) -> &Arc<CommandQueue> {
        &self.command_queue
    }
}

gizmo_core::impl_component!(Script);
