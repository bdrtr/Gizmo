//! Keeping [`Material`] and [`MaterialDesc`] in step — the pair that lets a scene file hold a
//! material at all.
//!
//! `Material` owns an `Arc<wgpu::BindGroup>`, so it cannot be written to a file; `MaterialDesc` is
//! everything else it is. Two passes, one each way:
//!
//! - [`sync_material_descriptions`] writes a description for every live material, so a save has
//!   something to write. Needs no GPU.
//! - [`resolve_material_descriptions`] builds a material for every description that has none, so a
//!   load produces something to draw. Needs the renderer, because building a bind group is what it
//!   is for.
//!
//! **Why this exists at all.** Until 2026-08-19 `Material` was simply absent from the scene
//! registry, so authoring a material in the editor, saving and reopening lost every value the user
//! had set — silently, because an unregistered component is not written and nothing reports it.
//! The gap was recorded as a known exception with exactly this fix written into it: serialise the
//! description, rebuild the bind group on load.
//!
//! # Two consequences of the arrangement, both deliberate
//!
//! **Every entity with a material carries a description too.** That is one small struct per
//! entity — not per *material*, since materials are shared by `Arc` while the component is not —
//! so a ten-thousand-entity scene pays roughly a megabyte for the ability to save it. The
//! alternative was to build descriptions only when a save is about to happen, which puts the
//! knowledge in every caller that saves (two of them today, and the day there are three is the day
//! one of them forgets). The per-frame pass costs a struct compare per material and writes
//! nothing when nothing changed.
//!
//! **The description is the authored thing, so removing a material means removing both.** Remove
//! only the `Material` and [`resolve_material_descriptions`] rebuilds it from the description on
//! the next frame. Nothing in the editor can do that today — the inspector has no delete button
//! for a material — but a game that removes one by hand should remove the description with it.

use crate::components::{Material, MaterialDesc};
use gizmo_core::world::World;

/// Give every entity that has a [`Material`] an up-to-date [`MaterialDesc`], so a save can write
/// it.
///
/// Runs per frame from both draw paths. The cost is one comparison per material — the description
/// is built and compared against the stored one, and written only when they differ — so a scene
/// whose materials are not being edited pays a struct compare and nothing else.
///
/// A `Material` an entity never had a description for gets one on the first frame it is seen,
/// which is what makes materials built in code (`Material::new`, the glTF loader, the editor's
/// ➕ menu) saveable without any of them knowing about descriptions.
pub fn sync_material_descriptions(world: &mut World) {
    let mut wanted: Vec<(u32, MaterialDesc)> = Vec::new();
    {
        let materials = world.borrow::<Material>();
        let descs = world.borrow::<MaterialDesc>();
        for (entity, material) in materials.iter() {
            let desc = MaterialDesc::from(material);
            match descs.get(entity) {
                Some(existing) if *existing == desc => {}
                _ => wanted.push((entity, desc)),
            }
        }
    }
    if wanted.is_empty() {
        return;
    }

    let count = wanted.len();
    for (entity, desc) in wanted {
        if let Some(e) = world.get_entity(entity) {
            world.add_component(e, desc);
        }
    }
    tracing::debug!(count, "[Material] descriptions refreshed for saving");
}

/// Build a [`Material`] for every entity that carries a [`MaterialDesc`] and no material — which
/// is the state a freshly loaded scene is in.
///
/// The bind group comes from [`MaterialDesc::texture_source`] through the renderer's cached
/// texture loader, so two entities that named the same file share one upload. A description with
/// no texture, or one whose file will not load, gets the renderer's white 1×1 — the same bind
/// group an untextured material already draws with, and a warning rather than a missing object.
pub fn resolve_material_descriptions(world: &mut World, renderer: &crate::Renderer) {
    let pending: Vec<(u32, MaterialDesc)> = {
        let descs = world.borrow::<MaterialDesc>();
        let materials = world.borrow::<Material>();
        descs
            .iter()
            .filter(|(entity, _)| materials.get(*entity).is_none())
            .map(|(entity, desc)| (entity, desc.clone()))
            .collect()
    };
    if pending.is_empty() {
        return;
    }

    let count = pending.len();
    for (entity, desc) in pending {
        let bind_group = match desc.texture_source.as_deref() {
            Some(path) => match renderer.load_texture(path) {
                Ok(bg) => bg,
                Err(e) => {
                    tracing::warn!(
                        entity,
                        path,
                        error = ?e,
                        "[Material] texture named by a scene would not load; falling back to white"
                    );
                    renderer.white_material_bind_group()
                }
            },
            None => renderer.white_material_bind_group(),
        };
        if let Some(e) = world.get_entity(entity) {
            world.add_component(e, desc.into_material(bind_group));
        }
    }
    tracing::debug!(count, "[Material] descriptions resolved into materials");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::MaterialType;

    /// A description survives RON — the format scene files are written in — with every field.
    ///
    /// The other half of the guarantee is a compile error, not a test: `From<&Material>`
    /// destructures the material exhaustively, so a new field cannot be forgotten there. This
    /// covers the serde half, which no destructuring can.
    #[test]
    fn a_description_round_trips_through_ron() {
        let desc = MaterialDesc {
            albedo: gizmo_math::Vec4::new(0.1, 0.2, 0.3, 0.4),
            roughness: 0.25,
            metallic: 0.75,
            anisotropy: 0.5,
            clear_coat: 0.6,
            subsurface: 0.7,
            ambient: gizmo_math::Vec3::new(0.01, 0.02, 0.03),
            emissive: gizmo_math::Vec3::new(1.0, 2.0, 3.0),
            texture_source: Some("textures/brick.png".to_string()),
            material_type: MaterialType::BakedLit,
            is_transparent: true,
            is_double_sided: true,
            alpha_cutoff: 0.0,
        };

        let text = ron::to_string(&desc).expect("serialize");
        let back: MaterialDesc = ron::from_str(&text).expect("deserialize");
        assert_eq!(back, desc, "a field was lost or changed on the way through RON");
    }

    /// An untextured material is a legitimate description, and must not round-trip into `Some("")`
    /// or anything else that would send the resolve step looking for a file.
    #[test]
    fn an_untextured_description_stays_untextured() {
        let desc = MaterialDesc {
            albedo: gizmo_math::Vec4::ONE,
            roughness: 0.5,
            metallic: 0.0,
            anisotropy: 0.0,
            clear_coat: 0.0,
            subsurface: 0.0,
            ambient: gizmo_math::Vec3::ZERO,
            emissive: gizmo_math::Vec3::ZERO,
            texture_source: None,
            material_type: MaterialType::Pbr,
            is_transparent: false,
            is_double_sided: false,
            alpha_cutoff: 0.0,
        };
        let back: MaterialDesc =
            ron::from_str(&ron::to_string(&desc).expect("serialize")).expect("deserialize");
        assert_eq!(back.texture_source, None);
    }
}
