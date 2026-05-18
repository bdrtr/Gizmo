pub struct TransformSyncSystem;

impl gizmo_core::system::System for TransformSyncSystem {
    fn access_info(&self) -> gizmo_core::system::AccessInfo {
        let mut info = gizmo_core::system::AccessInfo::new();
        // Since we borrow components, we can mark it as exclusive to be safe, or specify component access.
        // For simplicity and safety during hierarchy traversal, we'll make it exclusive.
        info.is_exclusive = true;
        info
    }

    fn run(&mut self, world: &gizmo_core::world::World, _dt: f32) {
        if let Some(mut transforms) = world.query::<gizmo_core::query::Mut<gizmo_physics_core::Transform>>() {
            for (_, mut trans) in transforms.iter_mut() {
                trans.update_local_matrix();
            }
        }
    }
}

pub struct TransformPropagateSystem;

impl gizmo_core::system::System for TransformPropagateSystem {
    fn access_info(&self) -> gizmo_core::system::AccessInfo {
        let mut info = gizmo_core::system::AccessInfo::new();
        info.is_exclusive = true; // Safe fallback for complex queries
        info
    }

    fn run(&mut self, world: &gizmo_core::world::World, _dt: f32) {
        // Query to get root transforms (no Parent)
        let root_query = world.query::<(
            &gizmo_physics_core::Transform,
            gizmo_core::query::Mut<gizmo_physics_core::components::GlobalTransform>,
            gizmo_core::query::Without<gizmo_core::component::Parent>,
        )>();

        let mut queue = Vec::new();

        if let Some(mut roots) = root_query {
            let mut children_query = world.query::<&gizmo_core::component::Children>();
            for (id, (local, mut global, _)) in roots.iter_mut() {
                global.matrix = local.local_matrix;
                if let Some(children_q) = &mut children_query {
                    if let Some(children) = children_q.get(id) {
                        for &child_id in &children.0 {
                            queue.push((global.matrix, child_id));
                        }
                    }
                }
            }
        }

        // Processing children (we need random access, so we do individual queries)
        let mut local_query = world.query::<&gizmo_physics_core::Transform>();
        let mut global_query = world.query::<gizmo_core::query::Mut<gizmo_physics_core::components::GlobalTransform>>();
        let mut children_query = world.query::<&gizmo_core::component::Children>();

        let mut head = 0;
        while head < queue.len() {
            let (parent_matrix, current_id) = queue[head];
            head += 1;

            if let (Some(lq), Some(gq)) = (&mut local_query, &mut global_query) {
                if let (Some(local), Some(mut global)) = (lq.get(current_id), gq.get(current_id)) {
                    global.matrix = parent_matrix * local.local_matrix;

                    if let Some(cq) = &mut children_query {
                        if let Some(children) = cq.get(current_id) {
                            for &child_id in &children.0 {
                                queue.push((global.matrix, child_id));
                            }
                        }
                    }
                }
            }
        }
    }
}

pub struct BoneAttachmentSystem;

impl gizmo_core::system::System for BoneAttachmentSystem {
    fn access_info(&self) -> gizmo_core::system::AccessInfo {
        let mut info = gizmo_core::system::AccessInfo::new();
        info.is_exclusive = true;
        info
    }

    fn run(&mut self, world: &gizmo_core::world::World, _dt: f32) {
        if let Some(query) = world.query::<&gizmo_renderer::components::BoneAttachment>() {
            let mut skeletons = world.query::<&gizmo_renderer::components::Skeleton>();
            let mut transforms = world.query::<gizmo_core::query::Mut<gizmo_physics_core::Transform>>();
            
            for (id, attachment) in query.iter() {
                if let Some(sq) = &mut skeletons {
                    if let Some(skeleton) = sq.get(attachment.target_entity.id()) {
                        if let Some(global_matrix) = skeleton.global_poses.get(attachment.bone_index) {
                            if let Some(tq) = &mut transforms {
                                if let Some(mut trans) = tq.get(id) {
                                    let final_mat = *global_matrix * attachment.offset;
                                    let (t, r, s) = gizmo_renderer::decompose_mat4(final_mat);
                                    trans.position = t;
                                    trans.rotation = r;
                                    trans.scale = s;
                                    trans.update_local_matrix();
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
