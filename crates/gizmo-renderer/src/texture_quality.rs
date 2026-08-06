//! Bir yerde toplanmış mipmap + anizotropi politikası: örneklenen her materyal dokusu
//! buradan geçer.
//!
//! **Neden tek yerde.** Bundan önce üç yükleme yolu üç farklı şey yapıyordu ve ikisi
//! mipmap'siz doku üretiyordu:
//!
//! | yol | mip | mipmap filtresi | aniso |
//! |---|---|---|---|
//! | `asset::loaders::images` (glTF — arabaların ve şehrin dokuları) | 1 | Nearest | kapalı |
//! | `asset::texture::install_decoded_material_texture` (disk/streaming/prosedürel) | 1 | Nearest | kapalı |
//! | `Renderer::create_texture` (elle RGBA) | tam zincir | Linear | kapalı |
//!
//! Yani mipmap üreteci ve `mipmap.wgsl` shader'ı zaten vardı, ama motorun asıl doku
//! yollarının hiçbiri onu çağırmıyordu — eğik açıdan bakılan her yüzey minification
//! aliasing'i yaşıyordu ve hiçbir sampler anizotropi istemiyordu.

/// Materyal sampler'larının istediği anizotropik filtreleme seviyesi.
///
/// 16, WebGPU'nun izin verdiği tavan; backend cihazın gerçek limitine kırpar, yani bunu
/// desteklemeyen bir GPU'da hata değil sessiz düşüş olur. Anizotropi maliyeti yalnız
/// minification'ın gerçekten olduğu (eğik, uzak) yüzeylerde ödenir.
pub(crate) const MATERIAL_ANISOTROPY: u16 = 16;

/// 2-B bir doku için tam mip zinciri uzunluğu.
///
/// `max(1)` sıfır boyuta karşı değil — çağıranlar zaten sıfırı eliyor — `ilog2()`'nin 0'da
/// panikleyecek olmasına karşı.
pub(crate) fn mip_level_count(width: u32, height: u32) -> u32 {
    width.max(height).max(1).ilog2() + 1
}

/// Mip zinciri üretilecek bir dokunun ihtiyaç duyduğu ek kullanım bayrağı.
///
/// Üretici her mip seviyesine ÇİZEREK indirger (blit), o yüzden doku render hedefi
/// olabilmeli. Bunu eklemeyi unutmak wgpu'da yükleme anında validation hatası verir.
pub(crate) const MIPPED_TEXTURE_USAGE: wgpu::TextureUsages =
    wgpu::TextureUsages::TEXTURE_BINDING
        .union(wgpu::TextureUsages::COPY_DST)
        .union(wgpu::TextureUsages::RENDER_ATTACHMENT);

/// Örneklenen bir materyal dokusunun sampler'ı.
///
/// `has_mips`, dokunun gerçekten bir mip zinciriyle yüklenip yüklenmediğidir — sampler'ı
/// dokunun sahip OLMADIĞI seviyelerden örneklemeye ayarlamak bir doğrulama hatası, ve
/// bunun tersi (zincir var ama sampler `Nearest`) tam olarak bu modülün düzelttiği
/// sessiz kayıptı.
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

/// Mip zincirini ardışık blit'lerle dolduran, YENİDEN KULLANILABİLİR üreteç.
///
/// Bir struct, çünkü eskiden bu bir serbest fonksiyondu ve **her çağrıda shader modülünü
/// ve render pipeline'ını yeniden derliyordu.** Tek bir elle oluşturulan dokuda bu
/// görünmezdi; yüzlerce dokusu olan bir glTF sahnesinde (Bayview) doku başına bir pipeline
/// derlemesi demek olurdu. Blitter'ı bir kez kurup partideki her doku için yeniden kullan.
pub(crate) struct MipmapBlitter {
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    format: wgpu::TextureFormat,
}

impl MipmapBlitter {
    /// `format` için bir üreteç kurar. Pipeline renk hedefi formatına bağlı olduğundan
    /// blitter tek bir formata özgüdür — sRGB ve lineer dokular ayrı blitter ister.
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

    /// `texture`'ın 1..`mip_level_count` seviyelerini, her birini bir öncekinden
    /// örnekleyerek doldurur. Seviye 0 zaten yazılmış olmalıdır.
    ///
    /// Komutları verilen `encoder`'a yazar; **submit etmez.** Bir parti dokunun tek bir
    /// komut tamponunda toplanabilmesi için böyle.
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

        let views: Vec<wgpu::TextureView> = (0..mip_level_count)
            .map(|mip| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some(&format!("Mip {mip}")),
                    format: None,
                    dimension: None,
                    usage: None,
                    aspect: wgpu::TextureAspect::All,
                    base_mip_level: mip,
                    mip_level_count: Some(1),
                    base_array_layer: 0,
                    array_layer_count: None,
                })
            })
            .collect();

        let bind_group_layout = self.pipeline.get_bind_group_layout(0);

        for target_mip in 1..mip_level_count as usize {
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                layout: &bind_group_layout,
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

    /// Tek bir doku için kolaylık sarmalayıcısı: kaydet ve hemen submit et.
    /// Bir parti yüklüyorsan [`record`](Self::record) kullan.
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
