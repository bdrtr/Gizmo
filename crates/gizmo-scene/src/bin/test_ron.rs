use gizmo_renderer::components::light::{DirectionalLight, LightRole};
use ron::Value;

fn main() {
    let role = LightRole::Sun;
    let string_repr = ron::ser::to_string(&role).unwrap();
    println!("Serialized directly: {}", string_repr);
    
    let value: Value = ron::from_str(&string_repr).unwrap();
    println!("AST: {:?}", value);
    
    let string_from_value = ron::ser::to_string(&value).unwrap();
    println!("String from AST: {}", string_from_value);

    let res: Result<LightRole, _> = ron::from_str(&string_from_value);
    match res {
        Ok(_) => println!("Success via String!"),
        Err(e) => println!("Error via String: {:?}", e),
    }
}
