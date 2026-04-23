use gizmo_core::{Component, World};
use ron::Value;
use serde::{de::DeserializeOwned, Serialize};
use std::collections::HashMap;

pub type SerializeFn = Box<dyn Fn(&World, u32) -> Option<Value> + Send + Sync>;
pub type DeserializeFn = Box<dyn Fn(&mut World, u32, &Value) + Send + Sync>;

pub struct SceneRegistry {
    serializers: HashMap<String, SerializeFn>,
    deserializers: HashMap<String, DeserializeFn>,
}

impl SceneRegistry {
    pub fn new() -> Self {
        Self {
            serializers: HashMap::new(),
            deserializers: HashMap::new(),
        }
    }

    /// T türündeki standart bileşeni kaydeder. T, Ser/De yeteneğine sahip olmalıdır.
    pub fn register<T>(&mut self, name: &str)
    where
        T: Component + Serialize + DeserializeOwned + Clone,
    {
        let name_ser = name.to_string();
        let name_de = name.to_string();

        self.serializers.insert(
            name_ser,
            Box::new(move |world, entity_id| {
                let storage = world.borrow::<T>();
                if let Some(comp) = storage.get(entity_id) {
                    // RON String'e dönüştür ve oradan AST'ye (Value) Parse et
                    match ron::ser::to_string(comp) {
                        Ok(string_repr) => match ron::from_str::<Value>(&string_repr) {
                            Ok(val) => return Some(val),
                            Err(e) => println!(
                                "[SceneRegistry] AST Donusturme Hatasi ({}): {}",
                                std::any::type_name::<T>(),
                                e
                            ),
                        },
                        Err(e) => println!(
                            "[SceneRegistry] Serilestirme Hatasi ({}): {}",
                            std::any::type_name::<T>(),
                            e
                        ),
                    }
                }
                None
            }),
        );

        self.deserializers.insert(
            name_de,
            Box::new(move |world, entity_id, value| {
                // RON AST'sinden (Value) doğrudan hedeflenen T türüne çevir (Gereksiz String dönüşümünü atlar ve düzgün parse eder)
                if let Ok(comp) = value.clone().into_rust::<T>() {
                    world.add_component(
                        world
                            .get_entity(entity_id)
                            .expect("Invalid entity mapping during deserialization!"),
                        comp,
                    );
                } else {
                    println!(
                        "[SceneRegistry] HATA: {} bileseni yuklenemedi! (Entity: {})",
                        std::any::type_name::<T>(),
                        entity_id
                    );
                }
            }),
        );
    }

    /// Ser/De Derive edilemeyen `Mesh`, `Material` gibi sistemler için manuel Closure kaydı.
    pub fn register_custom(
        &mut self,
        name: &str,
        serialize: impl Fn(&World, u32) -> Option<Value> + Send + Sync + 'static,
        deserialize: impl Fn(&mut World, u32, &Value) + Send + Sync + 'static,
    ) {
        self.serializers
            .insert(name.to_string(), Box::new(serialize));
        self.deserializers
            .insert(name.to_string(), Box::new(deserialize));
    }

    pub fn get_serializer(&self, name: &str) -> Option<&SerializeFn> {
        self.serializers.get(name)
    }

    pub fn get_deserializer(&self, name: &str) -> Option<&DeserializeFn> {
        self.deserializers.get(name)
    }

    pub fn all_components(&self) -> impl Iterator<Item = &String> {
        self.serializers.keys()
    }

    /// Gizmo motorunun varsayılan bileşenleri eklenmiş halde registry döndürür.
    pub fn with_core_components() -> Self {
        let mut reg = Self::new();

        reg.register::<gizmo_physics::components::Transform>("Transform");
        reg.register::<gizmo_physics::components::Velocity>("Velocity");
        reg.register::<gizmo_physics::components::RigidBody>("RigidBody");
        reg.register::<gizmo_physics::shape::Collider>("Collider");

        reg.register::<gizmo_renderer::components::Camera>("Camera");
        reg.register::<gizmo_renderer::components::PointLight>("PointLight");
        reg.register::<gizmo_renderer::components::DirectionalLight>("DirectionalLight");
        reg.register::<gizmo_renderer::components::Terrain>("Terrain");
        reg.register::<gizmo_renderer::components::ParticleEmitter>("ParticleEmitter");

        reg.register::<gizmo_audio::AudioSource>("AudioSource");
        reg.register::<gizmo_scripting::Script>("Script");

        reg
    }
}

impl Default for SceneRegistry {
    fn default() -> Self {
        Self::with_core_components()
    }
}
