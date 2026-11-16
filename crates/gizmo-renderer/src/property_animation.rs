//! The engine's animatable properties: which fields of its lights, materials, cameras, sprites,
//! texts and post-processing a keyed property track may name ([`engine_properties`]), and the
//! world's registry with them in it ([`animatable_properties`]).
//!
//! **What is registered is what the draw reads again every frame, on both draw paths** (the
//! game's `gizmo::systems::render` and the studio's): a light's colour and intensity, a
//! material's per-instance factors, a camera's projection. A keyed write goes through the
//! component's `Mut`, only when the value moves, before the render world copies the game world,
//! so the copy has it that frame.
//!
//! Deliberately **not** registered — each is a cache or a description that a write to the live
//! value would not reach, or would make leak:
//! - the `MaterialMaps` floats (`normal_scale`, `emissive_factor`, the UV transform…): the
//!   material's GPU parameters are built once per distinct maps value and kept in a cache that
//!   never evicts (`asset::AssetManager::load_material`, keyed by `{maps:?}`), so an animated
//!   value would leak one buffer per frame it took. Writing that buffer in place is separate work.
//! - `MaterialDesc`: it is the saved description; `material_sync` writes it from the live
//!   `Material` every frame, so a track on it would fight that sync and lose.
//! - `SpotLight::cookie`, `Camera::clear_color`, `Camera::viewport`, `Text::max_width`: not
//!   numbers a key can hold (a name, an `Option`).
//! - A screen sprite's or text's position and the bokeh's rotation: each a field of one variant of
//!   an enum (`SpriteSpace::Screen`, `TextSpace::Screen`, `DofBlur::Bokeh`), which a name cannot
//!   address without saying what happens when the variant is another.
//!
//! Two that are registered and cost something: `Text.size_px` lays the text out again at each new
//! size (the layout cache is bounded) and rasterises its glyphs once per whole pixel size into the
//! glyph atlas, which never evicts — a size swept from 16 to 64 px stores 49 copies of each glyph.

use gizmo_animation::property::{PropertyError, PropertyRegistry};
use gizmo_core::world::ResourceWriteGuard;
use gizmo_core::World;
use gizmo_math::{Vec2, Vec3, Vec4};

use crate::components::{Camera, DirectionalLight, Material, PointLight, PostProcess, RectLight, Sprite, SpotLight, Text};

/// Registers the engine's animatable fields into `reg` — see the [module docs](self) for what is
/// in and what is not, and why. Registering them again is harmless. Returns what could not be
/// registered: a [`PropertyError::NameTaken`] for each engine name a game registered for a field
/// of its own first — that name alone; every other is registered.
pub fn engine_properties(reg: &mut PropertyRegistry) -> Vec<PropertyError> {
    // Each on its own: one name a game took first costs that name, not every one after it.
    let mut taken = Vec::new();
    // ── Material: the per-instance factors both batchers upload every frame (`InstanceRaw::new`
    // + `with_alpha_cutoff`/`with_reflectance`/`with_lit_extras_of`).
    taken.extend(reg.register_lanes::<Material, Vec4>("Material.albedo", |m| m.albedo, |m, v| m.albedo = v).err());
    taken.extend(reg.register::<Material, Vec3>("Material.emissive", |m| m.emissive, |m, v| m.emissive = v).err());
    taken.extend(reg.register::<Material, Vec3>("Material.ambient", |m| m.ambient, |m, v| m.ambient = v).err());
    taken.extend(reg.register::<Material, f32>("Material.roughness", |m| m.roughness, |m, v| m.roughness = v).err());
    taken.extend(reg.register::<Material, f32>("Material.metallic", |m| m.metallic, |m, v| m.metallic = v).err());
    taken.extend(reg.register::<Material, f32>("Material.anisotropy", |m| m.anisotropy, |m, v| m.anisotropy = v).err());
    taken.extend(reg.register::<Material, f32>("Material.clear_coat", |m| m.clear_coat, |m, v| m.clear_coat = v).err());
    taken.extend(reg.register::<Material, f32>("Material.clear_coat_roughness", |m| m.clear_coat_roughness, |m, v| m.clear_coat_roughness = v).err());
    taken.extend(reg.register::<Material, f32>("Material.subsurface", |m| m.subsurface, |m, v| m.subsurface = v).err());
    taken.extend(reg.register::<Material, f32>("Material.alpha_cutoff", |m| m.alpha_cutoff, |m, v| m.alpha_cutoff = v).err());
    taken.extend(reg.register::<Material, f32>("Material.reflectance", |m| m.reflectance, |m, v| m.reflectance = v).err());
    taken.extend(reg.register::<Material, f32>("Material.transmission", |m| m.transmission, |m, v| m.transmission = v).err());
    taken.extend(reg.register::<Material, f32>("Material.thickness", |m| m.thickness, |m, v| m.thickness = v).err());
    taken.extend(reg.register::<Material, Vec3>("Material.attenuation_color", |m| m.attenuation_color, |m, v| m.attenuation_color = v).err());
    taken.extend(reg.register::<Material, f32>("Material.attenuation_distance", |m| m.attenuation_distance, |m, v| m.attenuation_distance = v).err());
    taken.extend(reg.register::<Material, Vec3>("Material.specular_tint", |m| m.specular_tint, |m, v| m.specular_tint = v).err());

    // ── Lights: gathered from the world every frame.
    taken.extend(reg.register::<PointLight, Vec3>("PointLight.color", |l| l.color, |l, v| l.color = v).err());
    taken.extend(reg.register::<PointLight, f32>("PointLight.intensity", |l| l.intensity, |l, v| l.intensity = v).err());
    taken.extend(reg.register::<PointLight, f32>("PointLight.radius", |l| l.radius, |l, v| l.radius = v).err());
    taken.extend(reg.register::<PointLight, f32>("PointLight.source_radius", |l| l.source_radius, |l, v| l.source_radius = v).err());
    taken.extend(reg.register::<PointLight, f32>("PointLight.source_length", |l| l.source_length, |l, v| l.source_length = v).err());
    taken.extend(reg.register::<PointLight, f32>("PointLight.contact_shadow_length", |l| l.contact_shadow_length, |l, v| l.contact_shadow_length = v).err());

    taken.extend(reg.register::<SpotLight, Vec3>("SpotLight.color", |l| l.color, |l, v| l.color = v).err());
    taken.extend(reg.register::<SpotLight, f32>("SpotLight.intensity", |l| l.intensity, |l, v| l.intensity = v).err());
    taken.extend(reg.register::<SpotLight, f32>("SpotLight.radius", |l| l.radius, |l, v| l.radius = v).err());
    taken.extend(reg.register::<SpotLight, f32>("SpotLight.inner_angle", |l| l.inner_angle, |l, v| l.inner_angle = v).err());
    taken.extend(reg.register::<SpotLight, f32>("SpotLight.outer_angle", |l| l.outer_angle, |l, v| l.outer_angle = v).err());
    taken.extend(reg.register::<SpotLight, f32>("SpotLight.source_radius", |l| l.source_radius, |l, v| l.source_radius = v).err());
    taken.extend(reg.register::<SpotLight, f32>("SpotLight.contact_shadow_length", |l| l.contact_shadow_length, |l, v| l.contact_shadow_length = v).err());

    taken.extend(reg.register::<RectLight, Vec3>("RectLight.color", |l| l.color, |l, v| l.color = v).err());
    taken.extend(reg.register::<RectLight, f32>("RectLight.intensity", |l| l.intensity, |l, v| l.intensity = v).err());
    taken.extend(reg.register::<RectLight, f32>("RectLight.radius", |l| l.radius, |l, v| l.radius = v).err());
    taken.extend(reg.register::<RectLight, f32>("RectLight.width", |l| l.width, |l, v| l.width = v).err());
    taken.extend(reg.register::<RectLight, f32>("RectLight.height", |l| l.height, |l, v| l.height = v).err());
    taken.extend(reg.register::<RectLight, f32>("RectLight.contact_shadow_length", |l| l.contact_shadow_length, |l, v| l.contact_shadow_length = v).err());

    taken.extend(reg.register::<DirectionalLight, Vec3>("DirectionalLight.color", |l| l.color, |l, v| l.color = v).err());
    taken.extend(reg.register::<DirectionalLight, f32>("DirectionalLight.intensity", |l| l.intensity, |l, v| l.intensity = v).err());
    taken.extend(reg.register::<DirectionalLight, f32>("DirectionalLight.angular_radius", |l| l.angular_radius, |l, v| l.angular_radius = v).err());
    taken.extend(reg.register::<DirectionalLight, f32>("DirectionalLight.contact_shadow_length", |l| l.contact_shadow_length, |l, v| l.contact_shadow_length = v).err());

    // ── Camera: the projection and exposure are built from these every frame.
    taken.extend(reg.register::<Camera, f32>("Camera.fov", |c| c.fov, |c, v| c.fov = v).err());
    taken.extend(reg.register::<Camera, f32>("Camera.near", |c| c.near, |c, v| c.near = v).err());
    taken.extend(reg.register::<Camera, f32>("Camera.far", |c| c.far, |c, v| c.far = v).err());
    taken.extend(reg.register::<Camera, f32>("Camera.exposure", |c| c.exposure, |c, v| c.exposure = v).err());

    // ── Sprites and texts: drawn from the component every frame. The image and the face are
    // untouched by these — a typed field write, not a serialised round trip that would reset the
    // sprite's resolved image to "not found yet".
    taken.extend(reg.register_lanes::<Sprite, Vec4>("Sprite.color", |s| s.color, |s, v| s.color = v).err());
    taken.extend(reg.register::<Sprite, Vec2>("Sprite.size", |s| s.size, |s, v| s.size = v).err());
    taken.extend(reg.register::<Sprite, Vec2>("Sprite.anchor", |s| s.anchor, |s, v| s.anchor = v).err());
    taken.extend(reg.register_lanes::<Sprite, Vec4>("Sprite.uv_rect", |s| Vec4::from_array(s.uv_rect), |s, v| s.uv_rect = v.to_array()).err());
    taken.extend(reg.register_lanes::<Text, Vec4>("Text.color", |t| t.color, |t, v| t.color = v).err());
    taken.extend(reg.register::<Text, f32>("Text.size_px", |t| t.size_px, |t, v| t.size_px = v).err());

    // ── Post-processing: its uniforms are written from the component every frame.
    taken.extend(reg.register::<PostProcess, f32>("PostProcess.bloom_intensity", |p| p.bloom_intensity, |p, v| p.bloom_intensity = v).err());
    taken.extend(reg.register::<PostProcess, f32>("PostProcess.bloom_threshold", |p| p.bloom_threshold, |p, v| p.bloom_threshold = v).err());
    taken.extend(reg.register::<PostProcess, f32>("PostProcess.vignette", |p| p.vignette, |p, v| p.vignette = v).err());
    taken.extend(reg.register::<PostProcess, f32>("PostProcess.chromatic_aberration", |p| p.chromatic_aberration, |p, v| p.chromatic_aberration = v).err());
    taken.extend(reg.register::<PostProcess, f32>("PostProcess.dof_focus_dist", |p| p.dof_focus_dist, |p, v| p.dof_focus_dist = v).err());
    taken.extend(reg.register::<PostProcess, f32>("PostProcess.dof_focus_range", |p| p.dof_focus_range, |p, v| p.dof_focus_range = v).err());
    taken.extend(reg.register::<PostProcess, f32>("PostProcess.dof_blur_size", |p| p.dof_blur_size, |p, v| p.dof_blur_size = v).err());
    taken.extend(reg.register::<PostProcess, f32>("PostProcess.dof_f_stops", |p| p.dof_f_stops, |p, v| p.dof_f_stops = v).err());
    taken.extend(reg.register::<PostProcess, f32>("PostProcess.dof_sensor_height", |p| p.dof_sensor_height, |p, v| p.dof_sensor_height = v).err());
    taken.extend(reg.register::<PostProcess, f32>("PostProcess.dof_max_depth", |p| p.dof_max_depth, |p, v| p.dof_max_depth = v).err());
    taken.extend(reg.register::<PostProcess, f32>("PostProcess.film_grain", |p| p.film_grain, |p, v| p.film_grain = v).err());
    taken.extend(reg.register::<PostProcess, f32>("PostProcess.motion_blur", |p| p.motion_blur, |p, v| p.motion_blur = v).err());
    taken
}

/// The world's [`PropertyRegistry`], inserted if there is none, with the engine's fields in it
/// ([`engine_properties`]) — registered the first time this sees the registry, so a game that
/// inserted an empty registry, or registered its own fields before the first frame, still gets
/// them. The engine's frame calls it before the keyed driver runs; a game calls it to add its own.
///
/// A game that took one of the engine's names for a field of its own keeps it; the engine's field
/// of that name is then not animatable, and this warns once.
pub fn animatable_properties(world: &mut World) -> ResourceWriteGuard<'_, PropertyRegistry> {
    let mut reg = world.get_resource_mut_or_default::<PropertyRegistry>();
    if !reg.engine_seeded() {
        reg.set_engine_seeded(true);
        for e in engine_properties(&mut reg) {
            tracing::warn!("[Animation] an engine property is not animatable: {e}");
        }
    }
    reg
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_animation::clip::{InterpolatedValue, PropertyKind};
    use std::sync::Arc;

    fn material() -> Material {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::NOOP,
            backend_options: wgpu::BackendOptions {
                noop: wgpu::NoopBackendOptions { enable: true, ..Default::default() },
                ..Default::default()
            },
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("the noop adapter");
        let (device, _queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("a noop device");
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &[] });
        Material::new(Arc::new(device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &layout, entries: &[] })))
    }

    fn registry() -> PropertyRegistry {
        let mut r = PropertyRegistry::default();
        assert!(engine_properties(&mut r).is_empty(), "the engine's names are its own");
        r
    }

    /// A value of `kind` that no default holds, different for each name.
    fn probe(kind: PropertyKind, seed: f32) -> InterpolatedValue {
        let s = 0.123 + seed * 0.01;
        match kind {
            PropertyKind::Scalar => InterpolatedValue::Scalar(s),
            PropertyKind::Vector2 => InterpolatedValue::Vector2(Vec2::new(s, s + 0.5)),
            PropertyKind::Vector3 => InterpolatedValue::Vector3(Vec3::new(s, s + 0.25, s + 0.5)),
            PropertyKind::Vector4 => InterpolatedValue::Vector4(Vec4::new(s, s + 0.25, s + 0.5, s + 0.75)),
            _ => unreachable!("every kind is covered"),
        }
    }

    /// The serialised form of `component` on `e` — `Material` through `MaterialDesc`, its saved
    /// description field for field.
    fn serialised(world: &World, e: gizmo_core::Entity, component: &str) -> serde_json::Value {
        fn json<T: serde::Serialize + gizmo_core::Component>(world: &World, e: gizmo_core::Entity) -> serde_json::Value {
            serde_json::to_value(world.get::<T>(e).expect("has it")).unwrap()
        }
        match component {
            "Material" => serde_json::to_value(crate::components::MaterialDesc::from(world.get::<Material>(e).unwrap())).unwrap(),
            "PointLight" => json::<PointLight>(world, e),
            "SpotLight" => json::<SpotLight>(world, e),
            "RectLight" => json::<RectLight>(world, e),
            "DirectionalLight" => json::<DirectionalLight>(world, e),
            "Camera" => json::<Camera>(world, e),
            "Sprite" => json::<Sprite>(world, e),
            "Text" => json::<Text>(world, e),
            "PostProcess" => json::<PostProcess>(world, e),
            other => panic!("no serialised form for {other}"),
        }
    }

    /// The numbers at `name`'s path in the component's serialised form: `"Material.albedo.w"` is
    /// `albedo[3]` of the `Material`.
    fn at_path(world: &World, e: gizmo_core::Entity, name: &str) -> Vec<f32> {
        let mut parts = name.split('.');
        let component = parts.next().unwrap();
        let mut v = serialised(world, e, component);
        for part in parts {
            v = match (part, &v) {
                ("x" | "y" | "z" | "w", serde_json::Value::Array(a)) => a[["x", "y", "z", "w"].iter().position(|l| *l == part).unwrap()].clone(),
                (field, serde_json::Value::Object(m)) => m.get(field).unwrap_or_else(|| panic!("{name}: no field {field}")).clone(),
                _ => panic!("{name}: cannot follow {part} into {v}"),
            };
        }
        match v {
            serde_json::Value::Number(n) => vec![n.as_f64().unwrap() as f32],
            serde_json::Value::Array(a) => a.iter().map(|x| x.as_f64().unwrap() as f32).collect(),
            other => panic!("{name}: not a number: {other}"),
        }
    }

    fn lanes(v: InterpolatedValue) -> Vec<f32> {
        match v {
            InterpolatedValue::Scalar(s) => vec![s],
            InterpolatedValue::Vector2(v) => v.to_array().to_vec(),
            InterpolatedValue::Vector3(v) => v.to_array().to_vec(),
            InterpolatedValue::Vector4(v) => v.to_array().to_vec(),
            other => panic!("not a value: {other:?}"),
        }
    }

    /// Every entry writes the live field its name says: driven through the registry onto one
    /// entity carrying every component, each value is read back out of the component's serialised
    /// form at the path the name spells — not through the entry's own getter, which an entry wired
    /// to the wrong field would agree with.
    #[test]
    fn engine_properties_write_live_fields() {
        let r = registry();
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, material());
        world.add_component(e, PointLight::new(Vec3::ONE, 1.0, 5.0));
        world.add_component(e, SpotLight::new(Vec3::ONE, 1.0, 5.0, 0.3, 0.5));
        world.add_component(e, RectLight::new(Vec3::ONE, 1.0, 5.0, 1.0, 1.0));
        world.add_component(e, DirectionalLight::new(Vec3::ONE, 1.0, crate::components::LightRole::Sun));
        world.add_component(e, Camera::new(1.0, 0.1, 100.0, true));
        world.add_component(e, Sprite::world(crate::sprite::SpriteImageId::placeholder(0), Vec2::ONE));
        world.add_component(e, Text::screen("hi", crate::text::FontId::UNRESOLVED, 16.5, Vec2::ZERO));
        world.add_component(e, PostProcess::default());
        let names: Vec<String> = r.names().map(str::to_owned).collect();
        assert_eq!(r.properties_of(&world, e).count(), names.len(), "the entity has every component");
        for (i, name) in names.iter().enumerate() {
            let kind = r.kind(name).unwrap();
            let v = probe(kind, i as f32);
            assert_eq!(r.set(&mut world, e, name, v), Ok(true), "{name}");
            assert_eq!(r.get(&world, e, name), Some(v), "{name} reads back");
            assert_eq!(at_path(&world, e, name), lanes(v), "{name} wrote the field its name spells");
        }
    }

    /// The reason the registry writes typed fields rather than round-tripping a component through
    /// its serialised form: a sprite's image id is `#[serde(skip)]`, and a round trip would hand
    /// back "not resolved yet" — a sprite that vanished the frame a colour track touched it.
    #[test]
    fn sprite_color_track_keeps_image() {
        let r = registry();
        let mut world = World::new();
        let e = world.spawn();
        let image = crate::sprite::SpriteImageId::placeholder(3);
        world.add_component(e, Sprite::world(image, Vec2::ONE));
        assert_eq!(r.set(&mut world, e, "Sprite.color.w", InterpolatedValue::Scalar(0.25)), Ok(true));
        let s = world.get::<Sprite>(e).unwrap();
        assert_eq!(s.color.w, 0.25);
        assert_eq!(s.image, image);
        assert_ne!(s.image, crate::components::sprite::unresolved_image());
    }

    /// Fields that are numbers but deliberately not animatable, each with why. A new numeric field
    /// on one of these components fails `every_numeric_field_is_registered_or_excluded` until
    /// someone decides which list it belongs on.
    const EXCLUDED: &[(&str, &str)] = &[
        ("Material.maps", "MaterialMaps: the GPU parameters are cached per distinct maps value and never evicted"),
        ("Camera.clear_color", "an Option: a key cannot say 'none'"),
        ("Camera.viewport", "an Option of a rectangle: a key cannot say 'none'"),
        ("Text.max_width", "an Option: a key cannot say 'none'"),
        ("Text.space", "a field of one variant of an enum (a screen text's position)"),
        ("Sprite.space", "a field of one variant of an enum (a screen sprite's position)"),
        ("PostProcess.dof_blur", "a field of one variant of an enum (the bokeh's rotation)"),
    ];

    /// Every numeric leaf of the serialised form — an `f32`, or an array of 2–4 numbers — as its
    /// dotted path.
    fn numeric_leaves(prefix: &str, v: &serde_json::Value, out: &mut Vec<String>) {
        match v {
            serde_json::Value::Number(n) if n.is_f64() => out.push(prefix.to_owned()),
            serde_json::Value::Array(a) if (2..=4).contains(&a.len()) && a.iter().all(serde_json::Value::is_number) => {
                out.push(prefix.to_owned())
            }
            serde_json::Value::Object(m) => {
                for (k, v) in m {
                    numeric_leaves(&format!("{prefix}.{k}"), v, out);
                }
            }
            _ => {}
        }
    }

    /// Completeness, grafted from the reflective design that was rejected: whatever numbers these
    /// components serialise, each is either an entry or on [`EXCLUDED`] with a reason. `Material`
    /// has no serialised form; `MaterialDesc`, its saved description field for field, stands in.
    ///
    /// The values are chosen so every `Option` is `Some` and every number is a float, so nothing
    /// hides behind a `null` or an integer-looking zero.
    #[test]
    fn every_numeric_field_is_registered_or_excluded() {
        let r = registry();
        let sprite = Sprite::screen(crate::sprite::SpriteImageId::placeholder(0), Vec2::ONE, Vec2::splat(3.5));
        let mut camera = Camera::new(1.1, 0.1, 100.0, true);
        camera.clear_color = Some(Vec3::splat(0.5));
        camera.viewport = Some(crate::components::Viewport::default());
        let mut text = Text::screen("hi", crate::text::FontId::UNRESOLVED, 16.5, Vec2::ZERO);
        text.max_width = Some(100.5);
        let samples: Vec<(&str, serde_json::Value)> = vec![
            ("PointLight", serde_json::to_value(PointLight::new(Vec3::splat(0.5), 1.5, 5.5)).unwrap()),
            ("DirectionalLight", serde_json::to_value(DirectionalLight::new(Vec3::splat(0.5), 1.5, crate::components::LightRole::Sun)).unwrap()),
            ("SpotLight", serde_json::to_value(SpotLight::new(Vec3::splat(0.5), 1.5, 5.5, 0.3, 0.5)).unwrap()),
            ("RectLight", serde_json::to_value(RectLight::new(Vec3::splat(0.5), 1.5, 5.5, 1.5, 1.5)).unwrap()),
            ("Camera", serde_json::to_value(camera).unwrap()),
            ("PostProcess", serde_json::to_value(PostProcess::default()).unwrap()),
            ("Sprite", serde_json::to_value(sprite).unwrap()),
            ("Text", serde_json::to_value(text).unwrap()),
            ("Material", serde_json::to_value(crate::components::MaterialDesc::default()).unwrap()),
        ];
        let excluded = |path: &str| EXCLUDED.iter().any(|(e, _)| path == *e || path.starts_with(&format!("{e}.")));
        let mut missing = Vec::new();
        let mut all = Vec::new();
        for (component, value) in &samples {
            let mut leaves = Vec::new();
            numeric_leaves(component, value, &mut leaves);
            assert!(!leaves.is_empty(), "{component} serialised no number at all: {value}");
            for leaf in leaves {
                if r.kind(&leaf).is_none() && !excluded(&leaf) {
                    missing.push(leaf.clone());
                }
                all.push(leaf);
            }
        }
        assert!(missing.is_empty(), "numeric fields neither registered nor excluded: {missing:?}");
        // An exclusion that matches no number has gone stale (the field was renamed or removed).
        for (e, _) in EXCLUDED {
            assert!(all.iter().any(|l| l == e || l.starts_with(&format!("{e}."))), "stale exclusion {e}");
        }
        // And every entry is a real field: its component is one of these, as the scene names it.
        for name in r.names() {
            let component = name.split('.').next().unwrap();
            assert!(samples.iter().any(|(c, _)| *c == component), "{name}");
        }
    }

    /// A game that took one of the engine's names first costs the engine that name alone: every
    /// other engine property — those registered after it included — is still there.
    #[test]
    fn a_name_a_game_took_costs_only_that_name() {
        #[derive(Clone)]
        struct Lens(f32);
        gizmo_core::impl_component!(Lens);

        let mut world = World::new();
        let mut game = PropertyRegistry::default();
        game.register::<Lens, f32>("Material.albedo.x", |l| l.0, |l, v| l.0 = v).unwrap();
        world.insert_resource(game);
        let reg = animatable_properties(&mut world);
        let full = registry();
        let missing: Vec<&str> = full.names().filter(|n| reg.kind(n).is_none()).collect();
        assert!(missing.is_empty(), "only the taken name may be missing, yet {missing:?} are");
        assert_eq!(reg.len(), full.len(), "the game's entry replaced the engine's, nothing more");
        let mut again = PropertyRegistry::default();
        again.register::<Lens, f32>("Material.albedo.x", |l| l.0, |l, v| l.0 = v).unwrap();
        let errors = engine_properties(&mut again);
        assert_eq!(errors.len(), 1, "one collision, one error: {errors:?}");
        assert!(matches!(&errors[0], PropertyError::NameTaken { name, .. } if name == "Material.albedo.x"));
    }

    #[test]
    fn animatable_properties_seeds_once_even_if_game_inserted_first() {
        #[derive(Clone)]
        struct Health(f32);
        gizmo_core::impl_component!(Health);

        let mut world = World::new();
        let mut game = PropertyRegistry::default();
        game.register::<Health, f32>("Health.value", |h| h.0, |h, v| h.0 = v).unwrap();
        world.insert_resource(game);
        let count = {
            let reg = animatable_properties(&mut world);
            assert!(reg.engine_seeded());
            assert_eq!(reg.kind("Health.value"), Some(PropertyKind::Scalar), "the game's entry stays");
            assert_eq!(reg.kind("PointLight.intensity"), Some(PropertyKind::Scalar), "the engine's were added");
            reg.len()
        };
        assert_eq!(animatable_properties(&mut world).len(), count, "a second call adds nothing");

        // With no registry at all, one is made.
        let mut fresh = World::new();
        assert!(animatable_properties(&mut fresh).kind("Material.albedo.w").is_some());
    }
}
