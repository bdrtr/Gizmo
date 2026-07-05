use super::*;

// Temporary workaround since we can't easily extract VBuf arrays from Arc<Buffer>
pub(super) fn alloc_sphere_verts(radius: f32, stacks: u32, slices: u32) -> Vec<Vertex> {
    let mut vertices = Vec::new();
    let pi = std::f32::consts::PI;

    for i in 0..stacks {
        let theta1 = (i as f32 / stacks as f32) * pi;
        let theta2 = ((i + 1) as f32 / stacks as f32) * pi;
        for j in 0..slices {
            let phi1 = (j as f32 / slices as f32) * 2.0 * pi;
            let phi2 = ((j + 1) as f32 / slices as f32) * 2.0 * pi;
            let p1 = [
                radius * theta1.sin() * phi1.cos(),
                radius * theta1.cos(),
                radius * theta1.sin() * phi1.sin(),
            ];
            let p2 = [
                radius * theta2.sin() * phi1.cos(),
                radius * theta2.cos(),
                radius * theta2.sin() * phi1.sin(),
            ];
            let p3 = [
                radius * theta2.sin() * phi2.cos(),
                radius * theta2.cos(),
                radius * theta2.sin() * phi2.sin(),
            ];
            let p4 = [
                radius * theta1.sin() * phi2.cos(),
                radius * theta1.cos(),
                radius * theta1.sin() * phi2.sin(),
            ];
            let n1 = [
                theta1.sin() * phi1.cos(),
                theta1.cos(),
                theta1.sin() * phi1.sin(),
            ];
            let n2 = [
                theta2.sin() * phi1.cos(),
                theta2.cos(),
                theta2.sin() * phi1.sin(),
            ];
            let n3 = [
                theta2.sin() * phi2.cos(),
                theta2.cos(),
                theta2.sin() * phi2.sin(),
            ];
            let n4 = [
                theta1.sin() * phi2.cos(),
                theta1.cos(),
                theta1.sin() * phi2.sin(),
            ];

            vertices.push(Vertex {
                position: p1,
                color: [1.0; 3],
                normal: n1,
                tex_coords: [0.0; 2],
                joint_indices: [0; 4],
                joint_weights: [0.0; 4],
                ..Default::default()
            });
            vertices.push(Vertex {
                position: p2,
                color: [1.0; 3],
                normal: n2,
                tex_coords: [0.0; 2],
                joint_indices: [0; 4],
                joint_weights: [0.0; 4],
                ..Default::default()
            });
            vertices.push(Vertex {
                position: p3,
                color: [1.0; 3],
                normal: n3,
                tex_coords: [0.0; 2],
                joint_indices: [0; 4],
                joint_weights: [0.0; 4],
                ..Default::default()
            });
            vertices.push(Vertex {
                position: p1,
                color: [1.0; 3],
                normal: n1,
                tex_coords: [0.0; 2],
                joint_indices: [0; 4],
                joint_weights: [0.0; 4],
                ..Default::default()
            });
            vertices.push(Vertex {
                position: p3,
                color: [1.0; 3],
                normal: n3,
                tex_coords: [0.0; 2],
                joint_indices: [0; 4],
                joint_weights: [0.0; 4],
                ..Default::default()
            });
            vertices.push(Vertex {
                position: p4,
                color: [1.0; 3],
                normal: n4,
                tex_coords: [0.0; 2],
                joint_indices: [0; 4],
                joint_weights: [0.0; 4],
                ..Default::default()
            });
        }
    }
    vertices
}
