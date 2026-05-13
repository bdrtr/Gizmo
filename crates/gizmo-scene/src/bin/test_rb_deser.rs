use gizmo_physics::components::RigidBody;
use ron::Value;

fn main() {
    let comp = RigidBody::new(1.0, 0.5, 0.5, true);

    // Step 1: to string
    let string_repr = ron::ser::to_string(&comp).unwrap();
    tracing::info!("String repr: {}", string_repr);

    // Step 2: to AST Value
    let val = ron::from_str::<Value>(&string_repr).unwrap();
    tracing::info!("AST Value: {:?}", val);

    // Step 3: back to Struct
    match val.clone().into_rust::<RigidBody>() {
        Ok(_) => tracing::info!("Success!"),
        Err(e) => panic!("Failed to into_rust: {:?}", e),
    }
}
