use std::sync::Arc;

/// A texture projected onto whatever geometry lies inside the entity's volume — a bullet hole, a
/// puddle, a painted marking.
///
/// The entity's `Transform` is the projector box: the decal is applied where that box intersects
/// the depth buffer, so it wraps over whatever it lands on rather than being a piece of geometry
/// that has to match the surface.
#[derive(Clone)]
pub struct Decal {
    /// The decal's bind group: its uniform buffer, texture and sampler.
    pub bind_group: Arc<wgpu::BindGroup>,
    /// A tint multiplied into the projected texture; `a` scales its opacity.
    pub color: gizmo_math::Vec4,
}

impl Decal {
    /// A decal drawn with the given bind group and tint.
    pub fn new(bind_group: Arc<wgpu::BindGroup>, color: gizmo_math::Vec4) -> Self {
        Self { bind_group, color }
    }
}

/// Everything a [`Decal`] is but its bind group — what a scene saves one as, as
/// [`MaterialDesc`](crate::components::MaterialDesc) is a material: the texture by its name and the
/// tint. Kept by [`sync_decals`]: written from every live decal, and a decal built from every
/// description that has none (a scene just opened, ⏹).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DecalDesc {
    /// The texture's name — a file's path or UUID, or a generated texture's
    /// ([`AssetManager::texture_source_of`](crate::asset::AssetManager::texture_source_of)) — or `None`
    /// for one the renderer did not load, which comes back white.
    pub texture_source: Option<String>,
    /// See [`Decal::color`].
    pub color: gizmo_math::Vec4,
}

/// Keeps every [`Decal`] and its [`DecalDesc`] together: the description follows the live decal, and
/// a description with no decal (a scene just opened) gets one, its texture found by name.
pub fn sync_decals(world: &mut gizmo_core::World, renderer: &crate::Renderer) {
    use gizmo_core::Entity;
    let (missing, stale): (Vec<(Entity, DecalDesc)>, Vec<(Entity, DecalDesc)>) = {
        let decals = world.query::<&Decal>();
        let descs = world.query::<&DecalDesc>();
        let assets = renderer.asset_manager.read().unwrap_or_else(|p| p.into_inner());
        let missing = world
            .query::<(Entity, &DecalDesc)>()
            .iter()
            .filter(|(e, _)| decals.get(*e).is_none())
            .map(|(e, d)| (e, d.clone()))
            .collect();
        let stale = world
            .query::<(Entity, &Decal)>()
            .iter()
            .filter_map(|(e, decal)| {
                let before = descs.get(e);
                // The texture's name only changes with the bind group; keep the one a description
                // already holds when the bind group is not one this renderer can name.
                let texture_source =
                    assets.texture_source_of(&decal.bind_group).or_else(|| before.and_then(|d| d.texture_source.clone()));
                let now = DecalDesc { texture_source, color: decal.color };
                (before != Some(&now)).then_some((e, now))
            })
            .collect();
        (missing, stale)
    };
    for (entity, desc) in missing {
        let bind_group = match desc.texture_source.as_deref() {
            Some(name) => renderer.load_texture(name).unwrap_or_else(|error| {
                tracing::warn!(texture = name, %error, "[Decal] the texture a scene names would not load; drawing white");
                renderer.white_material_bind_group()
            }),
            None => renderer.white_material_bind_group(),
        };
        world.add_component(entity, Decal::new(bind_group, desc.color));
    }
    for (entity, desc) in stale {
        world.add_component(entity, desc);
    }
}
