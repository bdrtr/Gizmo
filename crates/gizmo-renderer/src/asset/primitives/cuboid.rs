use crate::components::Mesh;
use crate::renderer::Vertex;
use gizmo_math::Vec3;
use std::sync::Arc;
use wgpu::util::DeviceExt;

impl crate::asset::AssetManager {
    /// İçi boş ters yüzlü küp (Skybox) mesh üretir.
    /// Normaller içe bakar, böylece kamera küpün merkezinden dışarıya baktığında yüzeyler görünür.
    pub fn create_inverted_cube(device: &wgpu::Device) -> Mesh {
        // 6 yüz × 2 üçgen × 3 köşe = 36 vertex
        // Her yüzün normali İÇE bakar (ters küp)
        let positions: [[f32; 3]; 8] = [
            [-1.0, -1.0, -1.0], // 0
            [1.0, -1.0, -1.0],  // 1
            [1.0, 1.0, -1.0],   // 2
            [-1.0, 1.0, -1.0],  // 3
            [-1.0, -1.0, 1.0],  // 4
            [1.0, -1.0, 1.0],   // 5
            [1.0, 1.0, 1.0],    // 6
            [-1.0, 1.0, 1.0],   // 7
        ];

        struct FaceDef {
            indices: [usize; 6],
            normal: [f32; 3],
            uvs: [[f32; 2]; 6],
        }

        let faces: [FaceDef; 6] = [
            // Arka (+Z içe)
            FaceDef {
                indices: [0, 1, 2, 0, 2, 3],
                normal: [0.0, 0.0, 1.0],
                uvs: [
                    [1.0, 1.0],
                    [0.0, 1.0],
                    [0.0, 0.0],
                    [1.0, 1.0],
                    [0.0, 0.0],
                    [1.0, 0.0],
                ],
            },
            // Ön (-Z içe)
            FaceDef {
                indices: [4, 6, 5, 4, 7, 6],
                normal: [0.0, 0.0, -1.0],
                uvs: [
                    [0.0, 1.0],
                    [1.0, 0.0],
                    [1.0, 1.0],
                    [0.0, 1.0],
                    [0.0, 0.0],
                    [1.0, 0.0],
                ],
            },
            // Alt (+Y içe)
            FaceDef {
                indices: [0, 5, 1, 0, 4, 5],
                normal: [0.0, 1.0, 0.0],
                uvs: [
                    [0.0, 0.0],
                    [1.0, 1.0],
                    [1.0, 0.0],
                    [0.0, 0.0],
                    [0.0, 1.0],
                    [1.0, 1.0],
                ],
            },
            // Üst (-Y içe)
            FaceDef {
                indices: [3, 2, 6, 3, 6, 7],
                normal: [0.0, -1.0, 0.0],
                uvs: [
                    [0.0, 0.0],
                    [1.0, 0.0],
                    [1.0, 1.0],
                    [0.0, 0.0],
                    [1.0, 1.0],
                    [0.0, 1.0],
                ],
            },
            // Sol (+X içe)
            FaceDef {
                indices: [0, 3, 7, 0, 7, 4],
                normal: [1.0, 0.0, 0.0],
                uvs: [
                    [0.0, 1.0],
                    [0.0, 0.0],
                    [1.0, 0.0],
                    [0.0, 1.0],
                    [1.0, 0.0],
                    [1.0, 1.0],
                ],
            },
            // Sağ (-X içe)
            FaceDef {
                indices: [1, 6, 2, 1, 5, 6],
                normal: [-1.0, 0.0, 0.0],
                uvs: [
                    [1.0, 1.0],
                    [0.0, 0.0],
                    [1.0, 0.0],
                    [1.0, 1.0],
                    [0.0, 1.0],
                    [0.0, 0.0],
                ],
            },
        ];

        let mut vertices = Vec::with_capacity(36);
        for face in &faces {
            for i in 0..6 {
                vertices.push(Vertex {
                    position: positions[face.indices[i]],
                    color: [1.0, 1.0, 1.0],
                    normal: face.normal,
                    tex_coords: face.uvs[i],
                    joint_indices: [0; 4],
                    joint_weights: [0.0; 4], ..Default::default()
                });
            }
        }

        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Skybox Inverted Cube VBuf"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Mesh::new(
            device,
            Arc::new(vbuf),
            &vertices,
            Vec3::ZERO,
            "inverted_cube".to_string(),
        )
    }

    /// Düzenli Küp mesh üretir (Dışa bakan normaller, PBR ışıklandırma ve gölgelendirme için doğru)
    pub fn create_cube(device: &wgpu::Device) -> Mesh {
        let positions: [[f32; 3]; 8] = [
            [-1.0, -1.0, -1.0], // 0
            [1.0, -1.0, -1.0],  // 1
            [1.0, 1.0, -1.0],   // 2
            [-1.0, 1.0, -1.0],  // 3
            [-1.0, -1.0, 1.0],  // 4
            [1.0, -1.0, 1.0],   // 5
            [1.0, 1.0, 1.0],    // 6
            [-1.0, 1.0, 1.0],   // 7
        ];

        struct FaceDef {
            indices: [usize; 6],
            normal: [f32; 3],
            uvs: [[f32; 2]; 6],
        }

        let faces: [FaceDef; 6] = [
            // Arka (-Z)
            FaceDef {
                indices: [1, 0, 3, 1, 3, 2],
                normal: [0.0, 0.0, -1.0],
                uvs: [
                    [0.0, 1.0],
                    [1.0, 1.0],
                    [1.0, 0.0],
                    [0.0, 1.0],
                    [1.0, 0.0],
                    [0.0, 0.0],
                ],
            },
            // Ön (+Z)
            FaceDef {
                indices: [4, 5, 6, 4, 6, 7],
                normal: [0.0, 0.0, 1.0],
                uvs: [
                    [0.0, 1.0],
                    [1.0, 1.0],
                    [1.0, 0.0],
                    [0.0, 1.0],
                    [1.0, 0.0],
                    [0.0, 0.0],
                ],
            },
            // Alt (-Y)
            FaceDef {
                indices: [0, 1, 5, 0, 5, 4],
                normal: [0.0, -1.0, 0.0],
                uvs: [
                    [0.0, 0.0],
                    [1.0, 0.0],
                    [1.0, 1.0],
                    [0.0, 0.0],
                    [1.0, 1.0],
                    [0.0, 1.0],
                ],
            },
            // Üst (+Y)
            FaceDef {
                indices: [7, 6, 2, 7, 2, 3],
                normal: [0.0, 1.0, 0.0],
                uvs: [
                    [0.0, 1.0],
                    [1.0, 1.0],
                    [1.0, 0.0],
                    [0.0, 1.0],
                    [1.0, 0.0],
                    [0.0, 0.0],
                ],
            },
            // Sol (-X)
            FaceDef {
                indices: [0, 4, 7, 0, 7, 3],
                normal: [-1.0, 0.0, 0.0],
                uvs: [
                    [0.0, 1.0],
                    [1.0, 1.0],
                    [1.0, 0.0],
                    [0.0, 1.0],
                    [1.0, 0.0],
                    [0.0, 0.0],
                ],
            },
            // Sağ (+X)
            FaceDef {
                indices: [5, 1, 2, 5, 2, 6],
                normal: [1.0, 0.0, 0.0],
                uvs: [
                    [0.0, 1.0],
                    [1.0, 1.0],
                    [1.0, 0.0],
                    [0.0, 1.0],
                    [1.0, 0.0],
                    [0.0, 0.0],
                ],
            },
        ];

        let mut vertices = Vec::with_capacity(36);
        for face in &faces {
            for i in 0..6 {
                vertices.push(Vertex {
                    position: positions[face.indices[i]],
                    color: [1.0, 1.0, 1.0],
                    normal: face.normal,
                    tex_coords: face.uvs[i],
                    joint_indices: [0; 4],
                    joint_weights: [0.0; 4], ..Default::default()
                });
            }
        }

        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Standard Cube VBuf"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Mesh::new(
            device,
            Arc::new(vbuf),
            &vertices,
            Vec3::ZERO,
            "standard_cube".to_string(),
        )
    }

    pub fn create_gizmo_arrow(device: &wgpu::Device) -> Mesh {
        let w = 0.03; // Shaft thickness
        let hw = 0.12; // Head width
        let sl = 0.8; // Shaft length

        let positions: [[f32; 3]; 13] = [
            // Shaft (0..8)
            [-w, 0.0, -w],
            [w, 0.0, -w],
            [w, sl, -w],
            [-w, sl, -w],
            [-w, 0.0, w],
            [w, 0.0, w],
            [w, sl, w],
            [-w, sl, w],
            // Head Base (8..12)
            [-hw, sl, -hw],
            [hw, sl, -hw],
            [hw, sl, hw],
            [-hw, sl, hw],
            // Apex (12)
            [0.0, 1.0, 0.0],
        ];

        let head_dy = 1.0 - sl;
        let head_dxz = hw;
        let head_norm_len = (head_dy * head_dy + head_dxz * head_dxz).sqrt();
        let n_y = head_dxz / head_norm_len;
        let n_xz = head_dy / head_norm_len;

        // Tuple of (Indices, Normal)
        let faces: Vec<(Vec<usize>, [f32; 3])> = vec![
            // Shaft
            (vec![0, 2, 1, 0, 3, 2], [0.0, 0.0, -1.0]), // Back
            (vec![4, 5, 6, 4, 6, 7], [0.0, 0.0, 1.0]),  // Front
            (vec![0, 1, 5, 0, 5, 4], [0.0, -1.0, 0.0]), // Bottom
            (vec![0, 4, 7, 0, 7, 3], [-1.0, 0.0, 0.0]), // Left
            (vec![1, 2, 6, 1, 6, 5], [1.0, 0.0, 0.0]),  // Right
            // Arrowhead Base
            (vec![8, 9, 10, 8, 10, 11], [0.0, -1.0, 0.0]),
            // Arrowhead Sides
            (vec![11, 10, 12], [0.0, n_y, n_xz]), // Front (+Z)
            (vec![9, 8, 12], [0.0, n_y, -n_xz]),  // Back (-Z)
            (vec![10, 9, 12], [n_xz, n_y, 0.0]),  // Right (+X)
            (vec![8, 11, 12], [-n_xz, n_y, 0.0]), // Left (-X)
        ];

        let mut vertices = Vec::new();
        for (indices, normal) in faces {
            for idx in indices {
                vertices.push(Vertex {
                    position: positions[idx],
                    color: [1.0, 1.0, 1.0],
                    normal,
                    tex_coords: [0.0, 0.0],
                    joint_indices: [0; 4],
                    joint_weights: [0.0; 4], ..Default::default()
                });
            }
        }

        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Gizmo Arrow VBuf"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Mesh::new(
            device,
            Arc::new(vbuf),
            &vertices,
            Vec3::ZERO,
            "gizmo_arrow".to_string(),
        )
    }
}
