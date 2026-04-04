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

        let mesh = Mesh::new(Arc::new(vbuf), vertices.len() as u32, Vec3::ZERO, format!("obj:{}", file_path));
        self.mesh_cache.insert(file_path.to_string(), mesh.clone());
        mesh
    }

    /// İçi boş ters yüzlü küp (Skybox) mesh üretir.
    /// Normaller içe bakar, böylece kamera küpün merkezinden dışarıya baktığında yüzeyler görünür.
    pub fn create_inverted_cube(device: &wgpu::Device) -> Mesh {
        // 6 yüz × 2 üçgen × 3 köşe = 36 vertex
        // Her yüzün normali İÇE bakar (ters küp)
        let positions: [[f32; 3]; 8] = [
            [-1.0, -1.0, -1.0], // 0
            [ 1.0, -1.0, -1.0], // 1
            [ 1.0,  1.0, -1.0], // 2
            [-1.0,  1.0, -1.0], // 3
            [-1.0, -1.0,  1.0], // 4
            [ 1.0, -1.0,  1.0], // 5
            [ 1.0,  1.0,  1.0], // 6
            [-1.0,  1.0,  1.0], // 7
        ];

        // Her yüz için ters vertex sırası (CW yerine CCW veya tam tersi) + içe bakan normal
        let faces: [([usize; 6], [f32; 3]); 6] = [
            ([0, 2, 1, 0, 3, 2], [0.0, 0.0,  1.0]),  // Arka yüz (+Z içe)
            ([4, 5, 6, 4, 6, 7], [0.0, 0.0, -1.0]),  // Ön yüz (-Z içe)
            ([0, 1, 5, 0, 5, 4], [0.0,  1.0, 0.0]),  // Alt yüz (+Y içe)
            ([3, 6, 2, 3, 7, 6], [0.0, -1.0, 0.0]),  // Üst yüz (-Y içe)
            ([0, 4, 7, 0, 7, 3], [ 1.0, 0.0, 0.0]),  // Sol yüz (+X içe)
            ([1, 2, 6, 1, 6, 5], [-1.0, 0.0, 0.0]),  // Sağ yüz (-X içe)
        ];

        let mut vertices = Vec::with_capacity(36);
        for (indices, normal) in &faces {
            for &idx in indices {
                vertices.push(Vertex {
                    position: positions[idx],
                    color: [1.0, 1.0, 1.0],
                    normal: *normal,
                    tex_coords: [0.0, 0.0],
                });
            }
        }

        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Skybox Inverted Cube VBuf"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Mesh::new(Arc::new(vbuf), vertices.len() as u32, Vec3::ZERO, "inverted_cube".to_string())
    }

    /// Basit, yatay bir düzlem (Plane) üretir.
    pub fn create_plane(device: &wgpu::Device, size: f32) -> Mesh {
        let half = size / 2.0;
        let y = 0.0;
        
        // Üstten bakışla Saat yönünün tersi (CCW) 2 üçgen (Quad)
        let vertices = [
            Vertex { position: [-half, y,  half], color: [1.0, 1.0, 1.0], normal: [0.0, 1.0, 0.0], tex_coords: [0.0, size] },
            Vertex { position: [ half, y,  half], color: [1.0, 1.0, 1.0], normal: [0.0, 1.0, 0.0], tex_coords: [size, size] },
            Vertex { position: [ half, y, -half], color: [1.0, 1.0, 1.0], normal: [0.0, 1.0, 0.0], tex_coords: [size, 0.0] },
            
            Vertex { position: [ half, y, -half], color: [1.0, 1.0, 1.0], normal: [0.0, 1.0, 0.0], tex_coords: [size, 0.0] },
            Vertex { position: [-half, y, -half], color: [1.0, 1.0, 1.0], normal: [0.0, 1.0, 0.0], tex_coords: [0.0, 0.0] },
            Vertex { position: [-half, y,  half], color: [1.0, 1.0, 1.0], normal: [0.0, 1.0, 0.0], tex_coords: [0.0, size] },
        ];

        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Plane VBuf"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Mesh::new(Arc::new(vbuf), vertices.len() as u32, Vec3::ZERO, "plane".to_string())
    }

    /// Programatik UV Küre (Sphere) üretir.
    pub fn create_sphere(device: &wgpu::Device, radius: f32, stacks: u32, slices: u32) -> Mesh {
        let mut vertices = Vec::new();
        let pi = std::f32::consts::PI;

        for i in 0..stacks {
            let theta1 = (i as f32 / stacks as f32) * pi;
            let theta2 = ((i + 1) as f32 / stacks as f32) * pi;

            for j in 0..slices {
                let phi1 = (j as f32 / slices as f32) * 2.0 * pi;
                let phi2 = ((j + 1) as f32 / slices as f32) * 2.0 * pi;

                // 4 köşe noktası
                let p1 = [radius * theta1.sin() * phi1.cos(), radius * theta1.cos(), radius * theta1.sin() * phi1.sin()];
                let p2 = [radius * theta2.sin() * phi1.cos(), radius * theta2.cos(), radius * theta2.sin() * phi1.sin()];
                let p3 = [radius * theta2.sin() * phi2.cos(), radius * theta2.cos(), radius * theta2.sin() * phi2.sin()];
                let p4 = [radius * theta1.sin() * phi2.cos(), radius * theta1.cos(), radius * theta1.sin() * phi2.sin()];

                let n1 = [theta1.sin() * phi1.cos(), theta1.cos(), theta1.sin() * phi1.sin()];
                let n2 = [theta2.sin() * phi1.cos(), theta2.cos(), theta2.sin() * phi1.sin()];
                let n3 = [theta2.sin() * phi2.cos(), theta2.cos(), theta2.sin() * phi2.sin()];
                let n4 = [theta1.sin() * phi2.cos(), theta1.cos(), theta1.sin() * phi2.sin()];

                let uv1 = [j as f32 / slices as f32, i as f32 / stacks as f32];
                let uv2 = [j as f32 / slices as f32, (i + 1) as f32 / stacks as f32];
                let uv3 = [(j + 1) as f32 / slices as f32, (i + 1) as f32 / stacks as f32];
                let uv4 = [(j + 1) as f32 / slices as f32, i as f32 / stacks as f32];

                // Üçgen 1
                vertices.push(Vertex { position: p1, color: [1.0; 3], normal: n1, tex_coords: uv1 });
                vertices.push(Vertex { position: p2, color: [1.0; 3], normal: n2, tex_coords: uv2 });
                vertices.push(Vertex { position: p3, color: [1.0; 3], normal: n3, tex_coords: uv3 });
                // Üçgen 2
                vertices.push(Vertex { position: p1, color: [1.0; 3], normal: n1, tex_coords: uv1 });
                vertices.push(Vertex { position: p3, color: [1.0; 3], normal: n3, tex_coords: uv3 });
                vertices.push(Vertex { position: p4, color: [1.0; 3], normal: n4, tex_coords: uv4 });
            }
        }

        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Sphere VBuf"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Mesh::new(Arc::new(vbuf), vertices.len() as u32, Vec3::ZERO, "sphere".to_string())
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

    /// Bir resmi okuyup Bind Group (Material Texture + Sampler) haline getirir
    pub fn load_material_texture(device: &wgpu::Device, queue: &wgpu::Queue, layout: &wgpu::BindGroupLayout, path: &str) -> Arc<wgpu::BindGroup> {
        let texture = Self::load_texture(device, queue, path);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Arc::new(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(path),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        }))
    }
}
