//! Light cookies (gobos): an image a [`SpotLight`](crate::components::SpotLight) projects through
//! its cone — a window frame, leaves, a stained-glass colour, a flashlight's uneven beam.
//!
//! Cookies live in one `Rgba8UnormSrgb` texture array of [`COOKIE_LAYERS`] layers,
//! [`COOKIE_SIZE`]² each, bound beside the shadow maps (shadow group, bindings 3 and 4), so adding
//! one never rebuilds a bind group. A spot names its cookie by a string
//! ([`SpotLight::cookie`](crate::components::SpotLight::cookie)); [`LightCookies`] maps names to
//! layers. A name is either registered from memory ([`LightCookies::insert`]) or, on native, read
//! from disk the first frame a spot names it ([`LightCookies::sync`]) — the name is then a path,
//! the same convention as `MaterialDesc::texture_source`.
//!
//! The cookie's **up** is world up as seen down the cone (for a cone pointing straight up or down,
//! world −Z): the packed light carries the cone's direction but not its roll. The web forward
//! pipeline has no shadow group, so cookies are native-only, like the shadows they sit beside.

use std::collections::HashSet;
use std::sync::Arc;

/// Edge length of every cookie layer, in texels. Images are resampled to it on upload.
pub const COOKIE_SIZE: u32 = 256;
/// How many distinct cookies can be resident at once.
pub const COOKIE_LAYERS: u32 = 8;
/// The cookie array's format: colour images, authored in sRGB.
pub const COOKIE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Why a cookie could not be registered.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CookieError {
    /// Every layer holds a different cookie already.
    Full,
    /// The pixel data is not `width × height` RGBA8, or the image is empty.
    BadImage,
    /// The file could not be read or decoded (native `sync` only).
    Load(String),
}

impl std::fmt::Display for CookieError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Full => write!(f, "every one of the {COOKIE_LAYERS} cookie layers is in use"),
            Self::BadImage => write!(f, "cookie pixels are not width × height RGBA8"),
            Self::Load(e) => write!(f, "cookie could not be loaded: {e}"),
        }
    }
}

impl std::error::Error for CookieError {}

/// The cookie texture array, its view and its sampler.
#[non_exhaustive]
pub struct CookieTextures {
    /// The array itself, [`COOKIE_LAYERS`] layers of [`COOKIE_SIZE`]².
    pub texture: wgpu::Texture,
    /// A `D2Array` view of it, for sampling.
    pub view: wgpu::TextureView,
    /// Bilinear, clamped to the edge.
    pub sampler: wgpu::Sampler,
}

impl CookieTextures {
    /// Creates the array. Its layers start zeroed — black — which is never seen: a spot samples a
    /// layer only once a cookie has been written to it.
    pub fn new(device: &wgpu::Device) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("light_cookies"),
            size: wgpu::Extent3d { width: COOKIE_SIZE, height: COOKIE_SIZE, depth_or_array_layers: COOKIE_LAYERS },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: COOKIE_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("light_cookies_view"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("light_cookies_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self { texture, view, sampler }
    }
}

/// Which cookie name sits in which layer — what the light packer reads. Cheap to clone.
#[derive(Debug, Clone, Default)]
pub struct CookieTable(Arc<Vec<String>>);

impl CookieTable {
    /// A table with `names` in layers 0, 1, … — for tests and tools that pack lights without a
    /// renderer.
    pub fn from_names<S: Into<String>>(names: impl IntoIterator<Item = S>) -> Self {
        Self(Arc::new(names.into_iter().map(Into::into).collect()))
    }

    /// The layer `name` is in, if it is resident.
    pub fn layer(&self, name: &str) -> Option<u32> {
        self.0.iter().position(|n| n == name).map(|i| i as u32)
    }
}

/// The resident cookies: names, their layers, and the names that failed to load (so a missing file
/// is reported once rather than retried every frame).
#[derive(Debug, Default)]
pub struct LightCookies {
    table: CookieTable,
    failed: HashSet<String>,
}

impl LightCookies {
    /// The name → layer table, for the light packer.
    pub fn table(&self) -> CookieTable {
        self.table.clone()
    }

    /// The layer `name` is in, if it is resident.
    pub fn layer(&self, name: &str) -> Option<u32> {
        self.table.layer(name)
    }

    /// Registers (or replaces) the cookie `name` from `width × height` RGBA8 pixels, row 0 at the
    /// top. The image is resampled to [`COOKIE_SIZE`]². Returns its layer.
    pub fn insert(
        &mut self,
        queue: &wgpu::Queue,
        textures: &CookieTextures,
        name: &str,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Result<u32, CookieError> {
        if width == 0 || height == 0 || rgba.len() != (width as usize) * (height as usize) * 4 {
            return Err(CookieError::BadImage);
        }
        let layer = match self.table.layer(name) {
            Some(l) => l,
            None => {
                if self.table.0.len() as u32 >= COOKIE_LAYERS {
                    return Err(CookieError::Full);
                }
                let mut names = (*self.table.0).clone();
                names.push(name.to_string());
                self.table = CookieTable(Arc::new(names));
                self.table.0.len() as u32 - 1
            }
        };
        self.failed.remove(name);
        let pixels = resample(rgba, width, height, COOKIE_SIZE);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &textures.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: 0, y: 0, z: layer },
                aspect: wgpu::TextureAspect::All,
            },
            &pixels,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(COOKIE_SIZE * 4), rows_per_image: Some(COOKIE_SIZE) },
            wgpu::Extent3d { width: COOKIE_SIZE, height: COOKIE_SIZE, depth_or_array_layers: 1 },
        );
        Ok(layer)
    }

    /// Loads, from disk, every cookie a `SpotLight` in `world` names that is neither resident nor
    /// already failed. The name is the path. A failure is logged once. Native only — the browser
    /// has no file system to read; register cookies there with [`insert`](Self::insert).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn sync(&mut self, world: &gizmo_core::World, queue: &wgpu::Queue, textures: &CookieTextures) {
        let mut wanted: Vec<String> = Vec::new();
        if let Some(q) = world.query::<&crate::components::SpotLight>() {
            for (_, light) in q.iter() {
                if let Some(name) = &light.cookie {
                    if self.layer(name).is_none() && !self.failed.contains(name) && !wanted.contains(name) {
                        wanted.push(name.clone());
                    }
                }
            }
        }
        for name in wanted {
            let result = image::open(&name)
                .map_err(|e| CookieError::Load(e.to_string()))
                .and_then(|img| {
                    let img = img.to_rgba8();
                    self.insert(queue, textures, &name, img.as_raw(), img.width(), img.height())
                });
            if let Err(e) = result {
                tracing::warn!(cookie = %name, error = %e, "[Light] cookie not loaded");
                self.failed.insert(name);
            }
        }
    }
}

impl crate::Renderer {
    /// Registers (or replaces) the light cookie `name` from `width × height` RGBA8 pixels, row 0
    /// at the top; a [`SpotLight`](crate::components::SpotLight) whose `cookie` is `name` projects
    /// it. See [`LightCookies::insert`].
    pub fn insert_light_cookie(&mut self, name: &str, rgba: &[u8], width: u32, height: u32) -> Result<u32, CookieError> {
        self.light_cookies.insert(&self.queue, &self.scene.cookies, name, rgba, width, height)
    }

    /// Loads from disk every cookie a spot light in `world` names and nobody registered. Both
    /// render paths call it once a frame; see [`LightCookies::sync`].
    #[cfg(not(target_arch = "wasm32"))]
    pub fn sync_light_cookies(&mut self, world: &gizmo_core::World) {
        self.light_cookies.sync(world, &self.queue, &self.scene.cookies);
    }
}

/// Bilinear resample of an RGBA8 image to `size`², texel centres aligned.
fn resample(rgba: &[u8], width: u32, height: u32, size: u32) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let at = |x: usize, y: usize, c: usize| rgba[(y * w + x) * 4 + c] as f32;
    let mut out = vec![0u8; (size * size * 4) as usize];
    for oy in 0..size as usize {
        let fy = ((oy as f32 + 0.5) * h as f32 / size as f32 - 0.5).clamp(0.0, (h - 1) as f32);
        let (y0, ty) = (fy.floor() as usize, fy.fract());
        let y1 = (y0 + 1).min(h - 1);
        for ox in 0..size as usize {
            let fx = ((ox as f32 + 0.5) * w as f32 / size as f32 - 0.5).clamp(0.0, (w - 1) as f32);
            let (x0, tx) = (fx.floor() as usize, fx.fract());
            let x1 = (x0 + 1).min(w - 1);
            for c in 0..4 {
                let top = at(x0, y0, c) * (1.0 - tx) + at(x1, y0, c) * tx;
                let bottom = at(x0, y1, c) * (1.0 - tx) + at(x1, y1, c) * tx;
                out[(oy * size as usize + ox) * 4 + c] = (top * (1.0 - ty) + bottom * ty).round() as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2×1 black-white image resampled keeps black on the left and white on the right, and a
    /// solid image stays solid — the resampler neither mirrors nor bleeds.
    #[test]
    fn resampling_keeps_left_left_and_solid_solid() {
        let img = [0, 0, 0, 255, 255, 255, 255, 255];
        let out = resample(&img, 2, 1, 8);
        assert_eq!(out[0], 0, "the left edge is the left pixel");
        assert_eq!(out[7 * 4], 255, "the right edge is the right pixel");
        assert!(out[3 * 4] < out[4 * 4], "left of centre is darker than right of centre");
        let solid = resample(&[10, 20, 30, 40], 1, 1, 4);
        assert!(solid.chunks(4).all(|p| p == [10, 20, 30, 40]));
    }

    /// `sync` reads a cookie a spot names by path, once; a path that is not there is reported and
    /// remembered as failed rather than retried every frame, and a later `insert` under that name
    /// still works.
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn sync_loads_named_cookies_from_disk_once() {
        let _gpu = crate::test_gpu::gpu_lock();
        let Some((device, queue)) = pollster::block_on(crate::test_gpu::headless_device()) else {
            eprintln!("skipping: no usable GPU adapter");
            return;
        };
        let dir = std::env::temp_dir().join(format!("gizmo-cookie-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bars.png");
        image::RgbaImage::from_raw(2, 1, vec![0, 0, 0, 255, 255, 255, 255, 255]).unwrap().save(&path).unwrap();
        let path = path.to_string_lossy().into_owned();
        let missing = dir.join("missing.png").to_string_lossy().into_owned();

        let textures = CookieTextures::new(&device);
        let mut cookies = LightCookies::default();
        let mut world = gizmo_core::World::new();
        for name in [&path, &missing] {
            let e = world.spawn();
            world.add_component(
                e,
                crate::components::SpotLight::new(gizmo_math::Vec3::ONE, 1.0, 10.0, 0.3, 0.5).with_cookie(name.clone()),
            );
        }
        cookies.sync(&world, &queue, &textures);
        assert_eq!(cookies.layer(&path), Some(0), "the file on disk is resident");
        assert_eq!(cookies.layer(&missing), None);
        assert!(cookies.failed.contains(&missing), "the missing one is remembered, not retried");
        cookies.sync(&world, &queue, &textures);
        assert_eq!(cookies.table().0.len(), 1, "a second frame loads nothing new");
        assert_eq!(cookies.insert(&queue, &textures, &missing, &[255; 4], 1, 1), Ok(1));
        assert!(!cookies.failed.contains(&missing), "registering it clears the failure");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Eight cookies fit; a ninth is refused, replacing one is not a new layer, and pixel data of
    /// the wrong length is refused.
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn the_array_holds_eight_and_says_so() {
        let _gpu = crate::test_gpu::gpu_lock();
        let Some((device, queue)) = pollster::block_on(crate::test_gpu::headless_device()) else {
            eprintln!("skipping: no usable GPU adapter");
            return;
        };
        let textures = CookieTextures::new(&device);
        let mut cookies = LightCookies::default();
        for i in 0..COOKIE_LAYERS {
            assert_eq!(cookies.insert(&queue, &textures, &format!("c{i}"), &[255; 4], 1, 1), Ok(i));
        }
        assert_eq!(cookies.insert(&queue, &textures, "c3", &[0; 4], 1, 1), Ok(3), "replacing keeps the layer");
        assert_eq!(cookies.insert(&queue, &textures, "ninth", &[255; 4], 1, 1), Err(CookieError::Full));
        assert_eq!(cookies.insert(&queue, &textures, "c0", &[255; 3], 1, 1), Err(CookieError::BadImage));
    }

    #[test]
    fn the_table_finds_what_was_named() {
        let t = CookieTable(Arc::new(vec!["a.png".into(), "b.png".into()]));
        assert_eq!(t.layer("b.png"), Some(1));
        assert_eq!(t.layer("c.png"), None);
    }
}
