use gizmo::math::{Vec3, Quat};
use gizmo::physics::narrowphase::NarrowPhase;
use gizmo::physics::components::{ColliderShape, BoxShape};

fn main() {
    let shape_ground = ColliderShape::Box(BoxShape { half_extents: Vec3::new(100.0, 0.05, 100.0) });
    let pos_ground = Vec3::new(0.0, 0.0, 0.0); 
    
    let shape_box = ColliderShape::Box(BoxShape { half_extents: Vec3::new(0.5, 0.5, 0.5) });
    let pos_box = Vec3::new(0.0, 0.8, 0.0); // Penetrating by 0.2
    
    let contact = NarrowPhase::test_collision(&shape_ground, pos_ground, Quat::IDENTITY, &shape_box, pos_box, Quat::IDENTITY);
    println!("Contact 0.8: {:?}", contact);
    
    let pos_box2 = Vec3::new(0.0, 0.5, 0.0); // Penetrating by 0.5
    let contact2 = NarrowPhase::test_collision(&shape_ground, pos_ground, Quat::IDENTITY, &shape_box, pos_box2, Quat::IDENTITY);
    println!("Contact 0.5: {:?}", contact2);
}
