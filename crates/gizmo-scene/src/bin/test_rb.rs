use gizmo_physics::components::RigidBody;

fn main() {
    let comp = RigidBody::new(1.0, 0.5, 0.5, true);

    match ron::ser::to_string(&comp) {
        Ok(str) => println!("Success! {}", str),
        Err(e) => println!("FAIL! {:?}", e),
    }
}
