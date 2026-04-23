use super::decode_rgba_image_file;
use std::sync::Arc;

impl super::AssetManager {
    /// Decode edilmiş RGBA8'i GPU'ya yükler ve `texture_cache`'e yazar ([`crate::async_assets::AsyncAssetLoader`] tamamlanınca).
    pub fn install_decoded_material_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        cache_key: &str,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Result<Arc<wgpu::BindGroup>, String> {
        let expected = (width as usize)
            .saturating_mul(height as usize)
            .saturating_mul(4);
        if rgba.len() != expected {
            return Err(format!(
                "RGBA boyut uyumsuz: {} byte, beklenen {} ({}x{}x4)",
                rgba.len(),
                expected,
                width,
                height
            ));
        }

        let texture_size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            size: texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            label: Some(cache_key),
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: Some(height),
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
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let bg = Arc::new(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(cache_key),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        }));

        self.texture_cache.insert(cache_key.to_string(), bg.clone());
        Ok(bg)
    }

    /// Bir resmi okuyup Bind Group (Material Texture + Sampler) haline getirir
    pub fn load_material_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        path_or_uuid: &str,
    ) -> Result<Arc<wgpu::BindGroup>, String> {
        let resolved_path = self.resolve_path_from_meta_source(path_or_uuid)?;

        let id_str = if let Some(id) = self.get_uuid(&resolved_path) {
            id.to_string()
        } else {
            resolved_path.clone()
        };

        if let Some(cached) = self.texture_cache.get(&id_str) {
            return Ok(cached.clone());
        }

        let (rgba, w, h) = decode_rgba_image_file(&resolved_path)?;
        self.install_decoded_material_texture(device, queue, layout, &id_str, &rgba, w, h)
    }

    /// Cache'i zorla silerek bir dokunun diskten tekrar yüklenmesini ve Bind Group'un güncellenmesini sağlar
    pub fn reload_material_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        path_or_uuid: &str,
    ) -> Result<Arc<wgpu::BindGroup>, String> {
        let resolved_path = self.resolve_path_from_meta_source(path_or_uuid)?;
        let id_str = if let Some(id) = self.get_uuid(&resolved_path) {
            id.to_string()
        } else {
            resolved_path.clone()
        };

        self.texture_cache.remove(&id_str);
        self.load_material_texture(device, queue, layout, path_or_uuid)
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

        let texture_size = wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        };
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
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &[255, 255, 255, 255],
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
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
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        }));

        self.texture_cache.insert(path.to_string(), bg.clone());
        bg
    }
}
