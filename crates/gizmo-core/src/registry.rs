//! Component Registry — type name ↔ TypeId mapping at runtime
//!
//! It lets Lua scripts and the Editor reach components by name.
//!
//! ```
//! use gizmo_core::registry::ComponentRegistry;
//! use std::any::TypeId;
//! # struct Transform;
//! # struct Camera;
//!
//! let mut registry = ComponentRegistry::new();
//! registry.register::<Transform>("Transform").unwrap();
//! registry.register::<Camera>("Camera").unwrap();
//!
//! assert_eq!(registry.get_name::<Transform>(), Some("Transform"));
//! assert_eq!(registry.get_type_id("Camera"), Some(TypeId::of::<Camera>()));
//! ```

use std::any::TypeId;
use std::collections::BTreeMap;

/// The errors that can arise during Component Registry registration operations.
///
/// These errors represent programmatic (recoverable) conflicts; since they can be triggered
/// by user input in the script/editor integration they are surfaced with a `Result` instead
/// of a panic.
#[derive(Debug)]
#[non_exhaustive]
pub enum RegistryError {
    /// The same name is already assigned to a different type.
    NameAlreadyRegistered {
        /// The conflicting name.
        name: String,
    },
    /// The same type is already registered under a different name.
    TypeAlreadyRegistered {
        /// The type's existing registered name.
        existing_name: String,
        /// The new name that was attempted to be registered.
        attempted_name: String,
    },
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryError::NameAlreadyRegistered { name } => write!(
                f,
                "ComponentRegistry: '{}' ismi zaten farklı bir tipe atanmış!",
                name
            ),
            RegistryError::TypeAlreadyRegistered {
                existing_name,
                attempted_name,
            } => write!(
                f,
                "ComponentRegistry: Bu tip zaten '{}' ismiyle kayıtlı, '{}' ile tekrar kayıt edilemez!",
                existing_name, attempted_name
            ),
        }
    }
}

impl std::error::Error for RegistryError {}

/// Type alias for component serialization function pointers.
///
/// The argument is a type-erased `*const T` for the exact type of the [`TypeRegistration`]
/// this pointer is stored in. The implementations installed by
/// [`ComponentRegistry::register_serializable`] cast it back without any check, so handing
/// over a pointer to a different type — or a dangling, misaligned or uninitialised one — is
/// undefined behaviour, even though the `fn` signature is safe. The result is RON text; the
/// `Err` string is the serializer's own message, meant for logs, not for matching on.
pub type SerializeFn = fn(*const u8) -> Result<String, String>;
/// Type alias for component deserialization function pointers.
///
/// Takes RON text and puts the resulting component on `entity`. `Ok(())` reports that the
/// text parsed, not that the world changed: the installed implementations forward to
/// `World::add_component`, which replaces any existing component of that type and does
/// nothing at all when the entity is not alive.
pub type DeserializeFn =
    fn(&mut crate::world::World, crate::entity::Entity, &str) -> Result<(), String>;
/// Type alias for component JSON getter function pointers.
///
/// Same erased-pointer contract, and the same undefined behaviour on a type mismatch, as
/// [`SerializeFn`] — only the output format differs.
pub type GetJsonFn = fn(*const u8) -> Result<serde_json::Value, String>;
/// Type alias for component JSON setter function pointers.
///
/// A whole-component replace, not a field patch: the value has to deserialize into a complete
/// component, so fields left out of the JSON fail unless the type's `serde` impl defaults
/// them. Carries the same "`Ok` means parsed" caveat as [`DeserializeFn`].
pub type SetJsonFn =
    fn(&mut crate::world::World, crate::entity::Entity, serde_json::Value) -> Result<(), String>;
/// Type alias for reflection-based component insertion function pointers.
///
/// Only present when the `reflect` feature is enabled.
#[cfg(feature = "reflect")]
pub type InsertReflectFn = fn(
    &mut crate::world::World,
    crate::entity::Entity,
    &dyn bevy_reflect::PartialReflect,
) -> Result<(), String>;

/// The serialization struct that carries the optional ECS-based reflection capabilities
#[derive(Debug, Clone)]
pub struct TypeRegistration {
    /// Identity of the component type this entry describes.
    ///
    /// Always equal to the key this registration is filed under, and it is the type that
    /// every erased `*const u8` passed to the function pointers below must actually point at.
    pub type_id: TypeId,
    /// The script- and editor-facing name chosen when the type was registered.
    ///
    /// This is what [`ComponentRegistry::get_name_by_id`] returns. Nothing derives it from or
    /// checks it against the Rust type name; it is whatever string the registration passed,
    /// and it is compared byte-for-byte, so it is case-sensitive.
    pub name: String,
    /// Component → RON text, or `None` if this type was registered without serialization
    /// support (plain [`ComponentRegistry::register`], or `register_reflect`).
    ///
    /// [`ComponentRegistry::register_serializable`] installs this and the three fields below
    /// in one go, so within a registration it produced they are either all `Some` or all
    /// `None`. See [`SerializeFn`] for the pointer contract — it is unchecked.
    pub serialize_fn: Option<SerializeFn>,
    /// RON text → component on an entity, parsing the dialect `serialize_fn` emits, so the
    /// two round-trip. See [`DeserializeFn`] for what it does to an existing component and
    /// what `Ok(())` does and does not promise.
    pub deserialize_fn: Option<DeserializeFn>,
    /// Component → `serde_json::Value` — a value tree rather than text, so a caller can walk
    /// or edit individual fields without re-parsing. Same unchecked erased-pointer contract
    /// as `serialize_fn`; see [`GetJsonFn`].
    pub get_json_fn: Option<GetJsonFn>,
    /// `serde_json::Value` → component on an entity: the inverse of `get_json_fn`. See
    /// [`SetJsonFn`] — in particular for why a caller that edited one field still has to send
    /// the whole object back.
    pub set_json_fn: Option<SetJsonFn>,
    /// Reflection accessor — only present with the `reflect` feature.
    #[cfg(feature = "reflect")]
    pub get_reflect_ptr_fn: Option<fn(*const u8) -> *const dyn bevy_reflect::Reflect>,
    /// Reflection-based insertion — only present with the `reflect` feature.
    #[cfg(feature = "reflect")]
    pub insert_reflect_fn: Option<InsertReflectFn>,
}

/// The registry for querying and managing component types by name.
///
/// It keeps a two-way mapping: name → TypeId and TypeId → TypeRegistration.
/// `register()` prevents double registration and desync.
pub struct ComponentRegistry {
    /// Name → TypeId mapping (ordered — deterministic iteration)
    name_to_type: BTreeMap<String, TypeId>,
    /// TypeId → Reflection & Serialization Registration
    type_to_reg: BTreeMap<TypeId, TypeRegistration>,
    /// `bevy_reflect`-based type registrations — only present with the `reflect` feature.
    #[cfg(feature = "reflect")]
    pub reflect_registry: bevy_reflect::TypeRegistry,
}

impl ComponentRegistry {
    /// An empty registry.
    ///
    /// No component type is registered for you — not even the engine's own — so every lookup
    /// misses until something calls one of the `register*` methods. Registration is per
    /// instance and lives in normal memory; there is no process-wide registry behind this,
    /// so two `ComponentRegistry` values know nothing about each other.
    pub fn new() -> Self {
        Self {
            name_to_type: BTreeMap::new(),
            type_to_reg: BTreeMap::new(),
            #[cfg(feature = "reflect")]
            reflect_registry: bevy_reflect::TypeRegistry::default(),
        }
    }

    /// Register a new component type by name.
    ///
    /// # Errors
    /// - [`RegistryError::NameAlreadyRegistered`] — if the same name is taken by a different type
    /// - [`RegistryError::TypeAlreadyRegistered`] — if the same type is registered under another name
    ///
    /// Registering again with the same type-name pair is safe (a no-op, `Ok(())`).
    pub fn register<T: 'static>(&mut self, name: &str) -> Result<(), RegistryError> {
        let type_id = TypeId::of::<T>();

        // Aynı çiftle tekrar kayıt — no-op
        if let Some(&existing_tid) = self.name_to_type.get(name) {
            if existing_tid == type_id {
                return Ok(()); // Zaten kayıtlı, aynı çift
            }
            return Err(RegistryError::NameAlreadyRegistered {
                name: name.to_string(),
            });
        }

        if let Some(existing_reg) = self.type_to_reg.get(&type_id) {
            return Err(RegistryError::TypeAlreadyRegistered {
                existing_name: existing_reg.name.clone(),
                attempted_name: name.to_string(),
            });
        }

        self.name_to_type.insert(name.to_string(), type_id);
        self.type_to_reg.insert(
            type_id,
            TypeRegistration {
                type_id,
                name: name.to_string(),
                serialize_fn: None,
                deserialize_fn: None,
                get_json_fn: None,
                set_json_fn: None,
                #[cfg(feature = "reflect")]
                get_reflect_ptr_fn: None,
                #[cfg(feature = "reflect")]
                insert_reflect_fn: None,
            },
        );
        Ok(())
    }

    /// Register a new component type by name and with the Reflection (serde) capability.
    ///
    /// Only available with the `reflect` feature.
    #[cfg(feature = "reflect")]
    pub fn register_reflect<T: bevy_reflect::Reflect + bevy_reflect::FromReflect + bevy_reflect::GetTypeRegistration + crate::component::Component + Clone + 'static>(&mut self, name: &str) {
        self.reflect_registry.register::<T>();
        let type_id = TypeId::of::<T>();

        let get_reflect_ptr_fn: fn(*const u8) -> *const dyn bevy_reflect::Reflect = |ptr| {
            // SAFETY: this closure is stored under `TypeId::of::<T>()` and the registry only ever
            // calls it with a pointer to a live component of that same type.
            let component = unsafe { &*(ptr as *const T) };
            component as &dyn bevy_reflect::Reflect as *const dyn bevy_reflect::Reflect
        };

        let insert_reflect_fn: fn(&mut crate::world::World, crate::entity::Entity, &dyn bevy_reflect::PartialReflect) -> Result<(), String> = |world, entity, partial_reflect| {
            if let Some(concrete) = <T as bevy_reflect::FromReflect>::from_reflect(partial_reflect) {
                world.add_component(entity, concrete);
                Ok(())
            } else {
                Err(format!("Could not convert PartialReflect to {}", std::any::type_name::<T>()))
            }
        };

        if let Some(reg) = self.type_to_reg.get_mut(&type_id) {
            reg.get_reflect_ptr_fn = Some(get_reflect_ptr_fn);
            reg.insert_reflect_fn = Some(insert_reflect_fn);
        } else {
            self.name_to_type.insert(name.to_string(), type_id);
            self.type_to_reg.insert(
                type_id,
                TypeRegistration {
                    type_id,
                    name: name.to_string(),
                    serialize_fn: None,
                    deserialize_fn: None,
                    get_json_fn: None,
                    set_json_fn: None,
                    get_reflect_ptr_fn: Some(get_reflect_ptr_fn),
                    insert_reflect_fn: Some(insert_reflect_fn),
                },
            );
        }
    }

    /// Registers `T` under `name` *and* installs its RON and JSON accessors.
    ///
    /// Everything [`ComponentRegistry::register`] does, plus the four function pointers of
    /// [`TypeRegistration`], built from `T`'s `serde` impls (RON for
    /// `serialize_fn`/`deserialize_fn`, `serde_json` for the JSON pair).
    ///
    /// # Errors
    /// - [`RegistryError::NameAlreadyRegistered`] — `name` is taken by a different type
    /// - [`RegistryError::TypeAlreadyRegistered`] — `T` is already registered under another name
    ///
    /// Repeating the same type-name pair is a no-op returning `Ok(())`. That short-circuit is
    /// blunt, and it bites: if `T` was registered under this same name by plain
    /// [`ComponentRegistry::register`], this call also returns `Ok(())` and the accessors are
    /// **never installed** — the registration keeps `serialize_fn: None` and the type stays
    /// silently unserializable. Use this method from the start for such a type, or
    /// [`ComponentRegistry::unregister`] it first.
    pub fn register_serializable<
        T: crate::component::Component + serde::Serialize + serde::de::DeserializeOwned,
    >(
        &mut self,
        name: &str,
    ) -> Result<(), RegistryError> {
        let type_id = TypeId::of::<T>();

        if let Some(&existing_tid) = self.name_to_type.get(name) {
            if existing_tid == type_id {
                return Ok(());
            }
            return Err(RegistryError::NameAlreadyRegistered {
                name: name.to_string(),
            });
        }
        if let Some(existing_reg) = self.type_to_reg.get(&type_id) {
            return Err(RegistryError::TypeAlreadyRegistered {
                existing_name: existing_reg.name.clone(),
                attempted_name: name.to_string(),
            });
        }

        self.name_to_type.insert(name.to_string(), type_id);

        let serialize_fn: fn(*const u8) -> Result<String, String> = |ptr| {
            // SAFETY: registered under `TypeId::of::<T>()`; called only with a live `T`.
            let component = unsafe { &*(ptr as *const T) };
            ron::to_string(component).map_err(|e| e.to_string())
        };

        let deserialize_fn: fn(
            &mut crate::world::World,
            crate::entity::Entity,
            &str,
        ) -> Result<(), String> = |world, entity, data| {
            let component: T = ron::from_str(data).map_err(|e| e.to_string())?;
            world.add_component(entity, component);
            Ok(())
        };

        let get_json_fn: fn(*const u8) -> Result<serde_json::Value, String> = |ptr| {
            // SAFETY: registered under `TypeId::of::<T>()`; called only with a live `T`.
            let component = unsafe { &*(ptr as *const T) };
            serde_json::to_value(component).map_err(|e| e.to_string())
        };

        let set_json_fn: fn(
            &mut crate::world::World,
            crate::entity::Entity,
            serde_json::Value,
        ) -> Result<(), String> = |world, entity, val| {
            let component: T = serde_json::from_value(val).map_err(|e| e.to_string())?;
            world.add_component(entity, component);
            Ok(())
        };

        self.type_to_reg.insert(
            type_id,
            TypeRegistration {
                type_id,
                name: name.to_string(),
                serialize_fn: Some(serialize_fn),
                deserialize_fn: Some(deserialize_fn),
                get_json_fn: Some(get_json_fn),
                set_json_fn: Some(set_json_fn),
                #[cfg(feature = "reflect")]
                get_reflect_ptr_fn: None,
                #[cfg(feature = "reflect")]
                insert_reflect_fn: None,
            },
        );
        Ok(())
    }

    /// Deletes a type's registration. The name and the TypeId mapping are removed together.
    /// Returns false if it is not registered.
    pub fn unregister<T: 'static>(&mut self) -> bool {
        let type_id = TypeId::of::<T>();
        if let Some(reg) = self.type_to_reg.remove(&type_id) {
            self.name_to_type.remove(&reg.name);
            true
        } else {
            false
        }
    }

    /// Deletes a type's registration by name.
    pub fn unregister_by_name(&mut self, name: &str) -> bool {
        if let Some(type_id) = self.name_to_type.remove(name) {
            self.type_to_reg.remove(&type_id);
            true
        } else {
            false
        }
    }

    // ──── Sorgulama ────

    /// Conversion from name to TypeId
    pub fn get_type_id(&self, name: &str) -> Option<TypeId> {
        self.name_to_type.get(name).copied()
    }

    /// Conversion from TypeId to name (generic — with compile-time type information)
    pub fn get_name<T: 'static>(&self) -> Option<&str> {
        self.get_name_by_id(TypeId::of::<T>())
    }

    /// Conversion from TypeId to name (with a runtime TypeId)
    pub fn get_name_by_id(&self, type_id: TypeId) -> Option<&str> {
        self.type_to_reg.get(&type_id).map(|reg| reg.name.as_str())
    }

    /// Holds the Serialization methods (if any) for the TypeId in question
    pub fn get_registration(&self, type_id: TypeId) -> Option<&TypeRegistration> {
        self.type_to_reg.get(&type_id)
    }

    /// Is the name registered?
    pub fn contains_name(&self, name: &str) -> bool {
        self.name_to_type.contains_key(name)
    }

    /// Is the type registered?
    pub fn contains_type<T: 'static>(&self) -> bool {
        self.type_to_reg.contains_key(&TypeId::of::<T>())
    }

    /// Returns all the registered component names in order.
    /// Because a BTreeMap is used the order is always alphabetical and deterministic.
    pub fn all_names(&self) -> Vec<&str> {
        self.name_to_type.keys().map(|s| s.as_str()).collect()
    }

    /// The number of registered components
    #[inline]
    pub fn len(&self) -> usize {
        self.name_to_type.len()
    }

    /// Whether the name table is empty — exactly what [`len`](Self::len) reports as `0`.
    ///
    /// It counts *names*, not types. Normally that is the same question, but it is not
    /// guaranteed to be: a type entry can outlive the name it was filed under, so this can
    /// report `true` while [`contains_type`](Self::contains_type) still answers `true` for
    /// some type. Read it as "nothing is reachable by name" — the question the name-keyed
    /// lookups ([`get_type_id`](Self::get_type_id), [`contains_name`](Self::contains_name),
    /// [`all_names`](Self::all_names)) actually ask.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.name_to_type.is_empty()
    }
}

impl Default for ComponentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::impl_component;

    #[derive(serde::Serialize, serde::Deserialize, Clone)]
    struct Transform {
        x: f32,
    }
    impl_component!(Transform);

    #[derive(serde::Serialize, serde::Deserialize, Clone)]
    struct Camera;
    impl_component!(Camera);

    #[derive(serde::Serialize, serde::Deserialize, Clone)]
    struct PointLight;
    impl_component!(PointLight);

    #[test]
    fn test_register_and_lookup() {
        let mut reg = ComponentRegistry::new();
        reg.register::<Transform>("Transform").unwrap();
        reg.register::<Camera>("Camera").unwrap();

        assert_eq!(reg.get_name::<Transform>(), Some("Transform"));
        assert_eq!(reg.get_name::<Camera>(), Some("Camera"));
        assert_eq!(
            reg.get_type_id("Transform"),
            Some(TypeId::of::<Transform>())
        );
        assert_eq!(reg.len(), 2);
    }

    #[test]
    fn test_idempotent_register() {
        let mut reg = ComponentRegistry::new();
        reg.register::<Transform>("Transform").unwrap();
        reg.register::<Transform>("Transform").unwrap(); // No-op
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn test_duplicate_name_errors() {
        let mut reg = ComponentRegistry::new();
        reg.register::<Transform>("Shared").unwrap();
        let err = reg.register::<Camera>("Shared").unwrap_err(); // Farklı tip, aynı isim
        assert!(matches!(
            err,
            RegistryError::NameAlreadyRegistered { .. }
        ));
    }

    #[test]
    fn test_duplicate_type_errors() {
        let mut reg = ComponentRegistry::new();
        reg.register::<Transform>("Transform").unwrap();
        let err = reg.register::<Transform>("transform").unwrap_err(); // Aynı tip, farklı isim
        assert!(matches!(
            err,
            RegistryError::TypeAlreadyRegistered { .. }
        ));
    }

    #[test]
    fn test_unregister() {
        let mut reg = ComponentRegistry::new();
        reg.register::<Transform>("Transform").unwrap();
        assert!(reg.contains_type::<Transform>());

        assert!(reg.unregister::<Transform>());
        assert!(!reg.contains_type::<Transform>());
        assert!(!reg.contains_name("Transform"));
        assert_eq!(reg.len(), 0);
    }

    #[test]
    fn test_unregister_by_name() {
        let mut reg = ComponentRegistry::new();
        reg.register::<Camera>("Camera").unwrap();

        assert!(reg.unregister_by_name("Camera"));
        assert!(!reg.contains_name("Camera"));
        assert!(!reg.contains_type::<Camera>());
    }

    #[test]
    fn test_unregister_nonexistent() {
        let mut reg = ComponentRegistry::new();
        assert!(!reg.unregister::<Transform>());
        assert!(!reg.unregister_by_name("Foo"));
    }

    #[test]
    fn test_contains() {
        let mut reg = ComponentRegistry::new();
        reg.register::<Transform>("Transform").unwrap();

        assert!(reg.contains_name("Transform"));
        assert!(reg.contains_type::<Transform>());
        assert!(!reg.contains_name("Camera"));
        assert!(!reg.contains_type::<Camera>());
    }

    #[test]
    fn test_all_names_sorted() {
        let mut reg = ComponentRegistry::new();
        reg.register::<PointLight>("PointLight").unwrap();
        reg.register::<Camera>("Camera").unwrap();
        reg.register::<Transform>("Transform").unwrap();

        let names = reg.all_names();
        assert_eq!(names, vec!["Camera", "PointLight", "Transform"]);
    }

    #[test]
    fn test_get_name_delegates_to_get_name_by_id() {
        let mut reg = ComponentRegistry::new();
        reg.register::<Transform>("Transform").unwrap();

        let by_generic = reg.get_name::<Transform>();
        let by_id = reg.get_name_by_id(TypeId::of::<Transform>());
        assert_eq!(by_generic, by_id);
    }

    #[test]
    fn test_re_register_after_unregister() {
        let mut reg = ComponentRegistry::new();
        reg.register::<Transform>("Transform").unwrap();
        reg.unregister::<Transform>();
        reg.register::<Transform>("NewTransform").unwrap(); // Farklı isimle tekrar kayıt — artık sorunsuz
        assert_eq!(reg.get_name::<Transform>(), Some("NewTransform"));
    }
}
