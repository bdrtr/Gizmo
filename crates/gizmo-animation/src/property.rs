//! Which component fields a keyed track may animate, by name: the [`PropertyRegistry`].
//!
//! A property track ([`Track::property`](crate::clip::Track::property)) names its field as
//! `"<Component>.<field>"` — `"PointLight.intensity"`, `"Material.albedo"` — plus, for a vector
//! registered with lanes, one lane of it: `"Sprite.color.w"` fades a sprite's alpha alone. The
//! component is named as the scene registry names it, so a clip reads the way a scene file does.
//! Game fields share that name space: a game registers `"Health.value"` the way the engine
//! registers its own, and a name registered twice for different fields is refused
//! ([`PropertyError::NameTaken`]).
//!
//! Each entry is a typed getter and setter (`fn(&C) -> V`, `fn(&mut C, V)`), monomorphised at
//! registration: no reflection, no serialisation and no `unsafe` on the write path. A write goes
//! through the component's `Mut` and only when the value differs, so a field held at one value
//! does not stamp `Changed<C>` every frame — the render extract copies what really moved.
//!
//! The registry is a world resource. The keyed driver
//! ([`keyed_animation_system`](crate::system::keyed_animation_system)) reads it once a frame;
//! `gizmo-renderer` seeds the engine's fields into it (`property_animation::engine_properties`).

use std::collections::{HashMap, HashSet};

use gizmo_core::component::Component;
use gizmo_core::component_key::ComponentKey;
use gizmo_core::entity::Entity;
use gizmo_core::World;
use gizmo_math::{Vec2, Vec3, Vec4};

use crate::clip::{InterpolatedValue, PropertyKind};

/// A value a property track can key: `f32`, `Vec2`, `Vec3` or `Vec4`.
pub trait PropertyValue: Copy + PartialEq + Send + Sync + 'static {
    /// The kind of keyframes that animate it.
    const KIND: PropertyKind;
    /// The value as a sample.
    fn into_value(self) -> InterpolatedValue;
    /// The value a sample holds, `None` when the sample is of another kind.
    fn from_value(value: InterpolatedValue) -> Option<Self>;
}

/// A [`PropertyValue`] made of `f32` lanes, which [`PropertyRegistry::register_lanes`] can also
/// expose one at a time.
pub trait PropertyVector: PropertyValue {
    /// How many lanes it has.
    const LANES: usize;
    /// Lane `i` (`0` is x).
    fn lane(self, i: usize) -> f32;
    /// The value with lane `i` replaced by `v`.
    fn with_lane(self, i: usize, v: f32) -> Self;
}

/// `value` with lane `i` replaced by `v`; `None` when `value` is not a vector of that many lanes.
pub(crate) fn with_lane(value: InterpolatedValue, i: usize, v: f32) -> Option<InterpolatedValue> {
    fn set<V: PropertyVector>(x: V, i: usize, v: f32) -> Option<InterpolatedValue> {
        (i < V::LANES).then(|| x.with_lane(i, v).into_value())
    }
    match value {
        InterpolatedValue::Vector2(x) => set(x, i, v),
        InterpolatedValue::Vector3(x) => set(x, i, v),
        InterpolatedValue::Vector4(x) => set(x, i, v),
        _ => None,
    }
}

impl PropertyValue for f32 {
    const KIND: PropertyKind = PropertyKind::Scalar;
    fn into_value(self) -> InterpolatedValue {
        InterpolatedValue::Scalar(self)
    }
    fn from_value(value: InterpolatedValue) -> Option<Self> {
        match value {
            InterpolatedValue::Scalar(v) => Some(v),
            _ => None,
        }
    }
}

macro_rules! vector_value {
    ($ty:ty, $variant:ident, $lanes:expr) => {
        impl PropertyValue for $ty {
            const KIND: PropertyKind = PropertyKind::$variant;
            fn into_value(self) -> InterpolatedValue {
                InterpolatedValue::$variant(self)
            }
            fn from_value(value: InterpolatedValue) -> Option<Self> {
                match value {
                    InterpolatedValue::$variant(v) => Some(v),
                    _ => None,
                }
            }
        }
        impl PropertyVector for $ty {
            const LANES: usize = $lanes;
            fn lane(self, i: usize) -> f32 {
                self[i]
            }
            fn with_lane(mut self, i: usize, v: f32) -> Self {
                self[i] = v;
                self
            }
        }
    };
}
vector_value!(Vec2, Vector2, 2);
vector_value!(Vec3, Vector3, 3);
vector_value!(Vec4, Vector4, 4);

/// What [`PropertyRegistry`] refused, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PropertyError {
    /// No property of that name is registered.
    Unknown(String),
    /// The name is already registered for another component or another kind of value.
    NameTaken {
        /// The name.
        name: String,
        /// The component (its Rust type name) that already holds it.
        component: String,
    },
    /// A value of the wrong kind for the property.
    KindMismatch {
        /// The property.
        name: String,
        /// The kind it is registered as.
        expected: PropertyKind,
        /// The kind it was given, `None` for a transform sample or no value.
        got: Option<PropertyKind>,
    },
    /// The entity is dead, or has no component of the property's type.
    MissingComponent(String),
}

impl std::fmt::Display for PropertyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PropertyError::Unknown(name) => write!(f, "no animatable property is registered as `{name}`"),
            PropertyError::NameTaken { name, component } => {
                write!(f, "property `{name}` is already registered, for `{component}`")
            }
            PropertyError::KindMismatch { name, expected, got: Some(got) } => {
                write!(f, "property `{name}` is a {expected}, not a {got}")
            }
            PropertyError::KindMismatch { name, expected, got: None } => {
                write!(f, "property `{name}` is a {expected}, and was given no property value")
            }
            PropertyError::MissingComponent(name) => {
                write!(f, "the entity has no component for property `{name}`")
            }
        }
    }
}

impl std::error::Error for PropertyError {}

type GetFn = Box<dyn Fn(&World, Entity) -> Option<InterpolatedValue> + Send + Sync>;
type ApplyFn = Box<dyn Fn(&mut World, &[(Entity, InterpolatedValue)]) -> usize + Send + Sync>;

/// One registered property.
struct Entry {
    name: String,
    component: ComponentKey,
    component_name: &'static str,
    kind: PropertyKind,
    /// For a lane entry (`"Sprite.color.w"`): its whole vector's entry and the lane. The driver
    /// folds lane and whole-vector samples of one field into one write of the whole vector, so
    /// they compose in player order and the field is written — and stamped — once.
    lane: Option<(usize, usize)>,
    get: GetFn,
    /// Writes each `(entity, value)` whose entity has the component and whose value is of the
    /// entry's kind and differs from the field's; how many it wrote.
    apply: ApplyFn,
}

/// The component fields keyed tracks may animate, by name — a world resource. See the
/// [module docs](self) for the naming contract.
///
/// ```
/// # use gizmo_animation::property::PropertyRegistry;
/// # use gizmo_animation::clip::InterpolatedValue;
/// # use gizmo_core::World;
/// #[derive(Clone)]
/// struct Health { value: f32 }
/// gizmo_core::impl_component!(Health);
///
/// let mut registry = PropertyRegistry::default();
/// registry.register::<Health, f32>("Health.value", |h| h.value, |h, v| h.value = v).unwrap();
///
/// let mut world = World::new();
/// let e = world.spawn();
/// world.add_component(e, Health { value: 10.0 });
/// assert_eq!(registry.set(&mut world, e, "Health.value", InterpolatedValue::Scalar(3.0)), Ok(true));
/// assert_eq!(world.get::<Health>(e).map(|h| h.value), Some(3.0));
/// ```
#[derive(Default)]
pub struct PropertyRegistry {
    by_name: HashMap<String, usize>,
    entries: Vec<Entry>,
    warned: HashSet<String>,
    pub(crate) engine_seeded: bool,
}

impl std::fmt::Debug for PropertyRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PropertyRegistry")
            .field("properties", &self.entries.len())
            .field("engine_seeded", &self.engine_seeded)
            .finish()
    }
}

impl PropertyRegistry {
    /// Registers `name` as `C`'s field read by `get` and written by `set`.
    ///
    /// Registering the same name again for the same component and kind does nothing and
    /// succeeds — the engine's set and a game's setup may both run — and the first accessors stay.
    ///
    /// # Errors
    ///
    /// [`PropertyError::NameTaken`] when `name` is registered for another component or kind.
    pub fn register<C: Component, V: PropertyValue>(
        &mut self,
        name: &str,
        get: fn(&C) -> V,
        set: fn(&mut C, V),
    ) -> Result<(), PropertyError> {
        self.insert::<C, V>(name, get, set)
    }

    /// Registers `name` as a vector field of `C` (as [`register`](Self::register) does), and
    /// each of its lanes as a scalar: `name.x`, `name.y`, then `name.z` and `name.w` as the
    /// vector has them. A lane track writes its lane and leaves the others as they are — keying
    /// `"Sprite.color.w"` fades the alpha alone.
    ///
    /// # Errors
    ///
    /// [`PropertyError::NameTaken`] when `name` or one of its lane names is registered for
    /// another component or kind. A taken whole name registers nothing; a taken lane name costs
    /// that lane alone — the other lanes are registered, and the first collision is returned.
    pub fn register_lanes<C: Component, V: PropertyVector>(
        &mut self,
        name: &str,
        get: fn(&C) -> V,
        set: fn(&mut C, V),
    ) -> Result<(), PropertyError> {
        self.insert::<C, V>(name, get, set)?;
        let whole = self.by_name[name];
        // Each lane on its own: a lane name taken by another component costs that lane alone.
        let mut first_error = None;
        for (i, lane) in ["x", "y", "z", "w"].into_iter().enumerate().take(V::LANES) {
            let lane_name = format!("{name}.{lane}");
            match self.insert::<C, f32>(&lane_name, move |c| get(c).lane(i), move |c, v| set(c, get(c).with_lane(i, v))) {
                Ok(()) => {
                    let index = self.by_name[&lane_name];
                    self.entries[index].lane = Some((whole, i));
                }
                Err(e) => {
                    first_error.get_or_insert(e);
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    fn insert<C: Component, V: PropertyValue>(
        &mut self,
        name: &str,
        get: impl Fn(&C) -> V + Copy + Send + Sync + 'static,
        set: impl Fn(&mut C, V) + Copy + Send + Sync + 'static,
    ) -> Result<(), PropertyError> {
        let component = ComponentKey::of::<C>();
        if let Some(&i) = self.by_name.get(name) {
            let e = &self.entries[i];
            return if e.component == component && e.kind == V::KIND {
                Ok(())
            } else {
                Err(PropertyError::NameTaken { name: name.to_owned(), component: e.component_name.to_owned() })
            };
        }
        let entry = Entry {
            name: name.to_owned(),
            component,
            component_name: std::any::type_name::<C>(),
            kind: V::KIND,
            lane: None,
            get: Box::new(move |world, e| world.get::<C>(e).map(|c| get(c).into_value())),
            apply: Box::new(move |world, items| {
                // One query for the whole batch; a field is written — and `Changed<C>` stamped —
                // only when its value really moves.
                let mut q = world.query_mut::<&mut C>();
                let mut written = 0;
                for &(e, value) in items {
                    let Some(v) = V::from_value(value) else { continue };
                    if let Some(mut c) = q.get_mut(e) {
                        if get(&c) != v {
                            set(&mut c, v);
                            written += 1;
                        }
                    }
                }
                written
            }),
        };
        self.by_name.insert(name.to_owned(), self.entries.len());
        self.entries.push(entry);
        Ok(())
    }

    /// The kind of value `name` holds, `None` when it is not registered.
    #[must_use]
    pub fn kind(&self, name: &str) -> Option<PropertyKind> {
        self.by_name.get(name).map(|&i| self.entries[i].kind)
    }

    /// Every registered name, in registration order.
    pub fn names(&self) -> impl Iterator<Item = &str> + '_ {
        self.entries.iter().map(|e| e.name.as_str())
    }

    /// How many properties are registered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether none is.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// `name`'s current value on `entity`; `None` when the name is unknown or the entity has no
    /// such component.
    #[must_use]
    pub fn get(&self, world: &World, entity: Entity, name: &str) -> Option<InterpolatedValue> {
        let &i = self.by_name.get(name)?;
        (self.entries[i].get)(world, entity)
    }

    /// Writes `value` to `name` on `entity`; whether the field changed (an equal value is not
    /// written, and stamps no change).
    ///
    /// # Errors
    ///
    /// [`PropertyError::Unknown`], [`PropertyError::KindMismatch`], or
    /// [`PropertyError::MissingComponent`] when the entity is dead or lacks the component.
    pub fn set(&self, world: &mut World, entity: Entity, name: &str, value: InterpolatedValue) -> Result<bool, PropertyError> {
        let &i = self.by_name.get(name).ok_or_else(|| PropertyError::Unknown(name.to_owned()))?;
        let entry = &self.entries[i];
        if value.kind() != Some(entry.kind) {
            return Err(PropertyError::KindMismatch { name: name.to_owned(), expected: entry.kind, got: value.kind() });
        }
        if world.get_component_ptr(entity, entry.component).is_none() {
            return Err(PropertyError::MissingComponent(name.to_owned()));
        }
        Ok((entry.apply)(world, &[(entity, value)]) > 0)
    }

    /// Every property of a component `entity` has, as `(name, kind)`, in registration order —
    /// what an editor offers to key.
    pub fn properties_of<'a>(&'a self, world: &'a World, entity: Entity) -> impl Iterator<Item = (&'a str, PropertyKind)> + 'a {
        self.entries
            .iter()
            .filter(move |e| world.get_component_ptr(entity, e.component).is_some())
            .map(|e| (e.name.as_str(), e.kind))
    }

    /// Logs `message` as a warning the first time `key` is seen, and never again for that key;
    /// whether it logged. What the keyed driver uses so a broken track warns once, not every frame.
    pub fn warn_once(&mut self, key: &str, message: impl std::fmt::Display) -> bool {
        if self.warned.contains(key) {
            return false;
        }
        self.warned.insert(key.to_owned());
        tracing::warn!(key, "[Animation] {message}");
        true
    }

    /// Whether the engine's own properties have been registered into this registry.
    #[must_use]
    pub fn engine_seeded(&self) -> bool {
        self.engine_seeded
    }

    /// Marks the engine's own properties as registered — for the crate that seeds them
    /// (`gizmo-renderer`'s `property_animation::animatable_properties`).
    pub fn set_engine_seeded(&mut self, seeded: bool) {
        self.engine_seeded = seeded;
    }

    /// The entry index and kind of `name`, for the driver.
    pub(crate) fn lookup(&self, name: &str) -> Option<(usize, PropertyKind)> {
        self.by_name.get(name).map(|&i| (i, self.entries[i].kind))
    }

    /// Entry `index`'s field: the whole vector's entry and the lane for a lane entry, itself and
    /// `None` otherwise.
    pub(crate) fn field_of(&self, index: usize) -> (usize, Option<usize>) {
        self.entries[index].lane.map_or((index, None), |(whole, lane)| (whole, Some(lane)))
    }

    /// Entry `index`'s current value on `entity`.
    pub(crate) fn get_at(&self, index: usize, world: &World, entity: Entity) -> Option<InterpolatedValue> {
        self.entries.get(index).and_then(|e| (e.get)(world, entity))
    }

    /// Applies a batch of samples to entry `index`; how many fields it wrote.
    pub(crate) fn apply_at(&self, index: usize, world: &mut World, items: &[(Entity, InterpolatedValue)]) -> usize {
        self.entries.get(index).map_or(0, |e| (e.apply)(world, items))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_core::query::Changed;

    #[derive(Clone, Debug, PartialEq)]
    struct Glow {
        power: f32,
        tint: Vec4,
    }
    gizmo_core::impl_component!(Glow);

    #[derive(Clone)]
    struct Other(f32);
    gizmo_core::impl_component!(Other);

    fn registry() -> PropertyRegistry {
        let mut r = PropertyRegistry::default();
        r.register::<Glow, f32>("Glow.power", |g| g.power, |g, v| g.power = v).unwrap();
        r.register_lanes::<Glow, Vec4>("Glow.tint", |g| g.tint, |g, v| g.tint = v).unwrap();
        r
    }

    fn world_with_glow() -> (World, Entity) {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Glow { power: 1.0, tint: Vec4::new(0.1, 0.2, 0.3, 0.4) });
        (world, e)
    }

    #[test]
    fn register_get_set_roundtrip() {
        let r = registry();
        let (mut world, e) = world_with_glow();
        assert_eq!(r.kind("Glow.power"), Some(PropertyKind::Scalar));
        assert_eq!(r.kind("Glow.tint"), Some(PropertyKind::Vector4));
        assert_eq!(r.get(&world, e, "Glow.power"), Some(InterpolatedValue::Scalar(1.0)));
        assert_eq!(r.set(&mut world, e, "Glow.power", InterpolatedValue::Scalar(7.5)), Ok(true));
        assert_eq!(world.get::<Glow>(e).unwrap().power, 7.5);
        let tint = Vec4::new(1.0, 0.5, 0.25, 1.0);
        assert_eq!(r.set(&mut world, e, "Glow.tint", InterpolatedValue::Vector4(tint)), Ok(true));
        assert_eq!(r.get(&world, e, "Glow.tint"), Some(InterpolatedValue::Vector4(tint)));
        assert_eq!(r.set(&mut world, e, "Glow.tint", InterpolatedValue::Vector4(tint)), Ok(false), "equal: not written");
        assert_eq!(r.names().count(), 6, "power, tint and tint's four lanes");
        let listed: Vec<_> = r.properties_of(&world, e).map(|(n, _)| n).collect();
        assert_eq!(listed, ["Glow.power", "Glow.tint", "Glow.tint.x", "Glow.tint.y", "Glow.tint.z", "Glow.tint.w"]);
        let bare = world.spawn();
        assert_eq!(r.properties_of(&world, bare).count(), 0, "only components the entity has");
    }

    #[test]
    fn lane_entry_changes_only_its_lane() {
        let r = registry();
        let (mut world, e) = world_with_glow();
        assert_eq!(r.kind("Glow.tint.w"), Some(PropertyKind::Scalar));
        assert_eq!(r.get(&world, e, "Glow.tint.z"), Some(InterpolatedValue::Scalar(0.3)));
        assert_eq!(r.set(&mut world, e, "Glow.tint.w", InterpolatedValue::Scalar(0.9)), Ok(true));
        assert_eq!(world.get::<Glow>(e).unwrap().tint, Vec4::new(0.1, 0.2, 0.3, 0.9));
        assert_eq!(r.set(&mut world, e, "Glow.tint.x", InterpolatedValue::Scalar(0.6)), Ok(true));
        assert_eq!(world.get::<Glow>(e).unwrap().tint, Vec4::new(0.6, 0.2, 0.3, 0.9));
        assert_eq!(world.get::<Glow>(e).unwrap().power, 1.0, "another field is untouched");
    }

    #[test]
    fn reregister_is_idempotent_conflict_is_name_taken() {
        let mut r = registry();
        assert_eq!(r.register::<Glow, f32>("Glow.power", |g| g.power, |g, v| g.power = v), Ok(()));
        assert_eq!(r.register_lanes::<Glow, Vec4>("Glow.tint", |g| g.tint, |g, v| g.tint = v), Ok(()));
        assert_eq!(r.len(), 6, "nothing was added twice");
        let other = r.register::<Other, f32>("Glow.power", |o| o.0, |o, v| o.0 = v).unwrap_err();
        assert!(matches!(&other, PropertyError::NameTaken { name, component } if name == "Glow.power" && component.contains("Glow")));
        let kind = r.register::<Glow, Vec4>("Glow.power", |g| g.tint, |g, v| g.tint = v).unwrap_err();
        assert!(matches!(kind, PropertyError::NameTaken { .. }), "same component, another kind");
        assert!(!other.to_string().is_empty());
    }

    /// The point of comparing before writing: a keyed field held at one value must not look
    /// changed to `Changed<C>` every frame, or every extract re-copies it.
    #[test]
    fn equal_value_does_not_stamp_change_tick() {
        let r = registry();
        let (mut world, e) = world_with_glow();
        let t1 = world.begin_change_frame(0);
        world.begin_change_frame(t1);
        assert_eq!(world.query_filtered::<(Entity, &Glow), Changed<Glow>>().iter().count(), 0, "nothing written yet");
        assert_eq!(r.set(&mut world, e, "Glow.power", InterpolatedValue::Scalar(2.0)), Ok(true));
        assert_eq!(world.query_filtered::<(Entity, &Glow), Changed<Glow>>().iter().count(), 1, "a real change is seen");
        world.begin_change_frame(t1 + 1);
        assert_eq!(r.set(&mut world, e, "Glow.power", InterpolatedValue::Scalar(2.0)), Ok(false));
        assert_eq!(world.query_filtered::<(Entity, &Glow), Changed<Glow>>().iter().count(), 0, "an equal value is not a change");
    }

    #[test]
    fn unknown_kind_mismatch_and_missing_component_errors() {
        let r = registry();
        let (mut world, e) = world_with_glow();
        assert_eq!(
            r.set(&mut world, e, "Glow.nope", InterpolatedValue::Scalar(1.0)),
            Err(PropertyError::Unknown("Glow.nope".into()))
        );
        assert_eq!(
            r.set(&mut world, e, "Glow.power", InterpolatedValue::Vector2(Vec2::ONE)),
            Err(PropertyError::KindMismatch { name: "Glow.power".into(), expected: PropertyKind::Scalar, got: Some(PropertyKind::Vector2) })
        );
        assert!(matches!(
            r.set(&mut world, e, "Glow.power", InterpolatedValue::Translation(Vec3::ZERO)),
            Err(PropertyError::KindMismatch { got: None, .. })
        ));
        let bare = world.spawn();
        assert_eq!(
            r.set(&mut world, bare, "Glow.power", InterpolatedValue::Scalar(1.0)),
            Err(PropertyError::MissingComponent("Glow.power".into()))
        );
        assert_eq!(r.get(&world, bare, "Glow.power"), None);
        world.despawn(e);
        assert!(matches!(r.set(&mut world, e, "Glow.power", InterpolatedValue::Scalar(1.0)), Err(PropertyError::MissingComponent(_))));
        for err in [PropertyError::Unknown("a".into()), PropertyError::MissingComponent("a".into())] {
            assert!(!err.to_string().is_empty());
        }
    }

    #[test]
    fn warn_once_warns_once_per_key() {
        let mut r = PropertyRegistry::default();
        assert!(r.warn_once("clip/A.b", "first"));
        assert!(!r.warn_once("clip/A.b", "again"));
        assert!(r.warn_once("clip/A.c", "another key"));
    }
}
