use tobj;

fn main() {
    let (models, _) = tobj::load_obj(
        "demo/assets/suzanne.obj",
        &tobj::LoadOptions {
            single_index: true,
            triangulate: true,
            ignore_points: true,
            ignore_lines: true,
        },
    )
    .unwrap();
    let m = &models[0].mesh;
    println!("positions count: {}", m.positions.len() / 3);
    println!("indices count: {}", m.indices.len());
    println!("normals count: {}", m.normals.len() / 3);
    println!("First 6 indices:");
    for i in 0..6.min(m.indices.len()) {
        print!("{} ", m.indices[i]);
    }
    println!();
}
