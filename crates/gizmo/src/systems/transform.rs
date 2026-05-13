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
        let transforms = world.borrow_mut::<crate::physics::Transform>();
        for (_, trans) in transforms.iter_mut() {
            trans.update_local_matrix();
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
        let locals = world.borrow::<crate::physics::Transform>();
        let mut globals = world.borrow_mut::<crate::physics::GlobalTransform>();
        let parents = world.borrow::<gizmo_core::component::Parent>();
        let children_storage = world.borrow::<gizmo_core::component::Children>();

        let mut queue = Vec::new();

        // 1. Kökleri (Root) işle ve GlobalTransform'larını kendi yerellerine eşitle
        for (id, local) in locals.iter() {
            if parents.get(id).is_none() {
                if let Some(global) = globals.get_mut(id) {
                    global.matrix = local.local_matrix;

                    // Eğer çocukları varsa kuyruğa ekle
                    if let Some(children) = children_storage.get(id) {
                        for &child_id in &children.0 {
                            queue.push((global.matrix, child_id));
                        }
                    }
                }
            }
        }

        // 2. Çocukları hiyerarşik olarak BFS ile işle
        let mut head = 0;
        while head < queue.len() {
            let (parent_matrix, current_id) = queue[head];
            head += 1;

            if let Some(local) = locals.get(current_id) {
                if let Some(global) = globals.get_mut(current_id) {
                    global.matrix = parent_matrix * local.local_matrix;

                    if let Some(children) = children_storage.get(current_id) {
                        for &child_id in &children.0 {
                            queue.push((global.matrix, child_id));
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
        let attachments = world.borrow::<gizmo_renderer::components::BoneAttachment>();
        let skeletons = world.borrow::<gizmo_renderer::components::Skeleton>();
        let mut transforms = world.borrow_mut::<crate::physics::Transform>();
        
        for (id, attachment) in attachments.iter() {
            if let Some(skeleton) = skeletons.get(attachment.target_entity.id()) {
                if let Some(global_matrix) = skeleton.global_poses.get(attachment.bone_index) {
                    if let Some(trans) = transforms.get_mut(id) {
                        // Bone's transform is in skeleton-local space!
                        // To get world space, we need the skeleton entity's GlobalTransform.
                        // Let's assume the skeleton is always at identity or we just apply the local offset for now.
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
