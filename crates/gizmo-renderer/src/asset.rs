use wgpu::util::DeviceExt;
use std::sync::Arc;
use tobj;
use gizmo_math::Vec3;
use crate::renderer::Vertex;
use crate::components::{Mesh, Material};
use crate::animation::{AnimationClip, Track, Keyframe, SkeletonHierarchy, SkeletonJoint};
use gizmo_math::Quat;

pub struct AssetManager {
    mesh_cache: std::collections::HashMap<String, Mesh>,
    pub texture_cache: std::collections::HashMap<String, Arc<wgpu::BindGroup>>,
}

impl Default for AssetManager {
    fn default() -> Self {
        Self::new()
    }
}

impl AssetManager {
    pub fn new() -> Self {
        Self {
            mesh_cache: std::collections::HashMap::new(),
            texture_cache: std::collections::HashMap::new(),
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

        let mut aabb = gizmo_math::Aabb::empty();
        let mut vertices = Vec::new();
        let m = &models[0].mesh;

        for i in &m.indices {
            let idx = *i as usize;

            let position = [
                m.positions[idx * 3],
                m.positions[idx * 3 + 1],
                m.positions[idx * 3 + 2],
            ];
            aabb.extend(Vec3::new(position[0], position[1], position[2]));

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
                joint_indices: [0; 4],
                joint_weights: [0.0; 4],
            });
        }

        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("Obj VBuf: {}", file_path)),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let mesh = Mesh::new(Arc::new(vbuf), vertices.len() as u32, Vec3::ZERO, format!("obj:{}", file_path), aabb);
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
            ([0, 1, 2, 0, 2, 3], [0.0, 0.0,  1.0]),  // Arka yüz (+Z içe)
            ([4, 6, 5, 4, 7, 6], [0.0, 0.0, -1.0]),  // Ön yüz (-Z içe)
            ([0, 5, 1, 0, 4, 5], [0.0,  1.0, 0.0]),  // Alt yüz (+Y içe)
            ([3, 2, 6, 3, 6, 7], [0.0, -1.0, 0.0]),  // Üst yüz (-Y içe)
            ([0, 3, 7, 0, 7, 4], [ 1.0, 0.0, 0.0]),  // Sol yüz (+X içe)
            ([1, 6, 2, 1, 5, 6], [-1.0, 0.0, 0.0]),  // Sağ yüz (-X içe)
        ];

        let mut vertices = Vec::with_capacity(36);
        for (indices, normal) in &faces {
            for &idx in indices {
                vertices.push(Vertex {
                    position: positions[idx],
                    color: [1.0, 1.0, 1.0],
                    normal: *normal,
                    tex_coords: [0.0, 0.0],
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

        let aabb = gizmo_math::Aabb::new(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));
        Mesh::new(Arc::new(vbuf), vertices.len() as u32, Vec3::ZERO, "inverted_cube".to_string(), aabb)
    }

    /// Düzenli Küp mesh üretir (Dışa bakan normaller, PBR ışıklandırma ve gölgelendirme için doğru)
    pub fn create_cube(device: &wgpu::Device) -> Mesh {
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

        let faces: [([usize; 6], [f32; 3]); 6] = [
            ([0, 2, 1, 0, 3, 2], [0.0, 0.0, -1.0]),  // Arka (-Z Dışa)
            ([4, 5, 6, 4, 6, 7], [0.0, 0.0,  1.0]),  // Ön (+Z Dışa)
            ([0, 1, 5, 0, 5, 4], [0.0, -1.0, 0.0]),  // Alt (-Y Dışa)
            ([3, 6, 2, 3, 7, 6], [0.0,  1.0, 0.0]),  // Üst (+Y Dışa)
            ([0, 4, 7, 0, 7, 3], [-1.0, 0.0, 0.0]),  // Sol (-X Dışa)
            ([1, 2, 6, 1, 6, 5], [ 1.0, 0.0, 0.0]),  // Sağ (+X Dışa)
        ];

        let mut vertices = Vec::with_capacity(36);
        for (indices, normal) in &faces {
            for &idx in indices {
                vertices.push(Vertex {
                    position: positions[idx],
                    color: [1.0, 1.0, 1.0],
                    normal: *normal,
                    tex_coords: [0.0, 0.0],
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

        let aabb = gizmo_math::Aabb::new(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));
        Mesh::new(Arc::new(vbuf), vertices.len() as u32, Vec3::ZERO, "standard_cube".to_string(), aabb)
    }

    /// Basit, yatay bir düzlem (Plane) üretir.
    pub fn create_plane(device: &wgpu::Device, size: f32) -> Mesh {
        let half = size / 2.0;
        let y = 0.0;
        
        // Üstten bakışla Saat yönünün tersi (CCW) 2 üçgen (Quad)
        let def_j = [0; 4];
        let def_w = [0.0; 4];
        let vertices = [
            Vertex { position: [-half, y,  half], color: [1.0, 1.0, 1.0], normal: [0.0, 1.0, 0.0], tex_coords: [0.0, size], joint_indices: def_j, joint_weights: def_w },
            Vertex { position: [ half, y,  half], color: [1.0, 1.0, 1.0], normal: [0.0, 1.0, 0.0], tex_coords: [size, size], joint_indices: def_j, joint_weights: def_w },
            Vertex { position: [ half, y, -half], color: [1.0, 1.0, 1.0], normal: [0.0, 1.0, 0.0], tex_coords: [size, 0.0], joint_indices: def_j, joint_weights: def_w },
            
            Vertex { position: [ half, y, -half], color: [1.0, 1.0, 1.0], normal: [0.0, 1.0, 0.0], tex_coords: [size, 0.0], joint_indices: def_j, joint_weights: def_w },
            Vertex { position: [-half, y, -half], color: [1.0, 1.0, 1.0], normal: [0.0, 1.0, 0.0], tex_coords: [0.0, 0.0], joint_indices: def_j, joint_weights: def_w },
            Vertex { position: [-half, y,  half], color: [1.0, 1.0, 1.0], normal: [0.0, 1.0, 0.0], tex_coords: [0.0, size], joint_indices: def_j, joint_weights: def_w },
        ];

        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Plane VBuf"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let aabb = gizmo_math::Aabb::new(Vec3::new(-size, -0.01, -size), Vec3::new(size, 0.01, size));
        Mesh::new(Arc::new(vbuf), vertices.len() as u32, Vec3::ZERO, "plane".to_string(), aabb)
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
            Vertex { position: [-hw, -hh, 0.0], color: [1.0, 1.0, 1.0], normal: [0.0, 0.0, 1.0], tex_coords: [0.0, 1.0], joint_indices: def_j, joint_weights: def_w },
            Vertex { position: [ hw, -hh, 0.0], color: [1.0, 1.0, 1.0], normal: [0.0, 0.0, 1.0], tex_coords: [1.0, 1.0], joint_indices: def_j, joint_weights: def_w },
            Vertex { position: [ hw,  hh, 0.0], color: [1.0, 1.0, 1.0], normal: [0.0, 0.0, 1.0], tex_coords: [1.0, 0.0], joint_indices: def_j, joint_weights: def_w },
            
            Vertex { position: [ hw,  hh, 0.0], color: [1.0, 1.0, 1.0], normal: [0.0, 0.0, 1.0], tex_coords: [1.0, 0.0], joint_indices: def_j, joint_weights: def_w },
            Vertex { position: [-hw,  hh, 0.0], color: [1.0, 1.0, 1.0], normal: [0.0, 0.0, 1.0], tex_coords: [0.0, 0.0], joint_indices: def_j, joint_weights: def_w },
            Vertex { position: [-hw, -hh, 0.0], color: [1.0, 1.0, 1.0], normal: [0.0, 0.0, 1.0], tex_coords: [0.0, 1.0], joint_indices: def_j, joint_weights: def_w },
        ];

        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Sprite Quad VBuf"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let aabb = gizmo_math::Aabb::new(Vec3::new(-hw, -hh, -0.01), Vec3::new(hw, hh, 0.01));
        Mesh::new(Arc::new(vbuf), vertices.len() as u32, Vec3::ZERO, "sprite_quad".to_string(), aabb)
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

                let def_j = [0; 4];
                let def_w = [0.0; 4];
                
                // Üçgen 1
                vertices.push(Vertex { position: p1, color: [1.0; 3], normal: n1, tex_coords: uv1, joint_indices: def_j, joint_weights: def_w });
                vertices.push(Vertex { position: p2, color: [1.0; 3], normal: n2, tex_coords: uv2, joint_indices: def_j, joint_weights: def_w });
                vertices.push(Vertex { position: p3, color: [1.0; 3], normal: n3, tex_coords: uv3, joint_indices: def_j, joint_weights: def_w });
                // Üçgen 2
                vertices.push(Vertex { position: p1, color: [1.0; 3], normal: n1, tex_coords: uv1, joint_indices: def_j, joint_weights: def_w });
                vertices.push(Vertex { position: p3, color: [1.0; 3], normal: n3, tex_coords: uv3, joint_indices: def_j, joint_weights: def_w });
                vertices.push(Vertex { position: p4, color: [1.0; 3], normal: n4, tex_coords: uv4, joint_indices: def_j, joint_weights: def_w });
            }
        }

        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Sphere VBuf"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let aabb = gizmo_math::Aabb::new(Vec3::new(-radius, -radius, -radius), Vec3::new(radius, radius, radius));
        Mesh::new(Arc::new(vbuf), vertices.len() as u32, Vec3::ZERO, "sphere".to_string(), aabb)
    }

    /// Bir resmi okuyup Bind Group (Material Texture + Sampler) haline getirir
    pub fn load_material_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        path: &str,
    ) -> Result<Arc<wgpu::BindGroup>, String> {
        // Yolu normalize et — aynı dosyanın farklı path'lerle cache'te çoğalmasını önle
        let canonical = std::path::Path::new(path).canonicalize()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| path.to_string());
        let path = canonical.as_str();
        
        if let Some(cached) = self.texture_cache.get(path) {
            return Ok(cached.clone());
        }

        let img = image::open(path)
            .map_err(|e| format!("Doku yuklenirken hata! {} ({})", path, e))?
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

        let bg = Arc::new(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(path),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        }));

        self.texture_cache.insert(path.to_string(), bg.clone());
        Ok(bg)
    }

    /// Dümdüz 1x1 beyaz (katı) bir kaplama üretir. Doku içermeyen materyallerin varsayılan kaplamasıdır.
    pub fn create_white_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
    ) -> Arc<wgpu::BindGroup> {
        let path = "__white_fallback_texture__";
        if let Some(cached) = self.texture_cache.get(path) {
            return cached.clone();
        }

        let texture_size = wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            size: texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            label: Some("White Fallback Texture"),
            view_formats: &[],
        });

        // Sadece 1 piksel tam beyaz [255, 255, 255, 255]
        queue.write_texture(
            wgpu::ImageCopyTexture { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &[255, 255, 255, 255],
            wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: Some(1) },
            texture_size,
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let bg = Arc::new(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("White Fallback BindGroup"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        }));

        self.texture_cache.insert(path.to_string(), bg.clone());
        bg
    }

    /// GLTF / GLB Sahnesini yükleyerek Mesh ve Hiyerarşi ağacını parçalar.
    /// (Bu veri yapısı daha sonra motorun diğer parçaları (örn: scene builder) tarafından ECS Entity'lerine basılır)
    pub fn load_gltf_scene(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture_bind_group_layout: &wgpu::BindGroupLayout,
        default_tbind: Arc<wgpu::BindGroup>,
        file_path: &str,
    ) -> Result<GltfSceneAsset, String> {
        let (document, buffers, images) = gltf::import(file_path)
            .map_err(|e| format!("GLTF dosyasi yuklenemedi ({}). Hata: {}", file_path, e))?;

        // --- 1. RESİMLERİ TEXTURE & BINDGROUP YAPMA ---
        let mut gltf_textures = Vec::new();

        for (i, image) in images.iter().enumerate() {
            let (width, height) = (image.width, image.height);
            let texture_size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
            
            let (img_data, format, bytes_per_row) = match image.format {
                gltf::image::Format::R8G8B8A8 => (image.pixels.clone(), wgpu::TextureFormat::Rgba8UnormSrgb, 4 * width),
                gltf::image::Format::R8G8B8 => {
                    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
                    for chunk in image.pixels.chunks_exact(3) {
                        rgba.extend_from_slice(&[chunk[0], chunk[1], chunk[2], 255]);
                    }
                    (rgba, wgpu::TextureFormat::Rgba8UnormSrgb, 4 * width)
                },
                gltf::image::Format::R8G8 => {
                    // Luminance + Alpha converts to R=Lum, G=Lum, B=Lum, A=Alpha
                    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
                    for chunk in image.pixels.chunks_exact(2) {
                        rgba.extend_from_slice(&[chunk[0], chunk[0], chunk[0], chunk[1]]);
                    }
                    (rgba, wgpu::TextureFormat::Rgba8UnormSrgb, 4 * width)
                },
                gltf::image::Format::R8 => {
                    // Luminance converts to R=Lum, G=Lum, B=Lum, A=255
                    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
                    for &lum in &image.pixels {
                        rgba.extend_from_slice(&[lum, lum, lum, 255]);
                    }
                    (rgba, wgpu::TextureFormat::Rgba8UnormSrgb, 4 * width)
                },
                _ => {
                    eprintln!("[GLTF WARN] Bilinmeyen piksel formatı (image idx={}), RGBA8 fallback kullanılıyor. Boyut: {}x{}, Pixel len: {}",
                        i, width, height, image.pixels.len());
                    // Fallback to RGBA8 padding if length doesn't match standard
                    let mut rgba = vec![255; (width * height * 4) as usize];
                    // At least prevent WGPU out-of-bounds panic
                    let copy_len = image.pixels.len().min(rgba.len());
                    rgba[..copy_len].copy_from_slice(&image.pixels[..copy_len]);
                    (rgba, wgpu::TextureFormat::Rgba8UnormSrgb, 4 * width)
                }
            };

            let texture = device.create_texture(&wgpu::TextureDescriptor {
                size: texture_size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                label: Some(&format!("{}_tex_{}", file_path, i)),
                view_formats: &[],
            });

            queue.write_texture(
                wgpu::ImageCopyTexture { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                &img_data,
                wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(bytes_per_row), rows_per_image: Some(height) },
                texture_size,
            );

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

            let bg = Arc::new(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&format!("{}_bg_{}", file_path, i)),
                layout: texture_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
                ],
            }));
            gltf_textures.push(bg);
        }

        // --- 2. MATERYALLERİ OLUŞTURMA ---
        let mut gltf_materials = Vec::new();
        for material in document.materials() {
            let pbr = material.pbr_metallic_roughness();
            let base_color = pbr.base_color_factor();
            
            let mut mat = if let Some(tex_info) = pbr.base_color_texture() {
                let tex_idx = tex_info.texture().source().index();
                if let Some(bg) = gltf_textures.get(tex_idx) {
                    Material::new(bg.clone())
                } else {
                    Material::new(default_tbind.clone())
                }
            } else {
                Material::new(default_tbind.clone())
            };
            mat.albedo = gizmo_math::Vec4::new(base_color[0], base_color[1], base_color[2], base_color[3]);
            mat.metallic = pbr.metallic_factor().min(0.2); // Sınırlı! IBL olmadığı için saf metaller simsiyah kalıyor.
            mat.roughness = pbr.roughness_factor().max(0.6); // Paralamaması için pürüzlü kalsın.
            // Varsayılan: PBR açık (unlit=0.0). GLTF modelleri artık ışıklandırma alacak.
            mat.unlit = 0.0;
            
            if material.alpha_mode() == gltf::material::AlphaMode::Blend {
                mat.is_transparent = true;
            }
            if material.double_sided() {
                mat.is_double_sided = true;
            }
            
            gltf_materials.push(mat);
        }

        let mut roots = Vec::new();

        for scene in document.scenes() {
            for node in scene.nodes() {
                roots.push(self.parse_gltf_node(device, &node, &buffers, &gltf_materials, file_path));
            }
        }

        let mut animations = Vec::new();
        for anim in document.animations() {
            let mut transl = Vec::new();
            let mut rot = Vec::new();
            let mut scl = Vec::new();

            for channel in anim.channels() {
                let target_node = channel.target().node().index();
                let reader = channel.reader(|b| Some(&buffers[b.index()]));
                
                if let Some(inputs) = reader.read_inputs() {
                    let times: Vec<f32> = inputs.collect();

                    if let Some(outputs) = reader.read_outputs() {
                        match outputs {
                            gltf::animation::util::ReadOutputs::Translations(tr) => {
                                let mut kfs = Vec::new();
                                for (time, val) in times.iter().zip(tr) {
                                    kfs.push(Keyframe { time: *time, value: Vec3::new(val[0], val[1], val[2]) });
                                }
                                transl.push(Track { target_node, keyframes: kfs });
                            },
                            gltf::animation::util::ReadOutputs::Rotations(rt) => {
                                let mut kfs = Vec::new();
                                for (time, val) in times.iter().zip(rt.into_f32()) {
                                    kfs.push(Keyframe { time: *time, value: Quat::from_xyzw(val[0], val[1], val[2], val[3]) });
                                }
                                rot.push(Track { target_node, keyframes: kfs });
                            },
                            gltf::animation::util::ReadOutputs::Scales(sc) => {
                                let mut kfs = Vec::new();
                                for (time, val) in times.iter().zip(sc) {
                                    kfs.push(Keyframe { time: *time, value: Vec3::new(val[0], val[1], val[2]) });
                                }
                                scl.push(Track { target_node, keyframes: kfs });
                            },
                            _ => {} // Morph targets vb. goz ardi edildi
                        }
                    }
                }
            }

            animations.push(AnimationClip {
                name: anim.name().unwrap_or("unnamed_anim").to_string(),
                duration: 0.0, // Hesaplamamiz lazim (track'lerin son keyframe time'larinin max'i)
                translations: transl,
                rotations: rot,
                scales: scl,
            });
        }

        // Sure hesaplama sonradan yapilabilir
        for anim in &mut animations {
            let mut max_t = 0.0f32;
            for t in &anim.translations { if let Some(k) = t.keyframes.last() { max_t = max_t.max(k.time); } }
            for t in &anim.rotations { if let Some(k) = t.keyframes.last() { max_t = max_t.max(k.time); } }
            for t in &anim.scales { if let Some(k) = t.keyframes.last() { max_t = max_t.max(k.time); } }
            anim.duration = max_t;
        }

        let mut node_parents = std::collections::HashMap::new();
        for node in document.nodes() {
            for child in node.children() {
                node_parents.insert(child.index(), node.index());
            }
        }

        let mut skeletons = Vec::new();
        for skin in document.skins() {
            let reader = skin.reader(|b| Some(&buffers[b.index()]));
            let ibm: Vec<[[f32; 4]; 4]> = reader.read_inverse_bind_matrices()
                .map(|v| v.collect())
                .unwrap_or_else(|| {
                    vec![[[1.0,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.],[0.,0.,0.,1.]]; skin.joints().count()]
                });
            
            let mut node_to_bone = std::collections::HashMap::new();
            for (bone_idx, node) in skin.joints().enumerate() {
                node_to_bone.insert(node.index(), bone_idx);
            }

            let mut joints = Vec::new();
            for (bone_idx, joint_node) in skin.joints().enumerate() {
                let inverse_bind_matrix = gizmo_math::Mat4::from_cols_array_2d(&ibm[bone_idx]);
                
                let parent_index = node_parents.get(&joint_node.index()).and_then(|p| node_to_bone.get(p).copied());

                let (t, r, s) = joint_node.transform().decomposed();
                let loc_t = gizmo_math::Mat4::from_translation(Vec3::new(t[0], t[1], t[2]));
                let loc_r = gizmo_math::Mat4::from_quat(Quat::from_array(r));
                let loc_s = gizmo_math::Mat4::from_scale(Vec3::new(s[0], s[1], s[2]));
                let local_bind_transform = loc_t * loc_r * loc_s;

                joints.push(SkeletonJoint {
                    name: joint_node.name().unwrap_or("bone").to_string(),
                    node_index: joint_node.index(),
                    inverse_bind_matrix,
                    parent_index,
                    local_bind_transform,
                });
            }

            skeletons.push(SkeletonHierarchy { joints });
        }

        Ok(GltfSceneAsset {
            roots,
            animations,
            skeletons,
        })
    }

    fn parse_gltf_node(&mut self, device: &wgpu::Device, node: &gltf::Node, buffers: &[gltf::buffer::Data], materials: &[Material], file_name: &str) -> GltfNodeData {
        let (translation, rotation, scale) = node.transform().decomposed();
        
        let mut primitives = Vec::new();
        if let Some(mesh) = node.mesh() {
            for (prim_i, primitive) in node.mesh().unwrap().primitives().enumerate() {
                let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));
                
                let positions = reader.read_positions().map(|v| v.collect::<Vec<_>>()).unwrap_or_default();
                let normals = reader.read_normals().map(|v| v.collect::<Vec<_>>()).unwrap_or_else(|| vec![[0.0, 1.0, 0.0]; positions.len()]);
                let tex_coords = reader.read_tex_coords(0).map(|v| v.into_f32().collect::<Vec<_>>()).unwrap_or_else(|| vec![[0.0, 0.0]; positions.len()]);
                
                let joints = reader.read_joints(0).map(|v| v.into_u16().collect::<Vec<_>>());
                let weights = reader.read_weights(0).map(|v| v.into_f32().collect::<Vec<_>>());

                let get_vertex = |i: usize, pos: [f32; 3]| -> Vertex {
                    let j = if let Some(ref js) = joints {
                        if i < js.len() { [js[i][0] as u32, js[i][1] as u32, js[i][2] as u32, js[i][3] as u32] } else { [0; 4] }
                    } else { [0; 4] };

                    let w = if let Some(ref ws) = weights {
                        if i < ws.len() { ws[i] } else { [0.0; 4] }
                    } else { [0.0; 4] };

                    Vertex {
                        position: pos,
                        normal: normals[i],
                        tex_coords: tex_coords[i],
                        color: [1.0, 1.0, 1.0],
                        joint_indices: j,
                        joint_weights: w,
                    }
                };

                let mut all_vertices = Vec::new();
                let mut aabb = gizmo_math::Aabb::empty();

                if let Some(indices) = reader.read_indices() {
                    let indices_u32: Vec<u32> = indices.into_u32().collect();
                    for idx in indices_u32 {
                        let i = idx as usize;
                        if i < positions.len() {
                            let pos = positions[i];
                            aabb.extend(Vec3::new(pos[0], pos[1], pos[2]));
                            all_vertices.push(get_vertex(i, pos));
                        }
                    }
                } else {
                    for i in 0..positions.len() {
                        let pos = positions[i];
                        aabb.extend(Vec3::new(pos[0], pos[1], pos[2]));
                        all_vertices.push(get_vertex(i, pos));
                    }
                }

                let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(&format!("GLTF VBuf: {}_prim{}", file_name, prim_i)),
                    contents: bytemuck::cast_slice(&all_vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });

                let mesh_comp = Mesh::new(
                    Arc::new(vbuf), 
                    all_vertices.len() as u32, 
                    Vec3::ZERO, 
                    format!("gltf_mesh_{}_{:?}_p{}", file_name, node.name(), prim_i), 
                    aabb
                );
                
                let mat_opt = primitive.material().index().and_then(|idx| materials.get(idx).cloned());
                primitives.push((mesh_comp, mat_opt));
            }
        }

        let mut children = Vec::new();
        for child in node.children() {
            children.push(self.parse_gltf_node(device, &child, buffers, materials, file_name));
        }

        GltfNodeData {
            index: node.index(),
            name: node.name().map(|n| n.to_string()),
            translation,
            rotation,
            scale,
            primitives,
            children,
        }
    }
}

pub struct GltfNodeData {
    pub index: usize,
    pub name: Option<String>,
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
    pub primitives: Vec<(Mesh, Option<Material>)>,
    pub children: Vec<GltfNodeData>,
}

pub struct GltfSceneAsset {
    pub roots: Vec<GltfNodeData>,
    pub animations: Vec<AnimationClip>,
    pub skeletons: Vec<SkeletonHierarchy>,
}
