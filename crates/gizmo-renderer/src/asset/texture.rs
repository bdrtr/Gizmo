use super::decode_rgba_image_file;
use std::sync::Arc;

// ============================================================================
//  Shared sampler descriptors
// ============================================================================

/// Standard sampler for real textures: bilinear, repeating.
/// Mipmap filter is Nearest because we only allocate one mip level —
/// using Linear here would trigger a wgpu validation warning.
const SAMPLER_LINEAR_REPEAT: wgpu::SamplerDescriptor<'static> = wgpu::SamplerDescriptor {
    label: Some("linear_repeat_sampler"),
    address_mode_u: wgpu::AddressMode::Repeat,
    address_mode_v: wgpu::AddressMode::Repeat,
    address_mode_w: wgpu::AddressMode::Repeat,
    mag_filter: wgpu::FilterMode::Linear,
    min_filter: wgpu::FilterMode::Linear,
    mipmap_filter: wgpu::FilterMode::Nearest, // single mip — must be Nearest
    lod_min_clamp: 0.0,
    lod_max_clamp: 0.0,
    compare: None,
    anisotropy_clamp: 1,
    border_color: None,
};

/// Point sampler for 1×1 fallback textures — no filtering needed.
const SAMPLER_NEAREST_REPEAT: wgpu::SamplerDescriptor<'static> = wgpu::SamplerDescriptor {
    label: Some("nearest_repeat_sampler"),
    address_mode_u: wgpu::AddressMode::Repeat,
    address_mode_v: wgpu::AddressMode::Repeat,
    address_mode_w: wgpu::AddressMode::Repeat,
    mag_filter: wgpu::FilterMode::Nearest,
    min_filter: wgpu::FilterMode::Nearest,
    mipmap_filter: wgpu::FilterMode::Nearest,
    lod_min_clamp: 0.0,
    lod_max_clamp: 0.0,
    compare: None,
    anisotropy_clamp: 1,
    border_color: None,
};

// ============================================================================
//  AssetManager — texture methods
// ============================================================================

impl super::AssetManager {
    // ── Internal helpers ──────────────────────────────────────────────────

    /// Resolve a `path_or_uuid` argument to the string key used in
    /// `texture_cache`.  Returns `(resolved_fs_path, cache_key)`.
    ///
    /// The cache key is the UUID string when one is registered, otherwise
    /// the normalised filesystem path.  Keeping cache keys stable across
    /// renames is why UUIDs are preferred.
    fn resolve_texture_cache_key(&self, path_or_uuid: &str) -> Result<(String, String), String> {
        let resolved = self.resolve_path_from_meta_source(path_or_uuid)?;
        let cache_key = self
            .get_uuid(&resolved)
            .map(|id| id.to_string())
            .unwrap_or_else(|| resolved.clone());
        Ok((resolved, cache_key))
    }

    /// Upload a single RGBA8 pixel buffer to the GPU, cache the bind group,
    /// and return it.
    ///
    /// Called by async loaders after decoding completes on a worker thread,
    /// and by the procedural texture helpers below.
    ///
    /// # Errors
    ///
    /// Returns an error when:
    /// * `width` or `height` is zero (wgpu would panic on a zero-sized texture).
    /// * `rgba.len()` does not equal `width * height * 4`.
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
        // Guard against zero-sized textures — wgpu panics on Extent3d { width:0, .. }.
        if width == 0 || height == 0 {
            return Err(format!(
                "Cannot create texture with zero dimension: {width}×{height} (key={cache_key})"
            ));
        }

        let expected = (width as usize)
            .saturating_mul(height as usize)
            .saturating_mul(4);

        if rgba.len() != expected {
            return Err(format!(
                "RGBA size mismatch for '{cache_key}': got {} bytes, expected {expected} \
                 ({width}×{height}×4)",
                rgba.len()
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

        let bg = self.build_bind_group(device, &texture, layout, &SAMPLER_LINEAR_REPEAT, cache_key);
        self.texture_cache.insert(cache_key.to_string(), bg.clone());
        Ok(bg)
    }

    // ── Public load API ───────────────────────────────────────────────────

    /// Load a texture from `path_or_uuid`, uploading it to the GPU on first
    /// access and returning the cached bind group on subsequent calls.
    ///
    /// Supports both filesystem paths and UUID strings registered by the asset
    /// scanner.  Embedded assets (registered with [`AssetManager::embed_asset`])
    /// take priority over filesystem reads.
    pub fn load_material_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        path_or_uuid: &str,
    ) -> Result<Arc<wgpu::BindGroup>, String> {
        let (resolved_path, cache_key) = self.resolve_texture_cache_key(path_or_uuid)?;

        if let Some(cached) = self.texture_cache.get(&cache_key) {
            return Ok(cached.clone());
        }

        let (rgba, w, h) = self.decode_texture_rgba(&resolved_path)?;
        self.install_decoded_material_texture(device, queue, layout, &cache_key, &rgba, w, h)
    }

    /// Evict `path_or_uuid` from the texture cache and reload it from disk.
    ///
    /// Useful for hot-reload workflows where an asset file changes at runtime.
    pub fn reload_material_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        path_or_uuid: &str,
    ) -> Result<Arc<wgpu::BindGroup>, String> {
        // Resolve the key first so we evict the correct entry, then reload.
        let (_, cache_key) = self.resolve_texture_cache_key(path_or_uuid)?;
        self.texture_cache.remove(&cache_key);
        self.load_material_texture(device, queue, layout, path_or_uuid)
    }

    // ── Procedural textures ───────────────────────────────────────────────

    /// Return (creating once) a 1×1 opaque-white texture.
    ///
    /// Used as the default albedo map for materials that specify no texture.
    pub fn create_white_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
    ) -> Arc<wgpu::BindGroup> {
        const KEY: &str = "__white_fallback_texture__";

        if let Some(cached) = self.texture_cache.get(KEY) {
            return cached.clone();
        }

        let bg = self.upload_solid_1x1(device, queue, layout, [255, 255, 255, 255], KEY);
        self.texture_cache.insert(KEY.to_string(), bg.clone());
        bg
    }

    /// Return (creating once) a 256×256 grey checkerboard texture.
    ///
    /// Used for geometry whose material has no texture assigned — makes UVs
    /// immediately visible in the editor.
    pub fn create_checkerboard_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
    ) -> Arc<wgpu::BindGroup> {
        const KEY: &str = "__checkerboard_texture__";
        const SIZE: u32 = 256;
        const CELL: u32 = 32; // pixels per checker square

        if let Some(cached) = self.texture_cache.get(KEY) {
            return cached.clone();
        }

        let mut pixels = vec![0u8; (SIZE * SIZE * 4) as usize];
        for y in 0..SIZE {
            for x in 0..SIZE {
                let light = ((x / CELL) + (y / CELL)).is_multiple_of(2);
                let luma = if light { 200u8 } else { 50u8 };
                let base = ((y * SIZE + x) * 4) as usize;
                pixels[base] = luma;
                pixels[base + 1] = luma;
                pixels[base + 2] = luma;
                pixels[base + 3] = 255;
            }
        }

        // SIZE and pixel count are compile-time constants; this cannot fail.

        self.install_decoded_material_texture(device, queue, layout, KEY, &pixels, SIZE, SIZE)
            .expect("checkerboard texture creation must not fail")
    }

    // ── Private GPU helpers ───────────────────────────────────────────────

    /// Decode a texture file to RGBA8, preferring embedded data over disk.
    fn decode_texture_rgba(&self, resolved_path: &str) -> Result<(Vec<u8>, u32, u32), String> {
        if let Some(data) = self.embedded_assets.get(resolved_path) {
            let img = image::load_from_memory(data)
                .map_err(|e| format!("Embedded texture decode failed ({resolved_path}): {e}"))?
                .to_rgba8();
            let (w, h) = img.dimensions();
            return Ok((img.into_raw(), w, h));
        }

        decode_rgba_image_file(resolved_path)
    }

    /// Upload a single RGBA pixel as a 1×1 texture and return its bind group.
    ///
    /// Uses the nearest-neighbour sampler — filtering a 1-pixel texture is
    /// meaningless.
    fn upload_solid_1x1(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        pixel: [u8; 4],
        label: &str,
    ) -> Arc<wgpu::BindGroup> {
        let size = wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        };

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            label: Some(label),
            view_formats: &[],
        });

        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &pixel,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            size,
        );

        self.build_bind_group(device, &texture, layout, &SAMPLER_NEAREST_REPEAT, label)
    }

    /// Create a texture view + sampler and assemble a bind group.
    ///
    /// Centralises the boilerplate that would otherwise be duplicated in every
    /// upload path.
    fn build_bind_group(
        &self,
        device: &wgpu::Device,
        texture: &wgpu::Texture,
        layout: &wgpu::BindGroupLayout,
        sampler_desc: &wgpu::SamplerDescriptor,
        label: &str,
    ) -> Arc<wgpu::BindGroup> {
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(sampler_desc);

        Arc::new(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
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
        }))
    }
}
