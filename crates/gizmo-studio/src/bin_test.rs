use gizmo_physics::shape::*;

fn main() {
    let shape = ColliderShape::Aabb(Aabb { half_extents: gizmo_math::Vec3::new(1., 1., 1.) });
    match ron::ser::to_string(&shape) {
        Ok(s) => println!("Success: {}", s),
        Err(e) => println!("Error: {}", e),
    }
}
