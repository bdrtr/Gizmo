use wgpu::util::DeviceExt;
use std::sync::Arc;
use tobj;
use yelbegen_math::vec3::Vec3;
use crate::renderer::Vertex;
use crate::components::Mesh;

pub struct AssetManager {
    mesh_cache: std::collections::HashMap<String, Mesh>,
}

impl AssetManager {
    pub fn new() -> Self {
        Self {
            mesh_cache: std::collections::HashMap::new(),
        }
    }

    /// Bir .obj dosyasını diskten okur ve Mesh ECS bileşenine dönüştürür.
    /// Daha önce okunmuşsa, RAM ve VRAM tüketimini önlemek için önbellekten direkt kopya döndürür.
    pub fn load_obj(&mut self, device: &wgpu::Device, file_path: &str) -> Mesh {
        if let Some(cached) = self.mesh_cache.get(file_path) {
            return cached.clone(); // Orijinal veri Arc<Buffer> olduğu için direkt kopya döner (sıfır maliyet)
        }

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
        let m = &models[0].mesh;

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
                    1.0 - m.texcoords[idx * 2 + 1],
                ]
            } else {
                [0.0, 0.0]
            };

            vertices.push(Vertex {
                position,
                normal,
                tex_coords,
                color: [1.0, 1.0, 1.0],
            });
        }

        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("Obj VBuf: {}", file_path)),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let mesh = Mesh::new(Arc::new(vbuf), vertices.len() as u32, Vec3::ZERO);
        self.mesh_cache.insert(file_path.to_string(), mesh.clone());
        mesh
    }

    /// Bir resim dosyasını diskten okur ve wgpu::Texture olarak döndürür. (Statik yardımcı)
    pub fn load_texture(device: &wgpu::Device, queue: &wgpu::Queue, path: &str) -> wgpu::Texture {
        let img = image::open(path)
            .unwrap_or_else(|e| panic!("AssetManager: Doku yuklenirken hata! {} ({})", path, e))
            .to_rgba8();
        let dimensions = img.dimensions();
        let texture_size = wgpu::Extent3d {
            width: dimensions.0,
            height: dimensions.1,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            size: texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            label: Some(path),
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &img,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4 * dimensions.0),
                rows_per_image: Some(dimensions.1),
            },
            texture_size,
        );
        texture
    }
}
