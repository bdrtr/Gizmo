//! glTF/GLB spawning: the `Commands::spawn_gltf[_async_completed]` entry points and the
//! `spawn_gltf_node_flat` scene-walker. Extracted verbatim from spawner.rs (pure move).
//! `use super::*` brings in Commands, EntityBuilder, GltfLoadError, `spawn_mesh_entity` and the
//! component/renderer imports.

use super::*;

impl<'a> Commands<'a> {
    // ── GLTF Yükleme ─────────────────────────────────────────────────────────────────────────────

    /// GLTF/GLB dosyasını yükler ve dünya içinde spawn eder.
    /// Animasyon ve iskelet hiyerarşisi otomatik oluşturulur.
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

                for node in &asset.roots {
                    spawn_gltf_node_flat(
                        self.world,
                        node,
                        root.id(),
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

    /// Asenkron GLTF yükleme tamamlandığında çağrılacak metot.
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

                for node in &asset.roots {
                    spawn_gltf_node_flat(
                        self.world,
                        node,
                        root.id(),
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

fn spawn_gltf_node_flat(
    world: &mut World,
    node: &gizmo_renderer::asset::GltfNodeData,
    parent_id: u32,
    default_mat: Material,
    attach_colliders: bool,
    skeletons: &[gizmo_renderer::components::Skeleton],
) {
    use gizmo_core::component::{Children, Parent};
    let entity = world.spawn();
    let name = node.name.clone().unwrap_or_else(|| "GLTF_Node".to_string());
    world.add_component(entity, EntityName(name));
    world.add_component(entity, Parent(parent_id));
    world.add_component(entity, Children(Vec::new()));

    {
        let mut ch_store = world.borrow_mut::<Children>();
        // Safe to push since entity just spawned and didn't trigger any complex re-borrow updates
        if let Some(mut parent_ch) = ch_store.get_mut(parent_id) {
            parent_ch.0.push(entity.id());
        }
    }

    let raw_rot = Quat::from_xyzw(
        node.rotation[0],
        node.rotation[1],
        node.rotation[2],
        node.rotation[3],
    );
    let rot = if raw_rot.is_nan() || raw_rot.length() < 0.0001 {
        Quat::IDENTITY
    } else {
        raw_rot.normalize()
    };

    let mut t = Transform::new(Vec3::new(
        node.translation[0],
        node.translation[1],
        node.translation[2],
    ))
    .with_rotation(rot)
    .with_scale(Vec3::new(node.scale[0], node.scale[1], node.scale[2]));
    t.update_local_matrix();

    if node.skin_index.is_some() || node.name.as_deref() == Some("Armature") {
        t = Transform::default();
        t.update_local_matrix();
    }
    
    world.add_component(entity, t);
    world.add_component(entity, gizmo_physics_core::components::GlobalTransform::default());

    // Per-node/per-primitive → iç-döngü/özyineleme detayı, bu yüzden trace! (bir glTF yüzlerce
    // node içerebilir; aggregate özet çağıran `spawn_gltf`'in info! satırında verilir).
    tracing::trace!(
        entity = entity.id(),
        name = ?node.name,
        primitives = node.primitives.len(),
        children = node.children.len(),
        "glTF node spawn'lanıyor"
    );
    let mut newly_added_prims = Vec::new();
    for (mesh, mat_opt) in node.primitives.iter() {
        tracing::trace!(
            source = %mesh.source,
            bounds_min = ?mesh.bounds.min,
            bounds_max = ?mesh.bounds.max,
            "glTF primitive spawn'lanıyor"
        );
        let prim = world.spawn();
        let mut prim_t = Transform::new(Vec3::ZERO);
        prim_t.update_local_matrix();
        world.add_component(prim, prim_t);
        world.add_component(prim, gizmo_physics_core::components::GlobalTransform::default());
        world.add_component(prim, Parent(entity.id()));
        world.add_component(prim, Children(Vec::new()));

        newly_added_prims.push(prim.id());

        world.add_component(prim, mesh.clone());
        world.add_component(prim, mat_opt.clone().unwrap_or_else(|| default_mat.clone()));
        world.add_component(prim, MeshRenderer::new());

        if let Some(skin_idx) = node.skin_index {
            if skin_idx < skeletons.len() {
                world.add_component(prim, skeletons[skin_idx].clone());
            }
        }

        if attach_colliders {
            let extents = (mesh.bounds.max - mesh.bounds.min) / 2.0;
            let center_offset = (mesh.bounds.max + mesh.bounds.min) / 2.0;
            let cx = extents.x.max(0.01);
            let cy = extents.y.max(0.01);
            let cz = extents.z.max(0.01);
            
            // Eğer merkeze tam oturmuyorsa Compound(offset_box) kullanarak hizala
            if center_offset.length_squared() > 0.0001 {
                world.add_component(
                    prim,
                    gizmo_physics_core::Collider::offset_box(center_offset.into(), gizmo_math::Vec3::new(cx, cy, cz)),
                );
            } else {
                world.add_component(prim, gizmo_physics_core::Collider::new_aabb(cx, cy, cz));
            }
        }
    }

    // Pulling borrow_mut OUTSIDE the loop avoiding multiple overlapping mutable queries
    if !newly_added_prims.is_empty() {
        {
            let mut ch_store = world.borrow_mut::<Children>();
            if let Some(mut parent_ch) = ch_store.get_mut(entity.id()) {
                parent_ch.0.extend(newly_added_prims);
            }
        }
    }

    for child_node in &node.children {
        spawn_gltf_node_flat(
            world,
            child_node,
            entity.id(),
            default_mat.clone(),
            attach_colliders,
            skeletons,
        );
    }
}

