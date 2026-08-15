use gizmo_math::Vec3;
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use wgpu::util::DeviceExt;

#[derive(Clone)]
pub struct Mesh {
    pub vbuf: Arc<wgpu::Buffer>,
    pub vertex_count: u32,
    /// İndeks tamponu, varsa. `None` ise mesh düz üçgen listesi olarak çizilir
    /// (`draw`), `Some` ise `draw_indexed` ile.
    ///
    /// Opsiyonel olmasının sebebi geçiş değil, **LOD**: `lod_vbufs` düzleştirilmiş
    /// tamponlar tutuyor, yani bir LOD seviyesi aktifken indeks tamponu geçerli
    /// DEĞİL (indeksler tam çözünürlüklü vertex dizisine göre). Batching bu durumda
    /// indeksi düşürüyor.
    pub ibuf: Option<Arc<wgpu::Buffer>>,
    /// `ibuf`'taki indeks sayısı — çizilecek eleman sayısı budur, `vertex_count` değil.
    /// `ibuf` `None` iken anlamsız (0).
    pub index_count: u32,
    /// `ibuf`'un eleman genişliği. Tampon hangi formatta YAZILDIYSA `set_index_buffer`'a da
    /// o verilmeli — 16-bit bir tamponu 32-bit diye bağlamak çökmez, **yanlış üçgen çizer**.
    /// Bu yüzden türetilmiyor, taşınıyor. `ibuf` `None` iken anlamsız.
    pub index_format: wgpu::IndexFormat,
    /// Geometrinin ağırlık merkezini orijine taşımak için kullanılan ofset değeri.
    /// Render aşamasında model matrisine uygulanabilir.
    /// AABB sınırlarını doğrudan etkilemez (sınırlar ham vertex verisinden hesaplanır).
    pub center_offset: Vec3,
    pub source: String,
    pub bounds: gizmo_math::Aabb,
    /// Geometrinin CPU tarafındaki kopyası, **düz üçgen listesi olarak**: ardışık her üçlü
    /// bir üçgendir.
    ///
    /// `vbuf` ile indeks-indeks eşleşmesi GARANTİ DEĞİL — indeksli bir mesh'te (`ibuf`
    /// `Some`) vertex tamponu tekilleştirilmiştir, bu alan ise üçgen listesi olarak kalır.
    /// Sözleşme budur çünkü tüketiciler burayı üçlü gruplar hâlinde yürüyor; GPU tamponuna
    /// karşılık gelen sırayı isteyen bir çağıran `ibuf`'u okumalı.
    pub cpu_vertices: Arc<Vec<Vec3>>,
    pub lod_vbufs: Vec<Arc<wgpu::Buffer>>,
    pub lod_vertex_counts: Vec<u32>,
}

impl Mesh {
    /// Yeni bir `Mesh` bileşeni oluşturur.
    /// `vertices` dizisi üzerinden otomatik olarak `vertex_count` ve `bounds` hesaplanır.
    /// Hata durumlarında boş bir mesh oluşturmak için `Mesh::empty()` kullanılmalıdır.
    // WASM: meshopt LOD üretimi native-only cfg'li — `device` ve `mut`'lar orada
    // kullanılmadığından hedefli allow (native lint gücü korunur).
    #[cfg_attr(target_arch = "wasm32", allow(unused_variables, unused_mut))]
    pub fn new(
        device: &wgpu::Device,
        vbuf: Arc<wgpu::Buffer>,
        vertices: &[crate::gpu_types::Vertex],
        center_offset: Vec3,
        source: String,
    ) -> Self {
        debug_assert!(
            !vertices.is_empty(),
            "Kullanım hatası: Normal kullanımlarda vertices boş olamaz. Boş (fallback) mesh için Mesh::empty() kullanın."
        );
        let vertex_count = vertices.len() as u32;
        debug_assert_eq!(
            vertex_count as usize * std::mem::size_of::<crate::gpu_types::Vertex>(),
            vbuf.size() as usize
        );
        let bounds = gizmo_math::Aabb::from_points(vertices.iter().map(|v| v.position));
        let cpu_vertices = Arc::new(vertices.iter().map(|v| Vec3::from(v.position)).collect());

        let mut lod_vbufs = Vec::new();
        let mut lod_vertex_counts = Vec::new();

        // 1. Un-indexed vertex array üzerinden index array oluştur (meshopt için gereklidir)
        #[cfg(not(target_arch = "wasm32"))]
        if vertex_count > 20000 {
            let (unique_count, indices) = meshopt::generate_vertex_remap(vertices, None);

            let mut unique_vertices = vec![crate::gpu_types::Vertex::default(); unique_count];
            for (i, &new_idx) in indices.iter().enumerate() {
                unique_vertices[new_idx as usize] = vertices[i];
            }

            let adapter = meshopt::VertexDataAdapter::new(
                bytemuck::cast_slice(&unique_vertices),
                std::mem::size_of::<crate::gpu_types::Vertex>(),
                0,
            )
            .unwrap();

            let target_count = (indices.len() as f32 * 0.5) as usize; // %50 decimation
            let lod1_indices = meshopt::simplify(
                &indices,
                &adapter,
                target_count,
                0.1, // %10 error tolerance
                meshopt::SimplifyOptions::empty(),
                None,
            );

            // Eğer başarıyla decimation yapıldıysa ve gerçekten vertex sayısı düştüyse GPU'ya at
            if !lod1_indices.is_empty() && lod1_indices.len() < indices.len() {
                // Flat vertex array'e geri döndür (Gizmo renderer flat bekliyor)
                let mut lod_flat = Vec::with_capacity(lod1_indices.len());
                for &idx in &lod1_indices {
                    lod_flat.push(unique_vertices[idx as usize]);
                }

                let lod_vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(&format!("LOD1 VBuf: {}", source)),
                    contents: bytemuck::cast_slice(&lod_flat),
                    usage: wgpu::BufferUsages::VERTEX,
                });

                lod_vbufs.push(Arc::new(lod_vbuf));
                lod_vertex_counts.push(lod_flat.len() as u32);
            }
        }

        Self {
            vbuf,
            vertex_count,
            ibuf: None,
            index_count: 0,
            index_format: wgpu::IndexFormat::Uint32,
            center_offset,
            source,
            bounds,
            cpu_vertices,
            lod_vbufs,
            lod_vertex_counts,
        }
    }

    /// [`Mesh::new`] gibi, ama vertex'leri **tekilleştirip** bir indeks tamponu da kurar,
    /// ve vertex tamponunu kendisi oluşturur (çağıran hazır bir `vbuf` vermez).
    ///
    /// Motorun tekilleştirme makinesi zaten vardı ve çıktısı atılıyordu: [`Mesh::new`],
    /// 20000'den fazla vertex'te LOD üretmek için `meshopt::generate_vertex_remap` çağırıp
    /// bir indeks dizisi kuruyor, sonra `unique_vertices[idx]`'i düz bir diziye geri açıyor
    /// ("Gizmo renderer flat bekliyor"). Bir küpün 36 vertex'i 8'e, bir glTF sahnesinin
    /// paylaşılan köşeleri komşu üçgen sayısı kadar aza iner — hem yükleme bandı hem
    /// vertex shader çağrısı olarak.
    ///
    /// **Her hedefte çağrılabilir.** `meshopt` native-only olduğundan WASM'da tekilleştirme
    /// yapılmaz; mesh düz kalır ve `ibuf` `None` döner. Bu bir çağıran yükü DEĞİL: çizim yolu
    /// zaten iki durumu da taşıyor (`record_draw`), dolayısıyla `cfg` dallanması burada bir
    /// kez yapılıyor, her çağrı yerinde değil.
    pub fn new_indexed(
        device: &wgpu::Device,
        vertices: &[crate::gpu_types::Vertex],
        center_offset: Vec3,
        source: String,
    ) -> Self {
        #[cfg(target_arch = "wasm32")]
        {
            use wgpu::util::DeviceExt;
            let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(&format!("VBuf (flat, no meshopt): {source}")),
                contents: bytemuck::cast_slice(vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
            // Tail expression, not a `return`: on wasm the block below is stripped, so this
            // block IS the function body and a `return` here reads as needless — which only the
            // wasm lint can see, and only since CI started linting that target.
            Mesh::new(device, Arc::new(vbuf), vertices, center_offset, source)
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
        let (unique_count, indices) = meshopt::generate_vertex_remap(vertices, None);

        let mut unique_vertices = vec![crate::gpu_types::Vertex::default(); unique_count];
        for (i, &new_idx) in indices.iter().enumerate() {
            unique_vertices[new_idx as usize] = vertices[i];
        }

        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("VBuf (indexed): {source}")),
            contents: bytemuck::cast_slice(&unique_vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        // 16-bit indeksler, sığdığı sürece: indeks tamponunun bandını ve belleğini yarıya
        // indiriyor, ve motorun ürettiği her şey rahatça sığıyor (en büyük primitif torus,
        // 561 tekil vertex). `meshopt` `u32` üretiyor, daraltma burada yapılıyor.
        //
        // Eşik `<= 65536`: `u16`'nın taşıyabildiği en büyük indeks 65535, dolayısıyla o kadar
        // TEKİL VERTEX adreslenebilir. Sınırı `< 65536` yazmak son vertex'i boşuna kaybederdi;
        // `<= 65537` yazmak sessizce sarardı.
        let (index_format, index_bytes): (wgpu::IndexFormat, Vec<u8>) = if unique_count <= 65536 {
            let narrow: Vec<u16> = indices.iter().map(|&i| i as u16).collect();
            (
                wgpu::IndexFormat::Uint16,
                bytemuck::cast_slice(&narrow).to_vec(),
            )
        } else {
            (
                wgpu::IndexFormat::Uint32,
                bytemuck::cast_slice(&indices).to_vec(),
            )
        };
        let ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("IBuf: {source}")),
            // `create_buffer_init` boyutu `COPY_BUFFER_ALIGNMENT`'a kendisi yuvarlıyor, yani
            // tek sayıda `u16` indeks (2 bayt hizası, 4 değil) sorun değil.
            contents: &index_bytes,
            usage: wgpu::BufferUsages::INDEX,
        });

        // `Mesh::new`'e TEKİLLEŞTİRİLMİŞ diziyi veriyoruz — `vertex_count` ile `vbuf.size()`
        // arasındaki debug_assert ancak böyle tutar, ve `bounds` iki durumda da aynı nokta
        // kümesinden çıkıyor.
        let mut mesh = Mesh::new(
            device,
            Arc::new(vbuf),
            &unique_vertices,
            center_offset,
            source,
        );
        mesh.index_count = indices.len() as u32;
        mesh.index_format = index_format;
        mesh.ibuf = Some(Arc::new(ibuf));

        // `cpu_vertices` ORİJİNAL üçgen listesine geri alınıyor. Tekilleştirilmiş diziyi
        // orada bırakmak sessiz bir bozulma olurdu: alanın tüketicileri onu üçlü gruplar
        // hâlinde yürüyor (`demo/src/bin/wind_tunnel.rs:535` — `i, i+1, i+2`, adım 3), ve
        // tekilleştirilmiş bir dizide ardışık üçlüler artık üçgen DEĞİL. Bellek açısından
        // bu bir gerileme de değil: bu alan zaten tam listeyi tutuyordu.
        mesh.cpu_vertices = Arc::new(vertices.iter().map(|v| Vec3::from(v.position)).collect());
        mesh
        }
    }

    /// Dosya yüklenememesi gibi durumlarda motorun çökmemesi için
    /// 0 vertex'li, boş bir yer tutucu (fallback) Mesh oluşturur.
    pub fn empty(vbuf: Arc<wgpu::Buffer>, source: String) -> Self {
        Self {
            vbuf,
            vertex_count: 0,
            ibuf: None,
            index_count: 0,
            index_format: wgpu::IndexFormat::Uint32,
            center_offset: Vec3::ZERO,
            source,
            bounds: gizmo_math::Aabb::empty(),
            cpu_vertices: Arc::new(Vec::new()),
            lod_vbufs: Vec::new(),
            lod_vertex_counts: Vec::new(),
        }
    }

    /// Keyfi (üçgen-liste) vertex verisinden mesh kurar — vertex buffer'ı oluşturup
    /// [`Mesh::new`]'e verir. Prosedürel geometri (ör. akış-çizgisi şeritleri, debug
    /// çizimleri) için kısayol; çağıran wgpu buffer detaylarıyla uğraşmaz.
    pub fn from_vertices(
        device: &wgpu::Device,
        vertices: &[crate::gpu_types::Vertex],
        source: impl Into<String>,
    ) -> Self {
        use wgpu::util::DeviceExt;
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ProcMesh VBuf"),
            contents: bytemuck::cast_slice(vertices),
            // COPY_DST → içerik her frame güncellenebilir (bkz `update_vertices`).
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        Mesh::new(device, Arc::new(vbuf), vertices, Vec3::ZERO, source.into())
    }

    /// Vertex buffer'ının İÇERİĞİNİ yerinde günceller (aynı sayıda vertex; pozisyon/renk/uv
    /// değişebilir). Prosedürel/animasyonlu geometri için — mesh'i yeniden kurmadan her frame
    /// yazılabilir. Buffer `COPY_DST` ile kurulmuş olmalı (`from_vertices` öyle kurar) ve
    /// vertex sayısı DEĞİŞMEMELİ. Not: `bounds`/`vertex_count` güncellenmez (pozisyon aralığı
    /// aynı kalmalı) ve `>20000` vertex'te LOD buffer'ları güncellenmez.
    pub fn update_vertices(&self, queue: &wgpu::Queue, vertices: &[crate::gpu_types::Vertex]) {
        let bytes: &[u8] = bytemuck::cast_slice(vertices);
        debug_assert!(bytes.len() as u64 <= self.vbuf.size());
        queue.write_buffer(&self.vbuf, 0, bytes);
    }
}

/// Bir entity'nin ekrana çizilebilir bir Mesh olduğunu belirten ECS marker bileşenidir.
/// Hiçbir ek alan içermez; sadece entity'nin render sistemine dahil edilmesini sağlar.
#[derive(Clone)]
pub struct MeshRenderer {
    /// Whether this object casts a shadow, and whether it is drawn at all.
    ///
    /// Per-object rather than per-material, which is where the engine used to decide it: shadow
    /// casting fell out of the material's routing (unlit/skybox/grid were excluded and everything
    /// else cast), so two crates sharing one material could not differ. A light fixture's glowing
    /// panel and the wall behind it often want opposite answers.
    pub shadows: ShadowCasting,
}

/// What an object contributes to the shadow maps and to the picture.
///
/// The third state is not a curiosity: a low-poly stand-in that casts for an expensive mesh, or a
/// blocker that shapes a light without appearing, both need "cast but do not draw". It is the
/// prototype's `On / Off / Only`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShadowCasting {
    /// Drawn, and casts. The default, and what every object did before this existed.
    #[default]
    On,
    /// Drawn, casts nothing.
    Off,
    /// Casts, but is not drawn.
    Only,
}

impl ShadowCasting {
    /// Does this object go into the shadow maps?
    pub fn casts(self) -> bool {
        matches!(self, Self::On | Self::Only)
    }

    /// Does this object go into the camera's picture?
    pub fn visible(self) -> bool {
        matches!(self, Self::On | Self::Off)
    }
}

impl MeshRenderer {
    pub fn new() -> Self {
        Self { shadows: ShadowCasting::On }
    }

    /// Builder form: `MeshRenderer::new().with_shadows(ShadowCasting::Only)`.
    pub fn with_shadows(mut self, shadows: ShadowCasting) -> Self {
        self.shadows = shadows;
        self
    }
}

impl Default for MeshRenderer {
    fn default() -> Self {
        Self::new()
    }
}
