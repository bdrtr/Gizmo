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

        // --- ASSET PIPELINE: AUTO BC7 COMPRESSION ---
        let path = std::path::Path::new(&resolved_path);
        if path.extension().map_or(false, |ext| ext == "png" || ext == "jpg" || ext == "jpeg") {
            let dds_path = path.with_extension("dds");
            
            // Check if DDS doesn't exist or is older than the original image
            let needs_cooking = if !dds_path.exists() {
                true
            } else {
                let img_time = std::fs::metadata(&path).and_then(|m| m.modified()).unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                let dds_time = std::fs::metadata(&dds_path).and_then(|m| m.modified()).unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                img_time > dds_time
            };

            if needs_cooking {
                println!("🛠 Asset Pipeline: Baking BC7 compressed texture for {:?}", path);
                if let Ok(img) = image::open(&path) {
                    let rgba = img.to_rgba8();
                    // Compress to BC7
                    if let Ok(dds) = image_dds::dds_from_image(
                        &rgba,
                        image_dds::ImageFormat::BC7RgbaUnormSrgb,
                        image_dds::Quality::Fast,
                        image_dds::Mipmaps::GeneratedAutomatic,
                    ) {
                        if let Ok(mut file) = std::fs::File::create(&dds_path) {
                            let _ = dds.write(&mut file);
                        }
                    }
                }
            }

            // Load DDS
            if dds_path.exists() {
                if let Ok(mut file) = std::fs::File::open(&dds_path) {
                    if let Ok(dds) = ddsfile::Dds::read(&mut file) {
                        let w = dds.get_width();
                        let h = dds.get_height();
                        let mip_count = dds.get_num_mipmap_levels();
                        let texture_size = wgpu::Extent3d {
                            width: w,
                            height: h,
                            depth_or_array_layers: 1,
                        };
                        let texture = device.create_texture(&wgpu::TextureDescriptor {
                            size: texture_size,
                            mip_level_count: mip_count,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: wgpu::TextureFormat::Bc7RgbaUnormSrgb,
                            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                            label: Some(&id_str),
                            view_formats: &[],
                        });

                        if let Ok(data) = dds.get_data(0) {
                            let mut offset = 0;
                            for mip in 0..mip_count {
                                let mip_w = (w >> mip).max(1);
                                let mip_h = (h >> mip).max(1);
                                
                                let blocks_x = (mip_w + 3) / 4;
                                let blocks_y = (mip_h + 3) / 4;
                                let mip_bytes = (blocks_x * blocks_y * 16) as usize;
                                
                                if offset + mip_bytes <= data.len() {
                                    queue.write_texture(
                                        wgpu::ImageCopyTexture {
                                            texture: &texture,
                                            mip_level: mip,
                                            origin: wgpu::Origin3d::ZERO,
                                            aspect: wgpu::TextureAspect::All,
                                        },
                                        &data[offset..offset + mip_bytes],
                                        wgpu::ImageDataLayout {
                                            offset: 0,
                                            bytes_per_row: Some(blocks_x * 16),
                                            rows_per_image: Some(blocks_y),
                                        },
                                        wgpu::Extent3d {
                                            width: mip_w,
                                            height: mip_h,
                                            depth_or_array_layers: 1,
                                        },
                                    );
                                }
                                offset += mip_bytes;
                            }
                        }

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
                            label: Some(&id_str),
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

                        self.texture_cache.insert(id_str.clone(), bg.clone());
                        return Ok(bg);
                    }
                }
            }
        }
        // --- END ASSET PIPELINE ---

        // Fallback to raw RGBA
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

    pub fn create_checkerboard_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
    ) -> Arc<wgpu::BindGroup> {
        let path = "__checkerboard_texture__";
        if let Some(cached) = self.texture_cache.get(path) {
            return cached.clone();
        }

        let size = 256u32;
        let mut pixels = vec![0u8; (size * size * 4) as usize];
        for y in 0..size {
            for x in 0..size {
                let is_white = ((x / 32) + (y / 32)) % 2 == 0;
                let color = if is_white { 200u8 } else { 50u8 };
                let idx = ((y * size + x) * 4) as usize;
                pixels[idx] = color;     // R
                pixels[idx + 1] = color; // G
                pixels[idx + 2] = color; // B
                pixels[idx + 3] = 255;   // A
            }
        }

        self.install_decoded_material_texture(device, queue, layout, path, &pixels, size, size).unwrap()
    }
}
