//! The mipmap and anisotropy policy, gathered in one place: every sampled material texture goes
//! through here.
//!
//! **Why one place.** Before this, three loading paths did three different things and two of them
//! produced textures without mipmaps:
//!
//! | path | mips | mipmap filter | aniso |
//! |---|---|---|---|
//! | `asset::loaders::images` (glTF — the cars' and the city's textures) | 1 | Nearest | off |
//! | `asset::texture::install_decoded_material_texture` (disk/streaming/procedural) | 1 | Nearest | off |
//! | `Renderer::create_texture` (hand-built RGBA) | full chain | Linear | off |
//!
//! So the mipmap generator and the `mipmap.wgsl` shader already existed, and none of the engine's
//! real texture paths called them — every surface viewed at an angle suffered minification
//! aliasing, and no sampler asked for anisotropy.

/// The anisotropic filtering level material samplers ask for.
///
/// 16 is the ceiling WebGPU allows; the backend clamps it to the device's real limit, so a GPU
/// that cannot do it degrades silently rather than erroring. Anisotropy only costs anything on
/// the surfaces where minification actually happens (angled, distant ones).
pub(crate) const MATERIAL_ANISOTROPY: u16 = 16;

/// The full mip-chain length of a 2D texture.
///
/// The `max(1)` is not there for a zero dimension — callers already filter those out — but
/// because `ilog2()` would panic at 0.
pub(crate) fn mip_level_count(width: u32, height: u32) -> u32 {
    width.max(height).max(1).ilog2() + 1
}

/// The extra usage flag a texture needs if a mip chain is going to be generated for it.
///
/// The generator downsamples by DRAWING into each mip level (a blit), so the texture has to be
/// usable as a render target. Forgetting this gives a wgpu validation error at upload time.
pub(crate) const MIPPED_TEXTURE_USAGE: wgpu::TextureUsages =
    wgpu::TextureUsages::TEXTURE_BINDING
        .union(wgpu::TextureUsages::COPY_DST)
        .union(wgpu::TextureUsages::RENDER_ATTACHMENT);

/// The sampler for a sampled material texture.
///
/// `has_mips` is whether the texture really was uploaded with a mip chain — setting the sampler
/// to sample levels the texture does NOT have is a validation error, and the converse (a chain
/// exists but the sampler says `Nearest`) is exactly the silent loss this module fixes.
pub(crate) fn material_sampler(
    device: &wgpu::Device,
    label: &str,
    address_mode_u: wgpu::AddressMode,
    address_mode_v: wgpu::AddressMode,
    mag_filter: wgpu::FilterMode,
    min_filter: wgpu::FilterMode,
    has_mips: bool,
) -> wgpu::Sampler {
    // wgpu, `anisotropy_clamp > 1`'i yalnız mag/min/mipmap filtrelerinin ÜÇÜ de `Linear`
    // iken kabul eder. Bir glTF materyali bilinçli olarak `Nearest` isteyebilir (pixel-art
    // dokular), o yüzden burada anizotropi dayatmak yerine kapatıyoruz — istenen görünüm
    // korunur ve validation hatası da alınmaz.
    let trilinear = has_mips
        && mag_filter == wgpu::FilterMode::Linear
        && min_filter == wgpu::FilterMode::Linear;

    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some(label),
        address_mode_u,
        address_mode_v,
        address_mode_w: wgpu::AddressMode::Repeat,
        mag_filter,
        min_filter,
        mipmap_filter: if trilinear {
            wgpu::MipmapFilterMode::Linear
        } else {
            wgpu::MipmapFilterMode::Nearest
        },
        lod_min_clamp: 0.0,
        // Mip'siz yolda 0.0 KALIYOR: tek seviyeli bir dokuda eski davranış buydu ve
        // sampler'ın var olmayan seviyelere uzanmamasını garantiliyor.
        lod_max_clamp: if has_mips { 32.0 } else { 0.0 },
        compare: None,
        anisotropy_clamp: if trilinear { MATERIAL_ANISOTROPY } else { 1 },
        border_color: None,
    })
}

/// A REUSABLE generator that fills the mip chain with successive blits.
///
/// It is a struct because this used to be a free function that **recompiled the shader module and
/// the render pipeline on every call.** On a single hand-built texture that was invisible; on a
/// glTF scene with hundreds of textures (Bayview) it would mean one pipeline compilation per
/// texture. Build the blitter once and reuse it for every texture in the batch.
pub(crate) struct MipmapBlitter {
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    format: wgpu::TextureFormat,
}

impl MipmapBlitter {
    /// Builds a generator for `format`. The pipeline depends on the colour target's format, so a
    /// blitter is specific to one format — sRGB and linear textures need separate blitters.
    pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Mipmap Blit Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/mipmap.wgsl").into()),
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Mipmap Blit Pipeline"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // İndirgeme sırasında `ClampToEdge`: kaynak seviyesinin kenarında sarmalamak
        // karşı kenarın renklerini içeri sızdırır.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("mipmap_blit_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        Self {
            pipeline,
            sampler,
            format,
        }
    }

    /// Fills levels 1..`mip_level_count` of `texture`, sampling each from the one before it, in every
    /// layer of it. Level 0 must already have been written.
    ///
    /// It records the commands into the given `encoder` and **does not submit.** That is what
    /// lets a batch of textures be gathered into a single command buffer.
    pub(crate) fn record(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
        mip_level_count: u32,
    ) {
        debug_assert_eq!(
            texture.format(),
            self.format,
            "MipmapBlitter formatı dokununkiyle eşleşmiyor — pipeline'ın renk hedefi uyuşmaz"
        );
        if mip_level_count <= 1 {
            return;
        }

        let bind_group_layout = self.pipeline.get_bind_group_layout(0);
        for layer in 0..texture.depth_or_array_layers() {
            let views: Vec<wgpu::TextureView> = (0..mip_level_count)
                .map(|mip| {
                    texture.create_view(&wgpu::TextureViewDescriptor {
                        label: Some(&format!("Mip {mip} layer {layer}")),
                        format: None,
                        dimension: Some(wgpu::TextureViewDimension::D2),
                        usage: None,
                        aspect: wgpu::TextureAspect::All,
                        base_mip_level: mip,
                        mip_level_count: Some(1),
                        base_array_layer: layer,
                        array_layer_count: Some(1),
                    })
                })
                .collect();
            self.record_layer(device, encoder, &bind_group_layout, &views);
        }
    }

    /// One layer's chain, from its level views.
    fn record_layer(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, bind_group_layout: &wgpu::BindGroupLayout, views: &[wgpu::TextureView]) {
        for target_mip in 1..views.len() {
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                layout: bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&views[target_mip - 1]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
                label: None,
            });

            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Mipmap Blit Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &views[target_mip],
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    /// A convenience wrapper for a single texture: record and submit immediately.
    /// If you are uploading a batch, use [`record`](Self::record).
    pub(crate) fn generate(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        mip_level_count: u32,
    ) {
        if mip_level_count <= 1 {
            return;
        }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Mipmap Encoder"),
        });
        self.record(device, &mut encoder, texture, mip_level_count);
        queue.submit(Some(encoder.finish()));
        tracing::debug!(
            mip_levels = mip_level_count,
            format = ?self.format,
            "[Renderer] generated mipmap chain"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every level the blitter writes must be the 2×2 average of the level above it — checked
    /// against the same chain worked on the CPU, texel by texel, on a texture that is not square.
    ///
    /// `mipmap.wgsl` used to divide the pixel it was writing by the size of the level it was
    /// *reading*. That is twice the right size, so the coordinate only ran to 0.5: each level was
    /// the top-left quarter of the one above, blown up to twice its size, and level `n` showed
    /// the top-left `1/2ⁿ` of the image. The shader was like that from April 2026, and every
    /// material texture went through it from August (`004554d6`, when the asset paths got mip
    /// chains) — under which a road sign read correctly up close and became its own corner from
    /// 15 m on, and every textured detail did the same. Nothing compared a level's content with
    /// anything; a count of levels, or "the chain exists", cannot see it. Only the texels can.
    #[test]
    fn each_level_is_the_two_by_two_average_of_the_one_above() {
        let _gpu = crate::test_gpu::gpu_lock();
        let Some((device, queue)) = pollster::block_on(crate::test_gpu::headless_device()) else {
            eprintln!("skipping: no usable GPU adapter");
            return;
        };
        // Wider than tall, so a coordinate built from the wrong axis shows too. Linear format, so
        // the GPU's average and the CPU's are the same arithmetic (an sRGB target averages in
        // linear light and re-encodes, which a byte mean does not model).
        let (w, h) = (32u32, 16u32);
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let levels = mip_level_count(w, h);
        assert_eq!(levels, 6);

        // A fixed pseudo-random image: every texel differs from its neighbours, so any texel taken
        // from the wrong place lands on a wrong value.
        let mut seed = 0x9E37_79B9u32;
        let base: Vec<u8> = (0..w * h * 4)
            .map(|i| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                if i % 4 == 3 {
                    255
                } else {
                    (seed >> 24) as u8
                }
            })
            .collect();

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mip test"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: MIPPED_TEXTURE_USAGE | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &base,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * w),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        MipmapBlitter::new(&device, format).generate(&device, &queue, &texture, levels);

        let mut want = base;
        let (mut lw, mut lh) = (w, h);
        for level in 1..levels {
            // The CPU's level: each texel the mean of the 2×2 block above it (every size here is
            // even until it reaches 1, where the block is 2×1 or 1×2).
            let (nw, nh) = ((lw / 2).max(1), (lh / 2).max(1));
            let mut next = vec![0u8; (nw * nh * 4) as usize];
            for y in 0..nh {
                for x in 0..nw {
                    for c in 0..4 {
                        let mut sum = 0u32;
                        let mut n = 0u32;
                        for (sx, sy) in [
                            (2 * x, 2 * y),
                            (2 * x + 1, 2 * y),
                            (2 * x, 2 * y + 1),
                            (2 * x + 1, 2 * y + 1),
                        ] {
                            if sx < lw && sy < lh {
                                sum += u32::from(want[((sy * lw + sx) * 4 + c) as usize]);
                                n += 1;
                            }
                        }
                        next[((y * nw + x) * 4 + c) as usize] =
                            ((sum as f32 / n as f32).round()) as u8;
                    }
                }
            }
            let got = read_level(&device, &queue, &texture, level, nw, nh);
            for (i, (g, e)) in got.iter().zip(&next).enumerate() {
                let texel = i / 4;
                assert!(
                    g.abs_diff(*e) <= 2,
                    "level {level} ({nw}×{nh}) texel ({}, {}) channel {}: GPU {g}, 2×2 mean {e}",
                    texel as u32 % nw,
                    texel as u32 / nw,
                    i % 4,
                );
            }
            // The next level is checked against what the GPU made of this one, so one level's
            // rounding does not compound into the next comparison.
            want = got;
            (lw, lh) = (nw, nh);
        }
    }

    fn read_level(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        level: u32,
        w: u32,
        h: u32,
    ) -> Vec<u8> {
        let row = (4 * w).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mip readback"),
            size: u64::from(row * h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: level,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(encoder.finish()));
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| r.expect("map mip readback"));
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        let mapped = slice.get_mapped_range().expect("mapped mip readback");
        let mut out = Vec::with_capacity((4 * w * h) as usize);
        for y in 0..h {
            let start = (y * row) as usize;
            out.extend_from_slice(&mapped[start..start + (4 * w) as usize]);
        }
        out
    }
}
