use wgpu::util::DeviceExt;
use std::sync::Arc;
use tobj;
use yelbegen_math::vec3::Vec3;
use crate::renderer::Vertex;
use crate::components::Mesh;

pub struct AssetManager;

impl AssetManager {
    pub fn load_obj(device: &wgpu::Device, file_path: &str) -> Mesh {
        let (models, _materials) = tobj::load_obj(
            file_path,
            &tobj::LoadOptions {
                single_index: true,
                triangulate: true,
                ignore_points: true,
                ignore_lines: true,
            },
        ).unwrap_or_else(|e| panic!("AssetManager: OBJ yuklenirken hata! {} ({})", file_path, e));

        if models.is_empty() {
            panic!("AssetManager: OBJ dosyasinda model bulunamadi: {}", file_path);
        }

        let mut vertices = Vec::new();
        // Sadece ilk modeli baz alıyoruz
        let m = &models[0].mesh;

        // single_index kullandigimiz icin her sey ayni indekse denk gelir
        for i in &m.indices {
            let idx = *i as usize;

            let position = [
                m.positions[idx * 3],
                m.positions[idx * 3 + 1],
                m.positions[idx * 3 + 2],
            ];

            let normal = if !m.normals.is_empty() {
                [
                    m.normals[idx * 3],
                    m.normals[idx * 3 + 1],
                    m.normals[idx * 3 + 2],
                ]
            } else {
                [0.0, 1.0, 0.0]
            };

            let tex_coords = if !m.texcoords.is_empty() {
                [
                    m.texcoords[idx * 2],
                    1.0 - m.texcoords[idx * 2 + 1], // V eksenini ceviriyoruz
                ]
            } else {
                [0.0, 0.0]
            };

            vertices.push(Vertex {
                position,
                normal,
                tex_coords,
                color: [1.0, 1.0, 1.0], // Varsayilan renk
            });
        }

        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("Obj VBuf: {}", file_path)),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Mesh::new(Arc::new(vbuf), vertices.len() as u32, Vec3::ZERO)
    }
}
