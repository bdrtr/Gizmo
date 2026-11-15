//! glTF/GLB spawning: the `Commands::spawn_gltf[_async_completed]` entry points and the
//! `spawn_gltf_node_flat` scene-walker. Extracted verbatim from spawner.rs (pure move).
//! `use super::*` brings in Commands, EntityBuilder, GltfLoadError, `spawn_mesh_entity` and the
//! component/renderer imports.

use super::*;
use crate::gltf_build::{spawn_gltf_node_flat, Joints, Place};

impl<'a> Commands<'a> {
    // ── GLTF Yükleme ─────────────────────────────────────────────────────────────────────────────

    /// Loads a GLTF/GLB file and spawns it inside the world.
    /// The animation and skeleton hierarchy are created automatically.
    pub fn spawn_gltf(
        &mut self,
        pos: Vec3,
        path: &str,
        attach_colliders: bool,
    ) -> Result<EntityBuilder<'_, 'a>, GltfLoadError> {
        let default_bg = self.asset_manager.as_mut().unwrap().create_white_texture(
            &self.renderer.device,
            &self.renderer.queue,
            &self.renderer.scene.texture_bind_group_layout,
        );
        let default_mat = Material::new(default_bg.clone());

        match self.asset_manager.as_mut().unwrap().load_gltf_scene(
            &self.renderer.device,
            &self.renderer.queue,
            &self.renderer.scene.texture_bind_group_layout,
            default_bg,
            path,
        ) {
            Ok(asset) => {
                let root = self.world.spawn();
                let mut trans = Transform::new(pos);
                trans.update_local_matrix();

                self.world.add_component(root, trans);
                self.world
                    .add_component(root, gizmo_physics_core::components::GlobalTransform::default());
                self.world
                    .add_component(root, EntityName(format!("GLTF: {}", path)));
                self.world
                    .add_component(root, gizmo_core::component::Children(Vec::new()));

                let mut skeletons = Vec::new();
                for skel_data in &asset.skeletons {
                    skeletons.push(self.renderer.create_skeleton(std::sync::Arc::new(skel_data.clone())));
                }

                let joints = Joints::of(&asset.skeletons);
                for node in &asset.roots {
                    spawn_gltf_node_flat(
                        self.world,
                        node,
                        root,
                        Place { root, bone: None },
                        &joints,
                        default_mat.clone(),
                        attach_colliders,
                        &skeletons,
                    );
                }

                if !skeletons.is_empty() {
                    self.world.add_component(root, skeletons[0].clone());
                }

                if !asset.animations.is_empty() {
                    self.world.add_component(
                        root,
                        gizmo_renderer::components::AnimationPlayer {
                            active_animation: 0,
                            current_time: 0.0,
                            loop_anim: true,
                            speed: 1.0,
                            animations: std::sync::Arc::from(
                                asset.animations.clone().into_boxed_slice(),
                            ),
                            ..Default::default()
                        },
                    );
                }

                // The file it came from, which a scene saves, and the clips it gave: the resolver
                // has nothing to do for this model, and an edit to its clips is told from them.
                gizmo_renderer::gltf_model::mark_spawned(self.world, root, path);
                // Bir-kez yaşam-döngüsü olayı: ağır bir asset yüklendi + sahneye spawn'landı.
                tracing::info!(
                    path,
                    entity = root.id(),
                    roots = asset.roots.len(),
                    skeletons = asset.skeletons.len(),
                    animations = asset.animations.len(),
                    "glTF sahnesi yüklendi ve spawn'landı"
                );

                Ok(EntityBuilder {
                    commands: self,
                    entity: root,
                })
            }
            Err(e) => {
                // Çağıran bunu sık sık `.ok()`/`.expect()` ile yutar → burada path + sebeple
                // görünür kıl (davranış değişmez; hata yine de döndürülür).
                tracing::warn!(path, error = %e, "glTF sahnesi yüklenemedi");
                Err(GltfLoadError::Load {
                    path: path.to_string(),
                    source: e.to_string(),
                })
            }
        }
    }

    /// The method to be called when asynchronous GLTF loading completes.
    pub fn spawn_gltf_async_completed(
        &mut self,
        completion: gizmo_renderer::async_assets::GltfImportCompletion,
        pos: Vec3,
        attach_colliders: bool,
    ) -> Result<EntityBuilder<'_, 'a>, GltfLoadError> {
        let default_bg = self.asset_manager.as_mut().unwrap().create_white_texture(
            &self.renderer.device,
            &self.renderer.queue,
            &self.renderer.scene.texture_bind_group_layout,
        );
        let default_mat = Material::new(default_bg.clone());

        match self.asset_manager.as_mut().unwrap().load_gltf_from_import(
            &self.renderer.device,
            &self.renderer.queue,
            &self.renderer.scene.texture_bind_group_layout,
            default_bg,
            &completion.path,
            completion.document,
            completion.buffers,
            completion.images,
        ) {
            Ok(asset) => {
                let root = self.world.spawn();
                let mut trans = Transform::new(pos);
                trans.update_local_matrix();

                self.world.add_component(root, trans);
                self.world
                    .add_component(root, gizmo_physics_core::components::GlobalTransform::default());
                self.world
                    .add_component(root, EntityName(format!("GLTF: {}", completion.path)));
                self.world
                    .add_component(root, gizmo_core::component::Children(Vec::new()));

                let mut skeletons = Vec::new();
                for skel_data in &asset.skeletons {
                    skeletons.push(self.renderer.create_skeleton(std::sync::Arc::new(skel_data.clone())));
                }

                let joints = Joints::of(&asset.skeletons);
                for node in &asset.roots {
                    spawn_gltf_node_flat(
                        self.world,
                        node,
                        root,
                        Place { root, bone: None },
                        &joints,
                        default_mat.clone(),
                        attach_colliders,
                        &skeletons,
                    );
                }

                if !skeletons.is_empty() {
                    self.world.add_component(root, skeletons[0].clone());
                }

                if !asset.animations.is_empty() {
                    self.world.add_component(
                        root,
                        gizmo_renderer::components::AnimationPlayer {
                            active_animation: 0,
                            current_time: 0.0,
                            loop_anim: true,
                            speed: 1.0,
                            animations: std::sync::Arc::from(
                                asset.animations.clone().into_boxed_slice(),
                            ),
                            ..Default::default()
                        },
                    );
                }

                gizmo_renderer::gltf_model::mark_spawned(self.world, root, &completion.path);
                // Bir-kez yaşam-döngüsü olayı: asenkron import GPU'ya alındı + spawn'landı.
                tracing::info!(
                    path = %completion.path,
                    entity = root.id(),
                    roots = asset.roots.len(),
                    skeletons = asset.skeletons.len(),
                    animations = asset.animations.len(),
                    "glTF sahnesi (async import) yüklendi ve spawn'landı"
                );

                Ok(EntityBuilder {
                    commands: self,
                    entity: root,
                })
            }
            Err(e) => {
                tracing::warn!(
                    path = %completion.path,
                    error = %e,
                    "glTF sahnesi (async import) yüklenemedi"
                );
                Err(GltfLoadError::Load {
                    path: completion.path.clone(),
                    source: e.to_string(),
                })
            }
        }
    }
}
