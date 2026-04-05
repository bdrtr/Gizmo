use yelbegen::math::{Vec3, Quat};
use yelbegen::physics::gjk::gjk_intersect;
use yelbegen::physics::shape::{ColliderShape, Sphere, Aabb};

fn main() {
    let ground = ColliderShape::Aabb(Aabb { half_extents: Vec3::new(25.0, 1.0, 25.0) });
    let ground_pos = Vec3::new(0.0, -1.0, 0.0);
    let ground_rot = Quat::IDENTITY;

    let sphere = ColliderShape::Sphere(Sphere { radius: 1.5 });
    let sphere_pos = Vec3::new(0.0, 0.5, 0.0);
    let sphere_rot = Quat::IDENTITY;


    let (is_colliding, simplex) = gjk_intersect(&ground, ground_pos, ground_rot, &sphere, sphere_pos, sphere_rot);

    println!("GJK Result: {}", is_colliding);
    println!("Simplex size: {}", simplex.size);
    for i in 0..simplex.size {
        println!("Point {}: {:?}", i, simplex.points[i]);
    }
}
