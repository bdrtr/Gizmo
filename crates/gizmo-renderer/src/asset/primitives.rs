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

    /// Basit, yatay bir düzlem (Plane) üretir.
    /// Düzlem köşeleri (XZ düzleminde, +Y'ye bakan). Saf veri — device gerekmez,
    /// winding testi buna doğrudan erişebilir.
    pub(crate) fn plane_data(size: f32) -> Vec<Vertex> {
        let half = size / 2.0;
        let y = 0.0;
        let def_j = [0; 4];
        let def_w = [0.0; 4];
        let vtx = |position: [f32; 3], tex_coords: [f32; 2]| Vertex {
            position,
            color: [1.0, 1.0, 1.0],
            normal: [0.0, 1.0, 0.0],
            tex_coords,
            joint_indices: def_j,
            joint_weights: def_w,
            ..Default::default()
        };
        let a = ([-half, y, -half], [0.0, 0.0]);
        let b = ([half, y, -half], [size, 0.0]);
        let c = ([half, y, half], [size, size]);
        let d = ([-half, y, half], [0.0, size]);
        // Üstten bakışta CCW (sağ-el normali = +Y) → Ccw+Back-cull pipeline'ında
        // üstten görünür. (Eskiden CW idi → düzlem üstten bakınca culllanıyordu.)
        vec![
            vtx(a.0, a.1), vtx(c.0, c.1), vtx(b.0, b.1), // Üçgen 1: A→C→B
            vtx(a.0, a.1), vtx(d.0, d.1), vtx(c.0, c.1), // Üçgen 2: A→D→C
        ]
    }

    pub fn create_plane(device: &wgpu::Device, size: f32) -> Mesh {
        let vertices = Self::plane_data(size);
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Plane VBuf"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Mesh::new(
            device,
            Arc::new(vbuf),
            &vertices,
            Vec3::ZERO,
            format!("plane_{}", size),
        )
    }

    /// Yuvarlak bir disk (Çember tabanı) köşeleri (+Y'ye bakan). Saf veri.
    pub(crate) fn circle_data(radius: f32, segments: u32) -> Vec<Vertex> {
        let segments = segments.max(3);
        let mut vertices = Vec::with_capacity((segments * 3) as usize);

        let center = [0.0, 0.0, 0.0];
        let normal = [0.0, 1.0, 0.0];
        let def_j = [0; 4];
        let def_w = [0.0; 4];
        let vtx = |position: [f32; 3], tex_coords: [f32; 2]| Vertex {
            position,
            color: [1.0, 1.0, 1.0],
            normal,
            tex_coords,
            joint_indices: def_j,
            joint_weights: def_w,
            ..Default::default()
        };

        for i in 0..segments {
            let angle1 = (i as f32 / segments as f32) * std::f32::consts::PI * 2.0;
            let angle2 = ((i + 1) as f32 / segments as f32) * std::f32::consts::PI * 2.0;

            let p1 = [radius * angle1.cos(), 0.0, radius * angle1.sin()];
            let p2 = [radius * angle2.cos(), 0.0, radius * angle2.sin()];

            let uv_center = [0.5, 0.5];
            let uv1 = [0.5 + 0.5 * angle1.cos(), 0.5 + 0.5 * angle1.sin()];
            let uv2 = [0.5 + 0.5 * angle2.cos(), 0.5 + 0.5 * angle2.sin()];

            // CCW sarım (Center → P2 → P1) → sağ-el normali +Y, üstten görünür.
            // (Eskiden Center→P1→P2 idi → disk üstten bakınca culllanıyordu.)
            vertices.push(vtx(center, uv_center));
            vertices.push(vtx(p2, uv2));
            vertices.push(vtx(p1, uv1));
        }
        vertices
    }

    pub fn create_circle(device: &wgpu::Device, radius: f32, segments: u32) -> Mesh {
        let vertices = Self::circle_data(radius, segments);
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Circle VBuf"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Mesh::new(
            device,
            Arc::new(vbuf),
            &vertices,
            Vec3::ZERO,
            format!("circle_{}_{}", radius, segments),
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
                joint_weights: [0.0; 4], ..Default::default()
            });
        }

        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Editor Infinite Grid VBuf"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Mesh::new(
            device,
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
                joint_weights: def_w, ..Default::default()
            },
            Vertex {
                position: [hw, -hh, 0.0],
                color: [1.0, 1.0, 1.0],
                normal: [0.0, 0.0, 1.0],
                tex_coords: [1.0, 1.0],
                joint_indices: def_j,
                joint_weights: def_w, ..Default::default()
            },
            Vertex {
                position: [hw, hh, 0.0],
                color: [1.0, 1.0, 1.0],
                normal: [0.0, 0.0, 1.0],
                tex_coords: [1.0, 0.0],
                joint_indices: def_j,
                joint_weights: def_w, ..Default::default()
            },
            Vertex {
                position: [hw, hh, 0.0],
                color: [1.0, 1.0, 1.0],
                normal: [0.0, 0.0, 1.0],
                tex_coords: [1.0, 0.0],
                joint_indices: def_j,
                joint_weights: def_w, ..Default::default()
            },
            Vertex {
                position: [-hw, hh, 0.0],
                color: [1.0, 1.0, 1.0],
                normal: [0.0, 0.0, 1.0],
                tex_coords: [0.0, 0.0],
                joint_indices: def_j,
                joint_weights: def_w, ..Default::default()
            },
            Vertex {
                position: [-hw, -hh, 0.0],
                color: [1.0, 1.0, 1.0],
                normal: [0.0, 0.0, 1.0],
                tex_coords: [0.0, 1.0],
                joint_indices: def_j,
                joint_weights: def_w, ..Default::default()
            },
        ];

        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Sprite Quad VBuf"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Mesh::new(
            device,
            Arc::new(vbuf),
            &vertices,
            Vec3::ZERO,
            "sprite_quad".to_string(),
        )
    }

    /// Programatik UV Küre (Sphere) üretir.
    /// UV-küre köşeleri (dış-yüzey CCW sarımlı, kutup dejenereleri atlanmış). Saf veri.
    pub(crate) fn sphere_data(radius: f32, stacks: u32, slices: u32) -> Vec<Vertex> {
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
                    if i == 0 {
                        (j as f32 + 0.5) / slices as f32
                    } else {
                        j as f32 / slices as f32
                    },
                    i as f32 / stacks as f32,
                ];
                let uv2 = [
                    if i + 1 == stacks {
                        (j as f32 + 0.5) / slices as f32
                    } else {
                        j as f32 / slices as f32
                    },
                    (i + 1) as f32 / stacks as f32,
                ];
                let uv3 = [
                    if i + 1 == stacks {
                        (j as f32 + 0.5) / slices as f32
                    } else {
                        (j + 1) as f32 / slices as f32
                    },
                    (i + 1) as f32 / stacks as f32,
                ];
                let uv4 = [
                    if i == 0 {
                        (j as f32 + 0.5) / slices as f32
                    } else {
                        (j + 1) as f32 / slices as f32
                    },
                    i as f32 / stacks as f32,
                ];

                let def_j = [0; 4];
                let def_w = [0.0; 4];
                let vtx = |position: [f32; 3], normal: [f32; 3], tex_coords: [f32; 2]| Vertex {
                    position,
                    color: [1.0; 3],
                    normal,
                    tex_coords,
                    joint_indices: def_j,
                    joint_weights: def_w,
                    ..Default::default()
                };

                // Sağ-el (CCW dıştan) sarım: geometrik normal = dış yüzey normali, böylece
                // Ccw+Back-cull pipeline'ında küre dışarıdan görünür. (Eskiden p1,p2,p3 /
                // p1,p3,p4 sırası geometrik normali İÇE veriyordu → küre içi-dışına culllanıyordu.)
                // Ayrıca kutup satırlarında iki köşesi çakışan DEJENERE üçgen atlanıyor.

                // Üçgen 1: p1 → p3 → p2  (güney kutbu satırında p2==p3 → dejenere)
                if i != stacks - 1 {
                    vertices.push(vtx(p1, n1, uv1));
                    vertices.push(vtx(p3, n3, uv3));
                    vertices.push(vtx(p2, n2, uv2));
                }
                // Üçgen 2: p1 → p4 → p3  (kuzey kutbu satırında p1==p4 → dejenere)
                if i != 0 {
                    vertices.push(vtx(p1, n1, uv1));
                    vertices.push(vtx(p4, n4, uv4));
                    vertices.push(vtx(p3, n3, uv3));
                }
            }
        }

        vertices
    }

    pub fn create_sphere(device: &wgpu::Device, radius: f32, stacks: u32, slices: u32) -> Mesh {
        let vertices = Self::sphere_data(radius, stacks, slices);
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Sphere VBuf"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Mesh::new(
            device,
            Arc::new(vbuf),
            &vertices,
            Vec3::ZERO,
            format!("sphere_{}_{}_{}", radius, stacks, slices),
        )
    }

    /// Silindir köşeleri (yan + iki kapak), dış-yüzey CCW sarımlı. Saf veri.
    pub(crate) fn cylinder_data(radius: f32, height: f32, radial_segments: u32) -> Vec<Vertex> {
        let radial_segments = radial_segments.max(3);
        let mut vertices = Vec::new();
        let pi = std::f32::consts::PI;
        let half_h = height / 2.0;
        let def_j = [0; 4]; let def_w = [0.0; 4]; let col = [1.0; 3];
        let vtx = |position: [f32; 3], normal: [f32; 3], tex_coords: [f32; 2]| Vertex {
            position, color: col, normal, tex_coords, joint_indices: def_j, joint_weights: def_w, ..Default::default()
        };

        // Tube
        for i in 0..radial_segments {
            let t1 = (i as f32 / radial_segments as f32) * 2.0 * pi;
            let t2 = ((i + 1) as f32 / radial_segments as f32) * 2.0 * pi;

            let u1 = i as f32 / radial_segments as f32;
            let u2 = (i + 1) as f32 / radial_segments as f32;

            let p1_top = [radius * t1.cos(), half_h, radius * t1.sin()];
            let p1_bot = [radius * t1.cos(), -half_h, radius * t1.sin()];
            let p2_top = [radius * t2.cos(), half_h, radius * t2.sin()];
            let p2_bot = [radius * t2.cos(), -half_h, radius * t2.sin()];

            let n1 = [t1.cos(), 0.0, t1.sin()];
            let n2 = [t2.cos(), 0.0, t2.sin()];

            // Yan yüzler: dıştan CCW (sağ-el normali dışa). (Eskiden ters → culllanıyordu.)
            // Tri 1: p1_top → p2_bot → p1_bot
            vertices.push(vtx(p1_top, n1, [u1, 0.0]));
            vertices.push(vtx(p2_bot, n2, [u2, 1.0]));
            vertices.push(vtx(p1_bot, n1, [u1, 1.0]));
            // Tri 2: p1_top → p2_top → p2_bot
            vertices.push(vtx(p1_top, n1, [u1, 0.0]));
            vertices.push(vtx(p2_top, n2, [u2, 0.0]));
            vertices.push(vtx(p2_bot, n2, [u2, 1.0]));

            // Üst kapak: center → p2_top → p1_top (sağ-el normali +Y).
            vertices.push(vtx([0.0, half_h, 0.0], [0.0, 1.0, 0.0], [0.5, 0.5]));
            vertices.push(vtx(p2_top, [0.0, 1.0, 0.0], [0.5 + 0.5 * t2.cos(), 0.5 + 0.5 * t2.sin()]));
            vertices.push(vtx(p1_top, [0.0, 1.0, 0.0], [0.5 + 0.5 * t1.cos(), 0.5 + 0.5 * t1.sin()]));

            // Alt kapak: center → p1_bot → p2_bot (sağ-el normali -Y).
            vertices.push(vtx([0.0, -half_h, 0.0], [0.0, -1.0, 0.0], [0.5, 0.5]));
            vertices.push(vtx(p1_bot, [0.0, -1.0, 0.0], [0.5 + 0.5 * t1.cos(), 0.5 + 0.5 * t1.sin()]));
            vertices.push(vtx(p2_bot, [0.0, -1.0, 0.0], [0.5 + 0.5 * t2.cos(), 0.5 + 0.5 * t2.sin()]));
        }
        vertices
    }

    pub fn create_cylinder(device: &wgpu::Device, radius: f32, height: f32, radial_segments: u32) -> Mesh {
        let vertices = Self::cylinder_data(radius, height, radial_segments);
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("Cylinder VBuf"), contents: bytemuck::cast_slice(&vertices), usage: wgpu::BufferUsages::VERTEX });
        Mesh::new(device, Arc::new(vbuf), &vertices, Vec3::ZERO, format!("cylinder_{}_{}", radius, height))
    }

    /// Koni köşeleri (yan + taban), dış-yüzey CCW sarımlı. Saf veri.
    pub(crate) fn cone_data(radius: f32, height: f32, radial_segments: u32) -> Vec<Vertex> {
        let radial_segments = radial_segments.max(3);
        let mut vertices = Vec::new();
        let pi = std::f32::consts::PI;
        let half_h = height / 2.0;
        let def_j = [0; 4]; let def_w = [0.0; 4]; let col = [1.0; 3];
        let vtx = |position: [f32; 3], normal: [f32; 3], tex_coords: [f32; 2]| Vertex {
            position, color: col, normal, tex_coords, joint_indices: def_j, joint_weights: def_w, ..Default::default()
        };

        let slant = (radius * radius + height * height).sqrt();
        let ny = radius / slant;
        let n_xz = height / slant;

        for i in 0..radial_segments {
            let t1 = (i as f32 / radial_segments as f32) * 2.0 * pi;
            let t2 = ((i + 1) as f32 / radial_segments as f32) * 2.0 * pi;

            let p1_bot = [radius * t1.cos(), -half_h, radius * t1.sin()];
            let p2_bot = [radius * t2.cos(), -half_h, radius * t2.sin()];
            let apex = [0.0, half_h, 0.0];

            let n1 = [n_xz * t1.cos(), ny, n_xz * t1.sin()];
            let n2 = [n_xz * t2.cos(), ny, n_xz * t2.sin()];
            let navg = [n_xz * ((t1+t2)/2.0).cos(), ny, n_xz * ((t1+t2)/2.0).sin()];

            let u1 = i as f32 / radial_segments as f32;
            let u2 = (i + 1) as f32 / radial_segments as f32;
            let umid = (u1 + u2) / 2.0;

            // Yan üçgen: apex → p2_bot → p1_bot (dıştan CCW). (Eskiden ters → culllanıyordu.)
            vertices.push(vtx(apex, navg, [umid, 0.0]));
            vertices.push(vtx(p2_bot, n2, [u2, 1.0]));
            vertices.push(vtx(p1_bot, n1, [u1, 1.0]));

            // Taban kapağı: center → p1_bot → p2_bot (sağ-el normali -Y).
            vertices.push(vtx([0.0, -half_h, 0.0], [0.0, -1.0, 0.0], [0.5, 0.5]));
            vertices.push(vtx(p1_bot, [0.0, -1.0, 0.0], [0.5 + 0.5 * t1.cos(), 0.5 + 0.5 * t1.sin()]));
            vertices.push(vtx(p2_bot, [0.0, -1.0, 0.0], [0.5 + 0.5 * t2.cos(), 0.5 + 0.5 * t2.sin()]));
        }
        vertices
    }

    pub fn create_cone(device: &wgpu::Device, radius: f32, height: f32, radial_segments: u32) -> Mesh {
        let vertices = Self::cone_data(radius, height, radial_segments);
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("Cone VBuf"), contents: bytemuck::cast_slice(&vertices), usage: wgpu::BufferUsages::VERTEX });
        Mesh::new(device, Arc::new(vbuf), &vertices, Vec3::ZERO, format!("cone_{}_{}", radius, height))
    }

    pub fn create_torus(device: &wgpu::Device, radius: f32, tube_radius: f32, radial_segments: u32, tubular_segments: u32) -> Mesh {
        let radial_segments = radial_segments.max(3);
        let tubular_segments = tubular_segments.max(3);
        let mut vertices = Vec::new();
        let pi = std::f32::consts::PI;

        for i in 0..radial_segments {
            for j in 0..tubular_segments {
                let u1 = i as f32 / radial_segments as f32;
                let u2 = (i + 1) as f32 / radial_segments as f32;
                let v1 = j as f32 / tubular_segments as f32;
                let v2 = (j + 1) as f32 / tubular_segments as f32;

                let t1 = u1 * 2.0 * pi;
                let t2 = u2 * 2.0 * pi;
                let p1 = v1 * 2.0 * pi;
                let p2 = v2 * 2.0 * pi;

                let pos = |t: f32, p: f32| {
                    [(radius + tube_radius * p.cos()) * t.cos(), tube_radius * p.sin(), (radius + tube_radius * p.cos()) * t.sin()]
                };
                let norm = |t: f32, p: f32| {
                    [p.cos() * t.cos(), p.sin(), p.cos() * t.sin()]
                };

                let p_00 = pos(t1, p1); let n_00 = norm(t1, p1);
                let p_10 = pos(t2, p1); let n_10 = norm(t2, p1);
                let p_01 = pos(t1, p2); let n_01 = norm(t1, p2);
                let p_11 = pos(t2, p2); let n_11 = norm(t2, p2);

                let def_j = [0; 4]; let def_w = [0.0; 4];
                let col = [1.0; 3];

                vertices.push(Vertex { position: p_00, normal: n_00, tex_coords: [u1, v1], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
                vertices.push(Vertex { position: p_01, normal: n_01, tex_coords: [u1, v2], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
                vertices.push(Vertex { position: p_10, normal: n_10, tex_coords: [u2, v1], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });

                vertices.push(Vertex { position: p_10, normal: n_10, tex_coords: [u2, v1], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
                vertices.push(Vertex { position: p_01, normal: n_01, tex_coords: [u1, v2], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
                vertices.push(Vertex { position: p_11, normal: n_11, tex_coords: [u2, v2], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            }
        }

        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("Torus VBuf"), contents: bytemuck::cast_slice(&vertices), usage: wgpu::BufferUsages::VERTEX });
        Mesh::new(device, Arc::new(vbuf), &vertices, Vec3::ZERO, format!("torus_{}_{}", radius, tube_radius))
    }

    /// Kapsül köşeleri (tüp + iki yarıküre), dış-yüzey CCW sarımlı. Saf veri.
    pub(crate) fn capsule_data(radius: f32, depth: f32, latitudes: u32, longitudes: u32) -> Vec<Vertex> {
        let latitudes = latitudes.max(4);
        let longitudes = longitudes.max(4);
        let mut vertices = Vec::new();
        let pi = std::f32::consts::PI;
        let half_d = depth / 2.0;

        for i in 0..=latitudes {
            let u1 = i as f32 / latitudes as f32;
            let u2 = (i + 1) as f32 / latitudes as f32;
            let theta1 = u1 * pi;
            let theta2 = u2 * pi;

            let y_offset1 = if u1 < 0.5 { half_d } else if u1 > 0.5 { -half_d } else { 0.0 };
            let y_offset2 = if u2 < 0.5 { half_d } else if u2 > 0.5 { -half_d } else { 0.0 };
            
            // To properly insert a tube, we duplicate the equator loop
            let is_equator = i == latitudes / 2;

            if is_equator {
                // Tube segment
                for j in 0..longitudes {
                    let v1 = j as f32 / longitudes as f32;
                    let v2 = (j + 1) as f32 / longitudes as f32;
                    let phi1 = v1 * 2.0 * pi;
                    let phi2 = v2 * 2.0 * pi;

                    let p1_top = [radius * phi1.cos(), half_d, radius * phi1.sin()];
                    let p1_bot = [radius * phi1.cos(), -half_d, radius * phi1.sin()];
                    let p2_top = [radius * phi2.cos(), half_d, radius * phi2.sin()];
                    let p2_bot = [radius * phi2.cos(), -half_d, radius * phi2.sin()];

                    let n1 = [phi1.cos(), 0.0, phi1.sin()];
                    let n2 = [phi2.cos(), 0.0, phi2.sin()];

                    let def_j = [0; 4]; let def_w = [0.0; 4]; let col = [1.0; 3];

                    // Tüp dıştan CCW (eskiden ters → culllanıyordu).
                    // Tri 1: p1_top → p2_bot → p1_bot
                    vertices.push(Vertex { position: p1_top, normal: n1, tex_coords: [v1, 0.5], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
                    vertices.push(Vertex { position: p2_bot, normal: n2, tex_coords: [v2, 0.5], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
                    vertices.push(Vertex { position: p1_bot, normal: n1, tex_coords: [v1, 0.5], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });

                    // Tri 2: p1_top → p2_top → p2_bot
                    vertices.push(Vertex { position: p1_top, normal: n1, tex_coords: [v1, 0.5], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
                    vertices.push(Vertex { position: p2_top, normal: n2, tex_coords: [v2, 0.5], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
                    vertices.push(Vertex { position: p2_bot, normal: n2, tex_coords: [v2, 0.5], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
                }
            }

            if i < latitudes {
                for j in 0..longitudes {
                    let v1 = j as f32 / longitudes as f32;
                    let v2 = (j + 1) as f32 / longitudes as f32;
                    let phi1 = v1 * 2.0 * pi;
                    let phi2 = v2 * 2.0 * pi;

                    let p1 = [radius * theta1.sin() * phi1.cos(), radius * theta1.cos() + y_offset1, radius * theta1.sin() * phi1.sin()];
                    let p2 = [radius * theta2.sin() * phi1.cos(), radius * theta2.cos() + y_offset2, radius * theta2.sin() * phi1.sin()];
                    let p3 = [radius * theta2.sin() * phi2.cos(), radius * theta2.cos() + y_offset2, radius * theta2.sin() * phi2.sin()];
                    let p4 = [radius * theta1.sin() * phi2.cos(), radius * theta1.cos() + y_offset1, radius * theta1.sin() * phi2.sin()];

                    let n1 = [theta1.sin() * phi1.cos(), theta1.cos(), theta1.sin() * phi1.sin()];
                    let n2 = [theta2.sin() * phi1.cos(), theta2.cos(), theta2.sin() * phi1.sin()];
                    let n3 = [theta2.sin() * phi2.cos(), theta2.cos(), theta2.sin() * phi2.sin()];
                    let n4 = [theta1.sin() * phi2.cos(), theta1.cos(), theta1.sin() * phi2.sin()];

                    let def_j = [0; 4]; let def_w = [0.0; 4]; let col = [1.0; 3];

                    // Yarıküre bantları dıştan CCW (eskiden ters → culllanıyordu).
                    // Kutup satırlarında çakışan köşeli DEJENERE üçgen atlanır (sphere ile aynı).
                    // Tri 1: p1 → p3 → p2  (güney kutbu satırında p2==p3 → dejenere)
                    if i != latitudes - 1 {
                        vertices.push(Vertex { position: p1, normal: n1, tex_coords: [v1, u1], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
                        vertices.push(Vertex { position: p3, normal: n3, tex_coords: [v2, u2], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
                        vertices.push(Vertex { position: p2, normal: n2, tex_coords: [v1, u2], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
                    }

                    // Tri 2: p1 → p4 → p3  (kuzey kutbu satırında p1==p4 → dejenere)
                    if i != 0 {
                        vertices.push(Vertex { position: p1, normal: n1, tex_coords: [v1, u1], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
                        vertices.push(Vertex { position: p4, normal: n4, tex_coords: [v2, u1], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
                        vertices.push(Vertex { position: p3, normal: n3, tex_coords: [v2, u2], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
                    }
                }
            }
        }

        vertices
    }

    pub fn create_capsule(device: &wgpu::Device, radius: f32, depth: f32, latitudes: u32, longitudes: u32) -> Mesh {
        let vertices = Self::capsule_data(radius, depth, latitudes, longitudes);
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("Capsule VBuf"), contents: bytemuck::cast_slice(&vertices), usage: wgpu::BufferUsages::VERTEX });
        Mesh::new(device, Arc::new(vbuf), &vertices, Vec3::ZERO, format!("capsule_{}_{}", radius, depth))
    }


    pub fn create_terrain(
        device: &wgpu::Device,
        heightmap_path: &str,
        width: f32,
        depth: f32,
        max_height: f32,
    ) -> Result<(Mesh, Vec<f32>, u32, u32), super::error::AssetError> {
        let canonical = std::path::Path::new(heightmap_path)
            .canonicalize()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| heightmap_path.to_string());

        let img = image::open(&canonical)
            .map_err(|source| super::error::AssetError::ImageDecode {
                path: std::path::PathBuf::from(&canonical),
                source,
            })?
            .into_luma8(); // Grayscale format

        let (img_width, img_height) = img.dimensions();
        if img_width < 2 || img_height < 2 {
            return Err(super::error::AssetError::HeightmapTooSmall {
                path: std::path::PathBuf::from(&canonical),
                width: img_width,
                height: img_height,
            });
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
                    joint_weights: [0.0; 4], ..Default::default()
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

        let mesh = Mesh::new(
            device,
            Arc::new(vbuf),
            &final_vertices,
            Vec3::ZERO,
            format!("terrain:{}", heightmap_path),
        );
        Ok((mesh, heights, img_width, img_height))
    }
}

#[cfg(test)]
mod winding_tests {
    //! Prosedürel mesh'lerin üçgen sarımı (winding) ile declared yüzey normalleri
    //! TUTARLI olmalı: geometrik (sağ-el) normal, declared outward normal ile aynı
    //! yöne bakmalı (dot > 0). Aksi halde Ccw + Back-cull pipeline'ında (bkz.
    //! pipeline.rs:579-580, deferred.rs:336-337) yüzeyler back-face sayılıp culllanır
    //! → şekil "içi-dışına" / görünmez render olur. (Bu testin yakaladığı bug buydu.)
    //!
    //! Saf `*_data` fonksiyonları üzerinde çalışır — GPU device gerekmez.

    use crate::asset::AssetManager;
    use crate::gpu_types::Vertex;
    use gizmo_math::Vec3;

    /// Bir non-indexed üçgen listesinin sarım + normal geçerliliğini doğrula.
    fn assert_outward(name: &str, verts: &[Vertex]) {
        assert!(
            !verts.is_empty() && verts.len() % 3 == 0,
            "{name}: vertex sayısı pozitif ve 3'ün katı olmalı, {}",
            verts.len()
        );
        let mut checked = 0usize;
        for tri in verts.chunks_exact(3) {
            for v in tri {
                let n = Vec3::from(v.normal);
                assert!(
                    n.is_finite() && (n.length() - 1.0).abs() < 1e-3,
                    "{name}: normal birim/sonlu değil: {:?}",
                    v.normal
                );
            }
            let p0 = Vec3::from(tri[0].position);
            let p1 = Vec3::from(tri[1].position);
            let p2 = Vec3::from(tri[2].position);
            let geo = (p1 - p0).cross(p2 - p0);
            // Dejenere üçgenleri (ör. kapsül yarıküre kutupları) atla.
            if geo.length() < 1e-9 {
                continue;
            }
            let n_avg = Vec3::from(tri[0].normal) + Vec3::from(tri[1].normal) + Vec3::from(tri[2].normal);
            let d = geo.normalize().dot(n_avg.normalize());
            assert!(
                d > 0.0,
                "{name}: üçgen sarımı declared normal'in TERSİNE (geo·n={d} ≤ 0) → \
                 Ccw+Back-cull'da içi-dışına culllanır. p0={p0:?} p1={p1:?} p2={p2:?}"
            );
            checked += 1;
        }
        assert!(checked > 0, "{name}: dejenere olmayan üçgen yok");
    }

    #[test]
    fn plane_winding_faces_up() {
        assert_outward("plane", &AssetManager::plane_data(2.0));
    }

    #[test]
    fn circle_winding_faces_up() {
        assert_outward("circle", &AssetManager::circle_data(1.0, 16));
    }

    #[test]
    fn sphere_winding_is_outward() {
        for &(stacks, slices) in &[(3, 3), (8, 12), (16, 24)] {
            assert_outward(
                &format!("sphere({stacks},{slices})"),
                &AssetManager::sphere_data(1.5, stacks, slices),
            );
        }
    }

    #[test]
    fn sphere_has_no_degenerate_triangles() {
        // Kutup dejenereleri kaldırıldı: hiçbir üçgenin iki köşesi çakışmamalı.
        let v = AssetManager::sphere_data(1.0, 8, 12);
        for tri in v.chunks_exact(3) {
            let geo = (Vec3::from(tri[1].position) - Vec3::from(tri[0].position))
                .cross(Vec3::from(tri[2].position) - Vec3::from(tri[0].position));
            assert!(geo.length() > 1e-9, "sphere dejenere üçgen üretti");
        }
    }

    #[test]
    fn cylinder_winding_is_outward() {
        assert_outward("cylinder", &AssetManager::cylinder_data(1.0, 2.0, 16));
    }

    #[test]
    fn cone_winding_is_outward() {
        assert_outward("cone", &AssetManager::cone_data(1.0, 2.0, 16));
    }

    #[test]
    fn capsule_winding_is_outward() {
        assert_outward("capsule", &AssetManager::capsule_data(0.5, 1.0, 8, 12));
    }

    #[test]
    fn tetrahedron_winding_is_outward() {
        assert_outward("tetrahedron", &AssetManager::tetrahedron_data(1.0));
    }

    #[test]
    fn conical_frustum_winding_is_outward() {
        assert_outward(
            "conical_frustum",
            &AssetManager::conical_frustum_data(1.0, 0.5, 2.0, 16),
        );
    }
}
