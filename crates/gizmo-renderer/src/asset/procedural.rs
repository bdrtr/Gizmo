use crate::components::Mesh;
use crate::renderer::Vertex;
use gizmo_math::Vec3;
use std::sync::Arc;
use wgpu::util::DeviceExt;

impl super::AssetManager {
    pub fn create_tetrahedron(device: &wgpu::Device, size: f32) -> Mesh {
        let s = size;
        let p0 = [s, s, s];
        let p1 = [-s, -s, s];
        let p2 = [-s, s, -s];
        let p3 = [s, -s, -s];

        let mut vertices = Vec::new();
        let faces = [
            (p0, p1, p2),
            (p0, p2, p3),
            (p0, p3, p1),
            (p1, p3, p2),
        ];

        let def_j = [0; 4];
        let def_w = [0.0; 4];

        for (a, b, c) in faces {
            let va = Vec3::from_array(a);
            let vb = Vec3::from_array(b);
            let vc = Vec3::from_array(c);
            let n = (vc - va).cross(vb - va).normalize();
            let normal = [n.x, n.y, n.z];

            vertices.push(Vertex { position: a, color: [1.0; 3], normal, tex_coords: [0.0, 0.0], joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            vertices.push(Vertex { position: b, color: [1.0; 3], normal, tex_coords: [1.0, 0.0], joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            vertices.push(Vertex { position: c, color: [1.0; 3], normal, tex_coords: [0.5, 1.0], joint_indices: def_j, joint_weights: def_w, ..Default::default() });
        }

        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Tetrahedron VBuf"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Mesh::new(device, Arc::new(vbuf), &vertices, Vec3::ZERO, "tetrahedron".to_string())
    }

    pub fn create_conical_frustum(device: &wgpu::Device, radius_bottom: f32, radius_top: f32, height: f32, radial_segments: u32) -> Mesh {
        let radial_segments = radial_segments.max(3);
        let mut vertices = Vec::new();
        let pi = std::f32::consts::PI;
        let half_h = height / 2.0;

        let def_j = [0; 4]; let def_w = [0.0; 4];
        let col = [1.0; 3];

        let y_normal = (radius_bottom - radius_top) / height;

        for i in 0..radial_segments {
            let t1 = (i as f32 / radial_segments as f32) * 2.0 * pi;
            let t2 = ((i + 1) as f32 / radial_segments as f32) * 2.0 * pi;

            let u1 = i as f32 / radial_segments as f32;
            let u2 = (i + 1) as f32 / radial_segments as f32;

            let p1_top = [radius_top * t1.cos(), half_h, radius_top * t1.sin()];
            let p1_bot = [radius_bottom * t1.cos(), -half_h, radius_bottom * t1.sin()];
            let p2_top = [radius_top * t2.cos(), half_h, radius_top * t2.sin()];
            let p2_bot = [radius_bottom * t2.cos(), -half_h, radius_bottom * t2.sin()];

            let n1 = Vec3::new(t1.cos(), y_normal, t1.sin()).normalize();
            let n2 = Vec3::new(t2.cos(), y_normal, t2.sin()).normalize();
            let n1_arr = [n1.x, n1.y, n1.z];
            let n2_arr = [n2.x, n2.y, n2.z];

            // Sides (CCW from outside)
            vertices.push(Vertex { position: p1_top, normal: n1_arr, tex_coords: [u1, 0.0], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            vertices.push(Vertex { position: p1_bot, normal: n1_arr, tex_coords: [u1, 1.0], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            vertices.push(Vertex { position: p2_bot, normal: n2_arr, tex_coords: [u2, 1.0], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });

            vertices.push(Vertex { position: p1_top, normal: n1_arr, tex_coords: [u1, 0.0], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            vertices.push(Vertex { position: p2_bot, normal: n2_arr, tex_coords: [u2, 1.0], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            vertices.push(Vertex { position: p2_top, normal: n2_arr, tex_coords: [u2, 0.0], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });

            // Top Cap (CCW from above)
            vertices.push(Vertex { position: [0.0, half_h, 0.0], normal: [0.0, 1.0, 0.0], tex_coords: [0.5, 0.5], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            vertices.push(Vertex { position: p1_top, normal: [0.0, 1.0, 0.0], tex_coords: [0.5 + 0.5 * t1.cos(), 0.5 + 0.5 * t1.sin()], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            vertices.push(Vertex { position: p2_top, normal: [0.0, 1.0, 0.0], tex_coords: [0.5 + 0.5 * t2.cos(), 0.5 + 0.5 * t2.sin()], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });

            // Bottom Cap (CCW from below)
            vertices.push(Vertex { position: [0.0, -half_h, 0.0], normal: [0.0, -1.0, 0.0], tex_coords: [0.5, 0.5], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            vertices.push(Vertex { position: p2_bot, normal: [0.0, -1.0, 0.0], tex_coords: [0.5 + 0.5 * t2.cos(), 0.5 + 0.5 * t2.sin()], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            vertices.push(Vertex { position: p1_bot, normal: [0.0, -1.0, 0.0], tex_coords: [0.5 + 0.5 * t1.cos(), 0.5 + 0.5 * t1.sin()], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
        }

        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Conical Frustum VBuf"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Mesh::new(device, Arc::new(vbuf), &vertices, Vec3::ZERO, "conical_frustum".to_string())
    }

    pub fn create_convex_extrusion(device: &wgpu::Device, points_2d: &[[f32; 2]], depth: f32) -> Mesh {
        let mut vertices = Vec::new();
        let half_d = depth / 2.0;
        let def_j = [0; 4]; let def_w = [0.0; 4];
        let col = [1.0; 3];

        let count = points_2d.len();
        
        let mut cx = 0.0;
        let mut cy = 0.0;
        for p in points_2d {
            cx += p[0];
            cy += p[1];
        }
        cx /= count as f32;
        cy /= count as f32;

        for i in 0..count {
            let p1 = points_2d[i];
            let p2 = points_2d[(i + 1) % count];

            let dx = p2[0] - p1[0];
            let dy = p2[1] - p1[1];
            let len = (dx*dx + dy*dy).sqrt();
            let nx = dy / len;
            let ny = -dx / len;
            let normal = [nx, 0.0, ny];

            let v1_top = [p1[0], half_d, p1[1]];
            let v1_bot = [p1[0], -half_d, p1[1]];
            let v2_top = [p2[0], half_d, p2[1]];
            let v2_bot = [p2[0], -half_d, p2[1]];

            // Side (CCW from outside)
            vertices.push(Vertex { position: v1_top, normal, tex_coords: [0.0, 0.0], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            vertices.push(Vertex { position: v1_bot, normal, tex_coords: [0.0, 1.0], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            vertices.push(Vertex { position: v2_bot, normal, tex_coords: [1.0, 1.0], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });

            vertices.push(Vertex { position: v1_top, normal, tex_coords: [0.0, 0.0], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            vertices.push(Vertex { position: v2_bot, normal, tex_coords: [1.0, 1.0], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            vertices.push(Vertex { position: v2_top, normal, tex_coords: [1.0, 0.0], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });

            // Top Cap (Triangulate via center, CCW from above)
            vertices.push(Vertex { position: [cx, half_d, cy], normal: [0.0, 1.0, 0.0], tex_coords: [0.5, 0.5], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            vertices.push(Vertex { position: v1_top, normal: [0.0, 1.0, 0.0], tex_coords: [0.5 + p1[0], 0.5 + p1[1]], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            vertices.push(Vertex { position: v2_top, normal: [0.0, 1.0, 0.0], tex_coords: [0.5 + p2[0], 0.5 + p2[1]], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });

            // Bottom Cap (CCW from below)
            vertices.push(Vertex { position: [cx, -half_d, cy], normal: [0.0, -1.0, 0.0], tex_coords: [0.5, 0.5], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            vertices.push(Vertex { position: v2_bot, normal: [0.0, -1.0, 0.0], tex_coords: [0.5 + p2[0], 0.5 + p2[1]], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            vertices.push(Vertex { position: v1_bot, normal: [0.0, -1.0, 0.0], tex_coords: [0.5 + p1[0], 0.5 + p1[1]], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
        }

        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Extrusion VBuf"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Mesh::new(device, Arc::new(vbuf), &vertices, Vec3::ZERO, "extrusion".to_string())
    }

    pub fn create_ring_extrusion(device: &wgpu::Device, inner_points: &[[f32; 2]], outer_points: &[[f32; 2]], depth: f32) -> Mesh {
        let mut vertices = Vec::new();
        let half_d = depth / 2.0;
        let def_j = [0; 4]; let def_w = [0.0; 4];
        let col = [1.0; 3];

        let count = outer_points.len();

        for i in 0..count {
            let o1 = outer_points[i];
            let o2 = outer_points[(i + 1) % count];
            let i1 = inner_points[i];
            let i2 = inner_points[(i + 1) % count];

            // Outer side normal
            let dx_o = o2[0] - o1[0]; let dy_o = o2[1] - o1[1];
            let len_o = (dx_o*dx_o + dy_o*dy_o).sqrt();
            let nx_o = dy_o / len_o; let ny_o = -dx_o / len_o;
            let normal_o = [nx_o, 0.0, ny_o];

            // Inner side normal (flipped)
            let dx_i = i2[0] - i1[0]; let dy_i = i2[1] - i1[1];
            let len_i = (dx_i*dx_i + dy_i*dy_i).sqrt();
            let nx_i = -dy_i / len_i; let ny_i = dx_i / len_i;
            let normal_i = [nx_i, 0.0, ny_i];

            let v_o1_t = [o1[0], half_d, o1[1]]; let v_o1_b = [o1[0], -half_d, o1[1]];
            let v_o2_t = [o2[0], half_d, o2[1]]; let v_o2_b = [o2[0], -half_d, o2[1]];
            
            let v_i1_t = [i1[0], half_d, i1[1]]; let v_i1_b = [i1[0], -half_d, i1[1]];
            let v_i2_t = [i2[0], half_d, i2[1]]; let v_i2_b = [i2[0], -half_d, i2[1]];

            // Outer Side (CCW from outside)
            vertices.push(Vertex { position: v_o1_t, normal: normal_o, tex_coords: [0.0, 0.0], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            vertices.push(Vertex { position: v_o1_b, normal: normal_o, tex_coords: [0.0, 1.0], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            vertices.push(Vertex { position: v_o2_b, normal: normal_o, tex_coords: [1.0, 1.0], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            
            vertices.push(Vertex { position: v_o1_t, normal: normal_o, tex_coords: [0.0, 0.0], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            vertices.push(Vertex { position: v_o2_b, normal: normal_o, tex_coords: [1.0, 1.0], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            vertices.push(Vertex { position: v_o2_t, normal: normal_o, tex_coords: [1.0, 0.0], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });

            // Inner Side (CCW from outside)
            vertices.push(Vertex { position: v_i2_t, normal: normal_i, tex_coords: [0.0, 0.0], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            vertices.push(Vertex { position: v_i2_b, normal: normal_i, tex_coords: [0.0, 1.0], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            vertices.push(Vertex { position: v_i1_b, normal: normal_i, tex_coords: [1.0, 1.0], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            
            vertices.push(Vertex { position: v_i2_t, normal: normal_i, tex_coords: [0.0, 0.0], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            vertices.push(Vertex { position: v_i1_b, normal: normal_i, tex_coords: [1.0, 1.0], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            vertices.push(Vertex { position: v_i1_t, normal: normal_i, tex_coords: [1.0, 0.0], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });

            // Top Cap Quad (o1, o2, i1, i2 - CCW from above)
            let n_top = [0.0, 1.0, 0.0];
            vertices.push(Vertex { position: v_o1_t, normal: n_top, tex_coords: [o1[0], o1[1]], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            vertices.push(Vertex { position: v_i1_t, normal: n_top, tex_coords: [i1[0], i1[1]], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            vertices.push(Vertex { position: v_o2_t, normal: n_top, tex_coords: [o2[0], o2[1]], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            
            vertices.push(Vertex { position: v_o2_t, normal: n_top, tex_coords: [o2[0], o2[1]], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            vertices.push(Vertex { position: v_i1_t, normal: n_top, tex_coords: [i1[0], i1[1]], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            vertices.push(Vertex { position: v_i2_t, normal: n_top, tex_coords: [i2[0], i2[1]], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });

            // Bottom Cap Quad (o1, o2, i1, i2 - CCW from below)
            let n_bot = [0.0, -1.0, 0.0];
            vertices.push(Vertex { position: v_o1_b, normal: n_bot, tex_coords: [o1[0], o1[1]], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            vertices.push(Vertex { position: v_o2_b, normal: n_bot, tex_coords: [o2[0], o2[1]], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            vertices.push(Vertex { position: v_i1_b, normal: n_bot, tex_coords: [i1[0], i1[1]], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            
            vertices.push(Vertex { position: v_o2_b, normal: n_bot, tex_coords: [o2[0], o2[1]], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            vertices.push(Vertex { position: v_i2_b, normal: n_bot, tex_coords: [i2[0], i2[1]], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            vertices.push(Vertex { position: v_i1_b, normal: n_bot, tex_coords: [i1[0], i1[1]], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
        }

        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Ring Extrusion VBuf"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Mesh::new(device, Arc::new(vbuf), &vertices, Vec3::ZERO, "ring_extrusion".to_string())
    }
}
