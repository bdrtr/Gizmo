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
        // Baked light is a quantity of light, not a colour on a surface: read linear.
        let lightmap = view(&maps.lightmap, false, "lightmap");
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
        if let Some(v) = lightmap.as_deref() {
            builder = builder.lightmap(v);
        }
        let bind_group = builder.build(self, device, queue, layout);
        // **Entries no material holds any more go before a new one comes in.** The key holds the
        // strengths (`params`), so every value an animated `emissive_factor` or UV offset takes is a
        // key of its own: kept for good, that was a uniform buffer and a bind group per frame the
        // value moved, for as long as the renderer lived. A material — the game's, the render
        // world's copy of it — holds its bind group's `Arc`; one held by the cache alone is drawn by
        // nothing, and a material that comes back to those sources builds it again (the textures
        // themselves stay, in `view_cache`). Swept on a miss only: the cache grows on nothing else.
        self.material_cache.retain(|_, held| Arc::strong_count(held) > 1);
        self.material_cache.insert(key, bind_group.clone());
        bind_group
    }

    /// How many material bind groups [`load_material`](Self::load_material) is keeping.
    #[cfg(test)]
    pub(crate) fn material_cache_len(&self) -> usize {
        self.material_cache.len()
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
        // A texture the engine generates is made again, not looked for on disk: its name is what a
        // material built over one is saved with (`texture_source_of`).
        match path_or_uuid {
            Self::CHECKERBOARD_TEXTURE => return Ok(self.create_checkerboard_texture(device, queue, layout)),
            Self::UV_DEBUG_TEXTURE => return Ok(self.create_uv_debug_texture(device, queue, layout)),
            Self::WHITE_TEXTURE => return Ok(self.create_white_texture(device, queue, layout)),
            _ => {}
        }
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

    /// The name [`create_white_texture`](Self::create_white_texture)'s bind group is cached under.
    pub const WHITE_TEXTURE: &'static str = "__white_fallback_texture__";
    /// The name [`create_checkerboard_texture`](Self::create_checkerboard_texture)'s is cached under,
    /// and what a material over it is saved as.
    pub const CHECKERBOARD_TEXTURE: &'static str = "__checkerboard_texture__";
    /// The same for [`create_uv_debug_texture`](Self::create_uv_debug_texture).
    pub const UV_DEBUG_TEXTURE: &'static str = "__uv_debug_texture__";

    /// Whether `key` names a texture the engine generates rather than a file.
    #[must_use]
    pub fn is_generated_texture_key(key: &str) -> bool {
        matches!(key, Self::WHITE_TEXTURE | Self::CHECKERBOARD_TEXTURE | Self::UV_DEBUG_TEXTURE)
    }

    /// The name a material bind group from this manager's texture cache was made under — a file's
    /// path or its asset UUID, or a generated texture's name — or `None` for one it did not make, and
    /// for the white one, which is a material with no texture. What lets a material built over a
    /// loaded texture without naming it (`Material::new(renderer.load_texture(path)?)`) be saved as
    /// textured: it was saved white.
    #[must_use]
    pub fn texture_source_of(&self, bind_group: &Arc<wgpu::BindGroup>) -> Option<String> {
        self.texture_cache
            .iter()
            .find(|(_, cached)| Arc::ptr_eq(cached, bind_group))
            .map(|(key, _)| key.clone())
            .filter(|key| key != Self::WHITE_TEXTURE)
    }

    /// Return (creating once) a 1×1 opaque-white texture.
    ///
    /// Used as the default albedo map for materials that specify no texture.
    pub fn create_white_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
    ) -> Arc<wgpu::BindGroup> {
        const KEY: &str = super::AssetManager::WHITE_TEXTURE;

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
        const KEY: &str = super::AssetManager::CHECKERBOARD_TEXTURE;
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
        const KEY: &str = super::AssetManager::UV_DEBUG_TEXTURE;
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

/// The material bind-group cache keeps what is drawn, not every value a material ever took.
#[cfg(test)]
mod material_cache_tests {
    use crate::asset::AssetManager;
    use crate::components::MaterialMaps;
    use std::sync::Arc;

    fn gpu() -> (wgpu::Device, wgpu::Queue, wgpu::BindGroupLayout) {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::NOOP,
            backend_options: wgpu::BackendOptions {
                noop: wgpu::NoopBackendOptions { enable: true, ..Default::default() },
                ..Default::default()
            },
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("the noop adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("a noop device");
        let layout = crate::pipeline::layouts::build_layouts(&device).texture;
        (device, queue, layout)
    }

    /// A UV offset of `x`: strengths that are not neutral, so the material gets a bind group of its own.
    fn scrolled(x: f32) -> MaterialMaps {
        MaterialMaps::default().with_uv_transform([1.0, 1.0], 0.0, [x, 0.0])
    }

    /// **A UV offset scrolled for 600 frames keeps one bind group, not 600.** Each frame's value is a
    /// key of its own; until 2026-11-16 every one stayed in the cache with its uniform buffer for
    /// the renderer's lifetime, which is why `MaterialMaps` could not be keyed. The bind groups a
    /// material still holds stay, and the same sources asked for again get the same one.
    #[test]
    fn an_animated_strength_does_not_pile_up_bind_groups() {
        let (device, queue, layout) = gpu();
        let mut assets = AssetManager::new();
        let held: Vec<_> = (0..3).map(|i| assets.load_material(&device, &queue, &layout, None, &scrolled(10.0 + i as f32))).collect();
        let mut drawn = assets.load_material(&device, &queue, &layout, None, &scrolled(0.0));
        for frame in 1..600 {
            // What `rebind_changed_materials` does: the material's bind group replaced by the new value's.
            drawn = assets.load_material(&device, &queue, &layout, None, &scrolled(frame as f32 / 600.0));
        }
        // The three held, the one being drawn, and the one before it — which the material still held
        // while its replacement was made, as `rebind_changed_materials` holds it.
        assert_eq!(assets.material_cache_len(), 5, "the cache kept values nothing draws any more");
        for (i, bind_group) in held.iter().enumerate() {
            let again = assets.load_material(&device, &queue, &layout, None, &scrolled(10.0 + i as f32));
            assert!(Arc::ptr_eq(bind_group, &again), "a held material's bind group was dropped and built again");
        }
        drop(drawn);
    }
}
