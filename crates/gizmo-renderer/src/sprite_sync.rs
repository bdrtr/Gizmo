//! A `Sprite`'s and a `Tilemap`'s image, kept in step with its name so a scene can hold it.
//!
//! The same arrangement as [`crate::text_sync`] for a face: a [`SpriteImageId`] is an index into this
//! run's images and means nothing in a file; its [`ImageName`] — a path and how it is sampled —
//! does. [`sync_sprite_images`] keeps each sprite's and each map's name naming the image drawn, and
//! gives one read from a scene ([`SpriteImageId::UNRESOLVED`]) its image back by name. Whichever side
//! changed since it last looked is followed.

use gizmo_core::{Entity, World};

use crate::components::{Sprite, Tilemap};
use crate::sprite::{ImageName, SpriteImageId};

/// The image [`sync_sprite_images`] last left an entity's sprite with — and, apart, its tilemap's, so
/// an entity carrying both does not have one undo the other.
#[derive(Clone, Copy)]
struct SyncedImage(SpriteImageId);
impl gizmo_core::component::Component for SyncedImage {}
#[derive(Clone, Copy)]
struct SyncedTileset(SpriteImageId);
impl gizmo_core::component::Component for SyncedTileset {}

/// One image and its name brought in line; `synced` is what this pass last left, `None` the first
/// time.
fn sync_one(
    renderer: &mut crate::Renderer,
    image: SpriteImageId,
    name: Option<ImageName>,
    synced: Option<SpriteImageId>,
) -> (SpriteImageId, Option<ImageName>) {
    let unresolved = image == SpriteImageId::UNRESOLVED;
    let image_changed = synced.is_none_or(|s| s != image);
    let renamed = !image_changed && name.is_some() && renderer.sprite_image_name(image) != name.as_ref();
    if unresolved || renamed {
        let Some(n) = name.as_ref() else { return (image, name) };
        return match renderer.sprite_image_by_name(n) {
            Ok(id) => (id, name),
            Err(error) => {
                tracing::warn!(image = n.path, %error, "[Sprite] the image a scene names would not load; nothing is drawn until it does");
                (image, name)
            }
        };
    }
    if image_changed {
        return (image, renderer.sprite_image_name(image).cloned());
    }
    (image, name)
}

/// Keeps every sprite's and tilemap's image and its name in step — see the [module docs](self).
pub fn sync_sprite_images(world: &mut World, renderer: &mut crate::Renderer) {
    let sprites: Vec<(Entity, SpriteImageId, Option<ImageName>, Option<SpriteImageId>)> = {
        let synced = world.query::<&SyncedImage>();
        world
            .query::<(Entity, &Sprite)>()
            .iter()
            .map(|(e, s)| (e, s.image, s.image_name.clone(), synced.get(e).map(|s| s.0)))
            .collect()
    };
    for (entity, image, name, synced) in sprites {
        let (id, n) = sync_one(renderer, image, name.clone(), synced);
        if id != image || n != name {
            if let Some(mut s) = world.query_mut::<&mut Sprite>().get_mut(entity) {
                s.image = id;
                s.image_name = n;
            }
        }
        if synced != Some(id) {
            world.add_component(entity, SyncedImage(id));
        }
    }
    let maps: Vec<(Entity, SpriteImageId, Option<ImageName>, Option<SpriteImageId>)> = {
        let synced = world.query::<&SyncedTileset>();
        world
            .query::<(Entity, &Tilemap)>()
            .iter()
            .map(|(e, m)| (e, m.tileset, m.tileset_name.clone(), synced.get(e).map(|s| s.0)))
            .collect()
    };
    for (entity, image, name, synced) in maps {
        let (id, n) = sync_one(renderer, image, name.clone(), synced);
        if id != image || n != name {
            if let Some(mut m) = world.query_mut::<&mut Tilemap>().get_mut(entity) {
                m.tileset = id;
                m.tileset_name = n;
            }
        }
        if synced != Some(id) {
            world.add_component(entity, SyncedTileset(id));
        }
    }
}
