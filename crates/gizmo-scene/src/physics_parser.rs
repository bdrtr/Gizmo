use gizmo_physics::components::{ColliderShape, TriMeshShape, ConvexHullShape};
use gizmo_renderer::components::Mesh;
// use gizmo_math::Vec3;

/// Mesh verisinden fiziksel bir ColliderShape çıkartır.
pub fn create_collider_from_mesh(mesh: &Mesh, use_convex_hull: bool) -> ColliderShape {
    let vertices = (*mesh.cpu_vertices).clone();
    
    // Eğer vertex verisi çok büyükse ve AABB isteniyorsa bunu optimize edebiliriz.
    // Ancak tam form isteniyorsa (TriMesh veya ConvexHull):
    if use_convex_hull {
        // İleride QuickHull algoritması ile gerçek bir Convex Hull oluşturulabilir.
        // Şimdilik sadece vertex dizisini aktarıyoruz (gizmo-physics NarrowPhase stub olarak SAT implementasyonunu bekler)
        ColliderShape::ConvexHull(ConvexHullShape {
            vertices,
        })
    } else {
        // TriMesh (Tüm vertex ve indexler)
        let mut indices = Vec::with_capacity(vertices.len());
        for i in 0..vertices.len() {
            indices.push(i as u32);
        }
        
        let bvh = gizmo_physics::bvh::BvhTree::build(&vertices, &mut indices);
        
        ColliderShape::TriMesh(TriMeshShape {
            vertices,
            indices,
            bvh,
        })
    }
}

/// Tüm mesh barındıran ancak Collider barındırmayan objelere otomatik ConvexHull/TriMesh ekler.
pub fn auto_generate_colliders(world: &mut gizmo_core::World, use_convex: bool) {
    let mut missing_colliders = Vec::new();
    
    // Collider'ı olmayan Mesh'leri bul
    if let Some(mesh_q) = world.query::<&Mesh>() {
        let colliders = world.borrow::<gizmo_physics::components::Collider>();
        for (e, mesh) in mesh_q.iter() {
            if colliders.get(e).is_none() {
                missing_colliders.push((e, mesh.clone()));
            }
        }
    }
    
    let count = missing_colliders.len();
    for (e, mesh) in missing_colliders {
        let entity = world.get_entity(e).unwrap();
        let shape = create_collider_from_mesh(&mesh, use_convex);
        let collider = gizmo_physics::components::Collider {
            shape,
            is_trigger: false,
            material: Default::default(),
            collision_layer: Default::default(),
        };
        
        // Eğer RigidBody yoksa, statik olarak ekleyelim (Zemin / Çevre objesi gibi davranması için)
        if world.borrow::<gizmo_physics::components::RigidBody>().get(e).is_none() {
            let mut rb = gizmo_physics::components::RigidBody::new_static();
            rb.update_inertia_from_collider(&collider);
            world.add_component(entity, rb);
        }
        
        world.add_component(entity, collider);
    }
    
    println!("[PhysicsParser] {} adet mesh'e otomatik Collider üretildi.", count);
}
