//! The mesh an entity draws, however it holds it: a `Mesh` component, or a `Handle<Mesh>` into
//! `Assets<Mesh>` — `MeshBundle`'s way, which the game's draw has always looked up. Everything in the
//! studio that asks "what does this entity look like" (picking, the selection box) asks here, so
//! none of it can be blind to one of the two again: until 2026-11-16 the viewport did not draw a
//! bundle's mesh, a click went through it and its selection box was missing.

use gizmo::core::asset::{Assets, Handle};
use gizmo::core::{Entity, Query, World};
use gizmo::renderer::components::Mesh;

/// Looks up the mesh each entity draws — see the [module docs](self).
pub struct DrawnMeshes<'w> {
    components: Query<'w, &'static Mesh>,
    handles: Query<'w, &'static Handle<Mesh>>,
    assets: Option<gizmo::core::world::ResourceReadGuard<'w, Assets<Mesh>>>,
}

impl<'w> DrawnMeshes<'w> {
    /// The lookup over `world`, holding a read of `Assets<Mesh>` (when there is one) while it lives.
    pub fn new(world: &'w World) -> Self {
        Self {
            components: world.query::<&Mesh>(),
            handles: world.query::<&Handle<Mesh>>(),
            assets: world.try_get_resource::<Assets<Mesh>>().ok(),
        }
    }

    /// The mesh `entity` draws: its own component first, as the game's draw takes it, then the asset
    /// its handle names.
    pub fn get(&self, entity: Entity) -> Option<&Mesh> {
        self.components
            .get(entity)
            .or_else(|| self.handles.get(entity).and_then(|h| self.assets.as_ref()?.get(h)))
    }
}
