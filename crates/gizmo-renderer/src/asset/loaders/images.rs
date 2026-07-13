//! glTF image handling — RGBA8 conversion, sRGB-vs-linear classification, and GPU upload.
//! Extracted verbatim from `loaders.rs` (pure move). `GpuImage`/`upload_gltf_images`/
//! `classify_gltf_image_srgb` are consumed by `material.rs` + `load_gltf_from_import`.
//! Self-contained: only `gltf`/`wgpu`/`tracing` (no `use super::*`).

fn convert_image_to_rgba8(image: &gltf::image::Data, idx: usize, file_path: &str) -> Vec<u8> {
    let (w, h) = (image.width as usize, image.height as usize);
    let pixel_count = w * h;

    match image.format {
        gltf::image::Format::R8G8B8A8 => {
            // Already in the right format — clone and return.
            // Guard against truncated data so write_texture can't panic.
            let expected = pixel_count * 4;
            if image.pixels.len() >= expected {
                image.pixels[..expected].to_vec()
            } else {
                // Pad with opaque black.
                let mut out = image.pixels.clone();
                out.resize(expected, 255);
                out
            }
        }

        gltf::image::Format::R8G8B8 => {
            // Drop the boundary check: chunks_exact only yields complete 3-byte chunks,
            // silently ignoring a trailing 1 or 2 bytes. A trailing partial pixel is
            // a malformed file; padding it to opaque black is the safest recovery.
            let mut out = Vec::with_capacity(pixel_count * 4);
            for chunk in image.pixels.chunks_exact(3) {
                out.extend_from_slice(&[chunk[0], chunk[1], chunk[2], 255]);
            }
            // Pad if the source was shorter than expected.
            out.resize(pixel_count * 4, 255);
            out
        }

        gltf::image::Format::R8G8 => {
            // glTF R8G8 = two independent channels (Red, Green) — NOT luminance+alpha.
            // Map R→R, G→G, B=0, A=opaque. (Previously broadcast R into RGB and put
            // G into alpha, losing the green channel.)
            let mut out = Vec::with_capacity(pixel_count * 4);
            for chunk in image.pixels.chunks_exact(2) {
                out.extend_from_slice(&[chunk[0], chunk[1], 0, 255]);
            }
            out.resize(pixel_count * 4, 255);
            out
        }

        gltf::image::Format::R8 => {
            // Single-channel luminance: replicate to RGB, full alpha.
            let mut out = Vec::with_capacity(pixel_count * 4);
            for &lum in &image.pixels {
                out.extend_from_slice(&[lum, lum, lum, 255]);
            }
            out.resize(pixel_count * 4, 255);
            out
        }

        unknown => {
            tracing::error!(
                "[GLTF WARN] Unknown pixel format {unknown:?} on image {idx} in '{file_path}'. \
                 Falling back to RGBA8 with clamped copy."
            );
            let expected = pixel_count * 4;
            // Opaque black canvas — copy whatever bytes we have.
            let mut out = vec![0u8; expected];
            // Set alpha channel of every pixel to 255 (opaque).
            for px in 0..pixel_count {
                out[px * 4 + 3] = 255;
            }
            let copy_len = image.pixels.len().min(expected);
            out[..copy_len].copy_from_slice(&image.pixels[..copy_len]);
            out
        }
    }
}

/// A GPU-resident glTF image.  Holds the [`wgpu::Texture`] alongside its view so
/// the texture outlives every material bind group that references the view.
pub(super) struct GpuImage {
    // Kept alive so views remain valid; not read directly.
    #[allow(dead_code)]
    texture: wgpu::Texture,
    pub(super) view: wgpu::TextureView,
}

/// Decide, per image index, whether it must be uploaded as sRGB.
///
/// Base-colour and emissive textures are colour data (sRGB); normal,
/// metallic-roughness and occlusion textures are linear data and must NOT be
/// gamma-decoded.  An image used as a colour map anywhere wins the sRGB vote.
pub(super) fn classify_gltf_image_srgb(document: &gltf::Document, num_images: usize) -> Vec<bool> {
    let mut is_srgb = vec![false; num_images];
    let mut mark = |idx: usize| {
        if idx < is_srgb.len() {
            is_srgb[idx] = true;
        }
    };
    for material in document.materials() {
        let pbr = material.pbr_metallic_roughness();
        if let Some(ti) = pbr.base_color_texture() {
            mark(ti.texture().source().index());
        }
        if let Some(ti) = material.emissive_texture() {
            mark(ti.texture().source().index());
        }
    }
    is_srgb
}

/// Upload every glTF image to the GPU with the correct colour space, returning
/// index-aligned [`GpuImage`]s.
pub(super) fn upload_gltf_images(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    file_path: &str,
    images: &[gltf::image::Data],
    srgb_flags: &[bool],
) -> Vec<GpuImage> {
    let mut out = Vec::with_capacity(images.len());

    for (i, image) in images.iter().enumerate() {
        let (width, height) = (image.width, image.height);
        let rgba: Vec<u8> = convert_image_to_rgba8(image, i, file_path);

        let texture_size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        let format = if srgb_flags.get(i).copied().unwrap_or(true) {
            wgpu::TextureFormat::Rgba8UnormSrgb
        } else {
            wgpu::TextureFormat::Rgba8Unorm
        };

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            size: texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            label: Some(&format!("{file_path}_tex_{i}")),
            view_formats: &[],
        });

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: Some(height),
            },
            texture_size,
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        out.push(GpuImage { texture, view });
    }

    out
}
