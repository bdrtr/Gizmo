#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuBox {
    pub position: [f32; 3],
    pub mass: f32,
    pub velocity: [f32; 3],
    pub state: u32,
    pub rotation: [f32; 4],
    pub angular_velocity: [f32; 3],
    pub sleep_counter: u32,
    pub color: [f32; 4],
    pub half_extents: [f32; 3],
    pub _pad: u32,
}

impl GpuBox {
    pub fn desc<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuBox>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 6,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 16,
                    shader_location: 7,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 32,
                    shader_location: 8,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 48,
                    shader_location: 9,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 64,
                    shader_location: 10,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 80,
                    shader_location: 11,
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuCollider {
    pub shape_type: u32, // 0 = AABB, 1 = Plane
    pub _pad1: [u32; 3],
    pub data1: [f32; 4], // AABB: min, Plane: normal
    pub data2: [f32; 4], // AABB: max, Plane: [d, pad, pad, pad]
}

// ═══ Joint / Constraint Sistemi ═══
// 5 joint tipi: Ball(0), Hinge(1), Fixed(2), Spring(3), Slider(4)
// 64 bytes — WGSL vec3<f32> 16-byte alignment uyumlu
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuJoint {
    pub body_a: u32,          // A gövde indeksi
    pub body_b: u32,          // B gövde indeksi (u32::MAX = dünya/statik)
    pub joint_type: u32,      // 0=Ball, 1=Hinge, 2=Fixed, 3=Spring, 4=Slider
    pub flags: u32,           // bit0=active, bit1=breakable
    pub anchor_a: [f32; 3],   // A gövdesi lokal-uzay bağlantı noktası
    pub compliance: f32,      // 0 = sert, >0 = yumuşak (XPBD)
    pub anchor_b: [f32; 3],   // B gövdesi lokal-uzay bağlantı noktası
    pub damping: f32,         // Yay sönümleme katsayısı
    pub axis: [f32; 3],       // Hinge/Slider ekseni (A lokal uzayı)
    pub max_force: f32,       // Kırılma kuvveti (0 = kırılmaz)
}

impl GpuJoint {
    /// Küresel eklem — iki gövdeyi bir noktada birleştirir, serbest dönüş.
    pub fn ball(body_a: u32, body_b: u32, anchor_a: [f32; 3], anchor_b: [f32; 3]) -> Self {
        Self {
            body_a,
            body_b,
            joint_type: 0,
            flags: 1, // active
            anchor_a,
            compliance: 0.0,
            anchor_b,
            damping: 0.0,
            axis: [0.0, 1.0, 0.0],
            max_force: 0.0,
        }
    }

    /// Menteşe — tek eksen etrafında dönüş, diğer tüm hareket kısıtlı.
    pub fn hinge(body_a: u32, body_b: u32, anchor_a: [f32; 3], anchor_b: [f32; 3], axis: [f32; 3]) -> Self {
        Self {
            body_a,
            body_b,
            joint_type: 1,
            flags: 1,
            anchor_a,
            compliance: 0.0,
            anchor_b,
            damping: 0.0,
            axis,
            max_force: 0.0,
        }
    }

    /// Sabit birleşim — tüm hareket kısıtlı (kaynak gibi).
    pub fn fixed(body_a: u32, body_b: u32, anchor_a: [f32; 3], anchor_b: [f32; 3]) -> Self {
        Self {
            body_a,
            body_b,
            joint_type: 2,
            flags: 1,
            anchor_a,
            compliance: 0.0,
            anchor_b,
            damping: 0.0,
            axis: [0.0, 1.0, 0.0],
            max_force: 0.0,
        }
    }

    /// Yay — yumuşak bağlantı, stiffness ve damping ile.
    pub fn spring(body_a: u32, body_b: u32, anchor_a: [f32; 3], anchor_b: [f32; 3], stiffness: f32, damping: f32) -> Self {
        let compliance = if stiffness > 0.0 { 1.0 / stiffness } else { 0.0 };
        Self {
            body_a,
            body_b,
            joint_type: 3,
            flags: 1,
            anchor_a,
            compliance,
            anchor_b,
            damping,
            axis: [0.0, 1.0, 0.0],
            max_force: 0.0,
        }
    }

    /// Sürgü — tek eksen boyunca kayma, diğer tüm hareket kısıtlı.
    pub fn slider(body_a: u32, body_b: u32, axis: [f32; 3]) -> Self {
        Self {
            body_a,
            body_b,
            joint_type: 4,
            flags: 1,
            anchor_a: [0.0; 3],
            compliance: 0.0,
            anchor_b: [0.0; 3],
            damping: 0.0,
            axis,
            max_force: 0.0,
        }
    }

    /// Kırılabilir yap — belirli kuvvet aşılınca joint kırılır.
    pub fn breakable(mut self, force: f32) -> Self {
        self.max_force = force;
        self.flags |= 2; // bit1 = breakable
        self
    }
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PhysicsSimParams {
    // WGSL vec3<f32> → 16-byte alignment. Toplam struct: 64 bytes.
    pub dt: f32,            // offset 0
    pub _pad0: [u32; 3],    // offset 4-15  (WGSL implicit padding — vec3 align 16)
    pub _pad1: [f32; 3],    // offset 16-27 (WGSL _pad1: vec3<f32>)
    pub _pad1b: u32,        // offset 28-31 (WGSL implicit padding — vec3 align 16)
    pub gravity: [f32; 3],  // offset 32-43 (WGSL gravity: vec3<f32>)
    pub damping: f32,       // offset 44-47
    pub num_boxes: u32,     // offset 48-51
    pub num_colliders: u32, // offset 52-55
    pub num_joints: u32,    // offset 56-59
    pub _pad2: u32,         // offset 60-63
}

// ═══ Physics Debug Renderer Tipleri ═══

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DebugVertex {
    pub position: [f32; 3],
    pub color: u32,  // packed RGBA
}

impl DebugVertex {
    pub fn desc<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<DebugVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: 12,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Uint32,
                },
            ],
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DebugParams {
    pub num_boxes: u32,
    pub num_joints: u32,
    pub show_wireframes: u32,  // bit0=boxes, bit1=joints, bit2=velocity
    pub _pad: u32,
}
