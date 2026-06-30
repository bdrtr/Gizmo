use crate::components::Mesh;
use crate::renderer::Vertex;
use gizmo_math::Vec3;
use std::sync::Arc;
use wgpu::util::DeviceExt;

impl crate::asset::AssetManager {
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
}
