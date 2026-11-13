use super::decode_rgba_image_file;
use super::error::AssetError;
use std::sync::Arc;

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
    fn resolve_texture_cache_key(
        &self,
        path_or_uuid: &str,
    ) -> Result<(String, String), AssetError> {
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
    ) -> Result<Arc<wgpu::BindGroup>, AssetError> {
        // Guard against zero-sized textures — wgpu panics on Extent3d { width:0, .. }.
        if width == 0 || height == 0 {
            return Err(AssetError::ZeroDimensionTexture {
                cache_key: cache_key.to_string(),
                width,
                height,
            });
        }

        let expected = (width as usize)
            .saturating_mul(height as usize)
            .saturating_mul(4);

        if rgba.len() != expected {
            return Err(AssetError::RgbaSizeMismatch {
                cache_key: cache_key.to_string(),
                got: rgba.len(),
                expected,
                width,
                height,
            });
        }

        let (view, mip_level_count) =
            Self::upload_rgba_texture(device, queue, cache_key, rgba, width, height, wgpu::TextureFormat::Rgba8UnormSrgb);
        let sampler = crate::texture_quality::material_sampler(
            device,
            "linear_repeat_sampler",
            wgpu::AddressMode::Repeat,
            wgpu::AddressMode::Repeat,
            wgpu::FilterMode::Linear,
            wgpu::FilterMode::Linear,
            mip_level_count > 1,
        );
        let bg = self
            .assemble_single_texture_bind_group(device, queue, layout, &view, &sampler, cache_key);
        self.texture_cache.insert(cache_key.to_string(), bg.clone());
        Ok(bg)
    }

    /// Uploads checked RGBA8 pixels as a mipmapped texture of `format` and returns its view and its
    /// mip count. `format` is sRGB for a colour, linear for data (a normal or a packed map).
    fn upload_rgba_texture(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        label: &str,
        rgba: &[u8],
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> (wgpu::TextureView, u32) {
        let texture_size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        let mip_level_count = crate::texture_quality::mip_level_count(width, height);

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            size: texture_size,
            mip_level_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: crate::texture_quality::MIPPED_TEXTURE_USAGE,
            label: Some(label),
            view_formats: &[],
        });

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: Some(height),
            },
            texture_size,
        );

        if mip_level_count > 1 {
            crate::texture_quality::MipmapBlitter::new(device, format)
                .generate(device, queue, &texture, mip_level_count);
        }

        (texture.create_view(&wgpu::TextureViewDescriptor::default()), mip_level_count)
    }

    /// A texture's view, read from `path_or_uuid` as [`load_material_texture`](Self::load_material_texture)
    /// reads one, as sRGB (`srgb`, a colour) or linear (data), uploaded once per path and format.
    ///
    /// # Errors
    ///
    /// As [`load_material_texture`](Self::load_material_texture): no such file or asset, an image that
    /// does not decode, or one of zero size.
    pub fn load_texture_view(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        path_or_uuid: &str,
        srgb: bool,
    ) -> Result<Arc<wgpu::TextureView>, AssetError> {
        // An image inside a glTF file (`gltf_image_key`): there when the file has been imported in
        // this run, and imported now when it has not — a scene that reopens names the textures of a
        // model before or after the mesh loader has read the file, and either order finds them.
        if let Some((file, _)) = super::loaders::split_gltf_image_key(path_or_uuid) {
            let key = (path_or_uuid.to_string(), srgb);
            if let Some(view) = self.view_cache.get(&key) {
                return Ok(view.clone());
            }
            let white = self.create_white_texture(device, queue, layout);
            self.load_gltf_scene(device, queue, layout, white, file)?;
            return self
                .view_cache
                .get(&key)
                .cloned()
                .ok_or_else(|| AssetError::GltfImageMissing { key: path_or_uuid.to_string() });
        }
        let (resolved_path, cache_key) = self.resolve_texture_cache_key(path_or_uuid)?;
        if let Some(view) = self.view_cache.get(&(cache_key.clone(), srgb)) {
            return Ok(view.clone());
        }
        let (rgba, width, height) = self.decode_texture_rgba(&resolved_path)?;
        if width == 0 || height == 0 {
            return Err(AssetError::ZeroDimensionTexture { cache_key, width, height });
        }
        let format = if srgb { wgpu::TextureFormat::Rgba8UnormSrgb } else { wgpu::TextureFormat::Rgba8Unorm };
        let (view, _) = Self::upload_rgba_texture(device, queue, &cache_key, &rgba, width, height, format);
        let view = Arc::new(view);
        self.view_cache.insert((cache_key, srgb), view.clone());
        Ok(view)
    }

    /// The bind group a material with base colour `base` and `maps` draws with, built once per set of
    /// sources and shared after.
    ///
    /// With no maps it is [`load_material_texture`](Self::load_material_texture)'s (or the white one's,
    /// for no base colour), so a material that names only a base colour shares the bind group it always
    /// had. A texture that does not load is warned about and drawn as the neutral one in its slot: one
    /// missing file costs that map, not the material.
    pub fn load_material(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        base: Option<&str>,
        maps: &crate::components::MaterialMaps,
    ) -> Arc<wgpu::BindGroup> {
        // The shared default buffer is `MaterialParams::default()`: strengths that ask for nothing
        // else need no buffer of their own.
        let params_neutral =
            bytemuck::bytes_of(&maps.params()) == bytemuck::bytes_of(&crate::gpu_types::MaterialParams::default());
        // A glTF image has no single-texture bind group of its own: it goes through the builder.
        let gltf_base = base.is_some_and(|b| super::loaders::split_gltf_image_key(b).is_some());
        if maps.is_empty() && params_neutral && maps.default_sampling() && !gltf_base {
            return match base {
                Some(path) => self.load_material_texture(device, queue, layout, path).unwrap_or_else(|error| {
                    tracing::warn!(path, %error, "[Material] base colour texture would not load; drawing white");
                    self.create_white_texture(device, queue, layout)
                }),
                None => self.create_white_texture(device, queue, layout),
            };
        }

        let key = format!("{base:?}|{maps:?}");
        if let Some(cached) = self.material_cache.get(&key) {
            return cached.clone();
        }
        let mut view = |path: &Option<String>, srgb: bool, slot: &str| -> Option<Arc<wgpu::TextureView>> {
            let path = path.as_deref()?;
            self.load_texture_view(device, queue, layout, path, srgb)
                .map_err(|error| tracing::warn!(path, slot, %error, "[Material] map would not load; drawing the neutral one"))
                .ok()
        };
        let base_view = view(&base.map(str::to_string), true, "base colour");
        let normal = view(&maps.normal, false, "normal");
        let metallic_roughness = view(&maps.metallic_roughness, false, "metallic-roughness");
        let emissive = view(&maps.emissive, true, "emissive");
        let occlusion = view(&maps.occlusion, false, "occlusion");
        let params = {
            use wgpu::util::DeviceExt;
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("material_params"),
                contents: bytemuck::cast_slice(&[maps.params()]),
                usage: wgpu::BufferUsages::UNIFORM,
            })
        };
        // The sampler only when the material asks for another: the builder's default is tiled and
        // blended with mipmaps, the one every texture is read through unless told otherwise.
        let sampler = (!maps.default_sampling()).then(|| {
            crate::texture_quality::material_sampler(
                device,
                "material_maps_sampler",
                maps.wrap_u.address_mode(),
                maps.wrap_v.address_mode(),
                maps.mag_filter.filter_mode(),
                maps.min_filter.filter_mode(),
                true,
            )
        });
        let mut builder = super::AssetManager::material().params(&params).label("material_maps");
        if let Some(s) = sampler.as_ref() {
            builder = builder.sampler(s);
        }
        if let Some(v) = base_view.as_deref() {
            builder = builder.base_colour(v);
        }
        if let Some(v) = normal.as_deref() {
            builder = builder.normal(v);
        }
        if let Some(v) = metallic_roughness.as_deref() {
            builder = builder.metallic_roughness(v);
        }
        if let Some(v) = emissive.as_deref() {
            builder = builder.emissive(v);
        }
        if let Some(v) = occlusion.as_deref() {
            builder = builder.occlusion(v);
        }
        let bind_group = builder.build(self, device, queue, layout);
        self.material_cache.insert(key, bind_group.clone());
        bind_group
    }

    // ── Public load API ───────────────────────────────────────────────────

    /// Load a texture from `path_or_uuid`, uploading it to the GPU on first
    /// access and returning the cached bind group on subsequent calls.
    ///
    /// Supports both filesystem paths and UUID strings registered by the asset
    /// scanner.  Embedded assets (registered with [`AssetManager::embed_asset`](Self::embed_asset))
    /// take priority over filesystem reads.
    pub fn load_material_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        path_or_uuid: &str,
    ) -> Result<Arc<wgpu::BindGroup>, AssetError> {
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
    ) -> Result<Arc<wgpu::BindGroup>, AssetError> {
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

        // A fully-neutral material: white base + flat normal + white MR/emissive/AO
        // + default params. Everything sourced from the shared defaults.
        self.ensure_material_defaults(device, queue);
        let d = self
            .material_defaults()
            .expect("material defaults ensured above");
        let bg = Self::assemble_material_bind_group(
            device,
            layout,
            &d.white_view,
            &d.sampler,
            &d.flat_normal_view,
            &d.white_view,
            &d.white_view,
            &d.white_view,
            &d.params_buffer,
            KEY,
        );
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

    /// A generated checkerboard with coloured axes, for seeing a mesh's UV layout — stretched,
    /// mirrored or wrapped texture coordinates are obvious on it and invisible on an ordinary map.
    pub fn create_uv_debug_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
    ) -> Arc<wgpu::BindGroup> {
        const KEY: &str = "__uv_debug_texture__";
        const TEXTURE_SIZE: usize = 8;

        if let Some(cached) = self.texture_cache.get(KEY) {
            return cached.clone();
        }

        let mut palette: [u8; 32] = [
            255, 102, 159, 255, 255, 159, 102, 255, 236, 255, 102, 255, 121, 255, 102, 255, 102, 255,
            198, 255, 102, 198, 255, 255, 121, 102, 255, 255, 236, 102, 255, 255,
        ];

        let mut texture_data = [0u8; TEXTURE_SIZE * TEXTURE_SIZE * 4];
        for y in 0..TEXTURE_SIZE {
            let offset = TEXTURE_SIZE * y * 4;
            texture_data[offset..(offset + TEXTURE_SIZE * 4)].copy_from_slice(&palette);
            palette.rotate_right(4);
        }

        self.install_decoded_material_texture(
            device,
            queue,
            layout,
            KEY,
            &texture_data,
            TEXTURE_SIZE as u32,
            TEXTURE_SIZE as u32,
        )
        .expect("uv debug texture creation must not fail")
    }

    // ── Private GPU helpers ───────────────────────────────────────────────

    /// Decode a texture file to RGBA8, preferring embedded data over disk.
    fn decode_texture_rgba(&self, resolved_path: &str) -> Result<(Vec<u8>, u32, u32), AssetError> {
        if let Some(data) = self.embedded_assets.get(resolved_path) {
            let img = image::load_from_memory(data)
                .map_err(|source| AssetError::ImageDecode {
                    path: std::path::PathBuf::from(resolved_path),
                    source,
                })?
                .to_rgba8();
            let (w, h) = img.dimensions();
            return Ok((img.into_raw(), w, h));
        }

        decode_rgba_image_file(resolved_path)
    }
}
