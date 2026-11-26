//! Textures a material's group is filled with that are made of others, drawn on the GPU.
//!
//! **Occlusion rides the metallic-roughness map's red channel** (glTF's ORM packing), so the
//! material group has no occlusion texture of its own. WebGPU gives a stage 16 sampled textures, and
//! the forward-only layout — every browser's and phone's frame — had one left (measured by
//! `the_forward_only_layout_fits_webgpu`, 2026-11-26): a surface's own textures needed it. A material
//! with an occlusion map gets one texture drawn from the two ([`MaterialPacker::orm`]): red from the
//! occlusion map, green and blue from the metallic-roughness map, each white where there is none.
//! A material whose metallic-roughness map is its occlusion map too — an ORM texture, the way most
//! glTF exporters write them — needs nothing drawn: its red is the occlusion already.
//!
//! **A surface's own textures are one array** ([`MaterialPacker::layers`]) at the slot occlusion
//! left, binding 5: what a custom surface (`MaterialType::CustomSurface`) or a shader graph samples
//! beyond the engine's maps — a terrain's grass, rock and sand. Its layers are drawn at the size of
//! the largest of them, colours read from sRGB as a base colour is.
//!
//! Both are mipmapped as an uploaded texture is (`MipmapBlitter`).

use crate::texture_quality::{mip_level_count, MipmapBlitter, MIPPED_TEXTURE_USAGE};

/// What the materials of an asset manager are packed with: built once, used for every material.
pub(crate) struct MaterialPacker {
    layout: wgpu::BindGroupLayout,
    orm: wgpu::RenderPipeline,
    copy: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    orm_mips: MipmapBlitter,
    copy_mips: MipmapBlitter,
}

/// The ORM texture's format: data, not colour.
const ORM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// A surface's own textures' format: colours, as a base colour is.
const SURFACE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

impl MaterialPacker {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                multisampled: false,
                view_dimension: wgpu::TextureViewDimension::D2,
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material_pack_layout"),
            entries: &[
                texture(0),
                texture(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("material_pack"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/material_pack.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("material_pack_pipeline_layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |entry: &str, format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[] },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                }),
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("material_pack_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        Self {
            orm: pipeline("fs_orm", ORM_FORMAT),
            copy: pipeline("fs_copy", SURFACE_FORMAT),
            layout,
            sampler,
            orm_mips: MipmapBlitter::new(device, ORM_FORMAT),
            copy_mips: MipmapBlitter::new(device, SURFACE_FORMAT),
        }
    }

    /// The ORM texture of an occlusion map and a metallic-roughness map (white for either a material
    /// does not have): red the occlusion, green and blue the metallic-roughness map's, at the size
    /// of the larger, mipmapped.
    pub(crate) fn orm(&self, device: &wgpu::Device, queue: &wgpu::Queue, occlusion: &wgpu::TextureView, metallic_roughness: &wgpu::TextureView) -> wgpu::TextureView {
        let (width, height) = largest(&[occlusion, metallic_roughness]);
        let texture = target(device, "material_orm", width, height, 1, ORM_FORMAT);
        let mips = mip_level_count(width, height);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("material_orm") });
        self.draw(device, &mut encoder, &self.orm, (occlusion, metallic_roughness), &texture, 0, (width, height));
        self.orm_mips.record(device, &mut encoder, &texture, mips);
        queue.submit(Some(encoder.finish()));
        texture.create_view(&wgpu::TextureViewDescriptor::default())
    }

    /// A surface's own textures as the layers of one array, each drawn at the size of the largest,
    /// mipmapped: at least two layers, the last repeated, since an array of one is a plain 2D texture
    /// to WebGL2 and cannot be bound where an array is declared.
    pub(crate) fn layers(&self, device: &wgpu::Device, queue: &wgpu::Queue, sources: &[&wgpu::TextureView]) -> wgpu::TextureView {
        assert!(!sources.is_empty(), "a surface's texture array has at least one source");
        let (width, height) = largest(sources);
        let count = sources.len().max(2) as u32;
        let texture = target(device, "surface_textures", width, height, count, SURFACE_FORMAT);
        let mips = mip_level_count(width, height);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("surface_textures") });
        for layer in 0..count {
            let source = sources[(layer as usize).min(sources.len() - 1)];
            self.draw(device, &mut encoder, &self.copy, (source, source), &texture, layer, (width, height));
        }
        self.copy_mips.record(device, &mut encoder, &texture, mips);
        queue.submit(Some(encoder.finish()));
        texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() })
    }

    #[allow(clippy::too_many_arguments)]
    fn draw(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
        (first, second): (&wgpu::TextureView, &wgpu::TextureView),
        texture: &wgpu::Texture,
        layer: u32,
        (width, height): (u32, u32),
    ) {
        use wgpu::util::DeviceExt;
        let size = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("material_pack_size"),
            contents: bytemuck::cast_slice(&[width as f32, height as f32, 0.0, 0.0]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("material_pack"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(first) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(second) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: size.as_entire_binding() },
            ],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2),
            base_mip_level: 0,
            mip_level_count: Some(1),
            base_array_layer: layer,
            array_layer_count: Some(1),
            ..Default::default()
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("material_pack"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::WHITE), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// The largest width and the largest height of the views' textures.
fn largest(views: &[&wgpu::TextureView]) -> (u32, u32) {
    views.iter().fold((1, 1), |(w, h), v| {
        let size = v.texture().size();
        (w.max(size.width), h.max(size.height))
    })
}

fn target(device: &wgpu::Device, label: &str, width: u32, height: u32, layers: u32, format: wgpu::TextureFormat) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: layers },
        mip_level_count: mip_level_count(width, height),
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        // Read back by the tests below; a copy source costs nothing a texture is not asked for.
        usage: MIPPED_TEXTURE_USAGE.union(wgpu::TextureUsages::COPY_SRC),
        view_formats: &[],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(device: &wgpu::Device, queue: &wgpu::Queue, (w, h): (u32, u32), rgba: [u8; 4], format: wgpu::TextureFormat) -> wgpu::TextureView {
        let pixels: Vec<u8> = (0..w * h).flat_map(|_| rgba).collect();
        let (view, _) = crate::asset::AssetManager::upload_rgba_texture_for_tests(device, queue, &pixels, w, h, format);
        view
    }

    /// Every texel of `layer`'s `level`, RGBA.
    fn read(device: &wgpu::Device, queue: &wgpu::Queue, view: &wgpu::TextureView, layer: u32, level: u32) -> Vec<[u8; 4]> {
        let texture = view.texture();
        let (w, h) = ((texture.width() >> level).max(1), (texture.height() >> level).max(1));
        let row = (4 * w).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: u64::from(row * h), usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture, mip_level: level, origin: wgpu::Origin3d { x: 0, y: 0, z: layer }, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        queue.submit(Some(encoder.finish()));
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| r.expect("map"));
        device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
        let data = slice.get_mapped_range().expect("mapped");
        (0..h).flat_map(|y| (0..w).map(move |x| (y, x))).map(|(y, x)| {
            let i = (y * row + x * 4) as usize;
            [data[i], data[i + 1], data[i + 2], data[i + 3]]
        }).collect()
    }

    /// The ORM texture is the occlusion map's red with the metallic-roughness map's green and blue,
    /// at the larger of the two sizes, every texel and every level: a 4×4 occlusion map and an 8×2
    /// metallic-roughness map make an 8×4.
    #[test]
    fn the_orm_texture_is_occlusion_red_and_the_maps_green_and_blue() {
        let _gpu = crate::test_gpu::gpu_lock();
        let Some((device, queue)) = pollster::block_on(crate::test_gpu::headless_device()) else {
            eprintln!("skipping: no usable GPU adapter");
            return;
        };
        let packer = MaterialPacker::new(&device);
        let ao = solid(&device, &queue, (4, 4), [64, 9, 9, 9], ORM_FORMAT);
        let mr = solid(&device, &queue, (8, 2), [9, 128, 192, 9], ORM_FORMAT);
        let orm = packer.orm(&device, &queue, &ao, &mr);
        assert_eq!((orm.texture().width(), orm.texture().height()), (8, 4));
        for level in 0..orm.texture().mip_level_count() {
            for t in read(&device, &queue, &orm, 0, level) {
                assert_eq!(t, [64, 128, 192, 255], "level {level}");
            }
        }
    }

    /// A surface's textures are one array at the largest size, each in its layer, mipmapped layer by
    /// layer; one texture is two layers, for WebGL2.
    #[test]
    fn a_surfaces_textures_are_one_array_a_layer_each() {
        let _gpu = crate::test_gpu::gpu_lock();
        let Some((device, queue)) = pollster::block_on(crate::test_gpu::headless_device()) else {
            eprintln!("skipping: no usable GPU adapter");
            return;
        };
        let packer = MaterialPacker::new(&device);
        let red = solid(&device, &queue, (4, 4), [255, 0, 0, 255], SURFACE_FORMAT);
        let green = solid(&device, &queue, (16, 8), [0, 255, 0, 255], SURFACE_FORMAT);
        let array = packer.layers(&device, &queue, &[&red, &green]);
        let t = array.texture();
        assert_eq!((t.width(), t.height(), t.depth_or_array_layers()), (16, 8, 2));
        for level in 0..t.mip_level_count() {
            assert!(read(&device, &queue, &array, 0, level).iter().all(|&p| p == [255, 0, 0, 255]), "layer 0, level {level}");
            assert!(read(&device, &queue, &array, 1, level).iter().all(|&p| p == [0, 255, 0, 255]), "layer 1, level {level}");
        }
        let one = packer.layers(&device, &queue, &[&green]);
        assert_eq!(one.texture().depth_or_array_layers(), 2);
        assert!(read(&device, &queue, &one, 1, 0).iter().all(|&p| p == [0, 255, 0, 255]), "the one texture is repeated");
    }
}
