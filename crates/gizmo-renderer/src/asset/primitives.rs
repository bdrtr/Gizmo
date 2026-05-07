use crate::components::Mesh;
use crate::renderer::Vertex;
use gizmo_math::Vec3;
use std::sync::Arc;
use wgpu::util::DeviceExt;

impl super::AssetManager {
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
                    joint_weights: [0.0; 4],
                });
            }
        }

        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Skybox Inverted Cube VBuf"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Mesh::new(device, 
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

        // Her yüz: 6 vertex indeksi, normal, ve 6 UV koordinatı
        // UV'ler her üçgen için sırasıyla: tri1(v0,v1,v2), tri2(v3,v4,v5)
        struct FaceDef {
            indices: [usize; 6],
            normal: [f32; 3],
            uvs: [[f32; 2]; 6],
        }

        let faces: [FaceDef; 6] = [
            // Arka (-Z)
            FaceDef {
                indices: [0, 2, 1, 0, 3, 2],
                normal: [0.0, 0.0, -1.0],
                uvs: [
                    [1.0, 1.0],
                    [0.0, 0.0],
                    [0.0, 1.0],
                    [1.0, 1.0],
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
                indices: [3, 6, 2, 3, 7, 6],
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
                indices: [1, 2, 6, 1, 6, 5],
                normal: [1.0, 0.0, 0.0],
                uvs: [
                    [1.0, 1.0],
                    [1.0, 0.0],
                    [0.0, 0.0],
                    [1.0, 1.0],
                    [0.0, 0.0],
                    [0.0, 1.0],
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
                    joint_weights: [0.0; 4],
                });
            }
        }

        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Standard Cube VBuf"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Mesh::new(device, 
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
                    joint_weights: [0.0; 4],
                });
            }
        }

        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Gizmo Arrow VBuf"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Mesh::new(device, 
            Arc::new(vbuf),
            &vertices,
            Vec3::ZERO,
            "gizmo_arrow".to_string(),
        )
    }

    /// Basit, yatay bir düzlem (Plane) üretir.
    pub fn create_plane(device: &wgpu::Device, size: f32) -> Mesh {
        let half = size / 2.0;
        let y = 0.0;

        // Üstten bakışla Saat yönünün tersi (CCW) 2 üçgen (Quad)
        let def_j = [0; 4];
        let def_w = [0.0; 4];
        let vertices = [
            // İlk Üçgen (CCW from above)
            Vertex {
                position: [-half, y, -half],
                color: [1.0, 1.0, 1.0],
                normal: [0.0, 1.0, 0.0],
                tex_coords: [0.0, 0.0],
                joint_indices: def_j,
                joint_weights: def_w,
            },
            Vertex {
                position: [half, y, half],
                color: [1.0, 1.0, 1.0],
                normal: [0.0, 1.0, 0.0],
                tex_coords: [size, size],
                joint_indices: def_j,
                joint_weights: def_w,
            },
            Vertex {
                position: [half, y, -half],
                color: [1.0, 1.0, 1.0],
                normal: [0.0, 1.0, 0.0],
                tex_coords: [size, 0.0],
                joint_indices: def_j,
                joint_weights: def_w,
            },
            // İkinci Üçgen (CCW from above)
            Vertex {
                position: [-half, y, -half],
                color: [1.0, 1.0, 1.0],
                normal: [0.0, 1.0, 0.0],
                tex_coords: [0.0, 0.0],
                joint_indices: def_j,
                joint_weights: def_w,
            },
            Vertex {
                position: [-half, y, half],
                color: [1.0, 1.0, 1.0],
                normal: [0.0, 1.0, 0.0],
                tex_coords: [0.0, size],
                joint_indices: def_j,
                joint_weights: def_w,
            },
            Vertex {
                position: [half, y, half],
                color: [1.0, 1.0, 1.0],
                normal: [0.0, 1.0, 0.0],
                tex_coords: [size, size],
                joint_indices: def_j,
                joint_weights: def_w,
            },
        ];

        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Plane VBuf"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Mesh::new(device, 
            Arc::new(vbuf),
            &vertices,
            Vec3::ZERO,
            format!("plane_{}", size),
        )
    }

    /// Editör sahneleri için GPU'da çizilen sonsuz grid mesh (tek bir quad). Shader içinde matematiksel olarak çizilir.
    pub fn create_editor_grid_mesh(device: &wgpu::Device, extents: f32) -> Mesh {
        let mut vertices = Vec::new();
        // Zemin boyunca devasa bir XY (veya XZ düzleminde) quad oluştur.
        let scale = extents;
        let v = [
            [-scale, 0.0, -scale],
            [scale, 0.0, -scale],
            [scale, 0.0, scale],
            [-scale, 0.0, scale],
        ];

        let indices = [0, 2, 1, 0, 3, 2];
        for i in indices {
            vertices.push(Vertex {
                position: v[i],
                color: [1.0, 1.0, 1.0],
                normal: [0.0, 1.0, 0.0],
                tex_coords: [0.0, 0.0],
                joint_indices: [0; 4],
                joint_weights: [0.0; 4],
            });
        }

        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Editor Infinite Grid VBuf"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Mesh::new(device, 
            Arc::new(vbuf),
            &vertices,
            Vec3::ZERO,
            "editor_grid".to_string(),
        )
    }

    /// 2D Sprite dörtgeni oluşturur (XY düzleminde, kameraya paralel).
    /// Ortografik projeksiyon ile kullanıldığında 2D oyun desteği sağlar.
    pub fn create_sprite_quad(device: &wgpu::Device, width: f32, height: f32) -> Mesh {
        let hw = width / 2.0;
        let hh = height / 2.0;
        let def_j = [0; 4];
        let def_w = [0.0; 4];

        // XY düzleminde dörtgen (Z=0), kameraya bakan yön +Z
        let vertices = [
            Vertex {
                position: [-hw, -hh, 0.0],
                color: [1.0, 1.0, 1.0],
                normal: [0.0, 0.0, 1.0],
                tex_coords: [0.0, 1.0],
                joint_indices: def_j,
                joint_weights: def_w,
            },
            Vertex {
                position: [hw, -hh, 0.0],
                color: [1.0, 1.0, 1.0],
                normal: [0.0, 0.0, 1.0],
                tex_coords: [1.0, 1.0],
                joint_indices: def_j,
                joint_weights: def_w,
            },
            Vertex {
                position: [hw, hh, 0.0],
                color: [1.0, 1.0, 1.0],
                normal: [0.0, 0.0, 1.0],
                tex_coords: [1.0, 0.0],
                joint_indices: def_j,
                joint_weights: def_w,
            },
            Vertex {
                position: [hw, hh, 0.0],
                color: [1.0, 1.0, 1.0],
                normal: [0.0, 0.0, 1.0],
                tex_coords: [1.0, 0.0],
                joint_indices: def_j,
                joint_weights: def_w,
            },
            Vertex {
                position: [-hw, hh, 0.0],
                color: [1.0, 1.0, 1.0],
                normal: [0.0, 0.0, 1.0],
                tex_coords: [0.0, 0.0],
                joint_indices: def_j,
                joint_weights: def_w,
            },
            Vertex {
                position: [-hw, -hh, 0.0],
                color: [1.0, 1.0, 1.0],
                normal: [0.0, 0.0, 1.0],
                tex_coords: [0.0, 1.0],
                joint_indices: def_j,
                joint_weights: def_w,
            },
        ];

        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Sprite Quad VBuf"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Mesh::new(device, 
            Arc::new(vbuf),
            &vertices,
            Vec3::ZERO,
            "sprite_quad".to_string(),
        )
    }

    /// Programatik UV Küre (Sphere) üretir.
    pub fn create_sphere(device: &wgpu::Device, radius: f32, stacks: u32, slices: u32) -> Mesh {
        let stacks = stacks.max(3);
        let slices = slices.max(3);
        let mut vertices = Vec::new();
        let pi = std::f32::consts::PI;

        for i in 0..stacks {
            let theta1 = (i as f32 / stacks as f32) * pi;
            let theta2 = ((i + 1) as f32 / stacks as f32) * pi;

            for j in 0..slices {
                let phi1 = (j as f32 / slices as f32) * 2.0 * pi;
                let phi2 = ((j + 1) as f32 / slices as f32) * 2.0 * pi;

                // 4 köşe noktası
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

                let uv1 = [
                    if i == 0 { (j as f32 + 0.5) / slices as f32 } else { j as f32 / slices as f32 },
                    i as f32 / stacks as f32,
                ];
                let uv2 = [
                    if i + 1 == stacks { (j as f32 + 0.5) / slices as f32 } else { j as f32 / slices as f32 },
                    (i + 1) as f32 / stacks as f32,
                ];
                let uv3 = [
                    if i + 1 == stacks { (j as f32 + 0.5) / slices as f32 } else { (j + 1) as f32 / slices as f32 },
                    (i + 1) as f32 / stacks as f32,
                ];
                let uv4 = [
                    if i == 0 { (j as f32 + 0.5) / slices as f32 } else { (j + 1) as f32 / slices as f32 },
                    i as f32 / stacks as f32,
                ];

                let def_j = [0; 4];
                let def_w = [0.0; 4];

                // Üçgen 1
                vertices.push(Vertex {
                    position: p1,
                    color: [1.0; 3],
                    normal: n1,
                    tex_coords: uv1,
                    joint_indices: def_j,
                    joint_weights: def_w,
                });
                vertices.push(Vertex {
                    position: p2,
                    color: [1.0; 3],
                    normal: n2,
                    tex_coords: uv2,
                    joint_indices: def_j,
                    joint_weights: def_w,
                });
                vertices.push(Vertex {
                    position: p3,
                    color: [1.0; 3],
                    normal: n3,
                    tex_coords: uv3,
                    joint_indices: def_j,
                    joint_weights: def_w,
                });
                // Üçgen 2
                vertices.push(Vertex {
                    position: p1,
                    color: [1.0; 3],
                    normal: n1,
                    tex_coords: uv1,
                    joint_indices: def_j,
                    joint_weights: def_w,
                });
                vertices.push(Vertex {
                    position: p3,
                    color: [1.0; 3],
                    normal: n3,
                    tex_coords: uv3,
                    joint_indices: def_j,
                    joint_weights: def_w,
                });
                vertices.push(Vertex {
                    position: p4,
                    color: [1.0; 3],
                    normal: n4,
                    tex_coords: uv4,
                    joint_indices: def_j,
                    joint_weights: def_w,
                });
            }
        }

        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Sphere VBuf"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Mesh::new(device, 
            Arc::new(vbuf),
            &vertices,
            Vec3::ZERO,
            format!("sphere_{}_{}_{}", radius, stacks, slices),
        )
    }

    pub fn create_terrain(
        device: &wgpu::Device,
        heightmap_path: &str,
        width: f32,
        depth: f32,
        max_height: f32,
    ) -> Result<(Mesh, Vec<f32>, u32, u32), String> {
        let canonical = std::path::Path::new(heightmap_path)
            .canonicalize()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| heightmap_path.to_string());

        let img = image::open(&canonical)
            .map_err(|e| format!("Heightmap yuklenemedi! {} ({})", canonical, e))?
            .into_luma8(); // Grayscale format

        let (img_width, img_height) = img.dimensions();
        if img_width < 2 || img_height < 2 {
            return Err("Heightmap boyutlari en az 2x2 olmalidir. 1x1 piksel ile arazi olusturulamaz.".to_string());
        }
        // Sınırlama: 512x512'den büyükse performans için uyar ya da downscale et

        let mut vertices: Vec<Vertex> = Vec::with_capacity((img_width * img_height) as usize);
        let mut heights: Vec<f32> = Vec::with_capacity((img_width * img_height) as usize);

        let half_w = width / 2.0;
        let half_d = depth / 2.0;

        // 1. GRID VERTEX'LERİ ÜRET
        for y in 0..img_height {
            for x in 0..img_width {
                let pixel = img.get_pixel(x, y)[0] as f32 / 255.0; // 0.0 - 1.0
                heights.push(pixel);
                let world_y = pixel * max_height;

                let world_x = -half_w + (x as f32 / (img_width as f32 - 1.0)) * width;
                let world_z = -half_d + (y as f32 / (img_height as f32 - 1.0)) * depth;

                // UV Mapping: Repeat 10 times across terrain so grass doesn't look stretched
                let uv_x = (x as f32 / (img_width as f32 - 1.0)) * 10.0;
                let uv_y = (y as f32 / (img_height as f32 - 1.0)) * 10.0;

                vertices.push(Vertex {
                    position: [world_x, world_y, world_z],
                    color: [1.0, 1.0, 1.0],
                    normal: [0.0, 1.0, 0.0], // İlk başta düz yukarı
                    tex_coords: [uv_x, uv_y],
                    joint_indices: [0; 4],
                    joint_weights: [0.0; 4],
                });
            }
        }

        // 2. INDEX'LERİ OLUŞTUR VE NORMALLERİ HESAPLA
        let mut indices = Vec::with_capacity(((img_width - 1) * (img_height - 1) * 6) as usize);
        for y in 0..(img_height - 1) {
            for x in 0..(img_width - 1) {
                let i0 = y * img_width + x;
                let i1 = y * img_width + (x + 1);
                let i2 = (y + 1) * img_width + x;
                let i3 = (y + 1) * img_width + (x + 1);

                // Triangle 1
                indices.push(i0);
                indices.push(i2);
                indices.push(i1);

                // Triangle 2
                indices.push(i1);
                indices.push(i2);
                indices.push(i3);
            }
        }

        // Face ve Smooth Normalleri hesapla
        let mut final_vertices = Vec::with_capacity(indices.len());
        for chunk in indices.chunks(3) {
            let i0 = chunk[0] as usize;
            let i1 = chunk[1] as usize;
            let i2 = chunk[2] as usize;

            let p0 = Vec3::from_array(vertices[i0].position);
            let p1 = Vec3::from_array(vertices[i1].position);
            let p2 = Vec3::from_array(vertices[i2].position);

            let norm = (p1 - p0).cross(p2 - p0);
            let normal = if norm.length_squared() > 1e-6 {
                norm.normalize()
            } else {
                Vec3::new(0.0, 1.0, 0.0)
            };

            // Triangle count for WGPU. Note: using flat normal per face first, optionally can be smoothed
            let mut v0 = vertices[i0];
            v0.normal = [normal.x, normal.y, normal.z];
            let mut v1 = vertices[i1];
            v1.normal = [normal.x, normal.y, normal.z];
            let mut v2 = vertices[i2];
            v2.normal = [normal.x, normal.y, normal.z];

            final_vertices.push(v0);
            final_vertices.push(v1);
            final_vertices.push(v2);
        }

        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("Terrain ({})", heightmap_path)),
            contents: bytemuck::cast_slice(&final_vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let mesh = Mesh::new(device, 
            Arc::new(vbuf),
            &final_vertices,
            Vec3::ZERO,
            format!("terrain:{}", heightmap_path),
        );
        Ok((mesh, heights, img_width, img_height))
    }
}
