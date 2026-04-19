use gizmo_physics::components::Transform;

fn main() {
    let t = Transform::new(gizmo_math::Vec3::new(1., 2., 3.));
    println!("{}", ron::ser::to_string(&t).unwrap());
}
