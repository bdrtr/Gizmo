use gizmo_physics::shape::Collider;
use ron::Value;

fn main() {
    let comp = Collider::aabb(gizmo_math::Vec3::new(1.0, 1.0, 1.0));
    
    // Step 1: to string
    let string_repr = ron::ser::to_string(&comp).unwrap();
    println!("String repr: {}", string_repr);
    
    // Step 2: to AST Value
    let val = ron::from_str::<Value>(&string_repr).unwrap();
    println!("AST Value: {:?}", val);
    
    // Step 3: back to Struct
    match val.clone().into_rust::<Collider>() {
        Ok(c) => println!("Success! {:?}", c),
        Err(e) => panic!("Failed to into_rust: {:?}", e),
    }
}
