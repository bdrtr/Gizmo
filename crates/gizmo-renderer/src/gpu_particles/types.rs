#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuParticle {
    pub position: [f32; 3],
    pub life: f32,
    pub velocity: [f32; 3],
    pub max_life: f32,
    pub color: [f32; 4],
    pub size_start: f32,
    pub size_end: f32,
    pub _padding: [f32; 2],
}

impl GpuParticle {
    pub fn desc<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuParticle>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x4,
                }, // pos + life
                wgpu::VertexAttribute {
                    offset: 16,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x4,
                }, // vel + max_life
                wgpu::VertexAttribute {
                    offset: 32,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x4,
                }, // color
                wgpu::VertexAttribute {
                    offset: 48,
                    shader_location: 3,
                    format: wgpu::VertexFormat::Float32x4,
                }, // sizes + padding
            ],
        }
    }
}

/// Parçacık sim uniform'unda taşınabilecek maksimum engel (küre) sayısı.
pub const MAX_PARTICLE_OBSTACLES: usize = 8;

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ParticleSimParams {
    pub dt: f32,
    pub global_gravity: f32,
    pub global_drag: f32,
    /// Aktif engel sayısı (0 = engel yok → sapma davranışı KAPALI, eski davranış).
    pub obstacle_count: f32,
    /// xyz = nominal akış hızı (relaks hedefi), w = relaks oranı (0 = kapalı).
    /// Parçacık hızı her frame bu hedefe doğru yumuşakça çekilir → engelden sonra
    /// akış çizgileri tekrar paralelleşir (aşağı-akış birleşmesi).
    pub flow_target: [f32; 4],
    /// x = türbülans gücü (relaks hedefine eklenen diverjanssız swirl genliği → duman
    /// gibi dalgalı filamentler). yzw = ileride kullanım için rezerve.
    pub misc: [f32; 4],
    /// Engel küreleri: xyz = merkez (dünya), w = yarıçap. `obstacle_count` kadarı geçerli.
    pub obstacles: [[f32; 4]; MAX_PARTICLE_OBSTACLES],
}
