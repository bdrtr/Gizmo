use crate::components::Mesh;
use crate::renderer::Vertex;
use gizmo_math::Vec3;
use std::sync::Arc;
use wgpu::util::DeviceExt;

impl crate::asset::AssetManager {
    pub fn create_terrain(
        device: &wgpu::Device,
        heightmap_path: &str,
        width: f32,
        depth: f32,
        max_height: f32,
    ) -> Result<(Mesh, Vec<f32>, u32, u32), crate::asset::error::AssetError> {
        let canonical = std::path::Path::new(heightmap_path)
            .canonicalize()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| heightmap_path.to_string());

        let img = image::open(&canonical)
            .map_err(|source| crate::asset::error::AssetError::ImageDecode {
                path: std::path::PathBuf::from(&canonical),
                source,
            })?
            .into_luma8(); // Grayscale format

        let (img_width, img_height) = img.dimensions();
        if img_width < 2 || img_height < 2 {
            return Err(crate::asset::error::AssetError::HeightmapTooSmall {
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
