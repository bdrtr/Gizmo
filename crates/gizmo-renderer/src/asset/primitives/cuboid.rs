use crate::components::Mesh;
use crate::renderer::Vertex;
use gizmo_math::Vec3;

impl crate::asset::AssetManager {
    /// Builds a hollow inward-facing cube mesh (a skybox).
    /// The normals point inwards, so the faces are visible with the camera at the cube's centre
    /// looking out.
    pub fn create_inverted_cube(device: &wgpu::Device) -> Mesh {
        Mesh::new_indexed(device, &Self::inverted_cube_data(), Vec3::ZERO, "inverted_cube".to_string())
    }

    /// [`create_inverted_cube`](Self::create_inverted_cube)'s triangle list.
    pub(crate) fn inverted_cube_data() -> Vec<Vertex> {
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
                    color: [1.0, 1.0, 1.0, 1.0],
                    normal: face.normal,
                    tex_coords: face.uvs[i],
                    joint_indices: [0; 4],
                    joint_weights: [0.0; 4], ..Default::default()
                });
            }
        }

        vertices
    }

    /// Builds an ordinary cube mesh (outward-facing normals, correct for PBR lighting and
    /// shading).
    pub fn create_cube(device: &wgpu::Device) -> Mesh {
        Mesh::new_indexed(device, &Self::cube_data(), Vec3::ZERO, "standard_cube".to_string())
    }

    /// [`create_cube`](Self::create_cube)'s triangle list, ±1 on every axis.
    pub(crate) fn cube_data() -> Vec<Vertex> {
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
                    color: [1.0, 1.0, 1.0, 1.0],
                    normal: face.normal,
                    tex_coords: face.uvs[i],
                    joint_indices: [0; 4],
                    joint_weights: [0.0; 4], ..Default::default()
                });
            }
        }

        vertices
    }
}
