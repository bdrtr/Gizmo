use gizmo_math::Vec3;
use std::sync::Arc;
use wgpu::util::DeviceExt;

#[derive(Clone)]
pub struct Mesh {
    pub vbuf: Arc<wgpu::Buffer>,
    pub vertex_count: u32,
    /// Geometrinin ağırlık merkezini orijine taşımak için kullanılan ofset değeri.
    /// Render aşamasında model matrisine uygulanabilir.
    /// AABB sınırlarını doğrudan etkilemez (sınırlar ham vertex verisinden hesaplanır).
    pub center_offset: Vec3,
    pub source: String,
    pub bounds: gizmo_math::Aabb,
    pub cpu_vertices: Arc<Vec<Vec3>>,
    pub lod_vbufs: Vec<Arc<wgpu::Buffer>>,
    pub lod_vertex_counts: Vec<u32>,
}

impl Mesh {
    /// Yeni bir `Mesh` bileşeni oluşturur.
    /// `vertices` dizisi üzerinden otomatik olarak `vertex_count` ve `bounds` hesaplanır.
    /// Hata durumlarında boş bir mesh oluşturmak için `Mesh::empty()` kullanılmalıdır.
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
        if vertex_count > 1000 {
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
            center_offset,
            source,
            bounds,
            cpu_vertices,
            lod_vbufs,
            lod_vertex_counts,
        }
    }

    /// Dosya yüklenememesi gibi durumlarda motorun çökmemesi için
    /// 0 vertex'li, boş bir yer tutucu (fallback) Mesh oluşturur.
    pub fn empty(vbuf: Arc<wgpu::Buffer>, source: String) -> Self {
        Self {
            vbuf,
            vertex_count: 0,
            center_offset: Vec3::ZERO,
            source,
            bounds: gizmo_math::Aabb::empty(),
            cpu_vertices: Arc::new(Vec::new()),
            lod_vbufs: Vec::new(),
            lod_vertex_counts: Vec::new(),
        }
    }
}

/// Bir entity'nin ekrana çizilebilir bir Mesh olduğunu belirten ECS marker bileşenidir.
/// Hiçbir ek alan içermez; sadece entity'nin render sistemine dahil edilmesini sağlar.
#[derive(Clone)]
pub struct MeshRenderer;

impl MeshRenderer {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MeshRenderer {
    fn default() -> Self {
        Self::new()
    }
}
