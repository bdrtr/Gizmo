/// One rigid box in the GPU physics demo — its full state, owned by the compute shader.
///
/// This is *not* the engine's rigid-body pipeline (that is `gizmo-physics-rigid`, on the CPU and
/// deterministic). It is a self-contained GPU solver whose state never round-trips through the
/// CPU: the buffer is seeded once, stepped by `physics_compute.wgsl`, and then drawn straight from
/// the same bytes as instance data via [`GpuBox::desc`]. That is why the render fields (`color`,
/// `half_extents`) sit in the simulation struct rather than beside it.
///
/// The scalars are wedged into the `w` slots of the vectors because WGSL aligns a `vec3<f32>` to 16
/// bytes: `mass` after `position` costs nothing, whereas `mass` on its own line would cost 12 bytes
/// of padding.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuBox {
    /// World-space centre of mass.
    pub position: [f32; 3],
    /// Mass in kilograms. Zero means immovable.
    pub mass: f32,
    /// Linear velocity, m/s.
    pub velocity: [f32; 3],
    /// 0 = awake and integrating, 1 = asleep. A sleeping box is skipped by the integrator and by
    /// the solver, and is woken by a neighbour's contact.
    pub state: u32,
    /// Orientation as a quaternion, `xyzw`.
    pub rotation: [f32; 4],
    /// Angular velocity, rad/s, about the world axes.
    pub angular_velocity: [f32; 3],
    /// How many consecutive frames this box has been below the sleep threshold. The shader puts it
    /// to sleep once the count passes its limit, and resets it on any significant motion.
    pub sleep_counter: u32,
    /// Render colour, RGBA — carried here because the simulation buffer is also the instance
    /// buffer.
    pub color: [f32; 4],
    /// Half the box's extent along each of its local axes.
    pub half_extents: [f32; 3],
    /// Padding to the 16-byte alignment WGSL gives the preceding `vec3`.
    pub _pad: u32,
}

impl GpuBox {
    /// This struct as a per-instance vertex layout, so the simulation buffer can be bound
    /// directly as instance data with no copy: six `vec4` slots at locations 8..=13, covering the
    /// whole 96-byte record.
    pub fn desc<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuBox>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 8,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 16,
                    shader_location: 9,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 32,
                    shader_location: 10,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 48,
                    shader_location: 11,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 64,
                    shader_location: 12,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 80,
                    shader_location: 13,
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        }
    }
}

/// A static collider the GPU boxes collide against — an axis-aligned box or an infinite plane.
///
/// One record for both shapes, with the meaning of `data1`/`data2` switching on `shape_type`: a
/// WGSL storage array has one element type, so a tagged union is the only way to hold two shapes
/// in one buffer.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuCollider {
    /// Which shape this is: 0 = AABB, 1 = plane.
    pub shape_type: u32,
    /// Padding to the 16-byte alignment the `vec4`s below require.
    pub _pad1: [u32; 3],
    /// AABB: the minimum corner. Plane: its normal.
    pub data1: [f32; 4],
    /// AABB: the maximum corner. Plane: `[d, _, _, _]`, the plane's offset along its normal.
    pub data2: [f32; 4],
}

// ═══ Joint / Constraint Sistemi ═══
// 5 joint tipi: Ball(0), Hinge(1), Fixed(2), Spring(3), Slider(4)
// 64 bytes — WGSL vec3<f32> 16-byte alignment uyumlu
/// A constraint between two [`GpuBox`]es, in the five flavours the compute solver knows.
///
/// 64 bytes, laid out so each `vec3` is followed by the scalar that fills its 16-byte slot.
/// Construct one through [`GpuJoint::ball`] and friends rather than by hand — they are what fix
/// the type tag and the active bit.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuJoint {
    /// Index of the first body in the box buffer.
    pub body_a: u32,
    /// Index of the second body, or `u32::MAX` to joint against the static world.
    pub body_b: u32,
    /// 0 = ball, 1 = hinge, 2 = fixed, 3 = spring, 4 = slider.
    pub joint_type: u32,
    /// Bit 0 = active, bit 1 = breakable. A joint that breaks clears bit 0.
    pub flags: u32,
    /// The attachment point in body A's local space.
    pub anchor_a: [f32; 3],
    /// XPBD compliance — the inverse of stiffness. 0 = rigid, larger = softer.
    pub compliance: f32,
    /// The attachment point in body B's local space.
    pub anchor_b: [f32; 3],
    /// Spring damping coefficient.
    pub damping: f32,
    /// The hinge or slider axis, in body A's local space.
    pub axis: [f32; 3],
    /// The force that breaks this joint; 0 means unbreakable. Only read when bit 1 of
    /// [`Self::flags`] is set.
    pub max_force: f32,
}

impl GpuJoint {
    /// A ball joint — joins two bodies at a point and rotates freely.
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

    /// A hinge — rotation about a single axis, every other motion constrained.
    pub fn hinge(
        body_a: u32,
        body_b: u32,
        anchor_a: [f32; 3],
        anchor_b: [f32; 3],
        axis: [f32; 3],
    ) -> Self {
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

    /// A fixed joint — every motion constrained (a weld).
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

    /// A spring — a soft connection, with stiffness and damping.
    pub fn spring(
        body_a: u32,
        body_b: u32,
        anchor_a: [f32; 3],
        anchor_b: [f32; 3],
        stiffness: f32,
        damping: f32,
    ) -> Self {
        let compliance = if stiffness > 0.0 {
            1.0 / stiffness
        } else {
            0.0
        };
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

    /// A slider — sliding along a single axis, every other motion constrained.
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

    /// Breakable — the joint breaks once a given force is exceeded.
    pub fn breakable(mut self, force: f32) -> Self {
        self.max_force = force;
        self.flags |= 2; // bit1 = breakable
        self
    }
}

/// The per-step parameters the physics compute shader reads: the timestep, gravity, and how much
/// of each buffer is live.
///
/// 64 bytes. Every `_pad` field below is the explicit Rust half of an implicit WGSL alignment gap —
/// naga inserts them whether or not this struct does, so leaving one out shifts every field after
/// it.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PhysicsSimParams {
    /// The timestep, in seconds.
    pub dt: f32, // offset 0
    /// Padding: WGSL aligns the following `vec3` to 16 bytes.
    pub _pad0: [u32; 3], // offset 4-15
    /// Padding: a `vec3<f32>` slot the shader declares but does not use.
    pub _pad1: [f32; 3], // offset 16-27
    /// Padding, completing that slot's 16 bytes.
    pub _pad1b: u32, // offset 28-31
    /// Gravity, m/s².
    pub gravity: [f32; 3], // offset 32-43
    /// Per-step linear velocity damping.
    pub damping: f32, // offset 44-47
    /// How many entries of the box buffer are live.
    pub num_boxes: u32, // offset 48-51
    /// How many entries of the collider buffer are live.
    pub num_colliders: u32, // offset 52-55
    /// How many entries of the joint buffer are live.
    pub num_joints: u32, // offset 56-59
    /// Padding to 64 bytes.
    pub _pad2: u32, // offset 60-63
}

// ═══ Physics Debug Renderer Tipleri ═══

/// One vertex of the physics debug overlay's line list.
///
/// Its own vertex type rather than the renderer's [`Vertex`](crate::gpu_types::Vertex): the overlay
/// draws untextured, unlit lines, and 16 bytes per vertex against 96 matters when the wireframe
/// pass emits a fresh buffer every frame.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DebugVertex {
    /// World-space position.
    pub position: [f32; 3],
    /// RGBA packed into one `u32`, 8 bits per channel.
    pub color: u32,
}

impl DebugVertex {
    /// This vertex as wgpu describes it: position at location 0, packed colour at location 1.
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

/// What the debug overlay's compute pass should emit this frame.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DebugParams {
    /// How many boxes to draw outlines for.
    pub num_boxes: u32,
    /// How many joints to draw.
    pub num_joints: u32,
    /// Which overlays are on: bit 0 = box wireframes, bit 1 = joints, bit 2 = velocity vectors.
    pub show_wireframes: u32,
    /// Padding to 16 bytes.
    pub _pad: u32,
}

/// CPU-side mirror of the WGSL `BoxContacts` struct (physics_compute.wgsl), used
/// ONLY to size `box_contacts_buffer` via `size_of` — the compute shader owns all
/// field semantics; the CPU never reads these bytes field-by-field.
///
/// The layout MUST match WGSL std430 exactly, because the shader indexes
/// `box_contacts[idx]` with the std430 element stride. Walking std430 offsets:
///   count            u32               @0
///   (implicit pad to align vec3<u32>)  @4..16
///   _pad             vec3<u32>          @16..28
///   neighbors        array<u32,8>       @28..60
///   (implicit pad to align vec4<f32>)  @60..64   ← the 16-byte alignment gap
///   normals          array<vec4<f32>,8> @64..192
///   accum_impulse    array<vec4<f32>,8> @192..320
///   is_active        array<u32,8>       @320..352
/// → stride **352 bytes**, NOT 336. The old hand-computed `336` literal omitted
/// both alignment gaps, under-allocating the buffer by 16 B/box; boxes past
/// `floor(size / 352)` then indexed out of bounds and silently received no
/// contact manifold (they interpenetrated / tunnelled).
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuBoxContacts {
    /// How many of the eight neighbour slots are in use.
    pub count: u32,
    /// Implicit std430 padding between `count` and the `vec3<u32>` below.
    pub _pad_count: [u32; 3], // @4
    /// The WGSL side's `_pad: vec3<u32>`.
    pub _pad: [u32; 3], // @16
    /// Indices of the boxes this one is touching.
    pub neighbors: [u32; 8], // @28
    /// Implicit std430 padding, aligning the `vec4` array below to 16 bytes.
    pub _pad_align: u32, // @60
    /// Contact normal per neighbour slot.
    pub normals: [[f32; 4]; 8], // @64
    /// Accumulated impulse per slot, carried between solver iterations (and between frames, which
    /// is what makes the stack warm-start instead of sinking).
    pub accum_impulse: [[f32; 4]; 8], // @192
    /// Whether each slot holds a live contact.
    pub is_active: [u32; 8], // @320 .. 352
}
