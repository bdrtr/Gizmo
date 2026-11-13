//! Keeping [`Material`] and [`MaterialDesc`] in step — the pair that lets a scene file hold a
//! material at all.
//!
//! `Material` owns an `Arc<wgpu::BindGroup>`, so it cannot be written to a file; `MaterialDesc` is
//! everything else it is. Two passes, one each way:
//!
//! - [`sync_material_descriptions`] keeps a live material and its description saying the same
//!   thing — the description follows an edited material (so a save writes what is on screen), and
//!   the material follows an edited description (so the screen shows what the game wrote). Needs
//!   no GPU.
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
//! so a ten-thousand-entity scene pays roughly a megabyte for the ability to save it — two since
//! 2026-11-04, when a hidden copy of the last synced description joined it to tell which side a
//! game edited. The
//! alternative was to build descriptions only when a save is about to happen, which puts the
//! knowledge in every caller that saves (two of them today, and the day there are three is the day
//! one of them forgets). The per-frame pass costs a struct compare per material and writes
//! nothing when nothing changed.
//!
//! **The description is the authored thing, so removing a material means removing both.** Remove
//! only the `Material` and [`resolve_material_descriptions`] rebuilds it from the description on
//! the next frame. Nothing in the editor can do that today — the inspector has no delete button
//! for a material — but a game that removes one by hand should remove the description with it.

use gizmo_core::Entity;
use crate::components::{Material, MaterialDesc};
use gizmo_core::world::World;

/// The description as this pass last left it — what tells "the game edited the description" from
/// "the game edited the material", since after either edit the two simply differ.
#[derive(Clone)]
struct SyncedDesc(MaterialDesc);
impl gizmo_core::component::Component for SyncedDesc {}

/// What [`sync_material_descriptions`] does for one entity.
#[derive(Debug, PartialEq)]
enum Sync {
    /// Nothing changed.
    Nothing,
    /// The material is newer: write its description (and remember it).
    TakeMaterial(MaterialDesc),
    /// The description is newer: draw with it (and remember it).
    TakeDescription(MaterialDesc),
}

/// Decides one entity: `material` is the description of its live material, `desc` its stored
/// description, `synced` what this pass last left there.
///
/// A description that differs from the last synced one was edited by the game, and wins — even
/// if the material was edited in the same frame. Otherwise a material that differs from its
/// description was edited, and its description follows.
fn decide(material: &MaterialDesc, desc: Option<&MaterialDesc>, synced: Option<&MaterialDesc>) -> Sync {
    match (desc, synced) {
        (Some(d), Some(s)) if d != s => Sync::TakeDescription(d.clone()),
        (Some(d), _) if d == material => match synced {
            Some(s) if s == d => Sync::Nothing,
            _ => Sync::TakeMaterial(material.clone()),
        },
        _ => Sync::TakeMaterial(material.clone()),
    }
}

/// Keep every entity's [`Material`] and [`MaterialDesc`] saying the same thing, **in whichever
/// direction the game changed them**.
///
/// Runs per frame from both draw paths. An entity with a material and no description gets one
/// (which is what makes materials built in code — `Material::new`, the glTF loader, the editor's
/// ➕ menu — saveable without any of them knowing about descriptions). After that:
///
/// - a game that edits the **material** (`kale`, `ritim`) has its description follow, so a save
///   writes what is on screen;
/// - a game that edits the **description** (`kapi`'s plate lamps, `kopru`'s beam loads) has the
///   material follow, so the screen shows what it wrote. Until 2026-11-04 this direction did not
///   exist: the pass rebuilt the description from the material every frame, and the edit was
///   undone before it was ever drawn — two games' colour changes never showed, silently. A
///   description whose `texture_source` changed cannot be applied without the GPU, so its
///   material is removed and [`resolve_material_descriptions`] builds it again next frame.
///
/// Which side was edited is told by a hidden copy of the description as this pass last left it.
/// The cost is that copy per material and a comparison or two per frame.
pub fn sync_material_descriptions(world: &mut World) {
    let mut decided: Vec<(Entity, Sync)> = Vec::new();
    {
        let descs = world.query::<&MaterialDesc>();
        let synced = world.query::<&SyncedDesc>();
        for (entity, material) in world.query::<(Entity, &Material)>().iter() {
            let live = MaterialDesc::from(material);
            match decide(&live, descs.get(entity), synced.get(entity).map(|s| &s.0)) {
                Sync::Nothing => {}
                other => decided.push((entity, other)),
            }
        }
    }
    if decided.is_empty() {
        return;
    }

    let count = decided.len();
    for (e, what) in decided {
        match what {
            Sync::Nothing => {}
            Sync::TakeMaterial(desc) => {
                world.add_component(e, SyncedDesc(desc.clone()));
                world.add_component(e, desc);
            }
            Sync::TakeDescription(desc) => {
                let rebuilt = {
                    let mut materials = world.query_mut::<&mut Material>();
                    match materials.get_mut(e) {
                        Some(mut m) if m.texture_source == desc.texture_source => {
                            *m = desc.clone().into_material(m.bind_group.clone());
                            true
                        }
                        _ => false,
                    }
                };
                if !rebuilt {
                    // A new texture needs a new bind group, which needs the GPU: let the resolve
                    // pass build the material from the description.
                    world.remove_component::<Material>(e);
                }
                world.add_component(e, SyncedDesc(desc));
            }
        }
    }
    tracing::debug!(count, "[Material] material and description brought together");
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
        let materials = world.query::<&Material>();
        world.query::<(Entity, &MaterialDesc)>().iter()
            .filter(|(entity, _)| materials.get(*entity).is_none())
            .map(|(entity, desc)| (entity.id(), desc.clone()))
            .collect()
    };
    if pending.is_empty() {
        return;
    }

    let count = pending.len();
    for (entity, desc) in pending {
        // Every map the description names, not the base colour alone: a scene that saved a normal
        // map draws one when it opens. What will not load is warned about and neutral.
        let bind_group = renderer.material_bind_group(desc.texture_source.as_deref(), &desc.maps);
        if let Some(e) = world.get_entity(entity) {
            let bound = BoundSources { texture_source: desc.texture_source.clone(), maps: desc.maps.clone() };
            world.add_component(e, desc.into_material(bind_group));
            world.add_component(e, bound);
        }
    }
    tracing::debug!(count, "[Material] descriptions resolved into materials");
}

/// What a material's bind group was built from, as [`rebind_changed_materials`] last saw it.
#[derive(Clone, PartialEq)]
struct BoundSources {
    texture_source: Option<String>,
    maps: crate::components::MaterialMaps,
}
impl gizmo_core::component::Component for BoundSources {}

/// Builds a material's bind group again when what it is built from changes: its
/// [`texture_source`](Material::texture_source) or its [`maps`](Material::maps) — a texture dropped
/// onto it in the editor, a map a game sets, an undo. Each is loaded as
/// [`resolve_material_descriptions`] loads them, and shared.
///
/// A material seen for the first time is taken as it is: its bind group may hold what its fields do
/// not name (the glTF loader's maps, a builder's views, a texture still streaming in), and only a
/// change of a source is a reason to replace it.
pub fn rebind_changed_materials(world: &mut World, renderer: &crate::Renderer) {
    let (adopt, changed): (Vec<(Entity, BoundSources)>, Vec<(Entity, BoundSources)>) = {
        let bound = world.query::<&BoundSources>();
        let mut adopt = Vec::new();
        let mut changed = Vec::new();
        for (entity, material) in world.query::<(Entity, &Material)>().iter() {
            let now = BoundSources { texture_source: material.texture_source.clone(), maps: material.maps.clone() };
            match bound.get(entity) {
                None => adopt.push((entity, now)),
                Some(before) if *before != now => changed.push((entity, now)),
                Some(_) => {}
            }
        }
        (adopt, changed)
    };
    // A material seen for the first time that names no texture may still be drawn over one this
    // renderer loaded or generated — built as `Material::new(renderer.load_texture(path)?)`. Its
    // bind group says which; naming it here is what a save writes, so the material comes back
    // textured instead of white. The bind group stays as it is.
    let assets = renderer.asset_manager.read().unwrap_or_else(|p| p.into_inner());
    let adopt: Vec<(Entity, BoundSources)> = adopt
        .into_iter()
        .map(|(entity, mut sources)| {
            if sources.texture_source.is_none() && sources.maps.is_empty() {
                let named = world.query::<&Material>().get(entity).and_then(|m| assets.texture_source_of(&m.bind_group));
                if let Some(name) = named {
                    sources.texture_source = Some(name);
                }
            }
            (entity, sources)
        })
        .collect();
    drop(assets);
    for (entity, sources) in adopt {
        if let Some(name) = sources.texture_source.clone() {
            if let Some(mut material) = world.query_mut::<&mut Material>().get_mut(entity) {
                if material.texture_source.is_none() {
                    material.texture_source = Some(name);
                }
            }
        }
        world.add_component(entity, sources);
    }
    if changed.is_empty() {
        return;
    }
    let count = changed.len();
    for (entity, sources) in changed {
        let bind_group = renderer.material_bind_group(sources.texture_source.as_deref(), &sources.maps);
        if let Some(mut material) = world.query_mut::<&mut Material>().get_mut(entity) {
            material.bind_group = bind_group;
        }
        world.add_component(entity, sources);
    }
    tracing::debug!(count, "[Material] bind groups rebuilt for changed sources");
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
            clear_coat_roughness: 0.3,
            subsurface: 0.7,
            ambient: gizmo_math::Vec3::new(0.01, 0.02, 0.03),
            emissive: gizmo_math::Vec3::new(1.0, 2.0, 3.0),
            texture_source: Some("textures/brick.png".to_string()),
            maps: crate::components::MaterialMaps::default()
                .with_normal("textures/brick_n.png", 0.5)
                .with_metallic_roughness("textures/brick_mr.png")
                .with_emissive("textures/brick_e.png", gizmo_math::Vec3::new(2.0, 1.0, 0.5))
                .with_occlusion("textures/brick_ao.png", 0.75),
            material_type: MaterialType::BakedLit,
            is_transparent: true,
            is_double_sided: true,
            alpha_cutoff: 0.0,
            blend_mode: crate::components::BlendMode::Additive,
            reflectance: 0.16,
            transmission: 0.5,
            thickness: 0.25,
            attenuation_color: gizmo_math::Vec3::new(0.2, 0.6, 0.9),
            attenuation_distance: 0.75,
            specular_tint: gizmo_math::Vec3::new(0.9, 0.4, 0.1),
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
            clear_coat_roughness: 0.08,
            subsurface: 0.0,
            ambient: gizmo_math::Vec3::ZERO,
            emissive: gizmo_math::Vec3::ZERO,
            texture_source: None,
            maps: Default::default(),
            material_type: MaterialType::Pbr,
            is_transparent: false,
            is_double_sided: false,
            alpha_cutoff: 0.0,
            blend_mode: Default::default(),
            reflectance: 0.04,
            transmission: 0.0,
            thickness: 0.0,
            attenuation_color: gizmo_math::Vec3::ONE,
            attenuation_distance: 0.0,
            specular_tint: gizmo_math::Vec3::ONE,
        };
        let back: MaterialDesc =
            ron::from_str(&ron::to_string(&desc).expect("serialize")).expect("deserialize");
        assert_eq!(back.texture_source, None);
    }

    /// A scene saved before materials had maps opens with none: the field is `serde(default)`, and
    /// so is every field inside it, so a file that names only a normal map gets the other strengths
    /// as authored.
    #[test]
    fn a_description_saved_before_maps_reads_as_none() {
        let mut text = ron::to_string(&MaterialDesc::default()).expect("serialize");
        // Cut `maps:(…),` out, parentheses balanced: what a file written before the field looks like.
        let start = text.find("maps:").expect("maps is written");
        let mut depth = 0;
        let mut end = start;
        for (i, c) in text[start..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = start + i + 2; // the `)` and the `,` after it
                        break;
                    }
                }
                _ => {}
            }
        }
        text.replace_range(start..end, "");
        assert!(!text.contains("normal_scale"), "the cut left part of the maps: {text}");
        let old: MaterialDesc = ron::from_str(&text).unwrap_or_else(|e| panic!("{e}: {text}"));
        assert_eq!(old.maps, crate::components::MaterialMaps::default());

        let partial: crate::components::MaterialMaps = ron::from_str("(normal: Some(\"n.png\"))").expect("one field");
        assert_eq!(partial.normal.as_deref(), Some("n.png"));
        assert_eq!((partial.normal_scale, partial.occlusion_strength), (1.0, 1.0));
    }

    /// The emissive factor shines through the emissive map only: without one the slot is white, and
    /// a factor reaching the shader would light the whole surface.
    #[test]
    fn the_emissive_factor_needs_an_emissive_map() {
        let factor = gizmo_math::Vec3::new(1.0, 0.5, 0.25);
        let mut maps = crate::components::MaterialMaps { emissive_factor: factor, ..Default::default() };
        assert_eq!(maps.params().emissive_and_normal_scale[..3], [0.0; 3]);
        maps.emissive = Some("glow.png".to_string());
        assert_eq!(maps.params().emissive_and_normal_scale[..3], [1.0, 0.5, 0.25]);
        let neutral = crate::components::MaterialMaps::default().params();
        assert_eq!(bytemuck::bytes_of(&neutral), bytemuck::bytes_of(&crate::gpu_types::MaterialParams::default()));
    }
}

#[cfg(test)]
mod decide_tests {
    use super::{decide, Sync};
    use crate::components::MaterialDesc;

    fn red() -> MaterialDesc {
        MaterialDesc { albedo: gizmo_math::Vec4::new(1.0, 0.0, 0.0, 1.0), ..Default::default() }
    }
    fn green() -> MaterialDesc {
        MaterialDesc { albedo: gizmo_math::Vec4::new(0.0, 1.0, 0.0, 1.0), ..Default::default() }
    }

    #[test]
    fn a_new_material_gets_its_description() {
        assert_eq!(decide(&red(), None, None), Sync::TakeMaterial(red()));
    }

    #[test]
    fn nothing_changed_is_nothing() {
        assert_eq!(decide(&red(), Some(&red()), Some(&red())), Sync::Nothing);
    }

    #[test]
    fn an_edited_description_wins() {
        assert_eq!(decide(&red(), Some(&green()), Some(&red())), Sync::TakeDescription(green()));
    }

    #[test]
    fn an_edited_material_wins() {
        assert_eq!(decide(&green(), Some(&red()), Some(&red())), Sync::TakeMaterial(green()));
    }

    /// A description loaded with a scene and a material resolved from it agree but were never
    /// synced: remember them, nothing to apply.
    #[test]
    fn a_resolved_pair_is_remembered() {
        assert_eq!(decide(&red(), Some(&red()), None), Sync::TakeMaterial(red()));
    }
}

