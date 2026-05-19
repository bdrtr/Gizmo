use super::World;
use crate::entity::Entity;

pub type DespawnHook = Box<dyn FnMut(&mut World, Entity) + Send + Sync>;

pub type AddHook = Box<dyn FnMut(&mut World, Entity) + Send + Sync>;
pub type RemoveHook = Box<dyn FnMut(&mut World, Entity) + Send + Sync>;
pub type SetHook = Box<dyn FnMut(&mut World, Entity) + Send + Sync>;

#[derive(Default)]
pub struct ComponentHooks {
    pub on_add: Vec<AddHook>,
    pub on_remove: Vec<RemoveHook>,
    pub on_set: Vec<SetHook>,
}
