use gizmo_physics::vehicle::VehicleController;
use ron::Value;

fn main() {
    let comp = VehicleController::new();
    
    // Step 1: to string
    let string_repr = ron::ser::to_string(&comp).unwrap();
    println!("String repr: {}", string_repr);
    
    // Step 2: to AST Value
    let val = ron::from_str::<Value>(&string_repr).unwrap();
    println!("AST Value: {:?}", val);
    
    // Step 3: back to Struct
    match val.clone().into_rust::<VehicleController>() {
        Ok(c) => println!("Success! engine_force: {:?}", c.engine_force),
        Err(e) => panic!("Failed to into_rust: {:?}", e),
    }
}
