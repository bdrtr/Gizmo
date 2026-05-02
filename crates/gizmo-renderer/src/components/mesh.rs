use gizmo_math::Vec3;
use std::sync::Arc;

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
}

impl Mesh {
    /// Yeni bir `Mesh` bileşeni oluşturur.
    /// `vertices` dizisi üzerinden otomatik olarak `vertex_count` ve `bounds` hesaplanır.
    /// Hata durumlarında boş bir mesh oluşturmak için `Mesh::empty()` kullanılmalıdır.
    pub fn new(
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
        Self {
            vbuf,
            vertex_count,
            center_offset,
            source,
            bounds,
            cpu_vertices,
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
