use gizmo_math::{Vec3, Quat};
use gizmo_physics::narrowphase::NarrowPhase;
use gizmo_physics::components::{ColliderShape, BoxShape};

fn main() {
    let shape_a = ColliderShape::Box(BoxShape { half_extents: Vec3::new(10.0, 0.5, 10.0) });
    let pos_a = Vec3::new(0.0, 0.0, 0.0);
    
    let shape_b = ColliderShape::Box(BoxShape { half_extents: Vec3::new(0.5, 0.5, 0.5) });
    let pos_b = Vec3::new(0.0, 0.9, 0.0); // Penetrating by 0.1
    
    let contact = NarrowPhase::test_collision(&shape_a, pos_a, Quat::IDENTITY, &shape_b, pos_b, Quat::IDENTITY);
    println!("Contact: {:?}", contact);
}
