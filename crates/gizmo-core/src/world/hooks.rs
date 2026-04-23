use super::World;
use crate::entity::Entity;

pub type DespawnHook = fn(&mut World, Entity);

pub type AddHook = fn(&mut World, Entity);
pub type RemoveHook = fn(&mut World, Entity);
pub type SetHook = fn(&mut World, Entity);

#[derive(Default, Clone)]
pub struct ComponentHooks {
    pub on_add: Vec<AddHook>,
    pub on_remove: Vec<RemoveHook>,
    pub on_set: Vec<SetHook>,
}
