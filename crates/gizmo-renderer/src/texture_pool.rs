//! Many textures of one size in a few shared texture arrays — what [`MeshArena`](crate::mesh_arena)
//! is to meshes.
//!
//! A material is a bind group, and a batch is the buffers and the bind group: two objects with
//! different textures never share a draw. A city of a few thousand textures was a few thousand
//! batches, and wgpu tracks every texture of every bind group set on a pass (`set_bind_group`,
//! `merge_bind_group`) — in `nfsu2-gizmo`'s city a third of the main thread (2026-11-18). A texture
//! put here goes into a layer of a 2D array with others of its size, and the array has **one** bind
//! group: materials made from it ([`Material::from_pool`](crate::components::Material::from_pool))
//! are one batch, which the arena's shared buffers then make one multi-draw. Which layer an object
//! shows rides its instance ([`InstanceRaw::texture`](crate::gpu_types::InstanceRaw::texture)), and
//! the engine's shaders read the base colour from that layer (group 1, binding 8).
//!
//! Measured before it existed, with the city's materials all made over one bind group: 3 872 draws
//! → 45, frame 17 → 10.6 ms on the Radeon 780M and 11.9 → 7.2 ms on the RTX 3050.
//!
//! A pooled picture is what
//! [`AssetManager::install_decoded_material_texture`](crate::asset::AssetManager::install_decoded_material_texture)
//! makes of the same pixels — uploaded and mipmapped the same way, then copied level by level into
//! its layer, read through the same sampler beside the same neutral maps — so a material from the
//! pool draws as the same material made the other way. Only the base colour is pooled: a material
//! with maps of its own (normal, metallic-roughness, …) has a bind group of its own.
//!
//! A layer is held by the [`PooledTexture`] it was given as, and by every material made from it;
//! when the last of them goes the layer is free for the next texture of that size. Arrays are not
//! given back.

use crate::asset::AssetError;
use std::sync::{Arc, Mutex, Weak};

/// What an array is sized for, all its layers and levels: enough layers that a city's textures of
/// one size share a few arrays, few enough that a size with one texture holds no large one empty.
const ARRAY_BYTES: u64 = 32 << 20;

/// The fewest layers an array has: two, because an array of one layer is a plain 2D texture to
/// WebGL2, which cannot then be bound where an array is declared.
const MIN_LAYERS: u32 = 2;

/// The format of every pooled picture: a colour, as a decoded texture is installed.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Textures of one size in shared arrays, a bind group per array — see the [module](self).
pub struct TexturePool {
    arrays: Arc<Mutex<Vec<Array>>>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    defaults: Arc<crate::asset::MaterialDefaults>,
}

struct Array {
    width: u32,
    height: u32,
    layers: u32,
    texture: wgpu::Texture,
    bind_group: Arc<wgpu::BindGroup>,
    /// Layers given back, taken first.
    free: Vec<u32>,
    /// The first layer never handed out.
    next: u32,
}

impl Array {
    fn take(&mut self) -> Option<u32> {
        self.free.pop().or_else(|| {
            (self.next < self.layers).then(|| {
                self.next += 1;
                self.next - 1
            })
        })
    }
}

/// A texture in the pool: the bind group of its array and its layer there. Make a material from it
/// with [`Material::from_pool`](crate::components::Material::from_pool).
#[derive(Clone, Debug)]
pub struct PooledTexture {
    bind_group: Arc<wgpu::BindGroup>,
    slot: PoolSlot,
}

impl PooledTexture {
    /// The bind group of the array the texture is in — shared with every texture of that array.
    #[must_use]
    pub fn bind_group(&self) -> &Arc<wgpu::BindGroup> {
        &self.bind_group
    }

    /// Its layer, held for as long as this or a clone of it lives.
    #[must_use]
    pub fn slot(&self) -> &PoolSlot {
        &self.slot
    }
}

/// A layer of a pool's array, held: the layer is free again when the last clone goes.
#[derive(Clone)]
pub struct PoolSlot {
    layer: u32,
    _lease: Arc<Lease>,
}

impl PoolSlot {
    /// Which layer of its array the picture is.
    #[must_use]
    pub fn layer(&self) -> u32 {
        self.layer
    }
}

impl std::fmt::Debug for PoolSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PoolSlot").field("layer", &self.layer).finish_non_exhaustive()
    }
}

struct Lease {
    arrays: Weak<Mutex<Vec<Array>>>,
    array: usize,
    layer: u32,
}

impl Drop for Lease {
    fn drop(&mut self) {
        if let Some(arrays) = self.arrays.upgrade() {
            let mut arrays = arrays.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            arrays[self.array].free.push(self.layer);
        }
    }
}

/// How many layers an array of `width`×`height` pictures (with their mips) has: what
/// [`ARRAY_BYTES`] holds, between [`MIN_LAYERS`] and the device's `max`.
fn layers_for(width: u32, height: u32, max: u32) -> u32 {
    let per_layer = (u64::from(width) * u64::from(height) * 4 * 4 / 3).max(1);
    let fit = u32::try_from(ARRAY_BYTES / per_layer).unwrap_or(u32::MAX);
    fit.clamp(MIN_LAYERS, max.max(MIN_LAYERS))
}

/// What the pool holds now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct TexturePoolStats {
    /// Arrays made.
    pub arrays: usize,
    /// Layers in them, used or not.
    pub layers: u32,
    /// Layers holding a picture now.
    pub used: u32,
}

impl TexturePool {
    /// An empty pool whose arrays are bound with `layout` (the material layout,
    /// [`SceneState::texture_bind_group_layout`](crate::pipeline::SceneState::texture_bind_group_layout)).
    #[must_use]
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, layout: &wgpu::BindGroupLayout) -> Self {
        Self {
            arrays: Arc::new(Mutex::new(Vec::new())),
            device: device.clone(),
            queue: queue.clone(),
            layout: layout.clone(),
            defaults: Arc::new(crate::asset::MaterialDefaults::new(device, queue)),
        }
    }

    /// Puts `width`×`height` RGBA8 pixels (sRGB, a colour) into a layer of an array of that size,
    /// mipmapped as [`AssetManager::install_decoded_material_texture`] would.
    ///
    /// [`AssetManager::install_decoded_material_texture`]: crate::asset::AssetManager::install_decoded_material_texture
    ///
    /// # Errors
    ///
    /// A zero width or height, or `rgba` not `width * height * 4` bytes long — what the asset manager
    /// refuses too.
    pub fn colour(&self, rgba: &[u8], width: u32, height: u32, label: &str) -> Result<PooledTexture, AssetError> {
        if width == 0 || height == 0 {
            return Err(AssetError::ZeroDimensionTexture { cache_key: label.to_string(), width, height });
        }
        let expected = (width as usize).saturating_mul(height as usize).saturating_mul(4);
        if rgba.len() != expected {
            return Err(AssetError::RgbaSizeMismatch {
                cache_key: label.to_string(),
                got: rgba.len(),
                expected,
                width,
                height,
            });
        }
        // The picture as a texture of its own first, mipmapped by the engine's one generator, then
        // copied level by level: the same pixels at every level as the texture made the other way.
        let (picture, levels) = crate::asset::AssetManager::upload_rgba_mipped(
            &self.device,
            &self.queue,
            label,
            rgba,
            width,
            height,
            FORMAT,
            crate::texture_quality::MIPPED_TEXTURE_USAGE | wgpu::TextureUsages::COPY_SRC,
        );

        let mut arrays = self.arrays.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let found = arrays
            .iter_mut()
            .enumerate()
            .filter(|(_, a)| a.width == width && a.height == height)
            .find_map(|(i, a)| a.take().map(|layer| (i, layer)));
        let (index, layer) = match found {
            Some(found) => found,
            None => {
                let mut array = self.make_array(width, height, levels);
                let layer = array.take().expect("a new array has a free layer");
                arrays.push(array);
                (arrays.len() - 1, layer)
            }
        };
        let array = &arrays[index];
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("texture pool copy") });
        for level in 0..levels {
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo { texture: &picture, mip_level: level, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                wgpu::TexelCopyTextureInfo {
                    texture: &array.texture,
                    mip_level: level,
                    origin: wgpu::Origin3d { x: 0, y: 0, z: layer },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d { width: (width >> level).max(1), height: (height >> level).max(1), depth_or_array_layers: 1 },
            );
        }
        self.queue.submit(std::iter::once(encoder.finish()));
        let bind_group = array.bind_group.clone();
        drop(arrays);
        Ok(PooledTexture {
            bind_group,
            slot: PoolSlot { layer, _lease: Arc::new(Lease { arrays: Arc::downgrade(&self.arrays), array: index, layer }) },
        })
    }

    /// A new array for pictures of `width`×`height` with `levels` mips, and its bind group: the
    /// array at binding 8, the neutral maps and parameters, and the sampler a decoded texture gets.
    fn make_array(&self, width: u32, height: u32, levels: u32) -> Array {
        let layers = layers_for(width, height, self.device.limits().max_texture_array_layers);
        let label = format!("texture pool {width}x{height}");
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&label),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: layers },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let sampler = crate::texture_quality::material_sampler(
            &self.device,
            "linear_repeat_sampler",
            wgpu::AddressMode::Repeat,
            wgpu::AddressMode::Repeat,
            wgpu::FilterMode::Linear,
            wgpu::FilterMode::Linear,
            levels > 1,
        );
        let d = &self.defaults;
        let bind_group = crate::asset::AssetManager::assemble_material_bind_group(
            &self.device,
            &self.layout,
            &d.white_view,
            &sampler,
            &d.flat_normal_view,
            &d.white_view,
            &d.white_view,
            &d.pool_placeholder_view,
            &d.white_view,
            &d.params_buffer,
            &view,
            &label,
        );
        Array { width, height, layers, texture, bind_group, free: Vec::new(), next: 0 }
    }

    /// What the pool holds now.
    #[must_use]
    pub fn stats(&self) -> TexturePoolStats {
        let arrays = self.arrays.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        TexturePoolStats {
            arrays: arrays.len(),
            layers: arrays.iter().map(|a| a.layers).sum(),
            used: arrays.iter().map(|a| a.next - a.free.len() as u32).sum(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An array holds what its byte budget holds, at least two layers (WebGL2) and at most what the
    /// device allows.
    #[test]
    fn an_array_is_sized_by_its_budget_between_two_and_the_device_limit() {
        // 512² with mips: ~1.33 MiB a layer, 24 in 32 MiB.
        assert_eq!(layers_for(512, 512, 2048), 24);
        // 64²: ~21 KiB a layer, far more than the device allows.
        assert_eq!(layers_for(64, 64, 256), 256);
        // 4096²: one layer is past the budget, and still two.
        assert_eq!(layers_for(4096, 4096, 2048), MIN_LAYERS);
        // A device that allows fewer than two is not one this runs on, and two is still asked for.
        assert_eq!(layers_for(64, 64, 1), MIN_LAYERS);
    }
}
